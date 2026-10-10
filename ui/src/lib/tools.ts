import type { ToolId } from "./api";

export interface ToolMeta {
  id: ToolId;
  label: string;
  kind: "switch" | "coexist";
  /** 卡片编辑器的形态。 */
  editor: "json" | "codex";
  color: string;
}

export const TOOLS: ToolMeta[] = [
  { id: "claude", label: "Claude Code", kind: "switch", editor: "json", color: "#D97757" },
  { id: "codex", label: "Codex", kind: "switch", editor: "codex", color: "#10A37F" },
  { id: "gemini", label: "Gemini CLI", kind: "switch", editor: "json", color: "#4285F4" },
  { id: "grokbuild", label: "Grok Build", kind: "switch", editor: "codex", color: "#111827" },
  { id: "claude-desktop", label: "Claude Desktop", kind: "switch", editor: "json", color: "#C2410C" },
  { id: "opencode", label: "OpenCode", kind: "coexist", editor: "json", color: "#334155" },
  { id: "openclaw", label: "OpenClaw", kind: "coexist", editor: "json", color: "#DC2626" },
  { id: "hermes", label: "Hermes", kind: "coexist", editor: "json", color: "#7C3AED" },
  { id: "pi", label: "Pi", kind: "coexist", editor: "json", color: "#0891B2" },
  { id: "mcode", label: "MCode", kind: "coexist", editor: "json", color: "#65A30D" },
];

export function toolMeta(id: ToolId): ToolMeta {
  const meta = TOOLS.find((t) => t.id === id);
  if (!meta) throw new Error(`unknown tool ${id}`);
  return meta;
}
