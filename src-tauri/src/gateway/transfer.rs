use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::profiles::GatewayProfile;
use super::ssh_bin::resolve_scp_bin;
use super::ssh_tunnel::prepare_askpass;

pub fn scp_upload_args(
    profile: &GatewayProfile,
    local: &str,
    remote: &str,
    recursive: bool,
) -> Result<(PathBuf, Vec<String>), String> {
    let mut args = scp_common_args(profile);
    if recursive {
        args.push("-r".to_string());
    }
    args.push(local.to_string());
    args.push(remote_target(profile, remote));
    Ok((resolve_scp_bin()?, args))
}

pub fn run_upload(profile: &GatewayProfile, local: &str, remote: &str) -> Result<(), String> {
    let recursive = Path::new(local).is_dir();
    let (bin, args) = scp_upload_args(profile, local, remote, recursive)?;
    run_scp(profile, bin, args)
}

pub fn run_download(profile: &GatewayProfile, remote: &str, local: &str) -> Result<(), String> {
    let mut args = scp_common_args(profile);
    // `-r` is harmless for ordinary files and permits remote directory downloads.
    args.push("-r".to_string());
    args.push(remote_target(profile, remote));
    args.push(local.to_string());
    run_scp(profile, resolve_scp_bin()?, args)
}

fn scp_common_args(profile: &GatewayProfile) -> Vec<String> {
    let mut args = vec![
        "-o".to_string(),
        "ConnectTimeout=15".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
        "-o".to_string(),
        "NumberOfPasswordPrompts=1".to_string(),
        "-P".to_string(),
        profile.port.to_string(),
    ];

    let has_key = !profile.identity_file.trim().is_empty();
    let has_password = !profile.password.trim().is_empty();

    if has_key {
        args.push("-i".to_string());
        args.push(profile.identity_file.trim().to_string());
    }

    if has_password {
        args.push("-o".to_string());
        args.push(
            if has_key {
                "PreferredAuthentications=publickey,password,keyboard-interactive"
            } else {
                "PreferredAuthentications=password,keyboard-interactive"
            }
            .to_string(),
        );
        if !has_key {
            args.push("-o".to_string());
            args.push("PubkeyAuthentication=no".to_string());
        }
    } else {
        args.push("-o".to_string());
        args.push("BatchMode=yes".to_string());
        args.push("-o".to_string());
        args.push("PreferredAuthentications=publickey".to_string());
    }

    args
}

fn remote_target(profile: &GatewayProfile, remote: &str) -> String {
    format!("{}@{}:{}", profile.user.trim(), profile.host.trim(), remote)
}

fn run_scp(profile: &GatewayProfile, bin: PathBuf, args: Vec<String>) -> Result<(), String> {
    let askpass = prepare_askpass(profile)?;
    let mut command = Command::new(bin);
    command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(askpass) = askpass.as_ref() {
        askpass.apply(&mut command);
    }

    let output = command
        .output()
        .map_err(|error| format!("无法启动 scp（请确认已安装 OpenSSH 客户端）: {error}"))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(format!(
        "SCP 传输失败 (code {:?}): {}",
        output.status.code(),
        if stderr.is_empty() { stdout } else { stderr }
    ))
}

#[cfg(test)]
mod tests {
    use super::scp_upload_args;
    use crate::gateway::profiles::GatewayProfile;

    fn profile() -> GatewayProfile {
        GatewayProfile {
            id: "profile-1".to_string(),
            name: "Test host".to_string(),
            host: "example.test".to_string(),
            port: 2202,
            user: "alice".to_string(),
            identity_file: "/tmp/id_ed25519".to_string(),
            password: String::new(),
            remote_http_port: 17890,
            remote_socks_port: 17891,
            auto_reconnect: true,
            no_proxy: vec![],
            port_forwards: vec![],
            updated_at: "0".to_string(),
        }
    }

    #[test]
    fn upload_args_include_port_and_remote_destination() {
        let (_, args) = scp_upload_args(&profile(), "/tmp/report.txt", "~/uploads", false).unwrap();

        assert!(args.windows(2).any(|args| args == ["-P", "2202"]));
        assert!(args.iter().any(|arg| arg == "alice@example.test:~/uploads"));
        assert_eq!(
            args.last(),
            Some(&"alice@example.test:~/uploads".to_string())
        );
    }

    #[test]
    fn upload_args_add_recursive_flag_for_directories() {
        let (_, args) = scp_upload_args(&profile(), "/tmp/folder", "/srv/files", true).unwrap();

        assert!(args.iter().any(|arg| arg == "-r"));
    }
}
