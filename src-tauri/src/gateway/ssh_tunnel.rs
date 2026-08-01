use std::io::Read as _;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::askpass::AskpassEnv;
use super::log_buffer::LogBuffer;
use super::profiles::GatewayProfile;

pub struct SshTunnel {
    child: Child,
    /// Kept alive so ASKPASS temp files remain until the tunnel exits.
    _askpass: Option<AskpassEnv>,
}

pub(crate) fn target(profile: &GatewayProfile) -> String {
    format!("{}@{}", profile.user.trim(), profile.host.trim())
}

/// SSH CLI args shared by every invocation (tunnel, probes, remote exec, and the
/// interactive terminal in `terminal.rs`). Does not include the ASKPASS env vars —
/// callers apply those separately since `terminal.rs` builds a `portable_pty::CommandBuilder`
/// rather than a `std::process::Command`.
pub(crate) fn ssh_common_args(profile: &GatewayProfile) -> Vec<String> {
    let mut args = vec![
        "-o".to_string(),
        "ConnectTimeout=15".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
        "-o".to_string(),
        "NumberOfPasswordPrompts=1".to_string(),
        "-p".to_string(),
        profile.port.to_string(),
    ];

    let has_key = !profile.identity_file.trim().is_empty();
    let has_password = !profile.password.trim().is_empty();

    if has_key {
        args.push("-i".to_string());
        args.push(profile.identity_file.trim().to_string());
    }

    if has_password {
        if has_key {
            args.push("-o".to_string());
            args.push("PreferredAuthentications=publickey,password,keyboard-interactive".to_string());
        } else {
            args.push("-o".to_string());
            args.push("PreferredAuthentications=password,keyboard-interactive".to_string());
            args.push("-o".to_string());
            args.push("PubkeyAuthentication=no".to_string());
        }
    } else {
        // Key-only: never prompt interactively.
        args.push("-o".to_string());
        args.push("BatchMode=yes".to_string());
        args.push("-o".to_string());
        args.push("PreferredAuthentications=publickey".to_string());
    }

    args
}

fn base_ssh_cmd(profile: &GatewayProfile, askpass: Option<&AskpassEnv>) -> Command {
    let mut cmd = Command::new("ssh");
    for arg in ssh_common_args(profile) {
        cmd.arg(arg);
    }
    if !profile.password.trim().is_empty() {
        if let Some(ap) = askpass {
            ap.apply(&mut cmd);
        }
    }
    cmd
}

pub(crate) fn prepare_askpass(profile: &GatewayProfile) -> Result<Option<AskpassEnv>, String> {
    if profile.password.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(AskpassEnv::setup(profile.password.trim())?))
}

/// Auth probe before starting the long-lived -N tunnel.
pub fn verify_ssh_auth(profile: &GatewayProfile, logs: Arc<LogBuffer>) -> Result<(), String> {
    let dest = target(profile);
    let has_password = !profile.password.trim().is_empty();
    let has_key = !profile.identity_file.trim().is_empty();
    logs.push(format!(
        "探测 SSH 认证 → {dest}（{}）",
        match (has_key, has_password) {
            (true, true) => "公钥优先，密码回退",
            (true, false) => "仅公钥",
            (false, true) => "密码登录",
            (false, false) => "未配置认证",
        }
    ));

    let askpass = prepare_askpass(profile)?;
    let mut cmd = base_ssh_cmd(profile, askpass.as_ref());
    cmd.arg(&dest).arg("true");
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd
        .output()
        .map_err(|e| format!("无法启动 ssh（请确认已安装 OpenSSH 客户端）: {e}"))?;

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        for line in stderr.lines().take(12) {
            let t = line.trim();
            if !t.is_empty() {
                logs.push(format!("ssh-probe: {t}"));
            }
        }
    }

    if output.status.success() {
        logs.push("SSH 认证探测成功".to_string());
        return Ok(());
    }

    Err(format!(
        "SSH 认证失败 (code {:?}): {} — 请检查主机/用户/私钥或密码是否正确",
        output.status.code(),
        if stderr.is_empty() {
            "无详细错误".to_string()
        } else {
            stderr.lines().take(3).collect::<Vec<_>>().join(" | ")
        }
    ))
}

impl SshTunnel {
    pub fn spawn(
        profile: &GatewayProfile,
        local_http: u16,
        local_socks: u16,
        logs: Arc<LogBuffer>,
    ) -> Result<Self, String> {
        verify_ssh_auth(profile, logs.clone())?;

        let remote_http = profile.remote_http_port;
        let remote_socks = profile.remote_socks_port;
        let dest = target(profile);
        let askpass = prepare_askpass(profile)?;

        let mut cmd = base_ssh_cmd(profile, askpass.as_ref());
        cmd.arg("-N")
            .arg("-o")
            .arg("ExitOnForwardFailure=yes")
            .arg("-o")
            .arg("ServerAliveInterval=30")
            .arg("-o")
            .arg("ServerAliveCountMax=3")
            .arg("-R")
            .arg(format!("127.0.0.1:{remote_http}:127.0.0.1:{local_http}"))
            .arg("-R")
            .arg(format!("127.0.0.1:{remote_socks}:127.0.0.1:{local_socks}"))
            .arg(&dest);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        logs.push(format!(
            "启动 SSH 隧道: {dest} -R 127.0.0.1:{remote_http}/:{remote_socks}"
        ));

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("无法启动 ssh 隧道: {e}"))?;

        let err_buf: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        if let Some(stderr) = child.stderr.take() {
            let logs2 = logs.clone();
            let err_buf2 = err_buf.clone();
            thread::spawn(move || {
                let mut reader = stderr;
                let mut buf = [0u8; 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let text = String::from_utf8_lossy(&buf[..n]);
                            for line in text.lines() {
                                let t = line.trim();
                                if t.is_empty() {
                                    continue;
                                }
                                logs2.push(format!("ssh: {t}"));
                                if let Ok(mut g) = err_buf2.lock() {
                                    if g.len() < 40 {
                                        g.push(t.to_string());
                                    }
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            thread::sleep(Duration::from_millis(250));
            match child.try_wait() {
                Ok(Some(status)) => {
                    let detail = err_buf
                        .lock()
                        .map(|g| g.join(" | "))
                        .unwrap_or_default();
                    return Err(format!(
                        "SSH 隧道退出 (code {:?}){}",
                        status.code(),
                        if detail.is_empty() {
                            String::new()
                        } else {
                            format!(": {detail}")
                        }
                    ));
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        break;
                    }
                }
                Err(e) => return Err(format!("检查 SSH 进程失败: {e}")),
            }
        }

        let mut check = base_ssh_cmd(profile, askpass.as_ref());
        check
            .arg(&dest)
            .arg(format!(
                "bash -lc 'exec 3<>/dev/tcp/127.0.0.1/{remote_http}'"
            ));
        check
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        match check.output() {
            Ok(out) if out.status.success() => {
                logs.push(format!(
                    "已确认服务器 127.0.0.1:{remote_http} 可连接（转发就绪）"
                ));
            }
            Ok(_) => {
                let mut check2 = base_ssh_cmd(profile, askpass.as_ref());
                check2.arg(&dest).arg(format!(
                    "bash -lc '(ss -lnt 2>/dev/null || netstat -lnt 2>/dev/null) | grep -E \":{remote_http}\\\\b\"'"
                ));
                check2
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                match check2.output() {
                    Ok(o2) if o2.status.success() && !o2.stdout.is_empty() => {
                        logs.push(format!(
                            "已确认服务器端口 {remote_http} 在监听（转发就绪）"
                        ));
                    }
                    _ => {
                        logs.push(format!(
                            "警告: 未能主动探测远程端口 {remote_http}；SSH 进程仍在运行，暂标为已连接"
                        ));
                    }
                }
            }
            Err(e) => {
                logs.push(format!("警告: 远程端口探测无法执行: {e}"));
            }
        }

        logs.push("SSH 隧道已建立（认证与进程存活已校验）".to_string());
        Ok(Self {
            child,
            _askpass: askpass,
        })
    }

    pub fn try_wait(&mut self) -> Result<Option<i32>, String> {
        match self.child.try_wait() {
            Ok(Some(status)) => Ok(status.code()),
            Ok(None) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        self.kill();
    }
}

pub fn remote_run_script(
    profile: &GatewayProfile,
    script: &str,
    env: &[(&str, String)],
    logs: Arc<LogBuffer>,
) -> Result<String, String> {
    let askpass = prepare_askpass(profile)?;
    let mut cmd = base_ssh_cmd(profile, askpass.as_ref());
    let dest = target(profile);

    // Encode script as base64 so we don't rely on SSH stdin forwarding
    // (more reliable across OpenSSH clients).
    let b64 = base64_encode(script.as_bytes());
    let mut remote = String::new();
    for (k, v) in env {
        let escaped = v.replace('\'', "'\"'\"'");
        remote.push_str(&format!("export {k}='{escaped}'; "));
    }
    remote.push_str(&format!(
        "echo '{b64}' | base64 -d | bash"
    ));

    cmd.arg(&dest).arg(&remote);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    logs.push(format!("远程执行脚本 → {dest}"));

    let output = cmd
        .output()
        .map_err(|e| format!("远程 ssh 执行失败: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stdout.is_empty() {
        for line in stdout.lines() {
            logs.push(format!("remote: {line}"));
        }
    }
    if !stderr.is_empty() {
        for line in stderr.lines() {
            logs.push(format!("remote-err: {line}"));
        }
    }

    if !output.status.success() {
        return Err(format!(
            "远程脚本失败 (code {:?}): {}",
            output.status.code(),
            if stderr.is_empty() { stdout } else { stderr }
        ));
    }
    Ok(stdout)
}

/// Run a remote shell command string (non-interactive).
pub fn remote_run_shell(
    profile: &GatewayProfile,
    shell_cmd: &str,
    logs: Arc<LogBuffer>,
) -> Result<String, String> {
    let askpass = prepare_askpass(profile)?;
    let mut cmd = base_ssh_cmd(profile, askpass.as_ref());
    let dest = target(profile);
    // Use bash -lc so PATH hooks from profile may apply; still prepend bin explicitly in callers when needed.
    let remote = format!("bash -lc {}", shell_escape(shell_cmd));
    cmd.arg(&dest).arg(&remote);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    logs.push(format!("远程命令 → {dest}: {shell_cmd}"));

    let output = cmd
        .output()
        .map_err(|e| format!("远程 ssh 执行失败: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stdout.is_empty() {
        for line in stdout.lines() {
            logs.push(format!("remote: {line}"));
        }
    }
    if !stderr.is_empty() {
        for line in stderr.lines() {
            logs.push(format!("remote-err: {line}"));
        }
    }

    if !output.status.success() {
        return Err(format!(
            "远程命令失败 (code {:?}): {}",
            output.status.code(),
            if stderr.is_empty() { stdout } else { stderr }
        ));
    }
    Ok(stdout)
}

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let mut n = (chunk[0] as u32) << 16;
        if chunk.len() > 1 {
            n |= (chunk[1] as u32) << 8;
        }
        if chunk.len() > 2 {
            n |= chunk[2] as u32;
        }
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
