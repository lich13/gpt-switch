import { describe, it, expect } from "vitest";
import { patch, get, changes, sensitive, fields } from "./claude-settings";

describe("Claude lossless configuration edits", () => {
  const original =
    '{\r\n\t"model" : "before",\r\n\t"unknown"  : {"zero":0, "off":false},\r\n\t"env": {"KEEP" : ""}\r\n}\r\n';
  it("changes only the selected scalar and preserves CRLF, spacing and unknown content", () => {
    expect(patch(original, ["model"], "after")).toBe(
      original.replace('"before"', '"after"'),
    );
    const added = patch(original, ["env", "CLAUDE_CODE_EFFORT_LEVEL"], "max");
    expect(added).toContain('"unknown"  : {"zero":0, "off":false}');
    expect(added).toContain('"KEEP" : ""');
    expect(added.replaceAll("\r\n", "")).not.toContain("\n");
    expect(get(added, ["env", "CLAUDE_CODE_EFFORT_LEVEL"])).toBe("max");
  });
  it("deletes first, middle and last fields without formatting remaining values", () => {
    for (const key of ["model", "unknown", "env"]) {
      const result = patch(original, [key], undefined);
      expect(get(result, [key])).toBeUndefined();
      for (const keep of ["model", "unknown", "env"].filter((k) => k !== key))
        expect(get(result, [keep])).toEqual(get(original, [keep]));
      if (key !== "unknown")
        expect(result).toContain('"unknown"  : {"zero":0, "off":false}');
    }
  });
  it("creates missing structure, preserves explicit zero, off and empty, and edits arrays", () => {
    let text = patch("{}", ["permissions", "allow"], ["Read", "Bash(ls)"]);
    text = patch(text, ["permissions", "allow", 0], undefined);
    text = patch(text, ["permissions", "allow", 1], "Write");
    expect(get(text, ["permissions", "allow"])).toEqual(["Bash(ls)", "Write"]);
    for (const value of [0, false, "", null]) {
      const next = patch(text, ["future"], value);
      expect(get(next, ["future"])).toBe(value);
      expect(
        get(patch(next, ["future"], undefined), ["future"]),
      ).toBeUndefined();
    }
  });
  it("rejects invalid JSON and duplicate keys without changing the input", () => {
    for (const text of [
      '{"x":1,"x":2}',
      '{"x":{"a":0,"a":1}}',
      '{"x":1,}',
      "[]",
      '{"env":',
    ])
      expect(() => patch(text, ["model"], "x")).toThrow();
  });
  it("redacts nested credentials in changes while keeping nonsecret token limits", () => {
    const next =
      '{"env":{"ANTHROPIC_AUTH_TOKEN":"private-fixture","CLAUDE_CODE_MAX_OUTPUT_TOKENS":"0"},"custom":{"password":"private-fixture"}}';
    const serialized = JSON.stringify(changes("{}", next));
    expect(serialized).not.toContain("private-fixture");
    expect(serialized).toContain("••••••••");
    expect(sensitive(["env", "CLAUDE_CODE_MAX_OUTPUT_TOKENS"])).toBe(false);
    expect(fields.模型.find((f) => f.label === "默认模型")?.override).toEqual([
      "env",
      "ANTHROPIC_MODEL",
    ]);
  });
});
