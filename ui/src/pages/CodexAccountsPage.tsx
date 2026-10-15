import { useConfirm } from "@/components/ConfirmDialog";
import { QueryState } from "@/components/QueryState";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, Copy, ExternalLink, Loader2, LogIn, RefreshCw, Star, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useAccounts, useAction, useLogins, useQuota, useQuotaRefresh, refreshAll } from "@/hooks/queries";
import { api, errorMessage, type DeviceFlow } from "@/lib/api";
import { accountLabel, planBadge } from "@/lib/cards";
import { Monogram } from "@/components/Glyph";
import { QuotaDetails } from "@/components/QuotaDetails";
import { agoText, sourceFor } from "@/lib/quota";

export function PlanBadge({ plan }: { plan: string | null }) {
  const badge = planBadge(plan);
  return badge ? <Badge variant={badge.variant}>{badge.label}</Badge> : null;
}

function DeviceLoginDialog({ target, onClose }: { target?: string; onClose: () => void }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const [flow, setFlow] = useState<DeviceFlow | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const done = useRef(false);

  useEffect(() => {
    setError(null);
    setFlow(null);
    done.current = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let active: DeviceFlow | null = null;
    let cancelled = false;
    api
      .codexStartLogin(target)
      .then((started) => {
        if (cancelled) {
          void api.codexCancelLogin(started.deviceCode).catch(() => undefined);
          return;
        }
        active = started;
        setFlow(started);
        const poll = () => {
          api
            .codexPollLogin(started.deviceCode)
            .then((result) => {
              if (cancelled) return;
              if (result.status === "added") {
                done.current = true;
                toast.success(t("codex.added", { label: accountLabel(result.account) }));
                void refreshAll(client);
                onClose();
              } else {
                timer = setTimeout(poll, Math.max(1, started.interval) * 1000);
              }
            })
            .catch((e) => !cancelled && setError(errorMessage(e)));
        };
        timer = setTimeout(poll, Math.max(1, started.interval) * 1000);
      })
      .catch((e) => !cancelled && setError(errorMessage(e)));
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
      if (active && !done.current) void api.codexCancelLogin(active.deviceCode).catch(() => undefined);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target, attempt]);

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("codex.deviceTitle")}</DialogTitle>
          <DialogDescription>{t("codex.hint")}</DialogDescription>
        </DialogHeader>
        {error ? (
          <QueryState state="error" title={t("codex.loginError")} description={error} onRetry={() => setAttempt((value) => value + 1)} />
        ) : !flow ? (
          <p role="status" className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" /> {t("codex.loginStarting")}
          </p>
        ) : (
          <div className="flex flex-col gap-3 text-sm">
            <div>
              <p className="text-muted-foreground">{t("codex.deviceStep1")}</p>
              <Button variant="outline" size="sm" className="mt-2" onClick={async () => {
                try { await api.openUrl(flow.verificationUri); }
                catch (error) { toast.error(t("codex.openError", { error: errorMessage(error) })); }
              }}>
                <ExternalLink /> {t("codex.openPage")}
              </Button>
              <p className="mt-1 break-all font-mono text-xs text-muted-foreground">{flow.verificationUri}</p>
            </div>
            <div>
              <p className="text-muted-foreground">{t("codex.deviceStep2")}</p>
              <div className="mt-1 flex items-center gap-2">
                <code className="rounded-md bg-muted px-3 py-2 text-2xl font-semibold tracking-widest" data-testid="user-code">
                  {flow.userCode}
                </code>
                <Button
                  variant="ghost"
                  size="sm"
                  aria-label={t("codex.copyCode")}
                  onClick={async () => {
                    try {
                      if (!navigator.clipboard?.writeText) throw new Error(t("codex.clipboardUnavailable"));
                      await navigator.clipboard.writeText(flow.userCode);
                      toast.success(t("codex.copied"));
                    } catch {
                      toast.error(t("codex.copyError"));
                    }
                  }}
                >
                  <Copy /> {t("codex.copy")}
                </Button>
              </div>
            </div>
            <p role="status" className="flex items-center gap-2 text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" /> {t("codex.waiting")}
            </p>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

export function CodexAccountsPage() {
  const { t, i18n } = useTranslation();
  const { data: accounts = [], isLoading, isError, error, refetch } = useAccounts();
  const localLogins = useLogins();
  const logins = localLogins.data ?? [];
  const [login, setLogin] = useState<{ target?: string } | null>(null);
  const setDefault = useAction((id: string) => api.codexSetDefault(id), () => toast.success(t("codex.defaultUpdated")));
  const remove = useAction((id: string) => api.codexRemoveAccount(id), () => toast.success(t("codex.removed")));
  const [confirm, confirmDialog] = useConfirm();
  const busy = setDefault.isPending || remove.isPending || login !== null;
  const quota = useQuota();
  const refreshQuota = useQuotaRefresh();
  const now = Date.now();
  const refreshingAccount = (id: string) => refreshQuota.isPending && (!refreshQuota.variables || refreshQuota.variables.includes(id));
  const lastOnline = Math.max(0, ...(quota.data?.sources ?? []).filter((s) => s.accountId && s.snapshot?.origin === "network").map((s) => s.snapshot!.fetchedAt));

  return (
    <div className="mx-auto flex w-full min-w-0 max-w-5xl flex-col gap-5 px-6 py-6 lg:px-8">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div className="max-w-2xl">
          <h1 className="text-[22px] font-semibold tracking-tight">{t("codex.title")}</h1>
          <p className="mt-1 text-sm text-muted-foreground">{t("codex.hint")}</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {accounts.length > 0 && (
            <Button size="sm" variant="outline" disabled={refreshQuota.isPending} title={t("quota.refreshHint")} onClick={() => refreshQuota.mutate(undefined)}>
              {refreshQuota.isPending && !refreshQuota.variables ? <Loader2 className="animate-spin" /> : <RefreshCw />}
              {refreshQuota.isPending && !refreshQuota.variables ? t("quota.refreshing") : lastOnline ? t("quota.checked", { ago: agoText(lastOnline, now, t) }) : t("quota.refresh")}
            </Button>
          )}
          {(isError || accounts.length > 0) && <Button size="sm" disabled={busy || isError} onClick={() => setLogin({})}>
            <LogIn /> {t("codex.add")}
          </Button>}
        </div>
      </header>

      {isLoading ? (
        <QueryState state="loading" title={t("codex.loading")} />
      ) : isError ? (
        <QueryState state="error" title={t("codex.loadError")} description={errorMessage(error)} onRetry={() => void refetch()} />
      ) : accounts.length === 0 ? (
        <div className="flex flex-col items-center gap-3 rounded-lg border border-dashed p-8 text-center">
          <p className="font-medium">{t("codex.empty")}</p>
          <p className="max-w-md text-sm leading-relaxed text-muted-foreground">{t("codex.emptyHint")}</p>
          <Button size="sm" disabled={busy} onClick={() => setLogin({})}><LogIn /> {t("codex.add")}</Button>
        </div>
      ) : (
        <Card className="divide-y" aria-busy={setDefault.isPending || remove.isPending}>
          {accounts.map((a) => (
            <div key={a.id} className="flex flex-wrap items-center gap-3 px-4 py-4" data-testid={`account-${a.id}`}>
              <Monogram name={accountLabel(a)} size={32} />
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="break-all font-medium">{accountLabel(a)}</span>
                  <PlanBadge plan={a.plan} />
                  {a.isDefault && (
                    <Badge variant="outline" className="gap-0.5">
                      <Star className="h-3 w-3" /> {t("codex.default")}
                    </Badge>
                  )}
                  {a.reauthRequired && (
                    <Badge variant="warning" className="gap-0.5">
                      <AlertTriangle className="h-3 w-3" /> {t("codex.reauth")}
                    </Badge>
                  )}
                </div>
                <p className="mt-1 break-words text-xs leading-relaxed text-muted-foreground">
                  {a.workspace && a.email ? `${a.workspace} · ` : ""}
                  {t("codex.authenticatedAt", { time: new Date(a.authenticatedAt * 1000).toLocaleString(i18n.language) })}
                </p>
                {quota.data && (
                  <QuotaDetails
                    source={sourceFor(quota.data, { accountId: a.id })}
                    label={accountLabel(a)}
                    now={now}
                    canRefresh={!a.reauthRequired}
                    refreshing={refreshingAccount(a.id)}
                    onRefresh={() => refreshQuota.mutate([a.id])}
                  />
                )}
              </div>
              <div className="flex flex-wrap items-center gap-1">
                {!a.isDefault && (
                  <Button size="sm" variant="ghost" disabled={busy} aria-label={t("codex.setDefaultFor", { label: accountLabel(a) })} onClick={() => setDefault.mutate(a.id)}>
                    {setDefault.isPending && setDefault.variables === a.id && <Loader2 className="animate-spin" />}
                    {t(setDefault.isPending && setDefault.variables === a.id ? "codex.settingDefault" : "codex.setDefault")}
                  </Button>
                )}
                <Button size="sm" variant={a.reauthRequired ? "default" : "ghost"} disabled={busy} aria-label={t("codex.reloginFor", { label: accountLabel(a) })} onClick={() => setLogin({ target: a.id })}>
                  <RefreshCw /> {t("codex.relogin")}
                </Button>
                <Button
                  size="icon"
                  variant="ghost"
                  className="text-muted-foreground hover:text-destructive"
                  disabled={busy}
                  aria-label={`${t("codex.remove")} ${accountLabel(a)}`}
                  onClick={async () => {
                    const ok = await confirm({
                      title: t("codex.confirmRemove", { label: accountLabel(a) }),
                      description: t("codex.confirmRemoveHint"),
                      confirmLabel: t("codex.confirmRemoveAction"),
                      destructive: true,
                    });
                    if (ok) remove.mutate(a.id);
                  }}
                >
                  {remove.isPending && remove.variables === a.id ? <Loader2 className="animate-spin" /> : <Trash2 />}
                </Button>
              </div>
            </div>
          ))}
        </Card>
      )}
      {(setDefault.isPending || remove.isPending) && <p role="status" className="text-sm text-muted-foreground">{t(remove.isPending ? "codex.removing" : "codex.settingDefault")}</p>}

      <section aria-labelledby="local-logins-title" className="flex flex-col gap-2">
        <h2 id="local-logins-title" className="font-medium">{t("codex.localTitle")}</h2>
        <p className="text-xs text-muted-foreground">{t("codex.localHint")}</p>
        {localLogins.isLoading ? (
          <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" /> {t("codex.localLoading")}
          </p>
        ) : localLogins.isError ? (
          <QueryState state="error" title={t("codex.localError")} description={errorMessage(localLogins.error)} onRetry={() => void localLogins.refetch()} />
        ) : logins.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("codex.localEmpty")}</p>
        ) : (
          <Card className="divide-y">
            {logins.map((l) => (
              <div key={l.identity} className="flex flex-wrap items-center gap-2 px-4 py-3 text-sm">
                <span className="break-all font-medium">{l.email ?? l.identity}</span>
                <PlanBadge plan={l.plan} />
                {l.workspace && <span className="text-xs text-muted-foreground">{l.workspace}</span>}
                {l.live && <Badge variant="success">{t("codex.live")}</Badge>}
                {sourceFor(quota.data, { login: l.identity })?.snapshot && (
                  <div className="basis-full">
                    <QuotaDetails source={sourceFor(quota.data, { login: l.identity })} label={l.email ?? l.identity} now={now} canRefresh={false} showError={false} />
                  </div>
                )}
              </div>
            ))}
          </Card>
        )}
      </section>

      {login && <DeviceLoginDialog target={login.target} onClose={() => setLogin(null)} />}
      {confirmDialog}
    </div>
  );
}
