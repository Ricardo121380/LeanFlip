import { useConfirm } from "@/components/ConfirmDialog";
import { QueryState } from "@/components/QueryState";
import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { useIsMutating } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { AlertTriangle, ArrowRightLeft, Check, ChevronRight, Copy, Download, FileCode2, Loader2, Pencil, Plus, Search, Star, Trash2, UserRound, X } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
// 编辑器（CodeMirror）按需加载。
const ProviderEditor = lazy(() => import("@/components/ProviderEditor").then((m) => ({ default: m.ProviderEditor })));
import { useAccounts, useAction, useLogins, useProviders } from "@/hooks/queries";
import { api, errorMessage, type AccountInfo, type Provider, type ToolId } from "@/lib/api";
import { accountLabel, endpointOf, isOfficial } from "@/lib/cards";
import { toolMeta } from "@/lib/tools";
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

export function ToolPage({ tool }: { tool: ToolId }) {
  const { t } = useTranslation();
  const meta = toolMeta(tool);
  const { data, isLoading, error, refetch } = useProviders(tool);
  const accounts = useAccounts(tool === "codex").data ?? [];
  const logins = useLogins(tool === "codex").data ?? [];
  const [editing, setEditing] = useState<Provider | null>(null);
  const [adding, setAdding] = useState(false);
  const [search, setSearch] = useState("");
  const searchInput = useRef<HTMLInputElement>(null);
  const [confirm, confirmDialog] = useConfirm();
  const busy = useIsMutating() > 0;

  useEffect(() => {
    setSearch("");
    setEditing(null);
    setAdding(false);
  }, [tool]);

  const switchAction = useAction(
    ({ id }: { id: string; name: string }) => api.switchProvider(tool, id),
    (_r, { name }) => toast.success(t("card.switched", { name })),
  );
  const toggleAction = useAction(({ id, enabled }: { id: string; enabled: boolean }) => api.setInLive(tool, id, enabled));
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
  const query = search.trim().toLocaleLowerCase();
  const providers = data.providers.filter((p) => [p.name, p.id, endpointOf(tool, p)].some((value) => value?.toLocaleLowerCase().includes(query)));
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
  const copyPath = async (path: string) => {
    try {
      await navigator.clipboard.writeText(path);
      toast.success(t("tool.pathCopied"));
    } catch {
      toast.error(t("tool.pathCopyFailed"));
    }
  };
  const importButton = (
    <Button variant="outline" size="sm" onClick={() => importAction.mutate(undefined)} disabled={busy}>
      {importAction.isPending ? <Loader2 className="animate-spin" /> : <Download />} {t(importAction.isPending ? "tool.importing" : "tool.importLive")}
    </Button>
  );
  const addButton = (
    <Button size="sm" onClick={() => setAdding(true)} disabled={busy}>
      <Plus /> {t("tool.add")}
    </Button>
  );

  return (
    <div className="flex min-w-0 flex-col gap-5 p-5 lg:p-6">
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0 flex-1 basis-full lg:basis-72">
          <div className="flex flex-wrap items-center gap-2">
            <span className="h-3 w-3 shrink-0 rounded-full" style={{ background: meta.color }} aria-hidden="true" />
            <h1 className="text-xl font-semibold">{meta.label}</h1>
            <Badge variant="outline">{t(`kind.${data.kind}`)}</Badge>
          </div>
          <p className="mt-1.5 max-w-prose text-sm leading-relaxed text-muted-foreground">{coexist ? t("tool.coexistHint") : t("tool.switchHint")}</p>
        </div>
        {data.providers.length > 0 && <div className="flex flex-wrap gap-2">{importButton}{addButton}</div>}
      </header>

      {error && (
        <div role="alert" className="flex min-w-0 items-start gap-2 rounded-md border border-amber-300 bg-amber-50 p-3 text-sm text-amber-900">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
          <div className="min-w-0 flex-1">
            <p className="font-medium">{t("tool.refreshError")}</p>
            <p className="mt-1 text-xs leading-relaxed">{t("tool.refreshErrorHint")}</p>
            <p className="mt-1 break-words text-xs leading-relaxed [overflow-wrap:anywhere]">{errorMessage(error)}</p>
            <Button variant="outline" size="sm" className="mt-2 border-amber-300" onClick={() => void refetch()}>{t("common.retry")}</Button>
          </div>
        </div>
      )}

      <div className="flex min-w-0 flex-col gap-3 rounded-lg bg-muted/60 px-4 py-3">
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-sm" aria-live="polite">
          {data.liveError ? <span className="text-amber-800">{t("tool.stateUnknown")}</span> : coexist ? (
            <><Check className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" /><span>{t("tool.enabledSummary", { count: data.providers.filter((p) => data.inLive.includes(p.id)).length, total: data.providers.length })}</span></>
          ) : (
            <><span className="shrink-0 text-muted-foreground">{t("tool.currentConfig")}</span><span className={cn("min-w-0 break-words font-medium [overflow-wrap:anywhere]", currentProvider && "text-primary")}>
              {currentProvider?.name ?? t("tool.noCurrent")}
            </span></>
          )}
        </div>
        <details className="group min-w-0 border-t pt-2">
          <summary className="flex w-fit cursor-pointer list-none items-center gap-1.5 rounded-sm text-xs text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
            <ChevronRight className="h-3.5 w-3.5 transition-transform group-open:rotate-90" aria-hidden="true" />
            <FileCode2 className="h-3.5 w-3.5" aria-hidden="true" />
            {t("tool.paths")} <span className="tabular-nums">({data.paths.length})</span>
          </summary>
          <div className="mt-2 space-y-1.5">
            {data.paths.length === 0 ? <p className="text-xs text-muted-foreground">{t("tool.noPaths")}</p> : data.paths.map((path) => (
              <div key={path} className="flex items-start gap-2">
                <code className="min-w-0 flex-1 break-all py-1.5 text-xs leading-relaxed text-muted-foreground">{path}</code>
                <Button variant="ghost" size="icon" className="shrink-0" onClick={() => void copyPath(path)} aria-label={t("tool.copyPath", { path })} title={t("tool.copyPath", { path })}><Copy /></Button>
              </div>
            ))}
          </div>
        </details>
      </div>

      {data.liveError && (
        <div role="alert" className="flex min-w-0 items-start gap-2 rounded-md border border-amber-300 bg-amber-50 p-3 text-sm text-amber-900">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
          <div className="min-w-0 flex-1">
            <p className="font-medium">{t("tool.liveErrorTitle")}</p>
            <p className="mt-1 break-words text-xs leading-relaxed [overflow-wrap:anywhere]">{t("tool.liveError")} {data.liveError}</p>
            <Button variant="outline" size="sm" className="mt-2 border-amber-300" onClick={() => void refetch()}>{t("common.retry")}</Button>
          </div>
        </div>
      )}

      {data.providers.length === 0 ? (
        <section className="flex flex-col items-start gap-3 rounded-lg border border-dashed px-6 py-8">
          <h2 className="text-base font-semibold">{t("tool.emptyTitle", { tool: meta.label })}</h2>
          <p className="max-w-prose text-sm leading-relaxed text-muted-foreground">{t("tool.empty")}</p>
          <div className="mt-1 flex flex-wrap gap-2">{addButton}{importButton}</div>
        </section>
      ) : (
        <>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <p className="text-sm text-muted-foreground" role="status">{query ? t("tool.searchCount", { count: providers.length, total: data.providers.length }) : t("tool.providerCount", { count: data.providers.length })}</p>
            <div className="relative w-full max-w-sm">
              <Search className="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" aria-hidden="true" />
              <Input ref={searchInput} type="search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder={t("tool.search")} aria-label={t("tool.search")} className="pl-9 pr-9 [&::-webkit-search-cancel-button]:appearance-none" />
              {search && <Button size="icon" variant="ghost" className="absolute right-0.5 top-0.5 h-8 w-8" aria-label={t("tool.clearSearch")} onClick={clearSearch}><X /></Button>}
            </div>
          </div>
          {providers.length === 0 ? (
            <div className="flex flex-col items-center gap-2 rounded-lg border border-dashed p-8 text-center">
              <h2 className="font-medium">{t("tool.noResults")}</h2>
              <p className="max-w-prose break-words text-sm text-muted-foreground [overflow-wrap:anywhere]">{t("tool.noResultsHint", { query: search.trim() })}</p>
              <Button variant="outline" size="sm" className="mt-2" onClick={clearSearch}>{t("tool.clearSearch")}</Button>
            </div>
          ) : !coexist ? (
            <div className="grid items-start gap-3 lg:grid-cols-2">
              {providers.map((p) => {
                const current = data.current === p.id;
                const endpoint = endpointOf(tool, p);
                const switching = switchAction.isPending && switchAction.variables?.id === p.id;
                const deleting = deleteAction.isPending && deleteAction.variables === p.id;
                return (
                  <Card key={p.id} data-testid={`card-${p.id}`} className={cn("flex min-w-0 flex-col gap-3 p-4 shadow-none", current && "border-primary bg-primary/[0.035]")}>
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-1.5">
                        <span className="min-w-0 break-words font-medium [overflow-wrap:anywhere]">{p.name}</span>
                        {current && <Badge>{t("card.current")}</Badge>}
                        {isOfficial(p) && <Badge variant="success" className="gap-0.5"><Star className="h-3 w-3" aria-hidden="true" /> {t("card.official")}</Badge>}
                        {tool === "codex" && <LoginBadge provider={p} accounts={accounts} />}
                      </div>
                      <p className="mt-1.5 truncate font-mono text-xs text-muted-foreground" title={endpoint ?? p.id}>{endpoint ?? p.id}</p>
                    </div>
                    <div className="flex flex-wrap items-center gap-1.5">
                      <Button size="sm" variant={current ? "secondary" : "default"} disabled={current || busy} onClick={() => switchAction.mutate({ id: p.id, name: p.name })} aria-label={current ? t("card.currentSwitch") : undefined}>
                        {switching ? <Loader2 className="animate-spin" /> : current ? <Check /> : <ArrowRightLeft />} {t(switching ? "card.switching" : current ? "card.active" : "card.switch")}
                      </Button>
                      <Button size="sm" variant="ghost" disabled={busy} onClick={() => setEditing(p)} aria-label={`${t("card.edit")} ${p.name}`}><Pencil /> {t("card.edit")}</Button>
                      <Button size="icon" variant="ghost" className="ml-auto text-muted-foreground hover:text-destructive" disabled={current || busy} onClick={() => void confirmDelete(p)} aria-label={deleting ? t("card.deleting", { name: p.name }) : `${t("card.delete")} ${p.name}`} title={current ? t("card.currentDeleteHint") : t("card.delete")}>
                        {deleting ? <Loader2 className="animate-spin" /> : <Trash2 />}
                      </Button>
                    </div>
                    {current && <p className="text-xs leading-relaxed text-muted-foreground">{t("card.currentDeleteHint")}</p>}
                  </Card>
                );
              })}
            </div>
          ) : (
            <Card className="min-w-0 divide-y shadow-none">
              {providers.map((p) => {
                const inLive = data.inLive.includes(p.id);
                const toggling = toggleAction.isPending && toggleAction.variables?.id === p.id;
                const switching = switchAction.isPending && switchAction.variables?.id === p.id;
                const deleting = deleteAction.isPending && deleteAction.variables === p.id;
                const endpoint = endpointOf(tool, p);
                return (
                  <div key={p.id} className="flex flex-wrap items-center gap-x-3 gap-y-2 px-4 py-3" data-testid={`row-${p.id}`}>
                    <Switch checked={inLive} disabled={Boolean(data.liveError) || busy} onCheckedChange={(enabled) => toggleAction.mutate({ id: p.id, enabled })} aria-label={t("card.toggleInLive", { name: p.name })} />
                    <div className="min-w-0 flex-1 basis-44">
                      <div className="flex flex-wrap items-center gap-1.5">
                        <span className="min-w-0 break-words font-medium [overflow-wrap:anywhere]">{p.name}</span>
                        {toggling ? <Badge variant="secondary" className="gap-1"><Loader2 className="h-3 w-3 animate-spin" />{t(toggleAction.variables?.enabled ? "card.enabling" : "card.disabling")}</Badge> : inLive && <Badge variant="success">{t("card.inLive")}</Badge>}
                      </div>
                      <p className="mt-1 truncate font-mono text-xs text-muted-foreground" title={endpoint ? `${p.id} · ${endpoint}` : p.id}>{p.id}{endpoint && ` · ${endpoint}`}</p>
                    </div>
                    <div className="ml-auto flex shrink-0 items-center gap-1">
                      {tool === "hermes" && <Button size="sm" variant="outline" disabled={Boolean(data.liveError) || busy} onClick={() => switchAction.mutate({ id: p.id, name: p.name })}>{switching && <Loader2 className="animate-spin" />}{t(switching ? "card.switching" : "card.setDefaultModel")}</Button>}
                      <Button size="icon" variant="ghost" disabled={busy} onClick={() => setEditing(p)} aria-label={`${t("card.edit")} ${p.name}`} title={t("card.edit")}><Pencil /></Button>
                      <Button size="icon" variant="ghost" className="text-muted-foreground hover:text-destructive" disabled={busy} onClick={() => void confirmDelete(p)} aria-label={deleting ? t("card.deleting", { name: p.name }) : `${t("card.delete")} ${p.name}`} title={t("card.delete")}>{deleting ? <Loader2 className="animate-spin" /> : <Trash2 />}</Button>
                    </div>
                  </div>
                );
              })}
            </Card>
          )}
        </>
      )}

      <Suspense fallback={<p role="status" className="text-sm text-muted-foreground">{t("tool.loadingEditor")}</p>}>
        {adding && <ProviderEditor tool={tool} open onOpenChange={setAdding} existingIds={data.providers.map((p) => p.id)} accounts={accounts} logins={logins} />}
        {editing && <ProviderEditor key={editing.id} tool={tool} open onOpenChange={(open) => !open && setEditing(null)} provider={editing} existingIds={data.providers.map((p) => p.id)} accounts={accounts} logins={logins} />}
      </Suspense>
      {confirmDialog}
    </div>
  );
}
