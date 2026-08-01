# OutGate Health + Port Forwards + Files Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make proxy failures explainable in Network, add preset local `-L` previews (3000/8080/5432) with extensible `portForwards` data, and ship a lightweight Files panel over system `scp` (macOS + Windows OpenSSH).

**Architecture:** Classify errors when writing `NetworkLogEntry`. Persist `portForwards` on `GatewayProfile` and pass enabled rules into the existing `-N` SSH tunnel as `-L` flags; toggling presets updates the profile and respawns only that host’s data-plane tunnel. File transfer is a short-lived `scp` process sharing askpass/identity helpers, gated to one job per host.

**Tech Stack:** Tauri 2, React 19, Rust (`std::process` for ssh/scp), existing gateway modules, `@tauri-apps/plugin-opener`, add `@tauri-apps/plugin-dialog` for save/open pickers.

**Spec:** `docs/superpowers/specs/2026-08-01-outgate-health-forward-files-design.md`

## Global Constraints

- System OpenSSH only — no ControlMaster, no russh/ssh2 SFTP stack.
- No active health probes, no per-request timing timeline.
- No custom port-forward editor UI this iteration (presets only; data model is full rules).
- No remote directory tree, terminal drag-drop, or multi-job transfer queue UI.
- Deferred: SSH compression, first-connect wizard, CLI/desktop version alignment.
- macOS + Windows; resolve `ssh`/`scp` via PATH then Windows OpenSSH install dir.
- Prefer small modules under `src-tauri/src/gateway/`; keep React panels thin.
- Do not bump release tag unless the user asks; stay on current feature branch.
- Commit after each task; Chinese user-facing strings OK (match existing UI).

---

## File map

| File | Responsibility |
|------|----------------|
| `src-tauri/src/gateway/classify.rs` | `classify_network_error` → `(category, hint)` |
| `src-tauri/src/gateway/network_log.rs` | Add `category` / `hint` on `NetworkLogEntry` |
| `src-tauri/src/gateway/proxy.rs` | Call classify inside `push_network_log` |
| `src-tauri/src/gateway/ssh_bin.rs` | `resolve_ssh_bin` / `resolve_scp_bin` |
| `src-tauri/src/gateway/profiles.rs` | `PortForwardRule` + `port_forwards` on profile |
| `src-tauri/src/gateway/ssh_tunnel.rs` | Use resolved ssh bin; append `-L` for enabled rules |
| `src-tauri/src/gateway/manager.rs` | Preset toggle + tunnel respawn; transfer busy map |
| `src-tauri/src/gateway/transfer.rs` | `scp` upload/download helpers |
| `src-tauri/src/gateway/mod.rs` | Module exports |
| `src-tauri/src/lib.rs` | Register new commands |
| `src-tauri/Cargo.toml` / `capabilities/default.json` | `tauri-plugin-dialog` |
| `package.json` | `@tauri-apps/plugin-dialog` |
| `src/components/NetworkPanel.tsx` | Category, hint, highlight, fail-only, NO_PROXY note |
| `src/components/FilesPanel.tsx` | Files UI + drag-drop |
| `src/App.tsx` / `src/App.css` | Nav Files; Host Details preview chips |
| Spec status line | Mark plan ready / implemented as you go |

---

### Task 1: Network error classifier

**Files:**
- Create: `src-tauri/src/gateway/classify.rs`
- Modify: `src-tauri/src/gateway/mod.rs` (add `mod classify;`)
- Test: unit tests in `classify.rs`

**Interfaces:**
- Produces:  
  `pub fn classify_network_error(error: Option<&str>) -> (String, Option<String>)`  
  - `None` → `("ok".into(), None)`  
  - Some → `(category, Some(hint))` with categories: `upstream` \| `dns` \| `timeout` \| `refused` \| `tunnel` \| `blocked` \| `other`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::classify_network_error;

    #[test]
    fn ok_when_no_error() {
        assert_eq!(classify_network_error(None), ("ok".into(), None));
    }

    #[test]
    fn upstream_from_chinese_or_connect() {
        let (c, h) = classify_network_error(Some("上游 HTTP 代理 CONNECT 失败: 503"));
        assert_eq!(c, "upstream");
        assert!(h.unwrap().contains("Clash") || h.unwrap().contains("上游"));
    }

    #[test]
    fn dns_from_lookup() {
        let (c, _) = classify_network_error(Some("failed to lookup address information"));
        assert_eq!(c, "dns");
    }

    #[test]
    fn timeout_category() {
        let (c, _) = classify_network_error(Some("connection timed out"));
        assert_eq!(c, "timeout");
    }

    #[test]
    fn refused_category() {
        let (c, _) = classify_network_error(Some("Connection refused (os error 61)"));
        assert_eq!(c, "refused");
    }

    #[test]
    fn other_fallback() {
        let (c, h) = classify_network_error(Some("weird glitch"));
        assert_eq!(c, "other");
        assert!(h.unwrap().contains("Logs") || h.unwrap().contains("日志"));
    }
}
```

- [ ] **Step 2: Run tests — expect FAIL (module missing)**

```bash
cd src-tauri && cargo test --lib classify:: -- --nocapture
```

Expected: compile error / no module

- [ ] **Step 3: Implement `classify.rs`**

```rust
/// Map proxy dial errors to (category, hint). No active probing.
pub fn classify_network_error(error: Option<&str>) -> (String, Option<String>) {
    let Some(raw) = error else {
        return ("ok".into(), None);
    };
    let lower = raw.to_lowercase();

    let (category, hint) = if raw.contains("上游")
        || lower.contains("upstream")
        || lower.contains("socks5 握手")
        || lower.contains("socks5 connect")
    {
        (
            "upstream",
            "上游代理不可达或拒绝。请确认 Clash 等已开启，且应用里上游地址正确。",
        )
    } else if lower.contains("lookup")
        || lower.contains("name or service not known")
        || lower.contains("nodename nor servname")
        || lower.contains("no such host")
    {
        (
            "dns",
            "域名解析失败。检查 DNS，或改用 IP；若仅远程解析失败，查看服务器 DNS。",
        )
    } else if lower.contains("timed out") || lower.contains("timeout") {
        (
            "timeout",
            "连接超时。目标慢、链路不稳或被干扰；可重试，并检查上游是否正常。",
        )
    } else if lower.contains("connection refused") || lower.contains("actively refused") {
        (
            "refused",
            "连接被拒绝。对端未监听该端口，或地址/端口写错。",
        )
    } else if lower.contains("broken pipe")
        || lower.contains("tunnel")
        || raw.contains("隧道")
    {
        (
            "tunnel",
            "隧道或本地代理可能已断开。请回到 Hosts 重新 Connect。",
        )
    } else if lower.contains("connection reset") || lower.contains("reset by peer") {
        (
            "blocked",
            "连接被重置。可能被墙或中间设备干扰；可查看上游代理日志。",
        )
    } else {
        (
            "other",
            "请求失败。请展开原始错误，并到 Logs 查看网关/SSH 详情。",
        )
    };

    (category.into(), Some(hint.into()))
}
```

Wire `mod classify;` in `mod.rs`.

- [ ] **Step 4: Run tests — expect PASS**

```bash
cd src-tauri && cargo test --lib classify:: -- --nocapture
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/gateway/classify.rs src-tauri/src/gateway/mod.rs
git commit -m "feat(gateway): classify network proxy errors into categories"
```

---

### Task 2: Persist category/hint on network logs

**Files:**
- Modify: `src-tauri/src/gateway/network_log.rs`
- Modify: `src-tauri/src/gateway/proxy.rs` (`push_network_log`)
- Test: update helper `entry(...)` in `network_log.rs` tests

**Interfaces:**
- Consumes: `classify_network_error`
- Produces: `NetworkLogEntry { category: String, hint: Option<String>, ... }` (serde camelCase)

- [ ] **Step 1: Extend struct and fix compile of tests**

Add fields:

```rust
pub category: String,
pub hint: Option<String>,
```

Update every `NetworkLogEntry { ... }` construction in tests to include `category: "ok".into(), hint: None`.

- [ ] **Step 2: Update `push_network_log` in `proxy.rs`**

```rust
use super::classify::classify_network_error;

fn push_network_log(
    net_log: &NetworkLogBuffer,
    profile_id: &str,
    protocol: &str,
    target: &str,
    error: Option<String>,
) {
    let (category, hint) = classify_network_error(error.as_deref());
    net_log.push(NetworkLogEntry {
        id: uuid::Uuid::new_v4().to_string(),
        ts_ms: now_ms(),
        profile_id: profile_id.to_string(),
        protocol: protocol.to_string(),
        target: target.to_string(),
        ok: error.is_none(),
        error,
        category,
        hint,
    });
}
```

- [ ] **Step 3: Run unit tests**

```bash
cd src-tauri && cargo test --lib network_log:: classify:: -- --nocapture
```

Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/gateway/network_log.rs src-tauri/src/gateway/proxy.rs
git commit -m "feat(gateway): attach category and hint to network log entries"
```

---

### Task 3: NetworkPanel explainability UI

**Files:**
- Modify: `src/components/NetworkPanel.tsx`
- Modify: `src/App.css` (highlight + note styles)
- Modify: `src/App.tsx` only if `NetworkPanel` needs `noProxy` for selected host — prefer pass optional `noProxySummary: string` prop from App using active/draft profile

**Interfaces:**
- Consumes: `NetworkLogEntry.category`, `.hint`
- Produces: UI only

- [ ] **Step 1: Extend TS interface and table**

```ts
interface NetworkLogEntry {
  id: string;
  tsMs: number;
  profileId: string;
  protocol: string;
  target: string;
  ok: boolean;
  error: string | null;
  category: string;
  hint: string | null;
}
```

Add state `failOnly: boolean`. Compute `displayRows` filtered; find latest fail id:

```ts
const latestFailId = [...displayRows].reverse().find((r) => !r.ok)?.id;
```

Table columns: Time | Host | Proto | Target | Category | Result.  
Fail cell: show `hint` as secondary line or `title={`${r.hint ?? ""}\n${r.error ?? ""}`}`.  
Row class: `className={r.id === latestFailId ? "network-row fail-latest" : !r.ok ? "network-row fail" : "network-row"}`.

Top note:

```tsx
<p className="network-note">
  服务器应用经 HTTP_PROXY/ALL_PROXY 进隧道；命中 NO_PROXY 则直连、不出现在本页。
  {noProxySummary ? ` 当前 NO_PROXY：${noProxySummary}` : null}
</p>
```

Toggle: checkbox「仅失败」.

- [ ] **Step 2: CSS**

```css
.network-note { font-size: 12px; color: #6b7280; margin: 0 0 12px; line-height: 1.4; }
.network-row.fail-latest { background: #fef2f2; box-shadow: inset 3px 0 0 #ef4444; }
.network-row.fail { background: #fff7f7; }
.network-hint { display: block; font-size: 11px; color: #6b7280; max-width: 280px; }
```

- [ ] **Step 3: Manual smoke**

```bash
npm run tauri dev
```

Connect a host, generate a failed request (bad upstream or bad host); confirm category/hint and latest-fail highlight.

- [ ] **Step 4: Commit**

```bash
git add src/components/NetworkPanel.tsx src/App.css src/App.tsx
git commit -m "feat(ui): show network failure categories, hints, and highlight"
```

---

### Task 4: Profile `portForwards` model

**Files:**
- Modify: `src-tauri/src/gateway/profiles.rs`
- Test: serde round-trip in `profiles.rs` or small `#[cfg(test)]`

**Interfaces:**
- Produces:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortForwardRule {
    pub id: String,
    pub enabled: bool,
    #[serde(default = "default_loopback")]
    pub local_host: String,
    pub local_port: u16,
    #[serde(default = "default_loopback")]
    pub remote_host: String,
    pub remote_port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

fn default_loopback() -> String { "127.0.0.1".into() }
```

On `GatewayProfile`:

```rust
#[serde(default)]
pub port_forwards: Vec<PortForwardRule>,
```

Update `new_blank` to `port_forwards: vec![]`.

Helper:

```rust
pub fn preset_forward(port: u16) -> PortForwardRule {
    PortForwardRule {
        id: Uuid::new_v4().to_string(),
        enabled: true,
        local_host: "127.0.0.1".into(),
        local_port: port,
        remote_host: "127.0.0.1".into(),
        remote_port: port,
        label: Some(format!("{port}")),
    }
}
```

- [ ] **Step 1: Write test — old JSON without field still loads**

```rust
#[test]
fn missing_port_forwards_defaults_empty() {
    let raw = r#"{"id":"1","name":"t","host":"h","port":22,"user":"u","identityFile":"","remoteHttpPort":17890,"remoteSocksPort":17891,"autoReconnect":true,"noProxy":[],"updatedAt":"x"}"#;
    let p: GatewayProfile = serde_json::from_str(raw).unwrap();
    assert!(p.port_forwards.is_empty());
}
```

(If `password` required with default, include `"password":""` as in real schema.)

- [ ] **Step 2: Implement fields + test PASS**

```bash
cd src-tauri && cargo test --lib profiles:: -- --nocapture
```

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/gateway/profiles.rs
git commit -m "feat(gateway): add extensible portForwards on profiles"
```

---

### Task 5: Resolve OpenSSH binaries

**Files:**
- Create: `src-tauri/src/gateway/ssh_bin.rs`
- Modify: `src-tauri/src/gateway/mod.rs`
- Modify: `src-tauri/src/gateway/ssh_tunnel.rs` — `Command::new(resolve_ssh_bin()?)` instead of `"ssh"`
- Modify: `src-tauri/src/gateway/terminal.rs` — same for `CommandBuilder::new`
- Test: unit tests for candidate list (path existence optional)

**Interfaces:**
- Produces:  
  `pub fn resolve_ssh_bin() -> Result<PathBuf, String>`  
  `pub fn resolve_scp_bin() -> Result<PathBuf, String>`

Logic:
1. If `which`/`where` finds executable on PATH (`ssh` / `scp`), use it.  
2. Else on Windows, try `%SystemRoot%\System32\OpenSSH\ssh.exe` and `scp.exe`, and `Program Files\OpenSSH\`.  
3. Else `Err("未找到 OpenSSH 的 ssh/scp，请安装 OpenSSH 客户端并确保在 PATH 中")`.

Keep helpers small — use `std::process::Command` with `where ssh` on Windows / `command -v ssh` on Unix, or check `Path::new(...).is_file()`.

- [ ] **Step 1: Failing test for error message shape when forced missing** (optional: test pure `candidates()` function)

```rust
pub(crate) fn ssh_candidates() -> Vec<PathBuf> { /* ... */ }

#[test]
fn ssh_candidates_include_name_ssh() {
    let c = ssh_candidates();
    assert!(c.iter().any(|p| p.file_name().unwrap() == "ssh" || p.file_name().unwrap() == "ssh.exe"));
}
```

- [ ] **Step 2: Implement + wire into `base_ssh_cmd` and terminal spawn**

```rust
fn base_ssh_cmd(...) -> Result<Command, String> {
    let mut cmd = Command::new(resolve_ssh_bin()?);
    ...
    Ok(cmd)
}
```

Propagate `Result` through callers (`verify_ssh_auth`, `SshTunnel::spawn`, remote scripts). Match existing error style.

- [ ] **Step 3: `cargo test --lib ssh_bin::` and `cargo build`**

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/gateway/ssh_bin.rs src-tauri/src/gateway/mod.rs \
  src-tauri/src/gateway/ssh_tunnel.rs src-tauri/src/gateway/terminal.rs
git commit -m "feat(gateway): resolve ssh/scp binaries including Windows OpenSSH path"
```

---

### Task 6: Apply `-L` in tunnel spawn

**Files:**
- Modify: `src-tauri/src/gateway/ssh_tunnel.rs` (`SshTunnel::spawn`)

**Interfaces:**
- Consumes: `profile.port_forwards` where `enabled`
- Produces: ssh args `-L {local_host}:{local_port}:{remote_host}:{remote_port}` per rule

- [ ] **Step 1: After existing `-R` args, before dest**

```rust
for fw in profile.port_forwards.iter().filter(|f| f.enabled) {
    cmd.arg("-L").arg(format!(
        "{}:{}:{}:{}",
        fw.local_host.trim(),
        fw.local_port,
        fw.remote_host.trim(),
        fw.remote_port
    ));
    logs.push(format!(
        "本地转发 -L {}:{} → {}:{}",
        fw.local_host, fw.local_port, fw.remote_host, fw.remote_port
    ));
}
```

When spawn fails and stderr/err string contains `bind` / `Address already in use` / `forwarding failed`, map message to:

`本地端口 {n} 已被占用，请关掉占用进程或换端口` when a single enabled local port is identifiable; else keep ssh stderr.

- [ ] **Step 2: Build**

```bash
cd src-tauri && cargo build
```

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/gateway/ssh_tunnel.rs
git commit -m "feat(gateway): apply enabled portForwards as SSH -L on tunnel"
```

---

### Task 7: Preset command + tunnel respawn

**Files:**
- Modify: `src-tauri/src/gateway/manager.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: optional unit test for preset merge logic extracted as pure fn

**Interfaces:**
- Produces:

```rust
pub fn set_port_forward_preset(&self, profile_id: String, port: u16, enabled: bool) -> Result<GatewayStatus, String>
```

Allowed ports: `3000 | 8080 | 5432` only (reject others with clear error).

Algorithm:
1. Load profile from store; find rule where `local_port==port && remote_port==port && remote_host=="127.0.0.1"` (and local_host loopback).  
2. If missing and `enabled`: push `preset_forward(port)`. If missing and `!enabled`: no-op OK.  
3. If present: set `enabled`.  
4. `profiles.upsert` + update `sessions.get_mut(id).profile` copy if connected.  
5. If session connected/reconnecting: **respawn data-plane only** — kill current `session.tunnel`, `SshTunnel::spawn` with updated profile and same `local_http/local_socks`, **do not** close terminal, **do not** stop proxy. On failure, set `last_error` and return Err (leave phase Connected if old tunnel already dead — best-effort restore previous profile forwards only if easy; otherwise surface error).

Extract respawn helper:

```rust
fn respawn_tunnel_keep_proxy(&self, profile: &GatewayProfile) -> Result<(), String>
```

Register:

```rust
#[tauri::command]
fn gateway_set_port_forward_preset(
    state: tauri::State<'_, GatewayState>,
    profile_id: String,
    port: u16,
    enabled: bool,
) -> Result<GatewayStatus, String> {
    state.set_port_forward_preset(profile_id, port, enabled)
}
```

Also expose forwards via existing `gateway_list_profiles` (field already on profile) — no separate list command required unless convenient.

- [ ] **Step 1: Implement pure merge helper + test**

```rust
pub fn apply_preset(forwards: &mut Vec<PortForwardRule>, port: u16, enabled: bool) {
    // ... as above
}
```

- [ ] **Step 2: Wire manager + command + build**

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/gateway/manager.rs src-tauri/src/gateway/profiles.rs src-tauri/src/lib.rs
git commit -m "feat(gateway): toggle local preview presets and respawn -L tunnel"
```

---

### Task 8: Host Details preview chips UI

**Files:**
- Modify: `src/App.tsx` (details section)
- Modify: `src/App.css`

**Interfaces:**
- Consumes: `draft.portForwards`, `gateway_set_port_forward_preset`, `openUrl` from `@tauri-apps/plugin-opener`

- [ ] **Step 1: Ensure frontend `GatewayProfile` type includes `portForwards`**

```ts
portForwards?: Array<{
  id: string;
  enabled: boolean;
  localHost: string;
  localPort: number;
  remoteHost: string;
  remotePort: number;
  label?: string | null;
}>;
```

- [ ] **Step 2: UI block in details**

```tsx
const PRESET_PORTS = [3000, 8080, 5432] as const;

function presetEnabled(draft: GatewayProfile, port: number) {
  return (draft.portForwards ?? []).some(
    (f) =>
      f.enabled &&
      f.localPort === port &&
      f.remotePort === port &&
      (f.remoteHost === "127.0.0.1" || f.remoteHost === "localhost"),
  );
}

async function togglePreset(port: number) {
  if (!draft) return;
  const enabled = !presetEnabled(draft, port);
  const st = await invoke<GatewayStatus>("gateway_set_port_forward_preset", {
    profileId: draft.id,
    port,
    enabled,
  });
  setStatus(st);
  // refresh profiles list from gateway_list_profiles
}
```

Chips + enabled list + for 3000/8080 when enabled and live: button `打开` → `openUrl(\`http://127.0.0.1:${port}\`)`.

Label chip state: selected when enabled (connected or not).

- [ ] **Step 3: Manual test** — toggle 8080 while connected; confirm log line for `-L`; open browser.

- [ ] **Step 4: Commit**

```bash
git add src/App.tsx src/App.css
git commit -m "feat(ui): local preview port chips in host details"
```

---

### Task 9: SCP transfer backend

**Files:**
- Create: `src-tauri/src/gateway/transfer.rs`
- Modify: `src-tauri/src/gateway/manager.rs` (busy set + methods)
- Modify: `src-tauri/src/gateway/mod.rs`, `lib.rs`
- Test: argv builder unit tests (no real network)

**Interfaces:**
- Produces:

```rust
pub struct TransferStatus {
    pub profile_id: Option<String>,
    pub state: String, // "idle" | "running"
    pub detail: Option<String>,
}

// Manager:
pub fn transfer_upload(&self, profile_id: String, local_path: String, remote_path: String) -> Result<(), String>
pub fn transfer_download(&self, profile_id: String, remote_path: String, local_path: String) -> Result<(), String>
pub fn transfer_status(&self) -> TransferStatus
```

Rules:
- Profile must be in `sessions` (connected); else Err「请先 Connect 该主机」.  
- If `transfer_busy` contains profile_id → Err「请等待当前传输完成」.  
- Build `scp` like ssh common args: `-P port` (note capital P for scp), `-i`, askpass env, `-o` same auth options.  
- Upload dir: if local is dir, add `-r`. Download: always pass `-r` if you cannot know; or `-r` always safe for single file on OpenSSH. Spec: support both — use `-r` when local path is directory on upload; on download use `-r` always OR detect trailing semantics — **use `-r` for both upload-dir and all downloads** to match spec.  
- Remote spec: `{user}@{host}:{remote_path}`  
- Clear busy in `finally` / defer guard.

```rust
pub fn scp_upload_args(profile: &GatewayProfile, local: &str, remote: &str, recursive: bool) -> Result<(PathBuf, Vec<String>), String>
```

- [ ] **Step 1: Tests for args include `-P` and dest**

```rust
#[test]
fn upload_args_include_port_and_remote() {
    // build minimal GatewayProfile in test
    let (bin, args) = scp_upload_args(&profile, "/tmp/a", "~/a", false).unwrap();
    assert!(args.iter().any(|a| a == "-P"));
    assert!(args.iter().any(|a| a.contains("@") && a.contains(":")));
}
```

- [ ] **Step 2: Implement transfer + commands**

```rust
#[tauri::command]
fn gateway_transfer_upload(...) -> Result<(), String> { state.transfer_upload(...) }

#[tauri::command]
fn gateway_transfer_download(...) -> Result<(), String> { ... }

#[tauri::command]
fn gateway_transfer_status(...) -> TransferStatus { state.transfer_status() }
```

Run blocking scp in `spawn_blocking` from async commands (same pattern as connect).

- [ ] **Step 3: `cargo test --lib transfer::` + build**

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/gateway/transfer.rs src-tauri/src/gateway/manager.rs \
  src-tauri/src/gateway/mod.rs src-tauri/src/lib.rs
git commit -m "feat(gateway): scp upload/download with per-host busy lock"
```

---

### Task 10: Files panel + dialog plugin

**Files:**
- Create: `src/components/FilesPanel.tsx`
- Modify: `src/App.tsx`, `src/App.css`
- Modify: `package.json`, `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs` (`.plugin(tauri_plugin_dialog::init())`), `src-tauri/capabilities/default.json` (`dialog:default`)

**Interfaces:**
- Consumes: transfer commands; `open`/`save` from `@tauri-apps/plugin-dialog`

- [ ] **Step 1: Add dependencies**

```bash
npm install @tauri-apps/plugin-dialog
cd src-tauri && cargo add tauri-plugin-dialog
```

Capability:

```json
"dialog:default"
```

- [ ] **Step 2: Implement `FilesPanel`**

Props: `profiles`, `status` (to know live ids), `active`.

UI:
- Host `<select>` (default first live, else first profile)
- Remote path `<input>` default `~`
- Note:「同名文件将被覆盖」
- Buttons: 上传文件、上传目录、下载（另存为）
- Drop zone `onDragOver/onDrop` → upload dropped paths via Tauri path from File objects — in Tauri 2 webview, use `file.path` if available (Tauri extends File), else show「请使用上传按钮」fallback for browser-only.

Disable controls when host not live or `status.state==="running"` for that host.

Poll `gateway_transfer_status` every 1s while panel active.

- [ ] **Step 3: Wire nav**

```ts
type Nav = "hosts" | "logs" | "network" | "files";
// Files under Network
{nav === "files" && <FilesPanel ... />}
```

- [ ] **Step 4: Manual smoke upload/download**

- [ ] **Step 5: Commit**

```bash
git add package.json package-lock.json src-tauri/Cargo.toml src-tauri/Cargo.lock \
  src-tauri/src/lib.rs src-tauri/capabilities/default.json \
  src/components/FilesPanel.tsx src/App.tsx src/App.css
git commit -m "feat(ui): Files panel with scp upload, download, and drag-drop"
```

---

### Task 11: Spec status + acceptance checklist pass

**Files:**
- Modify: `docs/superpowers/specs/2026-08-01-outgate-health-forward-files-design.md` status →「已实现（代码完成，人工 E2E 待验）」when code done
- Optional: short README blurb under gateway steps for 本地预览 / Files（only if README already documents gateway UX)

- [ ] **Step 1: Run automated tests**

```bash
cd src-tauri && cargo test --lib
npm run build
```

- [ ] **Step 2: Manual acceptance (from spec §6)**

1. Bad upstream / bad DNS → category + hint + highlight  
2. Enable 8080 → browser reaches remote :8080; disable / disconnect clears access; rule persists  
3. Occupy local 8080 → error mentions port  
4. Files upload file, upload dir, drag-drop, download; second transfer blocked while busy  
5. Windows OpenSSH path (if available)  
6. macOS regression Connect/terminal/proxy  

- [ ] **Step 3: Commit doc status**

```bash
git add docs/superpowers/specs/2026-08-01-outgate-health-forward-files-design.md
git commit -m "docs: mark health/forward/files spec implemented pending E2E"
```

---

## Spec coverage self-check

| Spec item | Task |
|-----------|------|
| category/hint classify | 1–2 |
| Network UI highlight / fail-only / NO_PROXY note | 3 |
| portForwards model | 4 |
| resolve ssh/scp Windows | 5 |
| `-L` on tunnel | 6 |
| preset API + respawn | 7 |
| chips + open URL | 8 |
| scp transfer + busy | 9 |
| Files panel + drag-drop | 10 |
| Acceptance / status | 11 |
| Non-goals (no ControlMaster, no probe, no custom editor) | respected — not scheduled |

## Placeholder scan

No TBD steps; signatures named consistently (`set_port_forward_preset`, `classify_network_error`, `resolve_ssh_bin`, `transfer_upload`).
