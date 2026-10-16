import { useConfirm } from "@/components/ConfirmDialog";
import { QueryState } from "@/components/QueryState";
import { lazy, Suspense, useEffect, useRef, useState, type ReactNode } from "react";
import { useIsMutating } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { AlertTriangle, ArrowRightLeft, Copy, CopyPlus, Download, Gauge, FileCode2, FolderSearch, Loader2, Pencil, Plus, RefreshCw, Search, Share2, Trash2, UserRound, X } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
// 编辑器（CodeMirror）按需加载。
const CommonConfigEditor = lazy(() => import("@/components/CommonConfigEditor").then((m) => ({ default: m.CommonConfigEditor })));
const ProviderEditor = lazy(() => import("@/components/ProviderEditor").then((m) => ({ default: m.ProviderEditor })));
const SyncDialog = lazy(() => import("@/components/SyncDialog").then((m) => ({ default: m.SyncDialog })));
import { useAccounts, useAction, useLogins, useOverview, useProviders, useQuota, useQuotaRefresh, useSwitchProvider } from "@/hooks/queries";
import { api, errorMessage, type AccountInfo, type ProbeResult, type Provider, type QuotaSource, type ToolId } from "@/lib/api";
import { agoText, errorText, quotaBrief, quotaSettingsOf, quotaTitle, sourcesByCard } from "@/lib/quota";
import { QuotaMeter } from "@/components/QuotaDetails";
import { accountLabel, endpointOf, isOfficial, modelOf } from "@/lib/cards";
import { syncSource, type SyncSource } from "@/lib/sync";
import { failureText, latencyTone, providerDetail, settle, toneClass } from "@/lib/probe";
import { toolMeta } from "@/lib/tools";
import { Monogram, ToolGlyph } from "@/components/Glyph";
import { Morph } from "@/components/Morph";
import { cn } from "@/lib/utils";

function LoginBadge({ provider, accounts }: { provider: Provider; accounts: AccountInfo[] }) {
  const { t } = useTranslation();
  const binding = provider.meta?.authBinding;
  if (binding?.authProvider === "codex_oauth") {
    const account = accounts.find((a) => a.id === binding.accountId);
    const label = account ? accountLabel(account) : t("card.managedDefault");
    return (
      <Badge variant="secondary" className="min-w-0 max-w-full gap-1" title={label}>
        <UserRound className="h-3 w-3 shrink-0" />
        <span className="truncate">{label}</span>
      </Badge>
    );
  }
  if (provider.meta?.codexLoginRef) {
    return <Badge variant="outline">{t("card.loginRef")}</Badge>;
  }
  return null;
}

/** 列表行里的测连通结果：延迟按快慢着色，失败时悬停可看原因。 */
function LatencyChip({ result }: { result?: ProbeResult | "pending" }) {
  const { t } = useTranslation();
  if (!result) return null;
  if (result === "pending") {
    return <span role="status" aria-label={t("probe.running_latency")} className="mr-1.5 inline-flex w-14 justify-end text-muted-foreground"><Loader2 className="h-3.5 w-3.5 animate-spin" /></span>;
  }
  const detail = [result.ok ? `${t("probe.reachable")} · HTTP ${result.status ?? "?"}` : failureText(t, result, "latency"), providerDetail(result), result.endpoint].filter(Boolean).join("\n");
  return (
    <span
      role="status"
      title={detail}
      className={cn("mr-1.5 inline-flex min-w-14 items-center justify-end gap-1.5 text-xs font-medium tabular-nums", result.ok ? toneClass[latencyTone(result.latencyMs)] : "text-destructive")}
    >
      <span className="h-1.5 w-1.5 rounded-full bg-current" aria-hidden="true" />
      {result.ok ? t("probe.ms", { ms: result.latencyMs }) : t("probe.unreachable")}
    </span>
  );
}

function Notice({ title, children, onRetry }: { title: string; children?: ReactNode; onRetry?: () => void }) {
  const { t } = useTranslation();
  return (
    <div role="alert" className="flex min-w-0 items-start gap-3 rounded-lg border border-warning/30 bg-warning/[0.07] px-4 py-3 text-sm">
      <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" aria-hidden="true" />
      <div className="min-w-0 flex-1">
        <p className="font-medium">{title}</p>
        {children}
        {onRetry && <Button variant="outline" size="sm" className="mt-2" onClick={onRetry}>{t("common.retry")}</Button>}
      </div>
    </div>
  );
}

function ProviderQuota({ source, name, now, refreshing, onRefresh }: { source: QuotaSource; name: string; now: number; refreshing: boolean; onRefresh: () => void }) {
  const { t, i18n } = useTranslation();
  const brief = source.snapshot ? quotaBrief(source.snapshot, now, t, i18n.language) : null;
  const online = source.kind === "relay" || source.accountId !== undefined;
  return (
    <div className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground" role="group" aria-label={t("quota.windowsLabel", { label: name })}>
      <span className={cn("font-mono tabular-nums", brief?.level === "warn" && "text-warning", brief?.level === "out" && "text-destructive", brief?.level === "ok" && "text-foreground")} title={quotaTitle(source, now, t)}>{brief?.text ?? t("quota.pending")}</span>
      {brief?.percent !== undefined && <QuotaMeter usedPercent={brief.percent} reset={false} className="w-16" />}
      {brief?.note && <span>{brief.note}</span>}
      {source.snapshot && <span>{agoText(source.snapshot.fetchedAt, now, t)}</span>}
      {source.error && <span role="status" className="inline-flex items-center gap-1 text-warning"><AlertTriangle className="size-3 shrink-0" />{errorText(source.error, t)}</span>}
      {online && <button type="button" disabled={refreshing} onClick={onRefresh} aria-label={t("quota.refreshFor", { label: name })} title={t("quota.refreshFor", { label: name })} className="inline-flex size-6 items-center justify-center rounded hover:bg-accent hover:text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring disabled:opacity-50">
        {refreshing ? <Loader2 className="size-3 animate-spin" /> : <RefreshCw className="size-3" />}
      </button>}
    </div>
  );
}

export function ToolPage({ tool }: { tool: ToolId }) {
  const { t } = useTranslation();
  const meta = toolMeta(tool);
  const { data, isLoading, error, refetch } = useProviders(tool);
  // Only the empty state needs to know whether the client config exists.
  const summary = useOverview(data?.providers.length === 0).data?.find((s) => s.tool === tool);
  const accounts = useAccounts(tool === "codex").data ?? [];
  const logins = useLogins(tool === "codex").data ?? [];
  const quota = useQuota();
  const refreshQuota = useQuotaRefresh();
  const quotaByCard = sourcesByCard(quota.data);
  const quotaKeys = [...new Set((quota.data?.sources ?? []).filter((s) => s.users.some((u) => u.tool === tool) && (s.kind === "relay" || s.accountId)).map((s) => s.key))];
  const now = Date.now();
  const [editing, setEditing] = useState<Provider | null>(null);
  const [copying, setCopying] = useState<Provider | null>(null);
  const [syncing, setSyncing] = useState<SyncSource | null>(null);
  // 新建的卡能同步时，保存后提议同步到其他 Agent。
  const onCreated = (provider: Provider) => setSyncing(syncSource(tool, provider));
  // 测连通结果只在本页保留，切换工具或刷新页面后清空。
  const [latency, setLatency] = useState<Record<string, ProbeResult | "pending">>({});
  const [testing, setTesting] = useState(false);
  const run = useRef(0);
  const [commonOpen, setCommonOpen] = useState(false);
  const [adding, setAdding] = useState(false);
  const [search, setSearch] = useState("");
  const searchInput = useRef<HTMLInputElement>(null);
  const [confirm, confirmDialog] = useConfirm();
  const busy = useIsMutating() > 0;

  useEffect(() => {
    setSearch("");
    setEditing(null);
    setCopying(null);
    setAdding(false);
    setLatency({});
    setTesting(false);
    run.current += 1;
  }, [tool]);

  const switchAction = useSwitchProvider();
  const toggleAction = useAction(
    ({ id, enabled }: { id: string; name: string; enabled: boolean }) => api.setInLive(tool, id, enabled),
    (_r, { name, enabled }) => toast.success(t(enabled ? "card.enabled" : "card.disabled", { name })),
  );
  const defaultAction = useAction(
    ({ id }: { id: string; name: string }) => api.switchProvider(tool, id),
    (_r, { name }) => toast.success(t("card.switched", { name })),
  );
  const deleteAction = useAction((id: string) => api.deleteProvider(tool, id));
  const importAction = useAction(
    () => api.importLive(tool),
    (r) => (r.created.length ? toast.success(t("tool.imported", { count: r.created.length })) : toast.info(t("tool.importedNone"))),
  );

  if (isLoading) return <QueryState state="loading" title={t("tool.loading", { tool: meta.label })} />;
  if (!data)
    return <QueryState state="error" title={t("tool.loadError", { tool: meta.label })} description={error ? errorMessage(error) : t("tool.noData")} onRetry={() => void refetch()} />;

  const coexist = data.kind === "coexist";
  const currentProvider = data.providers.find((p) => p.id === data.current);
  const enabledCount = data.providers.filter((p) => data.inLive.includes(p.id)).length;
  const query = search.trim().toLocaleLowerCase();
  const providers = data.providers.filter((p) =>
    [p.name, p.id, endpointOf(tool, p), modelOf(tool, p), p.notes].some((value) => typeof value === "string" && value.toLocaleLowerCase().includes(query)),
  );
  const detected = summary?.detected ?? true;
  const clearSearch = () => {
    setSearch("");
    searchInput.current?.focus();
  };
  const confirmDelete = async (p: Provider) => {
    const ok = await confirm({
      title: t("card.confirmDelete", { name: p.name }),
      description: t(coexist ? "card.confirmDeleteCoexistHint" : "card.confirmDeleteHint"),
      confirmLabel: t("card.confirmDeleteAction"),
      destructive: true,
    });
    if (ok) deleteAction.mutate(p.id);
  };
  const testAll = async () => {
    const targets = data.providers.filter((p) => !isOfficial(p));
    const current = ++run.current;
    setTesting(true);
    setLatency(Object.fromEntries(targets.map((p) => [p.id, "pending" as const])));
    const queue = [...targets];
    const worker = async () => {
      for (let next = queue.shift(); next; next = queue.shift()) {
        const provider = next;
        const result = await settle(() => api.probeLatency(tool, provider.settingsConfig));
        if (run.current === current) setLatency((s) => ({ ...s, [provider.id]: result }));
      }
    };
    await Promise.all(Array.from({ length: Math.min(4, queue.length) }, worker));
    if (run.current === current) setTesting(false);
  };
  const copyPath = async (path: string) => {
    try {
      if (!navigator.clipboard?.writeText) throw new Error("clipboard unavailable");
      await navigator.clipboard.writeText(path);
      toast.success(t("tool.pathCopied"));
    } catch {
      toast.error(t("tool.pathCopyFailed"));
    }
  };
  const importButton = (primary = false) => (
    <Button variant={primary ? "default" : "outline"} size="sm" onClick={() => importAction.mutate(undefined)} disabled={busy}>
      {importAction.isPending ? <Loader2 className="animate-spin" /> : <Download />} {t(importAction.isPending ? "tool.importing" : primary ? "tool.importFirst" : "tool.importLive")}
    </Button>
  );
  const addButton = (label = t("tool.add"), variant: "default" | "outline" = "default") => (
    <Button size="sm" variant={variant} onClick={() => setAdding(true)} disabled={busy}>
      <Plus /> {label}
    </Button>
  );
  const subline = (p: Provider) => {
    const endpoint = endpointOf(tool, p);
    const model = modelOf(tool, p);
    return [coexist ? p.id : endpoint ?? p.id, coexist ? endpoint : undefined, model].filter(Boolean).join(" · ");
  };
  const rowActions = (p: Provider, current: boolean) => {
    const deleting = deleteAction.isPending && deleteAction.variables === p.id;
    const sync = syncSource(tool, p);
    return (
      <>
        {sync && (
          <Button size="icon" variant="ghost" className="text-muted-foreground opacity-70 group-hover:opacity-100 focus-visible:opacity-100" disabled={busy} onClick={() => setSyncing(sync)} aria-label={`${t("sync.action")} ${p.name}`} title={t("sync.action")}>
            <Share2 />
          </Button>
        )}
        <Button size="icon" variant="ghost" className="text-muted-foreground opacity-70 group-hover:opacity-100 focus-visible:opacity-100" disabled={busy} onClick={() => setCopying(p)} aria-label={`${t("card.duplicate")} ${p.name}`} title={t("card.duplicate")}>
          <CopyPlus />
        </Button>
        <Button size="icon" variant="ghost" className="text-muted-foreground opacity-70 group-hover:opacity-100 focus-visible:opacity-100" disabled={busy} onClick={() => setEditing(p)} aria-label={`${t("card.edit")} ${p.name}`} title={t("card.edit")}>
          <Pencil />
        </Button>
        <Button
          size="icon"
          variant="ghost"
          className="text-muted-foreground opacity-70 hover:text-destructive group-hover:opacity-100 focus-visible:opacity-100"
          disabled={current || p.id === data.selected || busy}
          onClick={() => void confirmDelete(p)}
          aria-label={deleting ? t("card.deleting", { name: p.name }) : `${t("card.delete")} ${p.name}`}
          title={current ? t("card.currentDeleteHint") : p.id === data.selected ? t("card.selectedDeleteHint") : t("card.delete")}
        >
          {deleting ? <Loader2 className="animate-spin" /> : <Trash2 />}
        </Button>
      </>
    );
  };

  const pathList = (
    <ul aria-label={t("tool.paths")} className="flex min-w-0 flex-col gap-1">
      {data.paths.map((path) => (
        <li key={path} className="group/path flex min-w-0 items-center gap-1.5 text-muted-foreground">
          <FileCode2 className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
          <code className="min-w-0 truncate" title={path}>{path}</code>
          <button type="button" className="grid h-6 w-6 shrink-0 place-items-center rounded opacity-60 transition hover:bg-accent hover:text-foreground hover:opacity-100 group-hover/path:opacity-100" onClick={() => void copyPath(path)} aria-label={t("tool.copyPath", { path })} title={t("tool.copyPath", { path })}>
            <Copy className="h-3.5 w-3.5" />
          </button>
        </li>
      ))}
    </ul>
  );
  const enabledProviders = data.providers.filter((p) => data.inLive.includes(p.id));
  // 路由牌：配置文件 ──▶ 当前供应商。整页的视觉重心，切换时名称溶解换字并扫过一道光。
  const routePlate = (
    <section aria-label={t("tool.route")} className="route-plate overflow-hidden rounded-2xl border px-5 pb-4 pt-4 shadow-[0_1px_2px_hsl(var(--overlay)/0.04),0_8px_24px_-12px_hsl(var(--overlay)/0.12)]">
      <div className="flex min-w-0 items-start justify-between gap-4">
        <div className="min-w-0">
          <p className="text-xs font-medium text-muted-foreground">
            {coexist ? t("tool.routeCoexist", { count: data.liveError ? 0 : enabledCount, total: data.providers.length }) : t(currentProvider ? "tool.routeCurrent" : "tool.routeNone")}
          </p>
          {!coexist && currentProvider ? (
            <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-2.5">
              <Monogram key={currentProvider.id} name={currentProvider.name} size={34} className="pop-in" />
              <h2 className="min-w-0 text-[24px] font-semibold leading-tight tracking-tight"><Morph text={currentProvider.name} /></h2>
              {isOfficial(currentProvider) && <Badge variant="outline">{t("card.official")}</Badge>}
              {tool === "codex" && <LoginBadge provider={currentProvider} accounts={accounts} />}
            </div>
          ) : coexist && enabledProviders.length > 0 && !data.liveError ? (
            <ul className="mt-2 flex flex-wrap gap-1.5">
              {enabledProviders.map((p) => (
                <li key={p.id} className="rise-in inline-flex items-center gap-1.5 rounded-lg border bg-card py-1 pl-1 pr-2.5 text-sm font-medium">
                  <Monogram name={p.name} size={22} />
                  {p.name}
                </li>
              ))}
            </ul>
          ) : (
            <p className="mt-1.5 text-[22px] font-semibold leading-tight tracking-tight text-muted-foreground/70">{t(coexist ? "tool.routeEmptyCoexist" : "tool.routeEmpty")}</p>
          )}
        </div>
      </div>
      <div className="mt-4 flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2 border-t border-dashed pt-3.5 font-mono text-xs sm:flex-nowrap">
        <div className="min-w-0 max-w-full shrink">{data.paths.length ? pathList : <span className="text-muted-foreground">{t("tool.noPaths")}</span>}</div>
        <span aria-hidden="true" className={cn("route-line hidden flex-1 sm:block", !(coexist ? enabledCount : currentProvider) && "route-line-idle")} />
        <div className="min-w-0 max-w-full shrink truncate text-foreground/80" title={currentProvider ? subline(currentProvider) : undefined}>
          {coexist
            ? t("tool.routeEntries", { count: data.liveError ? 0 : enabledCount })
            : currentProvider
              ? [endpointOf(tool, currentProvider) ?? t(isOfficial(currentProvider) ? "tool.routeOfficial" : "tool.routeClientDefault"), modelOf(tool, currentProvider)].filter(Boolean).join(" · ")
              : t("tool.routeNoTarget")}
        </div>
      </div>
    </section>
  );

  return (
    <div className="mx-auto flex w-full min-w-0 max-w-5xl flex-col gap-6 px-6 py-7 lg:px-8">
      <header className="flex flex-col gap-2">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex min-w-0 items-center gap-3">
            <ToolGlyph tool={meta} size={34} />
            <div className="min-w-0">
              <h1 className="truncate text-[22px] font-semibold leading-tight tracking-tight">{meta.label}</h1>
              <p className="mt-0.5 max-w-prose text-xs leading-5 text-muted-foreground">{coexist ? t("tool.coexistHint") : t("tool.switchHint")}</p>
            </div>
          </div>
          <div className="flex flex-wrap gap-2"><Button variant="outline" size="sm" disabled={busy} onClick={() => setCommonOpen(true)}>{t("config.commonTitle")}</Button>{data.providers.length > 0 && <>{importButton()}{addButton()}</>}</div>
        </div>
        {data.providers.length === 0 && (data.paths.length === 0 ? <p className="text-xs text-muted-foreground">{t("tool.noPaths")}</p> : <div className="font-mono text-xs">{pathList}</div>)}
      </header>

      {error && (
        <Notice title={t("tool.refreshError")} onRetry={() => void refetch()}>
          <p className="mt-1 text-xs leading-relaxed text-muted-foreground">{t("tool.refreshErrorHint")}</p>
          <p className="mt-1 break-words text-xs leading-relaxed text-muted-foreground [overflow-wrap:anywhere]">{errorMessage(error)}</p>
        </Notice>
      )}
      {!coexist && data.selected && data.current !== data.selected && !data.liveError && (
        <Notice title={t("tool.driftTitle")} onRetry={() => void refetch()}><p className="text-xs text-muted-foreground">{t("tool.driftHint")}</p></Notice>
      )}
      {data.liveError && (
        <Notice title={t("tool.liveErrorTitle")} onRetry={() => void refetch()}>
          <p className="mt-1 break-words text-xs leading-relaxed text-muted-foreground [overflow-wrap:anywhere]">{t("tool.liveError")} {data.liveError}</p>
        </Notice>
      )}

      {data.providers.length === 0 ? (
        <section className="flex flex-col items-start gap-3 rounded-2xl border border-dashed px-6 py-7">
          <div className="grid h-9 w-9 place-items-center rounded-lg bg-muted text-muted-foreground" aria-hidden="true">
            {detected ? <FileCode2 className="h-[18px] w-[18px]" /> : <FolderSearch className="h-[18px] w-[18px]" />}
          </div>
          <h2 className="text-base font-semibold">{detected ? t("tool.emptyTitle", { tool: meta.label }) : t("tool.emptyMissingTitle", { tool: meta.label })}</h2>
          <p className="max-w-prose break-words text-sm leading-relaxed text-muted-foreground">
            {detected ? t("tool.emptyDetected", { path: data.paths[0] ?? "" }) : t("tool.emptyMissing", { path: data.paths[0] ?? "" })}
          </p>
          <div className="mt-1 flex flex-wrap gap-2">
            {detected ? (
              <>{importButton(true)}{addButton(t("tool.addFromPreset"), "outline")}</>
            ) : (
              <>
                <Button size="sm" variant="outline" onClick={() => { window.location.hash = "#settings"; }}><FolderSearch /> {t("tool.pickDir")}</Button>
                {addButton(t("tool.addFromPreset"), "outline")}
              </>
            )}
          </div>
        </section>
      ) : (
        <>
        {routePlate}
        <section className="flex min-w-0 flex-col gap-3" aria-label={t("tool.providerCount", { count: data.providers.length })}>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex min-w-0 items-baseline gap-2">
              <h2 className="text-sm font-semibold tracking-tight">{t("tool.allProviders")}</h2>
              <p className="text-xs text-muted-foreground" role="status">
                {query ? t("tool.searchCount", { count: providers.length, total: data.providers.length }) : t("tool.providerCount", { count: data.providers.length })}
                {coexist && !query && !data.liveError && <span> · {t("tool.enabledSummary", { count: enabledCount, total: data.providers.length })}</span>}
                {data.liveError && <span className="text-warning"> · {t("tool.stateUnknown")}</span>}
                {!coexist && !query && !currentProvider && <span> · {t("tool.noCurrent")}</span>}
              </p>
            </div>
            <div className="flex w-full max-w-md flex-wrap items-center gap-2">
              <Button size="sm" variant="outline" className="h-9 shrink-0" disabled={refreshQuota.isPending || quotaKeys.length === 0} onClick={() => refreshQuota.mutate(quotaKeys)}>
                {refreshQuota.isPending ? <Loader2 className="animate-spin" /> : <RefreshCw />} {t("quota.refresh")}
              </Button>
              <Button size="sm" variant="outline" className="h-9 shrink-0" disabled={testing || busy || data.providers.every(isOfficial)} onClick={() => void testAll()} title={t("probe.latencyAllHint")}>
                {testing ? <Loader2 className="animate-spin" /> : <Gauge />} {t("probe.latencyAll")}
              </Button>
              <div className="relative min-w-[8rem] flex-1">
                <Search className="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" aria-hidden="true" />
                <Input ref={searchInput} type="search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder={t("tool.search")} aria-label={t("tool.search")} className="h-9 pl-9 pr-9 [&::-webkit-search-cancel-button]:appearance-none" />
                {search && <Button size="icon" variant="ghost" className="absolute right-0.5 top-0.5" aria-label={t("tool.clearSearch")} onClick={clearSearch}><X /></Button>}
              </div>
            </div>
          </div>

          {quota.isError && <p className="text-xs text-warning">{errorMessage(quota.error)} <button type="button" className="underline" onClick={() => void quota.refetch()}>{t("common.retry")}</button></p>}

          {providers.length === 0 ? (
            <div className="flex flex-col items-center gap-2 rounded-2xl border border-dashed p-8 text-center">
              <h2 className="font-medium">{t("tool.noResults")}</h2>
              <p className="max-w-prose break-words text-sm text-muted-foreground [overflow-wrap:anywhere]">{t("tool.noResultsHint", { query: search.trim() })}</p>
              <Button variant="outline" size="sm" className="mt-2" onClick={clearSearch}>{t("tool.clearSearch")}</Button>
            </div>
          ) : (
            <ul className="divide-y overflow-hidden rounded-2xl border bg-card">
              {providers.map((p) => {
                const current = !coexist && data.current === p.id;
                const inLive = coexist && data.inLive.includes(p.id);
                const switching = switchAction.isPending && switchAction.variables?.id === p.id && switchAction.variables?.tool === tool;
                const toggling = toggleAction.isPending && toggleAction.variables?.id === p.id;
                const settingDefault = defaultAction.isPending && defaultAction.variables?.id === p.id;
                const source = quotaByCard.get(`${tool}:${p.id}`);
                return (
                  <li
                    key={p.id}
                    data-testid={`row-${p.id}`}
                    className={cn(
                      "group relative flex min-w-0 flex-wrap items-center gap-3 px-4 py-3 transition-colors hover:bg-accent/40 sm:flex-nowrap",
                      (current || inLive) && "before:absolute before:inset-y-3 before:left-0 before:w-[3px] before:rounded-r-full before:bg-primary",
                    )}
                  >
                    <Monogram name={p.name} size={30} />
                    <div className="min-w-0 flex-1 basis-[calc(100%-3rem)] sm:basis-0">
                      <div className="flex flex-wrap items-center gap-1.5">
                        <span className="min-w-0 break-words font-medium [overflow-wrap:anywhere]">{p.name}</span>
                        {current && <Badge>{t("card.active")}</Badge>}
                        {toggling ? (
                          <Badge variant="secondary" className="gap-1"><Loader2 className="h-3 w-3 animate-spin" />{t(toggleAction.variables?.enabled ? "card.enabling" : "card.disabling")}</Badge>
                        ) : inLive && <Badge variant="success">{t("card.inLive")}</Badge>}
                        {isOfficial(p) && <Badge variant="outline">{t("card.official")}</Badge>}
                        {tool === "codex" && <LoginBadge provider={p} accounts={accounts} />}
                      </div>
                      <p className="mt-0.5 truncate font-mono text-xs text-muted-foreground" title={subline(p)}>{subline(p)}</p>
                      {p.notes && <p className="mt-0.5 truncate text-xs text-muted-foreground" title={p.notes}>{p.notes}</p>}
                      {source && <ProviderQuota source={source} name={p.name} now={now} refreshing={refreshQuota.isPending && (!refreshQuota.variables || refreshQuota.variables.includes(source.key))} onRefresh={() => refreshQuota.mutate([source.key])} />}
                      {!source && quotaSettingsOf(p)?.adapter === "off" && <p className="mt-1.5 text-xs text-muted-foreground">{t("quota.off")}</p>}
                    </div>
                    <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
                      {rowActions(p, current)}
                    </div>
                    <LatencyChip result={latency[p.id]} />
                    {coexist ? (
                      <div className="flex shrink-0 items-center gap-2">
                        {tool === "hermes" && (
                          <Button size="sm" variant="ghost" className="text-primary" disabled={Boolean(data.liveError) || busy} onClick={() => defaultAction.mutate({ id: p.id, name: p.name })}>
                            {settingDefault && <Loader2 className="animate-spin" />}{t(settingDefault ? "card.switching" : "card.setDefaultModel")}
                          </Button>
                        )}
                        <Switch checked={inLive} disabled={Boolean(data.liveError) || busy} onCheckedChange={(enabled) => toggleAction.mutate({ id: p.id, name: p.name, enabled })} aria-label={t("card.toggleInLive", { name: p.name })} />
                      </div>
                    ) : !current ? (
                      <Button size="sm" variant="outline" className="shrink-0 bg-card" disabled={busy} onClick={() => switchAction.mutate({ tool, id: p.id, name: p.name, previous: currentProvider ? { id: currentProvider.id, name: currentProvider.name } : null })} aria-label={switching ? t("card.switching") : t("card.switchTo", { name: p.name })}>
                        {switching ? <Loader2 className="animate-spin" /> : <ArrowRightLeft />} {t(switching ? "card.switching" : "card.switch")}
                      </Button>
                    ) : (
                      <span className="w-[4.75rem] shrink-0" aria-hidden="true" />
                    )}
                  </li>
                );
              })}
            </ul>
          )}
        </section>
        </>
      )}
      <Suspense fallback={<p role="status" className="text-sm text-muted-foreground">{t("tool.loadingEditor")}</p>}>
        {adding && <ProviderEditor tool={tool} open onOpenChange={setAdding} existingIds={data.providers.map((p) => p.id)} accounts={accounts} logins={logins} onCreated={onCreated} />}
        {copying && <ProviderEditor key={`copy-${copying.id}`} tool={tool} open onOpenChange={(open) => !open && setCopying(null)} template={copying} existingIds={data.providers.map((p) => p.id)} accounts={accounts} logins={logins} onCreated={onCreated} />}
        {syncing && <SyncDialog key={`${syncing.tool}-${syncing.provider.id}`} source={syncing} onClose={() => setSyncing(null)} />}
        {editing && <ProviderEditor key={editing.id} tool={tool} open onOpenChange={(open) => !open && setEditing(null)} provider={editing} existingIds={data.providers.map((p) => p.id)} accounts={accounts} logins={logins} />}
      </Suspense>
      {commonOpen && <Suspense fallback={<p role="status">{t("common.loading")}</p>}><CommonConfigEditor tool={tool} onClose={() => setCommonOpen(false)} /></Suspense>}
      {confirmDialog}
    </div>
  );
}
