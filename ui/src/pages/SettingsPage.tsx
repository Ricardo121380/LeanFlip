import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input, NativeSelect } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { useAction, useSettings } from "@/hooks/queries";
import { api, type AppSettings, type DirOverrides } from "@/lib/api";
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

function Row({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-6 py-3">
      <div>
        <p className="text-sm font-medium">{label}</p>
        {hint && <p className="mt-0.5 text-xs text-muted-foreground">{hint}</p>}
      </div>
      {children}
    </div>
  );
}

export function SettingsPage() {
  const { t } = useTranslation();
  const { data } = useSettings();
  const [app, setApp] = useState<AppSettings | null>(null);
  const [keep, setKeep] = useState(false);

  useEffect(() => {
    if (data) {
      setApp(data.app);
      setKeep(data.keepCodexLogin);
    }
  }, [data]);

  const save = useAction(
    () => api.saveSettings({ ...app!, dirOverrides: cleanOverrides(app!.dirOverrides) }, keep),
    (result) => {
      applyLanguage(app!.language);
      toast.success(t("settings.saved"));
      if (result.restartRequired) toast.info(t("settings.restart"));
    },
  );

  if (!data || !app) return <p className="p-8 text-sm text-muted-foreground">{t("common.loading")}</p>;
  const set = (patch: Partial<AppSettings>) => setApp({ ...app, ...patch });

  return (
    <div className="flex max-w-3xl flex-col gap-5 p-6">
      <h1 className="text-xl font-semibold">{t("settings.title")}</h1>

      <Card className="divide-y px-4">
        <Row label={t("settings.language")}>
          <NativeSelect
            aria-label={t("settings.language")}
            className="w-40"
            value={app.language ?? ""}
            onChange={(e) => set({ language: (e.target.value || null) as AppSettings["language"] })}
          >
            <option value="">{t("settings.languageSystem")}</option>
            <option value="zh">中文</option>
            <option value="en">English</option>
          </NativeSelect>
        </Row>
        <Row label={t("settings.autostart")}>
          <Switch aria-label={t("settings.autostart")} checked={app.autostart} onCheckedChange={(v) => set({ autostart: v })} />
        </Row>
        <Row label={t("settings.closeToTray")}>
          <Switch aria-label={t("settings.closeToTray")} checked={app.closeToTray} onCheckedChange={(v) => set({ closeToTray: v })} />
        </Row>
        <Row label={t("settings.keepCodexLogin")} hint={t("settings.keepCodexLoginHint")}>
          <Switch aria-label={t("settings.keepCodexLogin")} checked={keep} onCheckedChange={setKeep} />
        </Row>
        <Row label={t("settings.refresh")} hint={t("settings.refreshHint")}>
          <Input
            type="number"
            min={0}
            className="w-24"
            aria-label={t("settings.refresh")}
            value={app.backgroundRefreshHours}
            onChange={(e) => set({ backgroundRefreshHours: Math.max(0, Number(e.target.value) || 0) })}
          />
        </Row>
      </Card>

      <section className="flex flex-col gap-2">
        <h2 className="font-medium">{t("settings.dirs")}</h2>
        <p className="text-xs text-muted-foreground">{t("settings.dirsHint")}</p>
        <Card className="grid gap-3 p-4">
          {DIR_FIELDS.map((f) => (
            <div key={f.key} className="grid grid-cols-[180px_1fr] items-center gap-3">
              <Label htmlFor={`dir-${f.key}`}>{f.label}</Label>
              <Input
                id={`dir-${f.key}`}
                placeholder={data.livePaths[f.tool]?.[0]}
                value={app.dirOverrides[f.key] ?? ""}
                onChange={(e) => set({ dirOverrides: { ...app.dirOverrides, [f.key]: e.target.value } })}
              />
            </div>
          ))}
        </Card>
        <p className="font-mono text-xs text-muted-foreground">{t("settings.dataDir", { path: data.dataDir })}</p>
      </section>

      <div>
        <Button onClick={() => save.mutate(undefined)} disabled={save.isPending}>
          {t("settings.save")}
        </Button>
      </div>
    </div>
  );
}
