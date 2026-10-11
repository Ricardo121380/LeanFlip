import type { Provider } from "@/lib/api";

/** Claude Desktop 直连卡的模型菜单：`meta.claudeDesktopModelRoutes`。 */
export interface RouteRow {
  id: string;
  label: string;
  supports1m: boolean;
}

const KEY = "claudeDesktopModelRoutes";

/** 和 core 的 `is_claude_safe_model_id` 保持一致。 */
export function isClaudeSafeModelId(model: string): boolean {
  const n = model.trim().toLowerCase();
  if (n.includes("[1m]")) return false;
  const tail = n.startsWith("anthropic/claude-") ? n.slice("anthropic/claude-".length) : n.startsWith("claude-") ? n.slice(7) : null;
  if (tail === null) return false;
  return ["sonnet-", "opus-", "haiku-", "fable-"].some((p) => tail.startsWith(p) && tail.length > p.length);
}

export function routesOf(provider?: Pick<Provider, "meta">): RouteRow[] {
  const raw = provider?.meta?.[KEY];
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return [];
  return Object.entries(raw as Record<string, Record<string, unknown>>).map(([id, r]) => ({
    id,
    label: typeof r?.labelOverride === "string" ? r.labelOverride : "",
    supports1m: r?.supports1m === true,
  }));
}

/** 每行的问题：空 / 名字不合法 / 重复；没问题是 null。 */
export function routeErrors(rows: RouteRow[]): (null | "invalid" | "duplicate")[] {
  const seen = new Set<string>();
  return rows.map((r) => {
    const id = r.id.trim();
    if (!isClaudeSafeModelId(id)) return "invalid";
    if (seen.has(id)) return "duplicate";
    seen.add(id);
    return null;
  });
}

/** 写回 meta；直连模式不能映射，所以不写 `model`。空表就去掉这个键。 */
export function applyRoutes<T extends Pick<Provider, "meta">>(provider: T, rows: RouteRow[]): T {
  const meta = { ...(provider.meta ?? {}) };
  const routes: Record<string, Record<string, unknown>> = {};
  for (const r of rows) {
    const id = r.id.trim();
    if (!id) continue;
    const route: Record<string, unknown> = {};
    if (r.label.trim()) route.labelOverride = r.label.trim();
    if (r.supports1m) route.supports1m = true;
    routes[id] = route;
  }
  if (Object.keys(routes).length) meta[KEY] = routes;
  else delete meta[KEY];
  if (Object.keys(meta).length > 0) return { ...provider, meta };
  const { meta: _drop, ...rest } = provider;
  return rest as T;
}
