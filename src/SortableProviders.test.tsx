import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import SortableProviders, { type ProviderCommit } from "./SortableProviders";
import { gatewayDemo } from "./gateway-preview";

const providers = ["a", "b", "c"].map((id) => ({
  ...gatewayDemo.providers[0],
  id,
  name: id,
  queued: id !== "b",
}));
beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
    function (this: HTMLElement) {
      const id =
        this.closest<HTMLElement>("[data-provider-id]")?.dataset.providerId;
      const top = id ? providers.findIndex((p) => p.id === id) * 60 : 0;
      return {
        x: 0,
        y: top,
        left: 0,
        right: 400,
        top,
        bottom: top + (id ? 60 : 400),
        width: 400,
        height: id ? 60 : 400,
        toJSON() {},
      };
    },
  );
  HTMLElement.prototype.scrollIntoView = vi.fn();
});
afterEach(() => vi.restoreAllMocks());
function List({
  commit,
  report,
  revision = "r1",
  visible = true,
}: {
  commit: ProviderCommit;
  report: (message: string) => void;
  revision?: string;
  visible?: boolean;
}) {
  return (
    <SortableProviders
      providers={providers}
      revision={revision}
      visible={visible}
      disabled={false}
      commit={commit}
      report={report}
      rowClass={() => "row"}
    >
      {(p, priority, handle) => (
        <>
          {handle}
          <span>
            {p.name}
            {priority ? ` P${priority}` : ""}
          </span>
          <button>选择 {p.name}</button>
        </>
      )}
    </SortableProviders>
  );
}
async function pick() {
  const user = userEvent.setup();
  screen.getByRole("button", { name: "拖动 a" }).focus();
  await user.keyboard(" ");
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "拖动 a" })).toHaveAttribute(
      "aria-pressed",
      "true",
    ),
  );
  return user;
}
it("keyboard sorting submits one complete insertion order with the original revision", async () => {
  const commit = vi.fn().mockResolvedValue(undefined);
  render(<List commit={commit} report={vi.fn()} />);
  expect(screen.getByText("c P2")).toBeInTheDocument();
  const user = await pick();
  await user.keyboard("{ArrowDown}{ArrowDown} ");
  await waitFor(() =>
    expect(commit).toHaveBeenCalledWith(
      { op: "reorder", ids: ["b", "c", "a"] },
      "r1",
    ),
  );
  expect(commit).toHaveBeenCalledTimes(1);
});
it.each(["Escape", "no-op", "blur", "hide", "conflict"])(
  "%s cancels without submitting an old order",
  async (kind) => {
    const commit = vi.fn(),
      report = vi.fn();
    const { rerender } = render(<List commit={commit} report={report} />);
    const user = await pick();
    if (kind === "no-op") await user.keyboard(" ");
    else {
      await user.keyboard("{ArrowDown}");
      if (kind === "Escape") {
        await user.keyboard("{Escape}");
        await waitFor(() =>
          expect(screen.getByRole("button", { name: "拖动 a" })).toHaveFocus(),
        );
      }
      if (kind === "blur") fireEvent(window, new Event("blur"));
      if (kind === "hide")
        rerender(<List commit={commit} report={report} visible={false} />);
      if (kind === "conflict") {
        rerender(<List commit={commit} report={report} revision="r2" />);
        expect(report).toHaveBeenCalledWith("供应商设置已变化，请重新排序");
      }
    }
    expect(commit).not.toHaveBeenCalled();
  },
);
it("failed persistence restores the authoritative order without changing queue membership", async () => {
  const commit = vi.fn().mockRejectedValue(new Error("写入失败")),
    report = vi.fn();
  const { container } = render(<List commit={commit} report={report} />);
  const user = await pick();
  await user.keyboard("{ArrowDown} ");
  await waitFor(() => expect(report).toHaveBeenCalledWith("写入失败"));
  expect(
    [...container.querySelectorAll<HTMLElement>("article")].map(
      (e) => e.dataset.providerId,
    ),
  ).toEqual(["a", "b", "c"]);
  expect(screen.getByText("c P2")).toBeInTheDocument();
});
