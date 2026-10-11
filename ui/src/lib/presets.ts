// cc-switch 的预设（MIT，Copyright (c) 2025 Jason Young），由 scripts/build-presets.mjs
// 裁剪生成：去掉了需要本地路由 / 专用登录的条目。
//
// 每个工具的预设是单独的 chunk，第一次打开「添加供应商」时才加载（主包里没有预设）。
import type { ToolId } from "./api";

export interface Preset {
  name: string;
  websiteUrl?: string;
  apiKeyUrl?: string;
  settingsConfig: Record<string, unknown>;
  isOfficial?: boolean;
  category?: string;
  apiKeyField?: string;
}

const loaders = import.meta.glob<{ default: Preset[] }>("../presets/*.json");
const cache = new Map<ToolId, Promise<Preset[]>>();

export function loadPresets(tool: ToolId): Promise<Preset[]> {
  const cached = cache.get(tool);
  if (cached) return cached;
  const load = loaders[`../presets/${tool}.json`];
  const pending: Promise<Preset[]> = load ? load().then((m) => m.default) : Promise.resolve([]);
  // 加载失败时不缓存，下次重试。
  pending.catch(() => cache.delete(tool));
  cache.set(tool, pending);
  return pending;
}

export function filterPresets(list: Preset[], query: string): Preset[] {
  const q = query.trim().toLowerCase();
  if (!q) return list;
  return list.filter(
    (p) => p.name.toLowerCase().includes(q) || (p.websiteUrl ?? "").toLowerCase().includes(q),
  );
}
