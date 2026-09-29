import { powerCommands, powerPreview } from "./power-preview";
import { gatewayPreview } from "./gateway-preview";
import type { ViewState, ConfigDocument, LoginState } from "./types";
const callbacks = new Map<string, Set<(p: never) => void>>();
export function subscribe<T>(event: string, fn: (p: T) => void) {
  if (!callbacks.has(event)) callbacks.set(event, new Set());
  callbacks.get(event)!.add(fn as (p: never) => void);
  return () => {
    callbacks.get(event)?.delete(fn as (p: never) => void);
  };
}
function emit(event: string, data: unknown) {
  callbacks.get(event)?.forEach((fn) => fn(data as never));
}
export const demo: ViewState = {
  accounts: [
    {
      id: "personal",
      name: "日常开发",
      kind: "chatgpt",
      email: "personal@example.invalid",
      current: true,
      updatedAt: 0,
    },
    {
      id: "work",
      name: "工作空间",
      kind: "chatgpt",
      email: "work@example.invalid",
      current: false,
      updatedAt: 0,
    },
    {
      id: "api",
      name: "API 开发账号",
      kind: "apiKey",
      email: null,
      current: false,
      updatedAt: 0,
    },
  ],
  authRevision: "preview",
  configRevision: "preview",
  currentState: "saved",
  preferences: { codexHome: "~/.codex", cliPath: "", theme: "system" },
  authSource: {
    provider: "openai",
    credentialStore: "file",
    inlineToken: false,
    envKey: false,
    commandAuth: false,
    requiresOpenaiAuth: true,
    warning: null,
  },
  error: null,
};
let doc: ConfigDocument = {
  text: '# Codex 全局配置\nmodel = "gpt-example"\nmodel_reasoning_effort = "high"\n\n# 保留你的注释与其他设置\n[features]\nweb_search_request = true\n',
  revision: "preview",
  path: "~/.codex/config.toml",
};
let login: LoginState = {
  phase: "idle",
  mode: "",
  url: null,
  code: null,
  message: "",
};
let startup = {
  launchOnBoot: false,
  restoreGateway: false,
  revision: "preview-startup",
};
let quick = { pinned: false, tab: "providers", visible: true };
let linkHandler = "com.lich13.studio";
const linkApps = [
  {
    id: "com.lich13.gpt-switch",
    name: "gpt-Switch",
    path: "/Applications/gpt-Switch.app",
  },
  {
    id: "com.lich13.studio",
    name: "lich13studio",
    path: "/Applications/lich13studio.app",
  },
];
export async function run(
  name: string,
  args: Record<string, unknown>,
): Promise<unknown> {
  if (name === "get_link_handler_state")
    return { current: linkHandler, apps: linkApps, systemPicker: false };
  if (name === "set_link_handler") {
    linkHandler = String(args.appId);
    return { current: linkHandler, apps: linkApps, systemPicker: false };
  }
  if (name === "get_provider_imports") return [];
  if (name === "cleanup_retired_data") return;
  if (powerCommands.includes(name)) return powerPreview(name, args, emit);
  if (name === "get_quick") return { ...quick };
  if (name === "set_quick") {
    quick = { ...quick, ...args };
    emit("quick-state", { ...quick });
    return { ...quick };
  }
  if (name === "resize_quick" || name === "hide_quick") return;
  if (name === "open_main") {
    location.search = "";
    return;
  }
  if (name === "list_provider_models")
    return {
      providerId: args.providerId,
      version: "preview-quota",
      models: ["gpt-example", "gpt-example-mini", "gpt-example-pro"],
      checkedAt: Date.now() / 1000,
      stale: false,
      error: null,
      retryAt: null,
    };
  if (name === "get_startup") return { ...startup };
  if (name === "set_startup") {
    startup = {
      ...startup,
      ...(args.preferences as {
        restoreGateway: boolean;
      }),
      launchOnBoot: Boolean(args.enabled),
      revision: crypto.randomUUID(),
    };
    return { ...startup };
  }
  if (
    [
      "get_gateway",
      "start_gateway",
      "stop_gateway",
      "update_gateway",
      "test_provider",
      "query_provider_quota",
    ].includes(name)
  ) {
    const result = gatewayPreview(name, args);
    if (name === "query_provider_quota") emit("provider-quota", result);
    else if (name !== "test_provider") emit("gateway-state", result);
    return result;
  }
  switch (name) {
    case "get_state":
      return structuredClone(demo);
    case "frontend_ready":
      return;
    case "switch_account":
      demo.accounts.forEach((a) => (a.current = a.id === args.id));
      emit("switch-state", structuredClone(demo));
      return structuredClone(demo);
    case "rename_account":
      demo.accounts.find((a) => a.id === args.id)!.name = String(args.name);
      break;
    case "delete_account":
      demo.accounts = demo.accounts.filter((a) => a.id !== args.id);
      break;
    case "add_api_key":
      demo.accounts.push({
        id: crypto.randomUUID(),
        name: String(args.name),
        kind: "apiKey",
        email: null,
        current: false,
        updatedAt: 0,
      });
      break;
    case "read_config":
      return { ...doc };
    case "validate_config":
      if (String(args.text).includes("INVALID"))
        throw { code: "TOML", message: "TOML 语法错误", line: 1, column: 1 };
      return;
    case "save_config":
      await run("validate_config", args);
      doc = { ...doc, text: String(args.text), revision: crypto.randomUUID() };
      demo.configRevision = doc.revision;
      emit("switch-state", structuredClone(demo));
      return { ...doc };
    case "set_preferences":
      demo.preferences = args.preferences as ViewState["preferences"];
      break;
    case "get_login":
      return login;
    case "start_login":
      login = {
        phase: "waiting",
        mode: String(args.mode),
        url: "https://auth.openai.com/codex/device",
        code: args.mode === "device" ? "DEMO-CODE" : null,
        message: "预览模式：在桌面客户端中完成真实登录",
      };
      emit("login-state", login);
      return login;
    case "cancel_login":
      login = { ...login, phase: "cancelled", message: "登录已取消" };
      emit("login-state", login);
      return;
    case "pick_path":
    case "import_auth_file":
      return null;
    case "import_current":
      break;
    default:
      return;
  }
  emit("switch-state", structuredClone(demo));
  return structuredClone(demo);
}
