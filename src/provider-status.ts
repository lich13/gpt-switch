import type { Provider } from "./types";
export function providerStatus(p: Provider): string {
  const h = p.health;
  if (h.probeInFlight) return "恢复探测中";
  if (h.cooldownReason === "capacity_retry") return `容量等待 ${h.retryIn}s`;
  if (h.cooldownReason === "single_provider_protected")
    return `单供应商冷却 ${h.retryIn}s`;
  if (h.cooldownReason === "rate_limit") return `限流 ${h.retryIn}s`;
  if (h.cooldownReason === "retry_after") return `冷却 ${h.retryIn}s`;
  if (h.state === "open") return `熔断 ${h.retryIn}s`;
  if (h.state === "half_open") return "恢复探测";
  if (p.rpmLedgerError) return "RPM 状态不可用";
  if (p.rpmLimited) return `RPM 等待 ${Math.max(1, p.rpmRetryIn)}s`;
  if (p.maxConcurrency > 0 && p.activeRequests >= p.maxConcurrency)
    return `满载 ${p.activeRequests}/${p.maxConcurrency}`;
  return "";
}
