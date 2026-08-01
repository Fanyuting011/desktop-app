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
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                        if app.emit(&event_name, chunk).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
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
