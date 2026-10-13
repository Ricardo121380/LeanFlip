// 每日用量：按天的堆叠柱。鼠标停在某一天显示当天各项，时间线上同一天也会出现竖线。
import { useEffect, useRef, useState, type PointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { axisLabel, formatCompact, niceTicks, type Metric } from "@/lib/usage";
import { cn } from "@/lib/utils";

export interface Series {
  key: string;
  label: string;
  color: string;
  /** 和 `days` 一一对应。 */
  values: number[];
}

interface Props {
  series: Series[];
  days: string[];
  metric: Metric;
  hover: string | null;
  onHover: (day: string | null) => void;
  /** 变化时柱子重新从底部长出来。 */
  animationKey: string;
}

const HEIGHT = 240;
const M = { l: 60, r: 12, t: 12, b: 28 };

export function dayLabel(day: string, today: string, t: (k: string) => string) {
  if (day === today) return t("stats.today");
  const [, m, d] = day.split("-").map(Number);
  return `${m}/${d}`;
}

function topPath(x: number, y: number, w: number, h: number) {
  const r = Math.min(4, w / 2, h);
  return `M${x} ${y + h}V${y + r}Q${x} ${y} ${x + r} ${y}H${x + w - r}Q${x + w} ${y} ${x + w} ${y + r}V${y + h}Z`;
}

export function tickDays(days: string[]): string[] {
  const step = days.length <= 7 ? 1 : days.length <= 30 ? 5 : 15;
  const out: string[] = [];
  for (let i = days.length - 1; i >= 0; i -= step) out.push(days[i]);
  return out;
}

export function DailyChart({ series, days, metric, hover, onHover, animationKey }: Props) {
  const { t, i18n } = useTranslation();
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(720);
  const [tip, setTip] = useState<{ x: number; y: number; flip: boolean } | null>(null);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const measure = () => setWidth(Math.max(560, el.clientWidth));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const n = days.length;
  const plotW = width - M.l - M.r;
  const plotH = HEIGHT - M.t - M.b;
  const totals = days.map((_, i) => series.reduce((sum, s) => sum + s.values[i], 0));
  const { top, step } = niceTicks(Math.max(0, ...totals), metric === "requests");
  const y = (v: number) => M.t + plotH - (v / top) * plotH;
  const band = plotW / n;
  const barW = Math.min(24, Math.max(2, band * 0.64));
  const today = days[days.length - 1];
  const show = (v: number) => (metric === "requests" ? Math.round(v).toLocaleString("en-US") : formatCompact(v, i18n.language));
  const ticks: number[] = [];
  for (let v = 0; v <= top + step / 2; v += step) ticks.push(v);
  const labelled = new Set(tickDays(days));
  const hoverIndex = hover ? days.indexOf(hover) : -1;

  const onMove = (event: PointerEvent<SVGRectElement>, day: string) => {
    const rect = box.current!.getBoundingClientRect();
    const x = event.clientX - rect.left + box.current!.scrollLeft;
    setTip({ x, y: event.clientY - rect.top, flip: x > rect.width - 240 });
    if (day !== hover) onHover(day);
  };

  return (
    <div ref={box} className={cn("chart-box", hoverIndex >= 0 && "hovering")} onPointerLeave={() => { setTip(null); onHover(null); }}>
      <svg width={width} height={HEIGHT} viewBox={`0 0 ${width} ${HEIGHT}`} role="img" aria-label={t("stats.chartLabel", { count: n })}>
        {ticks.map((v) => (
          <g key={v}>
            <line className={v === 0 ? "baseline" : "gridline"} x1={M.l} x2={width - M.r} y1={y(v)} y2={y(v)} />
            <text className="axis-t" x={M.l - 8} y={y(v) + 3.5} textAnchor="end">{metric === "requests" ? v.toLocaleString("en-US") : axisLabel(v, i18n.language)}</text>
          </g>
        ))}
        {hoverIndex >= 0 && <line className="xhair" x1={M.l + band * (hoverIndex + 0.5)} x2={M.l + band * (hoverIndex + 0.5)} y1={M.t} y2={M.t + plotH} />}
        <g key={animationKey} className="cols">
          {days.map((day, i) => {
            const x = M.l + band * i + band / 2 - barW / 2;
            const parts = series.map((s) => ({ s, v: s.values[i] })).filter((p) => p.v > 0);
            let acc = 0;
            return (
              <g key={day} className={cn("col", i === hoverIndex && "hot")} style={{ animationDelay: `${Math.min(10, 420 / n) * i}ms` }}>
                {parts.map(({ s, v }, k) => {
                  const yTop = y(acc + v);
                  const yBottom = y(acc);
                  acc += v;
                  // 2px 的表面色间隙把相邻的两段分开。
                  const h = yBottom - yTop - (k > 0 ? 2 : 0);
                  if (h <= 0.3) return null;
                  return k === parts.length - 1 ? (
                    <path key={s.key} d={topPath(x, yTop, barW, h)} style={{ fill: s.color }} />
                  ) : (
                    <rect key={s.key} x={x} y={yTop} width={barW} height={h} style={{ fill: s.color }} />
                  );
                })}
              </g>
            );
          })}
        </g>
        {days.map((day, i) =>
          labelled.has(day) ? (
            <text key={day} className="axis-t" x={M.l + band * i + band / 2} y={HEIGHT - 9} textAnchor={i === 0 ? "start" : i === n - 1 ? "end" : "middle"}>
              {dayLabel(day, today, t)}
            </text>
          ) : null,
        )}
        {days.map((day, i) => (
          <rect key={day} className="hit" x={M.l + band * i} y={M.t} width={band} height={plotH} onPointerMove={(e) => onMove(e, day)} />
        ))}
      </svg>
      {tip && hoverIndex >= 0 && (
        <div className="chart-tip" role="tooltip" style={{ left: tip.flip ? tip.x - 14 : tip.x + 14, top: Math.max(8, tip.y - 20), transform: tip.flip ? "translateX(-100%)" : undefined }}>
          <div className="tip-head">{new Date(`${hover}T00:00:00`).toLocaleDateString(i18n.language, { month: "long", day: "numeric", weekday: "short" })}{hover === today ? ` · ${t("stats.untilNow")}` : ""}</div>
          <div className="tip-total">{show(totals[hoverIndex])}{metric === "requests" ? ` ${t("stats.requestsUnit")}` : " token"}</div>
          {series.map((s) => (
            <div key={s.key} className="tip-row">
              <i style={{ background: s.color }} />
              <b>{show(s.values[hoverIndex])}</b>
              <span>{s.label}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
