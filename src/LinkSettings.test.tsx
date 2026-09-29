import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";

const mock = vi.hoisted(() => ({
  command: vi.fn(),
  listeners: new Map<string, (visible: boolean) => void>(),
}));
vi.mock("./bridge", () => ({
  command: mock.command,
  subscribe: vi.fn(
    async (name: string, listener: (visible: boolean) => void) => {
      mock.listeners.set(name, listener);
      return () => mock.listeners.delete(name);
    },
  ),
}));
import LinkSettings from "./LinkSettings";

const original = "com.example.original";
const target = "com.example.gpt-switch";
const apps = [
  { id: original, name: "原接收应用", path: "/fixture/Original.app" },
  { id: target, name: "gpt-Switch", path: "/fixture/gpt-Switch.app" },
];
type Handlers = {
  current: string | null;
  apps: typeof apps;
  systemPicker: boolean;
};
let state: Handlers;
let settingResult: Handlers;
let settingError: unknown;
let settingWait: Promise<void> | null;

beforeEach(() => {
  state = { current: original, apps, systemPicker: false };
  settingResult = state;
  settingError = null;
  settingWait = null;
  mock.command.mockReset();
  mock.listeners.clear();
  mock.command.mockImplementation(async (name: string) => {
    if (name === "get_link_handler_state") return structuredClone(state);
    if (name === "get_startup_error") return null;
    if (name === "set_link_handler") {
      if (settingError) throw settingError;
      if (settingWait) await settingWait;
      return structuredClone(settingResult);
    }
    throw new Error(`Unexpected command: ${name}`);
  });
});

it("shows the returned default association and does not assume a requested change succeeded", async () => {
  const user = userEvent.setup();
  let finish!: () => void;
  settingWait = new Promise((resolve) => {
    finish = resolve;
  });
  render(<LinkSettings />);
  const select = screen.getByRole("combobox", { name: "默认接收应用" });
  await waitFor(() => expect(select).toHaveValue(original));
  await user.selectOptions(select, target);
  expect(mock.command).toHaveBeenCalledWith("set_link_handler", {
    appId: target,
  });
  expect(select).toBeDisabled();
  expect(select).toHaveValue(original);
  expect(screen.getByRole("status")).toHaveTextContent("正在设置");
  await act(async () => finish());
  expect(await screen.findByRole("alert")).toHaveTextContent("默认应用未更改");
  expect(select).toHaveValue(original);
  expect(select).toBeEnabled();
  settingWait = null;
  settingResult = { ...state, current: target };
  await user.selectOptions(select, target);
  await waitFor(() => expect(select).toHaveValue(target));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it("keeps the actual app after opening the system picker and refreshes when the app returns", async () => {
  const user = userEvent.setup();
  state = { ...state, systemPicker: true };
  settingResult = state;
  render(<LinkSettings />);
  const select = screen.getByRole("combobox", { name: "默认接收应用" });
  await waitFor(() => expect(select).toHaveValue(original));
  await user.selectOptions(select, target);
  await waitFor(() => expect(select).toBeEnabled());
  expect(select).toHaveValue(original);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  state = { ...state, current: target };
  fireEvent(window, new Event("focus"));
  await waitFor(() => expect(select).toHaveValue(target));
  state = { ...state, current: original };
  act(() => mock.listeners.get("app-visibility")?.(false));
  expect(select).toHaveValue(target);
  act(() => mock.listeners.get("app-visibility")?.(true));
  await waitFor(() => expect(select).toHaveValue(original));
});

it("preserves an unknown current handler after a native error and releases the controls", async () => {
  const user = userEvent.setup();
  state = { ...state, current: "com.example.external" };
  settingError = { code: "ASSOCIATION", message: "系统拒绝更改默认应用" };
  render(<LinkSettings />);
  const select = screen.getByRole("combobox", { name: "默认接收应用" });
  await waitFor(() => expect(select).toHaveValue("com.example.external"));
  expect(screen.getByRole("option", { name: "其他应用" })).toBeInTheDocument();
  await user.selectOptions(select, target);
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "系统拒绝更改默认应用",
  );
  expect(select).toHaveValue("com.example.external");
  expect(select).toBeEnabled();
  expect(mock.command).toHaveBeenCalledWith("set_link_handler", {
    appId: target,
  });
});
