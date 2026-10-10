import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, Copy, ExternalLink, Loader2, LogIn, RefreshCw, Star, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useAccounts, useAction, useLogins } from "@/hooks/queries";
import { api, errorMessage, type DeviceFlow } from "@/lib/api";
import { accountLabel, planBadge } from "@/lib/cards";

export function PlanBadge({ plan }: { plan: string | null }) {
  const badge = planBadge(plan);
  return badge ? <Badge variant={badge.variant}>{badge.label}</Badge> : null;
}

function DeviceLoginDialog({ target, onClose }: { target?: string; onClose: () => void }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const [flow, setFlow] = useState<DeviceFlow | null>(null);
  const [error, setError] = useState<string | null>(null);
  const done = useRef(false);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let active: DeviceFlow | null = null;
    let cancelled = false;
    api
      .codexStartLogin(target)
      .then((started) => {
        if (cancelled) {
          void api.codexCancelLogin(started.deviceCode);
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
                void client.invalidateQueries();
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
      if (active && !done.current) void api.codexCancelLogin(active.deviceCode);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target]);

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("codex.deviceTitle")}</DialogTitle>
          <DialogDescription>{t("codex.hint")}</DialogDescription>
        </DialogHeader>
        {error ? (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        ) : !flow ? (
          <Loader2 className="mx-auto h-6 w-6 animate-spin text-muted-foreground" />
        ) : (
          <div className="flex flex-col gap-3 text-sm">
            <div>
              <p className="text-muted-foreground">{t("codex.deviceStep1")}</p>
              <Button variant="outline" size="sm" className="mt-1" onClick={() => void api.openUrl(flow.verificationUri)}>
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
                  onClick={() => void navigator.clipboard?.writeText(flow.userCode).then(() => toast.success(t("codex.copied")))}
                >
                  <Copy /> {t("codex.copy")}
                </Button>
              </div>
            </div>
            <p className="flex items-center gap-2 text-muted-foreground">
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
  const { data: accounts = [], isLoading } = useAccounts();
  const { data: logins = [] } = useLogins();
  const [login, setLogin] = useState<{ target?: string } | null>(null);
  const setDefault = useAction((id: string) => api.codexSetDefault(id));
  const remove = useAction((id: string) => api.codexRemoveAccount(id));

  return (
    <div className="flex flex-col gap-5 p-6">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div className="max-w-2xl">
          <h1 className="text-xl font-semibold">{t("codex.title")}</h1>
          <p className="mt-1 text-sm text-muted-foreground">{t("codex.hint")}</p>
        </div>
        <Button size="sm" onClick={() => setLogin({})}>
          <LogIn /> {t("codex.add")}
        </Button>
      </header>

      {isLoading ? (
        <p className="text-sm text-muted-foreground">{t("common.loading")}</p>
      ) : accounts.length === 0 ? (
        <p className="rounded-md border border-dashed p-8 text-center text-sm text-muted-foreground">{t("codex.empty")}</p>
      ) : (
        <Card className="divide-y">
          {accounts.map((a) => (
            <div key={a.id} className="flex items-center gap-3 px-4 py-3" data-testid={`account-${a.id}`}>
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="truncate font-medium">{accountLabel(a)}</span>
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
                <p className="text-xs text-muted-foreground">
                  {a.workspace && a.email ? `${a.workspace} · ` : ""}
                  {t("codex.authenticatedAt", { time: new Date(a.authenticatedAt).toLocaleString(i18n.language) })}
                </p>
              </div>
              {!a.isDefault && (
                <Button size="sm" variant="ghost" onClick={() => setDefault.mutate(a.id)}>
                  {t("codex.setDefault")}
                </Button>
              )}
              <Button size="sm" variant={a.reauthRequired ? "default" : "ghost"} onClick={() => setLogin({ target: a.id })}>
                <RefreshCw /> {t("codex.relogin")}
              </Button>
              <Button
                size="icon"
                variant="ghost"
                className="text-muted-foreground hover:text-destructive"
                aria-label={`${t("codex.remove")} ${accountLabel(a)}`}
                onClick={() => window.confirm(t("codex.confirmRemove", { label: accountLabel(a) })) && remove.mutate(a.id)}
              >
                <Trash2 />
              </Button>
            </div>
          ))}
        </Card>
      )}

      <section className="flex flex-col gap-2">
        <h2 className="font-medium">{t("codex.localTitle")}</h2>
        <p className="text-xs text-muted-foreground">{t("codex.localHint")}</p>
        {logins.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("codex.localEmpty")}</p>
        ) : (
          <Card className="divide-y">
            {logins.map((l) => (
              <div key={l.identity} className="flex items-center gap-2 px-4 py-2.5 text-sm">
                <span className="font-medium">{l.email ?? l.identity}</span>
                <PlanBadge plan={l.plan} />
                {l.workspace && <span className="text-xs text-muted-foreground">{l.workspace}</span>}
                {l.live && <Badge variant="success">{t("codex.live")}</Badge>}
              </div>
            ))}
          </Card>
        )}
      </section>

      {login && <DeviceLoginDialog target={login.target} onClose={() => setLogin(null)} />}
    </div>
  );
}
