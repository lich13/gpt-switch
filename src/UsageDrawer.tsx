import { useEffect, useRef } from "react";
import { X } from "lucide-react";
import type { LogicalDetail, UsageRecord } from "./usage-types";
import {
  number,
  money,
  date,
  speed,
  totalTokens,
  outcomeLabel,
  routingLabel,
  httpClass,
  logMoney,
} from "./usage-types";
export default function UsageDrawer({
  detail: r,
  close,
}: {
  detail: LogicalDetail;
  close: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const dialog = ref.current!;
    dialog.showModal();
    return () => {
      dialog.close();
      if (previous?.isConnected) previous.focus();
    };
  }, []);
  return (
    <dialog
      ref={ref}
      className="usage-drawer"
      aria-labelledby="usage-detail-title"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
    >
      <header className="drawer-heading">
        <div>
          <h2 id="usage-detail-title">请求详情</h2>
          <time>{date(r.createdAt)}</time>
        </div>
        <button
          className="icon-button"
          aria-label="关闭请求详情"
          onClick={close}
        >
          <X size={18} />
        </button>
      </header>
      <div className="drawer-body">
        <section aria-label="最终记录">
          <div className="request-result">
            <strong>{r.log.providerName}</strong>
            <span className={`http-status ${httpClass(r.log.status)}`}>
              {r.log.status ?? "—"}
            </span>
            <strong>{logMoney(r.log)}</strong>
            <span>{r.log.dataSource}</span>
          </div>
          <Attempt record={r.log} />
        </section>
        <details className="logical-detail">
          <summary>逻辑请求总消耗与真实终态</summary>
          <div className="request-result">
            <span className={`outcome-badge ${r.outcomeClass}`}>
              {outcomeLabel(r.outcomeClass)}
            </span>
            <strong>{money(r.cost.total)}</strong>
          </div>
          <dl className="usage-detail">
            <div className="detail-full">
              <dt>逻辑请求 ID</dt>
              <dd>{r.logicalId}</dd>
            </div>
            <div>
              <dt>最终供应商</dt>
              <dd>{r.providerName}</dd>
            </div>
            <div>
              <dt>总耗时</dt>
              <dd>{number(r.latencyMs)} ms</dd>
            </div>
            <div>
              <dt>上游尝试</dt>
              <dd>{r.attemptCount}</dd>
            </div>
            <div>
              <dt>总 Token</dt>
              <dd>{number(totalTokens(r.tokens))}</dd>
            </div>
            <div className="detail-full">
              <dt>终态</dt>
              <dd>
                {r.outcome}
                {r.terminalEvidence ? ` · ${r.terminalEvidence}` : ""}
              </dd>
            </div>
          </dl>
        </details>
        {r.routing.length > 0 && (
          <details className="routing-section">
            <summary>调度过程</summary>
            <ol>
              {r.routing.map((d, i) => (
                <li
                  key={i}
                  className={d.reason === "selected" ? "selected" : ""}
                >
                  <div>
                    <strong>{d.providerName}</strong>
                    <time>
                      {new Date(d.at * 1000).toLocaleTimeString("zh-CN", {
                        hour12: false,
                      })}
                    </time>
                  </div>
                  <span>
                    {routingLabel(d.reason)} · 并发 {d.active}/
                    {d.limit || "不限"}
                    {d.retryIn ? ` · ${d.retryIn}s` : ""}
                  </span>
                </li>
              ))}
            </ol>
          </details>
        )}
        <section className="attempts-section">
          <h3>上游尝试</h3>
          {r.attempts.length ? (
            r.attempts.map((a, i) => (
              <details className="attempt-detail" key={a.id}>
                <summary>
                  <span className="attempt-number">{i + 1}</span>
                  <strong title={a.providerName}>{a.providerName}</strong>
                  <span>{a.status ?? "—"}</span>
                  <Chevron />
                </summary>
                <Attempt record={a} />
              </details>
            ))
          ) : (
            <div className="usage-empty">未发送上游请求</div>
          )}
        </section>
      </div>
    </dialog>
  );
}
function Chevron() {
  return (
    <span aria-hidden="true" className="attempt-chevron">
      ⌄
    </span>
  );
}
function Attempt({ record: r }: { record: UsageRecord }) {
  const entries = [
    ["结果", r.outcome],
    ["流式", r.streaming ? "是" : "否"],
    ["请求模型", r.requestModel ?? "未提供"],
    ["响应模型", r.responseModel ?? "未提供"],
    ["计价模型", r.cost.pricingModel ?? r.billingModel ?? "未定价"],
    ["服务等级", r.serviceTier ?? "未提供"],
    ["倍率", r.multiplier],
    ["总耗时", `${r.latencyMs} ms`],
    ["首 Token", r.firstTokenMs === null ? "未提供" : `${r.firstTokenMs} ms`],
    ["输出速度", speed(r)],
    ["完整性", r.incomplete ? "不完整" : "完整"],
    ["估算费用", money(r.cost.total)],
  ];
  return (
    <div className="attempt-content">
      <dl className="usage-detail">
        {entries.map(([name, value]) => (
          <div key={name}>
            <dt>{name}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
      <div className="usage-table-scroll">
        <table className="usage-table token-detail-table">
          <thead>
            <tr>
              <th>用量</th>
              <th>Token</th>
              <th>USD / 百万</th>
              <th>费用</th>
            </tr>
          </thead>
          <tbody>
            {(
              [
                ["input", "输入"],
                ["output", "输出"],
                ["cacheRead", "缓存读取"],
                ["cacheWrite", "缓存写入"],
              ] as const
            ).map(([k, n]) => (
              <tr key={k}>
                <td>{n}</td>
                <td>{number(r.tokens[k])}</td>
                <td>{r.cost.price?.rates[k] ?? "未定价"}</td>
                <td>{money(r.cost[k])}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <details className="price-detail">
        <summary>价格快照</summary>
        <dl className="usage-detail">
          <div>
            <dt>来源</dt>
            <dd>{r.cost.price?.source ?? "未定价"}</dd>
          </div>
          <div>
            <dt>版本</dt>
            <dd>{r.cost.version || "未提供"}</dd>
          </div>
        </dl>
        {r.cost.price && (
          <pre className="price-snapshot">
            {JSON.stringify(
              { rates: r.cost.price.rates, variants: r.cost.price.variants },
              null,
              2,
            )}
          </pre>
        )}
      </details>
    </div>
  );
}
