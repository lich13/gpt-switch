import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
export const native = "__TAURI_INTERNALS__" in window;
export const preview = !native && import.meta.env.DEV;
export async function command<T>(
  name: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  if (native) return invoke<T>(name, args);
  if (preview) return (await import("./preview")).run(name, args) as Promise<T>;
  throw new Error("请在 lich13-switch 桌面客户端中打开");
}
export async function subscribe<T>(
  event: string,
  fn: (payload: T) => void,
): Promise<() => void> {
  if (native) return listen<T>(event, (e) => fn(e.payload));
  if (preview) return (await import("./preview")).subscribe(event, fn);
  return () => {};
}
