import Modal from "./Modal";
import { providerStatus } from "./provider-status";
import type { Provider } from "./types";
export default function ProviderSettings({
  provider,
  runtime,
  close,
  models,
}: {
  provider: Provider;
  runtime: Provider;
  close: () => void;
  models: () => void;
}) {
  return (
    <Modal title={provider.name} close={close}>
      <div className="provider-settings">
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
