import { useId, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ExternalLink, Loader2, RotateCcw, Search } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useConfirm } from "@/components/ConfirmDialog";
import { Input, NativeSelect } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { CodeEditor } from "@/components/CodeEditor";
import { ModelRoutesEditor } from "@/components/ModelRoutesEditor";
import { applyRoutes, routeErrors, routesOf } from "@/lib/routes";
import { api, errorMessage, type AccountInfo, type LoginSummary, type Provider, type ToolId } from "@/lib/api";
import {
  accountLabel,
  fromEditorText,
  hasRedacted,
  isOfficial,
  slugify,
  toEditorText,
  uniqueId,
  type EditorText,
} from "@/lib/cards";
import { filterPresets, loadPresets, type Preset } from "@/lib/presets";
import { toolMeta } from "@/lib/tools";
import { cn } from "@/lib/utils";
import { showWarnings, refreshAll } from "@/hooks/queries";
import { useQuery, useQueryClient } from "@tanstack/react-query";

const CODEX_TEMPLATE = (model: string) =>
  `model_provider = "custom"\nmodel = "${model}"\n\n[model_providers.custom]\nname = "custom"\nbase_url = ""\nwire_api = "responses"\nrequires_openai_auth = true\n`;

/** Grok Build 自己的格式：[models].default 指向一张 [model.<name>] 表。 */
const GROK_TEMPLATE =
  '[models]\ndefault = "grok-4.5"\n\n[model."grok-4.5"]\nmodel = "grok-4.5"\nbase_url = ""\nname = "custom"\napi_key = ""\napi_backend = "responses"\ncontext_window = 500000\n';

export function customTemplate(tool: ToolId): Record<string, unknown> {
  switch (tool) {
    case "claude":
    case "claude-desktop":
      return { env: { ANTHROPIC_BASE_URL: "", ANTHROPIC_AUTH_TOKEN: "" } };
    case "gemini":
      return { env: { GOOGLE_GEMINI_BASE_URL: "", GEMINI_API_KEY: "", GEMINI_MODEL: "" } };
    case "codex":
      return { auth: { OPENAI_API_KEY: "" }, config: CODEX_TEMPLATE("gpt-5.5") };
    case "grokbuild":
      return { config: GROK_TEMPLATE };
    case "opencode":
      return { npm: "@ai-sdk/openai-compatible", name: "", options: { baseURL: "", apiKey: "" }, models: {} };
    case "openclaw":
      return { baseUrl: "", apiKey: "", api: "openai-completions", models: [] };
    case "hermes":
      return { name: "", base_url: "", api_key: "", api_mode: "chat_completions", models: [] };
    case "pi":
      return { name: "", baseUrl: "", api: "openai-completions", apiKey: "", models: [] };
    case "mcode":
      return { name: "", kind: "custom", enabled: true, api: "openai-completions", options: { baseURL: "", apiKey: "" }, models: {} };
  }
}

/** Codex 官方卡的登录方式：native | ref:<identity> | managed: | managed:<id> */
export function loginSourceOf(provider?: Provider): string {
  const binding = provider?.meta?.authBinding;
  if (binding?.authProvider === "codex_oauth") return `managed:${binding.accountId ?? ""}`;
  const ref = provider?.meta?.codexLoginRef;
  if (typeof ref === "string" && ref) return `ref:${ref}`;
  return "native";
}

export function applyLoginSource(provider: Provider, source: string): Provider {
  const meta = { ...(provider.meta ?? {}) };
  delete meta.authBinding;
  delete meta.codexLoginRef;
  if (source.startsWith("managed:")) {
    const accountId = source.slice("managed:".length);
    meta.authBinding = accountId ? { authProvider: "codex_oauth", accountId } : { authProvider: "codex_oauth" };
  }
  const next: Provider = { ...provider, meta };
  if (Object.keys(meta).length === 0) delete next.meta;
  return next;
}

interface Props {
  tool: ToolId;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** 有值 = 编辑。 */
  provider?: Provider;
  existingIds: string[];
  accounts?: AccountInfo[];
  logins?: LoginSummary[];
}

export function ProviderEditor({ tool, open, onOpenChange, provider, existingIds, accounts = [], logins = [] }: Props) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const descriptionId = useId();
  const meta = toolMeta(tool);
  const editing = Boolean(provider);
  const [query, setQuery] = useState("");
  const [preset, setPreset] = useState<Preset | null>(null);
  const [name, setName] = useState(provider?.name ?? "");
  const [id, setId] = useState(provider?.id ?? "");
  const [idTouched, setIdTouched] = useState(false);
  const [website, setWebsite] = useState((provider?.websiteUrl as string | undefined) ?? "");
  const [category, setCategory] = useState<string | undefined>(provider?.category);
  const [text, setText] = useState<EditorText>(() =>
    toEditorText(meta.editor, provider?.settingsConfig ?? customTemplate(tool)),
  );
  const [loginSource, setLoginSource] = useState(loginSourceOf(provider));
  const [routes, setRoutes] = useState(() => routesOf(provider));
  const [apply, setApply] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [confirm, confirmDialog] = useConfirm();
  const signature = JSON.stringify({ name, id, website, category, text, loginSource, routes, apply });
  const [initialSignature] = useState(signature);
  const [presetSignature, setPresetSignature] = useState(signature);
  const dirty = signature !== initialSignature;

  const presetList = useQuery({
    queryKey: ["presets", tool],
    queryFn: () => loadPresets(tool),
    enabled: !editing,
    staleTime: Infinity,
  });
  const presets = useMemo(() => filterPresets(presetList.data ?? [], query), [presetList.data, query]);
  const parsed = fromEditorText(meta.editor, text, provider?.settingsConfig);
  const official = isOfficial({ category });
  const showLogin = tool === "codex" && official;
  const showRoutes = tool === "claude-desktop" && !official;
  const routesOk = !showRoutes || routeErrors(routes).every((e) => e === null);

  async function choosePreset(p: Preset | null) {
    if (saving || preset?.name === p?.name) return;
    if (signature !== presetSignature && !(await confirm({ title: t("editor.replaceTitle"), description: t("editor.replaceHint"), confirmLabel: t("editor.replaceAction") }))) return;
    setPreset(p);
    const nextName = p?.name ?? "";
    const nextId = idTouched ? id : uniqueId(slugify(nextName || "custom"), existingIds);
    const nextWebsite = p?.websiteUrl ?? "";
    const nextCategory = p?.isOfficial ? "official" : p?.category;
    const nextText = toEditorText(meta.editor, p?.settingsConfig ?? customTemplate(tool));
    setName(nextName);
    setId(nextId);
    setWebsite(nextWebsite);
    setCategory(nextCategory);
    setText(nextText);
    setSaveError(null);
    setPresetSignature(JSON.stringify({ name: nextName, id: nextId, website: nextWebsite, category: nextCategory, text: nextText, loginSource, routes, apply }));
  }

  async function requestClose() {
    if (saving) return;
    if (!dirty || await confirm({ title: t("editor.discardTitle"), description: t("editor.discardHint"), confirmLabel: t("editor.discardAction") })) onOpenChange(false);
  }

  async function save() {
    if (!canSave || !parsed.ok) return;
    setSaving(true);
    setSaveError(null);
    try {
      let next: Provider = editing
        ? { ...provider!, name: name.trim() || provider!.name, settingsConfig: parsed.value }
        : {
            id: id.trim(),
            name: name.trim() || id.trim(),
            settingsConfig: parsed.value,
            createdAt: Date.now(),
            ...(category ? { category } : {}),
          };
      if (website.trim()) next.websiteUrl = website.trim();
      else delete next.websiteUrl;
      if (showRoutes) next = applyRoutes(next, routes);
      if (showLogin) next = applyLoginSource(next, loginSource.startsWith("ref:") ? "native" : loginSource);

      const result = editing
        ? await api.updateProvider(tool, next)
        : await api.addProvider(tool, next, apply);
      showWarnings(result);
      if (showLogin && loginSource.startsWith("ref:")) {
        showWarnings(await api.codexSetLoginRef(next.id, loginSource.slice(4)));
      }
      toast.success(t("editor.saved"));
      onOpenChange(false);
    } catch (error) {
      setSaveError(errorMessage(error));
    } finally {
      setSaving(false);
      void refreshAll(client);
    }
  }

  const idTaken = !editing && existingIds.includes(id.trim());
  const canSave = parsed.ok && routesOk && (editing ? dirty : (id.trim() !== "" && !idTaken)) && !saving;

  return (
    <>
    <Dialog open={open} onOpenChange={(next) => { if (!next) void requestClose(); }}>
      <DialogContent aria-describedby={descriptionId} closeDisabled={saving} className="flex max-h-[calc(100dvh-2rem)] w-[calc(100vw-2rem)] max-w-4xl flex-col gap-0 overflow-hidden p-0">
        <DialogHeader className="shrink-0 border-b px-5 py-4 pr-12">
          <DialogTitle>
            {editing ? t("editor.titleEdit") : t("editor.titleAdd")} · {meta.label}
          </DialogTitle>
          <DialogDescription id={descriptionId}>{t(editing ? "editor.editHint" : "editor.addHint")}</DialogDescription>
        </DialogHeader>

        <div className="min-h-0 flex-1 overflow-y-auto p-5">
        <fieldset disabled={saving} className={cn("grid min-w-0 gap-5", !editing && "md:grid-cols-[200px_minmax(0,1fr)]")}>
          {!editing && (
            <div className="flex min-h-0 flex-col gap-2">
              <Label htmlFor="pe-preset-search">{t("editor.preset")}</Label>
              <div className="relative">
                <Search className="pointer-events-none absolute left-2.5 top-2.5 size-4 text-muted-foreground" aria-hidden="true" />
                <Input id="pe-preset-search" type="search" className="pl-8" placeholder={t("editor.searchPreset")} value={query} onChange={(e) => setQuery(e.target.value)} />
              </div>
              <div className="flex max-h-52 min-h-0 flex-col gap-0.5 overflow-y-auto rounded-md border p-1 md:max-h-96" role="group" aria-label={t("editor.preset")}>
                <button
                  type="button"
                  aria-pressed={preset === null}
                  className={cn("flex min-h-9 items-center justify-between gap-2 rounded px-2 py-1.5 text-left text-sm hover:bg-accent", preset === null && "bg-accent font-medium")}
                  onClick={() => void choosePreset(null)}
                >
                  {t("editor.custom")}
                  {preset === null && <Check className="size-4 shrink-0" aria-hidden="true" />}
                </button>
                {presetList.isLoading && <p role="status" className="px-2 py-1.5 text-xs text-muted-foreground">{t("common.loading")}</p>}
                {presetList.isError && <div role="alert" className="space-y-2 p-2 text-xs text-destructive"><p>{t("editor.presetsError")}</p><Button type="button" size="sm" variant="outline" onClick={() => void presetList.refetch()}><RotateCcw />{t("common.retry")}</Button></div>}
                {!presetList.isLoading && !presetList.isError && query.trim() && presets.length === 0 && <p role="status" className="px-2 py-3 text-xs leading-relaxed text-muted-foreground">{t("editor.noPresets")}</p>}
                {presets.map((p) => (
                  <button
                    type="button"
                    key={p.name}
                    aria-pressed={preset?.name === p.name}
                    className={cn(
                      "flex min-h-9 items-center justify-between gap-2 rounded px-2 py-1.5 text-left text-sm hover:bg-accent",
                      preset?.name === p.name && "bg-accent font-medium",
                    )}
                    onClick={() => void choosePreset(p)}
                  >
                    <span className="min-w-0 break-words">{p.name}</span>
                    {preset?.name === p.name && <Check className="size-4 shrink-0" aria-hidden="true" />}
                  </button>
                ))}
              </div>
            </div>
          )}

          <div className="flex min-w-0 flex-col gap-3">
            <div className="grid grid-cols-2 gap-3">
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="pe-name">{t("editor.name")}</Label>
                <Input id="pe-name" value={name} onChange={(e) => {
                  const next = e.target.value;
                  setName(next);
                  if (!editing && !idTouched) setId(next.trim() ? uniqueId(slugify(next), existingIds) : "");
                }} />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="pe-id">{t("editor.id")}</Label>
                <Input
                  id="pe-id"
                  value={id}
                  disabled={editing}
                  aria-invalid={idTaken}
                  aria-describedby={idTaken ? "pe-id-error" : undefined}
                  className={cn(idTaken && "border-destructive")}
                  onChange={(e) => {
                    setIdTouched(true);
                    setId(e.target.value);
                  }}
                />
              </div>
            </div>
            {idTaken && <p id="pe-id-error" role="alert" className="text-xs text-destructive">{t("editor.idTaken")}</p>}
            {!editing && meta.kind === "coexist" && <p className="-mt-1 text-xs text-muted-foreground">{t("editor.idHint")}</p>}
            <div className="flex items-end gap-2">
              <div className="flex flex-1 flex-col gap-1.5">
                <Label htmlFor="pe-web">{t("editor.website")}</Label>
                <Input id="pe-web" value={website} onChange={(e) => setWebsite(e.target.value)} />
              </div>
              {preset?.apiKeyUrl && (
                <Button variant="outline" size="sm" className="mb-0.5" onClick={async () => { try { await api.openUrl(preset.apiKeyUrl!); } catch { toast.error(t("editor.openError")); } }}>
                  <ExternalLink /> {t("editor.getKey")}
                </Button>
              )}
            </div>

            {showLogin && (
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="pe-login">{t("editor.loginSource")}</Label>
                <NativeSelect id="pe-login" value={loginSource} onChange={(e) => setLoginSource(e.target.value)}>
                  <option value="native">{t("editor.loginNative")}</option>
                  {logins.map((l) => (
                    <option key={l.identity} value={`ref:${l.identity}`}>
                      {t("editor.loginRef", { label: [l.email, l.workspace].filter(Boolean).join(" · ") || l.identity })}
                    </option>
                  ))}
                  <option value="managed:">{t("editor.loginManagedDefault")}</option>
                  {accounts.map((a) => (
                    <option key={a.id} value={`managed:${a.id}`}>
                      {t("editor.loginManaged", { label: `${accountLabel(a)}${a.plan ? ` (${a.plan})` : ""}` })}
                    </option>
                  ))}
                </NativeSelect>
              </div>
            )}

            {editing && hasRedacted(text) && (
              <p className="rounded-md bg-amber-50 px-3 py-2 text-xs text-amber-800">{t("editor.redactedNote")}</p>
            )}

            {meta.editor === "codex" ? (
              <>
                <Label>{t("editor.auth")}</Label>
                <CodeEditor readOnly={saving} language="json" minHeight={90} value={text.json} onChange={(json) => setText((s) => ({ ...s, json }))} ariaLabel={t("editor.auth")} />
                <Label>{t("editor.config")}</Label>
                <CodeEditor readOnly={saving} language="toml" value={text.toml ?? ""} onChange={(toml) => setText((s) => ({ ...s, toml }))} ariaLabel={t("editor.config")} />
              </>
            ) : (
              <>
                <Label>{t("editor.settings")}</Label>
                <CodeEditor readOnly={saving} language="json" minHeight={220} value={text.json} onChange={(json) => setText({ json })} ariaLabel={t("editor.settings")} />
              </>
            )}
            {showRoutes && <ModelRoutesEditor rows={routes} onChange={setRoutes} />}
            {!parsed.ok && (
              <p role="alert" className="text-xs text-destructive">
                {t("editor.invalid")}: {parsed.error}
              </p>
            )}

            {!editing && (
              <label className="flex items-center gap-2 text-sm">
                <Checkbox checked={apply} onCheckedChange={(v) => setApply(v === true)} />
                {meta.kind === "coexist" ? t("editor.addToLive") : t("editor.switchAfterAdd")}
              </label>
            )}
          </div>
        </fieldset>
        </div>

        <DialogFooter className="shrink-0 flex-wrap items-center border-t bg-background px-5 py-4">
          <div className="mr-auto min-w-0 flex-1 basis-40 text-xs text-muted-foreground">
            {saveError ? <p role="alert" className="break-words text-destructive">{t("editor.saveError")} {saveError}</p> : <p role="status">{t(saving ? "editor.saving" : editing && !dirty ? "editor.unchanged" : "editor.saveHint")}</p>}
          </div>
          <Button variant="outline" disabled={saving} onClick={() => void requestClose()}>
            {t("editor.cancel")}
          </Button>
          <Button onClick={() => void save()} disabled={!canSave}>
            {saving && <Loader2 className="animate-spin" />}{t(saving ? "editor.saving" : "editor.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
    {confirmDialog}
    </>
  );
}
