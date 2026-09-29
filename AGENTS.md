# lich13-switch

- 简体中文协作。先读 DESIGN.md；保持产品名 lich13-switch、包名 lich13-switch。
- Codex／Claude 网关相互隔离，代理共用；Codex 仅写 custom 两字段，Claude 仅写 settings.json 的 env 两字段。内部身份 com.lich13.gpt-switch 和旧私有路径保持兼容。
- 账号切换仅写 auth.json；config.toml 独立原文编辑，保留注释和未知字段。禁止自动重启 Codex。
- Rust 独占文件及凭据操作；列表和事件只能返回脱敏元数据。真实凭据、用户配置和登录输出不得提交、截图或写入日志。
- 前端 pnpm test、pnpm build；再执行 cargo test 和 cargo clippy。真实验收区分 Browser 预览、原生程序、官方 OAuth 和 Windows CI。
- 修改后完成 commit/push、对应 CI/Release 和本地安装验收；只清理任务创建的临时和可再生产物。
