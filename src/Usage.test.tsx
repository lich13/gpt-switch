import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mock = vi.hoisted(() => ({
  command: vi.fn(),
  listeners: new Map<string, Set<(v: unknown) => void>>(),
}));
vi.mock("./bridge", () => ({
  command: mock.command,
  subscribe: vi.fn(async (name: string, fn: (v: unknown) => void) => {
    const list = mock.listeners.get(name) ?? new Set();
    list.add(fn);
    mock.listeners.set(name, list);
    return () => list.delete(fn);
  }),
}));
import Usage from "./Usage";
import { usagePreview } from "./usage-preview";
import {
  successRate,
  compactMoney,
  outputSpeed,
  logMoney,
  httpClass,
  logPages,
  type Dashboard,
  type LogPage,
} from "./usage-types";
import { trendRows } from "./UsageChart";
const dirty = vi.fn();
const emit = (event: string, value: unknown) =>
  mock.listeners.get(event)?.forEach((fn) => fn(value));
async function flush() {
  await act(async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
  });
}
const loads = () =>
  mock.command.mock.calls.filter(([name]) => name === "get_usage_dashboard")
    .length;
beforeEach(() => {
  mock.listeners.clear();
  mock.command.mockReset();
  dirty.mockReset();
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: "visible",
  });
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute("open");
  };
  mock.command.mockImplementation(async (name: string, args = {}) =>
    name === "get_gateway" ? { providers: [] } : usagePreview(name, args, emit),
  );
});
afterEach(() => vi.useRealTimers());
describe("logical usage views", () => {
  it("excludes business rejection, cancellation and unknown results from the service denominator", () => {
    expect(successRate({ successes: 8, failures: 2 })).toBe("80.0%");
    expect(successRate({ successes: 0, failures: 0 })).toBe("—");
    expect(compactMoney(null)).toBe("未定价");
    expect(compactMoney("0")).toBe("$0.00");
  });
  it("keeps HTTP filters and page when opening attempts and closing the drawer", async () => {
    render(<Usage onDirtyChange={dirty} />);
    await screen.findByText("逻辑请求");
    fireEvent.change(screen.getByLabelText("时间范围"), {
      target: { value: "7" },
    });
    fireEvent.click(screen.getByRole("button", { name: "请求日志" }));
    await screen.findByLabelText("状态码");
    expect(screen.queryByText("逻辑请求")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("状态码"), {
      target: { value: "200" },
    });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "下一页" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "下一页" }));
    await waitFor(() =>
      expect(screen.getByLabelText("日志页码")).toHaveValue(2),
    );
    await waitFor(() =>
      expect(mock.command).toHaveBeenCalledWith(
        "get_usage_logs",
        expect.objectContaining({
          page: 1,
          filters: expect.objectContaining({ status: 200 }),
        }),
      ),
    );
    await flush();
    const trigger = screen.getAllByRole("button", {
      name: /请求详情 logical-/,
    })[0];
    trigger.focus();
    fireEvent.click(trigger);
    const drawer = await screen.findByRole("dialog", { name: "请求详情" });
    expect(within(drawer).getByText("调度过程")).toBeInTheDocument();
    expect(
      within(drawer).getByRole("region", { name: "最终记录" }),
    ).toBeInTheDocument();
    expect(
      mock.command.mock.calls
        .filter(([n]) => n === "get_usage_dashboard")
        .every(([, a]) => !a.filters.status),
    ).toBe(true);
    expect(screen.queryByLabelText("结果")).not.toBeInTheDocument();
    expect(
      screen
        .getAllByRole("columnheader")
        .map((h) => h.textContent)
        .slice(0, 9),
    ).toEqual([
      "时间",
      "供应商",
      "模型",
      "输入 Token",
      "输出 Token",
      "费用",
      "耗时",
      "状态",
      "来源",
    ]);
    fireEvent(drawer, new Event("cancel", { bubbles: true, cancelable: true }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByLabelText("状态码")).toHaveValue("200");
    expect(screen.getByLabelText("日志页码")).toHaveValue(2);
    expect(trigger).toHaveFocus();
  });
  it("pauses automatic refresh while hidden and on pricing, then refreshes on return", async () => {
    vi.useFakeTimers();
    const view = render(<Usage onDirtyChange={dirty} />);
    await flush();
    const initial = loads();
    await act(async () => vi.advanceTimersByTime(30_000));
    await flush();
    expect(loads()).toBe(initial + 1);
    act(() => emit("app-visibility", false));
    await act(async () => vi.advanceTimersByTime(90_000));
    expect(loads()).toBe(initial + 1);
    act(() => emit("app-visibility", true));
    await flush();
    expect(loads()).toBe(initial + 2);
    fireEvent.click(screen.getByRole("button", { name: "定价" }));
    await flush();
    const pricing = loads();
    await act(async () => vi.advanceTimersByTime(90_000));
    expect(loads()).toBe(pricing);
    fireEvent.click(screen.getByRole("button", { name: "概览" }));
    await flush();
    expect(loads()).toBe(pricing + 1);
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "hidden",
    });
    act(() => document.dispatchEvent(new Event("visibilitychange")));
    const hidden = loads();
    await act(async () => vi.advanceTimersByTime(60_000));
    expect(loads()).toBe(hidden);
    view.unmount();
    await act(async () => vi.advanceTimersByTime(60_000));
    expect(loads()).toBe(hidden);
  });
  it("keeps real timestamp gaps in trends and unknown reported values", async () => {
    const data = (await usagePreview(
      "get_usage_dashboard",
      {},
      emit,
    )) as Dashboard;
    const bucket = Math.floor(Date.now() / 3_600_000) * 3600;
    const first = { ...data.summary, at: bucket, requests: 1 };
    const last = {
      ...data.summary,
      at: bucket + 7200,
      requests: 2,
      cost: null,
    };
    const rows = trendRows({
      ...data,
      effectiveStart: bucket,
      effectiveEnd: bucket + 10800,
      trends: [first, last],
    });
    expect(rows.map((r) => r.at)).toEqual([
      bucket,
      bucket + 3600,
      bucket + 7200,
    ]);
    expect(rows.map((r) => r.requests)).toEqual([1, 0, 2]);
    expect(rows[2].cost).toBeNull();
  });
});

it("matches CC Switch numeric, cache and HTTP display without rewriting outcomes", async () => {
  const page = (await usagePreview(
    "get_usage_logs",
    { page: 0 },
    emit,
  )) as LogPage;
  const r = page.records[0];
  expect(httpClass(200)).toBe("success");
  expect(httpClass(503)).toBe("failure");
  expect(httpClass(null)).toBe("unknown");
  expect(logMoney({ ...r, cost: { ...r.cost, total: "0" } })).toBe("$0.0000");
  expect(
    logMoney({ ...r, cost: { ...r.cost, total: null, status: "unpriced" } }),
  ).toBe("未定价");
  expect(
    outputSpeed({
      ...r,
      tokens: { ...r.tokens, output: 100 },
      durationMs: null,
      latencyMs: 2000,
      firstTokenMs: 1000,
    }),
  ).toBe("100");
  expect(
    outputSpeed({ ...r, tokens: { ...r.tokens, output: null } }),
  ).toBeNull();
  expect(logPages(8, 400)).toEqual([
    0,
    1,
    2,
    "gap-7",
    7,
    8,
    9,
    "gap-17",
    17,
    18,
    19,
  ]);
});
