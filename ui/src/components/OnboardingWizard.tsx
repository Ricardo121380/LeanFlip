import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowRight, Check, CheckCircle2, FolderSearch, Layers, Loader2, MousePointer2, ShieldCheck } from "lucide-react";
import { api, errorMessage, reasonText, type ImportReport, type ProviderImportPreview, type ProviderImportSource } from "@/lib/api";
import { refreshAll } from "@/hooks/queries";
import { toolMeta } from "@/lib/tools";
import { cn } from "@/lib/utils";
import { BrandMark } from "@/components/BrandMark";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";

const SOURCES: { id: ProviderImportSource; name: string; hint: string }[] = [
  { id: "cc-switch", name: "CC Switch", hint: "ccSwitchHint" },
  { id: "ai-toolbox", name: "AI Toolbox", hint: "aiToolboxHint" },
  { id: "superba", name: "Superba", hint: "superbaHint" },
];

export function OnboardingWizard({ onClose }: { onClose: () => void }) {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const [step, setStep] = useState(0);
  const [source, setSource] = useState<ProviderImportSource>("cc-switch");
  const [directory, setDirectory] = useState("");
  const [preview, setPreview] = useState<ProviderImportPreview | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [apiKeys, setApiKeys] = useState<Record<number, string>>({});
  const [report, setReport] = useState<ImportReport | null>(null);
  const [operation, setOperation] = useState<"scan" | "import" | "finish" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pending = useRef(false);
  const title = useRef<HTMLHeadingElement>(null);
  const returnFocus = useRef(document.activeElement instanceof HTMLElement ? document.activeElement : null);
  const busy = operation !== null;

  useEffect(() => { title.current?.focus(); }, [step]);
  // Discard the backend snapshot after close, including its private credentials.
  useEffect(() => () => { void api.providerImportDiscard().catch(() => {}); }, []);

  function resetPreview() {
    setPreview(null); setSelected(new Set()); setApiKeys({}); setReport(null); setError(null);
  }

  async function scan() {
    if (pending.current) return;
    pending.current = true; setOperation("scan"); resetPreview();
    try {
      const result = await api.providerImportScan(source, directory.trim());
      setPreview(result);
      setSelected(new Set(result.items.filter((item) => !item.skipReason).map((item) => item.index)));
    } catch (e) { setError(errorMessage(e)); }
    finally { pending.current = false; setOperation(null); }
  }

  async function importProviders() {
    if (pending.current || !preview || selected.size === 0) return;
    pending.current = true; setOperation("import"); setError(null);
    try {
      const result = await api.providerImportApply(preview.previewId, [...selected].map((index) => ({ index, apiKey: apiKeys[index] })));
      setReport(result); setApiKeys({});
      void refreshAll(client);
    } catch (e) { setError(errorMessage(e)); }
    finally { pending.current = false; setOperation(null); }
  }

  async function finish() {
    if (pending.current) return;
    pending.current = true; setOperation("finish"); setError(null);
    try {
      await api.completeOnboarding();
      await refreshAll(client);
      onClose();
    } catch (e) { setError(errorMessage(e)); }
    finally { pending.current = false; setOperation(null); }
  }

  const missingKey = preview?.items.some((item) => selected.has(item.index) && item.needsApiKey && !apiKeys[item.index]?.trim());
  const next = () => { setError(null); setStep((s) => s + 1); };

  return <Dialog open onOpenChange={(open) => { if (!open && !pending.current) void finish(); }}>
    <DialogContent className="flex max-h-[92dvh] w-[calc(100%-1.5rem)] max-w-3xl flex-col gap-0 overflow-hidden p-0" closeDisabled={busy}
      onCloseAutoFocus={(event) => { event.preventDefault(); if (returnFocus.current?.isConnected) returnFocus.current.focus(); }}
      onInteractOutside={(event) => event.preventDefault()} onEscapeKeyDown={(event) => { if (pending.current) event.preventDefault(); }}>
      <header className="shrink-0 border-b px-6 pb-5 pt-6 sm:px-8">
        <div className="mb-5 flex items-center gap-2.5"><BrandMark className="size-7" /><span className="font-semibold">LeanFlip</span><span className="ml-auto pr-6 text-xs text-muted-foreground">{t("onboarding.stepCount", { current: step + 1 })}</span></div>
        <DialogTitle ref={title} tabIndex={-1} className="text-2xl tracking-tight outline-none">{t(`onboarding.title${step}`)}</DialogTitle>
        <DialogDescription className="mt-2 leading-6">{t(`onboarding.description${step}`)}</DialogDescription>
        <ol aria-label={t("wizard.progress")} className="mt-5 flex gap-4 text-xs sm:gap-8">
          {[0, 1, 2].map((n) => <li key={n} aria-current={step === n ? "step" : undefined} className={cn("flex items-center gap-2 text-muted-foreground", step === n && "font-medium text-primary")}><span className={cn("grid size-5 place-items-center rounded-full border", step === n && "border-primary bg-primary text-primary-foreground")}>{step > n ? <Check className="size-3" /> : n + 1}</span>{t(`onboarding.step${n}`)}</li>)}
        </ol>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto px-6 py-6 sm:px-8" aria-busy={busy}>
        {step === 0 && <div className="space-y-6">
          <div className="rounded-xl border bg-muted/40 p-5"><Layers className="mb-3 size-8 text-primary" /><h3 className="font-semibold">{t("onboarding.onePlace")}</h3><p className="mt-2 text-sm leading-6 text-muted-foreground">{t("onboarding.onePlaceHint")}</p></div>
          <div className="grid gap-5 sm:grid-cols-2">{["bring", "startFresh"].map((key) => <div key={key}><h3 className="text-sm font-medium">{t(`onboarding.${key}`)}</h3><p className="mt-1.5 text-sm leading-6 text-muted-foreground">{t(`onboarding.${key}Hint`)}</p></div>)}</div>
        </div>}
        {step === 1 && <div className="space-y-5">
          <fieldset disabled={busy}><legend className="mb-2 text-sm font-medium">{t("onboarding.source")}</legend><div className="grid gap-2 sm:grid-cols-3">{SOURCES.map((item) => <label key={item.id} className={cn("flex cursor-pointer items-start gap-2 rounded-lg border p-3 focus-within:ring-2 focus-within:ring-ring", source === item.id && "border-primary bg-primary/5", busy && "cursor-wait opacity-60")}>
            <input type="radio" name="onboarding-source" className="mt-1 accent-primary" value={item.id} checked={source === item.id} onChange={() => { setSource(item.id); setDirectory(""); resetPreview(); }} />
            <span><span className="text-sm font-medium">{item.name}</span><span className="mt-1 block text-xs leading-5 text-muted-foreground">{t(`onboarding.${item.hint}`)}</span></span>
          </label>)}</div></fieldset>
          <div className="flex items-start gap-2 text-xs leading-5 text-muted-foreground"><ShieldCheck className="mt-0.5 size-4 shrink-0" /><p>{t("onboarding.importSafety")}</p></div>
          <details className="rounded-lg border px-3 py-2"><summary className="cursor-pointer text-sm">{t("onboarding.customDirectory")}</summary><div className="mt-3 space-y-2"><Label htmlFor="onboarding-directory">{t("onboarding.directory")}</Label><Input id="onboarding-directory" value={directory} disabled={busy} placeholder={t("onboarding.directoryPlaceholder")} onChange={(e) => { setDirectory(e.target.value); resetPreview(); }} /><p className="text-xs text-muted-foreground">{t("onboarding.directoryHint")}</p></div></details>
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
        </div>}
        {step === 2 && <div className="space-y-5">{[{ key: "choose", Icon: Layers }, { key: "apply", Icon: MousePointer2 }, { key: "verify", Icon: ShieldCheck }].map(({ key, Icon }, index) => <div key={key} className="flex gap-4"><div className="grid size-10 shrink-0 place-items-center rounded-lg border bg-muted/40"><Icon className="size-5 text-primary" /></div><div><h3 className="text-sm font-semibold">{index + 1}. {t(`onboarding.${key}`)}</h3><p className="mt-1.5 text-sm leading-6 text-muted-foreground">{t(`onboarding.${key}Hint`)}</p></div></div>)}<p className="border-t pt-4 text-xs leading-5 text-muted-foreground">{t("onboarding.reopenHint")}</p></div>}
        {error && <p role="alert" className="mt-4 break-words rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive [overflow-wrap:anywhere]">{error}</p>}
      </div>
      <footer className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-t bg-muted/20 px-6 py-4 sm:px-8">
        <Button variant="ghost" disabled={busy} onClick={() => void finish()}>{t("onboarding.skip")}</Button>
        <div className="flex gap-2">{step > 0 && <Button variant="outline" disabled={busy} onClick={() => { setError(null); setStep((s) => s - 1); }}>{t("onboarding.back")}</Button>}<Button disabled={busy} onClick={() => step === 2 ? void finish() : next()}>{operation === "finish" && <Loader2 className="animate-spin" />}{t(step === 2 ? "onboarding.finish" : step === 1 && !report ? "onboarding.withoutImport" : "onboarding.next")}{!busy && <ArrowRight />}</Button></div>
      </footer>
    </DialogContent>
  </Dialog>;
}
