import { useCallback, useEffect, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
import { command, subscribe } from "./bridge";
import {
  errorOf,
  type Provider,
  type ProviderQuota,
  type QuotaPlan,
} from "./types";

export function useProviderQuota(
  providers: Provider[],
  active: boolean,
  visibilityEvent = "app-visibility",
) {
  const latest = useRef(providers);
  latest.current = providers;
  const [values, setValues] = useState<Record<string, ProviderQuota>>({});
  const [nativeVisible, setNativeVisible] = useState(true);
  const [visible, setVisible] = useState(document.visibilityState !== "hidden");
  const flights = useRef(new Set<string>());
  const mounted = useRef(true);
  const accept = useCallback((q: ProviderQuota) => {
    if (
      !mounted.current ||
      !q ||
      !latest.current.some(
        (p) => p.id === q.providerId && p.quotaVersion === q.version,
      )
    )
      return;
    setValues((v) => ({ ...v, [q.providerId]: q }));
  }, []);
  useEffect(() => {
    mounted.current = true;
    const clean: (() => void)[] = [];
    let disposed = false;
    for (const promise of [
      subscribe<ProviderQuota>("provider-quota", accept),
      subscribe<boolean>(visibilityEvent, setNativeVisible),
    ]) {
      void promise.then((fn) => (disposed ? fn() : clean.push(fn)));
    }
    const onVisibility = () =>
      setVisible(document.visibilityState !== "hidden");
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      disposed = true;
      mounted.current = false;
      clean.forEach((fn) => fn());
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [accept, visibilityEvent]);
  const refresh = useCallback(
    async (id: string, force = true) => {
      const provider = latest.current.find((p) => p.id === id);
      if (!provider) return;
      const flight = id + provider.quotaVersion;
      if (flights.current.has(flight)) return;
      flights.current.add(flight);
      try {
        accept(
          await command<ProviderQuota>("query_provider_quota", {
            providerId: id,
            force,
          }),
        );
      } catch (e) {
        const err = errorOf(e);
        if (
          err.code !== "STALE" &&
          mounted.current &&
          latest.current.some(
            (p) => p.id === id && p.quotaVersion === provider.quotaVersion,
          )
        ) {
          setValues((v) => ({
            ...v,
            [id]: {
              ...(v[id] ?? provider.quota ?? emptyQuota(provider)),
              state: "error",
              stale: Boolean((v[id] ?? provider.quota)?.successAt),
              error: err.message,
            },
          }));
        }
      } finally {
        flights.current.delete(flight);
      }
    },
    [accept],
  );
  const refreshAll = useCallback(
    (force = true) => {
      for (const p of latest.current) void refresh(p.id, force);
    },
    [refresh],
  );
  const identity = providers.map((p) => p.id + p.quotaVersion).join("|");
  useEffect(() => {
    if (!active || !nativeVisible || !visible || !identity) return;
    refreshAll(false);
    const timer = window.setInterval(() => refreshAll(false), 60_000);
    return () => window.clearInterval(timer);
  }, [active, nativeVisible, visible, identity, refreshAll]);
  const quotaFor = (p: Provider) => {
    const q = values[p.id];
    return q?.version === p.quotaVersion ? q : (p.quota ?? undefined);
  };
  return { quotaFor, refresh, refreshAll };
}
function emptyQuota(p: Provider): ProviderQuota {
  return {
    providerId: p.id,
    version: p.quotaVersion,
    state: "idle",
    source: null,
    checkedAt: null,
    successAt: null,
    retryAt: null,
    stale: false,
    error: null,
    keyStatus: null,
    plans: [],
    expiresAt: null,
    expiresAtUnix: null,
    today: null,
    totalUsage: null,
  };
}
const number = (n: number) =>
  n.toLocaleString(undefined, { maximumFractionDigits: 3 });
const amount = (p: QuotaPlan) =>
  p.unlimited
    ? "无限制"
    : p.remaining == null
      ? "未返回剩余额度"
      : `${number(p.remaining)} ${p.unit}`;
const date = (value: string | number) => {
  const d = new Date(value);
  return Number.isNaN(d.valueOf()) ? "" : d.toLocaleString();
};
export function QuotaInfo({
  provider,
  quota,
  refresh,
}: {
  provider: Provider;
  quota?: ProviderQuota;
  refresh: () => void;
}) {
  const loading = quota?.state === "loading";
  const stale =
    quota?.stale ||
    Boolean(quota?.successAt && Date.now() / 1000 - quota.successAt >= 60);
  const headline = quota?.plans[0]
    ? `${quota.plans[0].unlimited ? "" : "剩余 "}${amount(quota.plans[0])}`
    : (quota?.keyStatus ??
      (quota?.state === "unsupported"
        ? "不可查询"
        : quota?.state === "error"
          ? "查询失败"
          : loading
            ? "查询中…"
            : "未查询"));
  const source =
    quota?.source === "sub2api"
      ? "Sub2API"
      : quota?.source === "newapi"
        ? "New API"
        : "";
  const expiry = quota?.expiresAtUnix
    ? date(quota.expiresAtUnix * 1000)
    : quota?.expiresAt
      ? date(quota.expiresAt)
      : "";
  return (
    <div className="quota-area" aria-label={`${provider.name} 额度`}>
      <div className="quota-summary">
        <strong>{headline}</strong>
        {source && <span>{source}</span>}
        {quota?.keyStatus && quota.plans.length > 0 && (
          <span className="quota-warning">{quota.keyStatus}</span>
        )}
        {stale && <span className="quota-warning">已过期</span>}
        {quota?.successAt && (
          <time
            dateTime={new Date(quota.successAt * 1000).toISOString()}
            title={date(quota.successAt * 1000)}
          >
            {new Date(quota.successAt * 1000).toLocaleTimeString()}
          </time>
        )}
        <button
          className="icon-button"
          disabled={
            loading ||
            Boolean(quota?.retryAt && quota.retryAt > Date.now() / 1000)
          }
          onClick={refresh}
          aria-label={`刷新 ${provider.name} 额度`}
          title="刷新额度"
        >
          <RefreshCw size={13} className={loading ? "quota-spin" : ""} />
        </button>
      </div>
      {quota?.error && (
        <div className="quota-error" role="status">
          {quota.error}
          {quota.retryAt ? ` · ${date(quota.retryAt * 1000)} 后重试` : ""}
        </div>
      )}
      {quota &&
        (quota.plans.length > 0 ||
          expiry ||
          quota.today ||
          quota.totalUsage) && (
          <details className="quota-details">
            <summary>额度详情</summary>
            {quota.plans.map((p, i) => (
              <div key={i} className="quota-plan">
                <span>{p.name}</span>
                <strong>{amount(p)}</strong>
                <span>
                  {p.used != null ? `已用 ${number(p.used)} ${p.unit}` : ""}
                  {p.total != null && !p.unlimited
                    ? ` / ${number(p.total)} ${p.unit}`
                    : ""}
                </span>
                {p.resetAt && <span>重置 {date(p.resetAt) || p.resetAt}</span>}
              </div>
            ))}
            {expiry && <div>到期 {expiry}</div>}
            {(
              [
                ["今日", quota.today],
                ["累计", quota.totalUsage],
              ] as const
            ).map(
              ([label, u]) =>
                u && (
                  <div key={label}>
                    {label} {u.requests != null && `${number(u.requests)} 次`}
                    {u.tokens != null && ` · ${number(u.tokens)} Token`}
                    {u.cost != null && ` · 费用 ${number(u.cost)}`}
                  </div>
                ),
            )}
          </details>
        )}
    </div>
  );
}
