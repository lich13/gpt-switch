import { useEffect, useState } from "react";
import Modal from "./Modal";
import { errorOf } from "./types";
import { providerStatus } from "./provider-status";
import type { ClientId, Provider } from "./types";
export default function ProviderSettings({
  provider,
  runtime,
  clientId,
  revision,
  disabled = false,
  save,
  close,
  models,
}: {
  provider: Provider;
  runtime: Provider;
  clientId: ClientId;
  revision: string;
  disabled?: boolean;
  save: (
    edit: { op: "websocketProvider"; id: string; supportsWebsocket: boolean },
    revision: string,
  ) => Promise<void>;
  close: () => void;
  models: () => void;
}) {
  const [supportsWebsocket, setSupportsWebsocket] = useState(
    provider.supportsWebsocket,
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    setSupportsWebsocket(provider.supportsWebsocket);
  }, [provider.id]);
  const changeWebsocket = async (next: boolean) => {
    setSupportsWebsocket(next);
    setSaving(true);
    setError("");
    try {
      await save(
        {
          op: "websocketProvider",
          id: provider.id,
          supportsWebsocket: next,
        },
        revision,
      );
    } catch (cause) {
      setError(errorOf(cause).message);
    } finally {
      setSaving(false);
    }
  };
  return (
    <Modal title={provider.name} close={close}>
      <div className="provider-settings">
        {clientId === "codex" && (
          <label className="settings-toggle">
            <input
              type="checkbox"
              checked={supportsWebsocket}
              disabled={disabled || saving}
              onChange={(event) => {
                void changeWebsocket(event.target.checked);
              }}
            />
            <span>原生 WebSocket</span>
          </label>
        )}
        {error && (
          <div className="form-error" role="alert">
            {error}
          </div>
        )}
        <button
          type="button"
          className="secondary model-settings-entry"
          onClick={models}
        >
          模型白名单 <span>{provider.allowedModels?.length ?? "不限"}</span>
        </button>
        <details className="runtime-status">
          <summary>运行状态</summary>
          <dl>
            <dt>并发</dt>
            <dd>
              {runtime.activeRequests} / {runtime.maxConcurrency || "不限"}
            </dd>
            <dt>连续失败</dt>
            <dd>{runtime.health.failures}</dd>
            {providerStatus(runtime) && (
              <>
                <dt>准入状态</dt>
                <dd>{providerStatus(runtime)}</dd>
              </>
            )}
          </dl>
        </details>
      </div>
    </Modal>
  );
}
