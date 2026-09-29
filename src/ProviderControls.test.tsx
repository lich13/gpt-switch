import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import ProviderControls from "./ProviderControls";
import { gatewayDemo } from "./gateway-preview";

const provider = gatewayDemo.providers[0];
it("validates caps, preserves edits through runtime refresh and retries conflicts explicitly", async () => {
  const user = userEvent.setup();
  const commit = vi.fn().mockRejectedValueOnce(new Error("设置已变化"));
  const report = vi.fn();
  const { rerender } = render(
    <ProviderControls
      provider={provider}
      revision="old"
      disabled={false}
      commit={commit}
      report={report}
    />,
  );
  await user.click(screen.getByRole("button", { name: /并发上限/ }));
  const input = screen.getByRole("spinbutton");
  for (const value of ["", "-1", "1.5", "100001"]) {
    fireEvent.change(input, { target: { value } });
    await user.click(screen.getByRole("button", { name: "保存" }));
    expect(screen.getByRole("alert")).toHaveTextContent("0–100000");
    expect(commit).not.toHaveBeenCalled();
  }
  fireEvent.change(input, { target: { value: "8" } });
  rerender(
    <ProviderControls
      provider={{ ...provider, activeRequests: 3, maxConcurrency: 6 }}
      revision="new"
      disabled={false}
      commit={commit}
      report={report}
    />,
  );
  expect(input).toHaveValue(8);
  await user.click(screen.getByRole("button", { name: "保存" }));
  expect(commit).toHaveBeenLastCalledWith(
    { op: "concurrencyProvider", id: provider.id, maxConcurrency: 8 },
    "old",
  );
  expect(await screen.findByRole("alert")).toHaveTextContent("设置已变化");
  expect(input).toHaveValue(8);
  commit.mockResolvedValueOnce(undefined);
  fireEvent.change(input, { target: { value: "0" } });
  await user.click(screen.getByRole("button", { name: "保存" }));
  expect(commit).toHaveBeenLastCalledWith(
    { op: "concurrencyProvider", id: provider.id, maxConcurrency: 0 },
    "new",
  );
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
  expect(screen.getByRole("button", { name: /并发上限/ })).toHaveFocus();
  expect(report).not.toHaveBeenCalled();
});

it("keeps authoritative queue state on failure and prevents duplicate submissions", async () => {
  let reject!: (e: Error) => void;
  const commit = vi.fn(
    () =>
      new Promise<void>((_, fail) => {
        reject = fail;
      }),
  );
  const report = vi.fn(),
    user = userEvent.setup();
  render(
    <ProviderControls
      provider={provider}
      revision="r1"
      disabled={false}
      commit={commit}
      report={report}
    />,
  );
  const toggle = screen.getByRole("button", { name: /移出队列/ });
  await user.dblClick(toggle);
  expect(commit).toHaveBeenCalledTimes(1);
  expect(commit).toHaveBeenCalledWith(
    { op: "queueProvider", id: provider.id, queued: false },
    "r1",
  );
  expect(toggle).toBeDisabled();
  reject(new Error("写入失败"));
  await waitFor(() => expect(report).toHaveBeenCalledWith("写入失败"));
  expect(toggle).toHaveAttribute("aria-pressed", "true");
  expect(toggle).toBeEnabled();
});

it("Escape closes only the editor, returns focus and hidden panels discard drafts", async () => {
  const commit = vi.fn(),
    report = vi.fn(),
    user = userEvent.setup();
  const props = { provider, revision: "r", disabled: false, commit, report };
  const { rerender } = render(<ProviderControls {...props} />);
  const anchor = screen.getByRole("button", { name: /并发上限/ });
  await user.click(anchor);
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(anchor).toHaveFocus();
  await user.click(anchor);
  fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "17" } });
  rerender(<ProviderControls {...props} visible={false} />);
  rerender(<ProviderControls {...props} visible />);
  await user.click(anchor);
  expect(screen.getByRole("spinbutton")).toHaveValue(provider.maxConcurrency);
  expect(commit).not.toHaveBeenCalled();
});
