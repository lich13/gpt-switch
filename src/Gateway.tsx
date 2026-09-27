import { QuotaInfo, useProviderQuota } from "./Quota";
import { useEffect, useRef, useState } from "react";
import {
  Plus,
  Power,
  Download,
  ArrowUp,
  ArrowDown,
  Pencil,
  Trash2,
  X,
  Network,
  Shield,
  Activity,
  Check,
  RotateCcw,
  LoaderCircle,
} from "lucide-react";
import { command, subscribe } from "./bridge";
import {
  errorOf,
  type GatewayState,
  type GatewaySettings,
  type Provider,
  type ProxyProfile,
  type Health,
} from "./types";

type Edit = Record<string, unknown>;
type Dialog =
  | { kind: "provider"; item?: Provider }
  | { kind: "proxy"; item?: ProxyProfile }
  | { kind: "rename"; item: Provider }
  | null;
const healthText = (h: Health) =>
  h.state === "open"
    ? `熔断 · ${h.retryIn}s`
    : h.state === "half_open"
      ? "恢复探测"
      : h.requests
        ? "可用"
        : "待请求";
export default function Gateway({
  section,
  notify,
  onDirtyChange,
}: {
  section: "gateway" | "proxies";
  notify: (s: string) => void;
  onDirtyChange?: (dirty: boolean) => void;
}) {
  const [state, setState] = useState<GatewayState | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [dialog, setDialog] = useState<Dialog>(null);
  useEffect(() => {
    setDialog(null);
  }, [section]);
  const quota = useProviderQuota(state?.providers ?? [], section === "gateway");
  useEffect(() => {
    let disposed = false,
      clean: (() => void) | undefined;
    void command<GatewayState>("get_gateway")
      .then((s) => {
        if (!disposed) setState(s);
      })
      .catch((e) => setError(errorOf(e).message));
    void subscribe<GatewayState>("gateway-state", (s) => {
      if (!disposed) setState(s);
    }).then((c) => {
      if (disposed) c();
      else clean = c;
    });
    return () => {
      disposed = true;
      clean?.();
    };
  }, []);
  const action = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError("");
    try {
      await fn();
    } catch (e) {
      setError(errorOf(e).message);
      throw e;
    } finally {
      setBusy(false);
    }
  };
  const edit = async (payload: Edit, expected?: string) => {
    if (!state) return;
    const next = await command<GatewayState>("update_gateway", {
      edit: payload,
      expectedRevision: expected ?? state.revision,
      ...(payload.op === "select" && !state.running
        ? { expectedConfigRevision: state.configRevision }
        : {}),
    });
    setState(next);
  };
  const run = (fn: () => Promise<void>) => {
    void action(fn).catch(() => {});
  };
  const move = (id: string, direction: number) => {
    if (!state) return;
    const ids = state.providers.map((p) => p.id),
      i = ids.indexOf(id),
      j = i + direction;
    if (j < 0 || j >= ids.length) return;
    [ids[i], ids[j]] = [ids[j], ids[i]];
    run(() => edit({ op: "reorder", ids }));
  };
  if (!state)
    return (
      <div className="empty" role="status">
        {error || "正在读取网关…"}
      </div>
    );
  return (
    <section className="gateway-page">
      <div className="page-heading">
        <h1>{section === "gateway" ? "网关" : "代理设置"}</h1>
        {section === "gateway" ? (
          <button
            className={state.running ? "secondary" : "primary"}
            disabled={
              busy ||
              (!state.running &&
                !state.recoveryPending &&
                !state.providers.length)
            }
            onClick={() =>
              run(async () => {
                setState(
                  await command<GatewayState>(
                    state.running || state.recoveryPending
                      ? "stop_gateway"
                      : "start_gateway",
                    {
                      expectedRevision: state.revision,
                      expectedConfigRevision: state.configRevision,
                    },
                  ),
                );
              })
            }
          >
            <Power size={16} />
            {state.running
              ? "停用网关"
              : state.recoveryPending
                ? "处理配置事务"
                : "启用网关"}
          </button>
        ) : (
          <button
            className="primary"
            disabled={busy}
            onClick={() => setDialog({ kind: "proxy" })}
          >
            <Plus size={16} />
            添加代理
          </button>
        )}
      </div>
      {(error || state.error) && (
        <div className="banner error" role="alert">
          {error || state.error}
        </div>
      )}
      {section === "gateway" ? (
        <>
          <div className="gateway-status">
            <span className={`status-pill ${state.running ? "online" : ""}`}>
              <span className="dot" />
              {state.running ? "运行中" : "已关闭"}
            </span>
            <code>{state.address}</code>
            <span className="connection-count">
              <Activity size={14} />
              {state.activeConnections} 个连接
            </span>
            {state.waitingRequests > 0 && (
              <span className="capacity-status">
                等待 {state.waitingRequests}
              </span>
            )}
          </div>
          {state.configError && (
            <div className="banner error" role="alert">
              {state.configError}
            </div>
          )}
          <div className="config-binding" aria-label="配置使用状态">
            <span>
              配置使用：
              {state.configState === "gateway"
                ? "本地网关"
                : (state.providers.find((p) => p.id === state.configProvider)
                    ?.name ?? "未匹配供应商")}
            </span>
            {state.mode === "auto" && state.lastSuccessful && (
              <span>
                最近成功：
                {
                  state.providers.find((p) => p.id === state.lastSuccessful)
                    ?.name
                }
              </span>
            )}
          </div>
          <div className="gateway-toolbar">
            <div className="segmented" aria-label="路由模式">
              <button
                aria-pressed={state.mode === "manual"}
                disabled={busy}
                onClick={() => run(() => edit({ op: "mode", mode: "manual" }))}
              >
                手动
              </button>
              <button
                aria-pressed={state.mode === "auto"}
                disabled={busy}
                onClick={() => run(() => edit({ op: "mode", mode: "auto" }))}
              >
                自动故障转移
              </button>
            </div>
            <div className="inline-actions">
              <button
                className="secondary"
                onClick={() => quota.refreshAll()}
                disabled={!state.providers.length}
              >
                刷新额度
              </button>
              <button
                className="secondary"
                disabled={busy}
                onClick={() =>
                  run(async () => {
                    await edit({ op: "import" });
                    notify("已导入当前供应商");
                  })
                }
              >
                <Download size={14} />
                从配置导入
              </button>
              <button
                className="secondary"
                disabled={busy}
                onClick={() => setDialog({ kind: "provider" })}
              >
                <Plus size={14} />
                添加供应商
              </button>
            </div>
          </div>
          <div className="provider-list">
            {!state.providers.length && (
              <div className="empty">
                <Network size={24} />
                <strong>尚未添加供应商</strong>
                <button
                  className="text-button"
                  onClick={() => setDialog({ kind: "provider" })}
                >
                  填写 base_url 和 Token
                </button>
              </div>
            )}
            {state.providers.map((p, i) => (
              <article
                key={p.id}
                className={`provider-row ${state.mode === "manual" && state.selected === p.id ? "selected" : ""}`}
              >
                <div className="provider-main">
                  <span className="priority">P{i + 1}</span>
                  <div className="provider-name">
                    <strong title={p.name}>{p.name}</strong>
                    <code title={p.baseUrl}>{p.baseUrl}</code>
                  </div>
                  <span className={`health ${p.health.state}`}>
                    {healthText(p.health)}
                  </span>
                  <button
                    className="secondary compact"
                    disabled={busy}
                    onClick={() =>
                      run(async () => {
                        await edit({ op: "select", id: p.id });
                        notify(
                          state.running
                            ? "已切换供应商，新请求立即生效"
                            : "文件已切换，请重新打开 Codex",
                        );
                      })
                    }
                  >
                    {state.mode === "manual" && state.selected === p.id ? (
                      <>
                        <Check size={14} />
                        已选择
                      </>
                    ) : (
                      "选择"
                    )}
                  </button>
                </div>
                <QuotaInfo
                  provider={p}
                  quota={quota.quotaFor(p)}
                  refresh={() => void quota.refresh(p.id)}
                />
                <div className="provider-controls">
                  <ConcurrencyControl
                    provider={p}
                    busy={busy}
                    save={(maxConcurrency) =>
                      action(() =>
                        edit({
                          op: "concurrencyProvider",
                          id: p.id,
                          maxConcurrency,
                        }),
                      )
                    }
                  />
                  <label className="check-label">
                    <input
                      type="checkbox"
                      checked={p.queued}
                      disabled={busy}
                      onChange={(e) =>
                        run(() =>
                          edit({
                            op: "queueProvider",
                            id: p.id,
                            queued: e.target.checked,
                          }),
                        )
                      }
                    />
                    故障转移队列
                  </label>
                  <label className="route-label">
                    连接方式
                    <select
                      aria-label={`${p.name} 连接方式`}
                      value={p.proxyId ?? ""}
                      disabled={busy}
                      onChange={(e) =>
                        run(() =>
                          edit({
                            op: "routeProvider",
                            id: p.id,
                            proxyId: e.target.value || null,
                          }),
                        )
                      }
                    >
                      <option value="">直连</option>
                      {state.proxies.map((x) => (
                        <option key={x.id} value={x.id}>
                          {x.name}
                        </option>
                      ))}
                    </select>
                  </label>
                  <button
                    className="text-button"
                    disabled={busy}
                    onClick={() =>
                      run(async () => {
                        const ms = await command<number>("test_provider", {
                          id: p.id,
                        });
                        notify(`TCP / TLS 连接正常 · ${ms} ms`);
                      })
                    }
                  >
                    测试连接
                  </button>
                  <details className="row-menu">
                    <summary aria-label={`${p.name} 操作`}>···</summary>
                    <div className="menu-popover">
                      <button
                        onClick={() => setDialog({ kind: "provider", item: p })}
                      >
                        <Pencil size={14} />
                        编辑 API
                      </button>
                      <button
                        onClick={() => setDialog({ kind: "rename", item: p })}
                      >
                        重命名
                      </button>
                      <button
                        disabled={busy}
                        onClick={() =>
                          run(() =>
                            edit({ op: "reset", id: p.id, proxy: false }),
                          )
                        }
                      >
                        <RotateCcw size={14} />
                        重置熔断
                      </button>
                      <button
                        className="danger"
                        disabled={busy}
                        onClick={() => {
                          if (confirm(`删除供应商“${p.name}”？`))
                            run(() => edit({ op: "deleteProvider", id: p.id }));
                        }}
                      >
                        <Trash2 size={14} />
                        删除
                      </button>
                    </div>
                  </details>
                  <button
                    className="icon-button"
                    aria-label={`上移 ${p.name}`}
                    disabled={busy || i === 0}
                    onClick={() => move(p.id, -1)}
                  >
                    <ArrowUp size={14} />
                  </button>
                  <button
                    className="icon-button"
                    aria-label={`下移 ${p.name}`}
                    disabled={busy || i === state.providers.length - 1}
                    onClick={() => move(p.id, 1)}
                  >
                    <ArrowDown size={14} />
                  </button>
                </div>
              </article>
            ))}
          </div>
          <details className="advanced">
            <summary>高级设置</summary>
            <Advanced
              settings={state.settings}
              revision={state.revision}
              busy={busy}
              save={async (settings, revision) => {
                await action(() =>
                  edit({ op: "settings", settings }, revision),
                );
                notify("网关参数已保存");
              }}
            />
          </details>
          <details className="gateway-history">
            <summary>
              最近请求与故障转移{" "}
              {state.recent.length > 0 && (
                <span>
                  {state.recent[0].category === "OK"
                    ? "正常"
                    : state.recent[0].category}
                </span>
              )}
            </summary>
            {!state.recent.length ? (
              <div className="empty">暂无请求记录</div>
            ) : (
              <div className="history-scroll">
                <table>
                  <thead>
                    <tr>
                      <th>供应商 / 连接</th>
                      <th>结果</th>
                      <th>耗时</th>
                      <th>重试</th>
                    </tr>
                  </thead>
                  <tbody>
                    {state.recent.slice(0, 20).map((r, i) => (
                      <tr key={`${r.at}-${i}`}>
                        <td>
                          {r.provider}
                          <span>{r.proxy ?? "直连"}</span>
                        </td>
                        <td>
                          {r.status ?? "—"} · {r.category}
                        </td>
                        <td>{r.elapsedMs} ms</td>
                        <td>{r.retries}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </details>
        </>
      ) : (
        <div className="proxy-list">
          {!state.proxies.length && (
            <div className="empty">
              <Shield size={24} />
              <strong>尚未添加代理</strong>
            </div>
          )}
          {state.proxies.map((p) => (
            <article className="proxy-row" key={p.id}>
              <Shield size={20} />
              <div className="provider-name">
                <strong title={p.name}>{p.name}</strong>
                <code>
                  {p.host}:{p.port}
                </code>
              </div>
              <span className="proxy-auth">
                {p.hasPassword ? "密码认证" : "无认证"}
              </span>
              <span className={`health ${p.health.state}`}>
                {healthText(p.health)}
              </span>
              <button
                className="icon-button"
                aria-label={`编辑 ${p.name}`}
                onClick={() => setDialog({ kind: "proxy", item: p })}
              >
                <Pencil size={15} />
              </button>
              <button
                className="icon-button"
                aria-label={`重置 ${p.name} 熔断`}
                disabled={busy}
                onClick={() =>
                  run(() => edit({ op: "reset", id: p.id, proxy: true }))
                }
              >
                <RotateCcw size={15} />
              </button>
              <button
                className="icon-button danger"
                aria-label={`删除 ${p.name}`}
                disabled={busy}
                onClick={() => {
                  if (confirm(`删除代理“${p.name}”？`))
                    run(() => edit({ op: "deleteProxy", id: p.id }));
                }}
              >
                <Trash2 size={15} />
              </button>
            </article>
          ))}
          <details className="gateway-help">
            <summary>代理连接与凭据</summary>
            <p>
              供应商可单独绑定 SOCKS5。目标域名由代理解析，HTTPS
              证书仍严格校验。SOCKS5
              本身不加密；用户名和密码只用于代理认证。代理失败时不会改为直连。
            </p>
          </details>
        </div>
      )}
      {dialog && (
        <GatewayDialog
          dialog={dialog}
          onDirtyChange={onDirtyChange}
          close={() => setDialog(null)}
          save={async (payload) => {
            await edit(payload);
            setDialog(null);
          }}
        />
      )}
    </section>
  );
}
function ConcurrencyControl({
  provider,
  busy,
  save,
}: {
  provider: Provider;
  busy: boolean;
  save: (limit: number) => Promise<void>;
}) {
  const [draft, setDraft] = useState(String(provider.maxConcurrency));
  useEffect(
    () => setDraft(String(provider.maxConcurrency)),
    [provider.maxConcurrency],
  );
  const changed = draft !== String(provider.maxConcurrency);
  const full =
    provider.maxConcurrency > 0 &&
    provider.activeRequests >= provider.maxConcurrency;
  return (
    <form
      className="concurrency-control"
      onSubmit={(e) => {
        e.preventDefault();
        if (changed) void save(Number(draft)).catch(() => {});
      }}
    >
      <label>
        并发上限
        <input
          aria-label={`${provider.name} 并发上限`}
          type="number"
          min={0}
          max={100000}
          step={1}
          required
          value={draft}
          disabled={busy}
          title="0 表示不限"
          onChange={(e) => setDraft(e.target.value)}
        />
      </label>
      {changed && (
        <button
          className="text-button"
          disabled={busy}
          aria-label={`保存 ${provider.name} 并发上限`}
        >
          保存
        </button>
      )}
      <span
        className={`capacity-status ${full ? "at-capacity" : ""}`}
        aria-label={`${provider.name} 并发状态`}
      >
        {provider.activeRequests}/{provider.maxConcurrency || "不限"}
        {full ? " · 满载" : ""}
      </span>
    </form>
  );
}
function Advanced({
  settings,
  revision,
  busy,
  save,
}: {
  settings: GatewaySettings;
  revision: string;
  busy: boolean;
  save: (s: GatewaySettings, revision: string) => Promise<void>;
}) {
  const [draft, setDraft] = useState(settings),
    [error, setError] = useState("");
  const editing = useRef(false),
    baseRevision = useRef(revision);
  useEffect(() => {
    if (!editing.current) {
      setDraft(settings);
      baseRevision.current = revision;
    }
  }, [settings, revision]);
  const fields: [keyof GatewaySettings, string][] = [
    ["port", "本地端口"],
    ["maxRetries", "最大重试次数"],
    ["failureThreshold", "连续失败阈值"],
    ["successThreshold", "恢复成功次数"],
    ["cooldownSeconds", "熔断等待 / 秒"],
    ["errorRate", "错误率阈值 (0–1)"],
    ["minRequests", "最小请求数"],
    ["firstByteSeconds", "首字节 / 秒"],
    ["idleSeconds", "流静默 / 秒"],
    ["totalSeconds", "非流式总时限 / 秒"],
    ["connectSeconds", "连接超时 / 秒"],
    ["queueSeconds", "排队等待 / 秒"],
    ["maxWaiting", "最多等待请求"],
  ];
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        setError("");
        void save(draft, baseRevision.current)
          .then(() => {
            editing.current = false;
          })
          .catch((e) => setError(errorOf(e).message));
      }}
    >
      <div className="settings-grid">
        {fields.map(([key, label]) => (
          <label key={key}>
            {label}
            <input
              type="number"
              required
              min={key === "maxRetries" ? 0 : key === "errorRate" ? 0.01 : 1}
              step={key === "errorRate" ? 0.01 : 1}
              value={draft[key]}
              onChange={(e) => {
                if (!editing.current) baseRevision.current = revision;
                editing.current = true;
                setDraft({ ...draft, [key]: Number(e.target.value) });
              }}
            />
          </label>
        ))}
      </div>
      {error && (
        <div role="alert" className="form-error">
          {error}
        </div>
      )}
      <button disabled={busy} className="secondary">
        保存参数
      </button>
    </form>
  );
}
function GatewayDialog({
  dialog,
  close,
  save,
  onDirtyChange,
}: {
  dialog: NonNullable<Dialog>;
  close: () => void;
  save: (p: Edit) => Promise<void>;
  onDirtyChange?: (dirty: boolean) => void;
}) {
  const initialSave = useRef(save);
  const ref = useRef<HTMLDialogElement>(null),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const [baseUrl, setBaseUrl] = useState(
      dialog.kind === "provider" ? (dialog.item?.baseUrl ?? "") : "",
    ),
    [token, setToken] = useState("");
  const [name, setName] = useState(dialog.item?.name ?? "");
  const proxy = dialog.kind === "proxy" ? dialog.item : undefined;
  const [host, setHost] = useState(proxy?.host ?? ""),
    [port, setPort] = useState(proxy?.port ?? 1080),
    [username, setUsername] = useState(proxy?.username ?? ""),
    [password, setPassword] = useState("");
  const initialDraft = useRef(
    JSON.stringify([baseUrl, token, name, host, port, username, password]),
  );
  useEffect(() => {
    onDirtyChange?.(
      JSON.stringify([baseUrl, token, name, host, port, username, password]) !==
        initialDraft.current,
    );
  }, [baseUrl, token, name, host, port, username, password, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  const title =
    dialog.kind === "rename"
      ? "重命名供应商"
      : `${dialog.item ? "编辑" : "添加"}${dialog.kind === "provider" ? "供应商" : "代理"}`;
  return (
    <dialog
      ref={ref}
      className="modal gateway-modal"
      onCancel={(e) => {
        if (busy) e.preventDefault();
        else close();
      }}
    >
      <div className="modal-heading">
        <h2>{title}</h2>
        <button
          className="icon-button"
          aria-label="关闭"
          disabled={busy}
          onClick={close}
        >
          <X size={18} />
        </button>
      </div>
      <form
        className="gateway-form"
        onSubmit={(e) => {
          e.preventDefault();
          setError("");
          setBusy(true);
          const payload =
            dialog.kind === "provider"
              ? {
                  op: "saveProvider",
                  id: dialog.item?.id ?? null,
                  baseUrl,
                  token,
                }
              : dialog.kind === "rename"
                ? { op: "renameProvider", id: dialog.item.id, name }
                : {
                    op: "saveProxy",
                    id: proxy?.id ?? null,
                    name,
                    host,
                    port,
                    username,
                    password,
                  };
          void initialSave
            .current(payload)
            .catch((e) => setError(errorOf(e).message))
            .finally(() => setBusy(false));
        }}
      >
        {dialog.kind === "provider" ? (
          <>
            <label>
              base_url
              <input
                required
                type="url"
                autoFocus
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                placeholder="https://api.example.com/v1"
              />
            </label>
            <label>
              experimental_bearer_token
              <input
                required={!dialog.item}
                type="password"
                autoComplete="new-password"
                value={token}
                onChange={(e) => setToken(e.target.value)}
                placeholder={dialog.item ? "留空保留当前 Token" : "输入 Token"}
              />
            </label>
          </>
        ) : (
          <label>
            名称
            <input
              required
              autoFocus
              maxLength={120}
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
        )}
        {dialog.kind === "proxy" && (
          <>
            <div className="proxy-address">
              <label>
                主机 / IP
                <input
                  required
                  value={host}
                  onChange={(e) => setHost(e.target.value)}
                  placeholder="proxy.example.com"
                />
              </label>
              <label>
                端口
                <input
                  type="number"
                  min={1}
                  max={65535}
                  required
                  value={port}
                  onChange={(e) => setPort(Number(e.target.value))}
                />
              </label>
            </div>
            <label>
              用户名
              <input
                autoComplete="off"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                placeholder="无需认证时留空"
              />
            </label>
            <label>
              密码
              <input
                type="password"
                autoComplete="new-password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={
                  proxy?.hasPassword ? "留空保留当前密码" : "输入代理密码"
                }
              />
            </label>
          </>
        )}
        {error && (
          <div className="banner error" role="alert">
            {error}
          </div>
        )}
        <div className="form-actions">
          <button
            type="button"
            className="secondary"
            disabled={busy}
            onClick={close}
          >
            取消
          </button>
          <button className="primary" disabled={busy}>
            {busy && <LoaderCircle size={15} className="spin" />}保存
          </button>
        </div>
      </form>
    </dialog>
  );
}
