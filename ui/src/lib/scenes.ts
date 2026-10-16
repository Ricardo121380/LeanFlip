// 场景的纯函数：当前场景、预览分组、编辑草稿、快捷键和提醒规则。
import type { KnownSceneRule, Provider, Scene, SceneList, SceneRule, SceneTool, SceneView, ToolId, ToolKind, ToolPreview, ToolProviders } from "./api";

/**
 * 首页按钮上显示的场景：最近应用的那个（一致或已偏离），否则任意一个和现在完全一致的。
 * 都没有时返回 `null`。
 */
export function currentScene(list: SceneList | undefined): { scene: SceneView; drifted: boolean } | null {
  if (!list) return null;
  const last = list.scenes.find((scene) => scene.id === list.last);
  if (last && last.status !== "idle") return { scene: last, drifted: last.status === "drifted" };
  const active = list.scenes.find((scene) => scene.status === "active");
  return active ? { scene: active, drifted: false } : null;
}

/** 预览里要处理的工具（有步骤、有缺失或读不了）和已经一致的工具。 */
export function splitPreview(tools: ToolPreview[]): { changing: ToolPreview[]; inSync: ToolPreview[] } {
  const changing = tools.filter((tool) => tool.steps.length > 0 || tool.missing.length > 0 || tool.error);
  return { changing, inSync: tools.filter((tool) => !changing.includes(tool)) };
}

/** 这个工具在预览里有没有可以执行的步骤（读不了配置的不能应用）。 */
export function applicable(tool: ToolPreview): boolean {
  return tool.steps.length > 0 && !tool.error;
}

/** 编辑场景时新勾选一个工具：用它现在的状态当默认值。 */
export function defaultEntry(kind: ToolKind, data: ToolProviders | undefined): SceneTool | null {
  const providers = data?.providers ?? [];
  if (providers.length === 0) return null;
  if (kind === "switch") return { current: data?.selected ?? data?.current ?? providers[0].id };
  return { enabled: providers.filter((p) => data?.inLive.includes(p.id)).map((p) => p.id) };
}

/** 场景里引用、但列表里已经没有的卡片 id。 */
export function missingIds(entry: SceneTool | undefined, providers: Provider[]): string[] {
  const ids = [...(entry?.current ? [entry.current] : []), ...(entry?.enabled ?? [])];
  return [...new Set(ids.filter((id) => !providers.some((p) => p.id === id)))];
}

/** 在并存式工具的启用列表里加入或去掉一张卡；Hermes 的默认供应商被去掉时一并清掉。 */
export function toggleEnabled(entry: SceneTool, id: string, on: boolean): SceneTool {
  const enabled = (entry.enabled ?? []).filter((x) => x !== id);
  if (on) enabled.push(id);
  const next: SceneTool = { ...entry, enabled };
  if (!on && next.current === id) delete next.current;
  return next;
}

/** 草稿里实际包含的工具，按工具列表的顺序。 */
export function sceneTools(scene: Scene, order: { id: ToolId }[]): ToolId[] {
  return order.map((tool) => tool.id).filter((id) => scene.tools[id] !== undefined);
}

/** macOS 用 ⌘，其他系统用 Ctrl。 */
export const MAC = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);

/** 场景列表里第 `index` 个（从 0 起）的快捷键文字；只有前 9 个有。 */
export function sceneShortcut(index: number, mac = MAC): string | null {
  if (index < 0 || index > 8) return null;
  return mac ? `⌘⇧${index + 1}` : `Ctrl+Shift+${index + 1}`;
}

/** 按键对应的场景序号（从 0 起）：⌘⇧1…9（其他系统 Ctrl+Shift）。不是场景快捷键时返回 -1。 */
export function shortcutIndex(event: Pick<KeyboardEvent, "code" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">, mac = MAC): number {
  if (!(mac ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey) || !event.shiftKey || event.altKey) return -1;
  const match = /^Digit([1-9])$/.exec(event.code);
  return match ? Number(match[1]) - 1 : -1;
}

/** `HH:MM` → 当天第几分钟。 */
export function parseTime(text: string): number | null {
  const match = /^(\d{2}):(\d{2})$/.exec(text);
  if (!match) return null;
  const [hour, minute] = [Number(match[1]), Number(match[2])];
  return hour < 24 && minute < 60 ? hour * 60 + minute : null;
}

export function isCidr(text: string): boolean {
  const [address, prefix, ...rest] = text.trim().split("/");
  if (rest.length || !address || !prefix || !/^\d{1,3}$/.test(prefix)) return false;
  const bits = Number(prefix);
  if (/^\d{1,3}(\.\d{1,3}){3}$/.test(address)) return bits <= 32 && address.split(".").every((part) => Number(part) <= 255);
  return address.includes(":") && /^[0-9a-fA-F:.]+$/.test(address) && bits <= 128;
}

export function isHost(text: string): boolean {
  return text.length > 0 && text.length <= 253 && !/^[.-]/.test(text) && /^[A-Za-z0-9.-]+$/.test(text) && text.split(".").every((label) => label.length > 0 && label.length <= 63);
}

const KNOWN = ["time", "subnet", "host"];

/** 认识的、字段完整的规则。 */
export function knownRule(rule: SceneRule): KnownSceneRule | null {
  if (!KNOWN.includes(rule.kind)) return null;
  if (rule.kind === "time" && Array.isArray(rule.days) && typeof rule.start === "string" && typeof rule.end === "string") return rule as KnownSceneRule;
  if (rule.kind === "subnet" && typeof rule.cidr === "string") return rule as KnownSceneRule;
  if (rule.kind === "host" && typeof rule.host === "string") return rule as KnownSceneRule;
  return null;
}

/** 规则能不能保存（和后端校验一致）。 */
export function validRule(rule: KnownSceneRule): boolean {
  if (rule.kind === "time") {
    const [start, end] = [parseTime(rule.start), parseTime(rule.end)];
    return rule.days.length > 0 && rule.days.every((day) => day >= 1 && day <= 7) && start !== null && end !== null && start !== end;
  }
  return rule.kind === "subnet" ? isCidr(rule.cidr) : isHost(rule.host);
}

type T = (key: string, options?: Record<string, unknown>) => string;

/** 规则的简短说明：`工作日 09:00–18:00`、`网段 10.0.0.0/8`、`intranet.example 能解析`。 */
export function ruleText(rule: SceneRule, t: T): string {
  const known = knownRule(rule);
  if (!known) return t("scene.rule.unknown");
  if (known.kind === "subnet") return t("scene.rule.subnetText", { cidr: known.cidr });
  if (known.kind === "host") return t("scene.rule.hostText", { host: known.host });
  const days = [...new Set(known.days)].sort((a, b) => a - b);
  const key = days.join(",");
  const when =
    key === "1,2,3,4,5,6,7" ? t("scene.rule.everyDay") : key === "1,2,3,4,5" ? t("scene.rule.weekdays") : key === "6,7" ? t("scene.rule.weekends") : days.map((day) => t(`scene.rule.day${day}`)).join(t("common.separator"));
  const overnight = (parseTime(known.end) ?? 0) <= (parseTime(known.start) ?? 0);
  return t(overnight ? "scene.rule.overnight" : "scene.rule.timeText", { when, start: known.start, end: known.end });
}

