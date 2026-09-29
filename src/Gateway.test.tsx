import { render, screen, waitFor, act, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, it, expect, vi } from "vitest";
import type { GatewayState } from "./types";
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
import { gatewayDemo } from "./gateway-preview";
let state: GatewayState;
beforeEach(() => {
  state = structuredClone(gatewayDemo);
  mock.command.mockReset();
  mock.listeners.clear();
  mock.command.mockImplementation(async (name: string) => {
    if (name === "get_gateway" || name === "update_gateway")
      return structuredClone(state);
    if (name === "test_provider") return 51;
  });
});
describe("gateway controls", () => {
  it("keeps the provider API form to two fields and sends no guessed model settings", async () => {
    const user = userEvent.setup();
    render(<Gateway section="gateway" notify={() => {}} />);
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
  it("reflects tray gateway events and sends the latest revision for route edits", async () => {
    const user = userEvent.setup();
    render(<Gateway section="gateway" notify={() => {}} />);
    await screen.findByRole("button", { name: "启用" });
    const newer = {
      ...state,
      revision: "new-from-tray",
      mode: "auto" as const,
      running: true,
      activeConnections: 2,
    };
    act(() => mock.listeners.get("gateway-state")!(newer));
    await user.click(screen.getByText("高级设置", { selector: "summary" }));
    expect(screen.getByText("活动连接 2")).toBeVisible();
    expect(screen.getByRole("button", { name: "自动" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    const operations = screen.getByLabelText("api.example.com 操作");
    await user.click(operations);
    await user.click(
      within(operations.closest("details")!).getByRole("button", {
        name: "供应商设置",
      }),
    );
    await user.selectOptions(screen.getByLabelText("连接方式"), "cloud");
    await user.click(screen.getByRole("button", { name: "保存" }));
    expect(mock.command).toHaveBeenCalledWith("update_gateway", {
      edit: {
        op: "configureProvider",
        id: "primary",
        proxyId: "cloud",
        maxConcurrency: state.providers[0].maxConcurrency,
        queued: state.providers[0].queued,
      },
      expectedRevision: "new-from-tray",
    });
  });
  it("keeps a failed provider draft open with its error", async () => {
    const user = userEvent.setup();
    render(<Gateway section="gateway" notify={() => {}} />);
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
  it("preserves the provider settings draft and original revision when a save conflicts", async () => {
    const user = userEvent.setup();
    mock.command.mockImplementation(async (name: string) => {
      if (name === "get_gateway") return structuredClone(state);
      if (name === "update_gateway")
        throw { code: "CONFLICT", message: "供应商设置已变化" };
    });
    render(<Gateway section="gateway" notify={() => {}} />);
    const operations = await screen.findByLabelText("api.example.com 操作");
    await user.click(operations);
    await user.click(
      within(operations.closest("details")!).getByRole("button", {
        name: "供应商设置",
      }),
    );
    const concurrency = screen.getByLabelText(/^并发上限/);
    await user.clear(concurrency);
    await user.type(concurrency, "7");
    await user.selectOptions(screen.getByLabelText("连接方式"), "cloud");
    act(() =>
      mock.listeners.get("gateway-state")!({
        ...state,
        revision: "external-settings",
      }),
    );
    await user.click(screen.getByRole("button", { name: "保存" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "供应商设置已变化",
    );
    expect(concurrency).toHaveValue(7);
    expect(screen.getByLabelText("连接方式")).toHaveValue("cloud");
    expect(mock.command).toHaveBeenCalledWith("update_gateway", {
      edit: {
        op: "configureProvider",
        id: "primary",
        maxConcurrency: 7,
        proxyId: "cloud",
        queued: state.providers[0].queued,
      },
      expectedRevision: state.revision,
    });
  });
  it("does not populate stored proxy passwords and preserves a blank edit", async () => {
    const user = userEvent.setup();
    render(<Gateway section="proxies" notify={() => {}} />);
    await user.click(
      await screen.findByRole("button", { name: "编辑 腾讯云 SOCKS5" }),
    );
    expect(screen.getByLabelText("密码")).toHaveValue("");
    expect(screen.getByLabelText("密码")).toHaveAttribute("type", "password");
    await user.click(screen.getByRole("button", { name: "保存" }));
    expect(mock.command).toHaveBeenCalledWith("update_gateway", {
      edit: {
        op: "saveProxy",
        id: "cloud",
        name: "腾讯云 SOCKS5",
        host: "203.0.113.1",
        port: 10808,
        username: "preview",
        password: "",
      },
      expectedRevision: state.revision,
    });
  });
  it("switches a stopped provider with the visible config revision and restart feedback", async () => {
    const notify = vi.fn();
    const user = userEvent.setup();
    render(<Gateway section="gateway" notify={notify} />);
    await user.click(await screen.findByRole("button", { name: "选择" }));
    expect(mock.command).toHaveBeenCalledWith("update_gateway", {
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
  render(<Gateway section="gateway" notify={() => {}} />);
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
    edit: {
      op: "modelsProvider",
      id: "primary",
      allowedModels: ["gpt-A", "custom-model"],
    },
    expectedRevision: state.revision,
  });
});
