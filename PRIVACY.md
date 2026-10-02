# 隐私与安全

lich13-switch 面向本机使用。账号、供应商地址、访问令牌和客户端配置默认只写入当前用户的本地文件，不上传到本项目的服务或更新接口。启用网关后，业务请求会按用户选择发送到对应供应商。

## 本地数据

- Codex 的 `auth.json`、Claude Code 的 `settings.json` 以及应用私有数据目录由用户所在系统账户拥有。
- 供应商令牌会保存在应用私有目录中，用于连接对应供应商。文件采用当前用户权限保护，但默认不加密，应按原客户端凭据文件的敏感级别管理。
- 日志、界面事件和诊断只使用名称、状态、耗时等脱敏信息，不保存请求正文、响应正文、认证头或代理密码。
- 应用不扫描其他客户端的会话，也不把本地账号或配置同步到云端。

## 网络访问

- 启用网关后，业务请求只发送到用户配置的供应商地址；额度和模型列表查询也只在用户主动配置或刷新时访问对应供应商。
- “检查更新”只请求 [lich13/lich13-switch 的 GitHub Releases API](https://api.github.com/repos/lich13/lich13-switch/releases/latest)，并仅打开官方仓库或发布页。
- 应用不包含广告、遥测或使用行为上报服务。

## 公开仓库与发布包

- 仓库中的凭据、回调地址和账号均为脱敏测试夹具，使用 `example.invalid`、本机回环地址或明确标注的 `fixture` 值。
- 发布包不包含用户的 `auth.json`、`settings.json`、供应商令牌、电源状态记录或应用私有数据。
- 下载发布包后，可使用对应 Release 中的 `SHA256SUMS` 校验文件完整性。

发现安全问题时，请通过 [GitHub Security Advisories](https://github.com/lich13/lich13-switch/security/advisories/new) 私下提交；请勿在公开 Issue、截图或日志中粘贴令牌、回调地址或完整配置。
