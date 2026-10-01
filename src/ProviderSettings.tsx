import { useEffect, useRef, useState } from "react";
import type { EditRevision } from "./gateway-edit";
import { confirmAction } from "./confirmation";
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
  onDirtyChange,
}: {
  provider: Provider;
  runtime: Provider;
  clientId: ClientId;
  revision: string;
  disabled?: boolean;
  save: (
    edit: { op: "websocketProvider"; id: string; supportsWebsocket: boolean },
    revision: EditRevision,
  ) => Promise<void>;
  close: () => void;
  models: () => void;
  onDirtyChange?: (dirty: boolean) => void;
}) {
  const [supportsWebsocket, setSupportsWebsocket] = useState(
    provider.supportsWebsocket,
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(provider.supportsWebsocket);
  const baseline = useRef(revision);
  const [retry, setRetry] = useState(false);
  const dirty = supportsWebsocket !== saved;
  useEffect(() => {
    if (!dirty && !saving && !error) {
      setSupportsWebsocket(runtime.supportsWebsocket);
      setSaved(runtime.supportsWebsocket);
      baseline.current = revision;
    }
  }, [runtime.supportsWebsocket, revision, dirty, saving, error]);
  useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);
  const leave = async (action: () => void) => {
    if (saving) return;
    if (!dirty || (await confirmAction("放弃未保存的修改？"))) action();
  };
  const changeWebsocket = async () => {
    setSaving(true);
    setError("");
    try {
      await save(
        {
          op: "websocketProvider",
          id: provider.id,
          supportsWebsocket,
        },
        retry ? null : baseline.current,
      );
      setSaved(supportsWebsocket);
      setRetry(false);
    } catch (cause) {
      setError(errorOf(cause).message);
      setRetry(true);
    } finally {
      setSaving(false);
    }
  };
  return (
    <Modal title={provider.name} close={() => void leave(close)} busy={saving}>
      <div className="provider-settings">
        {clientId === "codex" && (
          <div className="settings-toggle-row">
            <label className="settings-toggle">
              <input
                type="checkbox"
                checked={supportsWebsocket}
                disabled={disabled || saving}
                onChange={(event) => {
                  setSupportsWebsocket(event.target.checked);
                }}
              />
              <span>原生 WebSocket</span>
            </label>
            {(dirty || error) && (
              <button
                type="button"
                className="primary small"
                disabled={disabled || saving}
                onClick={() => void changeWebsocket()}
              >
                {saving ? "保存中…" : retry ? "重试" : "保存"}
              </button>
            )}
          </div>
        )}
        {error && (
          <div className="form-error" role="alert">
            {error}
          </div>
        )}
        <button
          type="button"
          className="secondary model-settings-entry"
          onClick={() => void leave(models)}
          disabled={saving}
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
