use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub identity_file: String,
    /// Optional SSH password (stored in local profile JSON; for lab/intranet use).
    #[serde(default)]
    pub password: String,
    pub remote_http_port: u16,
    pub remote_socks_port: u16,
    pub auto_reconnect: bool,
    pub no_proxy: Vec<String>,
    pub updated_at: String,
}

impl GatewayProfile {
    pub fn default_no_proxy() -> Vec<String> {
        vec![
            "127.0.0.1".into(),
            "localhost".into(),
            "::1".into(),
        ]
    }

    pub fn new_blank(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            host: String::new(),
            port: 22,
            user: String::new(),
            identity_file: String::new(),
            password: String::new(),
            remote_http_port: 17890,
            remote_socks_port: 17891,
            auto_reconnect: true,
            no_proxy: Self::default_no_proxy(),
            updated_at: chrono_like_now(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("名称不能为空".to_string());
        }
        if self.host.trim().is_empty() {
            return Err("主机不能为空".to_string());
        }
        if self.user.trim().is_empty() {
            return Err("用户名不能为空".to_string());
        }
        if self.port == 0 {
            return Err("端口无效".to_string());
        }
        if self.identity_file.trim().is_empty() && self.password.trim().is_empty() {
            return Err("请填写私钥路径或密码（至少一种）".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ProfilesFile {
    active_profile_id: Option<String>,
    profiles: Vec<GatewayProfile>,
}

pub struct ProfilesStore {
    path: PathBuf,
    data: ProfilesFile,
}

impl ProfilesStore {
    pub fn load(path: PathBuf) -> Self {
        let data = if path.exists() {
            match fs::read_to_string(&path) {
                Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
                Err(_) => ProfilesFile::default(),
            }
        } else {
            ProfilesFile::default()
        };
        Self { path, data }
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let raw = serde_json::to_string_pretty(&self.data).map_err(|e| e.to_string())?;
        fs::write(&self.path, raw).map_err(|e| e.to_string())
    }

    pub fn list(&self) -> Vec<GatewayProfile> {
        self.data.profiles.clone()
    }

    pub fn active_id(&self) -> Option<String> {
        self.data.active_profile_id.clone()
    }

    pub fn get(&self, id: &str) -> Option<GatewayProfile> {
        self.data.profiles.iter().find(|p| p.id == id).cloned()
    }

    pub fn active(&self) -> Option<GatewayProfile> {
        self.data
            .active_profile_id
            .as_ref()
            .and_then(|id| self.get(id))
    }

    pub fn set_active(&mut self, id: Option<String>) -> Result<(), String> {
        if let Some(ref pid) = id {
            if self.get(pid).is_none() {
                return Err("配置不存在".to_string());
            }
        }
        self.data.active_profile_id = id;
        self.save()
    }

    pub fn upsert(&mut self, mut profile: GatewayProfile) -> Result<GatewayProfile, String> {
        profile.validate()?;
        profile.updated_at = chrono_like_now();
        if let Some(existing) = self
            .data
            .profiles
            .iter_mut()
            .find(|p| p.id == profile.id)
        {
            *existing = profile.clone();
        } else {
            if self.data.active_profile_id.is_none() {
                self.data.active_profile_id = Some(profile.id.clone());
            }
            self.data.profiles.push(profile.clone());
        }
        self.save()?;
        Ok(profile)
    }

    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        let before = self.data.profiles.len();
        self.data.profiles.retain(|p| p.id != id);
        if self.data.profiles.len() == before {
            return Err("配置不存在".to_string());
        }
        if self.data.active_profile_id.as_deref() == Some(id) {
            self.data.active_profile_id = self.data.profiles.first().map(|p| p.id.clone());
        }
        self.save()
    }
}

fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}
