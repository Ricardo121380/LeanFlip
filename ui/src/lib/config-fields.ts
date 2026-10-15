import type { ToolId } from "./api";
import { atPath, openCodeV2, type ConfigPath } from "./config-options";

export type ConfigField = { key: string; path?: ConfigPath; model?: string; label: string; hint?: string } & (
  | { kind: "number"; min: number; max?: number; string?: boolean; decimal?: boolean; exclusiveMin?: boolean }
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
  gemini: [
    { key: "general.defaultApprovalMode", label: "permissionMode", hint: "geminiPermissionHint", kind: "select", choices: [["default", "manual"], ["auto_edit", "acceptEdits"], ["plan", "plan"]] },
    { key: "model.compressionThreshold", label: "compressionRatio", hint: "ratioHint", kind: "number", min: 0, max: 1, decimal: true, exclusiveMin: true },
    { key: "model.maxSessionTurns", label: "maxTurns", hint: "geminiTurnsHint", kind: "number", min: -1 },
  ],
  grokbuild: [
    { key: "session.auto_compact_threshold_percent", label: "compressionPercent", hint: "grokCompactHint", kind: "number", min: 0, max: 100 },
    { key: "sandbox.profile", label: "sandboxMode", hint: "sandboxHint", kind: "select", choices: [["off", "fullAccess"], ["workspace", "workspaceWrite"], ["read-only", "readOnly"], ["strict", "strictSandbox"]] },
  ],
  opencode: [
    { key: "compaction.reserved", label: "compactBuffer", hint: "bufferHint", kind: "number", min: 0 },
  ],
  openclaw: [
    { key: "agents.defaults.compaction.mode", label: "compactMode", kind: "select", choices: [["default", "standard"], ["safeguard", "safeguard"]] },
    { key: "agents.defaults.compaction.keepRecentTokens", label: "keepRecent", hint: "keepRecentHint", kind: "number", min: 1 },
    { key: "agents.defaults.thinkingDefault", label: "thinkingLevel", hint: "thinkingHint", kind: "select", choices: [["off", "disabled"], ["minimal", "minimal"], ["low", "low"], ["medium", "balanced"], ["high", "high"]] },
    { key: "agents.defaults.maxConcurrent", label: "maxConcurrent", kind: "number", min: 1 },
    { key: "agents.defaults.timeoutSeconds", label: "timeoutSeconds", hint: "timeoutHint", kind: "number", min: 0 },
  ],
  hermes: [
    { key: "compression.threshold", label: "compressionRatio", hint: "hermesRatioHint", kind: "number", min: 0, max: 1, decimal: true, exclusiveMin: true },
    { key: "compression.threshold_tokens", label: "compactTokenCap", hint: "hermesCapHint", kind: "number", min: 1 },
  ],
  pi: [
    { key: "compaction.reserveTokens", label: "reserveOutput", hint: "piReserveHint", kind: "number", min: 0 },
    { key: "compaction.keepRecentTokens", label: "keepRecent", hint: "keepRecentHint", kind: "number", min: 0 },
    { key: "retry.maxRetries", label: "maxRetries", kind: "number", min: 0 },
    { key: "defaultThinkingLevel", label: "thinkingLevel", hint: "thinkingHint", kind: "select", choices: [["off", "disabled"], ["minimal", "minimal"], ["low", "low"], ["medium", "balanced"], ["high", "high"], ["xhigh", "xhigh"], ["max", "max"]] },
  ],
  mcode: [
    { key: "permissionMode", label: "permissionMode", hint: "mcodePermissionHint", kind: "select", choices: [["default", "manual"], ["auto", "autoPermission"], ["bypassPermissions", "bypass"]] },
    { key: "toolResultCompaction.maxInlineKiB", label: "toolInlineKiB", hint: "toolTrimHint", kind: "number", min: 1, max: 8796093022207 },
    { key: "toolResultCompaction.keepRecentRounds", label: "keepToolRounds", kind: "number", min: 1 },
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
    { key: "env.CLAUDE_CODE_AUTO_COMPACT_WINDOW", label: "compactWindow", hint: "claudeCompactHint", kind: "number", min: 100000, max: 1000000, string: true },
    { key: "env.CLAUDE_CODE_MAX_OUTPUT_TOKENS", label: "maxOutput", hint: "claudeOutputHint", kind: "number", min: 1, string: true },
  ],
};

export const fieldPath = (field: ConfigField): ConfigPath => field.path ?? field.key;
export function commonFieldsFor(tool: ToolId, config: unknown, v2 = false): ConfigField[] {
  if (tool !== "opencode") return commonFields[tool] ?? [];
  if (v2 || openCodeV2(config)) return [
    { key: "update", label: "updatePolicy", hint: "openCodeUpdateHint", kind: "select", choices: [["disable", "disabled"], ["notify", "notifyOnly"]] },
    { key: "compaction.buffer", label: "compactBuffer", hint: "bufferHint", kind: "number", min: 0 },
    { key: "compaction.keep.tokens", label: "keepRecent", hint: "keepRecentHint", kind: "number", min: 0 },
  ];
  const permission = atPath(config, "permission");
  if (permission !== undefined && typeof permission !== "string" && (!permission || typeof permission !== "object" || Array.isArray(permission))) return commonFields.opencode!;
  // A scalar policy applies to every tool. Expanding it to a map would silently
  // change the other tools' policy, so preserve that representation.
  const choices: [string, string][] = [["ask", "manual"], ["allow", "allow"], ["deny", "deny"]];
  return [...commonFields.opencode!, ...(typeof permission === "string"
    ? [{ key: "permission", label: "allToolsPermission", kind: "select" as const, choices }]
    : ["read", "edit", "bash", "webfetch"].map((tool) => ({ key: `permission.${tool}`, label: `permission_${tool}`, kind: "select" as const, choices })))];
}

export function providerFieldsFor(tool: ToolId, config: unknown): ConfigField[] {
  if (tool === "grokbuild") {
    const models = atPath(config, "model");
    if (!models || typeof models !== "object" || Array.isArray(models)) return [];
    const selected = atPath(config, "models.default");
    const id = typeof selected === "string" ? selected : Object.keys(models).length === 1 ? Object.keys(models)[0] : undefined;
    if (!id || !(models as Record<string, unknown>)[id]) return [];
    return [
      { key: "context_window", path: ["model", id, "context_window"], model: id, label: "contextWindow", hint: "contextHint", kind: "number", min: 1 },
      { key: "max_completion_tokens", path: ["model", id, "max_completion_tokens"], model: id, label: "maxOutput", hint: "modelOutputHint", kind: "number", min: 1 },
      { key: "auto_compact_threshold_percent", path: ["model", id, "auto_compact_threshold_percent"], model: id, label: "compressionPercent", hint: "grokModelCompactHint", kind: "number", min: 0, max: 100 },
    ];
  }
  if (tool === "opencode" || tool === "mcode") {
    const models = atPath(config, "models");
    if (!models || typeof models !== "object" || Array.isArray(models)) return [];
    return Object.entries(models).flatMap(([id, model]) => !model || typeof model !== "object" || Array.isArray(model) ? [] : [
      { key: JSON.stringify([id, "context"]), path: ["models", id, "limit", "context"], model: id, label: "contextWindow", hint: "contextHint", kind: "number" as const, min: 1 },
      { key: JSON.stringify([id, "output"]), path: ["models", id, "limit", "output"], model: id, label: "maxOutput", hint: "modelOutputHint", kind: "number" as const, min: 1 },
    ]);
  }
  if (tool === "openclaw") {
    const models = atPath(config, "models");
    return Array.isArray(models) ? models.flatMap((model, index) => model && typeof model === "object" && typeof model.id === "string" ? [
      { key: `context-${index}`, path: ["models", index, "contextWindow"], model: model.id, label: "contextWindow", hint: "contextHint", kind: "number" as const, min: 1 },
    ] : []) : [];
  }
  return providerFields[tool] ?? [];
}

// Invalid input stays in the draft and is reported before saving; never truncate
// "20abc" with parseInt, or silently turn a partially entered number into zero.
export function fieldInput(field: ConfigField, input: string): string | number | undefined {
  const text = input.trim();
  if (!text) return undefined;
  if (field.kind === "number" && !field.string && (field.decimal ? /^[+-]?(?:\d+\.?\d*|\.\d+)$/ : /^[+-]?\d+$/).test(text) && Number.isFinite(Number(text)) && Math.abs(Number(text)) <= Number.MAX_SAFE_INTEGER) return Number(text);
  return text;
}

export interface FieldIssue { label: string; reason: "integer" | "decimal" | "compactAboveWindow"; min?: number; max?: number }
export function fieldIssue(field: ConfigField, config: unknown, before?: unknown): FieldIssue | null {
  if (field.kind !== "number") return null;
  const value = atPath(config, fieldPath(field));
  if (value === undefined || (before !== undefined && Object.is(value, atPath(before, fieldPath(field))))) return null;
  // Hermes uses null as an explicit opt-out for this optional absolute cap.
  if (field.key === "compression.threshold_tokens" && value === null) return null;
  const number = field.string && typeof value === "string" && /^\d+$/.test(value) ? Number(value) : value;
  if ((field.string && typeof value !== "string") || typeof number !== "number" || !(field.decimal ? Number.isFinite(number) : Number.isSafeInteger(number)) || number < field.min || (field.exclusiveMin && number === field.min) || (field.max !== undefined && number > field.max)) {
    return { label: field.label, reason: field.decimal ? "decimal" : "integer", min: field.min, max: field.max };
  }
  return null;
}

export function configFieldsIssue(tool: ToolId, scope: "common" | "provider", config: unknown, before?: unknown, v2 = false): FieldIssue | null {
  for (const field of scope === "common" ? commonFieldsFor(tool, config, v2) : providerFieldsFor(tool, config)) {
    const issue = fieldIssue(field, config, before);
    if (issue) return issue;
  }
  if (scope === "provider" && tool === "codex") {
    const window = atPath(config, "model_context_window"), limit = atPath(config, "model_auto_compact_token_limit");
    if (typeof window === "number" && typeof limit === "number" && limit > window) return { label: "compactLimit", reason: "compactAboveWindow" };
  }
  return null;
}
