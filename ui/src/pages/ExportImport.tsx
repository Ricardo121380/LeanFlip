import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, CheckCircle2, FileUp, Loader2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/input";
import { refreshAll } from "@/hooks/queries";
import {
  api,
  errorMessage,
  reasonText,
  type DuplicatePolicy,
  type ExportItem,
  type ExportPreview,
  type ExportReport,
  type Reason,
  type ToolId,
} from "@/lib/api";
import { TOOLS, toolMeta } from "@/lib/tools";
import { cn } from "@/lib/utils";

/** 按工具分组；不支持的应用归到最后一组（tool = null）。 */
export function groupItems(items: ExportItem[]) {
  const groups: { tool: ToolId | null; items: ExportItem[] }[] = [];
  for (const tool of TOOLS) {
    const list = items.filter((i) => i.tool === tool.id);
    if (list.length) groups.push({ tool: tool.id, items: list });
  }
  const other = items.filter((i) => !i.tool);
  if (other.length) groups.push({ tool: null, items: other });
  return groups;
}

/** 有可导入卡的工具。 */
export function importableTools(preview: ExportPreview | null): Set<ToolId> {
  return new Set(
    (preview?.items ?? []).filter((i) => i.tool && i.targetId).map((i) => i.tool as ToolId),
  );
}

/** 读文件字节（老 WebView / jsdom 没有 Blob.arrayBuffer 时用 FileReader）。 */
export function readBytes(file: Blob): Promise<Uint8Array> {
  if (typeof file.arrayBuffer === "function") return file.arrayBuffer().then((b) => new Uint8Array(b));
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(new Uint8Array(reader.result as ArrayBuffer));
    reader.onerror = () => reject(reader.error);
    reader.readAsArrayBuffer(file);
  });
}

type Operation = "load" | "preview" | "apply" | "discard";

export function ExportImport({ onBusyChange }: { onBusyChange?: (busy: boolean) => void } = {}) {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const input = useRef<HTMLInputElement>(null);
  const file = useRef<File | null>(null);
  const pending = useRef(false);
  const [fileName, setFileName] = useState<string | null>(null);
  const [policy, setPolicy] = useState<DuplicatePolicy>("skip");
  const [loaded, setLoaded] = useState(false);
  const [preview, setPreview] = useState<ExportPreview | null>(null);
  const [selected, setSelected] = useState<Set<ToolId>>(new Set());
  const [report, setReport] = useState<ExportReport | null>(null);
  const [operation, setOperation] = useState<Operation | null>(null);
  const [error, setError] = useState<{ action: Operation; message: string } | null>(null);
  const busy = operation !== null;
  const text = (r: Reason) => reasonText(r, i18n.language);
  const groups = useMemo(() => groupItems(preview?.items ?? []), [preview]);

  function begin(action: Operation) {
    if (pending.current) return false;
    pending.current = true;
    setOperation(action);
    setError(null);
    onBusyChange?.(true);
    return true;
  }

  function finish() {
    pending.current = false;
    setOperation(null);
    onBusyChange?.(false);
  }

  function show(next: ExportPreview, preserveSelection = false) {
    setPreview(next);
    const eligible = importableTools(next);
    setSelected((previous) => preserveSelection ? new Set([...previous].filter((tool) => eligible.has(tool))) : eligible);
  }

  async function pick(next: File | undefined) {
    if (!next || !begin("load")) return;
    file.current = next;
    setReport(null);
    setPreview(null);
    setLoaded(false);
    setFileName(next.name);
    try {
      show(await api.exportLoad(await readBytes(next), policy));
      setLoaded(true);
    } catch (e) {
      setError({ action: "load", message: errorMessage(e) });
    } finally {
      finish();
      if (input.current) input.current.value = "";
    }
  }

  async function changePolicy(next: DuplicatePolicy) {
    if (pending.current) return;
    setPolicy(next);
    if (!loaded) return;
    if (!begin("preview")) return;
    // A displayed preview must always correspond to the currently chosen policy.
    setPreview(null);
    setReport(null);
    try {
      show(await api.exportPreview(next), true);
    } catch (e) {
      setError({ action: "preview", message: errorMessage(e) });
    } finally {
      finish();
    }
  }

  async function confirm() {
    if (!preview || toImport === 0 || !begin("apply")) return;
    try {
      setReport(await api.exportApply([...selected], policy));
      setPreview(null);
      setLoaded(false);
      file.current = null;
      void refreshAll(client);
    } catch (e) {
      setError({ action: "apply", message: errorMessage(e) });
    } finally {
      finish();
    }
  }

  async function cancel() {
    if (!begin("discard")) return;
    try {
      await api.exportDiscard();
      setPreview(null);
      setSelected(new Set());
      setFileName(null);
      setLoaded(false);
      setReport(null);
      file.current = null;
    } catch (e) {
      setError({ action: "discard", message: errorMessage(e) });
    } finally {
      finish();
    }
  }

  function retry() {
    switch (error?.action) {
      case "load": void pick(file.current ?? undefined); break;
      case "preview": void changePolicy(policy); break;
      case "apply": void confirm(); break;
      case "discard": void cancel(); break;
    }
  }

  const count = (status: ExportItem["status"]) => preview?.items.filter((i) => i.status === status).length ?? 0;
  const toImport = preview?.items.filter((i) => i.targetId && i.tool && selected.has(i.tool)).length ?? 0;
  const step = report ? 3 : loaded ? 2 : 1;

  return (
    <section className="flex min-w-0 flex-col gap-5">
      <div>
        <h2 className="text-base font-semibold">{t("exportImport.title")}</h2>
        <p className="mt-1 text-sm leading-relaxed text-muted-foreground">{t("exportImport.hint")}</p>
      </div>
      <ol aria-label={t("wizard.progress")} className="flex flex-wrap gap-x-5 gap-y-2 text-sm">
        {[1, 2, 3].map((n) => <li key={n} aria-current={step === n ? "step" : undefined} className={cn("text-muted-foreground", step === n && "font-semibold text-foreground")}>{t(`exportImport.step${n}`)}</li>)}
      </ol>
      <input ref={input} type="file" accept=".sql,.db,.json,application/sql,application/json" className="hidden" aria-label={t("exportImport.file")} data-testid="export-file" disabled={busy} onChange={(e) => void pick(e.target.files?.[0])} />
      {!report && <div className="flex min-w-0 flex-wrap items-end gap-3">
        <Button variant="outline" onClick={() => input.current?.click()} disabled={busy}><FileUp />{t("exportImport.choose")}</Button>
        <div className="flex min-w-0 flex-col gap-1.5">
          <Label htmlFor="exp-dup">{t("exportImport.duplicates")}</Label>
          <NativeSelect id="exp-dup" value={policy} disabled={busy} onChange={(e) => void changePolicy(e.target.value as DuplicatePolicy)}>
            <option value="skip">{t("exportImport.dupSkip")}</option>
            <option value="rename">{t("exportImport.dupRename")}</option>
          </NativeSelect>
        </div>
      </div>}
      {fileName && <p className="break-all font-mono text-xs leading-relaxed text-muted-foreground">{t("exportImport.fileSource", { name: fileName })}</p>}
      {busy && <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground"><Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />{t(`exportImport.progress.${operation}`)}</p>}
      {error && <div role="alert" className="flex min-w-0 items-start gap-2 rounded-lg border border-destructive/25 bg-destructive/5 p-4">
        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-destructive" aria-hidden="true" />
        <div className="min-w-0 space-y-2 text-sm">
          <p className="font-medium">{t(`exportImport.failed.${error.action}`)}</p>
          <p className="break-words text-muted-foreground [overflow-wrap:anywhere]">{error.message}</p>
          <Button size="sm" variant="outline" disabled={busy} onClick={retry}>{t("common.retry")}</Button>
        </div>
      </div>}
      {!fileName && !busy && <div className="rounded-lg border border-dashed p-5 text-sm text-muted-foreground">{t("exportImport.empty")}</div>}

      {preview && <>
        <p className="text-sm">{t(`exportImport.format.${preview.format}`)} · {t("exportImport.summary", { import: count("import"), duplicate: count("duplicate"), skip: count("skip") })}</p>
        {preview.items.length === 0 && <p className="rounded-lg border border-dashed p-5 text-sm text-muted-foreground">{t("exportImport.noItems")}</p>}
        <div className="flex flex-col gap-3">
          {groups.map(({ tool, items }) => {
            const ok = items.filter((i) => i.targetId).length;
            return <Card key={tool ?? "other"} className="min-w-0 p-4" data-testid={`export-group-${tool ?? "other"}`}>
              <label className="flex min-w-0 flex-wrap items-center gap-2">
                {tool && <Checkbox checked={selected.has(tool)} disabled={busy || ok === 0} aria-label={t("exportImport.selectTool", { tool: toolMeta(tool).label })} onCheckedChange={(v) => setSelected((s) => { const next = new Set(s); if (v === true) next.add(tool); else next.delete(tool); return next; })} />}
                <span className="font-medium">{tool ? toolMeta(tool).label : t("exportImport.otherApps")}</span>
                <Badge variant="secondary">{t("wizard.willImport", { count: ok })}</Badge>
              </label>
              <ul className="mt-3 grid gap-2 pl-6 text-sm">
                {items.map((item) => <li key={`${item.app}/${item.sourceId}`} className={cn("break-words [overflow-wrap:anywhere]", !item.targetId && "text-muted-foreground")} data-testid={`export-item-${item.app}-${item.sourceId}`}>
                  <Badge variant={item.status === "import" ? "default" : "secondary"} className="mr-2">{t(`exportImport.status.${item.targetId && item.status === "duplicate" ? "renamed" : item.status}`)}</Badge>
                  {item.name} <span className="font-mono text-xs text-muted-foreground">{item.sourceId}{item.targetId && item.targetId !== item.sourceId && ` → ${item.targetId}`}</span>
                  {item.reason && <span className="ml-2 text-xs">· {text(item.reason)}</span>}
                  {item.notes.map((n) => <p key={n.code} className="mt-1 text-xs text-amber-800">{text(n)}</p>)}
                </li>)}
              </ul>
            </Card>;
          })}
        </div>
        <div className="flex flex-wrap items-center justify-between gap-3 border-t pt-4">
          <p role="status" className="text-sm text-muted-foreground">{t("exportImport.selection", { count: toImport, tools: selected.size })}</p>
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" onClick={() => void cancel()} disabled={busy}>{t("exportImport.reset")}</Button>
            <Button onClick={() => void confirm()} disabled={busy || toImport === 0}>{operation === "apply" && <Loader2 className="animate-spin" />}{t("exportImport.confirm", { count: toImport })}</Button>
          </div>
        </div>
      </>}
      {report && <Card className="flex min-w-0 items-start gap-3 p-4" role="status">
        <CheckCircle2 className="mt-0.5 h-5 w-5 shrink-0 text-emerald-700" aria-hidden="true" />
        <div className="min-w-0 space-y-3 text-sm">
          <p className="font-medium">{t("exportImport.done", { created: report.created.length, skipped: report.skipped })}</p>
          <p className="text-muted-foreground">{t("exportImport.doneHint")}</p>
          <Button size="sm" variant="outline" onClick={() => input.current?.click()} disabled={busy}><FileUp />{t("exportImport.importAnother")}</Button>
        </div>
      </Card>}
    </section>
  );
}
