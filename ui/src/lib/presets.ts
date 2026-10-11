// LeanFlip starter templates, maintained in scripts/build-presets.mjs.
// Loaded on demand by tool. Existing user cards are independent of this catalogue.
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
