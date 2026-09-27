import { render, screen, waitFor, act } from "@testing-library/react";
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
    await user.click(await screen.findByRole("button", { name: "添加供应商" }));
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
    await screen.findByText("已关闭");
    const newer = {
      ...state,
      revision: "new-from-tray",
      mode: "auto" as const,
      running: true,
      activeConnections: 2,
    };
    act(() => mock.listeners.get("gateway-state")!(newer));
    expect(screen.getByText("2 个连接")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "自动故障转移" }),
    ).toHaveAttribute("aria-pressed", "true");
    await user.selectOptions(
      screen.getByLabelText("api.example.com 连接方式"),
      "cloud",
    );
    expect(mock.command).toHaveBeenCalledWith("update_gateway", {
      edit: { op: "routeProvider", id: "primary", proxyId: "cloud" },
      expectedRevision: "new-from-tray",
    });
  });
  it("keeps a failed provider draft open with its error", async () => {
    const user = userEvent.setup();
    render(<Gateway section="gateway" notify={() => {}} />);
    await user.click(await screen.findByRole("button", { name: "添加供应商" }));
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
});
