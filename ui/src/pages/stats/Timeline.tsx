// 时间线：每个工具一条泳道，每段是这段时间用的供应商。点过去的一段，光点沿泳道跑到“现在”，切回那个供应商。
import { useRef, useState, type CSSProperties, type MouseEvent, type ReactElement, type ReactNode } from "react";
import * as Tooltip from "@radix-ui/react-tooltip";
import { useTranslation } from "react-i18next";
import { ToolGlyph } from "@/components/Glyph";
import type { ToolMeta } from "@/lib/tools";
import { canAnimate } from "@/lib/motion";
import { dayStart, formatCompact, type Segment } from "@/lib/usage";
import { cn } from "@/lib/utils";
import { dayLabel, tickDays } from "./DailyChart";

export interface Lane {
  tool: ToolMeta;
  kind: "switch" | "coexist";
  /** 切换记录开始的时间；之前的部分画斜纹。 */
  since: number | null;
  /** 切换式一条，并存式每个供应商一条。 */
  rows: { provider: string; segments: Segment[] }[];
  current: string;
  tokens: number;
}

interface Props {
  lanes: Lane[];
  days: string[];
  now: number;
  hover: string | null;
  colorOf: (tool: ToolMeta, provider: string) => string;
  nameOf: (tool: ToolMeta, provider: string) => string;
  isCurrent: (tool: ToolMeta, provider: string) => boolean;
  canRestore: (tool: ToolMeta, provider: string) => boolean;
  onRestore: (tool: ToolMeta, provider: string) => void;
  animate: boolean;
}

const LANE = 26;
const THIN = 14;

function SegmentTip({ children, content }: { children: ReactElement; content: ReactNode }) {
  const trigger = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  return (
    <Tooltip.Root open={open} onOpenChange={(next) => {
      // Keyboard focus can scroll a narrow timeline. Keep its description open
      // while Popper follows the anchor; blur, Escape and activation still close it.
      if (next || document.activeElement !== trigger.current) setOpen(next);
    }}>
      <Tooltip.Trigger asChild ref={trigger} onBlur={() => setOpen(false)} onClick={() => setOpen(false)}>{children}</Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content className="chart-tip tl-tip" side="top" sideOffset={8} collisionPadding={12} onEscapeKeyDown={() => setOpen(false)}>{content}</Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

function when(ts: number, language: string) {
  return new Date(ts).toLocaleString(language, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function lasted(ms: number, t: (k: string, o?: Record<string, unknown>) => string) {
  if (ms < 60_000) return t("stats.durationSeconds", { count: Math.max(1, Math.ceil(ms / 1000)) });
  if (ms < 3_600_000) return t("stats.durationMinutes", { count: Math.ceil(ms / 60_000) });
  const hours = ms / 3_600_000;
  const days = Math.floor(hours / 24);
  const rest = Math.round(hours - days * 24);
  if (days) return rest ? t("stats.daysHours", { days, hours: rest }) : t("stats.durationDays", { count: days });
  return t("stats.durationHours", { count: Math.max(1, Math.round(hours)) });
}

export function Timeline({ lanes, days, now, hover, colorOf, nameOf, isCurrent, canRestore, onRestore, animate }: Props) {
  const { t, i18n } = useTranslation();
  const start = dayStart(days[0]);
  const x = (ts: number) => ((Math.min(Math.max(ts, start), now) - start) / (now - start)) * 100;
  const today = days[days.length - 1];
  const ticks = tickDays(days).map((day) => ({ day, left: x(dayStart(day)) })).filter((tick) => tick.left <= 94);
  const hair = hover ? x(dayStart(hover) + 43_200_000) : null;

  const restore = (event: MouseEvent<HTMLButtonElement>, tool: ToolMeta, segment: Segment, color: string) => {
    if (isCurrent(tool, segment.provider) || !canRestore(tool, segment.provider)) return;
    const button = event.currentTarget;
    const track = button.parentElement;
    if (!canAnimate() || !track) {
      onRestore(tool, segment.provider);
      return;
    }
    // 一颗光点从这一段跑到“现在”，到了再切换。
    const comet = document.createElement("span");
    comet.className = "tl-comet";
    comet.style.setProperty("--c", color);
    comet.style.top = `${button.offsetTop + button.offsetHeight / 2}px`;
    track.append(comet);
    const from = button.offsetLeft + button.offsetWidth / 2;
    const to = track.clientWidth + 6;
    comet
      .animate(
        [
          { transform: `translateX(${from}px) scale(.5)`, opacity: 0 },
          { transform: `translateX(${from + 8}px) scale(1)`, opacity: 1, offset: 0.12 },
          { transform: `translateX(${to}px)`, opacity: 1 },
        ],
        { duration: Math.min(620, Math.max(260, (to - from) * 0.8)), easing: "cubic-bezier(.55,0,.8,.45)" },
      )
      .finished.then(
        () => {
          comet.remove();
          onRestore(tool, segment.provider);
        },
        () => comet.remove(),
      );
  };

  return (
    <Tooltip.Provider delayDuration={150}>
    <div className="tl-scroll">
      <div className={cn("timeline", animate && "intro")}>
        <div className="tl-row tl-axis">
          <div />
          <div className="tl-track">
            {ticks.map((tick) => (
              <span key={tick.day} className="tl-tick" style={{ left: `${tick.left}%` }}>{dayLabel(tick.day, today, t)}</span>
            ))}
          </div>
          <div className="tl-now">
            <span className="now-label">{t("stats.now")}</span>
            <span className="tl-line" />
          </div>
          <div className="tl-info tl-sub">{t("stats.lastDays", { count: days.length })}</div>
        </div>
        {lanes.map((lane, index) => {
          const thin = lane.kind === "coexist" && lane.rows.length > 1;
          const laneH = thin ? THIN : LANE;
          const height = Math.max(LANE, lane.rows.length * laneH + (lane.rows.length - 1) * 3);
          const preEnd = lane.since === null ? now : lane.since;
          return (
            <div key={lane.tool.id} className="tl-row" style={{ "--i": index } as CSSProperties}>
              <div className="tl-tool">
                <ToolGlyph tool={lane.tool} size={26} />
                <div className="min-w-0">
                  <b>{lane.tool.label}</b>
                  <small>{t(lane.kind === "switch" ? "stats.switchKind" : "stats.coexistKind")}</small>
                </div>
              </div>
              <div className="tl-track" style={{ height }}>
                {ticks.map((tick) => <span key={tick.day} className="tl-day" style={{ left: `${tick.left}%` }} />)}
                {preEnd > start && (
                  <div className="tl-pre" style={{ width: `${x(preEnd)}%` }}>
                    {x(preEnd) > 16 && <span>{t(lane.since === null ? "stats.noLog" : "stats.beforeLog")}</span>}
                  </div>
                )}
                {lane.rows.flatMap((row, r) =>
                  row.segments
                    .filter((s) => s.end > start)
                    .map((segment) => {
                      const shownStart = Math.max(segment.start, start);
                      const left = x(shownStart);
                      const width = x(segment.end) - left;
                      const color = colorOf(lane.tool, segment.provider);
                      const name = nameOf(lane.tool, segment.provider);
                      const current = isCurrent(lane.tool, segment.provider);
                      return (
                        <SegmentTip key={`${row.provider}-${segment.start}`} content={<>
                          <div className="tip-head">{lane.tool.label} · {name}</div>
                          <div className="tip-total">{t("stats.segmentUsageUnavailable")}</div>
                          <div className="tip-note">{when(shownStart, i18n.language)} – {segment.live ? t("stats.now") : when(segment.end, i18n.language)} · {lasted(segment.end - shownStart, t)}</div>
                          <div className="tip-note">{current ? t("stats.inUse") : canRestore(lane.tool, segment.provider) ? t("stats.clickToRestore") : t("stats.unavailableProvider")}</div>
                        </>}>
                        <button
                          type="button"
                          aria-disabled={!current && !canRestore(lane.tool, segment.provider)}
                          className={cn("tl-seg", segment.live && "live", thin && "thin")}
                          style={{ left: `${left}%`, width: `${Math.max(0, width)}%`, top: r * (laneH + 3), height: thin ? laneH : height, "--c": color } as CSSProperties}
                          aria-label={`${lane.tool.label}：${name}，${when(shownStart, i18n.language)} – ${segment.live ? t("stats.now") : when(segment.end, i18n.language)}${current ? "" : `，${t(canRestore(lane.tool, segment.provider) ? "stats.clickToRestore" : "stats.unavailableProvider")}`}`}
                          onClick={(e) => restore(e, lane.tool, segment, color)}
                        >
                          {width > (thin ? 9 : 7) && <span>{name}</span>}
                        </button>
                        </SegmentTip>
                      );
                    }),
                )}
                {hair !== null && <span className="tl-hair" style={{ left: `${hair}%` }} />}
              </div>
              <div className="tl-now">
                <span className="tl-line" />
                {lane.rows.map((row, r) => {
                  const on = isCurrent(lane.tool, row.provider);
                  return (
                    <span
                      key={row.provider}
                      className={cn("tl-head", !on && "off")}
                      style={{ top: r * (laneH + 3), height: thin ? laneH : height, "--c": colorOf(lane.tool, row.provider) } as CSSProperties}
                    />
                  );
                })}
              </div>
              <div className="tl-info">
                <span className="tl-name">{lane.current}</span>
                <span className="tl-sub">{t("stats.laneTokens", { count: days.length, value: formatCompact(lane.tokens, i18n.language) })}</span>
                <details className="text-xs"><summary className="cursor-pointer text-primary">{t("stats.segmentHistory")}</summary>
                  <ul className="max-h-48 overflow-auto space-y-1 py-2">
                    {lane.rows.flatMap((row) => row.segments).filter((s) => s.end > start).sort((a, b) => b.start - a.start).map((s) => (
                      <li key={`${s.provider}-${s.start}`}><button type="button" className="w-full rounded border p-2 text-left disabled:opacity-60" disabled={isCurrent(lane.tool, s.provider) || !canRestore(lane.tool, s.provider)} onClick={() => onRestore(lane.tool, s.provider)}>
                        {nameOf(lane.tool, s.provider)} · {when(s.start, i18n.language)} – {s.live ? t("stats.now") : when(s.end, i18n.language)}
                      </button></li>
                    ))}
                  </ul>
                </details>
              </div>
            </div>
          );
        })}
      </div>
    </div>
    </Tooltip.Provider>
  );
}
