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
  opencode: [flag("autoupdate", "autoUpdates"), flag("share", "disableSharing", "disabled", "manual")],
};
export const providerOptions: Partial<Record<ToolId, ConfigOption[]>> = {
  claude: [flag("env.ENABLE_TOOL_SEARCH", "toolSearch", "true", "false"), flag("env.CLAUDE_CODE_ALWAYS_ENABLE_EFFORT", "effort", "1", "0"), flag("env.CLAUDE_CODE_DISABLE_ARTIFACT", "disableArtifact", "1", "0")],
  codex: [flag("disable_response_storage", "disableStorage"), flag("web_search", "webSearch", "live", "disabled")],
};
export function atPath(value: unknown, path: string): unknown {
  return path.split(".").reduce<unknown>((v, key) => v && typeof v === "object" ? (v as Record<string, unknown>)[key] : undefined, value);
}
export function setPath(value: Record<string, unknown>, path: string, next: unknown): Record<string, unknown> {
  const result = { ...value };
  const [key, ...rest] = path.split(".");
  if (["__proto__", "constructor", "prototype"].includes(key)) throw new Error("Invalid configuration key");
  if (rest.length) {
    const previous = value[key];
    if (previous !== undefined && (!previous || typeof previous !== "object" || Array.isArray(previous))) throw new Error(`Invalid object: ${key}`);
    const child = setPath((previous ?? {}) as Record<string, unknown>, rest.join("."), next);
    if (Object.keys(child).length) result[key] = child;
    else delete result[key];
  } else if (next === undefined) delete result[key];
  else result[key] = next;
  return result;
}
export function optionValue(value: unknown, option: ConfigOption): boolean {
  return Object.is(atPath(value, option.key), option.on) && (option.also ?? []).every((v) => Object.is(atPath(value, v.key), v.on));
}
export function applyOption(value: Record<string, unknown>, option: ConfigOption, checked: boolean | "default"): Record<string, unknown> {
  return [option, ...(option.also ?? [])].reduce((v, entry) => setPath(v, entry.key, checked === "default" ? undefined : checked ? entry.on : entry.off), value);
}
/** Existing boolean preferences are always editable, even when no documented shortcut exists. */
export function existingOptions(value: Record<string, unknown>, known: ConfigOption[]): ConfigOption[] {
  const result: ConfigOption[] = [];
  const walk = (object: Record<string, unknown>, parent: string, depth: number) => {
    for (const [key, value] of Object.entries(object)) {
      if (key.includes(".")) continue;
      const path = parent ? `${parent}.${key}` : key;
      if (typeof value === "boolean" && !known.some((o) => o.key === path)) result.push(flag(path, path));
      else if (depth < 2 && value && typeof value === "object" && !Array.isArray(value)) walk(value as Record<string, unknown>, path, depth + 1);
    }
  };
  walk(value, "", 0);
  return result;
}
