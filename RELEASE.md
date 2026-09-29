# lich13-switch v0.8.0

- 产品与仓库改名为 lich13-switch，沿用原应用身份和私有数据，保留全部 Codex 账号、供应商、代理和系统设置。
- 新增 Claude Code 独立供应商、网关、队列、并发、白名单、故障转移、模型列表与额度查询；Codex 和 Claude 网关可同时运行，代理配置共用。
- Claude 仅修改 settings.json 中 env.ANTHROPIC_BASE_URL、env.ANTHROPIC_AUTH_TOKEN；Codex 仍仅修改 custom 的原有两字段。热切换只改变内部路由，停止或退出写回供应商，冲突时保留现场。
- 主窗口和快捷面板增加客户端切换，分别记住选择；保留两行列表、拖动、并发浮层和队列开关。CC Switch 导入支持 app=claude，确认只新增对应供应商。
- 首次导入现有 Claude 供应商，Claude 网关默认关闭。静默启动迁移正式应用路径，电源助手保留原恢复记录，升级后可能需系统授权修复。
- 提供 macOS Apple Silicon DMG、Windows x64 NSIS、图标和 SHA-256。Windows 延续旧升级与卸载标识；不发布 Intel DMG。
