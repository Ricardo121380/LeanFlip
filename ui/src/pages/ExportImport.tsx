import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { CheckCircle2, FileUp } from "lucide-react";
import { toast } from "sonner";
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

export function ExportImport() {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const input = useRef<HTMLInputElement>(null);
  const [fileName, setFileName] = useState<string | null>(null);
  const [policy, setPolicy] = useState<DuplicatePolicy>("skip");
  const [preview, setPreview] = useState<ExportPreview | null>(null);
  const [selected, setSelected] = useState<Set<ToolId>>(new Set());
  const [report, setReport] = useState<ExportReport | null>(null);
  const [busy, setBusy] = useState(false);
  const text = (r: Reason) => reasonText(r, i18n.language);
  const groups = useMemo(() => groupItems(preview?.items ?? []), [preview]);

  function show(next: ExportPreview) {
    setPreview(next);
    setSelected(importableTools(next));
  }

  async function pick(file: File | undefined) {
    if (!file) return;
    setBusy(true);
    setReport(null);
    setPreview(null);
    setFileName(file.name);
    try {
      show(await api.exportLoad(await readBytes(file), policy));
    } catch (e) {
      setFileName(null);
      toast.error(errorMessage(e));
    } finally {
      setBusy(false);
      if (input.current) input.current.value = "";
    }
  }

  async function changePolicy(next: DuplicatePolicy) {
    setPolicy(next);
    if (!preview) return;
    try {
      show(await api.exportPreview(next));
    } catch (e) {
      toast.error(errorMessage(e));
    }
  }

  async function confirm() {
    setBusy(true);
    try {
      setReport(await api.exportApply([...selected], policy));
      setPreview(null);
      void refreshAll(client);
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  function cancel() {
    setPreview(null);
    setFileName(null);
    void api.exportDiscard().catch(() => {});
  }

  const count = (status: ExportItem["status"]) => preview?.items.filter((i) => i.status === status).length ?? 0;
  const toImport = preview?.items.filter((i) => i.targetId && i.tool && selected.has(i.tool)).length ?? 0;

  return (
    <section className="flex flex-col gap-3">
      <div>
        <h2 className="text-base font-semibold">{t("exportImport.title")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("exportImport.hint")}</p>
      </div>
      <div className="flex flex-wrap items-end gap-3">
        <input
          ref={input}
          type="file"
          accept=".sql,.db,.json,application/sql,application/json"
          className="hidden"
          aria-label={t("exportImport.file")}
          data-testid="export-file"
          onChange={(e) => void pick(e.target.files?.[0])}
        />
        <Button variant="outline" onClick={() => input.current?.click()} disabled={busy}>
          <FileUp /> {t("exportImport.choose")}
        </Button>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="exp-dup">{t("exportImport.duplicates")}</Label>
          <NativeSelect id="exp-dup" value={policy} onChange={(e) => void changePolicy(e.target.value as DuplicatePolicy)}>
            <option value="skip">{t("exportImport.dupSkip")}</option>
            <option value="rename">{t("exportImport.dupRename")}</option>
          </NativeSelect>
        </div>
        {fileName && <span className="pb-2 text-xs text-muted-foreground">{fileName}</span>}
      </div>

      {preview && (
        <>
          <p className="text-sm">
            {t(`exportImport.format.${preview.format}`)} ·{" "}
            {t("exportImport.summary", { import: count("import"), duplicate: count("duplicate"), skip: count("skip") })}
          </p>
          <div className="flex flex-col gap-3">
            {groups.map(({ tool, items }) => {
              const ok = items.filter((i) => i.targetId).length;
              return (
                <Card key={tool ?? "other"} className="p-4" data-testid={`export-group-${tool ?? "other"}`}>
                  <label className="flex items-center gap-2">
                    {tool && (
                      <Checkbox
                        checked={selected.has(tool)}
                        disabled={ok === 0}
                        aria-label={t("exportImport.selectTool", { tool: toolMeta(tool).label })}
                        onCheckedChange={(v) =>
                          setSelected((s) => {
                            const next = new Set(s);
                            if (v === true) next.add(tool);
                            else next.delete(tool);
                            return next;
                          })
                        }
                      />
                    )}
                    <span className="font-medium">{tool ? toolMeta(tool).label : t("exportImport.otherApps")}</span>
                    <Badge variant="secondary">{t("wizard.willImport", { count: ok })}</Badge>
                  </label>
                  <ul className="mt-2 grid gap-1 pl-6 text-sm">
                    {items.map((item) => (
                      <li
                        key={`${item.app}/${item.sourceId}`}
                        className={cn(!item.targetId && "text-muted-foreground")}
                        data-testid={`export-item-${item.app}-${item.sourceId}`}
                      >
                        <Badge
                          variant={item.status === "import" ? "default" : "secondary"}
                          className="mr-2 text-[10px]"
                        >
                          {t(`exportImport.status.${item.targetId && item.status === "duplicate" ? "renamed" : item.status}`)}
                        </Badge>
                        {item.name}{" "}
                        <span className="font-mono text-xs text-muted-foreground">
                          {item.sourceId}
                          {item.targetId && item.targetId !== item.sourceId && ` → ${item.targetId}`}
                        </span>
                        {item.reason && <span className="ml-2 text-xs">· {text(item.reason)}</span>}
                        {item.notes.map((n) => (
                          <p key={n.code} className="text-xs text-amber-700">
                            {text(n)}
                          </p>
                        ))}
                      </li>
                    ))}
                  </ul>
                </Card>
              );
            })}
          </div>
          <div className="flex gap-2">
            <Button onClick={() => void confirm()} disabled={busy || toImport === 0}>
              {t("exportImport.confirm", { count: toImport })}
            </Button>
            <Button variant="outline" onClick={cancel} disabled={busy}>
              {t("editor.cancel")}
            </Button>
          </div>
        </>
      )}

      {report && (
        <Card className="flex items-start gap-3 p-4">
          <CheckCircle2 className="mt-0.5 h-5 w-5 text-emerald-600" />
          <p className="text-sm">{t("exportImport.done", { created: report.created.length, skipped: report.skipped })}</p>
        </Card>
      )}
    </section>
  );
}
