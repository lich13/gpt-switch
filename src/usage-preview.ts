// Development-only fixtures. Never included in the installed application.
import type {
  Dashboard,
  UsageRecord,
  UsageState,
  PricingView,
  RemoteModel,
  Filters,
  Aggregate,
  Price,
  LogicalRecord,
} from "./usage-types";
import { emptyRates } from "./usage-types";
const tokens = {
  input: 1200,
  output: 430,
  cacheRead: 8400,
  cacheWrite: 0,
  cacheWriteHour: null,
  inputImages: null,
  outputImages: null,
  images: null,
  context: 9600,
};
const now = Math.floor(Date.now() / 1000);
const price: Price = {
  modelId: "gpt-example",
  displayName: "gpt-example",
  source: "LiteLLM",
  fixed: false,
  rates: {
    ...emptyRates,
    input: "2.5",
    output: "10",
    cacheRead: "0.25",
    cacheWrite: "0",
  },
  variants: [],
};
let pricing: PricingView = {
  revision: "pricing-preview",
  settings: {
    autoUpdate: true,
    modelsDevEnabled: false,
    includeCommon: true,
    selected: [],
    excluded: [],
  },
  sync: {
    checkedAt: now,
    updatedAt: now,
    error: null,
    etag: null,
    hash: "preview",
  },
  modelsDevSync: {
    checkedAt: null,
    updatedAt: null,
    error: null,
    etag: null,
    hash: null,
  },
  prices: [
    price,
    {
      ...price,
      modelId: "gpt-example-mini",
      rates: { ...price.rates, input: "0.25", output: "2" },
    },
    {
      ...price,
      modelId: "community/free",
      source: "manual",
      fixed: true,
      rates: { ...emptyRates, input: "0", output: "0" },
    },
  ],
  configPath:
    "~/Library/Application Support/com.lich13.gpt-switch/model-pricing.json",
};
let state: UsageState = {
  revision: "usage-preview",
  settings: {
    enabled: true,
    costMultiplier: "1",
    providerMultipliers: {},
    modelSource: "response",
    refreshSeconds: 30,
    retentionDays: 30,
  },
  error: null,
};
let power = {
  supported: true,
  enabled: false,
  batterySleep: 0,
  revision: "power-preview",
};
const attempts: UsageRecord[] = Array.from({ length: 76 }, (_, i) => ({
  id: `request-${i}`,
  logicalId: `logical-${Math.floor(i / 2)}`,
  attempt: i % 2,
  providerId: i % 3 ? "p1" : "p2",
  providerName:
    i % 3
      ? "Example Cloud"
      : "Research gateway with a deliberately long provider name",
  requestModel: "gpt-example",
  responseModel: i % 7 ? "gpt-example" : null,
  billingModel: "gpt-example",
  serviceTier: "default",
  modelSource: "response",
  multiplier: "1",
  createdAt: now - i * 420,
  status: i % 7 ? 200 : 502,
  outcome: i % 7 ? "OK" : "HTTP",
  streaming: !!(i % 2),
  latencyMs: 2320 + i * 22,
  firstTokenMs: 340,
  durationMs: 1980 + i * 22,
  tokens:
    i % 7
      ? tokens
      : {
          ...tokens,
          input: null,
          output: null,
          cacheRead: null,
          cacheWrite: null,
        },
  incomplete: !(i % 7),
  routing: [],
  terminalEvidence: i % 7 ? "OK" : null,
  cost: {
    status: i % 7 ? "priced" : "unreported",
    total: i % 7 ? "0.0094" : null,
    input: "0.003",
    output: "0.0043",
    cacheRead: "0.0021",
    cacheWrite: "0",
    image: null,
    pricingModel: price.modelId,
    price,
    version: "preview-price-version",
  },
}));
const records: LogicalRecord[] = Array.from({ length: 38 }, (_, i) => {
  const a = attempts[i * 2],
    b = attempts[i * 2 + 1];
  const outcomeClass =
    i % 13 === 0
      ? "cancelled"
      : i % 11 === 0
        ? "failure"
        : i % 9 === 0
          ? "rejected"
          : "success";
  const routing = [
    {
      providerId: "p1",
      providerName: "Example Cloud",
      reason: "rate_limit",
      active: 2,
      limit: 4,
      retryIn: 5,
      at: a.createdAt,
    },
    {
      providerId: "p2",
      providerName: b.providerName,
      reason: "selected",
      active: 1,
      limit: 0,
      retryIn: 0,
      at: b.createdAt,
    },
  ];
  return {
    ...b,
    id: `logical-${i}`,
    logicalId: `logical-${i}`,
    outcomeClass,
    outcome:
      outcomeClass === "success"
        ? "OK"
        : outcomeClass === "cancelled"
          ? "CANCELLED"
          : outcomeClass === "rejected"
            ? "MODEL_NOT_ALLOWED"
            : "HTTP",
    status:
      outcomeClass === "success"
        ? 200
        : outcomeClass === "failure"
          ? 503
          : outcomeClass === "rejected"
            ? 400
            : 200,
    attemptCount: 2,
    semanticsVersion: 2,
    routing,
  };
});
const aggregate = (rows: LogicalRecord[]): Aggregate => ({
  requests: rows.length,
  successes: rows.filter((r) => r.outcomeClass === "success").length,
  failures: rows.filter((r) => r.outcomeClass === "failure").length,
  rejected: rows.filter((r) => r.outcomeClass === "rejected").length,
  cancelled: rows.filter((r) => r.outcomeClass === "cancelled").length,
  pending: 0,
  unknown: 0,
  attempts: rows.length * 2,
  legacyAttempts: 0,
  tokens: {
    ...tokens,
    input: rows.reduce((n, r) => n + (r.tokens.input ?? 0), 0),
    output: rows.reduce((n, r) => n + (r.tokens.output ?? 0), 0),
    cacheRead: rows.reduce((n, r) => n + (r.tokens.cacheRead ?? 0), 0),
  },
  cost: String(rows.reduce((n, r) => n + Number(r.cost.total ?? 0), 0)),
  unpriced: 0,
  partial: 0,
  missingUsage: rows.filter((r) => r.tokens.input === null).length,
  latencyMs: rows.reduce((n, r) => n + r.latencyMs, 0),
  gatewayErrors: 0,
});
const filtered = (f: Filters) =>
  records.filter(
    (r) =>
      (f.start === undefined || r.createdAt >= f.start) &&
      (f.end === undefined || r.createdAt < f.end) &&
      (!f.providerId || r.providerId === f.providerId) &&
      (!f.model || r.billingModel === f.model) &&
      (!f.status || r.status === f.status) &&
      (!f.outcome || r.outcomeClass === f.outcome),
  );
export const usageCommands = [
  "get_usage_state",
  "set_usage_settings",
  "get_usage_dashboard",
  "get_usage_logs",
  "get_usage_detail",
  "get_pricing",
  "update_pricing",
  "sync_pricing",
  "list_models_dev",
  "import_models_dev",
  "reload_pricing",
  "open_pricing_folder",
  "get_clamshell_state",
  "set_clamshell_awake",
  "force_quit_codex_clients",
  "get_startup_error",
];
export async function usagePreview(
  name: string,
  args: Record<string, unknown>,
  emit: (name: string, payload: unknown) => void,
): Promise<unknown> {
  const f = (args.filters ?? {}) as Filters,
    rows = filtered(f);
  switch (name) {
    case "get_startup_error":
      return null;
    case "get_clamshell_state":
      return { ...power };
    case "set_clamshell_awake":
      power = {
        ...power,
        enabled: !!args.enabled,
        revision: crypto.randomUUID(),
      };
      emit("clamshell-state", power);
      return power;
    case "force_quit_codex_clients":
      return { terminated: 0, failed: 0 };
    case "get_usage_state":
      return structuredClone(state);
    case "set_usage_settings":
      state = {
        ...state,
        settings: args.settings as UsageState["settings"],
        revision: crypto.randomUUID(),
      };
      return state;
    case "get_usage_dashboard": {
      const times = [
        ...new Set(rows.map((r) => Math.floor(r.createdAt / 3600) * 3600)),
      ].sort();
      return {
        summary: aggregate(rows),
        trends: times.map((at) => ({
          at,
          ...aggregate(
            rows.filter((r) => Math.floor(r.createdAt / 3600) * 3600 === at),
          ),
        })),
        providers: [...new Set(rows.map((r) => r.providerId))].map((id) => ({
          id,
          name: rows.find((r) => r.providerId === id)!.providerName,
          ...aggregate(rows.filter((r) => r.providerId === id)),
        })),
        models: rows.length
          ? [{ id: "gpt-example", name: "gpt-example", ...aggregate(rows) }]
          : [],
        effectiveStart: f.start ?? null,
        effectiveEnd: f.end ?? null,
        precision: "hour",
        semanticsVersion: 2,
        availableModels: ["gpt-example"],
        availableProviders: [
          ["p1", "Example Cloud"],
          ["p2", "Research gateway with a deliberately long provider name"],
        ],
      } satisfies Dashboard;
    }
    case "get_usage_logs":
      return {
        records: rows.slice(
          Number(args.page) * 20,
          (Number(args.page) + 1) * 20,
        ),
        total: rows.length,
        page: args.page,
        pageSize: 20,
      };
    case "get_usage_detail": {
      const r = records.find((r) => r.id === args.id);
      return r
        ? {
            ...r,
            attempts: attempts.filter((a) => a.logicalId === r.logicalId),
          }
        : null;
    }
    case "sync_pricing":
      pricing.sync.checkedAt = Math.floor(Date.now() / 1000);
      return structuredClone(pricing);
    case "get_pricing":
    case "reload_pricing":
      return structuredClone(pricing);
    case "update_pricing": {
      const edit = args.edit as {
        op: string;
        settings: PricingView["settings"];
        price: Price;
        prices: Price[];
        id: string;
      };
      if (edit.op === "settings") pricing.settings = edit.settings;
      else if (edit.op === "delete")
        pricing.prices = pricing.prices.filter((p) => p.modelId !== edit.id);
      else if (edit.op === "automatic")
        pricing.prices = pricing.prices.map((p) =>
          p.modelId === edit.id ? { ...p, fixed: false, source: "LiteLLM" } : p,
        );
      else
        for (const p of edit.op === "import" ? edit.prices : [edit.price])
          pricing.prices = [
            ...pricing.prices.filter((v) => v.modelId !== p.modelId),
            { ...p, fixed: true, source: "manual" },
          ];
      pricing.revision = crypto.randomUUID();
      emit("pricing-state", null);
      return structuredClone(pricing);
    }
    case "list_models_dev":
      return pricing.prices.map(
        (p) =>
          ({
            key: `openai::${p.modelId}`,
            provider: "openai",
            modelId: p.modelId,
            name: p.modelId,
            released: "2026-09-29",
            common: true,
            price: p,
          }) satisfies RemoteModel,
      );
    case "import_models_dev":
      return pricing;
    default:
      return;
  }
}
