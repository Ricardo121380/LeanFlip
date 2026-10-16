// 统计页的“额度”标签：所有额度来源集中在一处，需要注意的放在最上面。
// 订阅窗口（Codex 官方登录）和中转余额分两组；每个来源写明用它的卡、数据来源和时间。
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Loader2, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { QueryState } from "@/components/QueryState";
import { QuotaDetails } from "@/components/QuotaDetails";
import { ToolGlyph } from "@/components/Glyph";
import { useQuota, useQuotaRefresh } from "@/hooks/queries";
import { errorMessage, type QuotaSource } from "@/lib/api";
import { adapterName, agoText, quotaBrief } from "@/lib/quota";
import { toolMeta } from "@/lib/tools";
import { cn } from "@/lib/utils";

/** 来源的名字：中转站用卡名，Codex 用账号邮箱。 */
function sourceName(source: QuotaSource): string {
  if (source.kind === "relay") return source.users[0]?.providerName ?? source.label ?? source.key;
  return source.label ?? source.users[0]?.providerName ?? "Codex";
}

function SourceRow({ source, now, refreshing, onRefresh }: { source: QuotaSource; now: number; refreshing: boolean; onRefresh: () => void }) {
  const { t } = useTranslation();
  const name = sourceName(source);
  const kind = source.kind === "relay"
    ? [t(source.adapter === "krill" ? "stats.quotaKrillKind" : "stats.quotaRelayKind", { name: source.adapter ? adapterName(source.adapter) : "" }), source.label].filter(Boolean).join(" · ")
    : [t("stats.quotaCodexKind"), source.snapshot?.plan && source.snapshot.plan.charAt(0).toUpperCase() + source.snapshot.plan.slice(1)].filter(Boolean).join(" · ");
  const online = source.kind === "relay" || source.accountId !== undefined;
  return (
    <li className="grid gap-x-6 gap-y-1 px-5 py-4 sm:grid-cols-[minmax(0,15rem)_minmax(0,1fr)]" data-testid={`quota-${source.key}`}>
      <div className="flex min-w-0 flex-col gap-1">
        <span className="break-all text-sm font-semibold">{name}</span>
        <span className="text-xs text-muted-foreground">{kind}</span>
        <span className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          {source.users.length === 0 && t("stats.quotaUnused")}
          {source.users.map((u) => (
            <span key={`${u.tool}:${u.providerId}`} className={cn("inline-flex items-center gap-1", u.active && "font-medium text-foreground")} title={toolMeta(u.tool).label}>
              <ToolGlyph tool={toolMeta(u.tool)} size={16} />
              {source.kind === "relay" ? toolMeta(u.tool).label : u.providerName}
            </span>
          ))}
          {source.kind === "relay" && source.users.length > 1 && <span>· {t("stats.quotaShared")}</span>}
        </span>
        {source.refreshMinutes && <span className="text-xs text-muted-foreground">{t("stats.quotaAuto", { count: source.refreshMinutes })}</span>}
      </div>
      <QuotaDetails
        source={source}
        label={name}
        now={now}
        canRefresh={online}
        refreshing={refreshing}
        onRefresh={onRefresh}
        emptyText={source.kind === "relay" ? t("stats.quotaRelayEmpty") : undefined}
      />
    </li>
  );
}

export function QuotaPanel({ tabs }: { tabs: ReactNode }) {
  const { t, i18n } = useTranslation();
  const quota = useQuota();
  const refresh = useQuotaRefresh();
  const now = Date.now();
  const sources = quota.data?.sources ?? [];
  const attention = sources.filter((s) => s.snapshot && s.snapshot.level !== "ok" && s.users.length > 0);
  const groups = [
    { title: t("stats.quotaCodex"), items: sources.filter((s) => s.kind === "codex") },
    { title: t("stats.quotaRelay"), items: sources.filter((s) => s.kind === "relay") },
  ].filter((g) => g.items.length > 0);
  const lastOnline = Math.max(0, ...sources.flatMap((s) => (s.snapshot?.origin === "network" ? [s.snapshot.fetchedAt] : [])));
  const refreshing = (key: string) => refresh.isPending && (!refresh.variables || refresh.variables.includes(key));

  return (
    <div className="mx-auto flex w-full min-w-0 max-w-6xl flex-col gap-5 px-6 py-7 lg:px-8">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0 max-w-3xl">
          <h1 className="text-[22px] font-semibold leading-tight tracking-tight">{t("stats.title")}</h1>
          <p className="mt-0.5 text-xs leading-5 text-muted-foreground">{t("stats.quotaSubtitle")}</p>
        </div>
        {sources.length > 0 && (
          <Button size="sm" variant="outline" className="h-8" disabled={refresh.isPending} title={t("quota.refreshHint")} onClick={() => refresh.mutate(undefined)}>
            {refresh.isPending && !refresh.variables ? <Loader2 className="animate-spin" /> : <RefreshCw />}
            {refresh.isPending && !refresh.variables ? t("quota.refreshing") : lastOnline ? t("quota.checked", { ago: agoText(lastOnline, now, t) }) : t("quota.refresh")}
          </Button>
        )}
      </header>
      {tabs}

      {quota.isLoading ? (
        <QueryState state="loading" title={t("common.loading")} />
      ) : quota.isError ? (
        <QueryState state="error" title={t("stats.quotaEmpty")} description={errorMessage(quota.error)} onRetry={() => void quota.refetch()} />
      ) : sources.length === 0 ? (
        <section className="flex flex-col items-start gap-2 rounded-2xl border border-dashed px-6 py-7">
          <h2 className="text-base font-semibold">{t("stats.quotaEmpty")}</h2>
          <p className="max-w-prose text-sm leading-relaxed text-muted-foreground">{t("stats.quotaEmptyHint")}</p>
        </section>
      ) : (
        <>
          <p className="text-sm text-muted-foreground">
            {t("stats.quotaSummary", { count: sources.length })}
            {attention.length > 0 && <> · <span className="font-medium text-foreground">{t("stats.quotaAttention", { count: attention.length })}</span></>}
          </p>
          {attention.length > 0 && (
            <ul className="flex flex-col gap-1.5" aria-label={t("stats.quotaAttention", { count: attention.length })}>
              {attention.map((s) => {
                const brief = quotaBrief(s.snapshot!, now, t, i18n.language);
                const out = s.snapshot!.level === "out";
                return (
                  <li key={s.key} className={cn("flex flex-wrap items-center gap-x-2.5 gap-y-0.5 rounded-lg border px-3 py-2 text-sm", out ? "border-destructive/30 bg-destructive/5" : "border-warning/30 bg-warning/5")}>
                    <AlertTriangle className={cn("h-4 w-4 shrink-0", out ? "text-destructive" : "text-warning")} />
                    <span className="font-medium">{sourceName(s)}</span>
                    {brief && <span className={cn("font-mono text-xs font-semibold", out ? "text-destructive" : "text-warning")}>{brief.text}</span>}
                    {brief?.note && <span className="text-xs text-muted-foreground">{brief.note}</span>}
                  </li>
                );
              })}
            </ul>
          )}
          {groups.map((group) => (
            <section key={group.title} className="flex flex-col gap-2" aria-label={group.title}>
              <h2 className="text-xs font-semibold tracking-wide text-muted-foreground">{group.title}</h2>
              <ul className="stats-panel divide-y">
                {group.items.map((source) => (
                  <SourceRow key={source.key} source={source} now={now} refreshing={refreshing(source.key)} onRefresh={() => refresh.mutate([source.key])} />
                ))}
              </ul>
            </section>
          ))}
        </>
      )}
    </div>
  );
}
