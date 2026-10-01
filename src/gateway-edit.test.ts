import { expect, it, vi } from "vitest";
const command = vi.hoisted(() => vi.fn());
vi.mock("./bridge", () => ({ command }));
import { saveGatewayEdit } from "./gateway-edit";
import { gatewayDemo } from "./gateway-preview";

it("never writes on explicit retry if authoritative state cannot be read", async () => {
  command.mockReset();
  command.mockRejectedValue(new Error("读取失败"));
  await expect(saveGatewayEdit("codex", gatewayDemo, { op: "reset", id: "primary" }, null, vi.fn())).rejects.toThrow("读取失败");
  expect(command.mock.calls).toEqual([["get_gateway", { clientId: "codex" }]]);
});

it("revalidates a fresh retry rather than removing optimistic concurrency", async () => {
  command.mockReset();
  command.mockImplementation(async (name) => {
    if (name === "get_gateway") return { ...gatewayDemo, revision: "fresh" };
    throw { code: "CONFLICT", message: "再次变化" };
  });
  await expect(saveGatewayEdit("codex", gatewayDemo, { op: "renameProvider", id: "primary", name: "Draft" }, null, vi.fn())).rejects.toMatchObject({ code: "CONFLICT" });
  expect(command).toHaveBeenCalledWith("update_gateway", expect.objectContaining({ expectedRevision: "fresh" }));
  expect(command.mock.calls.filter(([name]) => name === "update_gateway")).toHaveLength(1);
});
