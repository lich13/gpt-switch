import {
  findNodeAtLocation,
  getNodeValue,
  parseTree,
  type Node,
  type ParseError,
} from "jsonc-parser";
export type Json =
  | null
  | boolean
  | number
  | string
  | Json[]
  | { [key: string]: Json };
export type Path = (string | number)[];
export type Field = {
  label: string;
  path: string[];
  kind: "text" | "model" | "boolean" | "select" | "integer";
  options?: [string, string][];
  override?: string[];
};
export const groups = ["模型", "权限", "偏好", "环境变量", "高级"] as const;
export const fields: Record<string, Field[]> = {
  模型: [
    {
      label: "默认模型",
      path: ["model"],
      override: ["env", "ANTHROPIC_MODEL"],
      kind: "model",
    },
    ...["OPUS", "SONNET", "HAIKU", "FABLE"].map(
      (id): Field => ({
        label: `${id[0]}${id.slice(1).toLowerCase()} 映射`,
        path: ["env", `ANTHROPIC_DEFAULT_${id}_MODEL`],
        kind: "model",
      }),
    ),
    {
      label: "推理强度",
      path: ["env", "CLAUDE_CODE_EFFORT_LEVEL"],
      kind: "select",
      options: ["auto", "low", "medium", "high", "xhigh", "max"].map((v) => [
        v,
        v === "auto" ? "模型默认" : v,
      ]),
    },
    {
      label: "最大输出 Token",
      path: ["env", "CLAUDE_CODE_MAX_OUTPUT_TOKENS"],
      kind: "integer",
    },
  ],
  权限: [
    {
      label: "默认权限模式",
      path: ["permissions", "defaultMode"],
      kind: "select",
      options: [
        ["default", "逐次确认"],
        ["acceptEdits", "允许编辑"],
        ["plan", "仅规划"],
        ["auto", "自动审核"],
        ["dontAsk", "不询问，未允许则拒绝"],
        ["bypassPermissions", "跳过权限确认"],
      ],
    },
    { label: "沙箱", path: ["sandbox", "enabled"], kind: "boolean" },
  ],
  偏好: [
    { label: "回复语言", path: ["language"], kind: "text" },
    ...[
      ["思考", "alwaysThinkingEnabled"],
      ["自动记忆", "autoMemoryEnabled"],
      ["操作提示", "spinnerTipsEnabled"],
      ["耗时显示", "showTurnDuration"],
      ["减少动效", "prefersReducedMotion"],
    ].map(([label, key]): Field => ({ label, path: [key], kind: "boolean" })),
    { label: "提交署名", path: ["attribution", "commit"], kind: "text" },
    { label: "PR 署名", path: ["attribution", "pr"], kind: "text" },
  ],
};
export function tree(text: string): Node {
  const errors: ParseError[] = [];
  const node = parseTree(text, errors, {
    allowTrailingComma: false,
    disallowComments: true,
    allowEmptyContent: false,
  });
  if (!node || errors.length || node.type !== "object")
    throw new Error("请先修正 JSON 格式");
  const visit = (n: Node) => {
    if (n.type === "object") {
      const keys = new Set<string>();
      for (const p of n.children ?? []) {
        const key = p.children![0].value as string;
        if (keys.has(key)) throw new Error("配置包含重复字段");
        keys.add(key);
      }
    }
    n.children?.forEach(visit);
  };
  visit(node);
  return node;
}
export function get(text: string, path: Path): Json | undefined {
  const n = findNodeAtLocation(tree(text), path);
  return n ? (getNodeValue(n) as Json) : undefined;
}
export function patch(
  text: string,
  path: Path,
  value: Json | undefined,
): string {
  const root = tree(text);
  if (!path.length) throw new Error("不能替换配置根节点");
  const replace = (start: number, end: number, content: string) =>
    text.slice(0, start) + content + text.slice(end);
  const unit = text.match(/\n([\t ]+)"/u)?.[1] ?? "  ";
  const eol = text.includes("\r\n") ? "\r\n" : "\n";
  const indentation = (offset: number) =>
    text
      .slice(text.lastIndexOf("\n", offset - 1) + 1, offset)
      .match(/^[\t ]*/u)![0];
  const serialize = (v: Json, indent: string, multiline: boolean) =>
    multiline
      ? JSON.stringify(v, null, unit).replace(/\n/g, eol + indent)
      : JSON.stringify(v);
  const current = findNodeAtLocation(root, path);
  if (current) {
    if (value !== undefined)
      return replace(
        current.offset,
        current.offset + current.length,
        serialize(
          value,
          indentation(current.offset),
          text
            .slice(current.offset, current.offset + current.length)
            .includes("\n"),
        ),
      );
    const entry =
      current.parent?.type === "property" ? current.parent : current;
    const entries = entry.parent!.children!;
    const index = entries.indexOf(entry);
    if (entries[index + 1])
      return replace(entry.offset, entries[index + 1].offset, "");
    if (index > 0)
      return replace(
        entries[index - 1].offset + entries[index - 1].length,
        entry.offset + entry.length,
        "",
      );
    return replace(entry.offset, entry.offset + entry.length, "");
  }
  if (value === undefined) return text;
  let parent = root,
    depth = 0;
  while (depth < path.length - 1) {
    const next = findNodeAtLocation(root, path.slice(0, depth + 1));
    if (!next) break;
    parent = next;
    depth++;
  }
  if (parent.type !== "object" && parent.type !== "array")
    throw new Error("请先修正父字段类型");
  let nested: Json = value;
  for (let i = path.length - 1; i > depth; i--) {
    if (typeof path[i] === "number") {
      if (path[i] !== 0) throw new Error("列表索引无效");
      nested = [nested];
    } else nested = { [path[i]]: nested };
  }
  const entries = parent.children ?? [];
  if (parent.type === "array" && path[depth] !== entries.length)
    throw new Error("列表索引无效");
  const multiline = text
    .slice(parent.offset, parent.offset + parent.length)
    .includes("\n");
  const indent = entries.length
    ? indentation(entries[0].offset)
    : indentation(parent.offset) + unit;
  const content =
    (parent.type === "object" ? JSON.stringify(path[depth]) + ": " : "") +
    serialize(nested, indent, multiline);
  const last = entries.at(-1);
  const offset = last ? last.offset + last.length : parent.offset + 1;
  return replace(
    offset,
    offset,
    (last ? "," : "") + (multiline ? eol + indent : last ? " " : "") + content,
  );
}
export const sensitive = (path: Path) =>
  path.some((p) =>
    /password|secret|api[_-]?key|authorization|credential|(?:^|[_-])(?:auth[_-]?)?token(?:$|[_-])|accessToken|refreshToken|idToken/i.test(
      String(p),
    ),
  );
export const managed = (path: Path) =>
  path[0] === "env" &&
  (path[1] === "ANTHROPIC_BASE_URL" || path[1] === "ANTHROPIC_AUTH_TOKEN");
export function redact(value: Json, path: Path = []): Json {
  if (sensitive(path)) return "••••••••";
  if (Array.isArray(value)) return value.map((v, i) => redact(v, [...path, i]));
  if (value && typeof value === "object")
    return Object.fromEntries(
      Object.entries(value).map(([k, v]) => [k, redact(v, [...path, k])]),
    );
  return value;
}
export function changes(
  before: string,
  after: string,
): { path: string; before: string; after: string }[] {
  const result: { path: string; before: string; after: string }[] = [];
  const walk = (a: Json | undefined, b: Json | undefined, path: Path) => {
    if (JSON.stringify(a) === JSON.stringify(b)) return;
    if (
      a &&
      b &&
      typeof a === "object" &&
      typeof b === "object" &&
      !Array.isArray(a) &&
      !Array.isArray(b)
    ) {
      for (const k of new Set([...Object.keys(a), ...Object.keys(b)]))
        walk(a[k], b[k], [...path, k]);
    } else {
      const display = (v: Json | undefined) =>
        v === undefined ? "未设置" : JSON.stringify(redact(v, path));
      result.push({
        path: path.join("."),
        before: display(a),
        after: display(b),
      });
    }
  };
  walk(get(before, []), get(after, []), []);
  return result;
}
