import { useEffect, useState } from "react";
import { command } from "./bridge";
import { errorOf, type StartupState } from "./types";

export default function StartupSettings() {
  const [state, setState] = useState<StartupState | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let disposed = false;
    void command<StartupState>("get_startup")
      .then((s) => {
        if (!disposed) setState(s);
      })
      .catch((e) => {
        if (!disposed) setError(errorOf(e).message);
      });
    return () => {
      disposed = true;
    };
  }, []);
  const update = async (
    key: "launchOnBoot" | "restoreGateway",
    value: boolean,
  ) => {
    if (!state) return;
    setBusy(true);
    setError("");
    const next = { ...state, [key]: value };
    try {
      setState(
        await command<StartupState>("set_startup", {
          enabled: next.launchOnBoot,
          preferences: {
            restoreGateway: next.restoreGateway,
          },
          expectedRevision: state.revision,
        }),
      );
    } catch (e) {
      setError(errorOf(e).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <fieldset className="startup-settings" disabled={busy || !state}>
      <legend>启动</legend>
      {(
        [
          ["launchOnBoot", "开机静默启动"],
          ["restoreGateway", "启动时恢复网关"],
        ] as const
      ).map(([key, label]) => (
        <label className="check-label" key={key}>
          <input
            type="checkbox"
            checked={state?.[key] ?? false}
            onChange={(e) => void update(key, e.target.checked)}
          />
          {label}
        </label>
      ))}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </fieldset>
  );
}
