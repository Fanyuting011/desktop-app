# OutGate UI 设计（Termius 风格）

**日期：** 2026-07-31  
**状态：** 已按用户确认实现（Hosts/Logs 三栏；无代理开关；Connect 异步不卡 UI）  
**参考：** Termius Hosts 三栏布局（侧栏导航 + 主机卡片区 + 右侧详情）

## 1. 目标

把现有「表单堆叠」改成专业 SSH 客户端式布局，同时保留 OutGate 核心能力：

- 多服务器 Profile
- 选择 → 连接（隧道）→ 开启代理（`outgate on`）
- 上游代理（Clash 等）
- 日志与状态

**不做（本期）：** 完整 SFTP、多终端 Tab、串口、Telnet、群组协作分享。

## 2. 整体布局

```text
┌─────────────────────────────────────────────────────────────┐
│  OutGate   [Hosts]  [· 139.x 已连接]              🔔  v0.1.5 │
├────────┬──────────────────────────────┬─────────────────────┤
│        │  🔍 Find host or user@host…  │  Host Details    ∨  │
│ Hosts● │  [+ New]  [Connect]          │                     │
│ Keys   │                              │  名称 / 分组 / 标签  │
│ Tunnel │  Groups                      │  上游代理            │
│ Logs   │  ┌────┐ ┌────┐               │  SSH 主机/端口      │
│        │  │生产│ │测试│               │  用户 / 密码 / 密钥  │
│        │  └────┘ └────┘               │  远程端口 17890/91  │
│        │                              │  不走代理 NO_PROXY  │
│        │  Hosts                       │  ☑ 自动重连         │
│        │  ┌────┐ ┌────┐ ┌────┐        │                     │
│        │  │阿里│ │…   │ │…   │        │  状态：已连接        │
│        │  └────┘ └────┘ └────┘        │  [断开] [开启代理]  │
│        │                              │  [连接] 主按钮      │
├────────┴──────────────────────────────┴─────────────────────┤
│ 日志条 / 可展开日志面板                                      │
└─────────────────────────────────────────────────────────────┘
```

三栏比例约：`72px | 1fr | 320px`（窄窗时右侧详情改为抽屉）。

## 3. 左栏导航（精简版 Termius）

| 项 | 作用 |
|----|------|
| **Hosts** | 默认页：分组 + 主机卡片 |
| **Keys** | 私钥路径快捷管理（可选本期简化为详情内字段） |
| **Tunnel** | 当前隧道状态、上游代理、本地监听端口 |
| **Logs** | 完整网关日志 |

选中项：白底卡片 + 左侧蓝色竖条（参考 Termius）。

## 4. 中间：Hosts 工作区

### 4.1 顶栏

- 搜索框：按名称 / host / user 过滤  
- `+ New host`：新建 Profile  
- 快捷 `Connect`：对当前选中主机执行「连接」

### 4.2 Groups（一期轻量）

- 默认分组：`Default`、可自建（如「阿里云」「测试」）  
- 卡片显示：组名 + Host 数量  
- Profile 增加可选字段 `group: string`

### 4.3 Host 卡片网格

每张卡片：

- 图标（服务器）
- 标题：`name`（如「阿里云」）
- 副标题：`ssh, {user}@{host}` 或 `host`
- 状态点：灰=未连 / 蓝=隧道已连 / 绿=代理已开
- 选中：蓝色描边（同 Termius）

同一时刻仅一台可处于 Connected/ProxyOn（保持现有约束）。

## 5. 右侧：Host Details

结构对齐 Termius「Host Details」：

1. **Label** — Profile 名称  
2. **Parent** — 分组下拉  
3. **Upstream** — 上游代理（Clash `127.0.0.1:7890`），全局或按主机；连接时生效  
4. **Address** — host + SSH port  
5. **Credentials** — user / password（眼睛显隐）/ identity file  
6. **OutGate ports** — remote HTTP / SOCKS（默认 17890/17891）  
7. **No Proxy** — 多行文本  
8. **Auto reconnect** — 开关  

底部主操作（对应三步，文案更贴近 Termius 的大 Connect）：

| 按钮 | 相位 | 行为 |
|------|------|------|
| **Connect** | Idle | 隧道 + 部署 CLI/config |
| **Disconnect** | Connected/ProxyOn | 断开并清理远程端口 |
| **Enable Proxy** | Connected | `outgate on` |
| **Disable Proxy** | ProxyOn | `outgate off` |

状态摘要一行：`Idle | Tunnel up | Proxy on` + 最近错误。

帮助微文案（详情底部小字）：

> Server: `outgate on` / `outgate off` · `source ~/.outgate/env.sh`

## 6. 视觉规范

| Token | 值 |
|-------|-----|
| 页面底 | `#F4F6F8` |
| 卡片/面板 | `#FFFFFF` |
| 主色 | `#2F6BFF`（连接按钮、选中描边） |
| 次要文字 | `#6B7280` |
| 成功/已连接 | `#12B76A` |
| 圆角 | 卡片 12px，输入 10px |
| 字体 | 系统 UI 栈（与桌面原生接近，不追求花哨展示字体） |

动效（轻量）：

- 选中卡片描边 150ms  
- 右侧详情切换 fade 120ms  
- Connect 成功状态点颜色过渡  

## 7. 信息架构对照

| Termius | OutGate |
|---------|---------|
| Connect = 开 SSH 终端 | Connect = 开隧道网关 |
| Port Forwarding 独立页 | 折叠进 Tunnel + 自动 `-R` |
| Snippets | 不做；改为 Logs |
| 多会话 Tab | 一期仅状态条显示当前已连主机 |

## 8. 验收

- 首屏一眼能选主机、看详情、点 Connect  
- 三步状态在卡片圆点 + 右侧按钮上同时可读  
- 上游代理、NO_PROXY 仍可配  
- 窄屏可用（详情变抽屉）

## 9. 实现范围（确认后编码）

1. 重写 `App.tsx` / `App.css` 为三栏布局  
2. Profile 增加 `group`  
3. 不改网关 Rust 协议（仅 UI）
