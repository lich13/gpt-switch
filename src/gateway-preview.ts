import type { GatewayState, GatewaySettings, ProviderQuota } from "./types";
const healthy = {
  state: "closed" as const,
  failures: 0,
  requests: 0,
  retryIn: 0,
};
export const gatewayDemo: GatewayState = {
  revision: "preview-gateway",
  running: false,
  address: "http://127.0.0.1:15722/v1",
  mode: "manual",
  selected: "primary",
  lastSuccessful: null,
  configRevision: "preview-config",
  configProvider: "primary",
  configState: "provider",
  configError: null,
  settings: {
    port: 15722,
    maxRetries: 3,
    failureThreshold: 4,
    successThreshold: 2,
    cooldownSeconds: 60,
    rateLimitSeconds: 5,
    errorRate: 0.6,
    minRequests: 10,
    firstByteSeconds: 60,
    idleSeconds: 120,
    totalSeconds: 600,
    connectSeconds: 15,
    queueSeconds: 30,
    maxWaiting: 100,
  },
  providers: [
    {
      id: "primary",
      name: "api.example.com",
      baseUrl: "https://api.example.com/v1",
      proxyId: null,
      queued: true,
      health: { ...healthy },
      quotaVersion: "preview-quota",
      quota: null,
      maxConcurrency: 4,
      activeRequests: 0,
      allowedModels: null,
    },
    {
      id: "backup",
      name: "备用工作空间 · 长名称供应商的列表与键盘操作验证",
      baseUrl: "https://backup.example.com/api/v1",
      proxyId: "cloud",
      queued: true,
      health: { ...healthy },
      quotaVersion: "preview-quota",
      quota: null,
      maxConcurrency: 0,
      activeRequests: 0,
      allowedModels: null,
    },
  ],
  proxies: [
    {
      id: "cloud",
      name: "腾讯云 SOCKS5",
      host: "203.0.113.1",
      port: 10808,
      username: "preview",
      hasPassword: true,
      health: { ...healthy },
    },
  ],
  activeConnections: 0,
  waitingRequests: 0,
  recent: [],
  error: null,
  recoveryPending: false,
};
export function gatewayPreview(name: string, args: Record<string, unknown>) {
  const s = gatewayDemo;
  if (name === "start_gateway") {
    s.running = true;
    s.configState = "gateway";
    s.configProvider = null;
  }
  if (name === "stop_gateway") {
    s.running = false;
    s.configState = "provider";
    s.configProvider = s.selected;
  }
  if (name === "test_provider") return 84;
  if (name === "query_provider_quota") {
    const provider = s.providers.find((p) => p.id === args.providerId)!;
    const result: ProviderQuota = {
      providerId: provider.id,
      version: provider.quotaVersion,
      state: "ok",
      source: provider.id === "primary" ? "sub2api" : "newapi",
      checkedAt: Math.floor(Date.now() / 1000),
      successAt: Math.floor(Date.now() / 1000),
      retryAt: null,
      stale: false,
      error: null,
      keyStatus: null,
      plans: [
        {
          name: "API Key 配额",
          remaining: 42.35,
          used: 7.65,
          total: 50,
          unit: "USD",
          unlimited: false,
          resetAt: null,
        },
      ],
      expiresAt: null,
      expiresAtUnix: null,
      today: { requests: 18, tokens: 56200, cost: 1.25 },
      totalUsage: null,
    };
    provider.quota = result;
    return result;
  }
  if (name === "update_gateway") {
    const e = args.edit as Record<string, unknown>,
      id = String(e.id),
      p = s.providers.find((p) => p.id === id);
    if (e.op === "mode") s.mode = e.mode as "auto" | "manual";
    if (e.op === "select") {
      s.selected = id;
      s.mode = "manual";
      if (!s.running) {
        s.configState = "provider";
        s.configProvider = id;
      }
    }
    if (e.op === "queueProvider" && p) p.queued = Boolean(e.queued);
    if (e.op === "concurrencyProvider" && p)
      p.maxConcurrency = Number(e.maxConcurrency);
    if (e.op === "routeProvider" && p) p.proxyId = e.proxyId as string | null;
    if (e.op === "renameProvider" && p) p.name = String(e.name);
    if (e.op === "deleteProvider")
      s.providers = s.providers.filter((p) => p.id !== id);
    if (e.op === "reorder")
      s.providers.sort(
        (a, b) =>
          (e.ids as string[]).indexOf(a.id) - (e.ids as string[]).indexOf(b.id),
      );
    if (e.op === "saveProvider") {
      if (p) p.baseUrl = String(e.baseUrl);
      else
        s.providers.push({
          id: crypto.randomUUID(),
          name: new URL(String(e.baseUrl)).hostname,
          baseUrl: String(e.baseUrl),
          proxyId: null,
          queued: true,
          health: { ...healthy },
          quotaVersion: crypto.randomUUID(),
          quota: null,
          maxConcurrency: 0,
          activeRequests: 0,
          allowedModels: null,
        });
    }
    if (e.op === "saveProxy") {
      const x = s.proxies.find((x) => x.id === id),
        values = {
          name: String(e.name),
          host: String(e.host),
          port: Number(e.port),
          username: String(e.username),
        };
      if (x) Object.assign(x, values);
      else
        s.proxies.push({
          id: crypto.randomUUID(),
          ...values,
          hasPassword: Boolean(e.password),
          health: { ...healthy },
        });
    }
    if (e.op === "deleteProxy") {
      if (s.providers.some((p) => p.proxyId === id))
        throw new Error("请先解绑或替换引用此代理的供应商");
      s.proxies = s.proxies.filter((x) => x.id !== id);
    }
    if (e.op === "modelsProvider" && p)
      p.allowedModels = e.allowedModels as string[] | null;
    if (e.op === "settings") s.settings = e.settings as GatewaySettings;
    if (e.op === "import") throw new Error("预览模式无法读取真实 Codex 配置");
    s.revision = crypto.randomUUID();
  }
  return structuredClone(s);
}
