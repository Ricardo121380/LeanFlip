import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { CheckCircle2, FolderSearch, Loader2, ShieldCheck } from "lucide-react";
import { api, errorMessage, reasonText, type ImportReport, type ProviderImportPreview, type ProviderImportSource } from "@/lib/api";
import { refreshAll } from "@/hooks/queries";
import { toolMeta } from "@/lib/tools";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

const SOURCES: { id: ProviderImportSource; name: string; hint: string }[] = [
  { id: "cc-switch", name: "CC Switch", hint: "ccSwitchHint" },
  { id: "ai-toolbox", name: "AI Toolbox", hint: "aiToolboxHint" },
  { id: "superba", name: "Superba", hint: "superbaHint" },
];

export function ProviderImportSourcePicker({ value, onChange, disabled = false }: {
  value: ProviderImportSource;
  onChange: (source: ProviderImportSource) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const name = useId();
  return <fieldset disabled={disabled}><legend className="mb-2 text-sm font-medium">{t("onboarding.source")}</legend><div className="grid gap-2 sm:grid-cols-3">{SOURCES.map((item) => <label key={item.id} className={cn("flex cursor-pointer items-start gap-2 rounded-lg border p-3 focus-within:ring-2 focus-within:ring-ring", value === item.id && "border-primary bg-primary/5", disabled && "cursor-wait opacity-60")}>
    <input type="radio" name={name} className="mt-1 accent-primary" value={item.id} checked={value === item.id} onChange={() => onChange(item.id)} />
    <span><span className="text-sm font-medium">{item.name}</span><span className="mt-1 block text-xs leading-5 text-muted-foreground">{t(`onboarding.${item.hint}`)}</span></span>
  </label>)}</div></fieldset>;
}

/** Shared provider preview/import flow. Mount with a source key to reset private input on source changes. */
export function ProviderImportPanel({ source, disabled = false, onBusyChange, onImportedChange }: {
  source: ProviderImportSource;
  disabled?: boolean;
  onBusyChange?: (busy: boolean) => void;
  onImportedChange?: (imported: boolean) => void;
}) {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const [directory, setDirectory] = useState("");
  const [preview, setPreview] = useState<ProviderImportPreview | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [apiKeys, setApiKeys] = useState<Record<number, string>>({});
  const [report, setReport] = useState<ImportReport | null>(null);
  const [operation, setOperation] = useState<"scan" | "import" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pending = useRef(false);
  const busy = disabled || operation !== null;

  // Discard the backend snapshot after close, including its private credentials.
  useEffect(() => () => { void api.providerImportDiscard().catch(() => {}); }, []);

  function resetPreview() {
    setPreview(null); setSelected(new Set()); setApiKeys({}); setReport(null); setError(null); onImportedChange?.(false);
  }

  async function scan() {
    if (disabled || pending.current) return;
    pending.current = true; setOperation("scan"); onBusyChange?.(true); resetPreview();
    try {
      const result = await api.providerImportScan(source, directory.trim());
      setPreview(result);
      setSelected(new Set(result.items.filter((item) => !item.skipReason).map((item) => item.index)));
    } catch (e) { setError(errorMessage(e)); }
    finally { pending.current = false; setOperation(null); onBusyChange?.(false); }
  }

  async function importProviders() {
    if (disabled || pending.current || !preview || selected.size === 0) return;
    pending.current = true; setOperation("import"); onBusyChange?.(true); setError(null);
    try {
      const result = await api.providerImportApply(preview.previewId, [...selected].map((index) => ({ index, apiKey: apiKeys[index] })));
      setReport(result); setApiKeys({}); onImportedChange?.(true);
      void refreshAll(client);
    } catch (e) { setError(errorMessage(e)); }
    finally { pending.current = false; setOperation(null); onBusyChange?.(false); }
  }

  const missingKey = preview?.items.some((item) => selected.has(item.index) && item.needsApiKey && !apiKeys[item.index]?.trim());

  return <div className="space-y-5">
    <div className="flex items-start gap-2 text-xs leading-5 text-muted-foreground"><ShieldCheck className="mt-0.5 size-4 shrink-0" /><p>{t("onboarding.importSafety")}</p></div>
    <details className="rounded-lg border px-3 py-2"><summary className="cursor-pointer text-sm">{t("onboarding.customDirectory")}</summary><div className="mt-3 space-y-2"><Label htmlFor="provider-import-directory">{t("onboarding.directory")}</Label><Input id="provider-import-directory" value={directory} disabled={busy} placeholder={t("onboarding.directoryPlaceholder")} onChange={(e) => { setDirectory(e.target.value); resetPreview(); }} /><p className="text-xs text-muted-foreground">{t("onboarding.directoryHint")}</p></div></details>
    <Button variant="outline" disabled={busy} onClick={() => void scan()}>{operation === "scan" ? <Loader2 className="animate-spin" /> : <FolderSearch />}{t(preview ? "wizard.scanAgain" : "onboarding.scan")}</Button>
    {operation === "scan" && <p role="status" className="text-sm text-muted-foreground">{t("wizard.scanProgress")}</p>}
    {preview && <div className="space-y-3">
      <p className="break-all text-xs text-muted-foreground">{t("wizard.source", { path: preview.directory })}</p>
      {(!preview.found || preview.items.length === 0) && <p role="status" className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">{t(preview.found ? "onboarding.empty" : "onboarding.notFound")}</p>}
      {!report && preview.items.length > 0 && <>
        <div className="flex items-center justify-between text-xs"><span>{t("onboarding.selected", { count: selected.size })}</span><Button size="sm" variant="ghost" disabled={busy} onClick={() => setSelected(new Set(selected.size ? [] : preview.items.filter((item) => !item.skipReason).map((item) => item.index)))}>{t(selected.size ? "onboarding.clearSelection" : "onboarding.selectAll")}</Button></div>
        <ul className="divide-y rounded-lg border">{preview.items.map((item) => <li key={item.index} className="space-y-2 p-3">
          <label className={cn("flex items-start gap-3", item.skipReason && "text-muted-foreground")}><Checkbox className="mt-0.5" checked={selected.has(item.index)} disabled={busy || !!item.skipReason} onCheckedChange={(checked) => setSelected((old) => { const next = new Set(old); if (checked === true) next.add(item.index); else next.delete(item.index); return next; })} /><span className="min-w-0 flex-1 break-words text-sm [overflow-wrap:anywhere]">{item.name}<span className="ml-2 text-xs text-muted-foreground">{toolMeta(item.tool).label}</span></span></label>
          {item.skipReason && <p className="pl-7 text-xs leading-5 text-muted-foreground">{t("wizard.skipped")}: {reasonText(item.skipReason, i18n.language)}</p>}
          {item.notes.map((note) => <p key={note.code} className="pl-7 text-xs leading-5 text-muted-foreground">{reasonText(note, i18n.language)}</p>)}
          {item.needsApiKey && !item.skipReason && selected.has(item.index) && <div className="space-y-1.5 pl-7"><Label htmlFor={`import-key-${item.index}`}>{t("onboarding.apiKey", { name: item.name })}</Label><Input id={`import-key-${item.index}`} type="password" autoComplete="off" spellCheck={false} value={apiKeys[item.index] ?? ""} disabled={busy} onChange={(e) => setApiKeys((old) => ({ ...old, [item.index]: e.target.value }))} /><p className="text-xs leading-5 text-muted-foreground">{t("onboarding.keyHint")}</p></div>}
        </li>)}</ul>
        <Button disabled={busy || selected.size === 0 || missingKey} onClick={() => void importProviders()}>{operation === "import" && <Loader2 className="animate-spin" />}{t("onboarding.importSelected", { count: selected.size })}</Button>
      </>}
      {report && <div role="status" className="space-y-2 rounded-lg border border-success/30 bg-success/5 p-4 text-sm"><p className="flex items-center gap-2 font-medium"><CheckCircle2 className="size-4 text-success" />{t("onboarding.imported", { count: report.created.length, skipped: report.skipped.length })}</p><p className="text-muted-foreground">{t("onboarding.importedHint")}</p>{report.skipped.map(([tool, id, reason]) => <p key={`${tool}/${id}`} className="break-all text-xs">{reasonText(reason, i18n.language)}</p>)}<Button variant="outline" size="sm" onClick={resetPreview} disabled={busy}>{t("onboarding.anotherSource")}</Button></div>}
    </div>}
    {error && <p role="alert" className="break-words rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive [overflow-wrap:anywhere]">{error}</p>}
  </div>;
}
