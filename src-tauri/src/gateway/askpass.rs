use std::fs;
use std::path::PathBuf;
use std::process::Command;

use uuid::Uuid;

/// Temporary SSH_ASKPASS helper so OpenSSH can take a password non-interactively.
pub struct AskpassEnv {
    pub script_path: PathBuf,
    /// Private to this instance — several `AskpassEnv`s can be alive at once (one per
    /// host session, plus one per interactive terminal), so each owns a uniquely named
    /// directory that only it ever writes to or removes.
    dir: PathBuf,
    _password_file: PathBuf,
}

impl AskpassEnv {
    pub fn setup(password: &str) -> Result<Self, String> {
        let dir = std::env::temp_dir().join(format!(
            "gateway-askpass-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).map_err(|e| format!("创建 askpass 目录失败: {e}"))?;

        let password_file = dir.join("password.txt");
        write_password_file(&password_file, password)?;

        #[cfg(windows)]
        let script_path = {
            let path = dir.join("askpass.cmd");
            // Avoid PowerShell here — each ASKPASS invocation would flash a console window
            // during connect (auth probe, tunnel, deploy, cleanup). `type` is enough.
            let pf = password_file.display().to_string().replace('/', "\\");
            let content = format!("@echo off\r\ntype \"{pf}\"\r\n");
            fs::write(&path, content).map_err(|e| format!("写入 askpass 脚本失败: {e}"))?;
            path
        };

        #[cfg(not(windows))]
        let script_path = {
            let path = dir.join("askpass.sh");
            let pf = password_file.display();
            let content = format!("#!/bin/sh\ncat '{pf}'\n");
            fs::write(&path, content).map_err(|e| format!("写入 askpass 脚本失败: {e}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&path)
                    .map_err(|e| e.to_string())?
                    .permissions();
                perms.set_mode(0o700);
                fs::set_permissions(&path, perms).map_err(|e| e.to_string())?;
            }
            path
        };

        Ok(Self {
            script_path,
            dir,
            _password_file: password_file,
        })
    }

    pub fn apply(&self, cmd: &mut Command) {
        for (k, v) in self.env_pairs() {
            cmd.env(k, v);
        }
    }

    /// Same environment variables as [`Self::apply`], as plain string pairs — used by
    /// callers that build commands via a non-`std::process::Command` API (e.g.
    /// `portable_pty::CommandBuilder` for the interactive terminal).
    pub fn env_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = vec![
            (
                "SSH_ASKPASS".to_string(),
                self.script_path.display().to_string(),
            ),
            ("SSH_ASKPASS_REQUIRE".to_string(), "force".to_string()),
        ];
        // Some OpenSSH builds still check DISPLAY before honoring ASKPASS.
        if std::env::var_os("DISPLAY").is_none() {
            pairs.push(("DISPLAY".to_string(), "1".to_string()));
        }
        pairs
    }
}

/// Create the password file with owner-only permissions from the start on unix, so the
/// secret is never briefly readable by other users between `write` and `set_permissions`.
fn write_password_file(path: &PathBuf, password: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("写入临时密码文件失败: {e}"))?;
        file.write_all(password.as_bytes())
            .map_err(|e| format!("写入临时密码文件失败: {e}"))
    }
    #[cfg(not(unix))]
    {
        fs::write(path, password).map_err(|e| format!("写入临时密码文件失败: {e}"))
    }
}

impl Drop for AskpassEnv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::AskpassEnv;

    #[test]
    fn each_instance_owns_a_private_directory() {
        let a = AskpassEnv::setup("secret-a").unwrap();
        let b = AskpassEnv::setup("secret-b").unwrap();
        assert_ne!(a.dir, b.dir);

        // Dropping one must not take the other's password file with it.
        let b_password = b._password_file.clone();
        drop(a);
        assert!(b_password.exists());
        assert_eq!(std::fs::read_to_string(&b_password).unwrap(), "secret-b");

        let b_dir = b.dir.clone();
        drop(b);
        assert!(!b_dir.exists());
    }

    #[cfg(unix)]
    #[test]
    fn password_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let env = AskpassEnv::setup("secret").unwrap();
        let mode = std::fs::metadata(&env._password_file)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
