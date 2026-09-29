import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowDownRight,
  ArrowUpRight,
  ChevronRight,
  RefreshCw,
  SlidersHorizontal,
} from "lucide-react";
import { command, subscribe } from "./bridge";
import { errorOf, type GatewayState, type Provider } from "./types";
import type {
  Dashboard,
  Filters,
  UsageState,
  UsageSettings,
  LogPage,
  LogicalRecord,
  LogicalDetail,
  Group,
} from "./usage-types";
import {
  number,
  money,
  date,
  totalTokens,
  compact,
  compactMoney,
  successRate,
  outcomeLabel,
} from "./usage-types";
import UsageDialog from "./UsageDialog";
import UsageDrawer from "./UsageDrawer";
import UsageChart from "./UsageChart";
import Pricing from "./Pricing";
import { useProviderQuota } from "./Quota";
const localDate = (n: number) => {
  const d = new Date(n);
  return new Date(n - d.getTimezoneOffset() * 60000).toISOString().slice(0, 16);
};
export function range(
  preset: string,
  start: string,
  end: string,
  live: boolean,
  now = Date.now(),
): Filters {
  const finish = live ? now : Date.parse(end);
  const begin =
    preset === "custom"
      ? Date.parse(start)
      : preset === "today"
        ? new Date(now).setHours(0, 0, 0, 0)
        : now - Number(preset) * 86400000;
  return {
    start: Math.floor(begin / 1000),
    end: Math.floor((preset === "custom" ? finish : now) / 1000) + 1,
  };
}
export default function Usage({
  onDirtyChange,
}: {
  onDirtyChange: (dirty: boolean) => void;
}) {
  const [preset, setPreset] = useState("today"),
    [start, setStart] = useState(localDate(Date.now() - 86400000)),
    [end, setEnd] = useState(localDate(Date.now())),
    [live, setLive] = useState(true);
  const [provider, setProvider] = useState(""),
    [model, setModel] = useState(""),
    [status, setStatus] = useState(""),
    [outcome, setOutcome] = useState(""),
    [tab, setTab] = useState("overview"),
    [page, setPage] = useState(0),
    [ranking, setRanking] = useState("providers");
  const [data, setData] = useState<Dashboard | null>(null),
    [logs, setLogs] = useState<LogPage | null>(null),
    [settings, setSettings] = useState<UsageState | null>(null),
    [edit, setEdit] = useState<UsageSettings | null>(null),
    [detail, setDetail] = useState<LogicalDetail | null>(null);
  const [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [visible, setVisible] = useState(document.visibilityState !== "hidden"),
    [nativeVisible, setNativeVisible] = useState(true),
    [updated, setUpdated] = useState(0);
  const generation = useRef(0),
    detailGeneration = useRef(0),
    latest = useRef({
      preset,
      start,
      end,
      live,
      provider,
      model,
      status,
      outcome,
      page,
    });
  latest.current = {
    preset,
    start,
    end,
    live,
    provider,
    model,
    status,
    outcome,
    page,
  };
  const load = useCallback(async () => {
    const n = ++generation.current,
      p = latest.current;
    const f = {
      ...range(p.preset, p.start, p.end, p.live),
      ...(p.provider
        ? { providerId: p.provider === "__gateway" ? "" : p.provider }
        : {}),
      ...(p.model ? { model: p.model } : {}),
      ...(p.status ? { status: Number(p.status) } : {}),
      ...(p.outcome ? { outcome: p.outcome } : {}),
    };
    if (
      !Number.isFinite(f.start) ||
      !Number.isFinite(f.end) ||
      f.end! <= f.start!
    ) {
      setError("请选择有效的时间范围");
      setBusy(false);
      return;
    }
    setBusy(true);
    try {
      const [d, l, s] = await Promise.all([
        command<Dashboard>("get_usage_dashboard", { filters: f }),
        command<LogPage>("get_usage_logs", { filters: f, page: p.page }),
        command<UsageState>("get_usage_state"),
      ]);
      if (n === generation.current) {
        setData(d);
        setLogs(l);
        setSettings(s);
        setError(s.error ?? "");
        setUpdated(Date.now());
      }
    } catch (e) {
      if (n === generation.current) setError(errorOf(e).message);
    } finally {
      if (n === generation.current) setBusy(false);
    }
  }, []);
  useEffect(() => {
    let disposed = false;
    const clean: (() => void)[] = [];
    void subscribe<boolean>("app-visibility", setNativeVisible).then((c) =>
      disposed ? c() : clean.push(c),
    );
    const f = () => setVisible(document.visibilityState !== "hidden");
    document.addEventListener("visibilitychange", f);
    return () => {
      disposed = true;
      generation.current++;
      detailGeneration.current++;
      clean.forEach((c) => c());
      document.removeEventListener("visibilitychange", f);
    };
  }, []);
  useEffect(() => {
    if (visible && nativeVisible && tab !== "pricing") void load();
  }, [
    preset,
    start,
    end,
    live,
    provider,
    model,
    status,
    outcome,
    page,
    tab,
    visible,
    nativeVisible,
    load,
  ]);
  useEffect(() => {
    if (
      !visible ||
      !nativeVisible ||
      tab === "pricing" ||
      !settings?.settings.refreshSeconds
    )
      return;
    const t = setInterval(
      () => void load(),
      settings.settings.refreshSeconds * 1000,
    );
    return () => clearInterval(t);
  }, [visible, nativeVisible, tab, settings?.settings.refreshSeconds, load]);
  useEffect(() => {
    onDirtyChange(!!edit);
    return () => onDirtyChange(false);
  }, [edit, onDirtyChange]);
  const reset = () => setPage(0);
  const changeSettings = async (next: UsageSettings) => {
    if (!settings) return;
    setBusy(true);
    try {
      setSettings(
        await command<UsageState>("set_usage_settings", {
          settings: next,
          expectedRevision: settings.revision,
        }),
      );
      setEdit(null);
      setError("");
    } catch (e) {
      setError(errorOf(e).message);
    } finally {
      setBusy(false);
    }
  };
  const zoom = (a: number, b: number) => {
    setPreset("custom");
    setStart(localDate(a * 1000));
    setEnd(localDate(b * 1000));
    setLive(false);
    reset();
  };
  const openDetail = async (r: LogicalRecord) => {
    const n = ++detailGeneration.current;
    try {
      const next = await command<LogicalDetail | null>("get_usage_detail", {
        id: r.id,
      });
      if (n === detailGeneration.current) {
        if (next) setDetail(next);
        else setError("该请求明细已归档");
      }
    } catch (e) {
      if (n === detailGeneration.current) setError(errorOf(e).message);
    }
  };
  const summary = data?.summary,
    tokens = summary?.tokens;
  const cache =
    tokens?.cacheRead != null && tokens.input != null
      ? `${((tokens.cacheRead / Math.max(1, tokens.input + tokens.cacheRead + (tokens.cacheWrite ?? 0))) * 100).toFixed(1)}%`
      : "未提供";
  const setFilter = (setter: (v: string) => void, value: string) => {
    setter(value);
    reset();
  };
  const ranks =
    (ranking === "providers" ? data?.providers : data?.models)
      ?.slice()
      .sort((a, b) => b.requests - a.requests)
      .slice(0, 5) ?? [];
  return (
    <section className="usage-page">
      <div className="page-heading">
        <h1>使用统计</h1>
        <div className="heading-actions">
          <button
            className="icon-button"
            title="记录设置"
            aria-label="记录设置"
            disabled={!settings}
            onClick={() => setEdit(structuredClone(settings!.settings))}
          >
            <SlidersHorizontal size={17} />
          </button>
          <button
            className="secondary"
            disabled={busy}
            onClick={() => void load()}
          >
            <RefreshCw size={14} className={busy ? "spin" : ""} />
            刷新
          </button>
        </div>
      </div>
      <nav className="usage-tabs" aria-label="统计视图">
        {[
          ["overview", "概览"],
          ["logs", "请求日志"],
          ["providers", "供应商"],
          ["models", "模型"],
          ["pricing", "定价"],
        ].map(([k, n]) => (
          <button
            key={k}
            className={tab === k ? "active" : ""}
            aria-current={tab === k ? "page" : undefined}
            onClick={() => setTab(k)}
          >
            {n}
          </button>
        ))}
      </nav>
      {error && (
        <div className="form-error" role="alert">
          {error}
        </div>
      )}
      {tab === "pricing" ? (
        <Pricing onDirtyChange={onDirtyChange} />
      ) : (
        <>
          <div className="usage-filters">
            <label className="usage-range">
              时间范围
              <select
                value={preset}
                onChange={(e) => setFilter(setPreset, e.target.value)}
              >
                {[
                  ["today", "今日"],
                  ["1", "最近 24 小时"],
                  ["7", "最近 7 天"],
                  ["14", "最近 14 天"],
                  ["30", "最近 30 天"],
                  ["custom", "自定义"],
                ].map(([v, n]) => (
                  <option key={v} value={v}>
                    {n}
                  </option>
                ))}
              </select>
            </label>
            <label>
              最终供应商
              <select
                value={provider}
                onChange={(e) => setFilter(setProvider, e.target.value)}
              >
                <option value="">全部供应商</option>
                {data?.availableProviders.map(([id, name]) => (
                  <option key={id || "gateway"} value={id || "__gateway"}>
                    {name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              最终计价模型
              <select
                value={model}
                onChange={(e) => setFilter(setModel, e.target.value)}
              >
                <option value="">全部模型</option>
                {data?.availableModels.map((m) => (
                  <option key={m}>{m}</option>
                ))}
              </select>
            </label>
          </div>
          {preset === "custom" && (
            <div className="usage-date-range">
              <label>
                开始
                <input
                  type="datetime-local"
                  value={start}
                  onChange={(e) => setFilter(setStart, e.target.value)}
                />
              </label>
              <label>
                结束
                <input
                  type="datetime-local"
                  disabled={live}
                  value={end}
                  onChange={(e) => setFilter(setEnd, e.target.value)}
                />
              </label>
              <label className="check-label">
                <input
                  type="checkbox"
                  checked={live}
                  onChange={(e) => setLive(e.target.checked)}
                />
                跟随当前时间
              </label>
            </div>
          )}
          {(status || outcome) && tab !== "logs" && (
            <button
              className="text-button usage-filter-clear"
              onClick={() => {
                setStatus("");
                setOutcome("");
                reset();
              }}
            >
              清除结果筛选 · {outcomeLabel(outcome)} {status}
            </button>
          )}
          {tab === "overview" && summary && (
            <>
              <div className="usage-kpis">
                <Metric
                  label="逻辑请求"
                  value={compact(summary.requests)}
                  exact={number(summary.requests)}
                >
                  <dl>
                    {[
                      ["成功", summary.successes],
                      ["服务失败", summary.failures],
                      ["业务拒绝", summary.rejected],
                      ["取消", summary.cancelled],
                      ["进行中", summary.pending],
                      ["未确认", summary.unknown],
                      ["上游尝试", summary.attempts],
                    ].map(([k, v]) => (
                      <div key={k}>
                        <dt>{k}</dt>
                        <dd>{number(Number(v))}</dd>
                      </div>
                    ))}
                  </dl>
                </Metric>
                <Metric label="服务成功率" value={successRate(summary)}>
                  <dl>
                    <div>
                      <dt>成功</dt>
                      <dd>{number(summary.successes)}</dd>
                    </div>
                    <div>
                      <dt>服务失败</dt>
                      <dd>{number(summary.failures)}</dd>
                    </div>
                    <div>
                      <dt>网关失败</dt>
                      <dd>{number(summary.gatewayErrors)}</dd>
                    </div>
                  </dl>
                </Metric>
                <Metric
                  label="Token"
                  value={compact(totalTokens(summary.tokens))}
                  exact={number(totalTokens(summary.tokens))}
                >
                  <dl>
                    {[
                      ["输入", number(tokens?.input)],
                      ["输出", number(tokens?.output)],
                      ["缓存读取", number(tokens?.cacheRead)],
                      ["缓存写入", number(tokens?.cacheWrite)],
                      ["缓存命中率", cache],
                    ].map(([k, v]) => (
                      <div key={k}>
                        <dt>{k}</dt>
                        <dd>{v}</dd>
                      </div>
                    ))}
                  </dl>
                </Metric>
                <Metric
                  label="估算费用"
                  value={compactMoney(summary.cost)}
                  exact={money(summary.cost)}
                >
                  <dl>
                    <div>
                      <dt>完整金额</dt>
                      <dd>{money(summary.cost)}</dd>
                    </div>
                    <div>
                      <dt>未定价</dt>
                      <dd>{summary.unpriced}</dd>
                    </div>
                    <div>
                      <dt>部分计价</dt>
                      <dd>{summary.partial}</dd>
                    </div>
                    <div>
                      <dt>未报告用量</dt>
                      <dd>{summary.missingUsage}</dd>
                    </div>
                  </dl>
                </Metric>
              </div>
              <Balances />
              {data && <UsageChart data={data} zoom={zoom} />}
              <section className="usage-card usage-ranking">
                <div className="usage-section-heading">
                  <div className="segmented">
                    {[
                      ["providers", "供应商"],
                      ["models", "模型"],
                    ].map(([k, n]) => (
                      <button
                        key={k}
                        aria-pressed={ranking === k}
                        onClick={() => setRanking(k)}
                      >
                        {n}
                      </button>
                    ))}
                  </div>
                  <button
                    className="text-button"
                    onClick={() => setTab(ranking)}
                  >
                    查看全部
                    <ArrowUpRight size={13} />
                  </button>
                </div>
                {ranks.length ? (
                  ranks.map((r) => (
                    <button
                      key={r.id}
                      className="rank-row"
                      onClick={() => {
                        setTab("logs");
                        setFilter(
                          ranking === "providers" ? setProvider : setModel,
                          r.id || "__gateway",
                        );
                      }}
                    >
                      <span title={r.name}>{r.name}</span>
                      <span className="rank-bar">
                        <i
                          style={{
                            width: `${(r.requests / Math.max(1, ranks[0].requests)) * 100}%`,
                          }}
                        />
                      </span>
                      <span>{compact(r.requests)}</span>
                      <span>{compactMoney(r.cost)}</span>
                      <ChevronRight size={13} />
                    </button>
                  ))
                ) : (
                  <div className="usage-empty">当前范围没有请求</div>
                )}
              </section>
            </>
          )}
          {(tab === "providers" || tab === "models") && data && (
            <GroupTable
              kind={tab}
              rows={tab === "providers" ? data.providers : data.models}
            />
          )}
          {tab === "logs" && (
            <section className="usage-card usage-logs">
              <div className="usage-log-tools">
                <label>
                  结果
                  <select
                    value={outcome}
                    onChange={(e) => setFilter(setOutcome, e.target.value)}
                  >
                    <option value="">全部结果</option>
                    {[
                      "success",
                      "failure",
                      "rejected",
                      "cancelled",
                      "pending",
                      "unknown",
                    ].map((o) => (
                      <option key={o} value={o}>
                        {outcomeLabel(o)}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  状态码
                  <input
                    type="number"
                    placeholder="全部"
                    aria-label="状态码"
                    min="100"
                    max="599"
                    value={status}
                    onChange={(e) => setFilter(setStatus, e.target.value)}
                  />
                </label>
                <span>{number(logs?.total ?? 0)} 条</span>
              </div>
              <div className="usage-table-scroll">
                <table className="usage-table logs-table">
                  <thead>
                    <tr>
                      {[
                        "时间",
                        "最终供应商",
                        "模型",
                        "结果",
                        "Token",
                        "费用",
                      ].map((v, i) => (
                        <th
                          className={i === 4 ? "optional-column" : ""}
                          key={v}
                        >
                          {v}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {logs?.records.map((r) => (
                      <tr key={r.id} onClick={() => void openDetail(r)}>
                        <td>
                          <button
                            className="log-open"
                            aria-label={`请求详情 ${r.id}`}
                            title={date(r.createdAt)}
                          >
                            {new Date(r.createdAt * 1000).toLocaleTimeString(
                              "zh-CN",
                              { hour12: false },
                            )}
                          </button>
                        </td>
                        <td title={r.providerName}>{r.providerName}</td>
                        <td
                          title={r.billingModel ?? r.requestModel ?? "未提供"}
                        >
                          {r.billingModel ?? r.requestModel ?? "未提供"}
                        </td>
                        <td>
                          <span className={`outcome-badge ${r.outcomeClass}`}>
                            {outcomeLabel(r.outcomeClass)}
                          </span>
                          {r.attemptCount > 1 && (
                            <span
                              className="retry-count"
                              title={`${r.attemptCount} 次上游尝试`}
                            >
                              <ArrowDownRight size={11} />
                              {r.attemptCount}
                            </span>
                          )}
                        </td>
                        <td
                          className="optional-column"
                          title={number(totalTokens(r.tokens))}
                        >
                          {compact(totalTokens(r.tokens))}
                        </td>
                        <td title={money(r.cost.total)}>
                          {compactMoney(r.cost.total)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                {!logs?.records.length && (
                  <div className="usage-empty">当前范围没有明细</div>
                )}
              </div>
              <div className="usage-pagination">
                <span>每页 20 条</span>
                <button
                  disabled={!page || busy}
                  onClick={() => setPage((p) => p - 1)}
                >
                  上一页
                </button>
                <label>
                  <input
                    aria-label="日志页码"
                    type="number"
                    min="1"
                    max={Math.max(1, Math.ceil((logs?.total ?? 0) / 20))}
                    value={page + 1}
                    onChange={(e) => {
                      const n = Number(e.target.value);
                      if (
                        n >= 1 &&
                        n <= Math.max(1, Math.ceil((logs?.total ?? 0) / 20))
                      )
                        setPage(n - 1);
                    }}
                  />{" "}
                  / {Math.max(1, Math.ceil((logs?.total ?? 0) / 20))}
                </label>
                <button
                  disabled={busy || (page + 1) * 20 >= (logs?.total ?? 0)}
                  onClick={() => setPage((p) => p + 1)}
                >
                  下一页
                </button>
              </div>
            </section>
          )}
          <footer className="usage-footer">
            <span>
              {updated ? new Date(updated).toLocaleTimeString() : "—"}
              {data?.precision === "day" ? " · 按日汇总" : ""}
              {settings && !settings.settings.enabled ? " · 已暂停记录" : ""}
              {summary?.legacyAttempts
                ? ` · 旧口径尝试 ${summary.legacyAttempts}`
                : ""}
            </span>
            <label>
              自动刷新
              <select
                aria-label="自动刷新"
                value={settings?.settings.refreshSeconds ?? 30}
                disabled={!settings || busy}
                onChange={(e) =>
                  void changeSettings({
                    ...settings!.settings,
                    refreshSeconds: Number(e.target.value),
                  })
                }
              >
                {[0, 5, 10, 30, 60].map((n) => (
                  <option key={n} value={n}>
                    {n ? `${n} 秒` : "关闭"}
                  </option>
                ))}
              </select>
            </label>
          </footer>
        </>
      )}
      {detail && (
        <UsageDrawer
          detail={detail}
          close={() => {
            detailGeneration.current++;
            setDetail(null);
          }}
        />
      )}
      {edit && (
        <UsageDialog title="记录设置" close={() => setEdit(null)}>
          <form
            className="usage-form"
            onSubmit={(e) => {
              e.preventDefault();
              void changeSettings(edit);
            }}
          >
            <label className="check-label">
              <input
                type="checkbox"
                checked={edit.enabled}
                onChange={(e) =>
                  setEdit({ ...edit, enabled: e.target.checked })
                }
              />
              记录网关用量
            </label>
            <label>
              计价模型
              <select
                value={edit.modelSource}
                onChange={(e) =>
                  setEdit({
                    ...edit,
                    modelSource: e.target.value as "request" | "response",
                  })
                }
              >
                <option value="response">响应模型优先</option>
                <option value="request">请求模型优先</option>
              </select>
            </label>
            <label>
              成本倍率
              <input
                type="number"
                min="0"
                step="any"
                required
                value={edit.costMultiplier}
                onChange={(e) =>
                  setEdit({ ...edit, costMultiplier: e.target.value })
                }
              />
            </label>
            <label>
              明细保留天数
              <input
                type="number"
                min="1"
                max="3650"
                required
                value={edit.retentionDays}
                onChange={(e) =>
                  setEdit({ ...edit, retentionDays: Number(e.target.value) })
                }
              />
            </label>
            <details>
              <summary>供应商成本倍率</summary>
              {data?.availableProviders
                .filter(([id]) => id)
                .map(([id, name]) => (
                  <label key={id}>
                    {name}
                    <input
                      type="number"
                      min="0"
                      step="any"
                      placeholder="继承全局"
                      value={edit.providerMultipliers[id] ?? ""}
                      onChange={(e) => {
                        const m = { ...edit.providerMultipliers };
                        if (e.target.value) m[id] = e.target.value;
                        else delete m[id];
                        setEdit({ ...edit, providerMultipliers: m });
                      }}
                    />
                  </label>
                ))}
            </details>
            {error && (
              <div className="form-error" role="alert">
                {error}
              </div>
            )}
            <button className="primary" disabled={busy}>
              保存
            </button>
          </form>
        </UsageDialog>
      )}
    </section>
  );
}
function Metric({
  label,
  value,
  exact,
  children,
}: {
  label: string;
  value: string;
  exact?: string;
  children: React.ReactNode;
}) {
  return (
    <details className="usage-metric">
      <summary>
        <span>
          {label}
          <ChevronRight size={12} />
        </span>
        <strong title={exact}>{value}</strong>
      </summary>
      <div className="metric-popover">{children}</div>
    </details>
  );
}
function Balances() {
  const [providers, setProviders] = useState<Provider[]>([]),
    [error, setError] = useState("");
  useEffect(() => {
    let disposed = false;
    void command<GatewayState>("get_gateway")
      .then((g) => {
        if (!disposed) setProviders(g.providers);
      })
      .catch((e) => {
        if (!disposed) setError(errorOf(e).message);
      });
    return () => {
      disposed = true;
    };
  }, []);
  const { quotaFor, refresh } = useProviderQuota(providers, false);
  return (
    <details className="usage-balances">
      <summary>
        供应商额度
        <ChevronRight size={14} />
      </summary>
      {error && <div className="form-error">{error}</div>}
      <div>
        {providers.map((p) => {
          const q = quotaFor(p),
            plan = q?.plans[0];
          const amount = plan?.unlimited
            ? "无限制"
            : plan?.remaining == null
              ? q?.state === "unsupported"
                ? "不可查询"
                : "未查询"
              : `${number(plan.remaining)} ${plan.unit}`;
          return (
            <div className="balance-row" key={p.id}>
              <span title={p.name}>{p.name}</span>
              <strong
                title={q?.error ?? (q?.successAt ? date(q.successAt) : "")}
              >
                {amount}
                {q?.stale ? " · 已过期" : ""}
              </strong>
              <button
                className="icon-button"
                disabled={q?.state === "loading"}
                aria-label={`刷新 ${p.name} 额度`}
                onClick={() => void refresh(p.id)}
              >
                <RefreshCw size={13} />
              </button>
            </div>
          );
        })}
      </div>
    </details>
  );
}
function GroupTable({ rows, kind }: { rows: Group[]; kind: string }) {
  return (
    <section className="usage-card">
      <h2>{kind === "providers" ? "供应商统计" : "模型统计"}</h2>
      <div className="usage-table-scroll">
        <table className="usage-table group-table">
          <thead>
            <tr>
              <th>{kind === "providers" ? "供应商" : "计价模型"}</th>
              <th>最终请求</th>
              <th>成功率</th>
              <th>上游尝试</th>
              <th>Token</th>
              <th>上游费用</th>
              <th>{kind === "providers" ? "平均耗时" : "平均尝试费用"}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.id}>
                <td title={r.name}>{r.name}</td>
                <td>{number(r.requests)}</td>
                <td title={`成功 ${r.successes} / 服务失败 ${r.failures}`}>
                  {successRate(r)}
                </td>
                <td>{number(r.attempts)}</td>
                <td title={number(totalTokens(r.tokens))}>
                  {compact(totalTokens(r.tokens))}
                </td>
                <td title={money(r.cost)}>{compactMoney(r.cost)}</td>
                <td>
                  {kind === "providers"
                    ? r.requests
                      ? `${(r.latencyMs / r.requests / 1000).toFixed(1)} s`
                      : "—"
                    : r.cost === null
                      ? "未定价"
                      : money(String(Number(r.cost) / Math.max(1, r.attempts)))}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {!rows.length && <div className="usage-empty">暂无数据</div>}
      </div>
    </section>
  );
}
