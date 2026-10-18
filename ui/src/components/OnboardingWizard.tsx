import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowRight, Check, Layers, Loader2, MousePointer2, ShieldCheck } from "lucide-react";
import { api, errorMessage, type ProviderImportSource } from "@/lib/api";
import { refreshAll } from "@/hooks/queries";
import { cn } from "@/lib/utils";
import { ProviderImportPanel, ProviderImportSourcePicker } from "@/components/ProviderImportPanel";
import { BrandMark } from "@/components/BrandMark";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";

export function OnboardingWizard({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const [step, setStep] = useState(0);
  const [source, setSource] = useState<ProviderImportSource>("cc-switch");
  const [imported, setImported] = useState(false);
  const [importBusy, setImportBusy] = useState(false);
  const [operation, setOperation] = useState<"finish" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pending = useRef(false);
  const title = useRef<HTMLHeadingElement>(null);
  const returnFocus = useRef(document.activeElement instanceof HTMLElement ? document.activeElement : null);
  const busy = importBusy || operation !== null;

  useEffect(() => { title.current?.focus(); }, [step]);

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
        <div hidden={step !== 1} className="space-y-5">
          <ProviderImportSourcePicker value={source} disabled={busy} onChange={(next) => { setSource(next); setImported(false); }} />
          <ProviderImportPanel key={source} source={source} disabled={operation !== null} onImportedChange={setImported} onBusyChange={(value) => { pending.current = value; setImportBusy(value); }} />
        </div>
        {step === 2 && <div className="space-y-5">{[{ key: "choose", Icon: Layers }, { key: "apply", Icon: MousePointer2 }, { key: "verify", Icon: ShieldCheck }].map(({ key, Icon }, index) => <div key={key} className="flex gap-4"><div className="grid size-10 shrink-0 place-items-center rounded-lg border bg-muted/40"><Icon className="size-5 text-primary" /></div><div><h3 className="text-sm font-semibold">{index + 1}. {t(`onboarding.${key}`)}</h3><p className="mt-1.5 text-sm leading-6 text-muted-foreground">{t(`onboarding.${key}Hint`)}</p></div></div>)}<p className="border-t pt-4 text-xs leading-5 text-muted-foreground">{t("onboarding.reopenHint")}</p></div>}
        {error && <p role="alert" className="mt-4 break-words rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive [overflow-wrap:anywhere]">{error}</p>}
      </div>
      <footer className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-t bg-muted/20 px-6 py-4 sm:px-8">
        <Button variant="ghost" disabled={busy} onClick={() => void finish()}>{t("onboarding.skip")}</Button>
        <div className="flex gap-2">{step > 0 && <Button variant="outline" disabled={busy} onClick={() => { setError(null); setStep((s) => s - 1); }}>{t("onboarding.back")}</Button>}<Button disabled={busy} onClick={() => step === 2 ? void finish() : next()}>{operation === "finish" && <Loader2 className="animate-spin" />}{t(step === 2 ? "onboarding.finish" : step === 1 && !imported ? "onboarding.withoutImport" : "onboarding.next")}{!busy && <ArrowRight />}</Button></div>
      </footer>
    </DialogContent>
  </Dialog>;
}
