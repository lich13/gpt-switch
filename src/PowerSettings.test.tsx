import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
const mock = vi.hoisted(() => ({
  command: vi.fn(),
  listener: (_s: unknown) => {},
}));
vi.mock("./bridge", () => ({
  command: mock.command,
  subscribe: vi.fn(async (_name: string, listener: (s: unknown) => void) => {
    mock.listener = listener;
    return () => {};
  }),
}));
import PowerSettings from "./PowerSettings";
const initial = {
  supported: true,
  enabled: false,
  batterySleep: 0,
  revision: "initial",
  helper: "notInstalled",
};
beforeEach(() => {
  mock.command.mockReset();
  mock.command.mockImplementation(async () => initial);
});
it("keeps state and controls after authorization is cancelled, then accepts helper events", async () => {
  render(<PowerSettings />);
  await screen.findByText("未安装");
  mock.command.mockRejectedValueOnce({
    code: "POWER_AUTH",
    message: "已取消系统授权",
  });
  fireEvent.click(screen.getByRole("button", { name: "安装" }));
  await screen.findByRole("alert");
  expect(screen.getByText("未安装")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "移除" })).toBeDisabled();
  mock.command.mockResolvedValueOnce({ ...initial, helper: "ready" });
  fireEvent.click(screen.getByRole("button", { name: "安装" }));
  await screen.findByText("已就绪");
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  act(() => mock.listener({ ...initial, helper: "needsRepair" }));
  expect(screen.getByText("需要修复")).toBeInTheDocument();
  mock.command.mockRejectedValueOnce({
    code: "CONFLICT",
    message: "电源状态已被外部修改",
  });
  fireEvent.click(screen.getByRole("button", { name: "移除" }));
  await screen.findByText("电源状态已被外部修改");
  expect(screen.getByText("需要修复")).toBeInTheDocument();
});
it("disables management for an isolated native app and hides unsupported devices", async () => {
  mock.command.mockResolvedValueOnce({ ...initial, helper: "isolated" });
  const view = render(<PowerSettings />);
  await screen.findByText("隔离运行");
  expect(screen.getByRole("button", { name: "修复" })).toBeDisabled();
  view.unmount();
  mock.command.mockResolvedValueOnce({ ...initial, supported: false });
  render(<PowerSettings />);
  await waitFor(() =>
    expect(screen.queryByText("电源助手")).not.toBeInTheDocument(),
  );
});
