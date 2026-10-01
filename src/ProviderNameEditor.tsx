import { Check, Pencil, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { EditRevision } from "./gateway-edit";
import { errorOf, type Provider } from "./types";

export default function ProviderNameEditor({
  provider,
  revision,
  disabled,
  commit,
  report,
  className = "provider-title",
  request,
}: {
  provider: Provider;
  revision: string;
  disabled: boolean;
  commit: (edit: { op: "renameProvider"; id: string; name: string }, revision: EditRevision) => Promise<void>;
  report: (message: string) => void;
  className?: string;
  request?: number;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(provider.name);
  const [saving, setSaving] = useState(false);
  const baseline = useRef<EditRevision>(revision);
  const input = useRef<HTMLInputElement>(null);
  const lastRequest = useRef<number | undefined>(undefined);

  const begin = () => {
    if (disabled || saving) return;
    setDraft(provider.name);
    baseline.current = revision;
    setEditing(true);
  };
  useEffect(() => {
    if (request !== undefined && request !== lastRequest.current) {
      lastRequest.current = request;
      begin();
    }
  }, [request]);
  useEffect(() => {
    if (editing) {
      input.current?.focus();
      input.current?.select();
    }
  }, [editing]);
  const cancel = () => {
    if (saving) return;
    setEditing(false);
    setDraft(provider.name);
  };
  const save = async () => {
    const name = draft.trim();
    if (!name || name.length > 120 || /[\u0000-\u001f\u007f]/.test(name)) {
      report("名称应为 1–120 个可见字符");
      input.current?.focus();
      return;
    }
    setSaving(true);
    try {
      await commit({ op: "renameProvider", id: provider.id, name }, baseline.current);
      setEditing(false);
    } catch (error) {
      baseline.current = null;
      report(errorOf(error).message);
      input.current?.focus();
    } finally {
      setSaving(false);
    }
  };
  if (editing) {
    return (
      <span className={`${className} provider-name-editor`}>
        <input
          ref={input}
          aria-label={`${provider.name} 名称`}
          maxLength={120}
          value={draft}
          disabled={saving}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.nativeEvent.isComposing) {
              event.preventDefault();
              void save();
            } else if (event.key === "Escape" && !event.nativeEvent.isComposing) {
              event.preventDefault();
              cancel();
            }
          }}
        />
        <button type="button" aria-label="保存名称" disabled={saving} onClick={() => void save()}>
          <Check size={13} />
        </button>
        <button type="button" aria-label="取消重命名" disabled={saving} onClick={cancel}>
          <X size={13} />
        </button>
      </span>
    );
  }
  return (
    <span
      className={className}
      title={provider.name}
      tabIndex={disabled ? -1 : 0}
      role="button"
      aria-label={`${provider.name} 名称`}
      onDoubleClick={begin}
      onKeyDown={(event) => {
        if ((event.key === "Enter" || event.key === "F2") && !event.nativeEvent.isComposing) {
          event.preventDefault();
          begin();
        }
      }}
    >
      {provider.name}
      <Pencil className="provider-name-hint" size={11} aria-hidden="true" />
    </span>
  );
}
