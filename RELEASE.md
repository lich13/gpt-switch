# gpt-Switch v0.3.0

网关启停与供应商切换现在严格只写现有 `[model_providers.custom]` 的 `base_url` 和 `experimental_bearer_token`，保留其余配置字节，不新增 provider 或修改选择器。

- 关闭网关时切换供应商直接写入两字段；运行时切换只更新内部路由。停用或退出写入当前供应商，自动模式采用最近成功供应商或队首。
- 版本化配置事务覆盖并发冲突、部分写入与崩溃恢复；已解除接管的旧记录不会覆盖用户现有配置。
- 新增 Key 额度：Sub2API 优先、New API 回退；支持余额、配额、订阅窗口、到期及使用统计。New API 按站点单位换算，未知比例保留原始额度。
- 可见网关页每 60 秒自动刷新，支持单项和全部刷新；失败保留过期结果。额度请求使用供应商代理，不触发模型、业务熔断或换商。
- 修复腾讯云 Dante 对双栈域名的误拒绝，保持 IPv4 公网出口与内网目标限制。
- 原账号管理、官方登录、TOML 编辑、原生托盘、HTTP/SSE/WebSocket、故障转移和 SOCKS5 保留。

提供 macOS Apple Silicon / Intel DMG、Windows x64 NSIS、图标资源及 SHA256SUMS。macOS 使用 ad-hoc 签名；Windows 安装包未商业签名。真实凭据只在本机私有存储中保存。
