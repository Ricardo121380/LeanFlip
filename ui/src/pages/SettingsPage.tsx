import { useEffect, useState } from "react";
import { Loader2, RotateCcw, Save } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input, NativeSelect } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { QueryState } from "@/components/QueryState";
import { useAction, useSettings } from "@/hooks/queries";
import { api, errorMessage, type AppSettings, type DirOverrides, type SettingsView } from "@/lib/api";
import { applyLanguage } from "@/i18n";

const DIR_FIELDS: { key: keyof DirOverrides; label: string; tool: string }[] = [
  { key: "claudeDir", label: "Claude Code (~/.claude)", tool: "claude" },
  { key: "codexDir", label: "Codex (~/.codex)", tool: "codex" },
  { key: "geminiDir", label: "Gemini CLI (~/.gemini)", tool: "gemini" },
  { key: "grokDir", label: "Grok Build (~/.grok)", tool: "grokbuild" },
  { key: "claudeDesktopRoot", label: "Claude Desktop", tool: "claude-desktop" },
  { key: "opencodeDir", label: "OpenCode", tool: "opencode" },
  { key: "openclawDir", label: "OpenClaw", tool: "openclaw" },
  { key: "hermesDir", label: "Hermes", tool: "hermes" },
  { key: "piAgentDir", label: "Pi", tool: "pi" },
  { key: "mcodeDir", label: "MCode", tool: "mcode" },
];

/** 空字符串 → 不覆盖。 */
export function cleanOverrides(overrides: DirOverrides): DirOverrides {
  const out: DirOverrides = {};
  for (const [k, v] of Object.entries(overrides)) {
    const value = typeof v === "string" ? v.trim() : "";
    if (value) out[k as keyof DirOverrides] = value;
  }
  return out;
}

function Row({ id, label, hint, children }: { id: string; label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-4 py-3 sm:gap-6">
      <div className="min-w-0">
        <Label htmlFor={id}>{label}</Label>
        {hint && <p id={`${id}-hint`} className="mt-1 text-xs leading-relaxed text-muted-foreground">{hint}</p>}
      </div>
      {children}
    </div>
  );
}

type SettingsDraft = { app: AppSettings; keep: boolean; refresh: string };
type SettingsForm = { draft: SettingsDraft; baseline: SettingsDraft };
export type SettingsLeaveState = { dirty: boolean; saving: boolean };

function toDraft(data: SettingsView): SettingsDraft {
  return { app: data.app, keep: data.keepCodexLogin, refresh: String(data.app.backgroundRefreshHours) };
}

function sameDraft(a: SettingsDraft, b: SettingsDraft) {
  return JSON.stringify(a) === JSON.stringify(b);
}

export function SettingsPage({ onLeaveStateChange }: { onLeaveStateChange?: (state: SettingsLeaveState) => void } = {}) {
  const { t } = useTranslation();
  const { data, isError, error, refetch } = useSettings();
  const [form, setForm] = useState<SettingsForm | null>(null);

  const save = useAction(
    (submitted: SettingsDraft) => api.saveSettings(submitted.app, submitted.keep),
    (result, submitted) => {
      setForm({ draft: submitted, baseline: submitted });
      applyLanguage(submitted.app.language);
      toast.success(t("settings.saved"));
      if (result.dirsChanged) toast.info(t("settings.dirsApplied"));
    },
  );
  const dirty = form !== null && !sameDraft(form.draft, form.baseline);

  useEffect(() => {
    onLeaveStateChange?.({ dirty, saving: save.isPending });
  }, [dirty, save.isPending, onLeaveStateChange]);

  useEffect(() => () => onLeaveStateChange?.({ dirty: false, saving: false }), [onLeaveStateChange]);

  useEffect(() => {
    if (!data) return;
    setForm((current) => {
      // Backend events can refresh settings while the user is editing.
      if (current && !sameDraft(current.draft, current.baseline)) return current;
      const next = toDraft(data);
      return { draft: next, baseline: next };
    });
  }, [data]);

  if (!data || !form) return (
    <div className="p-6">
      <QueryState
        state={isError ? "error" : "loading"}
        title={t(isError ? "settings.loadError" : "settings.loading")}
        description={isError ? errorMessage(error) : undefined}
        onRetry={isError ? () => void refetch() : undefined}
      />
    </div>
  );
  const { draft } = form;
  const { app, keep } = draft;
  const refreshHours = Number(draft.refresh);
  const refreshValid = draft.refresh.trim() !== "" && Number.isInteger(refreshHours) && refreshHours >= 0 && refreshHours <= 4294967295;
  const setDraft = (next: SettingsDraft) => setForm({ ...form, draft: next });
  const set = (patch: Partial<AppSettings>) => setDraft({ ...draft, app: { ...app, ...patch } });
  const reset = () => {
    const next = toDraft(data);
    setForm({ draft: next, baseline: next });
  };
  const submit = () => {
    if (!dirty || !refreshValid || save.isPending) return;
    save.mutate({ ...draft, app: { ...app, backgroundRefreshHours: refreshHours, dirOverrides: cleanOverrides(app.dirOverrides) }, refresh: String(refreshHours) });
  };

  return (
    <form className="flex max-w-3xl flex-col gap-6 p-6" onSubmit={(event) => { event.preventDefault(); submit(); }}>
      <h1 className="text-xl font-semibold">{t("settings.title")}</h1>

      {isError && <QueryState state="error" title={t("settings.refreshError")} description={errorMessage(error)} onRetry={() => void refetch()} />}

      <fieldset disabled={save.isPending} className="flex min-w-0 flex-col gap-6">
        <section aria-labelledby="settings-general" className="flex flex-col gap-2">
          <h2 id="settings-general" className="font-medium">{t("settings.general")}</h2>
          <Card className="divide-y px-4">
            <Row id="settings-language" label={t("settings.language")}>
              <NativeSelect
                id="settings-language"
                aria-label={t("settings.language")}
                className="w-36 shrink-0"
                value={app.language ?? ""}
                onChange={(e) => set({ language: (e.target.value || null) as AppSettings["language"] })}
              >
                <option value="">{t("settings.languageSystem")}</option>
                <option value="zh">中文</option>
                <option value="en">English</option>
              </NativeSelect>
            </Row>
            <Row id="settings-autostart" label={t("settings.autostart")}>
              <Switch id="settings-autostart" aria-label={t("settings.autostart")} checked={app.autostart} onCheckedChange={(v) => set({ autostart: v })} />
            </Row>
            <Row id="settings-close-to-tray" label={t("settings.closeToTray")}>
              <Switch id="settings-close-to-tray" aria-label={t("settings.closeToTray")} checked={app.closeToTray} onCheckedChange={(v) => set({ closeToTray: v })} />
            </Row>
            <Row id="settings-keep-login" label={t("settings.keepCodexLogin")} hint={t("settings.keepCodexLoginHint")}>
              <Switch id="settings-keep-login" aria-describedby="settings-keep-login-hint" aria-label={t("settings.keepCodexLogin")} checked={keep} onCheckedChange={(keep) => setDraft({ ...draft, keep })} />
            </Row>
            <Row id="settings-refresh" label={t("settings.refresh")} hint={t("settings.refreshHint")}>
              <Input
                id="settings-refresh"
                type="number"
                min={0}
                max={4294967295}
                step={1}
                required
                className="w-24 shrink-0 tabular-nums"
                aria-label={t("settings.refresh")}
                aria-invalid={!refreshValid}
                aria-describedby={`settings-refresh-hint${refreshValid ? "" : " settings-refresh-error"}`}
                value={draft.refresh}
                onChange={(e) => setDraft({ ...draft, refresh: e.target.value })}
              />
            </Row>
            {!refreshValid && <p id="settings-refresh-error" role="alert" className="py-3 text-sm text-destructive">{t("settings.refreshInvalid")}</p>}
          </Card>
        </section>

        <section aria-labelledby="settings-dirs" className="flex flex-col gap-2">
          <h2 id="settings-dirs" className="font-medium">{t("settings.dirs")}</h2>
          <p id="settings-dirs-hint" className="text-xs leading-relaxed text-muted-foreground">{t("settings.dirsHint")}</p>
          <Card className="grid gap-3 p-4">
            {DIR_FIELDS.map((f) => (
              <div key={f.key} className="grid min-w-0 gap-2 sm:grid-cols-[180px_minmax(0,1fr)] sm:items-center sm:gap-3">
                <Label htmlFor={`dir-${f.key}`}>{f.label}</Label>
                <Input
                  id={`dir-${f.key}`}
                  aria-describedby="settings-dirs-hint"
                  placeholder={data.dirs?.[f.key] ?? data.livePaths[f.tool]?.[0]}
                  value={app.dirOverrides[f.key] ?? ""}
                  onChange={(e) => set({ dirOverrides: { ...app.dirOverrides, [f.key]: e.target.value } })}
                />
              </div>
            ))}
          </Card>
          <p className="break-all font-mono text-xs text-muted-foreground">{t("settings.dataDir", { path: data.dataDir })}</p>
        </section>
      </fieldset>

      <div className="sticky bottom-0 flex flex-wrap items-center justify-between gap-3 border-t bg-background py-4">
        <p role="status" className="text-sm text-muted-foreground">{t(save.isPending ? "settings.saving" : dirty ? "settings.unsaved" : "settings.unchanged")}</p>
        <div className="flex items-center gap-2">
          <Button type="button" variant="outline" onClick={reset} disabled={!dirty || save.isPending}><RotateCcw /> {t("settings.reset")}</Button>
          <Button type="submit" disabled={!dirty || !refreshValid || save.isPending}>
            {save.isPending ? <Loader2 className="animate-spin" /> : <Save />}
            {t(save.isPending ? "settings.saving" : "settings.save")}
          </Button>
        </div>
      </div>
    </form>
  );
}
