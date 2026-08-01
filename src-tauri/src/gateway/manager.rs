use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::log_buffer::LogBuffer;
use super::network_log::{NetworkLogBuffer, NetworkLogEntry};
use super::port_alloc::allocate_port_pair;
use super::profiles::{apply_preset, GatewayProfile, ProfilesStore};
use super::proxy::{start_local_proxies, ProxyHandles, UpstreamKind};
use super::ssh_tunnel::{remote_run_script, remote_run_shell, SshTunnel};
use super::terminal::TerminalHub;
use super::transfer::{run_download, run_upload};

const OUTGATE_CLI: &str = include_str!("../../../scripts/server/outgate");
const DEPLOY_SCRIPT: &str = include_str!("../../../scripts/server/deploy-outgate.sh");

/// Base port for per-session local proxy allocation (each host session gets its own pair).
const DEFAULT_BASE_HTTP_PORT: u16 = 17890;

/// Placeholder shown when no session is connected under the active profile.
const NO_PROXY_PLACEHOLDER: &str = "未连接";

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
    pub local_http_port: u16,
    pub local_socks_port: u16,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferStatus {
    pub profile_id: Option<String>,
    pub state: String,
    pub detail: Option<String>,
}

struct LiveSession {
    profile: GatewayProfile,
    tunnel: SshTunnel,
    /// Dedicated local proxy for this host — each session binds its own ports.
    proxy: ProxyHandles,
    local_http: u16,
    local_socks: u16,
    /// Monotonic session token used to discard stale tunnel spawn completions.
    generation: u64,
    /// An auto-reconnect closed an interactive terminal that must be recreated once a
    /// replacement data-plane tunnel has been installed.
    terminal_reopen_required: bool,
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
    /// Kept so background paths that have no command-invocation `AppHandle` of their own
    /// (auto-reconnect) can still reopen the PTY and emit frontend events.
    app: AppHandle,
    profiles: ProfilesStore,
    logs: Arc<LogBuffer>,
    network_logs: Arc<NetworkLogBuffer>,
    sessions: HashMap<String, LiveSession>,
    /// Profile ids with a `connect()` in flight — guards against a second concurrent
    /// `connect()` for the same host starting a duplicate proxy/tunnel before the first
    /// one has inserted its session (see `PendingGuard`).
    pending: HashSet<String>,
    /// Profile ids whose tunnel is currently being respawned by `reconnect_one`. A dead
    /// tunnel keeps reporting dead from `try_wait()` until it is replaced, so without this
    /// guard every 3s poll would enqueue yet another reconnect on top of the in-flight one.
    reconnecting: HashMap<String, u64>,
    /// Profile ids with an SCP process in flight, mapped to a human-readable description of
    /// what is being transferred (e.g. "上传 → ~/uploads"). A profile may only run one
    /// transfer at a time, while transfers to separate connected profiles remain independent.
    transfer_busy: HashMap<String, String>,
    /// Never reset across disconnect/reconnect, so a stale task cannot match a new session
    /// that reuses the same profile id.
    next_generation: u64,
    upstream: Option<UpstreamKind>,
    runtime: Option<tokio::runtime::Runtime>,
    /// Interactive PTY-backed `ssh` sessions for the embedded terminal — independent of
    /// `sessions`' `-N -R` data-plane tunnels. Wrapped in its own `Arc` (it manages its
    /// own internal locking) so background reader threads never need to touch the
    /// gateway-wide `Inner` mutex.
    terminals: Arc<TerminalHub>,
}

/// RAII guard that removes a profile id from `Inner::pending` once `connect()` finishes,
/// whether it succeeds or bails out early (error return, `?`, panic-free early return, etc).
struct PendingGuard {
    state: GatewayState,
    id: String,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.state.with_inner(|i| {
            i.pending.remove(&self.id);
        });
    }
}

/// Releases a profile's transfer reservation even when launching or running `scp` fails.
struct TransferGuard {
    state: GatewayState,
    id: String,
}

impl Drop for TransferGuard {
    fn drop(&mut self) {
        self.state.with_inner(|i| {
            i.transfer_busy.remove(&self.id);
        });
    }
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
                app: app.clone(),
                profiles: ProfilesStore::load(path),
                logs: Arc::new(LogBuffer::new()),
                network_logs: Arc::new(NetworkLogBuffer::new()),
                sessions: HashMap::new(),
                pending: HashSet::new(),
                reconnecting: HashMap::new(),
                transfer_busy: HashMap::new(),
                next_generation: 0,
                upstream: None,
                runtime,
                terminals: Arc::new(TerminalHub::new()),
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
                    local_http_port: s.local_http,
                    local_socks_port: s.local_socks,
                })
                .collect();
            // Each host session owns its own local proxy ports; surface the active
            // profile's ports here (falls back to a placeholder when nothing is connected).
            let (local_http, local_socks) = i
                .profiles
                .active_id()
                .and_then(|id| i.sessions.get(&id))
                .map(|s| {
                    (
                        format!("http://127.0.0.1:{}", s.local_http),
                        format!("socks5h://127.0.0.1:{}", s.local_socks),
                    )
                })
                .unwrap_or_else(|| {
                    (
                        NO_PROXY_PLACEHOLDER.to_string(),
                        NO_PROXY_PLACEHOLDER.to_string(),
                    )
                });
            GatewayStatus {
                active_profile_id: i.profiles.active_id(),
                sessions,
                local_http,
                local_socks,
                upstream_proxy: i.upstream.as_ref().map(|u| u.display()),
            }
        })
    }

    pub fn get_logs(&self, limit: usize, profile_id: Option<String>) -> Vec<String> {
        self.with_inner(|i| match profile_id {
            None => i.logs.snapshot(limit),
            Some(profile_id) => {
                let profile_name = i.profiles.get(&profile_id).map(|profile| profile.name);
                i.logs
                    .snapshot_filtered(limit, &profile_id, profile_name.as_deref())
            }
        })
    }

    pub fn get_network_logs(
        &self,
        profile_id: Option<String>,
        limit: usize,
    ) -> Vec<NetworkLogEntry> {
        self.with_inner(|i| i.network_logs.snapshot(profile_id.as_deref(), limit))
    }

    pub fn clear_network_logs(&self, profile_id: Option<String>) {
        self.with_inner(|i| i.network_logs.clear(profile_id.as_deref()));
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
            // Check-and-reserve under a single lock: `sessions` catches an already-connected
            // host, `pending` catches a second concurrent `connect()` for the same host that
            // hasn't inserted its session yet (both must be checked before either is set).
            if i.sessions.contains_key(&id) || i.pending.contains(&id) {
                return Err("该主机已连接或正在连接中".to_string());
            }
            let profile = i
                .profiles
                .get(&id)
                .ok_or_else(|| "配置不存在".to_string())?;
            profile.validate()?;
            if i.runtime.is_none() {
                return Err("Tokio runtime 不可用".to_string());
            }
            let _ = i.profiles.set_active(Some(id.clone()));
            i.pending.insert(id);
            Ok((profile, i.logs.clone()))
        })?;

        // Held for the remainder of this call so that any early return (error or success)
        // releases the `pending` reservation exactly once, including proxy/tunnel cleanup
        // paths below that run before the session is actually inserted.
        let _pending_guard = PendingGuard {
            state: self.clone(),
            id: profile.id.clone(),
        };

        // Allocate a dedicated local proxy for this session and start it.
        let (proxy, local_http, local_socks) = {
            let mut i = self.inner.lock().expect("poisoned");
            i.logs.push(format!(
                "连接服务器 {}@{}:{} ({})",
                profile.user, profile.host, profile.port, profile.name
            ));
            if i.sessions.is_empty() {
                if let Some(ref up) = upstream {
                    i.logs.push(format!("启动本地代理，上游 {}", up.display()));
                } else {
                    i.logs.push("启动本地代理（直连公网）".to_string());
                }
                i.upstream = upstream.clone();
            } else if upstream.is_some()
                && i.upstream.as_ref().map(|u| u.display())
                    != upstream.as_ref().map(|u| u.display())
            {
                i.logs
                    .push("提示: 已有主机连接中，本次连接沿用现有上游设置".to_string());
            }

            let used = collect_used_ports(&i.sessions);
            let (local_http, local_socks) = match allocate_port_pair(&used, DEFAULT_BASE_HTTP_PORT)
            {
                Ok(pair) => pair,
                Err(e) => {
                    i.logs.push(format!("连接失败: {e}"));
                    return Err(e);
                }
            };

            if i.runtime.is_none() {
                return Err("Tokio runtime 不可用".to_string());
            }
            let resolved_upstream = i.upstream.clone();
            let net_log = i.network_logs.clone();
            let proxy = match i.runtime.as_ref().unwrap().block_on(start_local_proxies(
                local_http,
                local_socks,
                resolved_upstream,
                profile.id.clone(),
                net_log,
            )) {
                Ok(p) => p,
                Err(e) => {
                    i.logs.push(format!("连接失败: {e}"));
                    return Err(e);
                }
            };
            i.logs.push(format!(
                "[{}] 本地代理已监听 {}",
                profile.name, proxy.http_addr
            ));
            (proxy, local_http, local_socks)
        };

        let tunnel_result = (|| {
            let _ = cleanup_remote_listen_ports(
                &profile,
                profile.remote_http_port,
                profile.remote_socks_port,
                logs.clone(),
            );

            match SshTunnel::spawn(&profile, local_http, local_socks, logs.clone()) {
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
                    SshTunnel::spawn(&profile, local_http, local_socks, logs.clone())
                }
                Err(e) => Err(e),
            }
        })();

        let tunnel = match tunnel_result {
            Ok(t) => t,
            Err(e) => {
                proxy.stop();
                self.with_inner(|i| {
                    i.logs.push(format!("连接失败 [{}]: {e}", profile.name));
                });
                return Err(e);
            }
        };

        let id = profile.id.clone();
        self.with_inner(|i| {
            i.next_generation += 1;
            let generation = i.next_generation;
            i.reconnecting.remove(&id);
            i.sessions.insert(
                id.clone(),
                LiveSession {
                    profile: profile.clone(),
                    tunnel,
                    proxy,
                    local_http,
                    local_socks,
                    generation,
                    terminal_reopen_required: false,
                    phase: Phase::Connected,
                    last_error: None,
                },
            );
            i.logs.push(format!(
                "SSH 隧道已建立 [{}]（本地端口 {local_http}/{local_socks}，可同时连接多台）",
                profile.name
            ));
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
        let (profile, logs) =
            self.with_inner(|i| -> Result<(GatewayProfile, Arc<LogBuffer>), String> {
                let id = profile_id
                    .or_else(|| i.profiles.active_id())
                    .ok_or_else(|| "未指定要断开的主机".to_string())?;
                let profile = i
                    .sessions
                    .get(&id)
                    .ok_or_else(|| "该主机未连接".to_string())?
                    .profile
                    .clone();
                // Invalidate every in-flight tunnel/terminal recovery before releasing
                // the lock to tear down the session.
                i.next_generation += 1;
                let generation = i.next_generation;
                if let Some(session) = i.sessions.get_mut(&id) {
                    session.generation = generation;
                }
                i.reconnecting.insert(id, generation);
                Ok((profile, i.logs.clone()))
            })?;

        // Kill any interactive terminal for this host before tearing down the tunnel —
        // the terminal is a second, independent ssh process that would otherwise be
        // left dangling once the profile shows as disconnected.
        self.terminal_close(profile.id.clone());

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
                session.proxy.stop();
                i.logs
                    .push(format!("已断开 [{}]", session.profile.name));
            }
            i.reconnecting.remove(&id);
        });

        let logs2 = self.with_inner(|i| i.logs.clone());
        let _ = cleanup_remote_listen_ports(&profile, ports.0, ports.1, logs2);

        Ok(self.status())
    }

    pub fn poll_and_maybe_reconnect(&self) -> Result<GatewayStatus, String> {
        // (profile id, generation, whether an interactive terminal was open before the tunnel died)
        let mut to_reconnect: Vec<(String, u64, bool)> = Vec::new();

        self.with_inner(|i| {
            let ids: Vec<String> = i.sessions.keys().cloned().collect();
            for id in ids {
                // A reconnect already in flight owns this session's tunnel: its old handle
                // still reports dead, so re-running the dead path here would kill the
                // terminal again and queue duplicate reconnects.
                if i.reconnecting.contains_key(&id) {
                    continue;
                }
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
                    // The interactive terminal (if open) is a second, independent ssh
                    // process tied to the same host. Once the tunnel itself has died,
                    // close it too — whether we're about to auto-reconnect (in which case
                    // `reconnect_one` reopens it) or dropping the session entirely — so it
                    // never leaks as an orphaned ssh process.
                    let had_terminal = i.terminals.close(&id);

                    let (name, auto) = {
                        let s = i.sessions.get(&id).unwrap();
                        (s.profile.name.clone(), s.profile.auto_reconnect)
                    };
                    if auto {
                        // Keep the session (and its local proxy) alive across reconnect —
                        // only the SSH tunnel needs to be respawned.
                        i.next_generation += 1;
                        let generation = i.next_generation;
                        if let Some(session) = i.sessions.get_mut(&id) {
                            session.phase = Phase::Reconnecting;
                            session.generation = generation;
                            session.terminal_reopen_required |= had_terminal;
                            to_reconnect.push((id.clone(), generation, had_terminal));
                            i.reconnecting.insert(id.clone(), generation);
                        }
                        i.logs.push(format!("[{name}] 将自动重连…"));
                    } else if let Some(session) = i.sessions.remove(&id) {
                        session.proxy.stop();
                        i.logs.push(format!("[{name}] 已停止本地代理"));
                    }
                }
            }
        });

        for (profile_id, generation, reopen_terminal) in to_reconnect {
            thread::sleep(Duration::from_secs(2));
            let result = self.reconnect_one(profile_id.clone(), generation, reopen_terminal);
            self.with_inner(|i| {
                if i.reconnecting.get(&profile_id) == Some(&generation) {
                    i.reconnecting.remove(&profile_id);
                }
                if let Err(e) = &result {
                    i.logs.push(format!("[{profile_id}] 重连失败: {e}"));
                }
            });
        }

        Ok(self.status())
    }

    fn reconnect_one(
        &self,
        id: String,
        generation: u64,
        reopen_terminal: bool,
    ) -> Result<(), String> {
        // Fetch the profile only after checking the generation so a preset update cannot
        // make this reconnect spawn with an obsolete captured profile.
        let (profile, local_http, local_socks, logs) = self.with_inner(|i| {
            let session = i
                .sessions
                .get(&id)
                .ok_or_else(|| "会话已丢失，无法重连".to_string())?;
            if session.generation != generation {
                return Err("重连任务已过期".to_string());
            }
            Ok::<_, String>((
                session.profile.clone(),
                session.local_http,
                session.local_socks,
                i.logs.clone(),
            ))
        })?;

        let mut tunnel = Some(SshTunnel::spawn(&profile, local_http, local_socks, logs)?);

        let install = self.with_inner(|i| {
            let Some(session) = i.sessions.get_mut(&id) else {
                return None;
            };
            if session.generation != generation {
                return None;
            }
            session.tunnel = tunnel.take().expect("tunnel is available for installation");
            session.phase = Phase::Connected;
            session.last_error = None;
            let reopen_terminal = session.terminal_reopen_required || reopen_terminal;
            session.terminal_reopen_required = false;
            i.logs.push(format!("[{}] 重连成功", profile.name));
            Some((i.app.clone(), i.terminals.clone(), reopen_terminal))
        });
        let Some((app, terminals, reopen_terminal)) = install else {
            if let Some(mut tunnel) = tunnel {
                tunnel.kill();
            }
            return Err("重连任务已过期或会话已断开".to_string());
        };

        // The terminal's ssh process was killed along with the dead tunnel; bring it back
        // so the still-mounted tab becomes usable again instead of staying frozen. The
        // event tells the frontend to reset its xterm buffer and re-arm the `outgate on`
        // injection for the fresh shell.
        if reopen_terminal {
            match terminals.open(app.clone(), &profile) {
                // Emitted only once the new PTY exists, so the resize the frontend sends
                // back in response lands on the live session instead of being rejected.
                Ok(()) => {
                    let _ = app.emit(&format!("terminal-reconnect-{id}"), ());
                }
                Err(e) => self.with_inner(|i| {
                    i.logs
                        .push(format!("[{}] 重连后恢复终端失败: {e}", profile.name));
                }),
            }
        }
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

    pub fn set_port_forward_preset(
        &self,
        profile_id: String,
        port: u16,
        enabled: bool,
    ) -> Result<GatewayStatus, String> {
        if !matches!(port, 3000 | 8080 | 5432) {
            return Err(format!("不支持端口 {port}；仅支持 3000、8080、5432"));
        }

        let (profile, respawn_generation) = self.with_inner(|i| {
            let mut profile = i
                .profiles
                .get(&profile_id)
                .ok_or_else(|| "配置不存在".to_string())?;
            if !apply_preset(&mut profile.port_forwards, port, enabled) {
                return Ok::<_, String>((profile, None));
            }
            let profile = i.profiles.upsert(profile)?;

            let should_respawn = i
                .sessions
                .get(&profile_id)
                .map(|session| matches!(session.phase, Phase::Connected | Phase::Reconnecting))
                .unwrap_or(false);
            if let Some(session) = i.sessions.get_mut(&profile_id) {
                session.profile = profile.clone();
            }
            let respawn_generation = if should_respawn {
                i.next_generation += 1;
                let generation = i.next_generation;
                if let Some(session) = i.sessions.get_mut(&profile_id) {
                    session.generation = generation;
                }
                i.reconnecting.insert(profile_id.clone(), generation);
                Some(generation)
            } else {
                None
            };
            Ok::<_, String>((profile, respawn_generation))
        })?;

        if let Some(generation) = respawn_generation {
            self.respawn_tunnel_keep_proxy(&profile, generation, Some((port, enabled)))?;
        }
        Ok(self.status())
    }

    /// `rollback_preset`, when set, is the `(port, enabled)` toggle that was just applied to
    /// reach `profile`. If the respawned tunnel fails to come up, that toggle is reverted
    /// (persisted store + in-memory session profile) before returning the error, so the
    /// profile never keeps a rule that is known to prevent the tunnel from starting — without
    /// this, the dead tunnel would keep tripping `poll_and_maybe_reconnect`'s auto-reconnect
    /// with the same broken rule every 3s, forever (see review I2).
    fn respawn_tunnel_keep_proxy(
        &self,
        profile: &GatewayProfile,
        generation: u64,
        rollback_preset: Option<(u16, bool)>,
    ) -> Result<(), String> {
        let id = profile.id.clone();
        let (latest_profile, local_http, local_socks, logs) = self.with_inner(|i| {
            let session = i
                .sessions
                .get_mut(&id)
                .ok_or_else(|| "会话已丢失，无法重建隧道".to_string())?;
            if session.generation != generation {
                return Err("端口转发任务已过期".to_string());
            }
            session.tunnel.kill();
            let latest_profile = session.profile.clone();
            let local_http = session.local_http;
            let local_socks = session.local_socks;
            i.reconnecting.insert(id.clone(), generation);
            Ok::<_, String>((latest_profile, local_http, local_socks, i.logs.clone()))
        })?;

        let tunnel = SshTunnel::spawn(&latest_profile, local_http, local_socks, logs);
        let (reopen_terminal, app, terminals) = self.with_inner(|i| {
            if i.reconnecting.get(&id) == Some(&generation) {
                i.reconnecting.remove(&id);
            }
            match tunnel {
                Ok(tunnel) => {
                    let mut tunnel = Some(tunnel);
                    let reopen_terminal = if let Some(session) = i.sessions.get_mut(&id) {
                        if session.generation != generation {
                            None
                        } else {
                            session.tunnel =
                                tunnel.take().expect("tunnel is available for installation");
                            session.phase = Phase::Connected;
                            session.last_error = None;
                            let reopen_terminal = session.terminal_reopen_required;
                            Some(reopen_terminal)
                        }
                    } else {
                        None
                    };
                    if let Some(reopen_terminal) = reopen_terminal {
                        i.logs
                            .push(format!("[{}] 已应用端口转发预设", latest_profile.name));
                        Ok((reopen_terminal, i.app.clone(), i.terminals.clone()))
                    } else {
                        if let Some(mut tunnel) = tunnel {
                            tunnel.kill();
                        }
                        Err("端口转发任务已过期或会话已断开".to_string())
                    }
                }
                Err(error) => {
                    let current = i
                        .sessions
                        .get(&id)
                        .map(|session| session.generation == generation)
                        .unwrap_or(false);
                    if current {
                        if let Some((port, enabled)) = rollback_preset {
                            // Revert the toggle that led to this failed respawn so the
                            // persisted profile (and the live session's copy of it) never
                            // keeps a rule that is known to prevent the tunnel from starting.
                            if let Some(mut stored) = i.profiles.get(&id) {
                                if apply_preset(&mut stored.port_forwards, port, !enabled) {
                                    match i.profiles.upsert(stored) {
                                        Ok(saved) => {
                                            if let Some(session) = i.sessions.get_mut(&id) {
                                                session.profile = saved;
                                            }
                                            i.logs.push(format!(
                                                "[{}] 端口 {port} 转发导致隧道失败，已回滚该预设",
                                                latest_profile.name
                                            ));
                                        }
                                        Err(save_err) => {
                                            i.logs.push(format!(
                                                "[{}] 回滚端口 {port} 预设失败: {save_err}",
                                                latest_profile.name
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(session) = i.sessions.get_mut(&id) {
                            session.last_error = Some(error.clone());
                        }
                        i.logs.push(format!(
                            "[{}] 重建端口转发隧道失败: {error}",
                            latest_profile.name
                        ));
                        Err(error)
                    } else {
                        Err("端口转发任务已过期或会话已断开".to_string())
                    }
                }
            }
        })?;

        if reopen_terminal {
            let recovery_is_current = self.with_inner(|i| {
                i.sessions
                    .get(&id)
                    .map(|session| {
                        session.generation == generation && session.terminal_reopen_required
                    })
                    .unwrap_or(false)
            });
            if !recovery_is_current {
                return Ok(());
            }

            match terminals.open(app.clone(), &latest_profile) {
                Ok(()) => {
                    let recovery_is_current = self.with_inner(|i| {
                        let Some(session) = i.sessions.get_mut(&id) else {
                            return false;
                        };
                        if session.generation != generation || !session.terminal_reopen_required {
                            return false;
                        }
                        session.terminal_reopen_required = false;
                        true
                    });
                    if recovery_is_current {
                        let _ = app.emit(&format!("terminal-reconnect-{id}"), ());
                    } else {
                        terminals.close(&id);
                    }
                }
                Err(error) => self.with_inner(|i| {
                    if let Some(session) = i.sessions.get_mut(&id) {
                        if session.generation == generation {
                            session.terminal_reopen_required = true;
                        }
                    }
                    i.logs.push(format!(
                        "[{}] 应用端口转发预设后恢复终端失败: {error}",
                        latest_profile.name
                    ));
                }),
            }
        }
        Ok(())
    }

    /// Open an interactive PTY-backed `ssh` session for `profile_id`'s embedded terminal.
    /// Requires the profile to already have a connected tunnel session (`connect()`).
    pub fn terminal_open(&self, app: AppHandle, profile_id: String) -> Result<(), String> {
        let (profile, terminals) = self.with_inner(|i| {
            let session = i
                .sessions
                .get(&profile_id)
                .ok_or_else(|| "该主机未连接，请先连接后再打开终端".to_string())?;
            Ok::<_, String>((session.profile.clone(), i.terminals.clone()))
        })?;
        terminals.open(app, &profile)
    }

    pub fn terminal_write(&self, profile_id: String, data: String) -> Result<(), String> {
        let terminals = self.with_inner(|i| i.terminals.clone());
        terminals.write(&profile_id, &data)
    }

    pub fn terminal_resize(&self, profile_id: String, cols: u16, rows: u16) -> Result<(), String> {
        let terminals = self.with_inner(|i| i.terminals.clone());
        terminals.resize(&profile_id, cols, rows)
    }

    pub fn terminal_close(&self, profile_id: String) {
        let terminals = self.with_inner(|i| i.terminals.clone());
        terminals.close(&profile_id);
    }

    pub fn transfer_upload(
        &self,
        profile_id: String,
        local_path: String,
        remote_path: String,
    ) -> Result<(), String> {
        let profile =
            self.reserve_transfer(&profile_id, format!("上传中 → {remote_path}"))?;
        let _guard = TransferGuard {
            state: self.clone(),
            id: profile_id,
        };
        run_upload(&profile, &local_path, &remote_path)
    }

    pub fn transfer_download(
        &self,
        profile_id: String,
        remote_path: String,
        local_path: String,
    ) -> Result<(), String> {
        let profile =
            self.reserve_transfer(&profile_id, format!("下载中 ← {remote_path}"))?;
        let _guard = TransferGuard {
            state: self.clone(),
            id: profile_id,
        };
        run_download(&profile, &remote_path, &local_path)
    }

    /// Reports transfer busy state for a single host (`profile_id`, defaulting to the
    /// active profile), not a global HashSet-derived state — a transfer on host A must never
    /// report as busy, or lock UI, for host B (see review I3).
    pub fn transfer_status(&self, profile_id: Option<String>) -> TransferStatus {
        self.with_inner(|i| {
            let query_id = profile_id.or_else(|| i.profiles.active_id());
            let busy = query_id
                .as_ref()
                .and_then(|id| i.transfer_busy.get(id).map(|desc| (id.clone(), desc.clone())));
            match busy {
                Some((id, desc)) => {
                    let name = i
                        .profiles
                        .get(&id)
                        .map(|p| p.name)
                        .unwrap_or_else(|| id.clone());
                    TransferStatus {
                        profile_id: Some(id),
                        state: "running".to_string(),
                        detail: Some(format!("{name} {desc}")),
                    }
                }
                None => TransferStatus {
                    profile_id: None,
                    state: "idle".to_string(),
                    detail: None,
                },
            }
        })
    }

    fn reserve_transfer(&self, profile_id: &str, description: String) -> Result<GatewayProfile, String> {
        self.with_inner(|i| {
            let profile = i
                .sessions
                .get(profile_id)
                .ok_or_else(|| "请先 Connect 该主机".to_string())?
                .profile
                .clone();
            if i.transfer_busy.contains_key(profile_id) {
                return Err("请等待当前传输完成".to_string());
            }
            i.transfer_busy.insert(profile_id.to_string(), description);
            Ok(profile)
        })
    }
}

fn collect_used_ports(sessions: &HashMap<String, LiveSession>) -> HashSet<u16> {
    let mut used = HashSet::new();
    for s in sessions.values() {
        used.insert(s.local_http);
        used.insert(s.local_socks);
    }
    used
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
