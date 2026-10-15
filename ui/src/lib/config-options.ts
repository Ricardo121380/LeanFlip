import type { ToolId } from "./api";

export interface ConfigOption { key: string; label: string; on: unknown; off: unknown; also?: { key: string; on: unknown; off: unknown }[] }
const flag = (key: string, label: string, on: unknown = true, off: unknown = false): ConfigOption => ({ key, label, on, off });
export const commonOptions: Partial<Record<ToolId, ConfigOption[]>> = {
  claude: [
    { ...flag("attribution.commit", "hideAttribution", "", undefined), also: [{ key: "attribution.pr", on: "", off: undefined }] },
    flag("env.CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "agentTeams", "1", "0"),
    flag("env.DISABLE_AUTOUPDATER", "disableUpdates", "1", "0"),
  ],
  codex: [flag("features.multi_agent", "multiAgent"), flag("features.shell_snapshot", "shellSnapshot"), flag("tui.notifications", "notifications")],
  gemini: [flag("general.enableAutoUpdate", "autoUpdates"), flag("general.vimMode", "vimMode"), flag("general.checkpointing.enabled", "checkpointing")],
  grokbuild: [flag("cli.auto_update", "autoUpdates")],
  // `true` changes meaning across OpenCode versions. `notify` never requests installation.
  opencode: [flag("autoupdate", "checkUpdates", "notify", false), flag("share", "disableSharing", "disabled", "manual"), flag("compaction.auto", "autoCompact"), flag("compaction.prune", "pruneTools")],
  openclaw: [flag("agents.defaults.compaction.enabled", "autoCompact"), flag("agents.defaults.compaction.memoryFlush.enabled", "memoryFlush")],
  hermes: [flag("compression.enabled", "autoCompact"), flag("memory.memory_enabled", "memoryEnabled"), flag("memory.user_profile_enabled", "userProfile")],
  pi: [flag("compaction.enabled", "autoCompact"), flag("retry.enabled", "retryEnabled")],
  mcode: [flag("toolResultCompaction.enabled", "pruneTools")],
};
export const providerOptions: Partial<Record<ToolId, ConfigOption[]>> = {
  claude: [flag("env.ENABLE_TOOL_SEARCH", "toolSearch", "true", "false"), flag("env.CLAUDE_CODE_ALWAYS_ENABLE_EFFORT", "effort", "1", "0"), flag("env.CLAUDE_CODE_DISABLE_ARTIFACT", "disableArtifact", "1", "0")],
};

export function openCodeV2(config: unknown): boolean {
  return typeof atPath(config, "update") !== "undefined" || typeof atPath(config, "permissions") !== "undefined"
    || typeof atPath(config, "providers") !== "undefined" || String(atPath(config, "$schema") ?? "").includes("/v2/")
    || atPath(config, "compaction.buffer") !== undefined || atPath(config, "compaction.keep") !== undefined;
}
export function commonOptionsFor(tool: ToolId, config: unknown, v2 = false): ConfigOption[] {
  const options = commonOptions[tool] ?? [];
  return tool === "opencode" && (v2 || openCodeV2(config))
    ? options.filter((o) => o.key !== "autoupdate" && o.key !== "compaction.prune") : options;
}

export type ConfigPath = string | readonly (string | number)[];
export const pathParts = (path: ConfigPath): readonly (string | number)[] => typeof path === "string" ? path.split(".") : path;
export function atPath(value: unknown, path: ConfigPath): unknown {
  return pathParts(path).reduce<unknown>((v, key) => v && typeof v === "object" ? (v as Record<string, unknown>)[key] : undefined, value);
}
export function setPath(value: Record<string, unknown>, path: ConfigPath, next: unknown): Record<string, unknown> {
  const edit = (node: unknown, parts: readonly (string | number)[]): unknown => {
    const [key, ...rest] = parts;
    if (key === undefined || ["__proto__", "constructor", "prototype"].includes(String(key))) throw new Error("Invalid configuration key");
    if (typeof key === "number") {
      if (!Array.isArray(node) || !Number.isInteger(key) || key < 0 || key >= node.length) throw new Error("Invalid model index");
      const result = [...node]; result[key] = rest.length ? edit(node[key], rest) : next;
      return result;
    }
    if (node !== undefined && (!node || typeof node !== "object" || Array.isArray(node))) throw new Error(`Invalid object: ${key}`);
    const result = { ...(node as Record<string, unknown> | undefined) };
    const child = rest.length ? edit(result[key], rest) : next;
    if (child === undefined || (rest.length && child && typeof child === "object" && !Array.isArray(child) && !Object.keys(child).length)) delete result[key];
    else result[key] = child;
    return result;
  };
  return edit(value, pathParts(path)) as Record<string, unknown>;
}
export function optionState(value: unknown, option: ConfigOption): "default" | "on" | "off" | "custom" {
  const entries = [option, ...(option.also ?? [])];
  if (entries.every((entry) => atPath(value, entry.key) === undefined)) return "default";
  if (entries.every((entry) => Object.is(atPath(value, entry.key), entry.on))) return "on";
  if (entries.every((entry) => Object.is(atPath(value, entry.key), entry.off))) return "off";
  return "custom";
}
export function applyOption(value: Record<string, unknown>, option: ConfigOption, checked: boolean | "default"): Record<string, unknown> {
  return [option, ...(option.also ?? [])].reduce((v, entry) => setPath(v, entry.key, checked === "default" ? undefined : checked ? entry.on : entry.off), value);
}
