import type { ToolId } from "./api";

export interface ToolMeta {
  id: ToolId;
  label: string;
  kind: "switch" | "coexist";
  /** 供应商编辑器的形态。 */
  editor: "json" | "codex";
  color: string;
  /** 侧栏和标题里的两字母标识。 */
  glyph: string;
}

export const TOOLS: ToolMeta[] = [
  { id: "claude", label: "Claude Code", kind: "switch", editor: "json", color: "#D97757", glyph: "Cc" },
  { id: "codex", label: "Codex", kind: "switch", editor: "codex", color: "#10A37F", glyph: "Cx" },
  { id: "gemini", label: "Gemini CLI", kind: "switch", editor: "json", color: "#4285F4", glyph: "Ge" },
  { id: "grokbuild", label: "Grok Build", kind: "switch", editor: "codex", color: "#71717A", glyph: "Gb" },
  { id: "claude-desktop", label: "Claude Desktop", kind: "switch", editor: "json", color: "#C2410C", glyph: "Cd" },
  { id: "opencode", label: "OpenCode", kind: "coexist", editor: "json", color: "#64748B", glyph: "Oc" },
  { id: "openclaw", label: "OpenClaw", kind: "coexist", editor: "json", color: "#DC2626", glyph: "Ow" },
  { id: "hermes", label: "Hermes", kind: "coexist", editor: "json", color: "#7C3AED", glyph: "He" },
  { id: "pi", label: "Pi", kind: "coexist", editor: "json", color: "#0891B2", glyph: "Pi" },
  { id: "mcode", label: "MCode", kind: "coexist", editor: "json", color: "#65A30D", glyph: "Mc" },
];

export function toolMeta(id: ToolId): ToolMeta {
  const meta = TOOLS.find((t) => t.id === id);
  if (!meta) throw new Error(`unknown tool ${id}`);
  return meta;
}
