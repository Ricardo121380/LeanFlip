// 场景的纯函数：当前场景、预览分组、编辑草稿。
import type { Provider, Scene, SceneList, SceneTool, SceneView, ToolId, ToolKind, ToolPreview, ToolProviders } from "./api";

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
