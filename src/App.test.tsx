import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, it, expect, vi } from "vitest";
import type { ViewState, ConfigDocument } from "./types";
const mocks = vi.hoisted(() => ({
  command: vi.fn(),
  listeners: new Map<string, (p: unknown) => void>(),
}));
vi.mock("./bridge", () => ({
  preview: false,
  command: mocks.command,
  subscribe: vi.fn(async (event: string, fn: (p: unknown) => void) => {
    mocks.listeners.set(event, fn);
    return () => mocks.listeners.delete(event);
  }),
}));
vi.mock("@uiw/react-codemirror", () => ({
  default: ({
    value,
    onChange,
  }: {
    value: string;
    onChange: (s: string) => void;
  }) => (
    <textarea
      aria-label="TOML 编辑器"
      value={value}
      onChange={(e) => onChange(e.target.value)}
    />
  ),
}));
import App from "./App";
let state: ViewState;
let doc: ConfigDocument;
beforeEach(() => {
  mocks.listeners.clear();
  mocks.command.mockReset();
  state = {
    accounts: [
      {
        id: "a",
        name: "个人账号",
        email: "person@example.invalid",
        kind: "chatgpt",
        current: true,
        updatedAt: 1,
      },
      {
        id: "b",
        name: "工作账号",
        email: null,
        kind: "apiKey",
        current: false,
        updatedAt: 1,
      },
    ],
    authRevision: "auth-1",
    configRevision: "cfg-1",
    currentState: "saved",
    preferences: { codexHome: "/test/.codex", cliPath: "", theme: "dark" },
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
  doc = {
    text: '# keep\nmodel = "original"\n',
    revision: "cfg-1",
    path: "/test/.codex/config.toml",
  };
  mocks.command.mockImplementation(
    async (name: string, args: Record<string, unknown>) => {
      switch (name) {
        case "get_state":
          return structuredClone(state);
        case "get_login":
          return {
            phase: "idle",
            mode: "",
            url: null,
            code: null,
            message: "",
          };
        case "read_config":
          return { ...doc };
        case "save_config":
          if (String(args.text).includes("bad = ["))
            throw { code: "TOML", message: "TOML 语法错误", line: 2 };
          if (args.expectedRevision !== doc.revision)
            throw {
              code: "CONFLICT",
              message: "配置已被其他程序修改。草稿已保留",
            };
          doc = { ...doc, text: String(args.text), revision: "cfg-2" };
          return { ...doc };
        case "switch_account":
          return {
            ...state,
            accounts: state.accounts.map((a) => ({
              ...a,
              current: a.id === args.id,
            })),
          };
        case "start_login":
          return {
            phase: "waiting",
            mode: "browser",
            url: null,
            code: null,
            message: "请在浏览器完成登录",
          };
        default:
          return;
      }
    },
  );
});
describe("user workflows", () => {
  it("preserves Windows CRLF when the editor changes content", async () => {
    doc.text = '# keep\r\nmodel = "original"\r\n';
    const u = userEvent.setup();
    render(<App />);
    await screen.findByText("你的账号，一个入口。");
    await u.click(screen.getByRole("button", { name: "配置" }));
    fireEvent.change(await screen.findByLabelText("TOML 编辑器"), {
      target: { value: '# keep\nmodel = "updated"\n' },
    });
    await u.click(screen.getByRole("button", { name: /保存/ }));
    await waitFor(() =>
      expect(mocks.command).toHaveBeenCalledWith("save_config", {
        text: '# keep\r\nmodel = "updated"\r\n',
        expectedRevision: "cfg-1",
      }),
    );
  });
  it("switches through the native command without saving config", async () => {
    const u = userEvent.setup();
    render(<App />);
    await u.click(
      await screen.findByRole("button", { name: "切换到 工作账号" }),
    );
    expect(mocks.command).toHaveBeenCalledWith("switch_account", {
      id: "b",
      expectedRevision: "auth-1",
    });
    expect(await screen.findByRole("status")).toHaveTextContent("文件已切换");
    expect(mocks.command.mock.calls.some((c) => c[0] === "save_config")).toBe(
      false,
    );
  });
  it("preserves unsaved drafts when external state changes and blocks conflicting saves", async () => {
    const u = userEvent.setup();
    render(<App />);
    await screen.findByText("你的账号，一个入口。");
    await u.click(screen.getByRole("button", { name: "配置" }));
    const edit = await screen.findByLabelText("TOML 编辑器");
    fireEvent.change(edit, { target: { value: '# my draft\nmodel="draft"' } });
    doc.revision = "external";
    mocks.listeners.get("switch-state")?.({
      ...state,
      configRevision: "external",
    });
    expect(await screen.findByText(/磁盘配置已变化/)).toBeInTheDocument();
    await u.click(screen.getByRole("button", { name: /保存/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("草稿已保留");
    expect(edit).toHaveValue('# my draft\nmodel="draft"');
  });
  it("keeps invalid TOML in the editor after save failure", async () => {
    const u = userEvent.setup();
    render(<App />);
    await screen.findByText("你的账号，一个入口。");
    await u.click(screen.getByRole("button", { name: "配置" }));
    const edit = await screen.findByLabelText("TOML 编辑器");
    fireEvent.change(edit, { target: { value: "# keep\nbad = [" } });
    await u.click(screen.getByRole("button", { name: /保存/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("TOML 语法错误");
    expect(edit).toHaveValue("# keep\nbad = [");
  });
  it("cancels the active official login", async () => {
    const u = userEvent.setup();
    render(<App />);
    await u.click(await screen.findByRole("button", { name: "添加账号" }));
    await u.click(screen.getByRole("button", { name: "使用浏览器登录" }));
    await u.click(await screen.findByRole("button", { name: "取消登录" }));
    expect(mocks.command).toHaveBeenCalledWith("cancel_login");
  });
  it("receives tray changes and refreshes the selected account", async () => {
    render(<App />);
    await screen.findByText("你的账号，一个入口。");
    mocks.listeners.get("switch-state")?.({
      ...state,
      accounts: state.accounts.map((a) => ({ ...a, current: a.id === "b" })),
    });
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "切换到 个人账号" }),
      ).toBeInTheDocument(),
    );
    expect(
      screen.queryByRole("button", { name: "切换到 工作账号" }),
    ).not.toBeInTheDocument();
  });
  it("asks before abandoning a config draft", async () => {
    const u = userEvent.setup();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<App />);
    await screen.findByText("你的账号，一个入口。");
    await u.click(screen.getByRole("button", { name: "配置" }));
    fireEvent.change(await screen.findByLabelText("TOML 编辑器"), {
      target: { value: 'model="draft"' },
    });
    await u.click(screen.getByRole("button", { name: /账号\s*2/ }));
    expect(confirm).toHaveBeenCalled();
    expect(screen.getByLabelText("TOML 编辑器")).toHaveValue('model="draft"');
  });
});
