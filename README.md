# Desktop Demo

## 1. 这是什么

这是一个可以安装到电脑上的小型桌面应用示例，使用 Tauri v2、React 和 TypeScript 编写。打开应用后，可以看到当前版本，也可以点击“检查更新”按钮下载并安装新版本。

即使你没有桌面应用开发经验，也可以按照本文一步一步完成本地运行、GitHub 自动打包和自动更新配置。

## 2. 和「网站后端」的区别

- 网站后端通常运行在服务器上，浏览器通过网络请求它。
- 本项目主要运行在用户自己的电脑上：React 负责界面，Tauri/Rust 负责创建系统窗口并调用桌面能力。
- `npm run tauri dev` 会同时启动网页开发服务和桌面窗口；它不是把后端部署到公网。
- 自动更新需要从 GitHub Release 下载安装包，因此检查更新时仍然需要网络。

## 3. 环境准备（macOS / Windows 分列；写明 Rust ≥ 1.77.2）

两种系统都需要：

1. 安装 [Node.js LTS](https://nodejs.org/)。安装后在终端执行 `node -v` 和 `npm -v`，能看到版本号即表示成功。
2. 安装 Rust，且版本必须为 **Rust ≥ 1.77.2**。推荐从 [rustup.rs](https://rustup.rs/) 安装，然后执行 `rustc --version` 检查版本。
3. 安装 Git，并准备一个 GitHub 账号。

### macOS

1. 在终端执行 `xcode-select --install`，安装 Apple 命令行开发工具。
2. 如果刚安装完工具仍提示许可问题，执行 `sudo xcodebuild -license accept`。

### Windows

1. 安装 [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)。
2. 安装时勾选“使用 C++ 的桌面开发”，并确保包含 MSVC、Windows SDK 和 C++ CMake 工具。
3. Windows 10 需要安装 WebView2 Runtime；Windows 11 通常已经自带。

安装完成后，如果终端仍找不到新命令，请关闭并重新打开终端。

## 4. 安装依赖并本地运行（`npm install`、`npm run tauri dev`）

先用终端进入本项目根目录，再依次执行：

```bash
npm install
npm run tauri dev
```

第一次运行需要下载并编译 Rust 依赖，等待时间可能较长。看到 Desktop Demo 窗口后即表示运行成功。修改 `src/` 中的界面代码并保存，窗口会自动刷新；在终端按 `Ctrl+C` 可以停止程序。

## 5. 项目结构说明

```text
desktop_app/
├── src/                    React 界面代码
│   ├── App.tsx             主界面和检查更新逻辑
│   └── App.css             界面样式
├── src-tauri/              Tauri/Rust 桌面端代码
│   ├── src/                Rust 入口和插件注册
│   ├── Cargo.toml          Rust 包与依赖配置
│   └── tauri.conf.json     应用名称、版本、打包和更新配置
├── .github/workflows/
│   └── release.yml         GitHub Actions 自动打包与发版流程
├── package.json            前端依赖、脚本和版本
└── README.md               本说明文档
```

## 6. 配置自动更新（生成密钥、填写 `pubkey`、替换 `OWNER/REPO`、GitHub Secrets）

自动更新安装包必须经过签名。签名用到一对密钥：私钥只用于发版，不能提交到 Git；公钥放进应用中，用来确认更新包确实由你发布。

### 6.1 生成密钥

在终端执行以下命令，并按提示设置一个密码：

```bash
npm run tauri signer generate -- -w ~/.tauri/desktop-demo.key
```

命令会生成私钥 `~/.tauri/desktop-demo.key`，并在终端显示公钥。请把私钥和密码分别备份到安全位置，不要把私钥提交到仓库，也不要发送给别人。

### 6.2 修改 Tauri 配置

打开 `src-tauri/tauri.conf.json`：

1. 将 `plugins.updater.pubkey` 中的 `REPLACE_WITH_TAURI_SIGNER_PUBLIC_KEY` 替换为刚才生成的公钥。
2. 将更新地址里的 `OWNER/REPO` 替换为真实 GitHub 仓库，例如仓库地址是 `https://github.com/alice/desktop-demo`，则地址应为 `https://github.com/alice/desktop-demo/releases/latest/download/latest.json`。

**在真实自动更新能够工作之前，必须先替换 `tauri.conf.json` 中的 `OWNER/REPO` 和 `REPLACE_WITH_TAURI_SIGNER_PUBLIC_KEY`。** 占位值不会指向有效更新，也无法验证签名。

### 6.3 添加 GitHub Secrets

进入 GitHub 仓库的 **Settings → Secrets and variables → Actions → New repository secret**，添加：

- `TAURI_SIGNING_PRIVATE_KEY`：内容是 `~/.tauri/desktop-demo.key` 私钥文件的完整文本。
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`：内容是生成密钥时设置的密码。

Secret 名称必须完全一致。GitHub Actions 自带的 `GITHUB_TOKEN` 无需手动添加。

## 7. 如何发版（改版本号、打 `v*` tag、Actions、draft Release）

假设要发布 `0.2.0`：

1. 将 `package.json`、`src-tauri/tauri.conf.json` 和 `src-tauri/Cargo.toml` 中的版本号都改为 `0.2.0`。
2. 提交版本号修改并推送到 GitHub。
3. 创建并推送以 `v` 开头的 tag：

   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

4. 打开 GitHub 仓库的 **Actions** 页面，查看 `release` 工作流。它会为 macOS 和 Windows 构建、签名并上传安装包。
5. 工作流成功后，进入 **Releases**。新 Release 默认是 **draft（草稿）**；检查安装包、签名文件和 `latest.json` 都已生成，再点击 **Publish release** 正式发布。

tag 应与应用版本一致，并且必须匹配 `v*`，否则不会自动触发该工作流。也可以在 Actions 页面手动运行工作流。

## 8. 如何验证自动更新（安装旧版 → 发新版 → 检查更新）

开发模式不能完整模拟已安装应用的更新过程，请按以下顺序验证：

1. 先发布并安装一个旧版本，例如 `0.1.0`。不要只运行 `npm run tauri dev`。
2. 按上一节发布更高版本，例如 `0.2.0`，并确认 GitHub Actions 成功、draft Release 已正式发布。
3. 启动电脑上已经安装的旧版，确认界面显示旧版本号。
4. 点击“检查更新”，等待下载和安装完成。
5. 应用重启后，确认界面显示新版本号。

如果应用提示“已是最新版本”，请检查新版本号是否确实更高、Release 是否已发布，以及 Release Assets 中是否存在 `latest.json`。

## 9. 常见问题（dev 测不全、未知开发者、GitHub 网络、私钥丢失）

### 为什么 `npm run tauri dev` 里测不全自动更新？

自动更新针对已打包、签名并安装的应用。开发模式适合调界面和基础逻辑，但不能代替“安装旧版 → 发布新版 → 检查更新”的完整测试。

### macOS 或 Windows 提示“未知开发者”怎么办？

更新签名只保证更新包来自本项目，并不等同于操作系统的代码签名。正式分发时，macOS 还需要 Apple Developer ID 签名与公证，Windows 通常需要受信任的代码签名证书。学习测试时可以按照系统提示手动允许应用，但不要让用户对来源不明的程序这样做。

### 无法连接 GitHub 或检查更新超时怎么办？

先在浏览器中打开配置的 `latest.json` 地址，确认网络可访问且文件存在；再检查 `OWNER/REPO` 是否正确、Release 是否已正式发布。公司代理、防火墙或地区网络限制也可能阻止访问 GitHub。

### 私钥丢失了怎么办？

丢失的私钥无法恢复。旧版应用只信任原公钥，因此换一对密钥后，旧版通常无法通过自动更新迁移到使用新公钥的版本。请妥善备份私钥和密码；如果已经丢失，通常需要生成新密钥、更新应用中的公钥，并让用户手动下载安装一次新版本。
