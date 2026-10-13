// 统计页：读各工具留在本机的会话记录，按天汇总 token 用量，并用切换记录画出时间线。
import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw, TriangleAlert } from "lucide-react";
import { Button } from "@/components/ui/button";
import { QueryState } from "@/components/QueryState";
import { useAllProviders, useSwitchProvider, useToggleLive, useUsage } from "@/hooks/queries";
import { errorMessage, type SourceStatus, type ToolId, type UsageRow } from "@/lib/api";
import { canAnimate } from "@/lib/motion";
import { TOOLS, type ToolMeta } from "@/lib/tools";
import {
  add,
  coexistLanes,
  compact,
  dayStart,
  days as rangeDays,
  emptyTotals,
  formatCompact,
  hitRate,
  metricOf,
  switchCount,
  switchSegments,
  tokensOf,
  type Metric,
  type Range,
  type Segment,
  type Totals,
} from "@/lib/usage";
import { cn } from "@/lib/utils";
import { DailyChart, dayLabel, type Series } from "./stats/DailyChart";
import { Timeline, type Lane } from "./stats/Timeline";

const RANGES: Range[] = [7, 30, 90];
const PROVIDER_SLOTS = 8;
// 开场只在启动后第一次打开统计页时播放。
let introPlayed = false;

/** 一个数字从上一次的值跑到新值；减少动态效果时直接显示。 */
function Animated({ value, format }: { value: number; format: (v: number) => { value: string; unit: string } }) {
  const [shown, setShown] = useState(value);
  const from = useRef(introPlayed ? value : 0);
  useEffect(() => {
    const start = from.current;
    from.current = value;
    if (!canAnimate() || typeof requestAnimationFrame !== "function" || start === value) {
      setShown(value);
      return;
    }
    const t0 = performance.now();
    let frame = 0;
    const tick = (now: number) => {
      const p = Math.min(1, (now - t0) / 620);
      setShown(start + (value - start) * (1 - Math.pow(1 - p, 3)));
      if (p < 1) frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [value]);
  const { value: text, unit } = format(shown);
  return (
    <>
      {text}
      {unit && <span className="kpi-unit">{unit}</span>}
    </>
  );
}

function Spark({ values }: { values: number[] }) {
  if (values.length < 2) return null;
  const max = Math.max(...values);
  const min = Math.min(...values);
  const y = (v: number) => 27 - ((v - min) / (max - min || 1)) * 23;
  const d = values.map((v, i) => `${i ? "L" : "M"}${((i / (values.length - 1)) * 100).toFixed(1)} ${y(v).toFixed(1)}`).join("");
  return (
    <svg className="spark" viewBox="0 0 100 30" preserveAspectRatio="none" aria-hidden="true">
      <path className="spark-area" d={`${d}L100 30L0 30Z`} />
      <path className="spark-line" d={d} vectorEffect="non-scaling-stroke" />
      <circle className="spark-end" cx={100} cy={y(values[values.length - 1])} r={3.5} vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

function sourceText(source: SourceStatus, t: (k: string) => string) {
  if (source.state === "unsupported") return t("stats.source.unsupported");
  if (source.state === "failed") return t("stats.source.failed");
  if (source.state === "missing") return t(source.tested ? "stats.source.missing" : "stats.source.missingUntested");
  if (!source.tested) return t("stats.source.readUntested");
  if (source.experimental) return t("stats.source.readExperimental");
  return t("stats.source.read");
}

export function StatsPage() {
  const { t, i18n } = useTranslation();
  const usage = useUsage();
  const providerResults = useAllProviders();
  const switchAction = useSwitchProvider();
  const toggleAction = useToggleLive();
  const [range, setRange] = useState<Range>(30);
  const [tool, setTool] = useState<ToolId | "all">("all");
  const [metric, setMetric] = useState<Metric>("tokens");
  const [hover, setHover] = useState<string | null>(null);
  const [sort, setSort] = useState<{ col: string; dir: 1 | -1 }>({ col: "total", dir: -1 });
  const [intro] = useState(() => !introPlayed && canAnimate());
  useEffect(() => {
    if (usage.data) introPlayed = true;
  }, [usage.data]);

  const report = usage.data;
  const language = i18n.language;
  const now = report?.scannedAt ?? Date.now();
  const today = new Date(now);

  // 每个工具的 LeanFlip 供应商：名字和颜色顺序都跟着供应商列表走，筛选不会让颜色变。
  const known = useMemo(() => {
    const map = new Map<ToolId, { ids: string[]; names: Map<string, string> }>();
    TOOLS.forEach((meta, i) => {
      const data = providerResults[i]?.data;
      map.set(meta.id, { ids: data?.providers.map((p) => p.id) ?? [], names: new Map(data?.providers.map((p) => [p.id, p.name]) ?? []) });
    });
    return map;
  }, [providerResults]);
  const current = (meta: ToolMeta): string[] => {
    const data = providerResults[TOOLS.indexOf(meta)]?.data;
    if (!data) return [];
    return data.kind === "switch" ? (data.current ? [data.current] : []) : data.inLive;
  };

  const model = useMemo(() => {
    if (!report) return null;
    const rows = report.rows;
    const events = report.events;
    const withData = TOOLS.filter((meta) => rows.some((r) => r.tool === meta.id) || events.some((e) => e.tool === meta.id));
    // 没有记录过供应商的名字（客户端自己的、已删除的）排在 LeanFlip 的供应商后面。
    const order = new Map<ToolId, string[]>();
    for (const meta of TOOLS) {
      const ids = [...(known.get(meta.id)?.ids ?? [])];
      const extra = new Set<string>();
      for (const r of rows) if (r.tool === meta.id && r.provider && !ids.includes(r.provider)) extra.add(r.provider);
      for (const e of events) if (e.tool === meta.id && e.provider && !ids.includes(e.provider)) extra.add(e.provider);
      order.set(meta.id, [...ids, ...[...extra].sort()]);
    }
    const eventName = new Map<string, string>();
    for (const e of events) if (e.name) eventName.set(`${e.tool}:${e.provider}`, e.name);
    return { rows, events, withData, order, eventName };
  }, [report, known]);

  if (usage.isLoading) return <QueryState state="loading" title={t("stats.loading")} />;
  if (!report || !model) return <QueryState state="error" title={t("stats.loadError")} description={usage.error ? errorMessage(usage.error) : undefined} onRetry={() => void usage.refetch()} />;

  const selected = tool === "all" ? model.withData : model.withData.filter((m) => m.id === tool);
  const tools = new Set(selected.map((m) => m.id));
  const rows = model.rows.filter((r) => tools.has(r.tool));
  const dayList = rangeDays(range, today);
  const prevList = rangeDays(range, today, 1);
  const daySet = new Set(dayList);
  const prevSet = new Set(prevList);
  const nameOf = (meta: ToolMeta, provider: string) =>
    provider ? known.get(meta.id)?.names.get(provider) ?? model.eventName.get(`${meta.id}:${provider}`) ?? provider : t("stats.unattributed");
  const colorOf = (meta: ToolMeta, provider: string) => {
    const index = provider ? (model.order.get(meta.id) ?? []).indexOf(provider) : -1;
    return index >= 0 && index < PROVIDER_SLOTS ? `var(--pv-${index + 1})` : "var(--pv-none)";
  };
  const isCurrent = (meta: ToolMeta, provider: string) => current(meta).includes(provider);

  const sumOf = (set: Set<string>, filter: (r: UsageRow) => boolean = () => true) => {
    const out = emptyTotals();
    for (const r of rows) if (set.has(r.day) && filter(r)) add(out, r);
    return out;
  };
  const cur = sumOf(daySet);
  const prev = sumOf(prevSet);
  const daily = (fn: (t: Totals) => number) =>
    dayList.map((day) => fn(rows.filter((r) => r.day === day).reduce((acc, r) => add(acc, r), emptyTotals())));

  const events = model.events.filter((e) => tools.has(e.tool));
  const from = dayStart(dayList[0]);
  const prevFrom = dayStart(prevList[0]);
  const logStart = events.length ? Math.min(...events.map((e) => e.ts)) : null;
  const switches = switchCount(events, from, now + 1);
  const prevSwitches = switchCount(events, prevFrom, from);
  const lastSwitch = [...events].filter((e) => e.ts > (logStart ?? 0)).sort((a, b) => b.ts - a.ts)[0];

  const change = (a: number, b: number) => (b ? ((a - b) / b) * 100 : null);
  const arrow = (v: number) => (v > 0.05 ? "↑" : v < -0.05 ? "↓" : "→");
  const vsPrev = (v: number | null, unit: string) => (v === null ? t("stats.noPrevious", { count: range }) : t("stats.vsPrevious", { arrow: arrow(v), value: `${Math.abs(v).toFixed(unit === "pp" ? 1 : 0)}${unit === "pp" ? t("stats.points") : "%"}`, count: range }));
  const hitCur = hitRate(cur);
  const hitPrev = hitRate(prev);
  const hitDelta = hitCur !== null && hitPrev !== null ? (hitCur - hitPrev) * 100 : null;
  const tokenFormat = (v: number) => compact(v, language);

  // 图表：全部工具时按工具着色，只看一个工具时按供应商着色。
  const series: Series[] =
    tool === "all"
      ? selected.map((meta) => ({
          key: meta.id,
          label: meta.label,
          color: `var(--tool-${meta.id})`,
          values: dayList.map((day) => metricOf(rows.filter((r) => r.tool === meta.id && r.day === day).reduce((acc, r) => add(acc, r), emptyTotals()), metric)),
        }))
      : selected.flatMap((meta) => {
          const keys = [...(model.order.get(meta.id) ?? []), ""].filter((p) => rows.some((r) => r.provider === p && daySet.has(r.day)));
          return keys.map((provider) => ({
            key: provider || "none",
            label: nameOf(meta, provider),
            color: colorOf(meta, provider),
            values: dayList.map((day) => metricOf(rows.filter((r) => r.provider === provider && r.day === day).reduce((acc, r) => add(acc, r), emptyTotals()), metric)),
          }));
        });

  // 时间线
  const lanes: Lane[] = selected.map((meta) => {
    const mine = model.events.filter((e) => e.tool === meta.id);
    const tokens = tokensOf(sumOf(daySet, (r) => r.tool === meta.id));
    const names = current(meta).map((p) => nameOf(meta, p));
    if (meta.kind === "switch") {
      const { since, segments } = switchSegments(mine, now);
      return { tool: meta, kind: "switch", since, rows: [{ provider: current(meta)[0] ?? segments[segments.length - 1]?.provider ?? "", segments }], current: names[0] ?? t("stats.noneSelected"), tokens };
    }
    const { since, lanes: map } = coexistLanes(mine, now);
    const providers = [...map.keys()].filter((p) => map.get(p)!.some((s) => s.end > from)).slice(0, 5);
    return { tool: meta, kind: "coexist", since, rows: providers.map((provider) => ({ provider, segments: map.get(provider)! })), current: names.join("、") || t("stats.noneConnected"), tokens };
  });
  const segmentTokens = (meta: ToolMeta, segment: Segment) =>
    model.rows
      .filter((r) => r.tool === meta.id && r.provider === segment.provider)
      .reduce((sum, r) => {
        const start = dayStart(r.day);
        const overlap = Math.max(0, Math.min(segment.end, start + 86_400_000) - Math.max(segment.start, start));
        return sum + tokensOf(r) * (overlap / 86_400_000);
      }, 0);
  const restore = (meta: ToolMeta, provider: string) => {
    const name = nameOf(meta, provider);
    if (meta.kind === "switch") {
      const before = current(meta)[0];
      switchAction.mutate({ tool: meta.id, id: provider, name, previous: before ? { id: before, name: nameOf(meta, before) } : null });
    } else {
      toggleAction.mutate({ tool: meta.id, id: provider, name, enabled: true });
    }
  };

  // 按供应商
  const groups = new Map<string, { meta: ToolMeta; provider: string; totals: Totals; attribution: Map<string, number> }>();
  for (const r of rows) {
    if (!daySet.has(r.day)) continue;
    const meta = TOOLS.find((m) => m.id === r.tool)!;
    const key = `${r.tool}:${r.provider}`;
    const group = groups.get(key) ?? groups.set(key, { meta, provider: r.provider, totals: emptyTotals(), attribution: new Map() }).get(key)!;
    add(group.totals, r);
    group.attribution.set(r.attribution, (group.attribution.get(r.attribution) ?? 0) + tokensOf(r));
  }
  const allTokens = [...groups.values()].reduce((s, g) => s + tokensOf(g.totals), 0) || 1;
  const untested = new Set(report.sources.filter((s) => !s.tested).map((s) => s.tool));
  const columns: { id: string; label: string; num?: boolean }[] = [
    { id: "name", label: t("stats.col.provider") },
    { id: "requests", label: t("stats.col.requests"), num: true },
    { id: "input", label: t("stats.col.input"), num: true },
    { id: "cacheRead", label: t("stats.col.cacheRead"), num: true },
    { id: "cacheWrite", label: t("stats.col.cacheWrite"), num: true },
    { id: "output", label: t("stats.col.output"), num: true },
    { id: "total", label: t("stats.col.total"), num: true },
    { id: "hit", label: t("stats.col.hit"), num: true },
    { id: "attribution", label: t("stats.col.attribution") },
  ];
  const table = [...groups.values()].map((g) => {
    const main = [...g.attribution.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? "none";
    const attribution = !g.provider ? (main === "time" ? "noneSelected" : "none") : main;
    // 缓存读写都是 0 的供应商（比如本地模型）不用缓存，命中率没有意义。
    const cached = g.totals.cacheRead + g.totals.cacheWrite > 0;
    return { ...g, name: nameOf(g.meta, g.provider), total: tokensOf(g.totals), hit: cached ? hitRate(g.totals) : null, attributionKey: attribution, managed: !g.provider || (known.get(g.meta.id)?.ids ?? []).includes(g.provider) };
  });
  const value = (r: (typeof table)[number]): number | string =>
    sort.col === "name" ? r.name : sort.col === "attribution" ? r.attributionKey : sort.col === "total" ? r.total : sort.col === "hit" ? r.hit ?? -1 : r.totals[sort.col as keyof Totals];
  table.sort((a, b) => {
    const x = value(a);
    const y = value(b);
    return (typeof x === "string" ? x.localeCompare(String(y), language) : x - (y as number)) * sort.dir;
  });

  const empty = model.withData.length === 0;
  const animationKey = `${range}|${tool}|${metric}`;

  return (
    <div className="mx-auto flex w-full min-w-0 max-w-6xl flex-col gap-5 px-6 py-7 lg:px-8">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0">
          <h1 className="text-[22px] font-semibold leading-tight tracking-tight">{t("stats.title")}</h1>
          <p className="mt-0.5 text-xs leading-5 text-muted-foreground">
            {t("stats.subtitle", { time: new Date(report.scannedAt).toLocaleTimeString(language, { hour: "2-digit", minute: "2-digit" }) })}
          </p>
        </div>
        <Button size="sm" variant="outline" className="h-8" disabled={usage.isFetching} onClick={() => void usage.refetch()}>
          <RefreshCw className={cn(usage.isFetching && "animate-spin")} /> {t(usage.isFetching ? "stats.scanning" : "stats.rescan")}
        </Button>
      </header>

      {empty ? (
        <section className="flex flex-col items-start gap-2 rounded-2xl border border-dashed px-6 py-7">
          <h2 className="text-base font-semibold">{t("stats.emptyTitle")}</h2>
          <p className="max-w-prose text-sm leading-relaxed text-muted-foreground">{t("stats.emptyHint")}</p>
        </section>
      ) : (
        <section className="stats-panel" aria-label={t("stats.title")}>
          <div className="stats-filters">
            <div className="segmented" role="group" aria-label={t("stats.range")}>
              {RANGES.map((r) => (
                <button key={r} type="button" aria-pressed={range === r} className={cn(range === r && "on")} onClick={() => setRange(r)}>{t("stats.days", { count: r })}</button>
              ))}
            </div>
            <div className="flex flex-wrap gap-1.5" role="group" aria-label={t("stats.tools")}>
              {[null, ...model.withData].map((meta) => {
                const id = meta?.id ?? "all";
                return (
                  <button key={id} type="button" aria-pressed={tool === id} className="chip-btn" onClick={() => setTool(id)}>
                    {meta && <span className="sw" style={{ background: `var(--tool-${meta.id})` }} />}
                    {meta?.label ?? t("stats.allTools")}
                  </button>
                );
              })}
            </div>
          </div>

          <div className="kpis">
            <div className="kpi">
              <div className="kpi-label">{t("stats.kpi.tokens")}</div>
              <div className="kpi-value"><Animated value={tokensOf(cur)} format={tokenFormat} /></div>
              <div className="kpi-delta">{vsPrev(change(tokensOf(cur), tokensOf(prev)), "%")}</div>
              <Spark values={daily(tokensOf)} />
            </div>
            <div className="kpi">
              <div className="kpi-label">{t("stats.kpi.requests")}</div>
              <div className="kpi-value"><Animated value={cur.requests} format={tokenFormat} /></div>
              <div className="kpi-delta">{vsPrev(change(cur.requests, prev.requests), "%")}</div>
              <Spark values={daily((v) => v.requests)} />
            </div>
            <div className="kpi">
              <div className="kpi-label">{t("stats.kpi.hit")}</div>
              <div className="kpi-value">{hitCur === null ? "—" : <Animated value={hitCur * 100} format={(v) => ({ value: v.toFixed(1), unit: "%" })} />}</div>
              <div className={cn("kpi-delta", hitDelta !== null && hitDelta > 0.2 && "good", hitDelta !== null && hitDelta < -0.2 && "bad")}>{vsPrev(hitDelta, "pp")}</div>
              <Spark values={daily((v) => hitRate(v) ?? 0)} />
            </div>
            <div className="kpi">
              <div className="kpi-label">{t("stats.kpi.switches")}</div>
              <div className="kpi-value"><Animated value={switches} format={(v) => ({ value: String(Math.round(v)), unit: t("stats.times") })} /></div>
              <div className="kpi-delta">{logStart !== null && logStart <= prevFrom ? t("stats.vsPreviousCount", { arrow: arrow(switches - prevSwitches), value: Math.abs(switches - prevSwitches), count: range }) : t("stats.logTooShort")}</div>
              <p className="kpi-last">
                {lastSwitch ? (
                  <>
                    {t("stats.lastSwitch")}
                    <b>{`${TOOLS.find((m) => m.id === lastSwitch.tool)?.label} ${lastSwitch.enabled ? "→" : "×"} ${nameOf(TOOLS.find((m) => m.id === lastSwitch.tool)!, lastSwitch.provider)}`}</b>
                    {` · ${new Date(lastSwitch.ts).toLocaleString(language, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })}`}
                  </>
                ) : (
                  t("stats.noSwitches")
                )}
              </p>
            </div>
          </div>

          <div className="stats-section">
            <div className="stats-head">
              <h2>{t("stats.timeline")}</h2>
              <div className="legend">
                <span><i style={{ background: "var(--pv-1)" }} />{t("stats.legendProvider")}</span>
                <span><i className="hatch" />{t("stats.legendBeforeLog")}</span>
              </div>
            </div>
            <Timeline
              lanes={lanes}
              days={dayList}
              now={now}
              hover={hover}
              colorOf={colorOf}
              nameOf={nameOf}
              isCurrent={isCurrent}
              segmentTokens={segmentTokens}
              onRestore={restore}
              animate={intro}
            />
          </div>

          <div className="stats-section">
            <div className="stats-head">
              <h2>{tool === "all" ? t("stats.dailyByTool") : t("stats.dailyByProvider", { tool: selected[0]?.label })}</h2>
              <div className="segmented" role="group" aria-label={t("stats.metric")}>
                {(["tokens", "output", "requests"] as Metric[]).map((m) => (
                  <button key={m} type="button" aria-pressed={metric === m} className={cn(metric === m && "on")} onClick={() => setMetric(m)}>{t(`stats.metric_${m}`)}</button>
                ))}
              </div>
            </div>
            {series.length > 1 && (
              <div className="legend">
                {series.map((s) => <span key={s.key}><i style={{ background: s.color }} />{s.label}</span>)}
              </div>
            )}
            <DailyChart series={series} days={dayList} metric={metric} hover={hover} onHover={setHover} animationKey={animationKey} />
            <details className="daily">
              <summary>{t("stats.dailyTable")}</summary>
              <div className="stats-table">
                <table>
                  <thead>
                    <tr>
                      <th>{t("stats.col.day")}</th>
                      {series.map((s) => <th key={s.key} className="num">{s.label}</th>)}
                      <th className="num">{t("stats.col.total")}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {[...dayList].reverse().map((day) => {
                      const i = dayList.indexOf(day);
                      const show = (v: number) => (metric === "requests" ? Math.round(v).toLocaleString("en-US") : formatCompact(v, language));
                      return (
                        <tr key={day}>
                          <td>{dayLabel(day, dayList[dayList.length - 1], t)}</td>
                          {series.map((s) => <td key={s.key} className="num">{show(s.values[i])}</td>)}
                          <td className="num">{show(series.reduce((sum, s) => sum + s.values[i], 0))}</td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            </details>
          </div>

          <div className="stats-section">
            <div className="stats-head">
              <h2>{t("stats.byProvider")}</h2>
              <span className="text-xs text-muted-foreground">{t("stats.hitFormula")}</span>
            </div>
            <div className="stats-table">
              <table className="bk">
                <thead>
                  <tr>
                    {columns.map((c) => (
                      <th key={c.id} className={cn(c.num && "num")} aria-sort={sort.col === c.id ? (sort.dir > 0 ? "ascending" : "descending") : undefined}>
                        <button type="button" onClick={() => setSort((s) => ({ col: c.id, dir: s.col === c.id ? (s.dir === 1 ? -1 : 1) : c.num ? -1 : 1 }))}>{c.label}</button>
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {table.map((r) => {
                    const low = r.hit !== null && r.hit < 0.5;
                    return (
                      <tr key={`${r.meta.id}:${r.provider}`}>
                        <td>
                          <div className="who">
                            <i style={{ background: colorOf(r.meta, r.provider) }} />
                            <span>{r.name}</span>
                            {tool === "all" && <small>{r.meta.label}</small>}
                            {!r.managed && <small>{t("stats.notManaged")}</small>}
                            {untested.has(r.meta.id) && <small className="untested">{t("stats.untested")}</small>}
                          </div>
                        </td>
                        <td className="num">{Math.round(r.totals.requests).toLocaleString("en-US")}</td>
                        <td className="num">{formatCompact(r.totals.input, language)}</td>
                        <td className="num">{formatCompact(r.totals.cacheRead, language)}</td>
                        <td className="num">{r.totals.cacheWrite ? formatCompact(r.totals.cacheWrite, language) : "—"}</td>
                        <td className="num">{formatCompact(r.totals.output, language)}</td>
                        <td className="num">
                          <span className="share" style={{ width: Math.max(2, (r.total / allTokens) * 60), background: colorOf(r.meta, r.provider) } as CSSProperties} />
                          {formatCompact(r.total, language)}
                        </td>
                        <td className="num">
                          {r.hit === null ? (
                            <span title={t("stats.noCache")}>—</span>
                          ) : (
                            <>
                              <span className={cn("meter", low && "low")}>
                                <span className="track"><span className="fill" style={{ width: `${r.hit * 100}%` }} /></span>
                                {(r.hit * 100).toFixed(1)}%
                              </span>
                              {low && <span className="flag"><TriangleAlert aria-hidden="true" />{t("stats.lowHit")}</span>}
                            </>
                          )}
                        </td>
                        <td className={cn("attr", r.attributionKey === "exact" && "exact")}>{t(`stats.attribution_${r.attributionKey}`)}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </div>
        </section>
      )}

      <section className="stats-sources" aria-label={t("stats.sources")}>
        <div className="stats-head">
          <h2>{t("stats.sources")}</h2>
          <span className="text-xs text-muted-foreground">{t("stats.sourcesHint")}</span>
        </div>
        <ul>
          {report.sources.map((source) => {
            const meta = TOOLS.find((m) => m.id === source.tool)!;
            const tone = source.state === "failed" ? "bad" : source.state === "read" ? (source.tested && !source.experimental ? "ok" : "warn") : "off";
            return (
              <li key={source.tool}>
                <b>
                  <span>{meta.label}</span>
                  <span className={cn("state-pill", tone)}>{sourceText(source, t)}</span>
                </b>
                {source.path && <code title={source.path}>{source.path}</code>}
                {source.retentionDays && source.state === "read" && <small>{t("stats.retention", { count: source.retentionDays })}</small>}
                {source.detail && <small className="text-destructive">{source.detail}</small>}
              </li>
            );
          })}
        </ul>
        <p className="text-xs text-muted-foreground">{t("stats.privacy")}</p>
      </section>
    </div>
  );
}
