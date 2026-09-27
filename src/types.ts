export type Preferences = {
  codexHome: string;
  cliPath: string;
  theme: "system" | "dark" | "light";
};
export type Account = {
  id: string;
  name: string;
  kind: "chatgpt" | "apiKey";
  email: string | null;
  current: boolean;
  updatedAt: number;
};
export type ViewState = {
  accounts: Account[];
  authRevision: string;
  configRevision: string;
  currentState: "saved" | "unsaved" | "missing" | "invalid";
  preferences: Preferences;
  authSource: {
    provider: string;
    credentialStore: string;
    inlineToken: boolean;
    envKey: boolean;
    commandAuth: boolean;
    requiresOpenaiAuth: boolean;
    warning: string | null;
  };
  error: string | null;
};
export type ConfigDocument = { text: string; revision: string; path: string };
export type LoginState = {
  phase: string;
  mode: string;
  url: string | null;
  code: string | null;
  message: string;
};
export type AppError = {
  code: string;
  message: string;
  line?: number;
  column?: number;
};
export const errorOf = (e: unknown): AppError =>
  typeof e === "object" && e !== null && "message" in e
    ? (e as AppError)
    : {
        code: "UNKNOWN",
        message: typeof e === "string" ? e : "操作失败，请重新尝试",
      };

export type GatewaySettings = {
  port: number;
  maxRetries: number;
  failureThreshold: number;
  successThreshold: number;
  cooldownSeconds: number;
  errorRate: number;
  minRequests: number;
  firstByteSeconds: number;
  idleSeconds: number;
  totalSeconds: number;
  connectSeconds: number;
};
export type Health = {
  state: "closed" | "open" | "half_open";
  failures: number;
  requests: number;
  retryIn: number;
};
export type Provider = {
  id: string;
  name: string;
  baseUrl: string;
  proxyId: string | null;
  queued: boolean;
  health: Health;
  quotaVersion: string;
  quota: ProviderQuota | null;
};
export type ProxyProfile = {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  hasPassword: boolean;
  health: Health;
};
export type GatewayState = {
  revision: string;
  running: boolean;
  address: string;
  mode: "manual" | "auto";
  selected: string | null;
  lastSuccessful: string | null;
  configRevision: string | null;
  configProvider: string | null;
  configState: string;
  configError: string | null;
  providers: Provider[];
  proxies: ProxyProfile[];
  settings: GatewaySettings;
  activeConnections: number;
  recent: {
    provider: string;
    proxy: string | null;
    status: number | null;
    elapsedMs: number;
    retries: number;
    category: string;
    at: number;
  }[];
  error: string | null;
  recoveryPending: boolean;
};

export type QuotaPlan = {
  name: string;
  remaining: number | null;
  used: number | null;
  total: number | null;
  unit: string;
  unlimited: boolean;
  resetAt: string | null;
};
export type QuotaUsage = {
  requests: number | null;
  tokens: number | null;
  cost: number | null;
};
export type ProviderQuota = {
  providerId: string;
  version: string;
  state: "idle" | "loading" | "ok" | "unsupported" | "error";
  source: "sub2api" | "newapi" | null;
  checkedAt: number | null;
  successAt: number | null;
  retryAt: number | null;
  stale: boolean;
  error: string | null;
  keyStatus: string | null;
  plans: QuotaPlan[];
  expiresAt: string | null;
  expiresAtUnix: number | null;
  today: QuotaUsage | null;
  totalUsage: QuotaUsage | null;
};
