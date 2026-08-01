use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Temporary SSH_ASKPASS helper so OpenSSH can take a password non-interactively.
pub struct AskpassEnv {
    pub script_path: PathBuf,
    _password_file: PathBuf,
}

impl AskpassEnv {
    pub fn setup(password: &str) -> Result<Self, String> {
        let dir = std::env::temp_dir().join(format!(
            "gateway-askpass-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).map_err(|e| format!("创建 askpass 目录失败: {e}"))?;

        let password_file = dir.join("password.txt");
        fs::write(&password_file, password)
            .map_err(|e| format!("写入临时密码文件失败: {e}"))?;

        #[cfg(windows)]
        let script_path = {
            let path = dir.join("askpass.cmd");
            // Read password file and print without trailing spaces; keep exact password bytes as UTF-8 text.
            let pf = password_file.display().to_string().replace('/', "\\");
            let content = format!(
                "@echo off\r\npowershell -NoProfile -Command \"[IO.File]::ReadAllText('{pf}').TrimEnd([char]13,[char]10)\"\r\n"
            );
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
                let mut pperms = fs::metadata(&password_file)
                    .map_err(|e| e.to_string())?
                    .permissions();
                pperms.set_mode(0o600);
                fs::set_permissions(&password_file, pperms).map_err(|e| e.to_string())?;
            }
            path
        };

        Ok(Self {
            script_path,
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

impl Drop for AskpassEnv {
    fn drop(&mut self) {
        if let Some(dir) = self.script_path.parent() {
            let _ = fs::remove_dir_all(dir);
        }
    }
}
