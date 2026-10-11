import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import * as Tabs from "@radix-ui/react-tabs";
import { AlertTriangle, CheckCircle2, FileUp, FolderSearch, Loader2, RefreshCw, ShieldCheck } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useSettings, refreshAll } from "@/hooks/queries";
import { api, errorMessage, reasonText, type ImportReport, type ImportScan, type ToolId } from "@/lib/api";
import { TOOLS, toolMeta } from "@/lib/tools";
import { cn } from "@/lib/utils";
import { ExportImport } from "./ExportImport";
import { BackupExport } from "./BackupPanel";

export function ImportWizard({ onBusyChange }: { onBusyChange?: (busy: boolean) => void }) {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const settings = useSettings().data;
  const [source, setSource] = useState("local");
  const [dir, setDir] = useState("");
  const [scannedDir, setScannedDir] = useState<string | undefined>();
  const [scan, setScan] = useState<ImportScan | null>(null);
  const [selected, setSelected] = useState<Set<ToolId>>(new Set());
  const [report, setReport] = useState<ImportReport | null>(null);
  const [operation, setOperation] = useState<"scan" | "import" | null>(null);
  const [error, setError] = useState<{ action: "scan" | "import"; message: string } | null>(null);
  const [fileBusy, setFileBusy] = useState(false);
  const [includeAccounts, setIncludeAccounts] = useState(true);
  const pending = useRef(false);
  const busy = operation !== null;

  useEffect(() => {
    onBusyChange?.(busy || fileBusy);
    return () => onBusyChange?.(false);
  }, [busy, fileBusy, onBusyChange]);

  const byTool = useMemo(() => {
    const map = new Map<ToolId, ImportScan["candidates"]>();
    for (const c of scan?.candidates ?? []) map.set(c.tool, [...(map.get(c.tool) ?? []), c]);
    // The account store can exist without a provider database or any Codex cards.
    if (scan && scan.accounts > 0 && !map.has("codex")) map.set("codex", []);
    return TOOLS.filter((tool) => map.has(tool.id)).map((tool) => ({ tool: tool.id, items: map.get(tool.id)! }));
  }, [scan]);

  function changeDir(next: string) {
    if (pending.current) return;
    setDir(next);
    setScan(null);
    setScannedDir(undefined);
    setSelected(new Set());
    setReport(null);
    setError(null);
  }

  async function runScan() {
    if (pending.current) return;
    const requestedDir = dir.trim() || undefined;
    setScan(null);
    setSelected(new Set());
    setReport(null);
    setError(null);
    if (requestedDir && !/^(?:\/|[a-z]:[\\/]|\\\\)/i.test(requestedDir)) {
      setError({ action: "scan", message: t("wizard.absoluteDir") });
      return;
    }
    pending.current = true;
    setOperation("scan");
    try {
      const result = await api.ccswitchScan(requestedDir);
      setScannedDir(result.directory ?? requestedDir);
      setScan(result);
      setSelected(new Set([...result.candidates.filter((c) => !c.skipReason).map((c) => c.tool), ...(result.accounts > 0 ? ["codex" as const] : [])]));
    } catch (e) {
      setError({ action: "scan", message: errorMessage(e) });
    } finally {
      pending.current = false;
      setOperation(null);
    }
  }

  async function runImport() {
    if (pending.current || !scan || selected.size === 0) return;
    pending.current = true;
    setOperation("import");
    setError(null);
    try {
      setReport(await api.ccswitchApply([...selected], scannedDir, includeAccounts));
      void refreshAll(client);
    } catch (e) {
      setError({ action: "import", message: errorMessage(e) });
    } finally {
      pending.current = false;
      setOperation(null);
    }
  }

  const step = report ? 3 : scan && (scan.source || scan.accounts > 0) ? 2 : 1;
  const cardCount = scan?.candidates.filter((c) => !c.skipReason && selected.has(c.tool)).length ?? 0;
  const accountCount = includeAccounts && selected.has("codex") ? scan?.accounts ?? 0 : 0;
  const canImport = cardCount > 0 || accountCount > 0;

  return (
    <div className="mx-auto flex min-w-0 max-w-5xl flex-col gap-7 p-6 lg:p-9">
      <header className="border-b pb-6">
        <h1 className="text-2xl font-semibold tracking-tight">{t("backup.dataTitle")}</h1>
        <p className="mt-2 max-w-prose text-sm leading-6 text-muted-foreground">{t("backup.dataHint")}</p>
      </header>
      <Tabs.Root value={source} onValueChange={(next) => !busy && !fileBusy && setSource(next)} className="min-w-0">
        <Tabs.List aria-label={t("wizard.sourceType")} className="mb-7 flex max-w-full flex-wrap gap-1 rounded-xl border bg-muted p-1.5">
          <Tabs.Trigger value="local" disabled={busy || fileBusy} className="inline-flex min-h-9 items-center gap-2 rounded-md px-3 text-sm font-medium text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50 data-[state=active]:bg-background data-[state=active]:text-foreground"><FolderSearch className="h-4 w-4" aria-hidden="true" />{t("wizard.localTab")}</Tabs.Trigger>
          <Tabs.Trigger value="file" disabled={busy || fileBusy} className="inline-flex min-h-9 items-center gap-2 rounded-md px-3 text-sm font-medium text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50 data-[state=active]:bg-background data-[state=active]:text-foreground"><FileUp className="h-4 w-4" aria-hidden="true" />{t("wizard.fileTab")}</Tabs.Trigger>
          <Tabs.Trigger value="backup" disabled={busy || fileBusy} className="inline-flex min-h-9 items-center gap-2 rounded-md px-3 text-sm font-medium text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50 data-[state=active]:bg-background data-[state=active]:text-foreground"><ShieldCheck className="h-4 w-4" aria-hidden="true" />{t("backup.exportTab")}</Tabs.Trigger>
        </Tabs.List>
        <Tabs.Content value="local" forceMount hidden={source !== "local"} className="min-w-0 space-y-5 focus-visible:outline-none">
          <div className="flex flex-wrap items-start justify-between gap-5 rounded-xl border bg-card p-5">
            <div className="max-w-xl"><FolderSearch className="mb-4 size-7 text-primary" aria-hidden="true" /><h2 className="text-lg font-semibold">{t("wizard.localTitle")}</h2>
            <p className="mt-2 text-sm leading-6 text-muted-foreground">{t("backup.localHint")}</p></div>
            <Button className="mt-2" onClick={() => void runScan()} disabled={busy}>{operation === "scan" ? <Loader2 className="animate-spin" /> : <FolderSearch />}{t(operation === "scan" ? "wizard.scanning" : "backup.localAction")}</Button>
          </div>
          <ol aria-label={t("wizard.progress")} className="flex flex-wrap gap-x-5 gap-y-2 text-sm">
            {[1, 2, 3].map((n) => <li key={n} aria-current={step === n ? "step" : undefined} className={cn("text-muted-foreground", step === n && "font-semibold text-foreground")}>{t(`wizard.step${n}`)}</li>)}
          </ol>
          <details className="rounded-lg border px-4 py-3"><summary className="cursor-pointer text-sm font-medium">{t("backup.manualDir")}</summary><div className="mt-3 flex flex-wrap items-end gap-2">
            <div className="flex min-w-0 flex-1 basis-72 flex-col gap-1.5">
              <Label htmlFor="wiz-dir">{t("wizard.dir")}</Label>
              <Input id="wiz-dir" placeholder={settings?.ccswitchDir} value={dir} onChange={(e) => changeDir(e.target.value)} disabled={busy} aria-describedby="wiz-dir-hint" />
            </div>
            <Button onClick={() => void runScan()} disabled={busy}>{operation === "scan" ? <Loader2 className="animate-spin" /> : <FolderSearch />}{t(operation === "scan" ? "wizard.scanning" : "wizard.scan")}</Button>
            <p id="wiz-dir-hint" className="w-full text-xs leading-relaxed text-muted-foreground">{t("wizard.dirHint")}</p>
          </div></details>

          {error && <div role="alert" className="flex min-w-0 items-start gap-2 rounded-lg border border-destructive/25 bg-destructive/5 p-4">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-destructive" aria-hidden="true" />
            <div className="min-w-0 space-y-2 text-sm">
              <p className="font-medium">{t(error.action === "scan" ? "wizard.scanFailed" : "wizard.importFailed")}</p>
              <p className="break-words text-muted-foreground [overflow-wrap:anywhere]">{error.message}</p>
              <Button size="sm" variant="outline" disabled={busy} onClick={() => void (error.action === "scan" ? runScan() : runImport())}>{t("common.retry")}</Button>
            </div>
          </div>}
          {busy && <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground"><Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />{t(operation === "scan" ? "wizard.scanProgress" : "wizard.importing")}</p>}
          {scan && !scan.source && scan.accounts === 0 && <div className="rounded-lg border border-dashed p-5 text-sm"><p className="font-medium">{t("wizard.noSource")}</p><p className="mt-1 text-muted-foreground">{t("wizard.noSourceHint")}</p></div>}

          {scan && (scan.source || scan.accounts > 0) && !report && <>
            <p className="break-all font-mono text-xs leading-relaxed text-muted-foreground">{t("wizard.source", { path: scan.source ?? scannedDir ?? settings?.ccswitchDir ?? t("wizard.defaultDir") })}</p>
            {scan.accounts > 0 && <label className="flex items-center gap-2 text-sm"><Checkbox checked={includeAccounts} disabled={busy} onCheckedChange={(checked) => setIncludeAccounts(checked === true)} />{t("backup.includeAccounts")}<span className="text-muted-foreground">({scan.accounts})</span></label>}
            {byTool.length === 0 && <p className="rounded-lg border border-dashed p-5 text-sm text-muted-foreground">{t("wizard.noItems")}</p>}
            <div className="flex flex-col gap-3">
              {byTool.map(({ tool, items }) => {
                const ok = items.filter((c) => !c.skipReason);
                const hasAccounts = tool === "codex" && scan.accounts > 0;
                return <Card key={tool} className="min-w-0 p-4">
                  <label className="flex min-w-0 flex-wrap items-center gap-2">
                    <Checkbox checked={selected.has(tool)} disabled={busy || (ok.length === 0 && !hasAccounts)} aria-label={t("wizard.selectTool", { tool: toolMeta(tool).label })} onCheckedChange={(v) => setSelected((s) => { const next = new Set(s); if (v === true) next.add(tool); else next.delete(tool); return next; })} />
                    <span className="font-medium">{toolMeta(tool).label}</span>
                    {ok.length > 0 && <Badge variant="secondary">{t("wizard.willImport", { count: ok.length })}</Badge>}
                    {hasAccounts && <Badge variant="secondary">{t("wizard.accountCount", { count: scan.accounts })}</Badge>}
                  </label>
                  {items.length > 0 && <ul className="mt-3 grid gap-2 pl-6 text-sm">
                    {items.map((c) => <li key={c.provider.id} className={cn("break-words [overflow-wrap:anywhere]", c.skipReason && "text-muted-foreground")}>{c.provider.name} <span className="font-mono text-xs text-muted-foreground">{c.provider.id}</span>{c.skipReason && <span className="ml-2 text-xs">· {t("wizard.skipped")}: {reasonText(c.skipReason, i18n.language)}</span>}</li>)}
                  </ul>}
                </Card>;
              })}
            </div>
            <div className="flex flex-wrap items-center justify-between gap-3 border-t pt-4">
              <p className="text-sm text-muted-foreground" role="status">{t("wizard.selection", { cards: cardCount, accounts: accountCount })}</p>
              <Button onClick={() => void runImport()} disabled={busy || !canImport}>{operation === "import" && <Loader2 className="animate-spin" />}{t("wizard.import")}</Button>
            </div>
          </>}
          {report && <Card className="flex min-w-0 items-start gap-3 p-4" role="status">
            <CheckCircle2 className="mt-0.5 h-5 w-5 shrink-0 text-emerald-700" aria-hidden="true" />
            <div className="min-w-0 space-y-3 text-sm">
              <p className="font-medium">{t("wizard.done", { created: report.created.length, accounts: report.accountsAdded.length, skipped: report.skipped.length })}</p>
              <p className="text-muted-foreground">{t("wizard.doneHint")}</p>
              {report.skipped.length > 0 && <ul className="space-y-1 text-xs text-muted-foreground">{report.skipped.map(([tool, id, reason]) => <li key={`${tool}/${id}`} className="break-words [overflow-wrap:anywhere]">{toolMeta(tool).label} / {id}: {reasonText(reason, i18n.language)}</li>)}</ul>}
              <Button variant="outline" size="sm" disabled={busy} onClick={() => void runScan()}><RefreshCw />{t("wizard.scanAgain")}</Button>
            </div>
          </Card>}
        </Tabs.Content>
        <Tabs.Content value="file" forceMount hidden={source !== "file"} className="min-w-0 focus-visible:outline-none"><ExportImport onBusyChange={setFileBusy} /></Tabs.Content>
        <Tabs.Content value="backup" forceMount hidden={source !== "backup"} className="min-w-0 focus-visible:outline-none"><BackupExport onBusyChange={setFileBusy} /></Tabs.Content>
      </Tabs.Root>
    </div>
  );
}
