# 独立 Dante SOCKS5

模板适用于 Ubuntu 24.04 的 `dante-server`。只开放 TCP CONNECT，认证必须为专用 `gpt-switch-proxy` 无登录用户；客户端主机可以使用其他名称，但要同步修改配置中的 `user`。不要复用 SSH 用户。模板中的 `eth0` 应替换为实例公网出口网卡。

1. 安装 `dante-server`，创建专用 system 用户（无 home、shell `/usr/sbin/nologin`），通过受保护 stdin 向 `chpasswd` 设置随机长密码。不要把密码放入 shell 参数或历史。
2. 将 `danted.conf.example` 保存为 `/etc/gpt-switch-socks/danted.conf`（目录 0700、文件 0600）。公网 HTTPS 供应商可以与代理位于同一实例，不要将实例公网地址误当作私网目标阻断。模板已拒绝回环、私网、链路本地/元数据、共享地址、多播和 IPv6 目标；默认规则拒绝匿名、其他用户、SOCKS4、BIND 和 UDP。域名解析后同样执行地址限制。
3. 若本机原来没有 Dante，只停用包安装新建的默认 `danted.service`；现有代理服务不得替换。复制 `gpt-switch-socks.service` 到 `/etc/systemd/system/`，使用 `danted -V -f /etc/gpt-switch-socks/danted.conf` 校验，执行 daemon-reload 和 enable --now。
4. 在 UFW 与云实例防火墙/安全组增加 **10808/TCP** 入站。保留既有 SSH 和其他业务规则，不开放 UDP。
5. 客户端使用 IP、10808、专用用户名及密码；通过公网检查匿名/错误密码拒绝、私网/元数据/回环拒绝、HTTPS 证书验证及出口 IP。重启服务后重复成功链路。普通 SOCKS5 不加密用户名/密码传输，HTTPS 内容仍端到端加密。

检查：`systemctl status gpt-switch-socks`、`journalctl -u gpt-switch-socks`、`ss -lntp`。日志只启用错误级连接信息，不输出认证密码或业务正文。修改配置前校验；更新密码后同步各客户端私有配置。

移除时先解绑客户端供应商，停用并 disable `gpt-switch-socks.service`；只删除该 unit、`/etc/gpt-switch-socks/` 和专用用户，daemon-reload，再移除对应的 UFW 与云防火墙 10808/TCP 规则。确认没有其他 Dante 用途后才卸载包，不触碰其他代理、SSH、ipset 或业务服务。
