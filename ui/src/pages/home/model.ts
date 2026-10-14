// 首页的数据整理：每个工具一行，光路里的服务端点按地址合并。
import type { ProbeResult, Provider, ToolId, ToolProviders, ToolSummary } from "@/lib/api";
import { endpointOf, isOfficial } from "@/lib/cards";
import type { ToolMeta } from "@/lib/tools";

export type Latency = Record<string, ProbeResult | "pending">;
export const latencyKey = (tool: ToolId, id: string) => `${tool}:${id}`;

export interface Row {
  tool: ToolMeta;
  /** loading / error：还没拿到数据；missing：本机没有这个客户端，也没有供应商。 */
  state: "loading" | "error" | "missing" | "ready";
  data?: ToolProviders;
  /** 切换式：当前供应商；并存式：已写入客户端的，按写入顺序。 */
  active: Provider[];
  others: Provider[];
}

export function buildRows(tools: ToolMeta[], results: { data?: ToolProviders; isError: boolean }[], overview: ToolSummary[]): Row[] {
  return tools.map((tool, index) => {
    const { data, isError } = results[index] ?? { isError: false };
    const summary = overview.find((s) => s.tool === tool.id);
    if (!data) return { tool, state: isError ? "error" : "loading", active: [], others: [] };
    if (data.providers.length === 0 && summary && !summary.detected) return { tool, state: "missing", data, active: [], others: [] };
    const live = data.kind === "switch" ? (data.current ? [data.current] : []) : data.liveError ? [] : data.inLive;
    const active = live.map((id) => data.providers.find((p) => p.id === id)).filter((p): p is Provider => Boolean(p));
    const others = data.providers.filter((p) => !active.includes(p));
    return { tool, state: "ready", data, active, others };
  });
}

/** 光路右侧的一个服务端点。没有地址的按工具分成“官方登录”和“默认地址”。 */
export interface Host {
  key: string;
  kind: "url" | "official" | "default";
  label: string;
  tool: ToolMeta;
}

export function hostOf(tool: ToolMeta, provider: Provider): Host {
  const endpoint = endpointOf(tool.id, provider);
  if (endpoint) {
    let label = endpoint;
    try {
      label = new URL(endpoint).host || endpoint;
    } catch {
      /* 不是完整 URL 时原样显示。 */
    }
    return { key: `url:${label}`, kind: "url", label, tool };
  }
  if (isOfficial(provider)) return { key: `official:${tool.id}`, kind: "official", label: "", tool };
  return { key: `default:${tool.id}`, kind: "default", label: "", tool };
}

export interface Edge {
  key: string;
  row: Row;
  provider: Provider;
  host: Host;
  active: boolean;
}

/** 光路的连线和端点。端点按所连工具的平均位置排序，减少交叉。 */
export function buildMap(rows: Row[]) {
  const shown = rows.filter((r) => r.state === "ready" && r.data && r.data.providers.length > 0);
  const edges: Edge[] = [];
  const hosts = new Map<string, Host & { weight: number; sum: number }>();
  shown.forEach((row, index) => {
    for (const provider of row.data!.providers) {
      const host = hostOf(row.tool, provider);
      const active = row.active.includes(provider);
      edges.push({ key: latencyKey(row.tool.id, provider.id), row, provider, host, active });
      const entry = hosts.get(host.key) ?? { ...host, weight: 0, sum: 0 };
      const weight = active ? 3 : 1;
      entry.weight += weight;
      entry.sum += index * weight;
      hosts.set(host.key, entry);
    }
  });
  const order = [...hosts.values()].sort((a, b) => a.sum / a.weight - b.sum / b.weight || a.label.localeCompare(b.label));
  return { rows: shown, edges, hosts: order.map(({ weight: _w, sum: _s, ...host }) => host) };
}

/** Latency sends an unauthenticated GET to this URL, so cards sharing it need one probe.
 * Paths and query strings remain distinct; missing endpoints are never merged. */
export function probeKey(tool: ToolId, provider: Provider): string {
  const endpoint = endpointOf(tool, provider);
  if (endpoint) {
    try {
      const url = new URL(endpoint);
      url.hash = "";
      return url.href;
    } catch { /* Preserve per-card validation for malformed URLs. */ }
  }
  return latencyKey(tool, provider.id);
}

export function probeGroups(rows: Row[]) {
  const groups = new Map<string, { tool: ToolId; provider: Provider; keys: string[] }>();
  for (const row of rows) {
    if (row.state !== "ready") continue;
    for (const provider of row.data?.providers ?? []) {
      if (isOfficial(provider)) continue;
      const key = probeKey(row.tool.id, provider);
      const group = groups.get(key) ?? { tool: row.tool.id, provider, keys: [] };
      group.keys.push(latencyKey(row.tool.id, provider.id));
      groups.set(key, group);
    }
  }
  return [...groups.values()];
}
