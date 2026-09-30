import { createRoot } from "react-dom/client";
import Modal from "./Modal";

let pending = false;

// WKWebView does not reliably present window.confirm. Use the same accessible
// dialog in the native app and browser, and reject competing navigation events.
export function confirmAction(
  message: string,
  label = "放弃修改",
): Promise<boolean> {
  if (pending) return Promise.resolve(false);
  pending = true;
  return new Promise((resolve) => {
    const container = document.createElement("div");
    container.dataset.confirmation = "true";
    document.body.appendChild(container);
    const root = createRoot(container);
    let settled = false;
    const finish = (accepted: boolean) => {
      if (settled) return;
      settled = true;
      queueMicrotask(() => {
        root.unmount();
        container.remove();
        pending = false;
        resolve(accepted);
      });
    };
    root.render(
      <Modal title="确认操作" compact close={() => finish(false)}>
        <p className="confirmation-message">{message}</p>
        <div className="modal-actions">
          <button type="button" onClick={() => finish(false)}>
            取消
          </button>
          <button
            className="primary"
            type="button"
            onClick={() => finish(true)}
          >
            {label}
          </button>
        </div>
      </Modal>,
    );
  });
}
