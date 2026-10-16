// 光路：左边是工具，右边是按地址合并的服务端点。正在用的路由上有光点流动，延迟越低流得越快。
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { Provider, QuotaSource, ToolId } from "@/lib/api";
import { isOfficial } from "@/lib/cards";
import { latencyTone, toneClass } from "@/lib/probe";
import { quotaBrief, quotaTitle } from "@/lib/quota";
import { canAnimate } from "@/lib/motion";
import { Morph } from "@/components/Morph";
import { ToolGlyph } from "@/components/Glyph";
import { cn } from "@/lib/utils";
import { buildMap, probeKey, type Edge, type Host, type Latency, type Row } from "./model";

interface Props {
  rows: Row[];
  latency: Latency;
  /** 每张卡（`tool:id`）的额度来源。 */
  quota?: Map<string, QuotaSource>;
  now?: number;
  onSwitch: (row: Row, provider: Provider) => void;
  onToggle: (row: Row, provider: Provider, enabled: boolean) => void;
}

interface Point {
  x: number;
  y: number;
}

interface Geometry {
  w: number;
  h: number;
  tools: Record<string, Point>;
  hosts: Record<string, Point>;
}

const SVG = "http://www.w3.org/2000/svg";
const IDLE_MS = 10_000;

function curve(a: Point, b: Point) {
  const dx = b.x - a.x;
  const f = (v: number) => v.toFixed(1);
  return `M${f(a.x)} ${f(a.y)}C${f(a.x + dx * 0.45)} ${f(a.y)} ${f(a.x + dx * 0.55)} ${f(b.y)} ${f(b.x)} ${f(b.y)}`;
}

function length(path?: SVGPathElement | null) {
  return path && typeof path.getTotalLength === "function" ? path.getTotalLength() : 0;
}

function point(path: SVGPathElement, at: number): Point {
  return typeof path.getPointAtLength === "function" ? path.getPointAtLength(at) : { x: 0, y: 0 };
}

/** 每条线在动画引擎里的状态；React 只负责结构，描线、光点和信号都在这里画。 */
interface Runtime {
  key: string;
  tool: ToolId;
  active: boolean;
  ms: number | null;
  main?: SVGPathElement | null;
  glow?: SVGPathElement | null;
  base?: SVGPathElement | null;
  dots: SVGCircleElement[];
  len: number;
  offset: number;
  tween?: { from: number; to: number; start: number; duration: number; comet?: Comet };
  probe?: { start: number; comet: Comet; settle?: { from: number; to: number; start: number; ok: boolean } };
  phase: number;
  host: string;
}

interface Comet {
  g: SVGGElement;
  head: SVGCircleElement;
  halo: SVGCircleElement;
  trail: SVGCircleElement[];
}

function svg<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number>, parent?: Element) {
  const node = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, String(v));
  parent?.appendChild(node);
  return node;
}

function makeComet(layer: SVGGElement, bad = false): Comet {
  const g = svg("g", { class: cn("lm-comet", bad && "bad") }, layer);
  const trail = [1, 2, 3, 4].map((k) => svg("circle", { r: 3.6 - k * 0.6, class: "lm-fx", opacity: (0.6 - k * 0.12).toFixed(2) }, g));
  const halo = svg("circle", { r: 11, class: "lm-fx-halo" }, g);
  const head = svg("circle", { r: 4.2, class: "lm-fx" }, g);
  return { g, head, halo, trail };
}

function placeComet(comet: Comet, path: SVGPathElement, len: number, at: number) {
  const clamp = (v: number) => Math.max(0, Math.min(len, v));
  const p = point(path, clamp(at));
  for (const n of [comet.head, comet.halo]) {
    n.setAttribute("cx", p.x.toFixed(1));
    n.setAttribute("cy", p.y.toFixed(1));
  }
  comet.trail.forEach((n, k) => {
    const q = point(path, clamp(at - (k + 1) * 9));
    n.setAttribute("cx", q.x.toFixed(1));
    n.setAttribute("cy", q.y.toFixed(1));
  });
  return p;
}

function ring(layer: SVGGElement, at: Point, tone: string) {
  const c = svg("circle", { cx: at.x.toFixed(1), cy: at.y.toFixed(1), r: 12, class: cn("lm-ring", tone) }, layer);
  c.animate([{ transform: "scale(1)", opacity: 0.85 }, { transform: "scale(2.2)", opacity: 0 }], { duration: 560, easing: "cubic-bezier(.2,.7,.2,1)" }).finished.then(() => c.remove(), () => c.remove());
}

function burst(layer: SVGGElement, at: Point) {
  ring(layer, at, "bad");
  for (let i = 0; i < 9; i++) {
    const a = (i / 9) * Math.PI * 2 + Math.random() * 0.5;
    const d = 16 + Math.random() * 18;
    const s = svg("circle", { cx: at.x.toFixed(1), cy: at.y.toFixed(1), r: 1.9, class: "lm-spark" }, layer);
    s.animate([{ transform: "translate(0px, 0px)", opacity: 1 }, { transform: `translate(${(Math.cos(a) * d).toFixed(1)}px, ${(Math.sin(a) * d).toFixed(1)}px)`, opacity: 0 }], {
      duration: 420 + Math.random() * 220,
      easing: "cubic-bezier(.1,.8,.3,1)",
    }).finished.then(() => s.remove(), () => s.remove());
  }
}

const easeOut = (t: number) => 1 - Math.pow(1 - t, 3);
const easeIn = (t: number) => t * t * t;

export function LightMap({ rows, latency, quota, now = Date.now(), onSwitch, onToggle }: Props) {
  const { t, i18n } = useTranslation();
  const map = useMemo(() => buildMap(rows), [rows]);
  const box = useRef<HTMLDivElement>(null);
  const fx = useRef<SVGGElement>(null);
  const toolNodes = useRef(new Map<string, HTMLElement>());
  const hostNodes = useRef(new Map<string, HTMLElement>());
  const edgeNodes = useRef(new Map<string, SVGGElement>());
  const [geo, setGeo] = useState<Geometry | null>(null);
  const [focus, setFocus] = useState<ToolId | null>(null);
  const runtime = useRef(new Map<string, Runtime>());
  const latest = useRef(latency);
  latest.current = latency;
  const kick = useRef<() => void>(() => {});

  const hostLabel = (host: Host) =>
    host.kind === "url" ? host.label : t(host.kind === "official" ? "home.hostOfficial" : "home.hostDefault", { tool: host.tool.label });

  // 量出每个工具和端点的位置，连线跟着窗口宽度走。
  useLayoutEffect(() => {
    const root = box.current;
    if (!root) return;
    const measure = () => {
      const origin = root.getBoundingClientRect();
      const center = (el: HTMLElement): Point => {
        const r = el.getBoundingClientRect();
        return { x: r.left + r.width / 2 - origin.left, y: r.top + r.height / 2 - origin.top };
      };
      const tools: Record<string, Point> = {};
      const hosts: Record<string, Point> = {};
      toolNodes.current.forEach((el, key) => { tools[key] = center(el); });
      hostNodes.current.forEach((el, key) => { hosts[key] = center(el); });
      const next = { w: origin.width, h: origin.height, tools, hosts };
      setGeo((prev) => (prev && JSON.stringify(prev) === JSON.stringify(next) ? prev : next));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(root);
    return () => observer.disconnect();
  }, [map]);

  // 结构变了之后同步动画状态：新接上的线从工具画向端点，断开的线收回。
  useLayoutEffect(() => {
    const reduced = !canAnimate();
    const seen = new Set<string>();
    const now = performance.now();
    const retracting = new Set<ToolId>();
    const pending: Runtime[] = [];
    for (const edge of map.edges) {
      seen.add(edge.key);
      const result = latest.current[edge.key];
      const ms = result && result !== "pending" && result.ok ? result.latencyMs : null;
      let rt = runtime.current.get(edge.key);
      const fresh = !rt;
      if (!rt) {
        rt = { key: edge.key, tool: edge.row.tool.id, active: edge.active, ms, dots: [], len: 0, offset: 0, phase: Math.random(), host: edge.host.key };
        runtime.current.set(edge.key, rt);
      }
      rt.ms = ms;
      rt.host = edge.host.key;
      const holder = edgeNodes.current.get(edge.key);
      rt.main = holder?.querySelector<SVGPathElement>(".lm-main");
      rt.glow = holder?.querySelector<SVGPathElement>(".lm-glow");
      rt.base = holder?.querySelector<SVGPathElement>(".lm-base");
      rt.dots = Array.from(holder?.querySelectorAll<SVGCircleElement>(".lm-dot") ?? []);
      const len = length(rt.main);
      if (len !== rt.len) {
        rt.len = len;
        for (const p of [rt.main, rt.glow]) if (p) p.style.strokeDasharray = `${len} ${len}`;
        if (!rt.tween) rt.offset = rt.active ? 0 : len;
      }
      if (!fresh && rt.active !== edge.active) {
        rt.active = edge.active;
        if (reduced) rt.offset = edge.active ? 0 : rt.len;
        else if (edge.active) pending.push(rt);
        else {
          rt.tween?.comet?.g.remove();
          rt.tween = { from: rt.offset, to: rt.len, start: now, duration: 360 };
          retracting.add(rt.tool);
        }
      }
      for (const p of [rt.main, rt.glow]) if (p) p.style.strokeDashoffset = String(rt.offset);
    }
    for (const rt of pending) {
      rt.tween?.comet?.g.remove();
      const comet = fx.current ? makeComet(fx.current) : undefined;
      comet?.g.setAttribute("opacity", "0");
      rt.tween = { from: rt.offset, to: 0, start: now + (retracting.has(rt.tool) ? 180 : 0), duration: 560, comet };
    }
    runtime.current.forEach((rt, key) => {
      if (seen.has(key)) return;
      rt.tween?.comet?.g.remove();
      rt.probe?.comet.g.remove();
      runtime.current.delete(key);
    });
    kick.current();
  });

  // 测连通：每条线一颗信号，等结果回来再到达端点；连不上的在半路炸开。
  useEffect(() => {
    if (!canAnimate() || !fx.current) return;
    const now = performance.now();
    runtime.current.forEach((rt) => {
      const result = latency[rt.key];
      if (result === "pending" && !rt.probe) rt.probe = { start: now, comet: makeComet(fx.current!) };
      else if (result && result !== "pending" && rt.probe && !rt.probe.settle) {
        const elapsed = (now - rt.probe.start) / 1000;
        const from = 0.85 * (1 - Math.exp(-elapsed / 0.9));
        rt.probe.settle = { from, to: result.ok ? 1 : 0.62, start: now, ok: result.ok };
        if (!result.ok) rt.probe.comet.g.classList.add("bad");
      }
    });
    kick.current();
  }, [latency]);

  // 一个绘制循环驱动所有线；窗口隐藏或 10 秒没有操作就停下。
  useEffect(() => {
    let frame = 0;
    let last = performance.now();
    let activity = performance.now();
    const step = (now: number) => {
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      const reduced = !canAnimate();
      let busy = false;
      runtime.current.forEach((rt) => {
        const path = rt.main;
        if (rt.tween && path) {
          const raw = (now - rt.tween.start) / rt.tween.duration;
          if (raw < 0) busy = true;
          else {
            const p = Math.min(1, raw);
            const ease = rt.tween.to === 0 ? easeOut : easeIn;
            rt.offset = rt.tween.from + (rt.tween.to - rt.tween.from) * ease(p);
            for (const el of [rt.main, rt.glow]) if (el) el.style.strokeDashoffset = rt.offset.toFixed(1);
            if (rt.tween.comet) {
              rt.tween.comet.g.setAttribute("opacity", "1");
              placeComet(rt.tween.comet, path, rt.len, rt.len - rt.offset);
            }
            if (p >= 1) {
              rt.tween.comet?.g.remove();
              if (rt.tween.to === 0 && fx.current) {
                const end = point(path, rt.len);
                ring(fx.current, end, "");
              }
              rt.tween = undefined;
            } else busy = true;
          }
        }
        if (rt.probe && rt.base) {
          busy = true;
          const len = length(rt.base);
          const s = rt.probe.settle;
          let at: number;
          if (s) {
            const p = Math.min(1, (now - s.start) / 260);
            at = s.from + (s.to - s.from) * easeOut(p);
            if (p >= 1) {
              const end = placeComet(rt.probe.comet, rt.base, len, at * len);
              rt.probe.comet.g.remove();
              if (fx.current) {
                if (s.ok) {
                  const result = latest.current[rt.key];
                  ring(fx.current, end, result && result !== "pending" && result.ok ? latencyTone(result.latencyMs) : "");
                } else burst(fx.current, end);
              }
              rt.probe = undefined;
              return;
            }
          } else at = 0.85 * (1 - Math.exp(-(now - rt.probe.start) / 900));
          placeComet(rt.probe.comet, rt.base, len, at * len);
        }
        const flowing = rt.active && !rt.tween && rt.dots.length > 0 && !reduced;
        const parts = rt.dots[0]?.parentElement;
        if (parts) parts.style.opacity = flowing ? "1" : "0";
        if (!flowing || !path) return;
        busy = true;
        const speed = 0.34 / Math.max(0.6, Math.min(4.2, (rt.ms ?? 300) / 230));
        rt.phase = (rt.phase + dt * speed) % 1;
        rt.dots.forEach((dot, k) => {
          const u = (rt.phase + k / rt.dots.length) % 1;
          const q = point(path, u * rt.len);
          dot.setAttribute("cx", q.x.toFixed(1));
          dot.setAttribute("cy", q.y.toFixed(1));
          dot.setAttribute("opacity", (Math.sin(Math.PI * u) * 0.95).toFixed(2));
        });
      });
      const idle = now - activity > IDLE_MS;
      const hasWork = [...runtime.current.values()].some((rt) => rt.tween || rt.probe);
      if ((busy && !idle) || hasWork) frame = requestAnimationFrame(step);
      else frame = 0;
    };
    const start = () => {
      if (frame || document.hidden) return;
      last = performance.now();
      frame = requestAnimationFrame(step);
    };
    const wake = () => {
      activity = performance.now();
      start();
    };
    const sleep = () => {
      cancelAnimationFrame(frame);
      frame = 0;
    };
    const visibility = () => (document.hidden ? sleep() : wake());
    kick.current = wake;
    window.addEventListener("pointermove", wake, { passive: true });
    window.addEventListener("keydown", wake);
    window.addEventListener("focus", wake);
    window.addEventListener("blur", sleep);
    document.addEventListener("visibilitychange", visibility);
    wake();
    return () => {
      sleep();
      kick.current = () => {};
      window.removeEventListener("pointermove", wake);
      window.removeEventListener("keydown", wake);
      window.removeEventListener("focus", wake);
      window.removeEventListener("blur", sleep);
      document.removeEventListener("visibilitychange", visibility);
      runtime.current.forEach((rt) => {
        rt.tween?.comet?.g.remove();
        rt.probe?.comet.g.remove();
        rt.tween = undefined;
        rt.probe = undefined;
      });
    };
  }, []);

  if (map.rows.length === 0) {
    return <p className="rounded-2xl border border-dashed px-6 py-10 text-center text-sm text-muted-foreground">{t("home.mapEmpty")}</p>;
  }

  const press = (edge: Edge) => {
    if (edge.row.data?.kind === "switch") {
      if (!edge.active) onSwitch(edge.row, edge.provider);
    } else if (!edge.row.data?.liveError) onToggle(edge.row, edge.provider, !edge.active);
  };
  const edgeLabel = (edge: Edge) => {
    const tool = edge.row.tool.label;
    const name = edge.provider.name;
    if (edge.row.data?.kind === "switch") return edge.active ? t("home.edgeCurrent", { tool, name }) : t("home.switchTo", { tool, name });
    return t(edge.active ? "home.remove" : "home.add", { tool, name });
  };
  const resultText = (edge: Edge) => {
    const result = latency[edge.key];
    if (!result || result === "pending" || isOfficial(edge.provider)) return null;
    if (!result.ok) return <span className="text-destructive">{t("probe.unreachable")}</span>;
    return <span className={toneClass[latencyTone(result.latencyMs)]}>{t("probe.ms", { ms: result.latencyMs })}</span>;
  };

  // 端点上的额度：连到这里的卡用到的每个额度来源各一个标记，同一个账号或 Key 只标一次。
  const quotaMarks = (edges: Edge[]) => {
    if (!quota) return null;
    const sources = new Map<string, QuotaSource>();
    for (const edge of edges) {
      const source = quota.get(`${edge.row.tool.id}:${edge.provider.id}`);
      if (source?.snapshot && !sources.has(source.key)) sources.set(source.key, source);
    }
    return [...sources.values()].map((source) => {
      const brief = quotaBrief(source.snapshot!, now, t, i18n.language);
      if (!brief) return null;
      const title = quotaTitle(source, now, t);
      return (
        <span key={source.key} className="qs" data-level={brief.level} title={title} aria-label={[brief.text, title].filter(Boolean).join("\n")}>
          {brief.text}
        </span>
      );
    });
  };

  return (
    <div ref={box} className="lightmap" style={{ minHeight: Math.max(map.rows.length * 72, map.hosts.length * 48) + 24 }}>
      {geo && (
        <svg className="lm-svg" width={geo.w} height={geo.h} viewBox={`0 0 ${geo.w} ${geo.h}`} role="group" aria-label={t("home.mapLabel")}>
          <g>
            {map.edges.map((edge) => {
              const a = geo.tools[edge.row.tool.id];
              const b = geo.hosts[edge.host.key];
              if (!a || !b) return null;
              const d = curve({ x: a.x + 8, y: a.y }, { x: b.x - 8, y: b.y });
              const result = latency[edge.key];
              const broken = result && result !== "pending" && !result.ok;
              return (
                <g
                  key={edge.key}
                  ref={(el) => { if (el) edgeNodes.current.set(edge.key, el); else edgeNodes.current.delete(edge.key); }}
                  className={cn("lm-edge", focus && focus !== edge.row.tool.id && "dim")}
                >
                  <path className={cn("lm-base", broken && "broken")} d={d} />
                  <path className="lm-glow" d={d} />
                  <path className="lm-main" d={d} />
                  <g className="lm-parts">
                    {Array.from({ length: 6 }, (_, k) => <circle key={k} className="lm-dot" r={2.4} />)}
                  </g>

                </g>
              );
            })}
          </g>
          <g ref={fx} className="pointer-events-none" />
        </svg>
      )}
      <ul className="lm-tools">
        {map.rows.map((row) => {
          const count = row.active.length;
          const sub = row.data?.kind === "switch" ? (row.active[0] ? `→ ${row.active[0].name}` : t("home.noRoute")) : t("home.routes", { count });
          return (
            <li key={row.tool.id} onPointerEnter={() => setFocus(row.tool.id)} onPointerLeave={() => setFocus(null)}>
              <ToolGlyph tool={row.tool} size={30} shared />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[13.5px] font-semibold">{row.tool.label}</span>
                <span className="block truncate font-mono text-[11px] text-primary"><Morph text={sub} /></span>
              </span>
              <span className="lm-node" style={{ ["--glyph" as string]: row.tool.color }} ref={(el) => { if (el) toolNodes.current.set(row.tool.id, el); else toolNodes.current.delete(row.tool.id); }} />
            </li>
          );
        })}
      </ul>
      <ul className="lm-hosts">
        {map.hosts.map((host) => {
          const into = map.edges.filter((e) => e.host.key === host.key);
          const on = into.some((e) => e.active);
          const labels = new Map<string, Edge[]>();
          const probes = new Map<string, Edge>();
          for (const edge of into) {
            const label = edge.provider.name;
            labels.set(label, [...(labels.get(label) ?? []), edge]);
            if (!isOfficial(edge.provider)) probes.set(probeKey(edge.row.tool.id, edge.provider), edge);
          }
          return (
            <li key={host.key} className={cn(focus && !into.some((e) => e.row.tool.id === focus) && "dim")}>
              <span className={cn("lm-host", on && "on")} ref={(el) => { if (el) hostNodes.current.set(host.key, el); else hostNodes.current.delete(host.key); }} />
              <span className="min-w-0 flex-1">
                <span className="flex min-w-0 items-baseline">
                  <span className="truncate font-mono text-[12px]">{hostLabel(host)}</span>
                  {quotaMarks(into)}
                </span>
                <span className="flex flex-wrap text-[11px] text-muted-foreground">
                  {[...labels].map(([label, edges]) => {
                    const sameTool = edges.every((e) => e.row.tool.id === edges[0].row.tool.id);
                    return <span key={label} className="mr-2 inline-flex items-center gap-1">
                      {edges.length > 1 && <span>{sameTool ? `${edges[0].row.tool.label} · ${label}` : label}</span>}
                      {edges.map((edge, index) => {
                        const siblings = edges.filter((e) => e.row.tool.id === edge.row.tool.id);
                        const number = siblings.length > 1 ? ` ${siblings.indexOf(edge) + 1}` : "";
                        const text = edges.length === 1 ? `${edge.row.tool.label} · ${label}` : sameTool ? index + 1 : `${edge.row.tool.label}${number}`;
                        return <button key={edge.key} type="button" aria-pressed={edge.active}
                          aria-label={`${edgeLabel(edge)}${edges.length > 1 ? ` (${index + 1}/${edges.length})` : ""}`}
                          onClick={() => press(edge)}
                          className={cn("rounded px-1 py-1 hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring", edge.active && "font-medium text-foreground")}>
                          {text}
                        </button>;
                      })}
                    </span>;
                  })}
                  {[...probes].map(([key, edge]) => {
                    const result = resultText(edge);
                    if (!result) return null;
                    return <span key={key} className="mr-2">{probes.size > 1 && <span className="font-mono">{key.startsWith("http") ? new URL(key).pathname : edge.provider.name} · </span>}{result}</span>;
                  })}
                </span>
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
