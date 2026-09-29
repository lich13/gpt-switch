import { useEffect, useRef, useState } from "react";
import Modal from "./Modal";
import { providerStatus } from "./provider-status";
import { errorOf, type Provider, type ProxyProfile } from "./types";
export default function ProviderSettings({
  provider,
  runtime,
  proxies,
  close,
  save,
  models,
  onDirtyChange,
}: {
  provider: Provider;
  runtime: Provider;
  proxies: ProxyProfile[];
  close: () => void;
  save: (edit: Record<string, unknown>) => Promise<void>;
  models: () => void;
  onDirtyChange?: (dirty: boolean) => void;
}) {
  const initialSave = useRef(save);
  const [proxy, setProxy] = useState(provider.proxyId ?? ""),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const dirty = proxy !== (provider.proxyId ?? "");
  useEffect(() => {
    onDirtyChange?.(dirty);
    return () => onDirtyChange?.(false);
  }, [dirty, onDirtyChange]);
  return (
    <Modal title={provider.name} close={close} busy={busy}>
      <form
        className="provider-settings"
        onSubmit={(e) => {
          e.preventDefault();
          setBusy(true);
          setError("");
          void initialSave
            .current({
              op: "routeProvider",
              id: provider.id,
              proxyId: proxy || null,
            })
            .catch((e) => setError(errorOf(e).message))
            .finally(() => setBusy(false));
        }}
      >
        <label>
          连接方式
          <select value={proxy} onChange={(e) => setProxy(e.target.value)}>
            <option value="">直连</option>
            {proxies.map((p) => (
              <option value={p.id} key={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          className="secondary model-settings-entry"
          onClick={() => {
            if (!dirty || confirm("离开会丢弃未保存的设置，是否继续？"))
              models();
          }}
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
            {providerStatus(
              runtime,
              proxies.find((p) => p.id === runtime.proxyId),
            ) && (
              <>
                <dt>准入状态</dt>
                <dd>
                  {providerStatus(
                    runtime,
                    proxies.find((p) => p.id === runtime.proxyId),
                  )}
                </dd>
              </>
            )}
          </dl>
        </details>
        {error && (
          <div className="form-error" role="alert">
            {error}
          </div>
        )}
        <div className="modal-actions">
          <button
            type="button"
            className="secondary"
            onClick={close}
            disabled={busy}
          >
            取消
          </button>
          <button className="primary" disabled={busy}>
            保存
          </button>
        </div>
      </form>
    </Modal>
  );
}
