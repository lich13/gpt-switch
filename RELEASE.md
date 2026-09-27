# gpt-Switch v0.1.0

首个版本提供 Codex 账号列表、菜单栏／托盘左键快捷切换、官方 ChatGPT 登录、凭据导入、API Key 添加和独立 TOML 编辑器。

- 原创 Prism Relay 图标，跟随系统的深浅主题。
- 切换只写 `auth.json`；TOML 原文保存，外部修改冲突保护。
- 文件权限限制、原子写入、回读校验、Token 刷新回存与一份最近回滚。
- macOS Apple Silicon / Intel DMG、Windows x64 NSIS 安装包及 SHA-256。

需要本机 Codex CLI 才能添加 ChatGPT 登录。切换文件后请自行重新打开 Codex。macOS 使用 ad-hoc 签名；Windows 安装包未商业签名。
