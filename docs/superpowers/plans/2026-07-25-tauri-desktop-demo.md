# Tauri Desktop Demo Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在空仓库中搭建 Tauri v2 + React + Vite 桌面 Demo，支持 macOS/Windows，经 GitHub Releases 自动更新，并提供零基础 README。

**Architecture:** 界面层为 React；本机原生层为 Tauri v2（含 updater / process 插件）；发版用 GitHub Actions + tauri-action，产物与 `latest.json` 上传到 GitHub Releases。

**Tech Stack:** Tauri v2、React、Vite、TypeScript、`@tauri-apps/plugin-updater`、`@tauri-apps/plugin-process`、GitHub Actions（`tauri-apps/tauri-action`）

## Global Constraints

- 平台：仅 macOS + Windows（CI matrix 不含 Linux）
- 应用显示名：`Desktop Demo`
- Bundle identifier：`com.example.desktop-demo`
- 初始版本：`0.1.0`
- 更新端点形态：`https://github.com/<OWNER>/<REPO>/releases/latest/download/latest.json`（实现时用占位符，README 说明如何替换）
- Rust 最低版本：`1.77.2`（当前机器若为 1.69 必须先 `rustup update`）
- 不要提交更新私钥；私钥只进 GitHub Secrets
- 用户未明确要求时不要 `git commit`（计划里的 Commit 步骤改为「暂存变更，等用户要求再提交」）
- 主说明文档写在仓库根目录 `README.md`（中文、面向零基础）

## File Structure

| 路径 | 职责 |
|------|------|
| `package.json` / `vite.config.ts` / `index.html` / `tsconfig*.json` | 前端工程与脚本 |
| `src/main.tsx` | React 入口 |
| `src/App.tsx` | 主界面：版本展示 + 检查更新 |
| `src/App.css` | 简单样式 |
| `src-tauri/tauri.conf.json` | 窗口、标识、updater 公钥与 endpoints、`createUpdaterArtifacts` |
| `src-tauri/capabilities/default.json` | `updater:default`、`process:default` 等权限 |
| `src-tauri/src/lib.rs` | 注册 updater、process 插件 |
| `src-tauri/Cargo.toml` | Rust 依赖 |
| `.github/workflows/release.yml` | Mac/Windows 构建并发布 Release |
| `README.md` | 零基础操作说明 |
| `docs/superpowers/specs/2026-07-25-tauri-desktop-demo-design.md` | 已批准设计（只读参考） |

---

### Task 1: 升级 Rust 并用官方脚手架初始化项目

**Files:**
- Create: 脚手架生成的整套前端 + `src-tauri` 文件（在仓库根目录）
- Modify: 无（空仓库）

**Interfaces:**
- Consumes: 无
- Produces: 可运行的 `npm run tauri dev` 基线；`package.json` scripts 含 `dev`、`build`、`tauri`

- [ ] **Step 1: 升级 Rust 到 >= 1.77.2**

```bash
rustup update stable
rustc --version
```

Expected: 版本号 ≥ `1.77.2`

- [ ] **Step 2: 用 create-tauri-app 非交互创建项目到当前目录**

工作目录：`/Users/zhanglei/project/desktop_app`

若目录非空（已有 `docs/`），优先在临时目录生成再移入文件，或使用官方 CLI 支持的「当前目录」参数。推荐命令（按 create-tauri-app 当前帮助为准，若 flag 变化以 `--help` 为准）：

```bash
npm create tauri-app@latest . -- --template react-ts --manager npm --yes
```

若因已有 `docs/` 失败：

```bash
npm create tauri-app@latest _scaffold -- --template react-ts --manager npm --yes
# 将 _scaffold 内文件移到仓库根，保留 docs/，删除 _scaffold
```

- [ ] **Step 3: 安装依赖并验证开发模式能启动**

```bash
npm install
npm run tauri dev
```

Expected: 弹出桌面窗口；可手动关闭。若失败，根据报错安装 Xcode CLT / WebView2 等（写入后续 README）。

- [ ] **Step 4: 暂存（不提交，除非用户要求）**

```bash
git status
```

---

### Task 2: 应用标识与最小主界面

**Files:**
- Modify: `src-tauri/tauri.conf.json`（productName、identifier、version、窗口标题）
- Modify: `package.json`（`name`、`version` 为 `0.1.0`）
- Modify: `src/App.tsx`、`src/App.css`
- Modify: `src/main.tsx`（若脚手架已够用可不动）

**Interfaces:**
- Consumes: Task 1 脚手架
- Produces: UI 展示标题 `Desktop Demo`；通过 `@tauri-apps/api/app` 的 `getVersion()` 显示版本字符串

- [ ] **Step 1: 修改 `src-tauri/tauri.conf.json` 关键字段**

确保至少包含（其余字段保留脚手架默认）：

```json
{
  "productName": "Desktop Demo",
  "version": "0.1.0",
  "identifier": "com.example.desktop-demo",
  "app": {
    "windows": [
      {
        "title": "Desktop Demo",
        "width": 720,
        "height": 480,
        "resizable": true
      }
    ]
  }
}
```

（实际 JSON 路径以脚手架 v2 结构为准：`app.windows` 或顶层等价字段。）

- [ ] **Step 2: 重写 `src/App.tsx` 为最小界面（先不做更新逻辑）**

```tsx
import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import "./App.css";

function App() {
  const [version, setVersion] = useState("…");
  const [status, setStatus] = useState("尚未检查更新");

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion("未知"));
  }, []);

  return (
    <main className="page">
      <h1>Desktop Demo</h1>
      <p className="lead">这是 Tauri v2 桌面端基础示例</p>
      <p className="meta">当前版本：{version}</p>
      <button type="button" disabled>
        检查更新（下一任务启用）
      </button>
      <p className="status" role="status">
        {status}
      </p>
    </main>
  );
}

export default App;
```

- [ ] **Step 3: 写简单 `src/App.css`（清晰可读即可，非营销落地页）**

```css
:root {
  font-family: "Segoe UI", "PingFang SC", "Hiragino Sans GB", sans-serif;
  color: #1a1a1a;
  background: #f3f5f7;
}

.page {
  max-width: 420px;
  margin: 64px auto;
  padding: 24px;
  text-align: center;
}

.lead {
  color: #555;
}

.meta {
  font-variant-numeric: tabular-nums;
}

button {
  margin-top: 16px;
  padding: 10px 18px;
  font-size: 15px;
  cursor: pointer;
}

button:disabled {
  cursor: not-allowed;
  opacity: 0.6;
}

.status {
  margin-top: 16px;
  min-height: 1.5em;
  color: #333;
}
```

- [ ] **Step 4: 验证**

```bash
npm run tauri dev
```

Expected: 窗口标题为 Desktop Demo；页面显示版本（开发态可能为 `0.1.0`）。

---

### Task 3: 接入 updater 与 process 插件（配置层）

**Files:**
- Modify: `src-tauri/Cargo.toml`、`src-tauri/src/lib.rs`（或 `main.rs`）
- Modify: `src-tauri/tauri.conf.json`（`bundle.createUpdaterArtifacts`、`plugins.updater`）
- Modify: `src-tauri/capabilities/default.json`
- Modify: `package.json`（JS 插件依赖）

**Interfaces:**
- Consumes: Task 2 应用标识
- Produces: 原生层已注册 updater/process；capabilities 含 `updater:default` 与 `process:default`；`plugins.updater.pubkey` 先用占位说明，Task 5/README 教用户生成真实密钥；`endpoints` 使用可替换的 GitHub latest.json URL

- [ ] **Step 1: 用 CLI 添加插件**

```bash
npm run tauri add updater
npm run tauri add process
```

Expected: Cargo 与 npm 依赖自动写入；`lib.rs` 出现 `.plugin(tauri_plugin_updater::init())` 与 process 注册。

- [ ] **Step 2: 确认 `src-tauri/capabilities/default.json` 权限**

```json
{
  "permissions": [
    "core:default",
    "updater:default",
    "process:default"
  ]
}
```

（保留脚手架已有权限项，追加上述两项。）

- [ ] **Step 3: 配置 updater（公钥先占位）**

在 `tauri.conf.json` 增加/合并：

```json
{
  "bundle": {
    "createUpdaterArtifacts": true
  },
  "plugins": {
    "updater": {
      "pubkey": "REPLACE_WITH_TAURI_SIGNER_PUBLIC_KEY",
      "endpoints": [
        "https://github.com/OWNER/REPO/releases/latest/download/latest.json"
      ],
      "windows": {
        "installMode": "passive"
      }
    }
  }
}
```

说明：占位 `pubkey` 会导致真正验签失败；本地 `dev` 仍应能启动。README 要求用户用 `npm run tauri signer generate` 替换，并把 `OWNER/REPO` 改成真实仓库。

- [ ] **Step 4: 验证仍能启动**

```bash
npm run tauri dev
```

Expected: 窗口正常打开，无插件初始化 panic。

---

### Task 4: 前端接通「检查更新」流程

**Files:**
- Modify: `src/App.tsx`

**Interfaces:**
- Consumes: `@tauri-apps/plugin-updater` 的 `check()`；`Update.downloadAndInstall(onEvent?)`；`@tauri-apps/plugin-process` 的 `relaunch()`；`getVersion()`
- Produces: 可点击的检查更新；状态文案覆盖：检查中 / 已是最新 / 发现版本并下载 / 失败信息

- [ ] **Step 1: 实现 `handleCheckUpdate`**

将 `src/App.tsx` 中按钮改为可用，逻辑如下：

```tsx
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

async function handleCheckUpdate(
  setStatus: (s: string) => void,
  setBusy: (b: boolean) => void,
) {
  setBusy(true);
  setStatus("正在检查更新…");
  try {
    const update = await check();
    if (!update) {
      setStatus("已是最新版本");
      return;
    }
    setStatus(`发现新版本 ${update.version}，开始下载…`);
    let downloaded = 0;
    let contentLength = 0;
    await update.downloadAndInstall((event) => {
      switch (event.event) {
        case "Started":
          contentLength = event.data.contentLength ?? 0;
          break;
        case "Progress":
          downloaded += event.data.chunkLength;
          if (contentLength > 0) {
            const pct = Math.min(100, Math.round((downloaded / contentLength) * 100));
            setStatus(`下载中 ${pct}%`);
          } else {
            setStatus(`已下载 ${downloaded} 字节`);
          }
          break;
        case "Finished":
          setStatus("下载完成，准备安装…");
          break;
      }
    });
    setStatus("更新已安装，正在重启…");
    await relaunch();
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    setStatus(`检查更新失败：${message}`);
  } finally {
    setBusy(false);
  }
}
```

按钮：`disabled={busy}`，文案「检查更新」。

- [ ] **Step 2: 开发态冒烟**

```bash
npm run tauri dev
```

点击「检查更新」。Expected：因 endpoint/pubkey/未发版，状态区出现失败或「已是最新」类反馈，**不应白屏或崩溃**。

---

### Task 5: GitHub Actions 发版工作流（仅 Mac + Windows）

**Files:**
- Create: `.github/workflows/release.yml`

**Interfaces:**
- Consumes: `createUpdaterArtifacts: true`；Secrets：`TAURI_SIGNING_PRIVATE_KEY`、可选 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`、`GITHUB_TOKEN`
- Produces: tag / `workflow_dispatch` 触发后构建并上传 Release（含 updater 产物与 `latest.json`，由 tauri-action 处理）

- [ ] **Step 1: 写入 `.github/workflows/release.yml`**

```yaml
name: release

on:
  workflow_dispatch:
  push:
    tags:
      - "v*"

jobs:
  publish-tauri:
    permissions:
      contents: write
    strategy:
      fail-fast: false
      matrix:
        include:
          - platform: macos-latest
            args: --target aarch64-apple-darwin
          - platform: macos-latest
            args: --target x86_64-apple-darwin
          - platform: windows-latest
            args: ""
    runs-on: ${{ matrix.platform }}
    steps:
      - uses: actions/checkout@v4

      - name: setup node
        uses: actions/setup-node@v4
        with:
          node-version: lts/*
          cache: npm

      - name: install Rust stable
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.platform == 'macos-latest' && 'aarch64-apple-darwin,x86_64-apple-darwin' || '' }}

      - name: Rust cache
        uses: swatinem/rust-cache@v2
        with:
          workspaces: "./src-tauri -> target"

      - name: install frontend dependencies
        run: npm install

      - uses: tauri-apps/tauri-action@v1
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
        with:
          tagName: v__VERSION__
          releaseName: Desktop Demo v__VERSION__
          releaseBody: 下载 Assets 中的安装包进行安装。自动更新使用 latest.json。
          releaseDraft: true
          prerelease: false
          args: ${{ matrix.args }}
```

- [ ] **Step 2: 静态检查 YAML 存在且 matrix 无 Linux**

```bash
grep -n "windows-latest\|macos-latest\|ubuntu" .github/workflows/release.yml
```

Expected: 有 macos/windows；无 ubuntu 构建 job。

---

### Task 6: 零基础 README

**Files:**
- Create/Overwrite: `README.md`

**Interfaces:**
- Consumes: 前述全部路径与 Secrets 名称
- Produces: 中文说明，覆盖设计文档第 8 节清单

- [ ] **Step 1: 撰写 `README.md`，必须包含以下章节标题**

1. 这是什么  
2. 和「网站后端」的区别  
3. 环境准备（macOS / Windows 分列；写明 Rust ≥ 1.77.2）  
4. 安装依赖并本地运行（`npm install`、`npm run tauri dev`）  
5. 项目结构说明  
6. 配置自动更新（生成密钥、填写 `pubkey`、替换 `OWNER/REPO`、GitHub Secrets）  
7. 如何发版（改版本号、打 `v*` tag、Actions、draft Release）  
8. 如何验证自动更新（安装旧版 → 发新版 → 检查更新）  
9. 常见问题（dev 测不全、未知开发者、GitHub 网络、私钥丢失）  

密钥生成命令写死为：

```bash
npm run tauri signer generate -- -w ~/.tauri/desktop-demo.key
```

Secrets 名称写死为：`TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。

- [ ] **Step 2: 通读 README，确认无 TBD/占位句「以后再写」**

---

### Task 7: 最终验收

**Files:**
- 无新文件（修复验收中发现的问题）

- [ ] **Step 1: 本地开发启动**

```bash
npm run tauri dev
```

Expected: 窗口打开；显示 Desktop Demo 与版本；按钮可点。

- [ ] **Step 2: 确认关键配置存在**

```bash
test -f src-tauri/tauri.conf.json
test -f .github/workflows/release.yml
test -f README.md
grep -q 'createUpdaterArtifacts' src-tauri/tauri.conf.json
grep -q 'updater:default' src-tauri/capabilities/default.json
grep -q 'plugin-updater' package.json
```

Expected: 全部成功。

- [ ] **Step 3: 向用户汇报**

说明：完整「旧版→新版自动更新」需用户自备 GitHub 仓库、填入真实 `pubkey`/endpoint、配置 Secrets 后打 tag；本仓库交付的是可运行 demo + CI 模板 + 文档。

---

## Spec Coverage Self-Review

| 规格项 | 对应任务 |
|--------|----------|
| Tauri v2 + React + Vite | Task 1 |
| Mac + Windows | Task 5 matrix；README |
| 最小 UI + 检查更新 | Task 2、4 |
| GitHub Releases 自动更新 | Task 3、5、6 |
| 零基础文档 | Task 6 |
| 非目标（登录/Linux/商店） | 未实现，符合 |
| Rust 版本门槛 | Task 1、README |

## Placeholder Scan

- `OWNER/REPO` 与 `REPLACE_WITH_TAURI_SIGNER_PUBLIC_KEY` 为**有意占位**，README 教用户替换；实现时不要编造虚假公钥。
- 无「TBD」「以后再写」类步骤。
