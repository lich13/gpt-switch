import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { StreamLanguage } from "@codemirror/language";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { linter, lintGutter, type Diagnostic } from "@codemirror/lint";
import { EditorView, keymap } from "@codemirror/view";
import { Save, RotateCcw, FileCode2, AlertTriangle, Check } from "lucide-react";
import { command } from "./bridge";
import { errorOf, type ConfigDocument, type AppError } from "./types";
export default function ConfigEditor({
  revision,
  home,
  theme,
  onDirty,
  onMessage,
}: {
  revision: string;
  home: string;
  theme: "light" | "dark";
  onDirty: (dirty: boolean) => void;
  onMessage: (s: string) => void;
}) {
  const [doc, setDoc] = useState<ConfigDocument | null>(null),
    [text, setText] = useState(""),
    [error, setError] = useState<AppError | null>(null),
    [busy, setBusy] = useState(false);
  const docRef = useRef(doc),
    textRef = useRef(text),
    saveRef = useRef<() => void>(() => {});
  docRef.current = doc;
  textRef.current = text;
  const dirty = !!doc && text !== doc.text,
    conflict = !!doc && revision !== doc.revision;
  const load = useCallback(async () => {
    try {
      const d = await command<ConfigDocument>("read_config");
      setDoc(d);
      setText(d.text);
      setError(null);
    } catch (e) {
      setError(errorOf(e));
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load, home]);
  useEffect(() => {
    onDirty(dirty);
  }, [dirty, onDirty]);
  useEffect(() => {
    if (conflict && !dirty) void load();
  }, [conflict, dirty, load]);
  const save = useCallback(async () => {
    const current = docRef.current;
    if (!current) return;
    setBusy(true);
    try {
      const d = await command<ConfigDocument>("save_config", {
        text: textRef.current,
        expectedRevision: current.revision,
      });
      setDoc(d);
      setError(null);
      onMessage("配置已保存，请重新打开 Codex");
    } catch (e) {
      setError(errorOf(e));
    } finally {
      setBusy(false);
    }
  }, [onMessage]);
  saveRef.current = () => {
    void save();
  };
  const extensions = useMemo(
    () => [
      StreamLanguage.define(toml),
      lintGutter(),
      keymap.of([
        {
          key: "Mod-s",
          run: () => {
            saveRef.current();
            return true;
          },
        },
      ]),
      EditorView.lineWrapping,
      linter(
        async (view) => {
          try {
            await command("validate_config", {
              text: view.state.doc.toString(),
            });
            return [];
          } catch (e) {
            const err = errorOf(e);
            if (!err.line) return [];
            const line = view.state.doc.line(
              Math.max(1, Math.min(err.line, view.state.doc.lines)),
            );
            const from = Math.min(
              line.to,
              line.from + Math.max(0, (err.column ?? 1) - 1),
            );
            return [
              {
                from,
                to: Math.min(view.state.doc.length, from + 1),
                severity: "error",
                message: err.message,
              },
            ] as Diagnostic[];
          }
        },
        { delay: 450 },
      ),
      EditorView.theme({
        "&": { height: "100%", background: "var(--canvas)" },
        ".cm-scroller": {
          fontFamily: "ui-monospace, SFMono-Regular, Consolas, monospace",
          fontSize: "13px",
        },
        ".cm-gutters": {
          background: "var(--canvas)",
          borderRight: "1px solid var(--line)",
        },
        ".cm-content": { padding: "16px 0" },
        ".cm-line": { padding: "0 16px" },
      }),
    ],
    [],
  );
  return (
    <section className="config-page">
      <div className="page-heading">
        <div>
          <div className="eyebrow">
            <FileCode2 size={14} />
            配置文件
          </div>
          <h1>config.toml</h1>
        </div>
        <button
          className="primary"
          onClick={() => void save()}
          disabled={!doc || !dirty || busy}
        >
          <Save size={15} />
          {busy ? "保存中…" : "保存"}
          <kbd>⌘ / Ctrl S</kbd>
        </button>
      </div>
      <div className="editor-toolbar">
        <span title={doc?.path}>{doc?.path ?? home + "/config.toml"}</span>
        <div className="inline">
          <span className={dirty ? "status amber" : "status"}>
            {dirty ? "未保存" : "已同步"}
          </span>
          <button
            className="icon-button"
            title="重新读取配置"
            aria-label="重新读取配置"
            onClick={() => {
              if (
                !dirty ||
                window.confirm("重新读取会丢弃未保存的草稿，是否继续？")
              )
                void load();
            }}
          >
            <RotateCcw size={15} />
          </button>
        </div>
      </div>
      {conflict && dirty && (
        <div className="banner warning">
          <AlertTriangle size={16} />
          <span>
            磁盘配置已变化，草稿已保留。请复制草稿，重新读取并合并后保存。
          </span>
        </div>
      )}
      {error && (
        <div className="banner error" role="alert">
          <AlertTriangle size={16} />
          <span>
            {error.message}
            {error.line ? `（第 ${error.line} 行）` : ""}
          </span>
        </div>
      )}
      <div className="editor-body">
        <CodeMirror
          value={text}
          onChange={(value) =>
            setText(
              docRef.current?.text.includes("\r\n")
                ? value.replace(/\r?\n/g, "\r\n")
                : value,
            )
          }
          theme={theme}
          height="100%"
          extensions={extensions}
          aria-label="TOML 编辑器"
          basicSetup={{
            foldGutter: true,
            autocompletion: false,
            highlightActiveLine: true,
          }}
        />
      </div>
      <div className="editor-footer">
        <span>
          <Check size={13} />
          保留原文、注释与未知字段
        </span>
        <span>TOML · UTF-8</span>
      </div>
    </section>
  );
}
