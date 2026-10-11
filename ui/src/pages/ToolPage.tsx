import { lazy, Suspense, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, ArrowRightLeft, Download, Pencil, Plus, Star, Trash2, UserRound } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
// 编辑器（CodeMirror）按需加载。
const ProviderEditor = lazy(() => import("@/components/ProviderEditor").then((m) => ({ default: m.ProviderEditor })));
import { useAccounts, useAction, useLogins, useProviders } from "@/hooks/queries";
import { api, type AccountInfo, type Provider, type ToolId } from "@/lib/api";
import { accountLabel, endpointOf, isOfficial } from "@/lib/cards";
import { toolMeta } from "@/lib/tools";
import { cn } from "@/lib/utils";

function LoginBadge({ provider, accounts }: { provider: Provider; accounts: AccountInfo[] }) {
  const { t } = useTranslation();
  const binding = provider.meta?.authBinding;
  if (binding?.authProvider === "codex_oauth") {
    const account = accounts.find((a) => a.id === binding.accountId);
    return (
      <Badge variant="secondary" className="gap-1">
        <UserRound className="h-3 w-3" />
        {account ? accountLabel(account) : t("card.managedDefault")}
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
  const accounts = useAccounts().data ?? [];
  const logins = useLogins(tool === "codex").data ?? [];
  const [editing, setEditing] = useState<Provider | null>(null);
  const [adding, setAdding] = useState(false);

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

  if (isLoading) return <p className="p-8 text-sm text-muted-foreground">{t("common.loading")}</p>;
  if (error || !data)
    return (
      <div className="p-8 text-sm">
        <p className="text-destructive">{String(error)}</p>
        <Button variant="outline" size="sm" className="mt-2" onClick={() => void refetch()}>
          {t("common.retry")}
        </Button>
      </div>
    );

  const coexist = data.kind === "coexist";
  const confirmDelete = (p: Provider) => {
    if (window.confirm(t("card.confirmDelete", { name: p.name }))) deleteAction.mutate(p.id);
  };

  return (
    <div className="flex flex-col gap-5 p-6">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2">
            <span className="h-3 w-3 rounded-full" style={{ background: meta.color }} />
            <h1 className="text-xl font-semibold">{meta.label}</h1>
            <Badge variant="outline">{t(`kind.${data.kind}`)}</Badge>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">{coexist ? t("tool.coexistHint") : t("tool.switchHint")}</p>
          <p className="mt-1 font-mono text-xs text-muted-foreground" title={t("tool.paths")}>
            {data.paths.join("  ·  ")}
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={() => importAction.mutate(undefined)} disabled={importAction.isPending}>
            <Download /> {t("tool.importLive")}
          </Button>
          <Button size="sm" onClick={() => setAdding(true)}>
            <Plus /> {t("tool.add")}
          </Button>
        </div>
      </header>

      {data.liveError && (
        <div className="flex items-start gap-2 rounded-md border border-amber-300 bg-amber-50 p-3 text-sm text-amber-900">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
          <span>
            {t("tool.liveError")} {data.liveError}
          </span>
        </div>
      )}

      {data.providers.length === 0 && <p className="rounded-md border border-dashed p-8 text-center text-sm text-muted-foreground">{t("tool.empty")}</p>}

      {!coexist ? (
        <div className="grid gap-3 lg:grid-cols-2">
          {data.providers.map((p) => {
            const current = data.current === p.id;
            const endpoint = endpointOf(tool, p);
            return (
              <Card
                key={p.id}
                data-testid={`card-${p.id}`}
                className={cn("flex flex-col gap-3 p-4 transition-shadow", current && "border-primary shadow-md ring-1 ring-primary/30")}
              >
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <span className="truncate font-medium">{p.name}</span>
                      {current && <Badge>{t("card.current")}</Badge>}
                      {isOfficial(p) && (
                        <Badge variant="success" className="gap-0.5">
                          <Star className="h-3 w-3" /> {t("card.official")}
                        </Badge>
                      )}
                      {tool === "codex" && <LoginBadge provider={p} accounts={accounts} />}
                    </div>
                    <p className="mt-1 truncate font-mono text-xs text-muted-foreground">{endpoint ?? p.id}</p>
                  </div>
                </div>
                <div className="flex items-center gap-1.5">
                  <Button
                    size="sm"
                    variant={current ? "secondary" : "default"}
                    disabled={current || switchAction.isPending}
                    onClick={() => switchAction.mutate({ id: p.id, name: p.name })}
                  >
                    <ArrowRightLeft /> {t("card.switch")}
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => setEditing(p)} aria-label={`${t("card.edit")} ${p.name}`}>
                    <Pencil /> {t("card.edit")}
                  </Button>
                  <Button
                    size="icon"
                    variant="ghost"
                    className="ml-auto text-muted-foreground hover:text-destructive"
                    disabled={current}
                    onClick={() => confirmDelete(p)}
                    aria-label={`${t("card.delete")} ${p.name}`}
                  >
                    <Trash2 />
                  </Button>
                </div>
              </Card>
            );
          })}
        </div>
      ) : (
        <Card className="divide-y">
          {data.providers.map((p) => {
            const inLive = data.inLive.includes(p.id);
            return (
              <div key={p.id} className="flex items-center gap-3 px-4 py-3" data-testid={`row-${p.id}`}>
                <Switch
                  checked={inLive}
                  disabled={Boolean(data.liveError) || toggleAction.isPending}
                  onCheckedChange={(enabled) => toggleAction.mutate({ id: p.id, enabled })}
                  aria-label={`toggle ${p.id}`}
                />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span className="truncate font-medium">{p.name}</span>
                    <span className="font-mono text-xs text-muted-foreground">{p.id}</span>
                    {inLive && <Badge variant="success">{t("card.inLive")}</Badge>}
                  </div>
                  <p className="truncate font-mono text-xs text-muted-foreground">{endpointOf(tool, p) ?? ""}</p>
                </div>
                {tool === "hermes" && (
                  <Button size="sm" variant="outline" onClick={() => switchAction.mutate({ id: p.id, name: p.name })}>
                    {t("card.setDefaultModel")}
                  </Button>
                )}
                <Button size="sm" variant="ghost" onClick={() => setEditing(p)} aria-label={`${t("card.edit")} ${p.name}`}>
                  <Pencil />
                </Button>
                <Button
                  size="icon"
                  variant="ghost"
                  className="text-muted-foreground hover:text-destructive"
                  onClick={() => confirmDelete(p)}
                  aria-label={`${t("card.delete")} ${p.name}`}
                >
                  <Trash2 />
                </Button>
              </div>
            );
          })}
        </Card>
      )}

      <Suspense fallback={null}>
      {adding && (
        <ProviderEditor
          tool={tool}
          open
          onOpenChange={setAdding}
          existingIds={data.providers.map((p) => p.id)}
          accounts={accounts}
          logins={logins}
        />
      )}
      {editing && (
        <ProviderEditor
          key={editing.id}
          tool={tool}
          open
          onOpenChange={(open) => !open && setEditing(null)}
          provider={editing}
          existingIds={data.providers.map((p) => p.id)}
          accounts={accounts}
          logins={logins}
        />
      )}
      </Suspense>
    </div>
  );
}
