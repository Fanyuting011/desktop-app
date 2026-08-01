import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { openUrl } from "@tauri-apps/plugin-opener";
import LogsPanel from "./components/LogsPanel";
import NetworkPanel from "./components/NetworkPanel";
import FilesPanel from "./components/FilesPanel";
import HostTerminal from "./components/HostTerminal";
import ConnectingOverlay from "./components/ConnectingOverlay";
import "./App.css";

type Phase = "idle" | "connected" | "proxyOn" | "reconnecting";
type Nav = "hosts" | "logs" | "network" | "files";
// "hosts" or a connected session's profileId (its terminal tab).
type CenterTab = string;

interface GatewayProfile {
  id: string;
  name: string;
  host: string;
  port: number;
  user: string;
  identityFile: string;
  password: string;
  remoteHttpPort: number;
  remoteSocksPort: number;
  autoReconnect: boolean;
  noProxy: string[];
  portForwards?: Array<{
    id: string;
    enabled: boolean;
    localHost: string;
    localPort: number;
    remoteHost: string;
    remotePort: number;
    label?: string | null;
  }>;
  updatedAt: string;
}

interface SessionInfo {
  profileId: string;
  phase: Phase;
  lastError: string | null;
  localHttpPort: number;
  localSocksPort: number;
}

interface GatewayStatus {
  activeProfileId: string | null;
  sessions: SessionInfo[];
  localHttp: string;
  localSocks: string;
  upstreamProxy?: string | null;
}

const UPSTREAM_KEY = "outgate.upstreamProxy";
const PRESET_PORTS = [3000, 8080, 5432] as const;

function presetEnabled(draft: GatewayProfile, port: number) {
  return (draft.portForwards ?? []).some(
    (forward) =>
      forward.enabled &&
      forward.localPort === port &&
      forward.remotePort === port &&
      (forward.remoteHost === "127.0.0.1" || forward.remoteHost === "localhost"),
  );
}

function noProxyToText(list: string[]) {
  return list.join("\n");
}

function textToNoProxy(text: string) {
  return text
    .split(/[\n,]+/)
    .map((s) => s.trim())
    .filter(Boolean);
}

function phaseOf(status: GatewayStatus | null, id: string | undefined): Phase {
  if (!status || !id) return "idle";
  return status.sessions.find((s) => s.profileId === id)?.phase ?? "idle";
}

function isLive(phase: Phase): boolean {
  return phase === "connected" || phase === "proxyOn" || phase === "reconnecting";
}

function statusDot(phase: Phase, busy: boolean): string {
  if (busy) return "dot blue pulse";
  if (phase === "connected" || phase === "proxyOn") return "dot green";
  if (phase === "reconnecting") return "dot blue pulse";
  return "dot gray";
}

export default function App() {
  const [nav, setNav] = useState<Nav>("hosts");
  const [centerTab, setCenterTab] = useState<CenterTab>("hosts");
  const [version, setVersion] = useState("…");
  const [profiles, setProfiles] = useState<GatewayProfile[]>([]);
  const [status, setStatus] = useState<GatewayStatus | null>(null);
  const [draft, setDraft] = useState<GatewayProfile | null>(null);
  const [busy, setBusy] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [presetBusyPort, setPresetBusyPort] = useState<number | null>(null);
  const [message, setMessage] = useState("");
  const [query, setQuery] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [connectingProfile, setConnectingProfile] = useState<GatewayProfile | null>(null);
  const [connectPhase, setConnectPhase] = useState<"connecting" | "failed">("connecting");
  const [connectError, setConnectError] = useState("");
  const [showConnectLogs, setShowConnectLogs] = useState(false);
  const [connectLogs, setConnectLogs] = useState<string[]>([]);
  const connectCancelled = useRef(false);
  const [upstream, setUpstream] = useState(() => {
    try {
      return localStorage.getItem(UPSTREAM_KEY) ?? "";
    } catch {
      return "";
    }
  });

  const draftPhase = phaseOf(status, draft?.id);
  const draftConnected = isLive(draftPhase);
  const draftEditable = !draftConnected;
  const enabledPresetPorts = draft
    ? PRESET_PORTS.filter((port) => presetEnabled(draft, port))
    : [];
  const draftIsSaved = !!draft && profiles.some((profile) => profile.id === draft.id);
  const anyConnected = (status?.sessions?.length ?? 0) > 0;
  const noProxySummary =
    draft?.noProxy.join(", ") ??
    profiles.find((profile) => profile.id === status?.activeProfileId)?.noProxy.join(", ") ??
    "";

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return profiles;
    return profiles.filter(
      (p) =>
        p.name.toLowerCase().includes(q) ||
        p.host.toLowerCase().includes(q) ||
        p.user.toLowerCase().includes(q),
    );
  }, [profiles, query]);

  const nameOf = useCallback(
    (id: string) => {
      const p = profiles.find((x) => x.id === id);
      return p?.name || p?.host || id;
    },
    [profiles],
  );

  const refresh = useCallback(async () => {
    const [plist, st] = await Promise.all([
      invoke<GatewayProfile[]>("gateway_list_profiles"),
      invoke<GatewayStatus>("gateway_get_status"),
    ]);
    setProfiles(plist);
    setStatus(st);
    setDraft((prev) => {
      if (prev) {
        const latest = plist.find((p) => p.id === prev.id);
        return latest ? { ...latest, password: prev.password || latest.password } : prev;
      }
      if (st.activeProfileId) {
        const active = plist.find((p) => p.id === st.activeProfileId);
        return active ? { ...active } : null;
      }
      return null;
    });
  }, []);

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion("未知"));
    refresh().catch((e) => setMessage(String(e)));
  }, [refresh]);

  useEffect(() => {
    const id = window.setInterval(() => {
      invoke<GatewayStatus>("gateway_poll")
        .then(setStatus)
        .catch(() => {});
    }, 3000);
    return () => window.clearInterval(id);
  }, []);

  useEffect(() => {
    try {
      localStorage.setItem(UPSTREAM_KEY, upstream);
    } catch {
      /* ignore */
    }
  }, [upstream]);

  async function selectProfile(id: string) {
    setMessage("");
    try {
      await invoke("gateway_set_active_profile", { id });
      const p = profiles.find((x) => x.id === id);
      if (p) setDraft({ ...p });
      await refresh();
    } catch (e) {
      setMessage(String(e));
    }
  }

  async function createProfile() {
    const blank = await invoke<GatewayProfile>("gateway_new_profile");
    setDraft(blank);
  }

  async function saveDraft() {
    if (!draft) return;
    setBusy(true);
    setMessage("");
    try {
      const saved = await invoke<GatewayProfile>("gateway_upsert_profile", {
        profile: draft,
      });
      setDraft(saved);
      await invoke("gateway_set_active_profile", { id: saved.id });
      await refresh();
    } catch (e) {
      setMessage(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function removeProfile() {
    if (!draft) return;
    if (!window.confirm(`删除「${draft.name}」？`)) return;
    setBusy(true);
    try {
      await invoke("gateway_delete_profile", { id: draft.id });
      setDraft(null);
      await refresh();
    } catch (e) {
      setMessage(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function togglePreset(port: number) {
    if (!draft || !draftIsSaved) return;
    setPresetBusyPort(port);
    setMessage("");
    try {
      const st = await invoke<GatewayStatus>("gateway_set_port_forward_preset", {
        profileId: draft.id,
        port,
        enabled: !presetEnabled(draft, port),
      });
      setStatus(st);
      const plist = await invoke<GatewayProfile[]>("gateway_list_profiles");
      setProfiles(plist);
      setDraft((prev) => {
        const latest = plist.find((profile) => profile.id === prev?.id);
        return latest && prev ? { ...prev, portForwards: latest.portForwards } : prev;
      });
    } catch (e) {
      setMessage(String(e));
    } finally {
      setPresetBusyPort(null);
    }
  }

  async function connect() {
    if (!draft || draftConnected) return;
    const profile = { ...draft };
    const id = profile.id;
    connectCancelled.current = false;
    setConnectingProfile(profile);
    setConnectPhase("connecting");
    setConnectError("");
    setShowConnectLogs(false);
    setConnectLogs([]);
    setBusy(true);
    setBusyId(id);
    setMessage("");
    try {
      await invoke("gateway_set_active_profile", { id });
      const st = await invoke<GatewayStatus>("gateway_connect", {
        profileId: id,
        upstreamProxy: upstream.trim() || null,
      });
      if (connectCancelled.current) {
        try {
          await invoke("gateway_disconnect", { profileId: id });
        } catch {
          /* ignore */
        }
        await refresh();
        return;
      }
      // Open PTY before React mounts HostTerminal (setStatus/setCenterTab),
      // otherwise early resize calls fail and bash stays at 80 cols — Up-arrow
      // history redraw then walks into previous lines and overwrites them.
      try {
        await invoke("gateway_terminal_open", { profileId: id });
      } catch (e) {
        setMessage(String(e));
      }
      setStatus(st);
      setConnectingProfile(null);
      setCenterTab(id);
      await refresh();
    } catch (e) {
      if (connectCancelled.current) {
        setConnectingProfile(null);
        await refresh();
        return;
      }
      setConnectPhase("failed");
      setConnectError(String(e));
      await refresh();
    } finally {
      setBusy(false);
      setBusyId(null);
    }
  }

  function closeConnectingOverlay() {
    if (connectPhase === "connecting" && busyId) {
      connectCancelled.current = true;
    }
    setConnectingProfile(null);
    setConnectError("");
    setShowConnectLogs(false);
  }

  async function disconnectProfile(id: string) {
    setBusy(true);
    setBusyId(id);
    setMessage("");
    try {
      const st = await invoke<GatewayStatus>("gateway_disconnect", {
        profileId: id,
      });
      setStatus(st);
      setCenterTab((prev) => (prev === id ? "hosts" : prev));
      await refresh();
    } catch (e) {
      setMessage(String(e));
      await refresh();
    } finally {
      setBusy(false);
      setBusyId(null);
    }
  }

  useEffect(() => {
    if (!connectingProfile || connectPhase !== "connecting") return;
    let alive = true;
    const tick = () => {
      invoke<string[]>("gateway_get_logs", {
        limit: 80,
        profileId: connectingProfile.id,
      })
        .then((lines) => {
          if (alive) setConnectLogs(lines);
        })
        .catch(() => {});
    };
    tick();
    const id = window.setInterval(tick, 800);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [connectingProfile, connectPhase]);

  async function checkUpdate() {
    setBusy(true);
    const trimmed = upstream.trim();
    const proxy = trimmed
      ? /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(trimmed)
        ? trimmed
        : `http://${trimmed}`
      : undefined;
    try {
      const update = await check({
        ...(proxy ? { proxy } : {}),
        timeout: 60_000,
      });
      if (!update) {
        setMessage("已是最新版本");
        return;
      }
      setMessage(`发现 ${update.version}，下载中…`);
      await update.downloadAndInstall(() => {});
      await relaunch();
    } catch (e) {
      setMessage(`更新失败：${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  const terminalFocused = nav === "hosts" && centerTab !== "hosts";
  const connectingView = nav === "hosts" && centerTab === "hosts" && !!connectingProfile;

  return (
    <div className={terminalFocused || connectingView ? "shell wide-center" : "shell"}>
      <aside className="nav">
        <div className="brand">OutGate</div>
        <button
          type="button"
          className={nav === "hosts" ? "nav-item active" : "nav-item"}
          onClick={() => setNav("hosts")}
        >
          Hosts
        </button>
        <button
          type="button"
          className={nav === "logs" ? "nav-item active" : "nav-item"}
          onClick={() => setNav("logs")}
        >
          Logs
        </button>
        <button
          type="button"
          className={nav === "network" ? "nav-item active" : "nav-item"}
          onClick={() => setNav("network")}
        >
          Network
        </button>
        <button
          type="button"
          className={nav === "files" ? "nav-item active" : "nav-item"}
          onClick={() => setNav("files")}
        >
          Files
        </button>
        <div className="nav-foot">
          <button type="button" className="nav-update" disabled={busy} onClick={checkUpdate}>
            Check for updates
          </button>
          <span className="nav-ver">v{version}</span>
        </div>
      </aside>

      {
        /* The Hosts workspace (tab bar + host grid/terminal panes) always stays
           mounted — only its visibility is toggled — so an open HostTerminal
           keeps its xterm instance, scrollback and listeners alive while the
           user browses Logs/Network and comes back, instead of losing state
           and re-running the outgate-on injection on every nav switch. */
      }
      <>
        <section
          className={terminalFocused ? "center center-terminal" : "center"}
          style={{ display: nav === "hosts" ? "flex" : "none" }}
        >
          <div className="tabbar">
            <button
              type="button"
              className={centerTab === "hosts" ? "tab active" : "tab"}
              onClick={() => setCenterTab("hosts")}
            >
              Hosts
            </button>
            {status?.sessions.map((s) => (
              <button
                key={s.profileId}
                type="button"
                className={centerTab === s.profileId ? "tab active" : "tab"}
                onClick={() => setCenterTab(s.profileId)}
              >
                {nameOf(s.profileId)}
                <span
                  className="tab-close"
                  role="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    disconnectProfile(s.profileId);
                  }}
                >
                  ×
                </span>
              </button>
            ))}
          </div>

          {centerTab === "hosts" && connectingProfile && (
            <ConnectingOverlay
              profile={connectingProfile}
              phase={connectPhase}
              error={connectError}
              logs={connectLogs}
              showLogs={showConnectLogs}
              onToggleLogs={() => setShowConnectLogs((v) => !v)}
              onClose={closeConnectingOverlay}
            />
          )}

          {centerTab === "hosts" && !connectingProfile && (
            <>
              <div className="toolbar">
                <input
                  className="search"
                  placeholder="Find a host or user@hostname…"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                />
                <button type="button" className="btn ghost" disabled={busy} onClick={createProfile}>
                  + New host
                </button>
                <button
                  type="button"
                  className="btn primary"
                  disabled={busy || !draft || draftConnected}
                  onClick={connect}
                >
                  Connect
                </button>
              </div>

              <h2 className="section-title">Hosts</h2>
              <div className="host-scroll scroll">
                <div className="host-grid">
                  {filtered.map((p) => {
                    const selected = draft?.id === p.id;
                    const phase = phaseOf(status, p.id);
                    return (
                      <button
                        key={p.id}
                        type="button"
                        className={selected ? "host-card selected" : "host-card"}
                        onClick={() => selectProfile(p.id)}
                      >
                        <span className={statusDot(phase, busyId === p.id)} />
                        <strong>{p.name || p.host || "未命名"}</strong>
                        <span className="sub">
                          ssh, {p.user || "?"}@{p.host || "?"}
                        </span>
                      </button>
                    );
                  })}
                </div>
                {filtered.length === 0 && (
                  <p className="empty">暂无主机，点击 + New host</p>
                )}
              </div>
            </>
          )}

          {status && status.sessions.length > 0 && (
            <div
              className="terminal-stack"
              style={{ display: centerTab === "hosts" ? "none" : "flex" }}
            >
              {status.sessions.map((s) => (
                <div
                  key={s.profileId}
                  className="terminal-pane"
                  style={{ display: s.profileId === centerTab ? "flex" : "none" }}
                >
                  <HostTerminal profileId={s.profileId} />
                </div>
              ))}
            </div>
          )}
        </section>

        {nav === "hosts" && centerTab === "hosts" && !connectingProfile && (
          <aside className="details">
            <header className="details-head">
              <h2>Host Details</h2>
              {draft && (
                <button
                  type="button"
                  className="link danger"
                  disabled={busy || draftConnected}
                  onClick={removeProfile}
                >
                  Delete
                </button>
              )}
            </header>

            {!draft ? (
              <p className="empty">选择或新建一台主机</p>
            ) : (
              <div className="form">
                <label>
                  Label
                  <input
                    value={draft.name}
                    disabled={busy || !draftEditable}
                    onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                  />
                </label>
                <label>
                  Host
                  <input
                    value={draft.host}
                    disabled={busy || !draftEditable}
                    onChange={(e) => setDraft({ ...draft, host: e.target.value })}
                  />
                </label>
                <div className="row2">
                  <label>
                    SSH Port
                    <input
                      type="number"
                      value={draft.port}
                      disabled={busy || !draftEditable}
                      onChange={(e) =>
                        setDraft({ ...draft, port: Number(e.target.value) || 22 })
                      }
                    />
                  </label>
                  <label>
                    User
                    <input
                      value={draft.user}
                      disabled={busy || !draftEditable}
                      onChange={(e) => setDraft({ ...draft, user: e.target.value })}
                    />
                  </label>
                </div>
                <label>
                  Password
                  <div className="pwd">
                    <input
                      type={showPassword ? "text" : "password"}
                      value={draft.password ?? ""}
                      disabled={busy || !draftEditable}
                      autoComplete="off"
                      onChange={(e) => setDraft({ ...draft, password: e.target.value })}
                    />
                    <button type="button" className="eye" onClick={() => setShowPassword((v) => !v)}>
                      {showPassword ? "Hide" : "Show"}
                    </button>
                  </div>
                </label>
                <label>
                  Identity file
                  <input
                    value={draft.identityFile}
                    disabled={busy || !draftEditable}
                    placeholder="可选，私钥路径"
                    onChange={(e) => setDraft({ ...draft, identityFile: e.target.value })}
                  />
                </label>
                <label>
                  Upstream proxy（本机 Clash 等）
                  <input
                    value={upstream}
                    disabled={busy || anyConnected}
                    placeholder="127.0.0.1:7890"
                    onChange={(e) => setUpstream(e.target.value)}
                  />
                </label>
                <div className="row2">
                  <label>
                    Remote HTTP
                    <input
                      type="number"
                      value={draft.remoteHttpPort}
                      disabled={busy || !draftEditable}
                      onChange={(e) =>
                        setDraft({
                          ...draft,
                          remoteHttpPort: Number(e.target.value) || 17890,
                        })
                      }
                    />
                  </label>
                  <label>
                    Remote SOCKS
                    <input
                      type="number"
                      value={draft.remoteSocksPort}
                      disabled={busy || !draftEditable}
                      onChange={(e) =>
                        setDraft({
                          ...draft,
                          remoteSocksPort: Number(e.target.value) || 17891,
                        })
                      }
                    />
                  </label>
                </div>
                <section className="preview-ports" aria-labelledby="preview-ports-title">
                  <div className="preview-ports-head">
                    <span id="preview-ports-title">本地预览</span>
                    {!draftIsSaved && <span>请先保存主机</span>}
                  </div>
                  <div className="preview-port-chips" aria-label="预览端口">
                    {PRESET_PORTS.map((port) => {
                      const enabled = presetEnabled(draft, port);
                      return (
                        <button
                          key={port}
                          type="button"
                          className={enabled ? "preview-port-chip selected" : "preview-port-chip"}
                          disabled={!draftIsSaved || presetBusyPort !== null}
                          aria-pressed={enabled}
                          onClick={() => togglePreset(port)}
                        >
                          {port}
                        </button>
                      );
                    })}
                  </div>
                  {enabledPresetPorts.length === 0 ? (
                    <p className="preview-port-empty">未启用预览端口</p>
                  ) : (
                    <div className="preview-port-list">
                      {enabledPresetPorts.map((port) => (
                        <div key={port} className="preview-port-item">
                          <span>127.0.0.1:{port}</span>
                          {draftConnected && (port === 3000 || port === 8080) && (
                            <button
                              type="button"
                              className="link"
                              onClick={() =>
                                openUrl(`http://127.0.0.1:${port}`).catch((e) =>
                                  setMessage(String(e)),
                                )
                              }
                            >
                              打开
                            </button>
                          )}
                        </div>
                      ))}
                    </div>
                  )}
                </section>
                <label>
                  NO_PROXY
                  <textarea
                    rows={3}
                    value={noProxyToText(draft.noProxy)}
                    disabled={busy || !draftEditable}
                    onChange={(e) =>
                      setDraft({ ...draft, noProxy: textToNoProxy(e.target.value) })
                    }
                  />
                </label>
                <label className="check">
                  <input
                    type="checkbox"
                    checked={draft.autoReconnect}
                    disabled={busy}
                    onChange={async (e) => {
                      const enabled = e.target.checked;
                      setDraft({ ...draft, autoReconnect: enabled });
                      try {
                        await invoke("gateway_set_reconnect", {
                          profileId: draft.id,
                          enabled,
                        });
                      } catch {
                        /* ignore */
                      }
                    }}
                  />
                  Auto reconnect
                </label>

                <div className="actions">
                  <button
                    type="button"
                    className="btn ghost"
                    disabled={busy || !draftEditable}
                    onClick={saveDraft}
                  >
                    Save
                  </button>
                  {!draftConnected && (
                    <button
                      type="button"
                      className="btn primary wide"
                      disabled={busy || !draft}
                      onClick={connect}
                    >
                      Connect
                    </button>
                  )}
                </div>
                {message && <p className="banner">{message}</p>}
              </div>
            )}
          </aside>
        )}
      </>

      {nav === "logs" && <LogsPanel profiles={profiles} />}
      {nav === "network" && (
        <NetworkPanel
          active={nav === "network"}
          profiles={profiles}
          noProxySummary={noProxySummary}
        />
      )}
      {nav === "files" && <FilesPanel active={nav === "files"} profiles={profiles} status={status} />}
    </div>
  );
}
