import {
  act,
  fireEvent,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { confirmAction } from "./confirmation";

it("requires an explicit choice and rejects concurrent navigation without native confirm", async () => {
  const native = vi.spyOn(window, "confirm").mockReturnValue(true);
  let decision!: Promise<boolean>;
  act(() => {
    decision = confirmAction("丢弃当前草稿？");
  });
  const dialog = await screen.findByRole("dialog", { name: "确认操作" });
  expect(await confirmAction("竞争的导航")).toBe(false);
  expect(native).not.toHaveBeenCalled();
  await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
  expect(await decision).toBe(false);
  act(() => {
    decision = confirmAction("丢弃当前草稿？");
  });
  await userEvent.click(
    within(await screen.findByRole("dialog", { name: "确认操作" })).getByRole(
      "button",
      { name: "放弃修改" },
    ),
  );
  expect(await decision).toBe(true);
  expect(document.querySelector("[data-confirmation]")).toBeNull();
  native.mockRestore();
});

it("Escape cancels and returns keyboard focus to the invoking control", async () => {
  const button = document.createElement("button");
  document.body.appendChild(button);
  button.focus();
  let decision!: Promise<boolean>;
  act(() => {
    decision = confirmAction("重新读取配置？");
  });
  const dialog = await screen.findByRole("dialog", { name: "确认操作" });
  expect(within(dialog).getByRole("heading")).toHaveFocus();
  await act(async () => {
    fireEvent(dialog, new Event("cancel", { cancelable: true }));
  });
  expect(await decision).toBe(false);
  await waitFor(() => expect(button).toHaveFocus());
  button.remove();
});
