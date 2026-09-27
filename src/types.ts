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
