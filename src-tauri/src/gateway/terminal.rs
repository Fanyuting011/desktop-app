//! Interactive SSH terminal sessions (Task 7).
//!
//! This is deliberately a *second*, independent SSH process from the `-N -R` data-plane
//! tunnel in `ssh_tunnel.rs` — the tunnel keeps forwarding traffic untouched while this
//! module spawns a normal interactive `ssh user@host` login shell inside a PTY so the
//! embedded terminal (Task 8, xterm.js) behaves like a real terminal (colors, cursor
//! movement, resize, Ctrl-C, etc).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Mutex;
use std::thread;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tauri::{AppHandle, Emitter};

use super::askpass::AskpassEnv;
use super::profiles::GatewayProfile;
use super::ssh_tunnel::{prepare_askpass, ssh_common_args, target};

/// Default terminal geometry until the frontend sends the real size via `resize`
/// (xterm.js fires a resize right after mount, so this is only visible for an instant).
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    /// Kept alive so the ASKPASS temp files survive for the life of the ssh process.
    _askpass: Option<AskpassEnv>,
}

/// Holds one interactive PTY-backed `ssh` session per connected profile.
pub struct TerminalHub {
    sessions: Mutex<HashMap<String, PtySession>>,
}

impl TerminalHub {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Spawn an interactive `ssh` login into a PTY for `profile` and start streaming its
    /// output to the frontend via the `terminal-output-{profile_id}` event (UTF-8 lossy
    /// string chunks — simplest to consume from JS, no base64 round-trip needed since
    /// Tauri events already carry structured JSON payloads).
    pub fn open(&self, app: AppHandle, profile: &GatewayProfile) -> Result<(), String> {
        // Reopening (e.g. the user closed and reopened the terminal tab) replaces any
        // stale session for this profile rather than erroring out.
        self.close(&profile.id);

        let askpass = prepare_askpass(profile)?;

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("创建 PTY 失败: {e}"))?;

        let mut cmd = CommandBuilder::new("ssh");
        for arg in ssh_common_args(profile) {
            cmd.arg(arg);
        }
        if let Some(ap) = askpass.as_ref() {
            for (k, v) in ap.env_pairs() {
                cmd.env(k, v);
            }
        }
        cmd.arg(target(profile));

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("启动交互式 SSH 失败（请确认已安装 OpenSSH 客户端）: {e}"))?;
        // Drop our copy of the slave fd/handle so the master side sees EOF once the ssh
        // child itself exits, instead of hanging open on our own lingering reference.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("获取 PTY 读取句柄失败: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("获取 PTY 写入句柄失败: {e}"))?;

        let event_name = format!("terminal-output-{}", profile.id);
        thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 8192];
            // Bytes read but not yet emitted because they end mid-multibyte-UTF-8-sequence
            // (a single `read()` can split a char across two syscalls, e.g. a 3-byte CJK
            // char landing right at the end of the buffer) — held until the rest arrives.
            let mut pending: Vec<u8> = Vec::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        pending.extend_from_slice(&buf[..n]);
                        let chunk = drain_utf8_prefix(&mut pending);
                        if !chunk.is_empty() && app.emit(&event_name, chunk).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            // Flush any trailing incomplete bytes (e.g. the ssh process died mid-char)
            // rather than silently dropping them.
            if !pending.is_empty() {
                let _ = app.emit(&event_name, String::from_utf8_lossy(&pending).into_owned());
            }
        });

        let mut sessions = self.sessions.lock().expect("terminal hub poisoned");
        sessions.insert(
            profile.id.clone(),
            PtySession {
                master: pair.master,
                writer,
                child,
                _askpass: askpass,
            },
        );
        Ok(())
    }

    pub fn write(&self, profile_id: &str, data: &str) -> Result<(), String> {
        let mut sessions = self.sessions.lock().expect("terminal hub poisoned");
        let session = sessions
            .get_mut(profile_id)
            .ok_or_else(|| "终端未打开".to_string())?;
        session
            .writer
            .write_all(data.as_bytes())
            .map_err(|e| format!("写入终端失败: {e}"))?;
        session
            .writer
            .flush()
            .map_err(|e| format!("刷新终端失败: {e}"))
    }

    pub fn resize(&self, profile_id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let sessions = self.sessions.lock().expect("terminal hub poisoned");
        let session = sessions
            .get(profile_id)
            .ok_or_else(|| "终端未打开".to_string())?;
        session
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("调整终端大小失败: {e}"))
    }

    /// Kill the ssh child (if any) for this profile and drop its PTY handles. Safe to
    /// call for a profile with no open terminal (no-op).
    pub fn close(&self, profile_id: &str) {
        let mut sessions = self.sessions.lock().expect("terminal hub poisoned");
        if let Some(mut session) = sessions.remove(profile_id) {
            let _ = session.child.kill();
            let _ = session.child.wait();
        }
    }
}

impl Default for TerminalHub {
    fn default() -> Self {
        Self::new()
    }
}

/// Drain the leading, complete-UTF-8 prefix of `pending` into a `String`, leaving any
/// trailing incomplete multibyte sequence (at most 3 bytes — a valid UTF-8 char is at
/// most 4 bytes, so the 4th byte would always complete it) in `pending` for the next
/// read to complete.
///
/// If the trailing bytes are *not* actually a truncated char (e.g. the remote sent
/// genuinely non-UTF-8 binary data), they can never grow past 3 bytes before being
/// resolved: the next call either finds them completed into a valid prefix, or — once
/// there's no way they could still be a legitimate truncated char — flushes them
/// lossily instead of buffering forever.
fn drain_utf8_prefix(pending: &mut Vec<u8>) -> String {
    if pending.is_empty() {
        return String::new();
    }
    let valid_up_to = match std::str::from_utf8(pending) {
        Ok(_) => pending.len(),
        Err(e) => e.valid_up_to(),
    };
    // A genuinely truncated (but eventually valid) UTF-8 sequence is at most 3 bytes
    // long here (the 4th byte would complete it) — anything longer than that can't be a
    // legitimate truncation and must be flushed instead of held forever.
    let incomplete_tail_len = pending.len() - valid_up_to;
    let take_len = if incomplete_tail_len > 3 {
        pending.len()
    } else {
        valid_up_to
    };
    let bytes: Vec<u8> = pending.drain(..take_len).collect();
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

#[cfg(test)]
mod tests {
    use super::drain_utf8_prefix;

    #[test]
    fn drains_complete_ascii() {
        let mut pending = b"hello world".to_vec();
        assert_eq!(drain_utf8_prefix(&mut pending), "hello world");
        assert!(pending.is_empty());
    }

    #[test]
    fn holds_back_split_multibyte_char() {
        // "中" (U+4E2D) is E4 B8 AD in UTF-8 — simulate a read() that stopped mid-char.
        let full = "hi 中".as_bytes().to_vec();
        let split_at = full.len() - 1; // cut the last byte of the 3-byte char
        let mut pending = full[..split_at].to_vec();
        let first = drain_utf8_prefix(&mut pending);
        assert_eq!(first, "hi ");
        assert_eq!(pending.len(), 2, "the 2 valid lead bytes of 中 stay pending");

        // The rest of the char arrives in the next read.
        pending.extend_from_slice(&full[split_at..]);
        let second = drain_utf8_prefix(&mut pending);
        assert_eq!(second, "中");
        assert!(pending.is_empty());
    }

    #[test]
    fn flushes_genuinely_invalid_tail_instead_of_growing_forever() {
        // 5 consecutive continuation-only bytes can never become a valid char no matter
        // how much more data arrives — must not be buffered indefinitely.
        let mut pending = vec![0x80u8; 5];
        let out = drain_utf8_prefix(&mut pending);
        assert!(!out.is_empty());
        assert!(pending.is_empty());
    }

    #[test]
    fn empty_input_is_a_noop() {
        let mut pending = Vec::new();
        assert_eq!(drain_utf8_prefix(&mut pending), "");
    }
}
