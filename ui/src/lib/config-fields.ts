import type { ToolId } from "./api";
import { atPath } from "./config-options";

export type ConfigField = { key: string; label: string; hint?: string } & (
  | { kind: "number"; min: number; max?: number; string?: boolean }
  | { kind: "select"; choices: [value: string, label: string][] }
);

export const commonFields: Partial<Record<ToolId, ConfigField[]>> = {
  claude: [
    { key: "permissions.defaultMode", label: "permissionMode", hint: "permissionHint", kind: "select", choices: [["default", "manual"], ["acceptEdits", "acceptEdits"], ["plan", "plan"], ["auto", "autoPermission"], ["dontAsk", "dontAsk"], ["bypassPermissions", "bypass"]] },
    { key: "promptCacheTtl", label: "cacheTtl", hint: "cacheHint", kind: "select", choices: [["5m", "fiveMinutes"], ["1h", "oneHour"]] },
    { key: "subagentPromptCacheTtl", label: "subagentCacheTtl", hint: "cacheHint", kind: "select", choices: [["5m", "fiveMinutes"], ["1h", "oneHour"]] },
  ],
  codex: [
    { key: "approval_policy", label: "approvalPolicy", hint: "approvalHint", kind: "select", choices: [["on-request", "onRequest"], ["never", "neverApprove"]] },
    { key: "sandbox_mode", label: "sandboxMode", hint: "sandboxHint", kind: "select", choices: [["read-only", "readOnly"], ["workspace-write", "workspaceWrite"], ["danger-full-access", "fullAccess"]] },
    { key: "tool_output_token_limit", label: "toolOutputLimit", hint: "toolOutputHint", kind: "number", min: 0 },
  ],
};

export const providerFields: Partial<Record<ToolId, ConfigField[]>> = {
  codex: [
    { key: "model_context_window", label: "contextWindow", hint: "contextHint", kind: "number", min: 1 },
    { key: "model_auto_compact_token_limit", label: "compactLimit", hint: "compactHint", kind: "number", min: 1 },
    { key: "web_search", label: "searchMode", kind: "select", choices: [["disabled", "disabled"], ["cached", "cached"], ["indexed", "indexed"], ["live", "live"]] },
    { key: "model_verbosity", label: "verbosity", hint: "verbosityHint", kind: "select", choices: [["low", "concise"], ["medium", "balanced"], ["high", "detailed"]] },
  ],
  claude: [
    { key: "env.CLAUDE_CODE_MAX_CONTEXT_TOKENS", label: "contextWindow", hint: "claudeContextHint", kind: "number", min: 1, string: true },
    { key: "env.CLAUDE_CODE_AUTO_COMPACT_WINDOW", label: "compactWindow", hint: "claudeCompactHint", kind: "number", min: 1, string: true },
    { key: "env.CLAUDE_CODE_MAX_OUTPUT_TOKENS", label: "maxOutput", hint: "claudeOutputHint", kind: "number", min: 1, max: 128000, string: true },
  ],
};

// Invalid input stays in the draft and is reported before saving; never truncate
// "20abc" with parseInt, or silently turn a partially entered number into zero.
export function fieldInput(field: ConfigField, input: string): string | number | undefined {
  const text = input.trim();
  if (!text) return undefined;
  if (field.kind === "number" && !field.string && /^[+-]?\d+$/.test(text) && Number.isSafeInteger(Number(text))) return Number(text);
  return text;
}

export interface FieldIssue { label: string; reason: "integer" | "compactAboveWindow"; min?: number; max?: number }
export function fieldIssue(field: ConfigField, config: unknown): FieldIssue | null {
  if (field.kind !== "number") return null;
  const value = atPath(config, field.key);
  if (value === undefined) return null;
  const number = field.string && typeof value === "string" && /^\d+$/.test(value) ? Number(value) : value;
  if ((field.string && typeof value !== "string") || typeof number !== "number" || !Number.isSafeInteger(number) || number < field.min || (field.max !== undefined && number > field.max)) {
    return { label: field.label, reason: "integer", min: field.min, max: field.max };
  }
  return null;
}

export function configFieldsIssue(tool: ToolId, scope: "common" | "provider", config: unknown): FieldIssue | null {
  for (const field of (scope === "common" ? commonFields : providerFields)[tool] ?? []) {
    const issue = fieldIssue(field, config);
    if (issue) return issue;
  }
  if (scope === "provider" && tool === "codex") {
    const window = atPath(config, "model_context_window"), limit = atPath(config, "model_auto_compact_token_limit");
    if (typeof window === "number" && typeof limit === "number" && limit > window) return { label: "compactLimit", reason: "compactAboveWindow" };
  }
  return null;
}
