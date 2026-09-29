# gpt-Switch

轻量的 Codex 账号切换、TOML 编辑与本地 API 网关，支持 macOS 和 Windows。Tauri 2 + Rust + React。首次启动网关默认关闭，可设置登录自启动与恢复上次网关状态。

<img src="assets/icon.svg" alt="Prism Relay icon" width="100" />

- 菜单栏／托盘左键打开供应商／账号快捷面板，右键保留精简原生菜单。
- 添加 ChatGPT 登录、导入现有凭据和添加 API Key；同一身份的 Token 刷新自动回存。
- 独立 CodeMirror TOML 原文编辑器，支持高亮、查找、语法检查、保存冲突保护。
- 跟随系统深浅主题，关闭窗口后驻留；不会自动重启 Codex。

## 安装

从 [GitHub Releases](https://github.com/lich13/gpt-switch/releases) 下载 macOS Apple Silicon DMG 或 Windows x64 安装程序。macOS 包使用 ad-hoc 签名，Windows 包未配置商业代码签名；系统可能要求确认来自非商店开发者的应用。校验下载文件时使用 Release 中的 `SHA256SUMS`。

## 使用

启动及每 2 秒稳定读取当前 `auth.json`：已有身份的更新版本同步入库，新身份自动加入列表。ChatGPT 按用户与工作区区分，API Key 按指纹区分；不按邮箱合并。比较有效 `last_refresh`、其次 Token 签发时间，保留已保存的较新凭据并提供“使用已保存版本”；无法可靠排序时采用最近检测到的稳定内容。监控不回写 auth 或配置，无效／缺失文件不会删除账号。删除账号后，同一文件版本在重启后也不会重新入库，出现新版本才重新收录。

点击“添加账号”通过官方 Codex CLI 登录，或导入完整 `auth.json`。选择账号后仅替换认证文件，配置保持不变。现有 Codex 进程可能缓存凭据，需要自行重新打开。

ChatGPT 登录需要本机已安装 [官方 Codex CLI](https://developers.openai.com/codex/cli)。应用自动查找 PATH、常见安装目录和 macOS nvm；也可在设置中指定 CLI 路径。Windows npm 安装使用同目录 Codex JavaScript 入口和 Node.js。登录使用独立临时目录，不修改当前 Codex 登录；10 分钟未完成会取消。浏览器授权需本人完成，也支持设备码登录。

默认目录为 `CODEX_HOME` 或 `~/.codex`（Windows 为用户主目录中的 `.codex`），可在设置中修改。账号切换仅影响文件；`keyring` / `auto` / `ephemeral`、provider bearer token、环境变量或命令认证可能使用其他来源。

配置页按原文保存，保留注释、未知字段和换行。外部修改发生时保留未保存草稿并阻止覆盖；复制草稿、重新读取并合并后保存。删除列表中的账号不会注销当前 Codex。

## 快捷面板

左键反复点击菜单栏／托盘图标始终显示并聚焦独立面板，失焦后收起；固定后保留，Escape 或关闭按钮始终可关闭。供应商和账号标签记住上次选择。供应商页可以启停网关、选择模式与供应商，查看优先级、当前选择和额度；满载、冷却或故障时显示异常状态，设置入口打开对应供应商配置；账号页可搜索并切换。面板不会重置主窗口草稿，也不会更改 Dock 显示状态。右键菜单提供打开主窗口、配置、网关、设置与退出，普通打开保留当前页面。

## 本地网关

在“网关”添加 `base_url` 和 `experimental_bearer_token`，或从现有 `custom` 配置导入。当前生效的 provider 必须为 `custom`，且已有 `[model_providers.custom]`。应用只替换该表中的两个字段，不新增 provider，不改选择器、模型、协议或其他设置；其余配置字节、注释和换行保持原样。

启用先绑定 `127.0.0.1:15722`，再将两字段替换为本地地址和访问令牌。运行时切换供应商只更新内部路由，新请求生效，进行中的连接保持原供应商。关闭网关时选择供应商会直接写入其地址和 Key。**停用或正常退出写入当前供应商，不恢复启用前供应商**；自动模式使用最近成功响应的供应商，没有成功记录时使用队首。写入文件后需要重新打开 Codex，不自动重启客户端。

两字段受到外部修改时停止覆盖并保留事务记录；其他字段的外部编辑会保留。异常退出后，下次启动按同一事务完成写入。旧版已经解除接管的失效记录只清理元数据；仍由旧 provider 接管时提示处理，不执行旧版整份配置回滚。更换 Codex 目录前须先停用网关。

网关只转发发往供应商 `base_url` 的 API：未知路径、Responses、compact、models、搜索、图片和上传均使用同一原始载荷通道，包括 SSE 和 WebSocket。登录、插件和 MCP 的其他地址不经过网关。`base_url` 子路径按输入保留，不猜测 `/v1`；本地 `/v1` 前缀替换为上游 base 路径。重定向原样返回。Bearer 认证替换为上游 Token，连接头按 HTTP 规范处理，模型和业务参数不重写。

自动模式每次从 P1 开始，按模型、上下文、健康和容量检查并原子预占，状态竞争时重新检查一次。每家最多一次，默认最多四家，满载或健康跳过不消耗上游尝试次数。默认连续失败 4 次、错误率 60% / 最小 10 次、恢复等待 60 秒、半开 2 次成功；半开只放行一个探测。首字节 60 秒、流静默 120 秒、非流式总超时 600 秒。`400/405/406/413/414/415/422/501` 直接返回且不计故障；客户端取消为中性。429 使用独立限流冷却，有效 Retry-After 按原时长执行，无有效值默认 5 秒，可在高级设置修改；不累计为网络／服务故障。服务故障熔断与 Retry-After 分别计算，恢复时最多一个并发探测。已向客户端提交输出后不重试，WebSocket 上游连接建立后不换商；已知 previous_response_id 固定原供应商，未知归属仅尝试一次。gzip / deflate / zstd 和磁盘暂存的 JSON 只流式读取路由归属，原始载荷不变；无法解析的 JSON 或未知压缩编码保守地只尝试一次。

请求最多 1 GiB，超过 2 MiB 使用权限受限的临时文件重放，完成或取消后清理。供应商和代理健康独立：代理连接或认证失败会跳过共享代理的其他候选；目标连接和 HTTP 错误归供应商。仅维护当前调度、冷却和并发状态，不保存请求历史、Token 或费用。故障转移来源和改动见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## 供应商模型白名单

在供应商更多菜单打开“供应商设置”，点击“模型”，选择不限模型或设置非空白名单。完整模型 ID 区分大小写，去除首尾空白及重复项，不支持通配符。可从供应商读取模型、搜索勾选，也可手动输入；读取失败不清除选择，刷新不会自动增删白名单。模型接口为填写的 `base_url` 后追加 `/models`，不猜测 `/v1`；查询使用已有 Key 和指定代理，10 秒超时、2 MB 上限、5 分钟缓存，凭据或代理变化后失效，不调用模型或消耗业务并发。

**白名单只在网关运行时生效，不写入 Codex 配置。** HTTP/SSE 从 JSON 或 multipart 读取模型，兼容 gzip、deflate、zstd，原始载荷不变。模型筛选先于健康、并发和排队；不匹配不计故障或重试，等待期间规则变更会重新检查。手动模式同样受限。已知响应游标固定原供应商，模型省略可继承已知上下文；没有可靠依据时只使用不限模型的供应商。没有匹配项返回 `400 MODEL_NOT_ALLOWED`，无法识别且没有不限供应商返回 `400 MODEL_UNDETERMINED`。明确的文件／模型等资源接口正常透传。

Responses WebSocket 每个 `response.create` 检查模型，连接建立后固定供应商；后续轮次可继承已确认模型，规则更新从下一轮生效。不匹配时以 `1008` 关闭，不向上游发送该轮内容。未知 WebSocket 协议只允许不限模型供应商，仍按整条连接占位。

## 并发与启动

供应商设置可调整独立并发上限，`0` 表示不限。HTTP/SSE 从上游转发开始占用到响应结束，取消或失败自动释放；额度和测连不占业务名额。自动模式跳过满载供应商，优先分配给下一家；高优先级供应商恢复空闲后重新优先使用。满载不计入失败或重试。所有匹配候选满载、冷却或恢复探测中时，共用默认累计等待 30 秒、最多 100 个请求的 FIFO 队列；唤醒后重新检查规则与优先级。参数可在高级设置修改。队列满或容量超时返回 429 与 Retry-After: 5；冷却超时返回 503 PROVIDERS_COOLING_DOWN 和重试时间。模型不匹配、队列为空等确定性问题立即返回；已有上游尝试全部失败时保留最后一次上游错误。手动模式及已有响应游标绑定的请求仅等待对应供应商。

Responses WebSocket 按每轮 `response.create` 生成计数，空闲长连接不占业务名额。首轮读取后选路；后续轮次固定已有上游，等待超时以 1013 关闭，请客户端重连。通过协议处理识别轮次和压缩／分片，消息内容不重新序列化；Ping/Pong、关闭信息与背压保留。单条 Responses WebSocket 消息上限 64 MiB，最多额外缓存一轮生成；未知 WebSocket 路径继续使用通用隧道，并按连接计数。

设置中提供开机静默启动、启动时恢复网关。macOS 使用用户级 LaunchAgent，Windows 使用当前用户启动注册；仅在用户开启后注册。应用正常退出仍将配置写为当前供应商，并保留上次开启意图；明确停用网关会清除该意图。启用恢复后，下次启动检查 Codex 目录和两字段指纹，先完成崩溃事务，再绑定端口并接管两字段；外部修改受控字段、端口占用或配置不满足 custom 契约时显示错误，不覆盖现场。

macOS 通过用户级 LaunchAgent 和系统的登录启动 Apple event 识别启动来源，兼容系统忽略旧登录项隐藏属性的情况；手动打开仍显示窗口。Windows 使用专用登录启动参数，重复的登录启动信号不弹出窗口。

切换应用、Dock 重新打开或托盘普通打开保留当前页面、弹窗与内存草稿。明确导航离开未保存的供应商／代理表单时需要确认；取消或退出清除草稿中的凭据。

## Key 额度

网关页使用供应商现有 Key，优先查询 Sub2API `/v1/usage`；未识别时回退到同一部署路径的 New API `/api/usage/token/`。支持余额、Key 配额、无限额度、订阅日／周／月窗口、5 小时／日／周限制、到期、重置及已返回的使用统计。New API 使用公开 `/api/status` 的比例和货币设置换算；信息不足时显示原始“额度单位”。不支持的站点显示“不可查询”，认证、限流、网络和代理错误分别显示原因。

进入网关页时查询，可见期间每 60 秒刷新；隐藏窗口或离开页面暂停，并提供单项及全部手动刷新。失败保留上次结果并标记过期。额度查询沿用供应商的代理与严格 TLS，失败不会静默直连；不调用模型、不触发业务熔断，也不更改故障转移选择。关闭本地网关时仍可查询额度；Codex 本身的供应商代理仅在网关运行时生效。

## 供应商代理

在“代理设置”添加 SOCKS5 主机、端口及可选用户名/密码，再为指定供应商选择连接方式。HTTP、上传、SSE、WebSocket 共用该连接层；域名在代理端解析，HTTPS 保持证书链与主机名验证。指定代理失败不会为同一供应商切回直连。代理修改使用新连接池；使用中的代理须解绑后才能删除。测试连接只检查认证、TCP 和适用的 TLS，不调用模型、不影响业务熔断。

SOCKS5 本身不加密；HTTPS 供应商内容仍经端到端 TLS 加密。腾讯云部署模板、认证限制与移除步骤见 [deploy/README.md](deploy/README.md)，仓库不含可用连接密码。

## CC Switch 链接

设置中的“CC Switch 链接”读取系统默认接收应用，可选择已安装且支持协议的 gpt-Switch、CC Switch 或 lich13studio。macOS 直接设置并回读；Windows 打开系统默认应用界面完成选择。安装、启动和刷新均不抢占默认关联。

支持 `ccswitch://v1/import?resource=provider&app=codex`。接收后确认名称和地址，再新增供应商；不切换当前供应商、不启停网关、不改 Codex 配置。部署子路径原样保留，凭据只在 Rust 内存中等待确认。已有表单草稿会保留，导入排队处理。其他资源、客户端、脚本和远程配置不导入。

## 本地数据

数据目录为 Tauri 的 `com.lich13.gpt-switch` 应用数据目录（macOS：`~/Library/Application Support/com.lich13.gpt-switch`；Windows：`%APPDATA%/com.lich13.gpt-switch`）。`accounts.json` 保存完整账号，`previous-auth.json`、`previous-config.toml` 各保留一份最近写入前的文件。`startup.json` 保存启动偏好，`quick.json` 保存快捷面板标签与固定状态。`gateway.json` 独立保存供应商、代理、并发上限、模型白名单、本地访问令牌及网关恢复意图，`gateway-recovery.json` 保存版本化的两字段事务与停止目标，写入完成后清理。供应商 Token 仅用于其上游认证，代理密码仅用于 SOCKS5 握手；凭据不写日志、不返回到列表或事件。文件采用当前用户权限，**未加密**，应像原 `auth.json` 一样保护。

v0.7.0 已移除使用统计、请求日志和模型定价。升级会精确删除应用私有目录内的 `usage.sqlite` 及 WAL／SHM／journal、`usage-settings.json`、`model-pricing.json`；失败可在设置中重试。账号、供应商、代理、Key 额度、电源助手和 Codex 文件保留。

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

`pnpm dev` 是标明“预览”的模拟界面，真实凭据只能在桌面应用中操作。`--smoke-test <绝对输出路径>` 使用独立临时账号和配置，验证 WebView、托盘、账号切换、网关监听与认证、两字段切换、退出恢复和 auth 不变后退出，不接触真实 Codex 目录。

`pnpm icons` 从原创 SVG 生成 9 档 PNG、黑白托盘图标、ICNS 和 ICO。发布通过 CI 构建 macOS Apple Silicon DMG 与 Windows x64 NSIS 安装包及校验和；真实 OAuth 和操作系统托盘交互验收与自动冒烟测试分别记录。

MIT License。独立项目，与 OpenAI 无隶属关系。


## 快捷控制

快捷面板底部提供“强制退出 ChatGPT / Codex”和 macOS 电池“合盖不休眠”。前者在本版本只做模拟验收，不会在开发预览中终止真实进程；后者通过独立电源助手执行限定的 `pmset` 操作，恢复原电池休眠值（包括 0），兼容旧脚本的恢复记录。

macOS 首次操作需系统管理员授权安装助手；日常开关走 XPC，不重复请求密码。设置页可安装、修复或移除助手。助手由系统 launchd 管理，仅接受状态读取、合盖切换与恢复；验证获授权用户、正式应用路径和实际进程的代码签名，升级导致代码身份变化时需重新授权。安装程序不保存密码、不配置免密 sudo。应用须位于 `/Applications/gpt-Switch.app`；开发和隔离运行不会调用或安装正式助手。

助手位于 `/Library/PrivilegedHelperTools/com.lich13.gpt-switch.power-helper`，系统服务为同名 LaunchDaemon，恢复记录位于 root 私有目录 `/Library/Application Support/gpt-Switch Power`。移除前恢复本工具接管的电源设置，状态冲突时保留现场及恢复记录；退出应用不改变电源状态。
