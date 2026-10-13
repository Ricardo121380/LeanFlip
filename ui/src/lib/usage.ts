// 统计页的纯函数：按天汇总、时间范围、切换记录分段、数字格式（有单元测试）。
import type { SwitchEvent, ToolId, UsageRow } from "./api";

export type Range = 7 | 30 | 90;
export type Metric = "tokens" | "output" | "requests";

export interface Totals {
  input: number;
  cacheRead: number;
  cacheWrite: number;
  output: number;
  requests: number;
}

export const emptyTotals = (): Totals => ({ input: 0, cacheRead: 0, cacheWrite: 0, output: 0, requests: 0 });
export const tokensOf = (t: Totals) => t.input + t.cacheRead + t.cacheWrite + t.output;
const inputSide = (t: Totals) => t.input + t.cacheRead + t.cacheWrite;
/** 缓存命中率 = 缓存读取 ÷（未缓存输入 + 缓存读取 + 缓存写入）。没有输入时为 null。 */
export const hitRate = (t: Totals): number | null => (inputSide(t) > 0 ? t.cacheRead / inputSide(t) : null);
export const metricOf = (t: Totals, metric: Metric) => (metric === "tokens" ? tokensOf(t) : metric === "output" ? t.output : t.requests);

export function add(into: Totals, row: Totals): Totals {
  into.input += row.input;
  into.cacheRead += row.cacheRead;
  into.cacheWrite += row.cacheWrite;
  into.output += row.output;
  into.requests += row.requests;
  return into;
}

/** 本地日期 `YYYY-MM-DD`，和后端的日期键一致。 */
export function dayKey(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

export function startOfDay(date: Date): Date {
  const d = new Date(date);
  d.setHours(0, 0, 0, 0);
  return d;
}

/** 时间范围里的日期，从早到晚。`back = 1` 是紧挨着的前一个同长度范围。 */
export function days(range: Range, now: Date, back = 0): string[] {
  const end = startOfDay(now);
  end.setDate(end.getDate() - back * range);
  const out: string[] = [];
  for (let i = range - 1; i >= 0; i--) {
    const d = new Date(end);
    d.setDate(d.getDate() - i);
    out.push(dayKey(d));
  }
  return out;
}

/** 某个日期 00:00 的时间戳。 */
export function dayStart(key: string): number {
  const [y, m, d] = key.split("-").map(Number);
  return new Date(y, m - 1, d).getTime();
}

/** 按 `key` 分组，每组按天汇总：`result.get(group).get(day)`。 */
export function byDay(rows: UsageRow[], key: (row: UsageRow) => string): Map<string, Map<string, Totals>> {
  const out = new Map<string, Map<string, Totals>>();
  for (const row of rows) {
    const group = key(row);
    let perDay = out.get(group);
    if (!perDay) out.set(group, (perDay = new Map()));
    add(perDay.get(row.day) ?? perDay.set(row.day, emptyTotals()).get(row.day)!, row);
  }
  return out;
}

export function sum(rows: UsageRow[], dayset: Set<string>): Totals {
  const out = emptyTotals();
  for (const row of rows) if (dayset.has(row.day)) add(out, row);
  return out;
}

/** 时间线上的一段：这段时间里用的是哪个供应商。 */
export interface Segment {
  provider: string;
  name: string;
  start: number;
  end: number;
  live: boolean;
}

/** 切换式工具：切换记录 → 连续的分段（没有选择供应商的时间不画）。`since` 是记录开始的时间。 */
export function switchSegments(events: SwitchEvent[], now: number): { since: number | null; segments: Segment[] } {
  const sorted = [...events].sort((a, b) => a.ts - b.ts);
  const segments: Segment[] = [];
  sorted.forEach((event, i) => {
    if (!event.provider) return;
    const next = sorted[i + 1];
    segments.push({ provider: event.provider, name: event.name, start: event.ts, end: next ? next.ts : now, live: !next });
  });
  return { since: sorted[0]?.ts ?? null, segments };
}

/** 并存式工具：每个供应商写进客户端的时间段。 */
export function coexistLanes(events: SwitchEvent[], now: number): { since: number | null; lanes: Map<string, Segment[]> } {
  const sorted = [...events].sort((a, b) => a.ts - b.ts);
  const lanes = new Map<string, Segment[]>();
  const open = new Map<string, Segment>();
  for (const event of sorted) {
    const current = open.get(event.provider);
    if (event.enabled && !current) {
      const segment = { provider: event.provider, name: event.name, start: event.ts, end: now, live: true };
      open.set(event.provider, segment);
      lanes.set(event.provider, [...(lanes.get(event.provider) ?? []), segment]);
    } else if (!event.enabled && current) {
      current.end = event.ts;
      current.live = false;
      open.delete(event.provider);
    }
  }
  return { since: sorted[0]?.ts ?? null, lanes };
}

/** 一段时间里的切换次数。每个工具的第一批记录只是记下当时的状态，不算切换。 */
export function switchCount(events: SwitchEvent[], from: number, to: number): number {
  const first = new Map<ToolId, number>();
  for (const event of events) first.set(event.tool, Math.min(first.get(event.tool) ?? Infinity, event.ts));
  return events.filter((e) => e.ts > first.get(e.tool)! && e.ts >= from && e.ts < to).length;
}

/** 大数字：中文按万 / 亿，英文按 K / M / B。 */
export function compact(n: number, language: string): { value: string; unit: string } {
  const digits = (v: number) => (v < 10 ? 2 : v < 100 ? 1 : 0);
  const steps: [number, string][] = language.startsWith("zh") ? [[1e8, "亿"], [1e4, "万"]] : [[1e9, "B"], [1e6, "M"], [1e3, "K"]];
  for (const [size, unit] of steps) {
    if (n >= size) {
      const v = n / size;
      return { value: v.toFixed(digits(v)), unit };
    }
  }
  return { value: Math.round(n).toLocaleString("en-US"), unit: "" };
}

export function formatCompact(n: number, language: string): string {
  const { value, unit } = compact(n, language);
  if (!unit) return value;
  return language.startsWith("zh") ? `${value} ${unit}` : `${value}${unit}`;
}

/** 坐标轴刻度：大约四格，步长取 1 / 2 / 2.5 / 5 × 10ⁿ。 */
export function niceTicks(max: number, integers = false): { top: number; step: number } {
  if (max <= 0) return { top: 1, step: 1 };
  const raw = max / 4;
  const magnitude = Math.pow(10, Math.floor(Math.log10(raw)));
  const norm = raw / magnitude;
  let step = (norm <= 1 ? 1 : norm <= 2 ? 2 : norm <= 2.5 ? 2.5 : norm <= 5 ? 5 : 10) * magnitude;
  if (integers) step = Math.max(1, Math.round(step));
  return { top: Math.ceil(max / step) * step, step };
}

/** 刻度标签：去掉多余的零。 */
export function axisLabel(n: number, language: string): string {
  if (n === 0) return "0";
  const { value, unit } = compact(n, language);
  const trimmed = String(Number(value.replace(/,/g, "")) || value);
  if (!unit) return value;
  return language.startsWith("zh") ? `${trimmed} ${unit}` : `${trimmed}${unit}`;
}
