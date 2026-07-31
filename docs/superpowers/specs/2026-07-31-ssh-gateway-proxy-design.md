# SSH 网关代理设计规格

**日期：** 2026-07-31  
**状态：** 实现中  
**范围：** 将 Desktop Demo 扩展为 Windows 侧网关（多服务器配置 + 本地代理 + SSH `-R`）

## 1. 目标

- 无外网服务器经应用层代理出网：Windows 内网 SSH 登录服务器，`-R` 暴露本机 HTTP/SOCKS 代理
- Windows 代理仅绑 `127.0.0.1`，无需管理员、无需对外开入站
- 前端：多套服务器 Profile；操作流为 **选择 → 连接 → 开启代理**
- 每套 Profile 可配置 `noProxy`（默认含 `127.0.0.1` / `localhost` / `::1`）
- 隧道可自动重连；可关闭代理 / 断开以回退

## 2. 非目标

- 不改服务器路由表 / NAT / 透明流量劫持
- 不做流量审计与拆包
- 不同时连接多台服务器
- 不内嵌 SSH 协议栈（使用系统 `ssh`）

## 3. 架构

```
[服务器应用] --HTTP_PROXY--> 127.0.0.1:17890
                    |
              SSH RemoteForward (-R)
                    |
[Windows Tauri] 本地 HTTP:17890 / SOCKS5:17891 --> 公网
```

命中 `NO_PROXY` 的目标由服务器应用直连（不进隧道）。

## 4. 相位

| 相位 | 含义 |
|------|------|
| Idle | 未连接 |
| Connected | 本地代理监听 + SSH `-R` 已建立 |
| ProxyOn | 已远程执行 enable-proxy（服务器应用层代理已落地） |
| Reconnecting | 隧道断开，正在退避重连 |

## 5. Profile 字段

`id`, `name`, `host`, `port`, `user`, `identityFile`, `remoteHttpPort`, `remoteSocksPort`, `autoReconnect`, `noProxy[]`, `updatedAt`

持久化：`{app_config_dir}/gateway-profiles.json`

## 6. 服务器侧代理落地

仅环境变量：远程执行 `enable-proxy.sh` 生成 `~/.config/offline-gateway/env.sh`。  
不修改 apt / pip / npm / git / Docker / bashrc。使用前在目标 shell 中 `source` 该文件。

## 7. 验收

见实现计划验收标准：多 Profile、三步操作、`noProxy` 生效、断开/关闭可回退。
