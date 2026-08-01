#[cfg(windows)]
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

pub fn resolve_ssh_bin() -> Result<PathBuf, String> {
    resolve_bin("ssh", ssh_candidates())
}

#[allow(dead_code)]
pub fn resolve_scp_bin() -> Result<PathBuf, String> {
    resolve_bin("scp", scp_candidates())
}

fn resolve_bin(name: &str, candidates: Vec<PathBuf>) -> Result<PathBuf, String> {
    if let Some(path) = find_on_path(name) {
        return Ok(path);
    }

    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| format!("未找到 OpenSSH 的 {name}，请安装 OpenSSH 客户端并确保在 PATH 中"))
}

#[cfg(windows)]
fn find_on_path(name: &str) -> Option<PathBuf> {
    let output = Command::new("where").arg(name).output().ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

#[cfg(not(windows))]
fn find_on_path(name: &str) -> Option<PathBuf> {
    let output = Command::new("sh")
        .args(["-c", "command -v \"$1\"", "sh", name])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    path.is_file().then_some(path)
}

pub(crate) fn ssh_candidates() -> Vec<PathBuf> {
    candidates("ssh")
}

#[allow(dead_code)]
pub(crate) fn scp_candidates() -> Vec<PathBuf> {
    candidates("scp")
}

fn candidates(name: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    let executable = format!("{name}.exe");
    #[cfg(not(windows))]
    let executable = name.to_string();

    #[allow(unused_mut)]
    let mut paths = vec![PathBuf::from(&executable)];

    #[cfg(windows)]
    {
        for root in ["SystemRoot", "WINDIR"] {
            if let Some(root) = std::env::var_os(root) {
                paths.push(
                    PathBuf::from(root)
                        .join("System32")
                        .join("OpenSSH")
                        .join(&executable),
                );
            }
        }

        for root in ["ProgramFiles", "ProgramW6432"] {
            if let Some(root) = std::env::var_os(root) {
                paths.push(PathBuf::from(root).join("OpenSSH").join(&executable));
            }
        }

        paths.push(
            Path::new(r"C:\Program Files")
                .join("OpenSSH")
                .join(&executable),
        );
    }

    paths
}

#[cfg(test)]
mod tests {
    use super::{scp_candidates, ssh_candidates};

    #[test]
    fn ssh_candidates_include_ssh_command_name() {
        assert!(ssh_candidates().iter().any(|path| path
            .file_name()
            .is_some_and(|name| name == "ssh" || name == "ssh.exe")));
    }

    #[test]
    fn scp_candidates_include_scp_command_name() {
        assert!(scp_candidates().iter().any(|path| path
            .file_name()
            .is_some_and(|name| name == "scp" || name == "scp.exe")));
    }
}
