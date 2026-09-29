export type Tokens = {
  input: number | null;
  output: number | null;
  cacheRead: number | null;
  cacheWrite: number | null;
  cacheWriteHour: number | null;
  inputImages: number | null;
  outputImages: number | null;
  images: number | null;
  context: number | null;
};
export type Rates = {
  input: string | null;
  output: string | null;
  cacheRead: string | null;
  cacheWrite: string | null;
  cacheHour: string | null;
  imageInput: string | null;
  imageOutput: string | null;
  image: string | null;
};
export type Price = {
  modelId: string;
  displayName: string;
  source: string;
  fixed: boolean;
  rates: Rates;
  variants: { above: number; tier: string; rates: Rates }[];
};
export type Cost = {
  status: string;
  total: string | null;
  input: string | null;
  output: string | null;
  cacheRead: string | null;
  cacheWrite: string | null;
  image: string | null;
  pricingModel: string | null;
  price: Price | null;
  version: string;
};
export type RoutingDecision = {
  providerId: string;
  providerName: string;
  reason: string;
  active: number;
  limit: number;
  retryIn: number;
  at: number;
};
export type UsageRecord = {
  id: string;
  logicalId: string;
  attempt: number;
  providerId: string;
  providerName: string;
  requestModel: string | null;
  responseModel: string | null;
  billingModel: string | null;
  serviceTier: string | null;
  modelSource: string;
  multiplier: string;
  createdAt: number;
  status: number | null;
  outcome: string;
  streaming: boolean;
  latencyMs: number;
  firstTokenMs: number | null;
  durationMs: number | null;
  tokens: Tokens;
  incomplete: boolean;
  cost: Cost;
  routing: RoutingDecision[];
  terminalEvidence: string | null;
};
export type LogicalRecord = UsageRecord & {
  outcomeClass: string;
  attemptCount: number;
  semanticsVersion: number;
};
export type LogicalDetail = LogicalRecord & { attempts: UsageRecord[] };
export type Aggregate = {
  requests: number;
  successes: number;
  failures: number;
  rejected: number;
  cancelled: number;
  unknown: number;
  pending: number;
  attempts: number;
  legacyAttempts: number;
  tokens: Tokens;
  cost: string | null;
  unpriced: number;
  partial: number;
  missingUsage: number;
  latencyMs: number;
  gatewayErrors: number;
};
export type Group = Aggregate & { id: string; name: string };
export type Dashboard = {
  summary: Aggregate;
  trends: (Aggregate & { at: number })[];
  providers: Group[];
  models: Group[];
  effectiveStart: number | null;
  effectiveEnd: number | null;
  precision: string;
  availableProviders: [string, string][];
  availableModels: string[];
  semanticsVersion: number;
};
export type Filters = {
  start?: number;
  end?: number;
  providerId?: string;
  model?: string;
  status?: number;
  outcome?: string;
};
export type UsageSettings = {
  enabled: boolean;
  costMultiplier: string;
  providerMultipliers: Record<string, string>;
  modelSource: "request" | "response";
  refreshSeconds: number;
  retentionDays: number;
};
export type UsageState = {
  settings: UsageSettings;
  revision: string;
  error: string | null;
};
export type LogPage = {
  records: LogicalRecord[];
  total: number;
  page: number;
  pageSize: number;
};
export type PricingSettings = {
  autoUpdate: boolean;
  modelsDevEnabled: boolean;
  includeCommon: boolean;
  selected: string[];
  excluded: string[];
};
export type SyncInfo = {
  checkedAt: number | null;
  updatedAt: number | null;
  error: string | null;
  etag: string | null;
  hash: string | null;
};
export type PricingView = {
  revision: string;
  settings: PricingSettings;
  sync: SyncInfo;
  modelsDevSync: SyncInfo;
  prices: Price[];
  configPath: string;
};
export type RemoteModel = {
  key: string;
  provider: string;
  modelId: string;
  name: string;
  released: string;
  common: boolean;
  price: Price;
};
export const emptyRates: Rates = {
  input: null,
  output: null,
  cacheRead: null,
  cacheWrite: null,
  cacheHour: null,
  imageInput: null,
  imageOutput: null,
  image: null,
};
export const number = (n: number | null | undefined) =>
  n == null
    ? "未提供"
    : n.toLocaleString("zh-CN", { maximumFractionDigits: 2 });
export const money = (n: string | null | undefined) =>
  n == null
    ? "未定价"
    : `$${Number(n).toLocaleString("en-US", { maximumFractionDigits: 8 })}`;
export const date = (n: number | null | undefined) =>
  n == null
    ? "—"
    : new Date(n * 1000).toLocaleString("zh-CN", { hour12: false });
export const totalTokens = (t: Tokens) =>
  [t.input, t.output, t.cacheRead, t.cacheWrite].some((x) => x !== null)
    ? (t.input ?? 0) +
      (t.output ?? 0) +
      (t.cacheRead ?? 0) +
      (t.cacheWrite ?? 0)
    : null;
export const speed = (r: UsageRecord) =>
  r.tokens.output !== null && r.durationMs && r.durationMs > 0
    ? `${((r.tokens.output / r.durationMs) * 1000).toFixed(1)} t/s`
    : "未提供";

export const compact = (n: number | null | undefined) =>
  n == null
    ? "未提供"
    : Intl.NumberFormat("en-US", {
        notation: "compact",
        maximumFractionDigits: 1,
      }).format(n);
export const compactMoney = (n: string | null | undefined) =>
  n == null
    ? "未定价"
    : Number(n) === 0
      ? "$0.00"
      : Number(n) < 0.01
        ? "< $0.01"
        : `$${Intl.NumberFormat("en-US", { maximumFractionDigits: 2 }).format(Number(n))}`;
export const successRate = (r: Pick<Aggregate, "successes" | "failures">) =>
  r.successes + r.failures
    ? `${((r.successes / (r.successes + r.failures)) * 100).toFixed(1)}%`
    : "—";
export const outcomeLabel = (key: string) =>
  ({
    success: "成功",
    failure: "服务失败",
    rejected: "业务拒绝",
    cancelled: "已取消",
    pending: "进行中",
    unknown: "未确认",
  })[key] ?? key;
export const routingLabel = (key: string) =>
  ({
    model: "模型不匹配",
    capacity: "并发已满",
    fifo: "等待在先请求",
    selected: "已选用",
    state_changed: "状态变化，重新检查",
    circuit_open: "故障熔断",
    rate_limit: "上游限流",
    retry_after: "上游要求等待",
    half_open_probe: "恢复探测中",
    proxy_cooldown: "代理冷却",
  })[key] ?? key;
