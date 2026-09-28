import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
const mock = vi.hoisted(() => ({
  command: vi.fn(),
  listeners: new Map<string, (v: unknown) => void>(),
}));
vi.mock("./bridge", () => ({
  preview: true,
  command: mock.command,
  subscribe: async (name: string, fn: (v: unknown) => void) => {
    mock.listeners.set(name, fn);
    return () => mock.listeners.delete(name);
  },
}));
import QuickPanel from "./QuickPanel";
import { demo } from "./preview";
import { gatewayDemo } from "./gateway-preview";
beforeEach(() => {
  mock.command.mockReset();
  mock.listeners.clear();
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  mock.command.mockImplementation(
    async (name: string, args: Record<string, unknown>) => {
      if (name === "get_state" || name === "switch_account")
        return structuredClone(demo);
      if (name === "get_gateway" || name === "update_gateway")
        return structuredClone(gatewayDemo);
      if (name === "get_quick")
        return { pinned: false, tab: "providers", visible: true };
      if (name === "set_quick")
        return { pinned: false, tab: "providers", ...args };
    },
  );
});
it("uses current revisions for quick actions without initializing the main window", async () => {
  const user = userEvent.setup();
  render(<QuickPanel />);
  await screen.findByText("api.example.com");
  act(() =>
    mock.listeners.get("gateway-state")!({
      ...gatewayDemo,
      running: true,
      revision: "current-route",
      waitingRequests: 2,
    }),
  );
  expect(screen.getByText("等待 2")).toBeInTheDocument();
  await user.click(
    screen.getByRole("button", { name: "选择 api.example.com" }),
  );
  expect(mock.command).toHaveBeenCalledWith("update_gateway", {
    edit: { op: "select", id: "primary" },
    expectedRevision: "current-route",
  });
  expect(mock.command).not.toHaveBeenCalledWith(
    "frontend_ready",
    expect.anything(),
  );
  await user.click(screen.getByRole("button", { name: "账号" }));
  await user.type(screen.getByRole("textbox", { name: "搜索账号" }), "工作");
  expect(
    screen.getByRole("button", { name: "切换到 工作空间" }),
  ).toBeInTheDocument();
  expect(screen.queryByText("日常开发")).not.toBeInTheDocument();
  act(() =>
    mock.listeners.get("switch-state")!({
      ...demo,
      authRevision: "external-auth",
    }),
  );
  await user.click(screen.getByRole("button", { name: "切换到 工作空间" }));
  expect(mock.command).toHaveBeenCalledWith("switch_account", {
    id: "work",
    expectedRevision: "external-auth",
  });
});
it("can restore a newer saved credential without auto-writing the auth file", async () => {
  const user = userEvent.setup();
  render(<QuickPanel />);
  await screen.findByText("api.example.com");
  await user.click(screen.getByRole("button", { name: "账号" }));
  act(() =>
    mock.listeners.get("switch-state")!({
      ...demo,
      authSync: {
        state: "older",
        accountId: "personal",
        message: "检测到旧凭据，已保留已保存版本",
        at: 1,
      },
    }),
  );
  expect(
    mock.command.mock.calls.some(([name]) => name === "switch_account"),
  ).toBe(false);
  await user.click(screen.getByRole("button", { name: "使用已保存版本" }));
  expect(mock.command).toHaveBeenCalledWith("switch_account", {
    id: "personal",
    expectedRevision: demo.authRevision,
  });
});
