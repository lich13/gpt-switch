import { useMemo, useState } from "react";
import type { Aggregate, Dashboard, Tokens } from "./usage-types";
import { compact, number, money, date } from "./usage-types";
const colors = ["var(--accent)", "#2B7FFF", "#8B5CF6", "#D49B50"];
const empty: Aggregate = {
  requests: 0,
  successes: 0,
  failures: 0,
  rejected: 0,
  cancelled: 0,
  unknown: 0,
  pending: 0,
  attempts: 0,
  legacyAttempts: 0,
  tokens: {
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    cacheWriteHour: null,
    inputImages: null,
    outputImages: null,
    images: null,
    context: null,
  },
  cost: "0",
  unpriced: 0,
  partial: 0,
  missingUsage: 0,
  latencyMs: 0,
  gatewayErrors: 0,
};
export function trendRows(data: Dashboard) {
  if (!data.trends.length) return [];
  const day = data.precision === "day",
    bucket = (at: number) =>
      day
        ? Math.floor(new Date(at * 1000).setHours(0, 0, 0, 0) / 1000)
        : Math.floor(at / 3600) * 3600;
  const end =
    data.effectiveEnd ?? data.trends.at(-1)!.at + (day ? 86400 : 3600);
  const points = new Map(data.trends.map((r) => [r.at, r]));
  const rows: Dashboard["trends"] = [];
  for (
    let at = bucket(data.effectiveStart ?? data.trends[0].at);
    at < end && rows.length < 4096;
  ) {
    rows.push(points.get(at) ?? { ...empty, at });
    if (day) {
      const next = new Date(at * 1000);
      next.setDate(next.getDate() + 1);
      at = Math.floor(next.valueOf() / 1000);
    } else at += 3600;
  }
  return rows;
}
export default function UsageChart({
  data,
  zoom,
}: {
  data: Dashboard;
  zoom: (a: number, b: number) => void;
}) {
  const [mode, setMode] = useState("requests"),
    [hover, setHover] = useState<number | null>(null),
    [from, setFrom] = useState<number | null>(null);
  const rows = useMemo(() => trendRows(data), [data]);
  const keys =
    mode === "tokens" ? ["input", "output", "cacheRead", "cacheWrite"] : [mode];
  const value = (i: number, key: string): number | null =>
    key === "requests"
      ? rows[i].requests
      : key === "cost"
        ? rows[i].cost == null
          ? null
          : Number(rows[i].cost)
        : rows[i].tokens[key as keyof Tokens];
  const max = Math.max(
    1,
    ...rows.flatMap((_, i) => keys.map((k) => value(i, k) ?? 0)),
  );
  const first = rows[0]?.at ?? 0,
    last = rows.at(-1)?.at ?? first + 1;
  const x = (i: number) =>
      48 +
      (rows.length === 1
        ? 0.5
        : (rows[i].at - first) / Math.max(1, last - first)) *
        660,
    y = (n: number) => 166 - (n / max) * 138;
  const nearest = (target: number) =>
    rows.reduce(
      (best, r, i) =>
        Math.abs(r.at - target) < Math.abs(rows[best].at - target) ? i : best,
      0,
    );
  const index = (e: React.PointerEvent<SVGSVGElement>) =>
    nearest(
      first +
        Math.max(
          0,
          Math.min(
            1,
            (((e.clientX - e.currentTarget.getBoundingClientRect().left) /
              e.currentTarget.getBoundingClientRect().width) *
              744 -
              48) /
              660,
          ),
        ) *
          (last - first),
    );
  const path = (key: string) => {
    let move = true;
    return rows
      .map((_, i) => {
        const v = value(i, key);
        if (v === null) {
          move = true;
          return "";
        }
        const point = `${move ? "M" : "L"}${x(i)},${y(v)}`;
        move = false;
        return point;
      })
      .join(" ");
  };
  const label = (at: number) =>
    new Date(at * 1000).toLocaleString(
      "zh-CN",
      data.precision === "hour"
        ? { hour: "2-digit", minute: "2-digit", hour12: false }
        : { month: "short", day: "numeric" },
    );
  return (
    <section className="usage-card trend-card">
      <div className="usage-section-heading">
        <h2>用量趋势</h2>
        <div className="segmented">
          {[
            ["requests", "请求"],
            ["tokens", "Token"],
            ["cost", "估算费用"],
          ].map(([k, n]) => (
            <button
              key={k}
              aria-pressed={mode === k}
              onClick={() => setMode(k)}
            >
              {n}
            </button>
          ))}
        </div>
      </div>
      {rows.length ? (
        <>
          <svg
            viewBox="0 0 744 202"
            role="img"
            tabIndex={0}
            aria-label="用量趋势；方向键查看，回车放大当前时段，拖动选择范围"
            onKeyDown={(e) => {
              if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
                e.preventDefault();
                setHover(
                  Math.max(
                    0,
                    Math.min(
                      rows.length - 1,
                      (hover ?? 0) + (e.key === "ArrowRight" ? 1 : -1),
                    ),
                  ),
                );
              } else if (e.key === "Enter" && hover !== null)
                zoom(
                  rows[hover].at,
                  rows[hover + 1]?.at ??
                    rows[hover].at + (data.precision === "hour" ? 3600 : 86400),
                );
            }}
            onPointerMove={(e) => setHover(index(e))}
            onPointerLeave={() => {
              if (from === null) setHover(null);
            }}
            onPointerDown={(e) => {
              setFrom(index(e));
              e.currentTarget.setPointerCapture(e.pointerId);
            }}
            onPointerUp={(e) => {
              const to = index(e);
              if (from !== null && to !== from)
                zoom(
                  rows[Math.min(from, to)].at,
                  rows[Math.max(from, to) + 1]?.at ??
                    rows[Math.max(from, to)].at +
                      (data.precision === "hour" ? 3600 : 86400),
                );
              setFrom(null);
            }}
            onPointerCancel={() => setFrom(null)}
          >
            {[0, 0.5, 1].map((f) => (
              <g key={f}>
                <line
                  x1="48"
                  x2="708"
                  y1={y(f * max)}
                  y2={y(f * max)}
                  stroke="var(--line)"
                  strokeDasharray={f ? "3 5" : undefined}
                />
                <text x="38" y={y(f * max) + 4} textAnchor="end">
                  {mode === "cost" ? `$${compact(f * max)}` : compact(f * max)}
                </text>
              </g>
            ))}
            {keys.map((k, c) => (
              <g key={k}>
                <path
                  fill="none"
                  stroke={colors[c]}
                  strokeWidth="2.5"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d={path(k)}
                />
                {rows.length === 1 && value(0, k) !== null && (
                  <circle
                    cx={x(0)}
                    cy={y(value(0, k)!)}
                    r="3"
                    fill={colors[c]}
                  />
                )}
              </g>
            ))}
            {hover !== null && rows[hover] && (
              <line
                x1={x(hover)}
                x2={x(hover)}
                y1="24"
                y2="168"
                stroke="var(--muted)"
                strokeDasharray="3 3"
              />
            )}
            {from !== null && hover !== null && rows[from] && rows[hover] && (
              <rect
                x={Math.min(x(from), x(hover))}
                y="24"
                width={Math.abs(x(from) - x(hover))}
                height="144"
                fill="#5ce1e620"
              />
            )}
            <text x="48" y="194">
              {label(first)}
            </text>
            {rows.length > 2 && (
              <text x="378" y="194" textAnchor="middle">
                {label(rows[Math.floor(rows.length / 2)].at)}
              </text>
            )}
            <text x="708" y="194" textAnchor="end">
              {label(last)}
            </text>
          </svg>
          <div className="chart-legend">
            {keys.map((k, c) => (
              <span key={k}>
                <i style={{ background: colors[c] }} />
                {
                  (
                    {
                      input: "输入",
                      output: "输出",
                      cacheRead: "缓存读取",
                      cacheWrite: "缓存写入",
                      requests: "请求",
                      cost: "估算费用",
                    } as Record<string, string>
                  )[k]
                }{" "}
                {hover !== null && rows[hover]
                  ? k === "cost"
                    ? money(rows[hover].cost)
                    : number(value(hover, k))
                  : ""}
              </span>
            ))}
            {hover !== null && rows[hover] && (
              <time>{date(rows[hover].at)}</time>
            )}
          </div>
        </>
      ) : (
        <div className="usage-empty">当前范围没有请求</div>
      )}
    </section>
  );
}
