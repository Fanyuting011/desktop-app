# OutGate 内嵌终端 + 按 Host 网络日志 设计规格

**日期：** 2026-08-01  
**状态：** 已实现（代码完成，人工 E2E 待验）  
**前置：** 多 Host 并行隧道（`feat/ssh-gateway-proxy` / v0.1.6）  
**相关：** `2026-07-31-ssh-gateway-proxy-design.md`、`2026-07-31-outgate-termius-ui-design.md`

## 1. 目标

1. **内嵌终端**：连接成功后自动打开该 Host 的终端 Tab，体验接近 Termius。  
2. **代理仅作用于内嵌终端（默认）**：PTY 就绪后注入 `source ~/.outgate/path.sh && outgate on`；**不做**登录 shell 全局自动注入。  
3. **外部 SSH** 若要用代理，需用户手动 `outgate on`。  
4. **Logs** 可按 Host 切换查看。  
5. **Network** 新分页（左栏位于 Logs 下方）：记录本机代理看到的 HTTP/SOCKS 目标，并按 Host 过滤。

## 2. 非目标（本期）

- 登录/bashrc 根据 `state=on` 自动给所有新 shell 注入代理  
- 同一 Host 多个 shell Tab、终端分屏、SFTP  
- 完整 HTTP 正文 / TLS 解密抓包  
- 自研 SSH 协议栈（继续用系统 `ssh`）  
- 透明代理 / TUN

## 3. 产品行为

### 3.1 连接与终端

| 动作 | 行为 |
|------|------|
| Connect 成功 | 建立该 Host 的本地代理 + SSH `-R`；部署 CLI；**打开并聚焦**该 Host 终端 Tab；PTY 就绪后注入 `source ~/.outgate/path.sh && outgate on` |
| Disconnect | 结束交互 SSH（PTY）+ 拆除隧道 + 停该 Host 本地代理；关闭对应终端 Tab；切回 **Hosts** |
| 关闭终端 Tab | **等同 Disconnect** |
| 多 Host | 每台已连接 Host 一个终端 Tab；隧道与 PTY 一一对应、互不干扰 |

**代理作用域（已确认 B）：**

- 注入只影响**当前内嵌终端**环境变量。  
- 不修改「所有新 shell 自动带代理」的行为。  
- Disconnect 时可远程执行一次 `outgate off`（清服务器侧 state/env 文件），减轻外部曾手动 `on` 的残留；**无法**清除其他已打开进程内存里的环境变量。

### 3.2 布局

```text
┌─ OutGate ──┬─ Hosts │ 阿里云 × │ 测试机 × ──────────────────┐
│ Hosts      │                                              │
│ Logs       │   [Hosts 卡片网格] 或 [当前终端 xterm]          │  Host Details
│ Network    │                                              │  ※ 仅 Hosts Tab
│            │                                              │
│ v / update │                                              │
└────────────┴──────────────────────────────────────────────┘
```

- 中间顶栏：`Hosts` | 各已连接 Host 的终端 Tab（标签为 Host 显示名）。  
- **仅当顶栏选中 Hosts 时**显示右侧 Host Details；终端 Tab 时不显示 Details（中间主区加宽）。  
- 左栏新增 **Network**，放在 **Logs** 下方。

### 3.3 Logs 分页

- 展示现有网关/SSH 文本日志。  
- 顶栏或下拉：**全部 | Host A | Host B | …**（仅列出有会话或近期有日志的 Host）。  
- 过滤规则：日志行带 `[Host名]` / `profile_id` 前缀或结构化字段；无法归属的进「全部」或「系统」。

### 3.4 Network 分页

- 记录本机代理观测到的目标，例如：  
  - HTTP：`CONNECT www.google.com:443`  
  - SOCKS5：解析后的 `host:port`  
- 字段：时间、Host、协议（http/socks）、目标、结果（ok / 失败原因摘要）。  
- 同样支持按 Host 切换（全部 | 各 Host）。  
- 保留最近 N 条（如 500～2000），可清空；不做持久化到磁盘（本期）。

## 4. 架构

### 4.1 每 Host 独立本地端口（网络日志归属）

当前共享 `17890/17891` 时，SSH `-R` 回连均来自本机 `127.0.0.1`，代理无法区分 Host。

**改为：**

- 连接时为每个 session 分配一对空闲本地端口 `(local_http, local_socks)`（例如从 `17890` 起向上探测绑定，或按 profile 稳定哈希落在一段端口范围并处理冲突）。  
- SSH：`-R 127.0.0.1:{remote_http}:127.0.0.1:{local_http}`（socks 同理）。  
- 部署到服务器的 `~/.outgate/config` 仍写该服务器自己的 **remote** 端口与 URL（对服务器透明）。  
- 上游 Clash 等：所有 session 的本地代理可共用同一 `upstream` 配置（与现 UI 一致）；第一个连接锁定上游，后续沿用（保持现行为或文档说明）。

```text
Server A  17890  --R-->  Local :17900  ──┐
Server B  17890  --R-->  Local :17902  ──┼--> 可选上游 :7890 --> 公网
                                         │
                              每端口代理打上 profile_id 标签写入 Network 日志
```

（服务器 remote 端口仍可都是 17890——那是各机自己的 loopback，不冲突。）

### 4.2 双通道 SSH

| 通道 | 用途 |
|------|------|
| `-N -R` 隧道 | 现有 `SshTunnel`，不提供 shell |
| 交互 `ssh` + PTY | 内嵌终端；`portable-pty`（或等价）spawn 系统 `ssh`；复用现有 askpass / 密钥逻辑 |

断开任一路径的产品语义：用户 Disconnect / 关 Tab → **两条都拆**。

### 4.3 前端终端

- **xterm.js**（+ fit addon）渲染。  
- Tauri 命令/事件：创建 session PTY、stdin 写入、stdout/stderr 推送、resize（rows/cols）、销毁。  
- 注入时机：检测到 shell 就绪（首屏 prompt 或短延时 + 一次写入），发送：  
  `source ~/.outgate/path.sh && outgate on\n`  
  注入命令可在 UI 回显（用户可见，便于排错）。

## 5. 后端接口（草案）

在现有 `gateway_*` 之上扩展（名称可调）：

| API | 说明 |
|-----|------|
| 现有 `connect` / `disconnect` | connect 成功后前端再调 `terminal_open`；或 connect 内一并拉起 PTY 并由事件通知 |
| `terminal_open(profile_id)` | 若未开则开 PTY |
| `terminal_write(profile_id, data)` | 用户输入 |
| `terminal_resize(profile_id, cols, rows)` | |
| `terminal_close` | 与 disconnect 合并亦可 |
| 事件 `terminal://output/{profile_id}` | 二进制或 base64 文本块 |
| `gateway_get_logs` | 增加可选 `profile_id` 过滤 |
| `gateway_get_network_logs` | `{ profile_id?, limit }` → 结构化条目 |
| `gateway_clear_network_logs` | 可选 |

Network 日志在 `proxy` 处理 CONNECT / SOCKS 请求时 `push`，条目带 `profile_id`（由该监听端口所属 session 决定）。

## 6. UI 状态

- `centerTab`: `'hosts' | { type: 'terminal', profileId }`  
- 左栏 `nav`: `'hosts' | 'logs' | 'network'`  
- `logHostFilter` / `networkHostFilter`: `'all' | profileId`  
- 已连接列表驱动顶栏终端 Tab；断开后移除 Tab 且若当前在该 Tab 则回到 Hosts。

## 7. 验收标准

1. 连接 Host A → 自动出现终端 Tab 并聚焦；注入后该终端内 `echo $HTTP_PROXY` 指向该机 remote 代理 URL。  
2. 另开系统 SSH 到同一服务器 → **默认没有** `HTTP_PROXY`（除非手动 `outgate on`）。  
3. 同时连 A、B → 两个终端 Tab；关 B 的 Tab → 仅 B 断开，A 隧道与终端仍在。  
4. Disconnect A 或关 A Tab → A 终端消失，回到 Hosts（若无其他终端选中）。  
5. 仅 Hosts 顶栏显示 Details；终端 Tab 无 Details。  
6. Logs 可按 Host 过滤；连两台时切换过滤结果不同。  
7. Network 页可见 `CONNECT`/SOCKS 目标；按 Host 过滤时只显示对应端口产生的条目。  
8. Mac / Windows 均可打开终端（依赖系统 OpenSSH）。

## 8. 风险与缓解

| 风险 | 缓解 |
|------|------|
| 密码/askpass 与 PTY 抢交互 | 隧道与终端复用同一套非交互 askpass；BatchMode/密钥优先 |
| 注入过早（shell 未就绪） | 短延时重试或等首次输出后再发 |
| 端口耗尽/冲突 | 绑定失败则递增重试；断开释放 |
| 上游变更 | 已有会话不热切上游；文案提示需全断后重连 |
| Windows 控制台闪窗 | PTY/ssh 创建时隐藏控制台窗口 |

## 9. 实现分期建议

1. **P0** 每 Host 独立本地端口 + Network 日志 API/UI + Logs 按 Host 过滤  
2. **P0** 内嵌终端（xterm + PTY）+ Connect 自动开 Tab + 注入 `outgate on`  
3. **P1** 关 Tab=Disconnect、Details 显隐、注入稳健性与 Win 隐藏控制台  

（P0 两项可并行，但终端依赖端口/session 模型稳定。）
