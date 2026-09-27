# gpt-Switch

轻量的 Codex 账号切换与 `config.toml` 编辑工具，支持 macOS 和 Windows。Tauri 2 + Rust + React，不启动代理服务。

<img src="assets/icon.svg" alt="Prism Relay icon" width="100" />

- 菜单栏／托盘左键展开账号列表，一键切换 `auth.json`。
- 添加 ChatGPT 登录、导入现有凭据和添加 API Key；同一身份的 Token 刷新自动回存。
- 独立 CodeMirror TOML 原文编辑器，支持高亮、查找、语法检查、保存冲突保护。
- 跟随系统深浅主题，关闭窗口后驻留；不会自动重启 Codex。

## 安装

从 [GitHub Releases](https://github.com/lich13/gpt-switch/releases) 下载 macOS Apple Silicon / Intel DMG 或 Windows x64 安装程序。macOS 包使用 ad-hoc 签名，Windows 包未配置商业代码签名；系统可能要求确认来自非商店开发者的应用。校验下载文件时使用 Release 中的 `SHA256SUMS`。

## 使用

首次启动导入当前有效账号。点击“添加账号”通过官方 Codex CLI 登录，或导入完整 `auth.json`。选择账号后仅替换认证文件，配置保持不变。现有 Codex 进程可能缓存凭据，需要自行重新打开。

ChatGPT 登录需要本机已安装 [官方 Codex CLI](https://developers.openai.com/codex/cli)。应用自动查找 PATH、常见安装目录和 macOS nvm；也可在设置中指定 CLI 路径。Windows npm 安装使用同目录 Codex JavaScript 入口和 Node.js。登录使用独立临时目录，不修改当前 Codex 登录；10 分钟未完成会取消。浏览器授权需本人完成，也支持设备码登录。

默认目录为 `CODEX_HOME` 或 `~/.codex`（Windows 为用户主目录中的 `.codex`），可在设置中修改。账号切换仅影响文件；`keyring` / `auto` / `ephemeral`、provider bearer token、环境变量或命令认证可能使用其他来源，应用会提示。

配置页按原文保存，保留注释、未知字段和换行。外部修改发生时保留未保存草稿并阻止覆盖；复制草稿、重新读取并合并后保存。删除列表中的账号不会注销当前 Codex。

## 本地数据

数据目录为 Tauri 的 `com.lich13.gpt-switch` 应用数据目录（macOS：`~/Library/Application Support/com.lich13.gpt-switch`；Windows：`%APPDATA%/com.lich13.gpt-switch`）。`accounts.json` 保存完整账号，`previous-auth.json`、`previous-config.toml` 各保留一份最近写入前的文件。凭据不上传，不写入日志，不进入前端账号列表；文件采用当前用户权限，**未加密**，应像原 `auth.json` 一样保护。

## 开发与验证

需要 Node.js 24、pnpm 11、Rust 1.94.1 和 [Tauri 平台依赖](https://v2.tauri.app/start/prerequisites/)。

```sh
pnpm install --frozen-lockfile
pnpm icons
pnpm test
pnpm build
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
pnpm tauri dev
```

`pnpm dev` 是标明“预览”的模拟界面，真实凭据只能在桌面应用中操作。`--smoke-test <绝对输出路径>` 使用独立临时账号和配置，验证 WebView、托盘、切换与配置不变后退出，不接触真实 Codex 目录。

`pnpm icons` 从原创 SVG 生成 9 档 PNG、黑白托盘图标、ICNS 和 ICO。首版发布通过 CI 构建三平台安装包及校验和；真实 OAuth 和操作系统托盘交互验收与自动冒烟测试分别记录。

MIT License。独立项目，与 OpenAI 无隶属关系。
