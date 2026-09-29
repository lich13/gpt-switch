# gpt-Switch v0.7.0

- 精简供应商主窗口与快捷面板：默认两行，额度详情和供应商设置独立打开，次要操作收纳到更多菜单。
- 删除认证来源横条、使用统计、请求日志和模型定价；升级清理对应私有数据，保留账号、供应商、代理和 Key 额度。
- 支持 `ccswitch://v1/import` 的 Codex 供应商链接，确认后只新增，不切换供应商或修改 Codex 配置。导入队列保留现有表单草稿。
- 设置中可选择已安装的 gpt-Switch、CC Switch 或 lich13studio 接收链接；使用系统实际关联，不在启动时抢占。Windows 通过系统默认应用界面选择。
- 保留账号自动同步、TOML 原文编辑、代理、模型白名单、并发调度、故障转移、HTTP/SSE/WebSocket、静默启动及快捷控制。

安装包：macOS Apple Silicon DMG、Windows x64 NSIS；另附图标和 SHA-256。macOS 使用 ad-hoc 签名，Windows 未配置商业代码签名。
