import type { ViewState } from "./types";
export default function AuthSyncNotice({
  state,
  busy,
  apply,
}: {
  state: ViewState;
  busy: boolean;
  apply: (id: string) => void;
}) {
  const sync = state.authSync;
  if (!sync?.message) return null;
  return (
    <div
      className={`auth-sync ${sync.state === "older" ? "warning" : ""}`}
      role="status"
      title={new Date(sync.at * 1000).toLocaleString()}
    >
      <span>{sync.message}</span>
      {sync.state === "older" && sync.accountId && (
        <button
          className="text-button"
          disabled={busy}
          onClick={() => apply(sync.accountId!)}
        >
          使用已保存版本
        </button>
      )}
    </div>
  );
}
