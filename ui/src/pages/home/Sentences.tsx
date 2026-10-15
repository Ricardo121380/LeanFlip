// 首页：每个工具一句话。供应商名就是按钮，点它在同一行展开其他供应商。
// 有额度数据的供应商带上额度：当前供应商后面是进度条加文字，其他的是右上角的小字。
import { useRef, type CSSProperties, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";
import type { ProbeResult, Provider, QuotaSource, ToolId } from "@/lib/api";
import { endpointOf, isOfficial, modelOf } from "@/lib/cards";
import { latencyTone, toneClass } from "@/lib/probe";
import { useFlip } from "@/lib/motion";
import { CountUp } from "@/components/Morph";
import { ToolGlyph, providerColor } from "@/components/Glyph";
import { cn } from "@/lib/utils";
import { agoText, meterWidth, quotaBrief, quotaSettingsOf, type QuotaBrief } from "@/lib/quota";
import { latencyKey, type Latency, type Row } from "./model";

export interface SentenceActions {
  onOpen: (tool: ToolId | null) => void;
  onSwitch: (row: Row, provider: Provider) => void;
  onToggle: (row: Row, provider: Provider, enabled: boolean) => void;
  onImport: (row: Row) => void;
  onRetry: (row: Row) => void;
  onNavigate: (hash: string) => void;
  /** 当前供应商还没设置额度查询时，点“查询额度”。 */
  onDetect: (row: Row, provider: Provider) => void;
}

interface Props extends SentenceActions {
  rows: Row[];
  latency: Latency;
  /** 每张卡（`tool:id`）的额度来源。 */
  quota: Map<string, QuotaSource>;
  now: number;
  open: ToolId | null;
  importing: ToolId | null;
  intro: boolean;
}

/** 链接走应用自己的导航（离开设置页时会确认）。 */
function follow(event: MouseEvent<HTMLAnchorElement>, go: (hash: string) => void) {
  if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  event.preventDefault();
  go(event.currentTarget.hash);
}

export function Sentences({ rows, latency, quota, now, open, importing, intro, ...actions }: Props) {
  const { t } = useTranslation();
  const present = rows.filter((r) => r.state !== "missing");
  const missing = rows.filter((r) => r.state === "missing");
  return (
    <div className={cn("sentences", intro && "intro")}>
      <ul className="divide-y">
        {present.map((row, index) => (
          <Sentence key={row.tool.id} row={row} index={index} latency={latency} quota={quota} now={now} open={open === row.tool.id} importing={importing === row.tool.id} {...actions} />
        ))}
      </ul>
      {missing.length > 0 && (
        <p className="missing-line">
          <span className="mr-1.5">{t("home.missing")}</span>
          {missing.map((row, i) => (
            <span key={row.tool.id}>
              <a href={`#tool=${row.tool.id}`} onClick={(e) => follow(e, actions.onNavigate)}>{row.tool.label}</a>
              {i < missing.length - 1 && <span aria-hidden="true">、</span>}
            </span>
          ))}
        </p>
      )}
    </div>
  );
}

function Latest({ result }: { result?: ProbeResult | "pending" }) {
  const { t } = useTranslation();
  if (!result || result === "pending") return null;
  if (!result.ok) return <span className="lat text-destructive">{t("probe.unreachable")}</span>;
  return (
    <span className={cn("lat", toneClass[latencyTone(result.latencyMs)])}>
      <CountUp value={result.latencyMs} /> ms
    </span>
  );
}

/** 本机记录超过这么久，额度后面标出时间。 */
const STALE_MS = 30 * 60_000;

function Sentence({ row, index, latency, quota, now, open, importing, onOpen, onSwitch, onToggle, onImport, onRetry, onNavigate, onDetect }: SentenceActions & { row: Row; index: number; latency: Latency; quota: Map<string, QuotaSource>; now: number; open: boolean; importing: boolean }) {
  const { t, i18n } = useTranslation();
  const flow = useRef<HTMLDivElement>(null);
  const ghosts = useRef<HTMLDivElement>(null);
  const { tool, data } = row;
  const sourceOf = (p: Provider) => quota.get(`${tool.id}:${p.id}`);
  const briefOf = (p: Provider): QuotaBrief | null => {
    const snapshot = sourceOf(p)?.snapshot;
    return snapshot ? quotaBrief(snapshot, now, t, i18n.language) : null;
  };
  const quotaTitle = (p: Provider) => {
    const source = sourceOf(p);
    const snapshot = source?.snapshot;
    if (!source || !snapshot) return undefined;
    return [source.label, `${t(snapshot.origin === "local" ? "quota.local" : "quota.network")} · ${agoText(snapshot.fetchedAt, now, t)}`].filter(Boolean).join("\n");
  };
  const marks = row.active.map((p) => `${p.id}:${briefOf(p)?.text ?? ""}`).join(",");
  const signature = [row.state, open, row.active.map((p) => p.id).join(","), row.others.map((p) => p.id).join(","), data?.liveError ?? "", marks].join("|");
  useFlip(flow, ghosts, signature, open);

  let order = 1;
  const at = (): CSSProperties => ({ ["--i" as string]: order++ });
  const words: ReactNode[] = [];
  const verb = (key: string, text: string, small = false) => words.push(<span key={key} data-k={key} className={cn("w tok w-verb", small && "small")} style={at()}>{text}</span>);
  const sep = (key: string, text: string) => words.push(<span key={key} data-k={key} className={cn("w tok sep", text === "、" && "comma")} style={at()} aria-hidden="true">{text}</span>);
  const word = (p: Provider, { alt = false, plus = false, label, expanded, onClick, sup = false }: { alt?: boolean; plus?: boolean; label: string; expanded?: boolean; onClick: () => void; sup?: boolean }) => {
    const brief = sup ? briefOf(p) : null;
    const result = latency[latencyKey(tool.id, p.id)];
    const detail = [endpointOf(tool.id, p), modelOf(tool.id, p)].filter(Boolean).join(" · ");
    words.push(
      <button
        key={`p:${p.id}`}
        type="button"
        data-k={`p:${p.id}`}
        className={cn("pw tok", alt && "alt", plus && "plus", !alt && result === "pending" && "probing")}
        style={{ ...at(), ["--pc" as string]: providerColor(p.name) }}
        aria-label={label}
        aria-expanded={expanded}
        title={[detail, brief ? quotaTitle(p) : undefined].filter(Boolean).join("\n") || undefined}
        onClick={onClick}
      >
        <span className="nm">{p.name}</span>
        {!alt && <Latest result={result} />}
        {brief && <span className="qs" data-level={brief.level} title={quotaTitle(p)}>{brief.text}</span>}
        <span className="ul" aria-hidden="true" />
      </button>,
    );
  };

  words.push(
    <a key="tool" data-k="tool" href={`#tool=${tool.id}`} className="w tok w-tool" style={at()} onClick={(e) => follow(e, onNavigate)} title={t("home.manage", { tool: tool.label })}>
      {tool.label}
    </a>,
  );

  if (row.state === "loading") {
    words.push(<span key="loading" data-k="loading" className="w tok skeleton" style={at()} aria-label={t("common.loading")} />);
  } else if (row.state === "error") {
    verb("v", t("home.loadError"));
    words.push(<button key="retry" data-k="retry" type="button" className="w tok link-w" style={at()} onClick={() => onRetry(row)}>{t("common.retry")}</button>);
  } else if (data && data.providers.length === 0) {
    verb("v", t("home.empty"));
    words.push(
      <button key="imp" data-k="imp" type="button" className="w tok link-w" style={at()} disabled={importing} onClick={() => onImport(row)}>
        {importing && <Loader2 className="mr-1 inline size-3.5 animate-spin" aria-hidden="true" />}
        {t(importing ? "home.importing" : "home.importLive")}
      </button>,
    );
    words.push(<a key="add" data-k="add" href={`#tool=${tool.id}`} className="w tok link-w quiet" style={at()} onClick={(e) => follow(e, onNavigate)}>{t("home.addPreset")}</a>);
  } else if (data && data.kind === "switch") {
    const current = row.active[0];
    if (current) {
      verb("v", t("home.using"));
      word(current, {
        label: t(open ? "home.collapse" : "home.change", { tool: tool.label, name: current.name }),
        expanded: open,
        onClick: () => onOpen(open ? null : tool.id),
      });
      const brief = briefOf(current);
      if (brief) {
        const snapshot = sourceOf(current)!.snapshot!;
        const stale = snapshot.origin === "local" && now - snapshot.fetchedAt > STALE_MS;
        words.push(
          <span key="quota" data-k="quota" className="w tok q" data-level={brief.level} style={at()} title={quotaTitle(current)}>
            {brief.percent !== undefined && (
              <span className="quota-meter" data-level={brief.level} aria-hidden="true"><i style={{ width: meterWidth(brief.percent, false) }} /></span>
            )}
            <span className="q-main">{brief.text}</span>
            {brief.note && !open && <span className="q-dim">{brief.note}</span>}
            {stale && <span className="q-age">{agoText(snapshot.fetchedAt, now, t)}</span>}
          </span>,
        );
      } else if (!open && !isOfficial(current) && !quotaSettingsOf(current) && !sourceOf(current)) {
        words.push(
          <button key="quota" data-k="quota" type="button" className="w tok link-w quiet quota-ask" style={at()} onClick={() => onDetect(row, current)}>
            {t("home.quotaDetect")}
          </button>,
        );
      }
      if (open) {
        for (const p of row.others) {
          sep(`s:${p.id}`, "/");
          word(p, { alt: true, sup: true, label: t("home.switchTo", { tool: tool.label, name: p.name }), onClick: () => onSwitch(row, p) });
        }
      } else {
        const endpoint = endpointOf(tool.id, current) ?? t(isOfficial(current) ? "home.official" : "home.clientDefault");
        const meta = [modelOf(tool.id, current), endpoint].filter(Boolean).join(" · ");
        words.push(<span key="meta" data-k="meta" className="w tok w-meta" style={at()}>{meta}</span>);
      }
    } else {
      verb("v", t("home.noCurrent"));
      for (const p of row.others) word(p, { alt: true, plus: true, label: t("home.switchTo", { tool: tool.label, name: p.name }), onClick: () => onSwitch(row, p) });
    }
  } else if (data?.liveError) {
    verb("v", t("home.liveError"));
    words.push(<button key="retry" data-k="retry" type="button" className="w tok link-w" style={at()} onClick={() => onRetry(row)}>{t("common.retry")}</button>);
  } else if (data) {
    verb("v", t(row.active.length ? "home.coexistUsing" : "home.coexistNone"));
    row.active.forEach((p, i) => {
      if (i > 0) sep(`c:${p.id}`, "、");
      word(p, { sup: true, label: t("home.remove", { tool: tool.label, name: p.name }), onClick: () => onToggle(row, p, false) });
    });
    if (row.others.length) {
      verb("v2", t(row.active.length ? "home.canAdd" : "home.canEnable"), true);
      for (const p of row.others) word(p, { alt: true, plus: true, label: t("home.add", { tool: tool.label, name: p.name }), onClick: () => onToggle(row, p, true) });
    }
  }

  const accent = row.active[0] ? providerColor(row.active[0].name) : undefined;
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key !== "Escape" || !open) return;
    event.stopPropagation();
    onOpen(null);
    requestAnimationFrame(() => flow.current?.querySelector<HTMLElement>(".pw:not(.alt)")?.focus());
  };

  return (
    <li
      className={cn("sentence", open && "open", row.active.length > 0 && "live")}
      style={{ ["--r" as string]: Math.min(index, 7), ["--n" as string]: order, ...(accent ? { ["--pc" as string]: accent } : {}) }}
      onKeyDown={onKeyDown}
    >
      <span className="tok sentence-glyph" style={{ ["--i" as string]: 0 }}>
        <ToolGlyph tool={tool} size={28} shared />
      </span>
      <div className="relative min-w-0">
        <div ref={flow} className="flow">{words}</div>
        <div ref={ghosts} className="ghosts" aria-hidden="true" />
      </div>
    </li>
  );
}
