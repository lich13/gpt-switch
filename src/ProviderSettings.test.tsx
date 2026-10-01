import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import ProviderSettings from "./ProviderSettings";
import { gatewayDemo } from "./gateway-preview";

it("preserves a transport draft across events and retries a conflict with the refreshed revision", async () => {
  const user = userEvent.setup();
  const provider = structuredClone(gatewayDemo.providers[0]);
  const save = vi
    .fn()
    .mockRejectedValueOnce({ message: "网关设置已变化" })
    .mockResolvedValue(undefined);
  const dirty = vi.fn();
  const props = {
    provider,
    runtime: provider,
    clientId: "codex" as const,
    revision: "before",
    save,
    close: vi.fn(),
    models: vi.fn(),
    onDirtyChange: dirty,
  };
  const { rerender } = render(<ProviderSettings {...props} />);
  const checkbox = screen.getByRole("checkbox", { name: "原生 WebSocket" });
  await user.click(checkbox);
  expect(save).not.toHaveBeenCalled();
  rerender(
    <ProviderSettings
      {...props}
      runtime={{ ...provider, activeRequests: 2 }}
      revision="external"
    />,
  );
  expect(checkbox).not.toBeChecked();
  expect(checkbox).toHaveFocus();
  await user.click(screen.getByRole("button", { name: "保存" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("网关设置已变化");
  expect(save).toHaveBeenLastCalledWith(
    { op: "websocketProvider", id: provider.id, supportsWebsocket: false },
    "before",
  );
  expect(checkbox).not.toBeChecked();
  rerender(<ProviderSettings {...props} revision="refreshed" />);
  await user.click(screen.getByRole("button", { name: "重试" }));
  expect(save).toHaveBeenLastCalledWith(
    { op: "websocketProvider", id: provider.id, supportsWebsocket: false },
    null,
  );
  rerender(
    <ProviderSettings
      {...props}
      runtime={{ ...provider, supportsWebsocket: false }}
      revision="saved"
    />,
  );
  await waitFor(() =>
    expect(screen.queryByRole("alert")).not.toBeInTheDocument(),
  );
  expect(dirty).toHaveBeenLastCalledWith(false);
});

it("blocks duplicate transport saves and keeps the draft until completion", async () => {
  const user = userEvent.setup();
  const provider = structuredClone(gatewayDemo.providers[0]);
  let resolve!: () => void;
  const save = vi.fn(
    () =>
      new Promise<void>((done) => {
        resolve = done;
      }),
  );
  render(
    <ProviderSettings
      provider={provider}
      runtime={provider}
      clientId="codex"
      revision="1"
      save={save}
      close={() => {}}
      models={() => {}}
    />,
  );
  await user.click(screen.getByRole("checkbox"));
  await user.click(screen.getByRole("button", { name: "保存" }));
  expect(screen.getByRole("checkbox")).toBeDisabled();
  expect(screen.getByRole("button", { name: "保存中…" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "关闭" })).toBeDisabled();
  expect(save).toHaveBeenCalledTimes(1);
  await act(async () => resolve());
});

it("does not expose transport controls for Claude", () => {
  const provider = structuredClone(gatewayDemo.providers[0]);
  render(
    <ProviderSettings
      provider={provider}
      runtime={provider}
      clientId="claude"
      revision="1"
      save={vi.fn()}
      close={() => {}}
      models={() => {}}
    />,
  );
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
});
