# Tauri v2 桌面端基础 Demo 设计

**日期：** 2026-07-25  
**状态：** 待用户审阅  
**目标用户：** 零基础 / 未做过桌面端开发  

## 1. 背景与目标

在空仓库中实现一个可在 **macOS** 与 **Windows** 上运行的最基础桌面 Demo，技术栈为 **Tauri v2 + React + Vite**，并通过 **GitHub Releases** 支持自动更新。同时提供一份面向零基础的说明文档（主文档为仓库根目录 `README.md`）。

### 成功标准

- 本地可执行 `npm run tauri dev` 打开桌面窗口
- 界面显示应用名、当前版本、「检查更新」按钮与状态反馈
- 正式安装包可通过 GitHub Releases 检查并安装更新（需正确配置签名密钥与 CI）
- README 覆盖：环境准备、本地运行、签名、发版、验证更新

### 非目标（本期不做）

- 登录、数据库、多窗口、系统托盘、复杂业务页
- Linux 支持
- 苹果 / 微软应用商店上架
- 付费代码签名证书的完整生产配置（文档中说明：本地可跑；对外广泛分发再单独配置）

## 2. 术语说明（避免「后端」误解）

本应用是**桌面程序**，不是必须依赖自建业务服务器的网站。

| 名称 | 含义 |
|------|------|
| 界面层 | 窗口内的 React 页面 |
| 本机原生层 | 本机上的 Tauri/Rust 进程：创建窗口、调用系统能力、执行更新 |
| 更新源 | GitHub Releases：存放安装包与更新清单，仅在检查/下载更新时访问 |

## 3. 架构

```
界面层（React + Vite）
  - 展示版本、触发「检查更新」、显示进度/结果
        │
        │ Tauri API / plugin-updater
        ▼
本机原生层（Tauri v2 + Rust）
  - 窗口与权限（capabilities）
  - 校验签名、下载、安装更新
        │
        │ HTTPS
        ▼
GitHub Releases
  - 平台安装包（macOS / Windows）
  - 更新清单（由发版流程生成）
  - 客户端用公钥校验，私钥仅保存在 CI Secrets
```

发版由 **GitHub Actions** 完成：推送版本 tag（如 `v0.1.1`）→ 构建双端 → 上传 Release → 写出更新清单。

## 4. 界面与功能

### 界面元素

- 应用标题：`Desktop Demo`
- 当前版本号（来自应用配置）
- 简短说明文案
- 主按钮：检查更新
- 状态区：已是最新 / 发现新版本 / 下载中 / 失败原因

### 行为

- 启动后显示主窗口
- 点击「检查更新」调用官方 updater 插件流程
- 有新版本则下载，并按官方流程提示重启安装

### 应用标识（可后续修改）

- 显示名：`Desktop Demo`
- Bundle identifier：`com.example.desktop-demo`

## 5. 技术选型

| 项 | 选择 | 理由 |
|----|------|------|
| 壳框架 | Tauri v2 | 包体小、官方 updater、文档较新 |
| 界面 | React + Vite | 用户指定；便于后续扩展 |
| 更新托管 | GitHub Releases | 配置简单、与官方示例一致 |
| 脚手架 | create-tauri-app + 官方 updater 插件 | 可复现、易跟文档 |

## 6. 仓库结构（目标形态）

```
desktop_app/
├── README.md                      # 零基础主说明文档
├── docs/superpowers/specs/        # 本设计文档
├── package.json
├── src/                           # React 界面
│   ├── App.tsx
│   └── main.tsx
├── src-tauri/                     # 本机原生层
│   ├── tauri.conf.json
│   ├── capabilities/
│   └── src/
└── .github/workflows/
    └── release.yml                # tag 触发构建与发布
```

## 7. 开发与发版流程

### 本地开发

1. 安装 Node.js、Rust；（Windows 另需 C++ 构建工具等，详见 README）
2. `npm install`
3. `npm run tauri dev` → 弹出窗口

### 自动更新发版

1. 生成更新签名密钥对：公钥写入 Tauri 配置；私钥放入 GitHub Secrets（禁止提交私钥）
2. 提升 `package.json` 与 `tauri.conf.json` 中的版本号
3. 创建并推送 git tag（如 `v0.1.1`）
4. Actions 构建 macOS / Windows 产物并发布到 GitHub Releases
5. 旧版客户端检查更新 → 发现新版本 → 下载安装

### 验证注意

- `tauri dev` 开发模式无法完整等价于生产更新路径
- 完整验证需安装正式构建产物，再发布更高版本进行对比

## 8. 文档交付

根目录 `README.md` 面向零基础，按顺序覆盖：

1. 这是什么 / 和网站的区别  
2. Mac / Windows 环境准备  
3. 安装依赖并本地运行  
4. 项目结构一眼看懂  
5. 更新签名密钥与 GitHub Secrets  
6. 如何打 tag 发版  
7. 如何验证自动更新  
8. 常见问题  

## 9. 风险与约束

- 本机 Rust 工具链需满足 Tauri v2 要求（过旧需升级）
- 无付费代码签名时，Windows/macOS 可能出现「未知开发者」提示；不影响本 demo 学习路径
- GitHub 在部分网络环境下需代理才能完成 Releases 下载；属环境问题，文档中提示
- 更新私钥泄露会导致恶意更新风险；文档强调 Secrets 管理

## 10. 实现顺序（获批后）

1. 用官方脚手架初始化 Tauri v2 + React + Vite 项目  
2. 接入 updater 插件与最小 UI  
3. 配置 GitHub Actions 发版工作流  
4. 撰写 README 说明文档  
5. 本地能 `tauri dev` 启动作为完成门槛；发版/更新路径以文档 + 配置就绪为准（完整端到端依赖用户的 GitHub 仓库与 Secrets）
