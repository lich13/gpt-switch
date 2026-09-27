# gpt-Switch v0.2.1

修复 macOS Cmd+Q 退出时未立即恢复 Codex 配置的问题。应用菜单退出现在等待恢复完成；系统退出增加同步恢复，并保留冲突现场。同时修复停止后立即启用时的短暂端口占用。安装烟测覆盖网关运行中的退出与原文恢复。建议 v0.2.0 用户升级。

新增本地 Codex API 网关、供应商热切换、自动故障转移和供应商独立 SOCKS5 代理。升级后网关关闭，供应商默认直连。

- 通用 HTTP / SSE / WebSocket 转发，保持原始请求载荷、模型与参数；供应商 API 只需 base_url、experimental_bearer_token。
- 故障转移参考固定版本 cc-switch，支持顺序优先级、熔断、单个半开探测、Retry-After、共享代理故障隔离和响应归属固定。已开始的流与 WebSocket 不跨供应商重放。
- SOCKS5 支持密码认证、远端 DNS、严格目标 TLS 验证；失败不静默直连。独立代理设置、连接测试及托盘供应商选择。
- 受管 provider 的配置接管、原文恢复、外部修改冲突和崩溃恢复；网关不改 auth.json。
- 精简界面说明，保留原有账号、官方登录、导入、API Key、TOML 编辑、托盘和 Prism Relay 图标。

macOS Apple Silicon / Intel DMG、Windows x64 NSIS、图标资源与 SHA256SUMS。macOS 使用 ad-hoc 签名；Windows 安装包未商业签名。真实账号与代理凭据只在本机私有数据中保存，不随安装包发布。
