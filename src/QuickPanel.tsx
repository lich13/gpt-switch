import { useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  ArrowUpRight,
  Check,
  Pin,
  Power,
  Search,
  X,
  Settings2,
} from "lucide-react";
import { command, preview, subscribe } from "./bridge";
import { errorOf, type GatewayState, type ViewState } from "./types";
import { QuotaInfo, useProviderQuota } from "./Quota";
import { providerStatus } from "./provider-status";
import QuickControls from "./QuickControls";
import AuthSyncNotice from "./AuthSyncNotice";
type Preferences = {
  pinned: boolean;
  tab: "providers" | "accounts";
  visible?: boolean;
};
export default function QuickPanel() {
  const [accounts, setAccounts] = useState<ViewState | null>(null);
  const [gateway, setGateway] = useState<GatewayState | null>(null);
  const [prefs, setPrefs] = useState<Preferences>({
    pinned: false,
    tab: "providers",
  });
  const [visible, setVisible] = useState(preview);
  const [query, setQuery] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [systemDark, setSystemDark] = useState(
    matchMedia("(prefers-color-scheme: dark)").matches,
  );
  const root = useRef<HTMLDivElement>(null),
    content = useRef<HTMLDivElement>(null);
  const quota = useProviderQuota(
    gateway?.providers ?? [],
    visible && prefs.tab === "providers",
    "panel-visibility",
  );
  useEffect(() => {
    let disposed = false;
    const clean: (() => void)[] = [];
    const watch = <T,>(event: string, fn: (v: T) => void) =>
      void subscribe<T>(event, (v) => {
        if (!disposed) fn(v);
      }).then((c) => (disposed ? c() : clean.push(c)));
    watch<ViewState>("switch-state", setAccounts);
    watch<GatewayState>("gateway-state", setGateway);
    watch<Preferences>("quick-state", setPrefs);
    watch<boolean>("panel-visibility", setVisible);
    watch<string>("switch-notice", setNotice);
    watch<unknown>("switch-error", (e) => setError(errorOf(e).message));
    void Promise.all([
      command<ViewState>("get_state"),
      command<GatewayState>("get_gateway"),
      command<Preferences>("get_quick"),
    ])
      .then(([a, g, p]) => {
        if (!disposed) {
          setAccounts(a);
          setGateway(g);
          setPrefs(p);
          setVisible(p.visible ?? preview);
        }
      })
      .catch((e) => {
        if (!disposed) setError(errorOf(e).message);
      });
    void command<{ message: string } | null>("get_startup_error")
      .then((e) => {
        if (e && !disposed) setError(e.message);
      })
      .catch(() => {});
    const media = matchMedia("(prefers-color-scheme: dark)");
    const change = () => setSystemDark(media.matches);
    media.addEventListener("change", change);
    return () => {
      disposed = true;
      clean.forEach((c) => c());
      media.removeEventListener("change", change);
    };
  }, []);
  useEffect(() => {
    document.documentElement.dataset.theme =
      !accounts || accounts.preferences.theme === "system"
        ? systemDark
          ? "dark"
          : "light"
        : accounts.preferences.theme;
  }, [accounts?.preferences.theme, systemDark]);
  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(""), 6500);
    return () => clearTimeout(timer);
  }, [notice]);
  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.isComposing)
        void command("hide_quick");
    };
    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, []);
  useLayoutEffect(() => {
    if (!root.current || !content.current) return;
    const element = root.current,
      inner = content.current;
    let last = 0;
    const resize = () => {
      const chrome = [...element.children]
        .filter((e) => !e.classList.contains("quick-scroll"))
        .reduce((h, e) => h + e.getBoundingClientRect().height, 0);
      const height = Math.min(
        520,
        Math.max(
          128,
          Math.ceil(chrome + inner.getBoundingClientRect().height + 2),
        ),
      );
      if (height === last) return;
      last = height;
      if (preview) element.style.height = `${height}px`;
      void command("resize_quick", { height }).catch(() => {});
    };
    const observer = new ResizeObserver(resize);
    observer.observe(inner);
    [...element.children]
      .filter((e) => !e.classList.contains("quick-scroll"))
      .forEach((e) => observer.observe(e));
    resize();
    return () => observer.disconnect();
  }, []);
  const action = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError("");
    try {
      await fn();
    } catch (e) {
      setError(errorOf(e).message);
    } finally {
      setBusy(false);
    }
  };
  const run = (fn: () => Promise<void>) => void action(fn);
  const edit = async (edit: Record<string, unknown>) => {
    if (!gateway) return;
    setGateway(
      await command("update_gateway", {
        edit,
        expectedRevision: gateway.revision,
        ...(edit.op === "select" && !gateway.running
          ? { expectedConfigRevision: gateway.configRevision }
          : {}),
      }),
    );
    if (edit.op === "select")
      setNotice(
        gateway.running
          ? "供应商已切换，新请求已生效"
          : "配置已切换，请重新打开 Codex",
      );
  };
  const account = async (id: string) => {
    if (!accounts) return;
    setAccounts(
      await command("switch_account", {
        id,
        expectedRevision: accounts.authRevision,
      }),
    );
    setNotice("文件已切换，请重新打开 Codex");
  };
  const open = (page?: string) =>
    run(async () => {
      await command("open_main", { page: page ?? null });
    });
  return (
    <div ref={root} className="quick-panel">
      <header className="quick-toolbar">
        <nav className="segmented" aria-label="快捷面板">
          {(["providers", "accounts"] as const).map((tab) => (
            <button
              key={tab}
              aria-pressed={prefs.tab === tab}
              onClick={() =>
                run(async () => {
                  setPrefs(await command("set_quick", { tab }));
                  setQuery("");
                })
              }
            >
              {tab === "providers" ? "供应商" : "账号"}
            </button>
          ))}
        </nav>
        <span className="quick-spacer" />
        <button
          className="icon-button"
          aria-label={prefs.pinned ? "取消固定面板" : "固定面板"}
          aria-pressed={prefs.pinned}
          onClick={() =>
            run(async () =>
              setPrefs(await command("set_quick", { pinned: !prefs.pinned })),
            )
          }
        >
          <Pin size={15} />
        </button>
        <button
          className="icon-button"
          aria-label="关闭快捷面板"
          onClick={() => void command("hide_quick")}
        >
          <X size={16} />
        </button>
      </header>
      <div className="quick-scroll">
        <div ref={content}>
          {(error || gateway?.error || accounts?.error) && (
            <div className="quick-feedback error" role="alert">
              {error || gateway?.error || accounts?.error}
            </div>
          )}
          {notice && (
            <div className="quick-feedback" role="status">
              {notice}
            </div>
          )}
          {prefs.tab === "providers" ? (
            <>
              <div className="quick-status">
                <span
                  className={`status-pill ${gateway?.running ? "online" : ""}`}
                >
                  <span className="dot" />
                  {gateway?.running ? "运行中" : "已关闭"}
                </span>
                <span className="quick-spacer" />
                <button
                  className="text-button"
                  disabled={busy || !gateway}
                  aria-pressed={gateway?.mode === "auto"}
                  onClick={() =>
                    run(() =>
                      edit({
                        op: "mode",
                        mode: gateway?.mode === "auto" ? "manual" : "auto",
                      }),
                    )
                  }
                >
                  {gateway?.mode === "auto" ? "自动" : "手动"}
                </button>
                <button
                  className="icon-button"
                  aria-label={gateway?.running ? "停用网关" : "启用网关"}
                  disabled={
                    busy ||
                    !gateway ||
                    (!gateway.running && !gateway.providers.length)
                  }
                  onClick={() =>
                    run(async () => {
                      if (!gateway) return;
                      setGateway(
                        await command(
                          gateway.running || gateway.recoveryPending
                            ? "stop_gateway"
                            : "start_gateway",
                          {
                            expectedRevision: gateway.revision,
                            expectedConfigRevision: gateway.configRevision,
                          },
                        ),
                      );
                      setNotice("配置已切换，请重新打开 Codex");
                    })
                  }
                >
                  <Power size={16} />
                </button>
              </div>
              {Boolean(gateway?.waitingRequests) && (
                <div className="quick-waiting">
                  等待 {gateway?.waitingRequests}
                </div>
              )}
              {!gateway ? (
                <div className="quick-empty">正在读取供应商…</div>
              ) : !gateway.providers.length ? (
                <div className="quick-empty">
                  尚无供应商
                  <button
                    className="text-button"
                    onClick={() => open("gateway")}
                  >
                    添加供应商
                  </button>
                </div>
              ) : (
                gateway.providers.map((p, i) => (
                  <article className="quick-provider" key={p.id}>
                    <button
                      className="quick-select"
                      disabled={busy}
                      onClick={() =>
                        run(() => edit({ op: "select", id: p.id }))
                      }
                      aria-label={`选择 ${p.name}`}
                    >
                      <span className="quick-priority">
                        {p.queued
                          ? `P${gateway.providers.slice(0, i + 1).filter((p) => p.queued).length}`
                          : "—"}
                      </span>
                      <span className="quick-name">
                        <strong title={p.name}>{p.name}</strong>
                      </span>
                      {gateway.mode === "manual" &&
                        gateway.selected === p.id && <Check size={15} />}
                    </button>
                    <button
                      className="icon-button quick-provider-settings"
                      aria-label={`${p.name} 设置`}
                      onClick={() =>
                        void command("open_main", {
                          page: "gateway",
                          providerId: p.id,
                        })
                      }
                    >
                      <Settings2 size={14} />
                    </button>
                    {providerStatus(
                      p,
                      gateway.proxies.find((x) => x.id === p.proxyId),
                    ) && (
                      <span className="quick-provider-alert">
                        {providerStatus(
                          p,
                          gateway.proxies.find((x) => x.id === p.proxyId),
                        )}
                      </span>
                    )}
                    <QuotaInfo
                      compact
                      provider={p}
                      quota={quota.quotaFor(p)}
                      refresh={() => void quota.refresh(p.id)}
                    />
                  </article>
                ))
              )}
            </>
          ) : (
            <>
              <label className="quick-search">
                <Search size={14} />
                <input
                  aria-label="搜索账号"
                  placeholder="搜索账号"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                />
              </label>
              {accounts && (
                <AuthSyncNotice
                  state={accounts}
                  busy={busy}
                  apply={(id) => run(() => account(id))}
                />
              )}
              {accounts?.accounts
                .filter((a) =>
                  `${a.name} ${a.email ?? ""}`
                    .toLowerCase()
                    .includes(query.toLowerCase()),
                )
                .map((a) => (
                  <button
                    className="quick-account"
                    key={a.id}
                    disabled={busy || a.current}
                    onClick={() => run(() => account(a.id))}
                    aria-label={`切换到 ${a.name}`}
                  >
                    <span className="quick-name">
                      <strong title={a.name}>{a.name}</strong>
                      <span>
                        {a.kind === "chatgpt" ? "ChatGPT" : "API Key"}
                        {a.current ? " · 当前文件" : ""}
                      </span>
                    </span>
                    {a.current && <Check size={15} />}
                  </button>
                ))}
              {!accounts?.accounts.length && (
                <div className="quick-empty">
                  尚无已保存账号
                  <button
                    className="text-button"
                    onClick={() => open("accounts")}
                  >
                    添加账号
                  </button>
                </div>
              )}
            </>
          )}
        </div>
      </div>
      <QuickControls visible={visible} notify={setNotice} error={setError} />
      <footer className="quick-footer">
        <button onClick={() => open()}>
          打开 gpt-Switch <ArrowUpRight size={12} />
        </button>
        <button
          onClick={() =>
            open(prefs.tab === "providers" ? "gateway" : "accounts")
          }
        >
          {prefs.tab === "providers" ? "管理供应商" : "管理账号"}
        </button>
      </footer>
    </div>
  );
}
