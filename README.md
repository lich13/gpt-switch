# gpt-Switch

轻量的 Codex 账号切换、TOML 编辑与本地 API 网关，支持 macOS 和 Windows。Tauri 2 + Rust + React。网关默认关闭。

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

## 本地网关

在“网关”添加 `base_url` 和 `experimental_bearer_token`，或从当前 Codex 配置导入。应用不会导入自身的受管 provider。启用会先绑定 `127.0.0.1:15722`，再添加独立 provider 并切换选择器；首次启用或停用后重新打开 Codex。供应商切换即时作用于新请求，进行中的流保持原连接。停止或正常退出会恢复受管字段；外部修改冲突时保留现场，修复后再次停止。异常退出后，下次启动先恢复。更换 Codex 目录前须先停用网关。

网关只转发发往供应商 `base_url` 的 API：未知路径、Responses、compact、models、搜索、图片和上传均使用同一原始载荷通道，包括 SSE 和 WebSocket。登录、插件和 MCP 的其他地址不经过网关。`base_url` 子路径按输入保留，不猜测 `/v1`；本地 `/v1` 前缀替换为上游 base 路径。重定向原样返回。Bearer 认证替换为上游 Token，连接头按 HTTP 规范处理，模型和业务参数不重写。

自动模式每次从 P1 开始，跳过熔断项，每家最多一次，默认最多四家。默认连续失败 4 次、错误率 60% / 最小 10 次、恢复等待 60 秒、半开 2 次成功；半开只放行一个探测。首字节 60 秒、流静默 120 秒、非流式总超时 600 秒。`400/405/406/413/414/415/422/501` 直接返回且不计故障；客户端取消为中性。有效 Retry-After 延长冷却。已向客户端提交输出后不重试，WebSocket 升级后不换商；已知 previous_response_id 固定原供应商，未知归属仅尝试一次。gzip / deflate / zstd 和磁盘暂存的 JSON 只流式读取路由归属，原始载荷不变；无法解析的 JSON 或未知压缩编码保守地只尝试一次。

请求最多 1 GiB，超过 2 MiB 使用权限受限的临时文件重放，完成或取消后清理。供应商和代理健康独立：代理连接或认证失败会跳过共享代理的其他候选；目标连接和 HTTP 错误归供应商。仅保留内存中的最近 40 条元数据，不记录请求/响应正文与凭据。故障转移来源和改动见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## 供应商代理

在“代理设置”添加 SOCKS5 主机、端口及可选用户名/密码，再为指定供应商选择连接方式。HTTP、上传、SSE、WebSocket 共用该连接层；域名在代理端解析，HTTPS 保持证书链与主机名验证。指定代理失败不会为同一供应商切回直连。代理修改使用新连接池；使用中的代理须解绑后才能删除。测试连接只检查认证、TCP 和适用的 TLS，不调用模型、不影响业务熔断。

SOCKS5 本身不加密；HTTPS 供应商内容仍经端到端 TLS 加密。腾讯云部署模板、认证限制与移除步骤见 [deploy/README.md](deploy/README.md)，仓库不含可用连接密码。

## 本地数据

数据目录为 Tauri 的 `com.lich13.gpt-switch` 应用数据目录（macOS：`~/Library/Application Support/com.lich13.gpt-switch`；Windows：`%APPDATA%/com.lich13.gpt-switch`）。`accounts.json` 保存完整账号，`previous-auth.json`、`previous-config.toml` 各保留一份最近写入前的文件。`gateway.json` 独立保存供应商、代理及本地访问令牌，`gateway-recovery.json` 在接管期间保存恢复信息。供应商 Token 仅用于其上游认证，代理密码仅用于 SOCKS5 握手；凭据不写日志、不返回到列表或事件。文件采用当前用户权限，**未加密**，应像原 `auth.json` 一样保护。

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

`pnpm dev` 是标明“预览”的模拟界面，真实凭据只能在桌面应用中操作。`--smoke-test <绝对输出路径>` 使用独立临时账号和配置，验证 WebView、托盘、账号切换、网关监听与认证、接管恢复和 auth 不变后退出，不接触真实 Codex 目录。

`pnpm icons` 从原创 SVG 生成 9 档 PNG、黑白托盘图标、ICNS 和 ICO。发布通过 CI 构建三平台安装包及校验和；真实 OAuth 和操作系统托盘交互验收与自动冒烟测试分别记录。

MIT License。独立项目，与 OpenAI 无隶属关系。
