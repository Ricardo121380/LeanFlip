// 首页：清单（每个工具一句话）和光路两个视图。冷启动时清单逐词浮现。
// 清单里正在用的供应商带上额度；标题区可以切换场景、联网刷新额度。
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Gauge, List, Loader2, RefreshCw, Waypoints } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { QueryState } from "@/components/QueryState";
import { QuotaDetectDialog } from "@/components/QuotaDetectDialog";
import { refreshAll, useAction, useAllProviders, useOverview, useQuota, useQuotaRefresh, useSwitchProvider, useToggleLive } from "@/hooks/queries";
import { api, errorMessage, type Provider, type QuotaSettings, type ToolId } from "@/lib/api";
import { agoText, quotaSettingsOf, sourcesByCard } from "@/lib/quota";
import { isOfficial } from "@/lib/cards";
import { settle } from "@/lib/probe";
import { prefersReducedMotion } from "@/lib/motion";
import { TOOLS } from "@/lib/tools";
import { cn } from "@/lib/utils";
import { buildRows, probeGroups, type Latency, type Row } from "./home/model";
import { LightMap } from "./home/LightMap";
import { Sentences } from "./home/Sentences";
import { SceneMenu } from "./home/SceneMenu";

export type HomeMode = "list" | "map";

// 开场只在启动后第一次进入首页时播放。
let introPlayed = false;
const INTRO_MS = 1900;

export function HomePage({ mode, onNavigate }: { mode: HomeMode; onNavigate: (hash: string, options?: { transition?: boolean }) => void }) {
  const { t } = useTranslation();
  const overview = useOverview().data;
  const results = useAllProviders();
  const rows = useMemo(() => buildRows(TOOLS, results, overview ?? []), [results, overview]);
  const ready = overview !== undefined && rows.every((r) => r.state !== "loading");
  const [open, setOpen] = useState<ToolId | null>(null);
  const [latency, setLatency] = useState<Latency>({});
  const [testing, setTesting] = useState(false);
  const run = useRef(0);
  const [intro, setIntro] = useState(() => mode === "list" && !introPlayed && !prefersReducedMotion());
  const client = useQueryClient();
  const quota = useQuota();
  const refreshQuota = useQuotaRefresh();
  const cards = useMemo(() => sourcesByCard(quota.data), [quota.data]);
  const [detecting, setDetecting] = useState<{ tool: ToolId; provider: Provider } | null>(null);
  const now = Date.now();
  const lastOnline = Math.max(0, ...(quota.data?.sources ?? []).flatMap((s) => (s.snapshot?.origin === "network" ? [s.snapshot.fetchedAt] : [])));
  const configureQuota = async (target: { tool: ToolId; provider: Provider }, adapter: QuotaSettings["adapter"]) => {
    try {
      await api.quotaConfigure(target.tool, target.provider.id, { ...(quotaSettingsOf(target.provider) ?? { refreshMinutes: 0 }), adapter });
    } finally {
      void refreshAll(client);
    }
  };

  const switchAction = useSwitchProvider({ optimistic: true });
  const toggleAction = useToggleLive({ optimistic: true });
  const importAction = useAction(
    (tool: ToolId) => api.importLive(tool),
    (r) => (r.created.length ? toast.success(t("tool.imported", { count: r.created.length })) : toast.info(t("tool.importedNone"))),
  );

  // 数据齐了才开场；点击或按键直接跳到结果。
  useEffect(() => {
    if (!intro || !ready) return;
    introPlayed = true;
    const stop = () => setIntro(false);
    const timer = setTimeout(stop, INTRO_MS);
    window.addEventListener("pointerdown", stop, { once: true });
    window.addEventListener("keydown", stop, { once: true });
    return () => {
      clearTimeout(timer);
      window.removeEventListener("pointerdown", stop);
      window.removeEventListener("keydown", stop);
    };
  }, [intro, ready]);
  useEffect(() => {
    if (mode !== "list") setIntro(false);
  }, [mode]);

  // 点句子外面收起展开的供应商。
  useEffect(() => {
    if (!open) return;
    const close = (event: PointerEvent) => {
      if (!(event.target as Element | null)?.closest?.(".sentence.open")) setOpen(null);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, [open]);

  useEffect(() => () => { run.current += 1; }, []);

  const onSwitch = (row: Row, provider: Provider) => {
    setOpen(null);
    const previous = row.active[0];
    switchAction.mutate({ tool: row.tool.id, id: provider.id, name: provider.name, previous: previous ? { id: previous.id, name: previous.name } : null });
  };
  const onToggle = (row: Row, provider: Provider, enabled: boolean) => toggleAction.mutate({ tool: row.tool.id, id: provider.id, name: provider.name, enabled });
  const onRetry = (row: Row) => void results[TOOLS.indexOf(row.tool)]?.refetch();

  const testAll = async () => {
    const targets = probeGroups(rows);
    if (targets.length === 0) return;
    const current = ++run.current;
    setTesting(true);
    setLatency(Object.fromEntries(targets.flatMap(({ keys }) => keys.map((key) => [key, "pending" as const]))));
    const queue = [...targets];
    const worker = async () => {
      for (let next = queue.shift(); next; next = queue.shift()) {
        const { tool, provider, keys } = next;
        const result = await settle(() => api.probeLatency(tool, provider.settingsConfig));
        if (run.current === current) setLatency((s) => ({ ...s, ...Object.fromEntries(keys.map((key) => [key, result])) }));
      }
    };
    await Promise.all(Array.from({ length: Math.min(4, queue.length) }, worker));
    if (run.current === current) setTesting(false);
  };

  const testable = rows.some((row) => row.data?.providers.some((p) => !isOfficial(p)));
  const views: { mode: HomeMode; label: string; icon: typeof List }[] = [
    { mode: "list", label: t("home.list"), icon: List },
    { mode: "map", label: t("home.map"), icon: Waypoints },
  ];

  return (
    <div className="mx-auto flex w-full min-w-0 max-w-5xl flex-col gap-5 px-6 py-7 lg:px-8">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0">
          <h1 className="text-[22px] font-semibold leading-tight tracking-tight">{t("home.title")}</h1>
          <p className="mt-0.5 text-xs leading-5 text-muted-foreground">{t(mode === "list" ? "home.subtitle" : "home.mapHint")}</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <SceneMenu />
          <div role="group" aria-label={t("home.views")} className="segmented">
            {views.map(({ mode: value, label, icon: Icon }) => (
              <button
                key={value}
                type="button"
                aria-pressed={mode === value}
                className={cn(mode === value && "on")}
                onClick={() => mode !== value && onNavigate(value === "list" ? "#home" : "#map", { transition: true })}
              >
                <Icon aria-hidden="true" />
                {label}
              </button>
            ))}
          </div>
          {mode === "list" && (quota.data?.sources.length ?? 0) > 0 && (
            <Button size="sm" variant="outline" className="h-8" disabled={refreshQuota.isPending} title={t("quota.refreshHint")} onClick={() => refreshQuota.mutate(undefined)}>
              {refreshQuota.isPending ? <Loader2 className="animate-spin" /> : <RefreshCw />}
              {refreshQuota.isPending ? t("quota.refreshing") : lastOnline ? t("quota.checked", { ago: agoText(lastOnline, now, t) }) : t("quota.refresh")}
            </Button>
          )}
          {mode === "map" && (
            <Button size="sm" variant="outline" className="h-8" disabled={testing || !testable} onClick={() => void testAll()} title={t("probe.latencyAllHint")}>
              {testing ? <Loader2 className="animate-spin" /> : <Gauge />} {t(testing ? "home.testing" : "probe.latencyAll")}
            </Button>
          )}
        </div>
      </header>

      {overview === undefined && rows.every((r) => r.state === "loading") ? (
        <QueryState state="loading" title={t("common.loading")} />
      ) : mode === "list" ? (
        <section aria-label={t("home.list")} className="home-panel">
          <Sentences
            rows={rows}
            latency={latency}
            open={open}
            importing={importAction.isPending ? (importAction.variables ?? null) : null}
            intro={intro && ready}
            quota={cards}
            now={now}
            onDetect={(row, provider) => setDetecting({ tool: row.tool.id, provider })}
            onOpen={setOpen}
            onSwitch={onSwitch}
            onToggle={onToggle}
            onImport={(row) => importAction.mutate(row.tool.id)}
            onRetry={onRetry}
            onNavigate={(hash) => onNavigate(hash)}
          />
        </section>
      ) : (
        <section aria-label={t("home.map")} className="home-panel">
          <LightMap rows={rows} latency={latency} quota={cards} now={now} onSwitch={onSwitch} onToggle={onToggle} />
        </section>
      )}
      {detecting && (
        <QuotaDetectDialog
          tool={detecting.tool}
          name={detecting.provider.name}
          settingsConfig={detecting.provider.settingsConfig}
          queryBaseUrl={quotaSettingsOf(detecting.provider)?.queryBaseUrl}
          onUse={(adapter) => configureQuota(detecting, adapter)}
          onDisable={() => void configureQuota(detecting, "off").catch((error) => toast.error(errorMessage(error)))}
          onClose={() => setDetecting(null)}
        />
      )}
    </div>
  );
}
