# OutGate Terminal + Network Logs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Per-host local proxy ports with Network request logs, Host-filterable gateway Logs, and Termius-style embedded SSH terminal tabs that inject `outgate on` only into that PTY (no global shell injection).

**Architecture:** Each connected session owns its own HTTP/SOCKS listeners tagged with `profile_id`. A separate interactive `ssh` PTY (portable-pty) feeds xterm.js. Closing a terminal tab calls the same disconnect path as Disconnect. Left nav adds Network under Logs.

**Tech Stack:** Tauri 2, React 19, Rust (tokio, portable-pty), xterm.js + @xterm/addon-fit, system OpenSSH.

**Spec:** `docs/superpowers/specs/2026-08-01-outgate-terminal-design.md`

## Global Constraints

- No login/bashrc auto-inject of proxy for all shells (scope B).
- System `ssh` only — no russh interactive stack this iteration.
- Remote server remote ports stay per-profile (often 17890/17891); only **local** bind ports become per-session.
- Mac + Windows must work; hide console flash on Windows where practical.
- Prefer small focused modules under `src-tauri/src/gateway/` and thin React components under `src/`.
- Do not bump release tag unless the user asks; keep working on `feat/ssh-gateway-proxy` (or current feature branch).

---

## File map

| File | Responsibility |
|------|----------------|
| `src-tauri/src/gateway/port_alloc.rs` | Allocate free local HTTP/SOCKS port pairs from base 17890 |
| `src-tauri/src/gateway/network_log.rs` | Ring buffer of structured proxy access events |
| `src-tauri/src/gateway/proxy.rs` | Accept `profile_id` + `NetworkLog` callback; emit CONNECT/SOCKS targets |
| `src-tauri/src/gateway/manager.rs` | Per-session `ProxyHandles` (remove shared proxy); wire network logs; status fields |
| `src-tauri/src/gateway/terminal.rs` | PTY session map: open/write/resize/close via portable-pty |
| `src-tauri/src/gateway/ssh_tunnel.rs` | Reuse `base_ssh_cmd` / askpass for interactive spawn helper if needed |
| `src-tauri/src/gateway/mod.rs` | Module exports |
| `src-tauri/src/lib.rs` | Register commands + terminal output events |
| `src-tauri/Cargo.toml` | Add `portable-pty`, `parking_lot` optional; keep tokio |
| `package.json` | Add `@xterm/xterm`, `@xterm/addon-fit` |
| `src/App.tsx` | Nav Network; center tabs; details visibility; wire APIs |
| `src/App.css` | Tab bar, network table, terminal pane, details hide |
| `src/components/HostTerminal.tsx` | xterm wrapper for one profileId |
| `src/components/NetworkPanel.tsx` | Network log UI + host filter |
| `src/components/LogsPanel.tsx` | Extract logs UI + host filter (optional split from App) |

---

### Task 1: Port allocator

**Files:**
- Create: `src-tauri/src/gateway/port_alloc.rs`
- Modify: `src-tauri/src/gateway/mod.rs`
- Test: unit tests in `port_alloc.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces: `pub fn allocate_port_pair(used: &HashSet<u16>, base_http: u16) -> Result<(u16, u16), String>`  
  Returns `(http, socks)` with `socks == http + 1`, both free and not in `used`. Scans upward from `base_http` (default 17890).

- [ ] **Step 1: Write failing unit test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::net::TcpListener;

    #[test]
    fn skips_ports_in_used_set() {
        let mut used = HashSet::new();
        used.insert(17890);
        used.insert(17891);
        let (h, s) = allocate_port_pair(&used, 17890).unwrap();
        assert_ne!(h, 17890);
        assert_eq!(s, h + 1);
        assert!(!used.contains(&h));
    }

    #[test]
    fn skips_ports_already_bound() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let busy = listener.local_addr().unwrap().port();
        // Force allocator to consider a range that includes busy by using busy as base when even
        let base = if busy % 2 == 0 { busy } else { busy.saturating_sub(1) };
        let mut used = HashSet::new();
        let (h, s) = allocate_port_pair(&used, base).unwrap();
        assert!(h != busy && s != busy);
    }
}
```

- [ ] **Step 2: Run test — expect fail (module missing)**

Run: `cd src-tauri && cargo test --lib gateway::port_alloc -- --nocapture`  
Expected: compile error / unresolved module

- [ ] **Step 3: Implement allocator**

```rust
use std::collections::HashSet;
use std::net::TcpListener;

const MAX_TRIES: u16 = 200;

pub fn allocate_port_pair(used: &HashSet<u16>, base_http: u16) -> Result<(u16, u16), String> {
    let mut http = if base_http % 2 == 0 { base_http } else { base_http + 1 };
    for _ in 0..MAX_TRIES {
        let socks = http + 1;
        if !used.contains(&http)
            && !used.contains(&socks)
            && port_free(http)
            && port_free(socks)
        {
            return Ok((http, socks));
        }
        http = http.saturating_add(2);
        if http > 60000 {
            break;
        }
    }
    Err("无法分配本地代理端口".into())
}

fn port_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}
```

- [ ] **Step 4: Export module and re-run tests**

`mod port_alloc;` in `mod.rs`.  
Run: `cargo test --lib gateway::port_alloc`  
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/gateway/port_alloc.rs src-tauri/src/gateway/mod.rs
git commit -m "feat(gateway): allocate per-session local proxy port pairs"
```

---

### Task 2: Network log buffer

**Files:**
- Create: `src-tauri/src/gateway/network_log.rs`
- Modify: `src-tauri/src/gateway/mod.rs`

**Interfaces:**
- Produces:
```rust
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkLogEntry {
    pub id: String,
    pub ts_ms: u64,
    pub profile_id: String,
    pub protocol: String, // "http" | "socks"
    pub target: String,
    pub ok: bool,
    pub error: Option<String>,
}

pub struct NetworkLogBuffer { /* cap 1000 */ }
impl NetworkLogBuffer {
    pub fn new() -> Self;
    pub fn push(&self, entry: NetworkLogEntry);
    pub fn snapshot(&self, profile_id: Option<&str>, limit: usize) -> Vec<NetworkLogEntry>;
    pub fn clear(&self, profile_id: Option<&str>);
}
```

- [ ] **Step 1: Write unit tests** for push, filter by profile_id, cap eviction, clear

- [ ] **Step 2: Run — expect fail**

- [ ] **Step 3: Implement buffer** (Mutex + VecDeque; filter in snapshot)

- [ ] **Step 4: Tests PASS**

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(gateway): add structured network request log buffer"
```

---

### Task 3: Proxy emits network logs + profile_id

**Files:**
- Modify: `src-tauri/src/gateway/proxy.rs`
- Modify callers in `manager.rs` (minimal stub OK until Task 4)

**Interfaces:**
- Change signature:
```rust
pub async fn start_local_proxies(
    http_port: u16,
    socks_port: u16,
    upstream: Option<UpstreamKind>,
    profile_id: String,
    net_log: Arc<NetworkLogBuffer>,
) -> Result<ProxyHandles, String>
```
- On HTTP CONNECT success/fail and SOCKS connect success/fail, `net_log.push(NetworkLogEntry { ... })`.

- [ ] **Step 1: Thread `profile_id` + `net_log` into `run_http_proxy` / `handle_http_client` / socks handlers**

In `handle_http_client` after parsing CONNECT target:

```rust
let target = hostport.to_string();
match dial_target(...).await {
    Ok(up) => {
        net_log.push(NetworkLogEntry {
            id: uuid::Uuid::new_v4().to_string(),
            ts_ms: now_ms(),
            profile_id: profile_id.clone(),
            protocol: "http".into(),
            target: target.clone(),
            ok: true,
            error: None,
        });
        // ... existing 200 + relay
    }
    Err(e) => {
        net_log.push(NetworkLogEntry { ok: false, error: Some(e.to_string()), ... });
        // existing error response
    }
}
```

Same pattern for SOCKS after destination resolved.

- [ ] **Step 2: `cargo check` — fix call sites to pass empty Arc temporarily if needed**

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(gateway): record CONNECT/SOCKS targets into network log"
```

---

### Task 4: Manager — per-session proxy (drop shared proxy)

**Files:**
- Modify: `src-tauri/src/gateway/manager.rs`
- Modify: `src-tauri/src/lib.rs` (status shape if needed)

**Interfaces:**
- `LiveSession` gains: `proxy: ProxyHandles`, `local_http: u16`, `local_socks: u16`
- Remove `shared_proxy` field; `connecting` hold still used during connect before insert
- `SessionInfo` gains `localHttpPort`, `localSocksPort` (and optionally display URLs)
- `GatewayStatus.local_http` / `local_socks`: keep as **active profile’s** ports or first session; document in UI that each session has its own
- `self_stop_proxy_if_empty`: stop nothing global; each disconnect already `session.proxy.stop()`
- On connect: `allocate_port_pair` from used ports across sessions → `start_local_proxies(..., profile.id, net_logs.clone())` → `SshTunnel::spawn(profile, local_http, local_socks, ...)`

- [ ] **Step 1: Add `network_logs: Arc<NetworkLogBuffer>` to `Inner`**

- [ ] **Step 2: Refactor `connect` / `disconnect` / `reconnect_one` to own per-session proxy**

Pseudo:

```rust
let used = collect_used_ports(&i.sessions);
let (local_http, local_socks) = allocate_port_pair(&used, 17890)?;
let proxy = runtime.block_on(start_local_proxies(
    local_http, local_socks, upstream.clone(), profile.id.clone(), i.network_logs.clone(),
))?;
let tunnel = SshTunnel::spawn(&profile, local_http, local_socks, logs)?;
// insert LiveSession { proxy, local_http, local_socks, ... }
```

On disconnect / session removal: `session.proxy.stop()` then kill tunnel (order: remote outgate off → kill tunnel → stop proxy).

- [ ] **Step 3: Remove SHARED_LOCAL_* constants and shared_proxy stop logic**

- [ ] **Step 4: `cargo check` + manual connect one host — tunnel should still work**

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(gateway): bind a dedicated local proxy per host session"
```

---

### Task 5: Network + Logs Tauri commands

**Files:**
- Modify: `src-tauri/src/gateway/manager.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/gateway/log_buffer.rs` (optional filter helper)

**Interfaces:**
```rust
// manager
pub fn get_network_logs(&self, profile_id: Option<String>, limit: usize) -> Vec<NetworkLogEntry>;
pub fn clear_network_logs(&self, profile_id: Option<String>);
pub fn get_logs(&self, limit: usize, profile_id: Option<String>) -> Vec<String>;
```

Log filter: if `profile_id` Some, resolve profile **name**, keep lines containing `[name]` or `profile_id`; always include lines with no bracket tag when filter is `None` only.

- [ ] **Step 1: Add commands**

```rust
#[tauri::command]
fn gateway_get_network_logs(state: State<GatewayState>, profile_id: Option<String>, limit: usize) -> Vec<NetworkLogEntry> {
    state.get_network_logs(profile_id, limit)
}

#[tauri::command]
fn gateway_clear_network_logs(state: State<GatewayState>, profile_id: Option<String>) {
    state.clear_network_logs(profile_id)
}

// extend gateway_get_logs with profile_id: Option<String>
```

Register in `generate_handler!`.

- [ ] **Step 2: `cargo check`**

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(gateway): expose filtered logs and network log commands"
```

---

### Task 6: UI — Logs filter + Network panel

**Files:**
- Create: `src/components/NetworkPanel.tsx`
- Create: `src/components/LogsPanel.tsx` (or inline in App if smaller)
- Modify: `src/App.tsx`, `src/App.css`

**Interfaces:**
- `nav`: `"hosts" | "logs" | "network"`
- Host filter select: `all` | each `status.sessions[].profileId` (+ profiles by id for labels)

- [ ] **Step 1: Add left nav button Network under Logs**

- [ ] **Step 2: LogsPanel** — dropdown filter; poll `gateway_get_logs` with `profileId`

- [ ] **Step 3: NetworkPanel** — table columns Time / Host / Proto / Target / Result; filter; Clear button → `gateway_clear_network_logs`; poll every 2s while on page

```tsx
interface NetworkLogEntry {
  id: string;
  tsMs: number;
  profileId: string;
  protocol: string;
  target: string;
  ok: boolean;
  error: string | null;
}
```

- [ ] **Step 4: Manual test with two hosts curling different sites — Network filter separates rows**

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(ui): add Network page and Host-filtered Logs"
```

---

### Task 7: Terminal PTY backend

**Files:**
- Create: `src/tauri/src/gateway/terminal.rs` → correct path `src-tauri/src/gateway/terminal.rs`
- Modify: `Cargo.toml` add `portable-pty = "0.8"`
- Modify: `mod.rs`, `manager.rs` or keep `TerminalHub` on `GatewayState`
- Modify: `lib.rs`

**Interfaces:**
```rust
pub struct TerminalHub { /* HashMap<profile_id, PtySession> */ }

impl GatewayState {
  pub fn terminal_open(&self, app: AppHandle, profile_id: String) -> Result<(), String>;
  pub fn terminal_write(&self, profile_id: String, data: String) -> Result<(), String>;
  pub fn terminal_resize(&self, profile_id: String, cols: u16, rows: u16) -> Result<(), String>;
  pub fn terminal_close(&self, profile_id: String);
}
```

Event: `app.emit(format!("terminal-output-{profile_id}"), base64_or_utf8_lossy_chunk)`.

Spawn: build argv like interactive ssh (no `-N`), reuse askpass from `askpass.rs` / `ssh_tunnel::` helpers — extract `fn build_ssh_command(profile, askpass, interactive: bool) -> Command` if needed.

Reader thread: read PTY master → emit events.  
On disconnect path: `terminal_close` then kill tunnel.

Windows: set `CREATE_NO_WINDOW` on Command if using std Command; portable-pty may spawn ssh differently — document verification.

- [ ] **Step 1: Add dependency and skeleton open/write/close**

- [ ] **Step 2: Commands**

```rust
#[tauri::command]
async fn gateway_terminal_open(app: AppHandle, state: State<'_, GatewayState>, profile_id: String) -> Result<(), String>;
#[tauri::command]
fn gateway_terminal_write(state: State<'_, GatewayState>, profile_id: String, data: String) -> Result<(), String>;
#[tauri::command]
fn gateway_terminal_resize(state: State<'_, GatewayState>, profile_id: String, cols: u16, rows: u16) -> Result<(), String>;
```

- [ ] **Step 3: Wire `disconnect` to `terminal_close`**

- [ ] **Step 4: `cargo check`**

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(gateway): interactive SSH PTY sessions for embedded terminal"
```

---

### Task 8: UI — center tabs + xterm + auto inject

**Files:**
- Create: `src/components/HostTerminal.tsx`
- Modify: `package.json` / lockfile — `@xterm/xterm`, `@xterm/addon-fit`
- Modify: `src/App.tsx`, `src/App.css`
- Import xterm CSS in component or `main.tsx`

**Interfaces:**
- After successful `gateway_connect`, set `centerTab = { type: 'terminal', profileId }` and `invoke('gateway_terminal_open', { profileId })`
- HostTerminal: on mount listen `listen('terminal-output-'+id)`, write to xterm; onData → `gateway_terminal_write`; fit + resize observer → `gateway_terminal_resize`
- After first output **or** 800ms timeout, once: `write("source ~/.outgate/path.sh && outgate on\n")` (guard with `injectedRef`)

Tab bar:

```tsx
<button onClick={() => setCenterTab('hosts')}>Hosts</button>
{status?.sessions.map(s => (
  <button key={s.profileId} className={...}>
    {nameOf(s.profileId)}
    <span onClick={(e) => { e.stopPropagation(); disconnect(s.profileId); }}>×</span>
  </button>
))}
```

Closing × → `gateway_disconnect({ profileId })` (closes PTY + tunnel).  
Disconnect from details → same; then `setCenterTab('hosts')`.

Details column: render only when `centerTab === 'hosts'`; adjust `.shell` grid to 2 columns when terminal focused (`grid-template-columns: nav 1fr` without details).

- [ ] **Step 1: npm install xterm packages**

- [ ] **Step 2: HostTerminal component**

- [ ] **Step 3: App tab bar + connect/disconnect wiring**

- [ ] **Step 4: Manual acceptance** against spec §7 items 1–5, 8

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(ui): Termius-style terminal tabs with scoped outgate on inject"
```

---

### Task 9: Polish + acceptance sweep

**Files:**
- Modify as needed: inject timing, Windows no-window, empty Network state, CSS
- Update: `docs/superpowers/specs/2026-08-01-outgate-terminal-design.md` status → 实现中/已实现

- [ ] **Step 1: Checklist from spec §7** — mark each pass/fail in commit message body or PR note

- [ ] **Step 2: Ensure Logs/Network host switch works with 2 sessions**

- [ ] **Step 3: Confirm external SSH has no HTTP_PROXY after desktop connect (scope B)**

- [ ] **Step 4: Commit**

```bash
git commit -m "chore: polish terminal/network UX and update spec status"
```

---

## Spec coverage (self-review)

| Spec item | Task |
|-----------|------|
| Per-host local ports | 1, 4 |
| Network CONNECT/SOCKS logs + Host filter | 2, 3, 5, 6 |
| Logs Host filter | 5, 6 |
| Embedded terminal + inject outgate on | 7, 8 |
| No global auto-inject | 8 (inject only via PTY write); do not change path.sh auto-on |
| Tab close = disconnect | 8 |
| Details only on Hosts tab | 8 |
| Multi-host tabs | 8 |
| Mac/Win OpenSSH | 7, 9 |

## Placeholder scan

No TBD steps; concrete signatures and file paths included.

## Type consistency

- `NetworkLogEntry` / camelCase serde aligns with frontend `tsMs`, `profileId`.
- Event name pattern: `terminal-output-{profileId}` (document in Task 7–8; keep identical).
- Disconnect always stops proxy + tunnel + PTY for that `profile_id`.
