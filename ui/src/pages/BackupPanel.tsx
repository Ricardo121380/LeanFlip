import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowDownToLine, CheckCircle2, KeyRound, Loader2, LockKeyhole, ShieldCheck } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input, NativeSelect } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api, errorMessage, type BackupPreview, type RestoreOptions, type RestoreReport } from "@/lib/api";
import { TOOLS } from "@/lib/tools";
import { refreshAll } from "@/hooks/queries";

function Failure({ message }: { message: string | null }) {
  return message ? <p role="alert" className="rounded-lg border border-destructive/25 bg-destructive/5 p-3 text-sm text-destructive [overflow-wrap:anywhere]">{message}</p> : null;
}

export function BackupExport({ onBusyChange }: { onBusyChange?: (busy: boolean) => void }) {
  const { t } = useTranslation();
  const [password, setPassword] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const pending = useRef(false);
  const valid = [...password].length >= 10 && [...password].length <= 4096 && password === confirmation;

  async function save() {
    if (pending.current || !valid) return;
    pending.current = true;
    setBusy(true); onBusyChange?.(true); setError(null); setSaved(null);
    try {
      const path = await api.backupSave(password);
      if (path) { setSaved(path); setPassword(""); setConfirmation(""); }
    } catch (problem) { setError(errorMessage(problem)); }
    finally { setPassword(""); setConfirmation(""); pending.current = false; setBusy(false); onBusyChange?.(false); }
  }

  return <section className="grid gap-6 lg:grid-cols-[1fr_1.1fr]">
    <div className="space-y-5">
      <div className="inline-flex size-12 items-center justify-center rounded-xl bg-primary/10 text-primary"><ShieldCheck className="size-6" aria-hidden="true" /></div>
      <div><h2 className="text-lg font-semibold tracking-tight">{t("backup.exportTitle")}</h2><p className="mt-2 text-sm leading-6 text-muted-foreground">{t("backup.exportHint")}</p></div>
      <ul className="space-y-3 text-sm">{["providers", "accounts", "preferences"].map((scope) => <li key={scope} className="flex items-start gap-2"><CheckCircle2 className="mt-0.5 size-4 shrink-0 text-primary" aria-hidden="true" />{t(`backup.scope.${scope}`)}</li>)}</ul>
      <p className="text-xs leading-5 text-muted-foreground">{t("backup.portableHint")}</p>
    </div>
    <form onSubmit={(event) => { event.preventDefault(); void save(); }} className="space-y-4 rounded-xl border bg-card p-5">
      <div className="flex items-center gap-2 text-sm font-semibold"><LockKeyhole className="size-4 text-primary" aria-hidden="true" />{t("backup.protectTitle")}</div>
      <div className="space-y-1.5"><Label htmlFor="backup-password">{t("backup.password")}</Label><Input id="backup-password" type="password" autoComplete="new-password" value={password} disabled={busy} onChange={(event) => setPassword(event.target.value)} aria-describedby="backup-password-hint" /></div>
      <div className="space-y-1.5"><Label htmlFor="backup-confirm">{t("backup.confirmPassword")}</Label><Input id="backup-confirm" type="password" autoComplete="new-password" value={confirmation} disabled={busy} onChange={(event) => setConfirmation(event.target.value)} aria-invalid={confirmation.length > 0 && confirmation !== password} /></div>
      <p id="backup-password-hint" className="text-xs leading-5 text-muted-foreground">{t("backup.passwordHint")}</p>
      {confirmation && password !== confirmation && <p className="text-xs text-destructive">{t("backup.passwordMismatch")}</p>}
      <Failure message={error} />
      <Button className="w-full" type="submit" disabled={!valid || busy}>{busy ? <Loader2 className="animate-spin" /> : <ArrowDownToLine />}{t(busy ? "backup.saving" : "backup.save")}</Button>
      {saved && <div role="status" className="space-y-1 text-sm"><p className="font-medium text-primary">{t("backup.saved")}</p><p className="break-all font-mono text-xs text-muted-foreground">{saved}</p></div>}
    </form>
  </section>;
}

export function BackupRestore({ bytes, fileName, onBack, onBusyChange }: { bytes: Uint8Array; fileName: string; onBack: () => void; onBusyChange?: (busy: boolean) => void }) {
  const mounted = useRef(true);
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const [password, setPassword] = useState("");
  const [preview, setPreview] = useState<BackupPreview | null>(null);
  const [options, setOptions] = useState<RestoreOptions>({ tools: [], accounts: true, settings: true, paths: false, duplicate: "skip" });
  const [report, setReport] = useState<RestoreReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pending = useRef(false);

  async function perform(work: () => Promise<void>) {
    if (pending.current) return;
    pending.current = true; setBusy(true); onBusyChange?.(true); setError(null);
    try { await work(); } catch (problem) { setError(errorMessage(problem)); }
    finally { pending.current = false; setBusy(false); onBusyChange?.(false); }
  }

  async function unlock() {
    await perform(async () => {
      const next = await api.backupLoad(bytes, password);
      if (!mounted.current) { await api.backupDiscard(); return; }
      setPreview(next); setReport(null);
      setOptions({ tools: [...new Set(next.items.map((item) => item.tool))], accounts: next.accounts > 0, settings: next.hasSettings, paths: false, duplicate: "skip" });
    });
    setPassword("");
  }

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; void api.backupDiscard().catch(() => {}); };
  }, []);

  const canRestore = options.tools.length > 0 || options.accounts || options.settings;
  return <section className="space-y-5">
    <header className="flex flex-wrap items-center justify-between gap-3"><div><h2 className="text-lg font-semibold">{t("backup.restoreTitle")}</h2><p className="mt-1 break-all text-sm text-muted-foreground">{fileName}</p></div><Button variant="ghost" disabled={busy} onClick={() => void perform(async () => { await api.backupDiscard(); onBack(); })}>{t("backup.changeFile")}</Button></header>
    {!preview && <form className="max-w-lg space-y-4 rounded-xl border bg-card p-5" onSubmit={(event) => { event.preventDefault(); if (password) void unlock(); }}>
      <div className="flex items-center gap-2 font-medium"><KeyRound className="size-4 text-primary" aria-hidden="true" />{t("backup.unlockTitle")}</div>
      <p className="text-sm leading-6 text-muted-foreground">{t("backup.unlockHint")}</p>
      <Label htmlFor="restore-password">{t("backup.password")}</Label><Input id="restore-password" type="password" autoComplete="off" value={password} disabled={busy} onChange={(event) => setPassword(event.target.value)} />
      <Button type="submit" disabled={busy || !password}>{busy && <Loader2 className="animate-spin" />}{t(busy ? "backup.unlocking" : "backup.unlock")}</Button>
    </form>}
    <Failure message={error} />
    {preview && (!report || report.failures.length > 0) && <>
      <div className="flex flex-wrap gap-x-5 gap-y-2 border-b pb-4 text-sm text-muted-foreground"><span>{t("backup.cardCount", { count: preview.items.length })}</span><span>{t("backup.accountCount", { count: preview.accounts })}</span><span>{new Date(preview.createdAt * 1000).toLocaleString(i18n.language)}</span></div>
      <fieldset disabled={busy} className="space-y-3"><legend className="mb-3 text-sm font-semibold">{t("backup.restoreScopes")}</legend>
        {TOOLS.filter((tool) => preview.items.some((item) => item.tool === tool.id)).map((tool) => <label key={tool.id} className="flex items-center gap-3 rounded-lg border bg-card p-3 text-sm"><Checkbox checked={options.tools.includes(tool.id)} onCheckedChange={(checked) => setOptions((old) => ({ ...old, tools: checked === true ? [...old.tools, tool.id] : old.tools.filter((id) => id !== tool.id) }))} /><span className="flex-1 font-medium">{tool.label}</span><span className="tabular-nums text-muted-foreground">{preview.items.filter((item) => item.tool === tool.id).length}</span></label>)}
        <label className="flex items-center gap-3 py-1 text-sm"><Checkbox checked={options.accounts} disabled={preview.accounts === 0} onCheckedChange={(checked) => setOptions((old) => ({ ...old, accounts: checked === true }))} />{t("backup.restoreAccounts", { count: preview.accounts })}</label>
        <p className="pl-7 text-xs text-muted-foreground">{t("backup.accountMergeHint", { count: preview.existingAccounts })}</p>
        <label className="flex items-center gap-3 py-1 text-sm"><Checkbox checked={options.settings} onCheckedChange={(checked) => setOptions((old) => ({ ...old, settings: checked === true, paths: checked === true && old.paths }))} />{t("backup.restorePreferences")}</label>
        {preview.hasPaths && <label className="flex items-center gap-3 py-1 text-sm"><Checkbox checked={options.paths} disabled={!options.settings} onCheckedChange={(checked) => setOptions((old) => ({ ...old, paths: checked === true }))} />{t("backup.restorePaths")}</label>}
      </fieldset>
      <div className="max-w-md space-y-1.5"><Label htmlFor="restore-duplicate">{t("backup.duplicate")}</Label><NativeSelect id="restore-duplicate" disabled={busy} value={options.duplicate} onChange={(event) => setOptions((old) => ({ ...old, duplicate: event.target.value as "skip" | "rename" }))}><option value="skip">{t("backup.skip")}</option><option value="rename">{t("backup.rename")}</option></NativeSelect></div>
      <p className="text-sm leading-6 text-muted-foreground">{t("backup.restoreHint")}</p>
      <Button disabled={busy || !canRestore} onClick={() => void perform(async () => { setReport(await api.backupApply(options)); void refreshAll(client); })}>{busy && <Loader2 className="animate-spin" />}{t(busy ? "backup.restoring" : "backup.restore")}</Button>
    </>}
    {report && <div role="status" className="space-y-2 rounded-xl border bg-card p-5"><h3 className="font-semibold">{t(report.failures.length ? "backup.partial" : "backup.restored")}</h3><p className="text-sm">{t("backup.report", { cards: report.providersAdded, skipped: report.providersSkipped, accounts: report.accountsAdded })}</p><p className="text-sm text-muted-foreground">{t("backup.noActivation")}</p>{report.failures.length > 0 && <p className="font-mono text-xs text-destructive">{report.failures.join(" · ")}</p>}</div>}
  </section>;
}
