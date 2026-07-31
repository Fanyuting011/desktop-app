use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use super::log_buffer::LogBuffer;
use super::profiles::{GatewayProfile, ProfilesStore};
use super::proxy::{start_local_proxies, ProxyHandles, UpstreamKind};
use super::ssh_tunnel::{remote_run_script, remote_run_shell, SshTunnel};

const OUTGATE_CLI: &str = include_str!("../../../scripts/server/outgate");
const DEPLOY_SCRIPT: &str = include_str!("../../../scripts/server/deploy-outgate.sh");

/// Shared local proxy ports (one listener; many SSH -R tunnels).
const SHARED_LOCAL_HTTP: u16 = 17890;
const SHARED_LOCAL_SOCKS: u16 = 17891;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Idle,
    Connected,
    ProxyOn,
    Reconnecting,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub profile_id: String,
    pub phase: Phase,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    pub active_profile_id: Option<String>,
    pub sessions: Vec<SessionInfo>,
    pub local_http: String,
    pub local_socks: String,
    pub upstream_proxy: Option<String>,
}

struct LiveSession {
    profile: GatewayProfile,
    tunnel: SshTunnel,
    phase: Phase,
    last_error: Option<String>,
}

pub struct GatewayState {
    inner: Arc<Mutex<Inner>>,
}

impl Clone for GatewayState {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct Inner {
    profiles: ProfilesStore,
    logs: Arc<LogBuffer>,
    sessions: HashMap<String, LiveSession>,
    /// Connect/reconnect in flight — keeps shared proxy alive before session is inserted.
    connecting: usize,
    shared_proxy: Option<ProxyHandles>,
    upstream: Option<UpstreamKind>,
    runtime: Option<tokio::runtime::Runtime>,
}

impl GatewayState {
    pub fn new(app: &AppHandle) -> Self {
        let dir = app
            .path()
            .app_config_dir()
            .unwrap_or_else(|_| PathBuf::from("."));
        let path = dir.join("gateway-profiles.json");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .ok();
        Self {
            inner: Arc::new(Mutex::new(Inner {
                profiles: ProfilesStore::load(path),
                logs: Arc::new(LogBuffer::new()),
                sessions: HashMap::new(),
                connecting: 0,
                shared_proxy: None,
                upstream: None,
                runtime,
            })),
        }
    }

    fn with_inner<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Inner) -> R,
    {
        let mut guard = self.inner.lock().expect("gateway state poisoned");
        f(&mut guard)
    }

    pub fn list_profiles(&self) -> Vec<GatewayProfile> {
        self.with_inner(|i| i.profiles.list())
    }

    pub fn upsert_profile(&self, profile: GatewayProfile) -> Result<GatewayProfile, String> {
        self.with_inner(|i| {
            if i.sessions.contains_key(&profile.id) {
                return Err("该主机已连接，请先断开再修改".to_string());
            }
            i.profiles.upsert(profile)
        })
    }

    pub fn delete_profile(&self, id: String) -> Result<(), String> {
        self.with_inner(|i| {
            if i.sessions.contains_key(&id) {
                return Err("该主机已连接，请先断开".to_string());
            }
            i.profiles.delete(&id)
        })
    }

    pub fn set_active_profile(&self, id: String) -> Result<(), String> {
        self.with_inner(|i| i.profiles.set_active(Some(id)))
    }

    pub fn status(&self) -> GatewayStatus {
        self.with_inner(|i| {
            let sessions = i
                .sessions
                .iter()
                .map(|(id, s)| SessionInfo {
                    profile_id: id.clone(),
                    phase: s.phase,
                    last_error: s.last_error.clone(),
                })
                .collect();
            let (local_http, local_socks) = if let Some(ref p) = i.shared_proxy {
                (
                    format!("http://{}", p.http_addr),
                    format!("socks5h://{}", p.socks_addr),
                )
            } else {
                (
                    format!("http://127.0.0.1:{SHARED_LOCAL_HTTP}"),
                    format!("socks5h://127.0.0.1:{SHARED_LOCAL_SOCKS}"),
                )
            };
            GatewayStatus {
                active_profile_id: i.profiles.active_id(),
                sessions,
                local_http,
                local_socks,
                upstream_proxy: i.upstream.as_ref().map(|u| u.display()),
            }
        })
    }

    pub fn get_logs(&self, limit: usize) -> Vec<String> {
        self.with_inner(|i| i.logs.snapshot(limit))
    }

    pub fn connect(
        &self,
        profile_id: Option<String>,
        upstream_proxy: Option<String>,
    ) -> Result<GatewayStatus, String> {
        let upstream = UpstreamKind::parse(upstream_proxy.as_deref().unwrap_or(""))?;

        let (profile, logs) = self.with_inner(|i| {
            let id = profile_id
                .or_else(|| i.profiles.active_id())
                .ok_or_else(|| "请先选择服务器".to_string())?;
            if i.sessions.contains_key(&id) {
                return Err("该主机已连接".to_string());
            }
            let profile = i
                .profiles
                .get(&id)
                .ok_or_else(|| "配置不存在".to_string())?;
            profile.validate()?;
            if i.runtime.is_none() {
                return Err("Tokio runtime 不可用".to_string());
            }
            let _ = i.profiles.set_active(Some(id));
            Ok((profile, i.logs.clone()))
        })?;

        // Ensure shared local proxy (hold prevents poll from stopping it mid-connect).
        {
            let mut i = self.inner.lock().expect("poisoned");
            i.connecting += 1;
            i.logs.push(format!(
                "连接服务器 {}@{}:{} ({})",
                profile.user, profile.host, profile.port, profile.name
            ));
            if i.shared_proxy.is_none() {
                if let Some(ref up) = upstream {
                    i.logs
                        .push(format!("启动共享本地代理，上游 {}", up.display()));
                    i.upstream = Some(up.clone());
                } else {
                    i.logs.push("启动共享本地代理（直连公网）".to_string());
                    i.upstream = None;
                }
                if i.runtime.is_none() {
                    i.connecting = i.connecting.saturating_sub(1);
                    return Err("Tokio runtime 不可用".to_string());
                }
                let proxy = match i.runtime.as_ref().unwrap().block_on(start_local_proxies(
                    SHARED_LOCAL_HTTP,
                    SHARED_LOCAL_SOCKS,
                    upstream.clone(),
                )) {
                    Ok(p) => p,
                    Err(e) => {
                        i.logs.push(format!("连接失败: {e}"));
                        i.connecting = i.connecting.saturating_sub(1);
                        return Err(e);
                    }
                };
                i.logs
                    .push(format!("本地代理已监听 {}", proxy.http_addr));
                i.shared_proxy = Some(proxy);
            } else if upstream.is_some()
                && i.upstream.as_ref().map(|u| u.display())
                    != upstream.as_ref().map(|u| u.display())
            {
                i.logs.push(
                    "提示: 共享代理已在运行，本次连接沿用现有上游设置".to_string(),
                );
            }
        }

        let tunnel_result = (|| {
            let _ = cleanup_remote_listen_ports(
                &profile,
                profile.remote_http_port,
                profile.remote_socks_port,
                logs.clone(),
            );

            match SshTunnel::spawn(
                &profile,
                SHARED_LOCAL_HTTP,
                SHARED_LOCAL_SOCKS,
                logs.clone(),
            ) {
                Ok(t) => Ok(t),
                Err(e)
                    if e.contains("remote port forwarding failed") || e.contains("listen port") =>
                {
                    self.with_inner(|i| {
                        i.logs
                            .push("远程端口仍被占用，尝试强制释放后重试…".to_string());
                    });
                    let _ = cleanup_remote_listen_ports(
                        &profile,
                        profile.remote_http_port,
                        profile.remote_socks_port,
                        logs.clone(),
                    );
                    thread::sleep(Duration::from_millis(500));
                    SshTunnel::spawn(
                        &profile,
                        SHARED_LOCAL_HTTP,
                        SHARED_LOCAL_SOCKS,
                        logs.clone(),
                    )
                }
                Err(e) => Err(e),
            }
        })();

        let tunnel = match tunnel_result {
            Ok(t) => t,
            Err(e) => {
                self.with_inner(|i| {
                    i.logs.push(format!("连接失败 [{}]: {e}", profile.name));
                    i.connecting = i.connecting.saturating_sub(1);
                    self_stop_proxy_if_empty(&mut *i);
                });
                return Err(e);
            }
        };

        let id = profile.id.clone();
        self.with_inner(|i| {
            i.sessions.insert(
                id.clone(),
                LiveSession {
                    profile: profile.clone(),
                    tunnel,
                    phase: Phase::Connected,
                    last_error: None,
                },
            );
            i.connecting = i.connecting.saturating_sub(1);
            i.logs
                .push(format!("SSH 隧道已建立 [{}]（可同时连接多台）", profile.name));
        });

        if let Err(e) = deploy_outgate_cli(&profile, logs) {
            self.with_inner(|i| {
                i.logs
                    .push(format!("警告: [{}] CLI 部署失败（隧道仍可用）: {e}", profile.name));
            });
        }

        Ok(self.status())
    }

    pub fn disconnect(&self, profile_id: Option<String>) -> Result<GatewayStatus, String> {
        let (profile, logs) = self.with_inner(|i| -> Result<(GatewayProfile, Arc<LogBuffer>), String> {
            let id = profile_id
                .or_else(|| i.profiles.active_id())
                .ok_or_else(|| "未指定要断开的主机".to_string())?;
            let session = i
                .sessions
                .get(&id)
                .ok_or_else(|| "该主机未连接".to_string())?;
            Ok((session.profile.clone(), i.logs.clone()))
        })?;

        let _ = remote_run_shell(
            &profile,
            "export PATH=\"$HOME/.outgate/bin:$PATH\"; outgate off >/dev/null 2>&1 || true",
            logs.clone(),
        );

        let ports = (profile.remote_http_port, profile.remote_socks_port);
        let id = profile.id.clone();
        self.with_inner(|i| {
            if let Some(mut session) = i.sessions.remove(&id) {
                session.tunnel.kill();
                i.logs
                    .push(format!("已断开 [{}]", session.profile.name));
            }
            self_stop_proxy_if_empty(&mut *i);
        });

        let logs2 = self.with_inner(|i| i.logs.clone());
        let _ = cleanup_remote_listen_ports(&profile, ports.0, ports.1, logs2);

        Ok(self.status())
    }

    pub fn poll_and_maybe_reconnect(&self) -> Result<GatewayStatus, String> {
        let mut to_reconnect: Vec<GatewayProfile> = Vec::new();

        self.with_inner(|i| {
            let ids: Vec<String> = i.sessions.keys().cloned().collect();
            for id in ids {
                let dead = {
                    let session = i.sessions.get_mut(&id).unwrap();
                    match session.tunnel.try_wait() {
                        Ok(None) => false,
                        Ok(Some(code)) => {
                            i.logs.push(format!(
                                "[{}] SSH 隧道退出 (code={code:?})",
                                session.profile.name
                            ));
                            true
                        }
                        Err(e) => {
                            session.last_error = Some(e);
                            false
                        }
                    }
                };
                if dead {
                    let (name, auto) = {
                        let s = i.sessions.get(&id).unwrap();
                        (s.profile.name.clone(), s.profile.auto_reconnect)
                    };
                    if auto {
                        if let Some(session) = i.sessions.get_mut(&id) {
                            session.phase = Phase::Reconnecting;
                            to_reconnect.push(session.profile.clone());
                        }
                        i.logs.push(format!("[{name}] 将自动重连…"));
                    } else {
                        i.sessions.remove(&id);
                    }
                }
            }
            self_stop_proxy_if_empty(&mut *i);
        });

        for profile in to_reconnect {
            thread::sleep(Duration::from_secs(2));
            if let Err(e) = self.reconnect_one(profile.clone()) {
                self.with_inner(|i| {
                    i.logs
                        .push(format!("[{}] 重连失败: {e}", profile.name));
                });
            }
        }

        Ok(self.status())
    }

    fn reconnect_one(&self, profile: GatewayProfile) -> Result<(), String> {
        let logs = self.with_inner(|i| {
            i.connecting += 1;
            if i.shared_proxy.is_none() {
                let upstream = i.upstream.clone();
                if i.runtime.is_none() {
                    i.connecting = i.connecting.saturating_sub(1);
                    return Err("Tokio runtime 不可用".to_string());
                }
                let proxy = match i.runtime.as_ref().unwrap().block_on(start_local_proxies(
                    SHARED_LOCAL_HTTP,
                    SHARED_LOCAL_SOCKS,
                    upstream,
                )) {
                    Ok(p) => p,
                    Err(e) => {
                        i.connecting = i.connecting.saturating_sub(1);
                        return Err(e);
                    }
                };
                i.shared_proxy.replace(proxy);
            }
            Ok::<_, String>(i.logs.clone())
        })?;

        let tunnel = match SshTunnel::spawn(
            &profile,
            SHARED_LOCAL_HTTP,
            SHARED_LOCAL_SOCKS,
            logs,
        ) {
            Ok(t) => t,
            Err(e) => {
                self.with_inner(|i| {
                    i.connecting = i.connecting.saturating_sub(1);
                    self_stop_proxy_if_empty(&mut *i);
                });
                return Err(e);
            }
        };

        self.with_inner(|i| {
            let name = profile.name.clone();
            i.sessions.insert(
                profile.id.clone(),
                LiveSession {
                    profile,
                    tunnel,
                    phase: Phase::Connected,
                    last_error: None,
                },
            );
            i.connecting = i.connecting.saturating_sub(1);
            i.logs.push(format!("[{name}] 重连成功"));
        });
        Ok(())
    }

    pub fn set_reconnect(&self, profile_id: Option<String>, enabled: bool) -> Result<(), String> {
        self.with_inner(|i| {
            let id = profile_id
                .or_else(|| i.profiles.active_id())
                .ok_or_else(|| "未选择主机".to_string())?;
            if let Some(session) = i.sessions.get_mut(&id) {
                session.profile.auto_reconnect = enabled;
            }
            if let Some(mut p) = i.profiles.get(&id) {
                p.auto_reconnect = enabled;
                let _ = i.profiles.upsert(p);
            }
            Ok(())
        })
    }
}

fn self_stop_proxy_if_empty(i: &mut Inner) {
    if i.sessions.is_empty() && i.connecting == 0 {
        if let Some(proxy) = i.shared_proxy.take() {
            proxy.stop();
            i.logs.push("所有隧道已断开，已停止本地代理".to_string());
        }
    }
}

fn cleanup_remote_listen_ports(
    profile: &GatewayProfile,
    http_port: u16,
    socks_port: u16,
    logs: Arc<LogBuffer>,
) -> Result<(), String> {
    let cmd = format!(
        "set +e; for p in {http_port} {socks_port}; do \
command -v fuser >/dev/null 2>&1 && fuser -k ${{p}}/tcp >/dev/null 2>&1; \
if command -v lsof >/dev/null 2>&1; then pids=$(lsof -t -iTCP:$p -sTCP:LISTEN 2>/dev/null); [ -n \"$pids\" ] && kill $pids >/dev/null 2>&1; fi; \
command -v ss >/dev/null 2>&1 && ss -K sport = :$p >/dev/null 2>&1; \
done; sleep 0.2; echo cleaned:{http_port},{socks_port}"
    );
    remote_run_shell(profile, &cmd, logs).map(|_| ())
}

fn deploy_outgate_cli(profile: &GatewayProfile, logs: Arc<LogBuffer>) -> Result<(), String> {
    logs.push(format!(
        "[{}] 部署 OutGate CLI → ~/.outgate/bin",
        profile.name
    ));
    let b64 = base64_encode_local(OUTGATE_CLI.as_bytes());
    let http = format!("http://127.0.0.1:{}", profile.remote_http_port);
    let socks = format!("socks5h://127.0.0.1:{}", profile.remote_socks_port);
    let no_proxy = profile.no_proxy.join(",");
    let env = [
        ("OUTGATE_B64", b64),
        ("OUTGATE_HTTP", http),
        ("OUTGATE_SOCKS", socks),
        ("OUTGATE_NO_PROXY", no_proxy),
    ];
    remote_run_script(profile, DEPLOY_SCRIPT, &env, logs)?;
    Ok(())
}

fn base64_encode_local(data: &[u8]) -> String {
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
