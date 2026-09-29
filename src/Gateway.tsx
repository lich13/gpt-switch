import ProviderSettings from "./ProviderSettings";
import Modal from "./Modal";
import { providerStatus } from "./provider-status";
import ModelPolicyDialog from "./ModelPolicyDialog";
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
  Settings2,
  MoreHorizontal,
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
  | { kind: "models"; item: Provider }
  | { kind: "settings"; item: Provider }
  | { kind: "quota"; item: Provider }
  | null;
const healthText = (h: Health) =>
  h.probeInFlight
    ? "恢复探测中"
    : h.cooldownReason === "rate_limit"
      ? `限流冷却 · ${h.retryIn}s`
      : h.cooldownReason === "retry_after"
        ? `上游冷却 · ${h.retryIn}s`
        : h.state === "open"
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
  focusProvider,
}: {
  section: "gateway" | "proxies";
  notify: (s: string) => void;
  onDirtyChange?: (dirty: boolean) => void;
  focusProvider?: { id: string; sequence: number } | null;
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
  const focusHandled = useRef<number | null>(null);
  useEffect(() => {
    if (
      focusProvider &&
      state &&
      focusHandled.current !== focusProvider.sequence
    ) {
      const p = state.providers.find((p) => p.id === focusProvider.id);
      if (p) {
        focusHandled.current = focusProvider.sequence;
        setDialog({ kind: "settings", item: p });
      }
    }
  }, [focusProvider, state]);
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
  const toggle = () =>
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
      notify("配置已切换，请重新打开 Codex");
    });
  const choose = (p: Provider) =>
    run(async () => {
      await edit({ op: "select", id: p.id });
      notify(
        state.running
          ? "已切换供应商，新请求立即生效"
          : "文件已切换，请重新打开 Codex",
      );
    });
  const menuClose = (e: React.MouseEvent) =>
    e.currentTarget.closest("details")?.removeAttribute("open");
  return (
    <section className="gateway-page">
      <div className="page-heading gateway-heading">
        <h1>{section === "gateway" ? "网关" : "代理设置"}</h1>
        {section === "gateway" ? (
          <div className="gateway-toolbar">
            <div className="segmented" aria-label="路由模式">
              <button
                disabled={busy}
                aria-pressed={state.mode === "manual"}
                onClick={() => run(() => edit({ op: "mode", mode: "manual" }))}
              >
                手动
              </button>
              <button
                disabled={busy}
                aria-pressed={state.mode === "auto"}
                onClick={() => run(() => edit({ op: "mode", mode: "auto" }))}
              >
                自动
              </button>
            </div>
            <button
              className={state.running ? "secondary" : "primary"}
              disabled={
                busy ||
                (!state.running &&
                  !state.recoveryPending &&
                  !state.providers.length)
              }
              onClick={toggle}
            >
              <Power size={15} />
              {state.running
                ? "停用"
                : state.recoveryPending
                  ? "处理事务"
                  : "启用"}
            </button>
            <button
              className="secondary"
              disabled={busy}
              onClick={() => setDialog({ kind: "provider" })}
            >
              <Plus size={15} />
              添加
            </button>
            <details className="row-menu">
              <summary aria-label="网关操作">
                <MoreHorizontal size={17} />
              </summary>
              <div className="menu-popover" onClick={menuClose}>
                <button
                  onClick={() => quota.refreshAll()}
                  disabled={!state.providers.length}
                >
                  刷新全部额度
                </button>
                <button
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
              </div>
            </details>
          </div>
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
      {(error || state.error || state.configError) && (
        <div className="banner error" role="alert">
          {error || state.error || state.configError}
        </div>
      )}
      {section === "gateway" ? (
        <>
          {state.waitingRequests > 0 && (
            <div className="gateway-waiting">等待 {state.waitingRequests}</div>
          )}
          <div className="provider-list">
            {!state.providers.length && (
              <div className="empty">
                <Network size={24} />
                <strong>尚未添加供应商</strong>
              </div>
            )}
            {state.providers.map((p, i) => {
              const selected =
                state.mode === "manual" && state.selected === p.id;
              const status = providerStatus(
                p,
                state.proxies.find((x) => x.id === p.proxyId),
              );
              return (
                <article
                  key={p.id}
                  className={`provider-row ${selected ? "selected" : ""}`}
                >
                  <div className="provider-main">
                    {p.queued && (
                      <span className="priority">
                        P
                        {
                          state.providers
                            .slice(0, i + 1)
                            .filter((p) => p.queued).length
                        }
                      </span>
                    )}
                    <strong className="provider-title" title={p.name}>
                      {p.name}
                    </strong>
                    {status && (
                      <button
                        className="provider-alert"
                        onClick={() => setDialog({ kind: "settings", item: p })}
                      >
                        {status}
                      </button>
                    )}
                    {state.configProvider === p.id && !selected && (
                      <span className="provider-binding">配置使用</span>
                    )}
                    {state.mode === "auto" && state.lastSuccessful === p.id && (
                      <span className="provider-binding">最近使用</span>
                    )}
                    <button
                      className="secondary compact"
                      disabled={busy}
                      onClick={() => choose(p)}
                    >
                      {selected ? (
                        <>
                          <Check size={14} />
                          已选择
                        </>
                      ) : (
                        "选择"
                      )}
                    </button>
                    <details className="row-menu">
                      <summary aria-label={`${p.name} 操作`}>
                        <MoreHorizontal size={17} />
                      </summary>
                      <div className="menu-popover" onClick={menuClose}>
                        <button
                          onClick={() =>
                            setDialog({ kind: "settings", item: p })
                          }
                        >
                          <Settings2 size={14} />
                          供应商设置
                        </button>
                        <button
                          onClick={() =>
                            setDialog({ kind: "provider", item: p })
                          }
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
                            run(async () =>
                              notify(
                                `TCP / TLS 连接正常 · ${await command<number>("test_provider", { id: p.id })} ms`,
                              ),
                            )
                          }
                        >
                          测试连接
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
                          disabled={busy || i === 0}
                          onClick={() => move(p.id, -1)}
                        >
                          <ArrowUp size={14} />
                          上移
                        </button>
                        <button
                          disabled={busy || i === state.providers.length - 1}
                          onClick={() => move(p.id, 1)}
                        >
                          <ArrowDown size={14} />
                          下移
                        </button>
                        <button
                          className="danger"
                          disabled={busy}
                          onClick={() => {
                            if (confirm(`删除供应商“${p.name}”？`))
                              run(() =>
                                edit({ op: "deleteProvider", id: p.id }),
                              );
                          }}
                        >
                          <Trash2 size={14} />
                          删除
                        </button>
                      </div>
                    </details>
                  </div>
                  <div className="provider-secondary">
                    <span className="provider-domain" title={p.baseUrl}>
                      {new URL(p.baseUrl).host}
                    </span>
                    <QuotaInfo
                      compact
                      provider={p}
                      quota={quota.quotaFor(p)}
                      refresh={() => void quota.refresh(p.id)}
                      details={() => setDialog({ kind: "quota", item: p })}
                    />
                  </div>
                </article>
              );
            })}
          </div>
          <details className="advanced">
            <summary>高级设置</summary>
            <div className="gateway-diagnostics">
              <code>{state.address}</code>
              <span>{state.running ? "运行中" : "已关闭"}</span>
              <span>活动连接 {state.activeConnections}</span>
              <span>
                配置使用{" "}
                {state.configState === "gateway"
                  ? "本地网关"
                  : (state.providers.find((p) => p.id === state.configProvider)
                      ?.name ?? "未匹配供应商")}
              </span>
            </div>
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
              {p.health.state !== "closed" && (
                <span className="provider-alert">{healthText(p.health)}</span>
              )}
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
        </div>
      )}
      {dialog?.kind === "settings" && (
        <ProviderSettings
          provider={dialog.item}
          runtime={
            state.providers.find((p) => p.id === dialog.item.id) ?? dialog.item
          }
          proxies={state.proxies}
          onDirtyChange={onDirtyChange}
          close={() => setDialog(null)}
          models={() => setDialog({ kind: "models", item: dialog.item })}
          save={async (payload) => {
            await edit(payload);
            setDialog(null);
          }}
        />
      )}
      {dialog?.kind === "quota" && (
        <Modal title={`${dialog.item.name} 额度`} close={() => setDialog(null)}>
          <QuotaInfo
            provider={dialog.item}
            quota={quota.quotaFor(
              state.providers.find((p) => p.id === dialog.item.id) ??
                dialog.item,
            )}
            refresh={() => void quota.refresh(dialog.item.id)}
          />
        </Modal>
      )}
      {dialog?.kind === "models" && (
        <ModelPolicyDialog
          provider={dialog.item}
          version={
            state.providers.find((p) => p.id === dialog.item.id)
              ?.quotaVersion ?? "deleted"
          }
          onDirtyChange={onDirtyChange}
          close={() => setDialog(null)}
          save={async (allowedModels) => {
            await edit({
              op: "modelsProvider",
              id: dialog.item.id,
              allowedModels,
            });
            setDialog(null);
          }}
        />
      )}
      {dialog &&
        (dialog.kind === "provider" ||
          dialog.kind === "proxy" ||
          dialog.kind === "rename") && (
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
    ["rateLimitSeconds", "429 默认冷却 / 秒"],
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
  dialog: Extract<
    NonNullable<Dialog>,
    { kind: "provider" | "proxy" | "rename" }
  >;
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
