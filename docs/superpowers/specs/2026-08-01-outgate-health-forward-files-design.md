# OutGate：失败可解释 + 本地预览转发 + 轻量传文件

**日期：** 2026-08-01  
**状态：** 设计已确认，待写实现计划  
**前置：** 多 Host 隧道、内嵌终端、Network 日志（`2026-08-01-outgate-terminal-design.md`）  
**方案：** 系统 SSH 增强（不用 ControlMaster / 自研 SSH）

**本期优先；搁置：** SSH 压缩与连接复用、首连向导、CLI/桌面版本对齐。

## 1. 目标与非目标

### 1.1 目标

1. **失败可解释**：Network 日志带失败分类 + 简短「可能原因 / 下一步」；最近失败高亮。  
2. **本地预览转发**：已连接 Host 可一键打开常见 `-L`（3000 / 8080 / 5432）；规则持久化在 Profile，结构支持日后自定义。  
3. **轻量传文件**：左栏 Files 面板：上传文件/目录、下载文件/目录；支持拖到面板；走系统 `scp`。

### 1.2 非目标

- 主动健康探测（Clash/公网 ping）、单请求耗时时间线  
- ControlMaster、自研 SSH/SFTP 协议栈  
- 完整远程资源管理器、终端内拖拽、多任务队列管理器  
- 自定义转发规则编辑 UI（只预留数据；本期用预设生成规则）  
- 额外业务向 `-R`、动态 SOCKS 业务转发  
- SSH 压缩 / 首连向导 / CLI 版本对齐（另案）

### 1.3 与现有约束

仍是每 Host：一条 `-N` 数据面隧道 + 一条交互终端。传文件是**第三条短生命周期** `scp` 进程。平台：macOS 与 Windows（依赖本机 OpenSSH 客户端）。

## 2. 失败分类与 Network UI

### 2.1 数据扩展

`NetworkLogEntry` 新增（向后兼容）：

| 字段 | 含义 |
|------|------|
| `category` | 枚举字符串；失败时必填；成功为 `ok` |
| `hint` | 一句中文：可能原因 + 建议下一步；成功可为 `null` |
| `error` | 保留原始错误串（调试用） |

### 2.2 分类枚举

| category | 判定 | hint 方向 |
|----------|------|-----------|
| `ok` | 拨号/握手成功 | — |
| `upstream` | 文案含「上游」或连上游失败 | 检查 Clash / 上游地址 |
| `dns` | 解析失败 / failed to lookup | 检查 DNS 或换 IP |
| `timeout` | timed out / timeout | 重试或检查上游 |
| `refused` | connection refused | 对端未监听或端口错 |
| `tunnel` | 会话已断 / 本地代理已停时尚有请求（若能标到） | 请重新 Connect |
| `blocked` | 仅有较明确重置类信号时 | 可能被墙；看上游日志 |
| `other` | 其余 | 展示原始 error，建议看 Logs |

在 `push_network_log` 内用纯函数 `classify(error, context)` 完成；**不做**主动探测。`blocked` 信号不明时归 `other`，避免误导。

### 2.3 「为何走/不走代理」

不做实时策略引擎。Network 页顶固定说明条（可带当前 Host 的 `noProxy` 摘要）：

> 服务器应用经 `HTTP_PROXY`/`ALL_PROXY` 进隧道；命中该 Host 的 NO_PROXY 则直连、不进本页。本页只记录进入本机代理的流量。

### 2.4 UI

- 表格增加 **Category** 列；Fail 行展示 `hint`（及原始 `error`）。  
- **最近失败高亮**：当前过滤结果中最新一条 `ok=false` 行醒目样式。  
- 筛选：`全部 | 仅失败`。  
- 隧道级错误仍主要在 Logs。

## 3. 端口转发（本地预览）

### 3.1 数据模型

挂在 `GatewayProfile`（缺省 `[]`，旧 Profile 兼容）：

```text
portForwards: [{
  id: string
  enabled: boolean
  localHost: string      // 默认 "127.0.0.1"
  localPort: number
  remoteHost: string     // 默认 "127.0.0.1"
  remotePort: number
  label?: string
}]
```

本期 UI 只用预设；结构按完整自定义规则预留。

### 3.2 本期 UI

- 入口：Host Details / 已连接态 — **本地预览** 芯片：`3000` / `8080` / `5432`。  
- 点击：若无「本机端口 = 远程同端口、remoteHost=127.0.0.1」规则则追加并 `enabled=true`；已有则 toggle `enabled`。  
- 展示已启用短列表 + 关闭。  
- 未连接时可点芯片预选，**Connect 时一并带上**。  
- 启用成功后提供 `打开 http://127.0.0.1:{port}`（5432 不提供 http 打开）。  
- **不做**自定义端口输入框。

### 3.3 生效方式

- 启用规则追加到该 Host `-N` 隧道：  
  `-L 127.0.0.1:{localPort}:{remoteHost}:{remotePort}`  
- 与现有 `-R` 同一进程；`ExitOnForwardFailure=yes` 保持。  
- **改规则 → 重拉该 Host 数据面隧道**（对齐现有 reconnect：尽量保留本地代理端口与交互终端）。  
- 本机端口占用 → 明确错误「本地端口 {n} 已被占用…」。  
- Disconnect 后转发失效；规则仍存 Profile，下次 Connect 按 `enabled` 带上。

## 4. 轻量传文件（Files）

### 4.1 入口与能力

- 左栏 **Files**（Network 下方）。  
- 面板内 Host 下拉；**未连接时传输按钮禁用**（须先 Connect）。  
- 远程路径输入（默认 `~`；按 Host 记在内存）。  
- 上传文件 / 上传目录；**拖拽到面板**上传。  
- 下载文件或目录（均 `scp` / `scp -r`）。  
- **不做**：远程目录树、队列 UI、断点续传、终端内拖拽。

### 4.2 实现

- 系统 `scp`，复用与隧道相同的认证参数（端口、`-i`、askpass）。  
- 同一 Host **同时只跑一个传输**；忙时拒绝并提示等待。  
- 进度：至少「传输中…」+ 路径；结束成功/失败反馈。  
- 覆盖：说明同名将被覆盖。

### 4.3 Windows

- 依赖本机 OpenSSH（`ssh` / `scp`）。  
- 共享 `resolve_ssh_bin()` / `resolve_scp_bin()`：PATH →（Windows）`%SystemRoot%\System32\OpenSSH\` 等候选；找不到或只有残缺安装时返回明确中文错误。  
- 本地路径为 Windows 绝对路径；远程为 Linux 风格路径。  
- 打开预览用系统默认浏览器。

## 5. 架构与 API

```text
Host Connect
  └─ ssh -N  -R (代理)  +  -L (enabled portForwards)
  └─ ssh -t  (终端，不变)
  └─ scp …   (Files，按需)

本机 HTTP/SOCKS ──push──► NetworkLog{ category, hint, error }
```

| Command | 作用 |
|---------|------|
| `gateway_get_network_logs` | 返回增加 `category` / `hint` |
| `gateway_set_port_forward_preset` | `{ profileId, port, enabled }`；已连接则重拉隧道 |
| `gateway_list_port_forwards` | 或经现有 profile 同步暴露规则 |
| `gateway_transfer_upload` | `{ profileId, localPath, remotePath }` |
| `gateway_transfer_download` | `{ profileId, remotePath, localPath }` |
| `gateway_transfer_status` | 可选：`idle \| running` + 最近错误 |

`portForwards` 随现有 Profile 持久化。

## 6. 验收

1. 关掉上游或访问坏域名时，Network 出现对应 category 与可读 hint；最新失败行高亮。  
2. 连接后启用 `8080`，本机可访问服务器上 8080 服务；关闭或 Disconnect 后不可访问；规则仍在 Profile。  
3. 本机端口占用时启用失败且错误含端口号。  
4. Files：上传文件、上传目录、拖拽上传、下载；忙时二次操作被拒绝或提示。  
5. Windows：标准 OpenSSH 可完成转发与 scp；无 `scp` 时错误可读。  
6. macOS 回归：Connect / 终端 / 代理 / Network 基本行为不坏。

## 7. 实现范围（确认后编码）

1. `network_log` + `proxy`：classify / hint；前端 NetworkPanel。  
2. Profile `portForwards`；`ssh_tunnel` 追加 `-L`；预设 API + Details UI。  
3. `resolve_ssh_bin` / `resolve_scp_bin`；transfer commands + Files 面板（含拖拽）。  
4. 手工 E2E：macOS + Windows（有 OpenSSH）各走一遍验收清单。
