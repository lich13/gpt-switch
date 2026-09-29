import { render, screen, waitFor, act, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, it, expect, vi } from "vitest";
import type { ClientId, GatewayState } from "./types";
const mock = vi.hoisted(() => ({
  command: vi.fn(),
  listeners: new Map<string, (s: GatewayState) => void>(),
}));
vi.mock("./bridge", () => ({
  command: mock.command,
  subscribe: vi.fn(async (name: string, fn: (s: GatewayState) => void) => {
    mock.listeners.set(name, fn);
    return () => mock.listeners.delete(name);
  }),
}));
import Gateway from "./Gateway";
import { claudeGatewayDemo, gatewayDemo } from "./gateway-preview";
let state: GatewayState;
let claudeState: GatewayState;
beforeEach(() => {
  localStorage.clear();
  state = structuredClone(gatewayDemo);
  claudeState = structuredClone(claudeGatewayDemo);
  mock.command.mockReset();
  mock.listeners.clear();
  mock.command.mockImplementation(
    async (name: string, args?: { clientId?: ClientId }) => {
      if (name === "get_gateway" || name === "update_gateway")
        return structuredClone(
          args?.clientId === "claude" ? claudeState : state,
        );
      if (name === "test_provider") return 51;
    },
  );
});
afterEach(() => localStorage.clear());
describe("gateway controls", () => {
  it("keeps the provider API form to two fields and sends no guessed model settings", async () => {
    const user = userEvent.setup();
    render(<Gateway notify={() => {}} />);
    await user.click(await screen.findByRole("button", { name: "添加" }));
    await user.type(
      screen.getByLabelText("base_url"),
      "https://new.example.com/sub/v1",
    );
    const token = screen.getByLabelText("experimental_bearer_token");
    expect(token).toHaveAttribute("type", "password");
    await user.type(token, "fixture-token");
    act(() =>
      mock.listeners.get("gateway-state")!({
        ...state,
        revision: "changed-while-editing",
      }),
    );
    await user.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() =>
      expect(mock.command).toHaveBeenCalledWith("update_gateway", {
        clientId: "codex",
        edit: {
          op: "saveProvider",
          id: null,
          baseUrl: "https://new.example.com/sub/v1",
          token: "fixture-token",
        },
        expectedRevision: state.revision,
      }),
    );
  });
  it("keeps a failed provider draft open with its error", async () => {
    const user = userEvent.setup();
    render(<Gateway notify={() => {}} />);
    await user.click(await screen.findByRole("button", { name: "添加" }));
    await user.type(
      screen.getByLabelText("base_url"),
      "http://127.0.0.1:15722/v1",
    );
    await user.type(
      screen.getByLabelText("experimental_bearer_token"),
      "fixture",
    );
    mock.command.mockRejectedValueOnce({
      code: "LOOP",
      message: "上游不能指向本网关",
    });
    await user.click(screen.getByRole("button", { name: "保存" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "上游不能指向本网关",
    );
    expect(screen.getByLabelText("base_url")).toHaveValue(
      "http://127.0.0.1:15722/v1",
    );
  });
  it("switches a stopped provider with the visible config revision and restart feedback", async () => {
    const notify = vi.fn();
    const user = userEvent.setup();
    render(<Gateway notify={notify} />);
    await user.click(await screen.findByRole("button", { name: "选择" }));
    expect(mock.command).toHaveBeenCalledWith("update_gateway", {
      clientId: "codex",
      edit: { op: "select", id: "backup" },
      expectedRevision: state.revision,
      expectedConfigRevision: state.configRevision,
    });
    expect(notify).toHaveBeenCalledWith("文件已切换，请重新打开 Codex");
  });
});

it("keeps model choices and manual entries when discovery fails and events refresh", async () => {
  const user = userEvent.setup();
  let discoveryFailed = false;
  mock.command.mockImplementation(async (name: string) => {
    if (name === "get_gateway") return structuredClone(state);
    if (name === "list_provider_models") {
      if (discoveryFailed)
        throw { code: "NETWORK", message: "模型列表读取失败" };
      return { models: ["gpt-A", "gpt-B"], error: null, stale: false };
    }
    if (name === "update_gateway")
      throw { code: "CONFLICT", message: "设置已变化" };
  });
  render(<Gateway notify={() => {}} />);
  const operations = await screen.findByLabelText("api.example.com 操作");
  await user.click(operations);
  await user.click(
    within(operations.closest("details")!).getByRole("button", {
      name: "供应商设置",
    }),
  );
  await user.click(screen.getByRole("button", { name: /^模型白名单/ }));
  await user.click(await screen.findByRole("checkbox", { name: "gpt-A" }));
  await user.type(
    screen.getByRole("textbox", { name: "手动添加模型 ID" }),
    "custom-model",
  );
  await user.click(screen.getByRole("button", { name: "添加到白名单" }));
  act(() =>
    mock.listeners.get("gateway-state")!({
      ...state,
      revision: "changed",
      activeConnections: 2,
    }),
  );
  expect(screen.getByRole("checkbox", { name: "gpt-A" })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "custom-model" })).toBeChecked();
  discoveryFailed = true;
  await user.click(screen.getByRole("button", { name: "刷新模型列表" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "模型列表读取失败",
  );
  expect(screen.getByRole("checkbox", { name: "gpt-A" })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "custom-model" })).toBeChecked();
  await user.click(screen.getByRole("button", { name: "保存" }));
  expect(await screen.findByText("设置已变化")).toBeInTheDocument();
  expect(screen.getByRole("checkbox", { name: "custom-model" })).toBeChecked();
  expect(mock.command).toHaveBeenCalledWith("update_gateway", {
    clientId: "codex",
    edit: {
      op: "modelsProvider",
      id: "primary",
      allowedModels: ["gpt-A", "custom-model"],
    },
    expectedRevision: state.revision,
  });
});

it("refreshes authoritative configuration after a cap conflict, preserves input and permits explicit retry", async () => {
  const user = userEvent.setup();
  let attempts = 0;
  mock.command.mockImplementation(
    async (
      name: string,
      args?: { edit: { maxConcurrency: number }; expectedRevision: string },
    ) => {
      if (name === "get_gateway") return structuredClone(state);
      if (name === "update_gateway") {
        if (++attempts === 1) {
          state.revision = "other-window";
          state.providers[0].queued = false;
          throw { code: "CONFLICT", message: "网关设置已变化" };
        }
        expect(args!.expectedRevision).toBe("other-window");
        state.providers[0].maxConcurrency = args!.edit.maxConcurrency;
        state.revision = "saved";
        return structuredClone(state);
      }
    },
  );
  render(<Gateway notify={() => {}} />);
  await user.click(
    await screen.findByRole("button", { name: "api.example.com 并发上限" }),
  );
  const input = screen.getByRole("spinbutton", { name: "上限（0 不限）" });
  await user.clear(input);
  await user.type(input, "12");
  await user.click(screen.getByRole("button", { name: "保存" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("网关设置已变化");
  expect(input).toHaveValue(12);
  expect(
    screen.getByRole("button", { name: "api.example.com 加入队列" }),
  ).toHaveAttribute("aria-pressed", "false");
  await user.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
  expect(
    screen.getByRole("button", { name: "api.example.com 并发上限" }),
  ).toHaveTextContent("并发 0/12");
  expect(
    mock.command.mock.calls
      .filter(([name]) => name === "update_gateway")
      .map(([, args]) => args.edit.op),
  ).toEqual(["concurrencyProvider", "concurrencyProvider"]);
  expect(state.providers[0].queued).toBe(false);
});

describe("client isolation", () => {
  it("queries each selected client and ignores the other client's gateway events", async () => {
    const user = userEvent.setup();
    render(<Gateway notify={() => {}} />);
    await screen.findByLabelText(`${state.providers[0].name} 操作`);
    expect(mock.command).toHaveBeenCalledWith("get_gateway", {
      clientId: "codex",
    });
    await waitFor(() =>
      expect(mock.command).toHaveBeenCalledWith("query_provider_quota", {
        clientId: "codex",
        providerId: state.providers[0].id,
        force: false,
      }),
    );
    act(() =>
      mock.listeners.get("gateway-state")!({
        ...claudeState,
        revision: "background-claude",
      }),
    );
    expect(
      screen.getByLabelText(`${state.providers[0].name} 操作`),
    ).toBeInTheDocument();
    expect(
      screen.queryByLabelText(`${claudeState.providers[0].name} 操作`),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Claude Code" }));
    await screen.findByLabelText(`${claudeState.providers[0].name} 操作`);
    expect(
      screen.queryByLabelText(`${state.providers[0].name} 操作`),
    ).not.toBeInTheDocument();
    expect(mock.command).toHaveBeenCalledWith("get_gateway", {
      clientId: "claude",
    });
    await waitFor(() =>
      expect(mock.command).toHaveBeenCalledWith("query_provider_quota", {
        clientId: "claude",
        providerId: claudeState.providers[0].id,
        force: false,
      }),
    );
    expect(localStorage.getItem("lich13-switch.main.client")).toBe("claude");
    act(() =>
      mock.listeners.get("gateway-state")!({
        ...state,
        revision: "background-codex",
        providers: state.providers.map((p) => ({
          ...p,
          name: `后台 ${p.name}`,
        })),
      }),
    );
    expect(
      screen.getByLabelText(`${claudeState.providers[0].name} 操作`),
    ).toBeInTheDocument();
    expect(
      screen.queryByLabelText(`后台 ${state.providers[0].name} 操作`),
    ).not.toBeInTheDocument();
    act(() =>
      mock.listeners.get("gateway-state")!({
        ...claudeState,
        revision: "foreground-claude",
        providers: claudeState.providers.map((p, index) => ({
          ...p,
          name: index ? p.name : "当前 Claude 供应商",
        })),
      }),
    );
    expect(
      screen.getByLabelText("当前 Claude 供应商 操作"),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Codex" }));
    await screen.findByLabelText(`${state.providers[0].name} 操作`);
    expect(
      mock.command.mock.calls
        .filter(([name]) => name === "get_gateway")
        .map(([, args]) => args.clientId),
    ).toEqual(["codex", "claude", "codex"]);
    expect(
      mock.command.mock.calls.some(([name]) => name === "update_gateway"),
    ).toBe(false);
  });

  it("keeps an unsaved settings draft when switching is refused and discards it only after confirmation", async () => {
    const user = userEvent.setup();
    const confirm = vi
      .spyOn(window, "confirm")
      .mockReturnValueOnce(false)
      .mockReturnValueOnce(true);
    render(<Gateway notify={() => {}} />);
    await screen.findByLabelText(`${state.providers[0].name} 操作`);
    await user.click(screen.getByText("高级设置", { selector: "summary" }));
    const port = screen.getByRole("spinbutton", { name: "本地端口" });
    await user.clear(port);
    await user.type(port, "23456");
    await user.click(screen.getByRole("button", { name: "Claude Code" }));
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(port).toHaveValue(23456);
    expect(screen.getByRole("button", { name: "Codex" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(mock.command).not.toHaveBeenCalledWith("get_gateway", {
      clientId: "claude",
    });

    await user.click(screen.getByRole("button", { name: "Claude Code" }));
    await screen.findByLabelText(`${claudeState.providers[0].name} 操作`);
    expect(confirm).toHaveBeenCalledTimes(2);
    await user.click(screen.getByText("高级设置", { selector: "summary" }));
    expect(screen.getByRole("spinbutton", { name: "本地端口" })).toHaveValue(
      claudeState.settings.port,
    );
    expect(
      mock.command.mock.calls.some(([name]) => name === "update_gateway"),
    ).toBe(false);
    await user.click(screen.getByRole("button", { name: "Codex" }));
    await screen.findByLabelText(`${state.providers[0].name} 操作`);
    expect(confirm).toHaveBeenCalledTimes(2);
    await user.click(screen.getByText("高级设置", { selector: "summary" }));
    expect(screen.getByRole("spinbutton", { name: "本地端口" })).toHaveValue(
      state.settings.port,
    );
  });

  it("restores Claude selection and saves only its two credential fields with the Claude revision", async () => {
    localStorage.setItem("lich13-switch.main.client", "claude");
    const user = userEvent.setup();
    render(<Gateway notify={() => {}} />);
    await screen.findByLabelText(`${claudeState.providers[0].name} 操作`);
    expect(mock.command).not.toHaveBeenCalledWith("get_gateway", {
      clientId: "codex",
    });
    await user.click(screen.getByRole("button", { name: "添加" }));
    const dialog = screen.getByRole("dialog", { name: "添加供应商" });
    expect(dialog.querySelectorAll("input")).toHaveLength(2);
    const base = within(dialog).getByLabelText("ANTHROPIC_BASE_URL");
    const token = within(dialog).getByLabelText("ANTHROPIC_AUTH_TOKEN");
    expect(token).toHaveAttribute("type", "password");
    expect(
      within(dialog).queryByLabelText("experimental_bearer_token"),
    ).not.toBeInTheDocument();
    await user.type(base, "https://claude.fixture.invalid/deployment");
    await user.type(token, "claude-form-fixture");
    act(() =>
      mock.listeners.get("gateway-state")!({
        ...state,
        revision: "unrelated-codex-update",
      }),
    );
    expect(base).toHaveValue("https://claude.fixture.invalid/deployment");
    expect(token).toHaveValue("claude-form-fixture");
    await user.click(within(dialog).getByRole("button", { name: "保存" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    expect(mock.command).toHaveBeenCalledWith("update_gateway", {
      clientId: "claude",
      edit: {
        op: "saveProvider",
        id: null,
        baseUrl: "https://claude.fixture.invalid/deployment",
        token: "claude-form-fixture",
      },
      expectedRevision: claudeState.revision,
    });
  });
});
