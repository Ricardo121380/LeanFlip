import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { CheckCircle2, FolderSearch } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useSettings, refreshAll } from "@/hooks/queries";
import { api, errorMessage, type ImportReport, type ImportScan, type ToolId } from "@/lib/api";
import { TOOLS, toolMeta } from "@/lib/tools";
import { cn } from "@/lib/utils";

export function ImportWizard() {
  const { t } = useTranslation();
  const client = useQueryClient();
  const settings = useSettings().data;
  const [dir, setDir] = useState<string>("");
  const [scan, setScan] = useState<ImportScan | null>(null);
  const [selected, setSelected] = useState<Set<ToolId>>(new Set());
  const [report, setReport] = useState<ImportReport | null>(null);
  const [busy, setBusy] = useState(false);

  const byTool = useMemo(() => {
    const map = new Map<ToolId, ImportScan["candidates"]>();
    for (const c of scan?.candidates ?? []) map.set(c.tool, [...(map.get(c.tool) ?? []), c]);
    return TOOLS.filter((tool) => map.has(tool.id)).map((tool) => ({ tool: tool.id, items: map.get(tool.id)! }));
  }, [scan]);

  async function runScan() {
    setBusy(true);
    setReport(null);
    try {
      const result = await api.ccswitchScan(dir.trim() || undefined);
      setScan(result);
      setSelected(
        new Set(
          result.candidates.filter((c) => !c.skipReason).map((c) => c.tool),
        ),
      );
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  async function runImport() {
    setBusy(true);
    try {
      setReport(await api.ccswitchApply([...selected], dir.trim() || undefined));
      void refreshAll(client);
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  const step = report ? 3 : scan ? 2 : 1;

  return (
    <div className="flex max-w-4xl flex-col gap-5 p-6">
      <header>
        <h1 className="text-xl font-semibold">{t("wizard.title")}</h1>
        <p className="mt-1 text-sm text-muted-foreground">{t("wizard.hint")}</p>
      </header>
      <ol className="flex gap-4 text-sm">
        {[1, 2, 3].map((n) => (
          <li key={n} className={cn("text-muted-foreground", step === n && "font-semibold text-foreground")}>
            {t(`wizard.step${n}`)}
          </li>
        ))}
      </ol>

      <div className="flex items-end gap-2">
        <div className="flex flex-1 flex-col gap-1.5">
          <Label htmlFor="wiz-dir">{t("wizard.dir")}</Label>
          <Input id="wiz-dir" placeholder={settings?.ccswitchDir} value={dir} onChange={(e) => setDir(e.target.value)} />
        </div>
        <Button onClick={() => void runScan()} disabled={busy}>
          <FolderSearch /> {t("wizard.scan")}
        </Button>
      </div>

      {scan && !scan.source && <p className="text-sm text-muted-foreground">{t("wizard.noSource")}</p>}

      {scan?.source && !report && (
        <>
          <p className="text-xs text-muted-foreground">{t("wizard.source", { path: scan.source })}</p>
          {scan.accounts > 0 && <p className="text-sm">{t("wizard.accounts", { count: scan.accounts })}</p>}
          <div className="flex flex-col gap-3">
            {byTool.map(({ tool, items }) => {
              const ok = items.filter((c) => !c.skipReason);
              return (
                <Card key={tool} className="p-4">
                  <label className="flex items-center gap-2">
                    <Checkbox
                      checked={selected.has(tool)}
                      disabled={ok.length === 0 && !(tool === "codex" && scan.accounts > 0)}
                      aria-label={`select ${tool}`}
                      onCheckedChange={(v) =>
                        setSelected((s) => {
                          const next = new Set(s);
                          if (v === true) next.add(tool);
                          else next.delete(tool);
                          return next;
                        })
                      }
                    />
                    <span className="font-medium">{toolMeta(tool).label}</span>
                    <Badge variant="secondary">{t("wizard.willImport", { count: ok.length })}</Badge>
                  </label>
                  <ul className="mt-2 grid gap-1 pl-6 text-sm">
                    {items.map((c) => (
                      <li key={c.provider.id} className={cn(c.skipReason && "text-muted-foreground")}>
                        {c.provider.name} <span className="font-mono text-xs text-muted-foreground">{c.provider.id}</span>
                        {c.skipReason && (
                          <span className="ml-2 text-xs">
                            · {t("wizard.skipped")}: {c.skipReason}
                          </span>
                        )}
                      </li>
                    ))}
                  </ul>
                </Card>
              );
            })}
          </div>
          <div>
            <Button onClick={() => void runImport()} disabled={busy || selected.size === 0}>
              {t("wizard.import")}
            </Button>
          </div>
        </>
      )}

      {report && (
        <Card className="flex items-start gap-3 p-4">
          <CheckCircle2 className="mt-0.5 h-5 w-5 text-emerald-600" />
          <div className="text-sm">
            <p>
              {t("wizard.done", {
                created: report.created.length,
                accounts: report.accountsAdded.length,
                skipped: report.skipped.length,
              })}
            </p>
            {report.skipped.length > 0 && (
              <ul className="mt-2 text-xs text-muted-foreground">
                {report.skipped.map(([tool, id, reason]) => (
                  <li key={`${tool}/${id}`}>
                    {toolMeta(tool).label} / {id}: {reason}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </Card>
      )}
    </div>
  );
}
