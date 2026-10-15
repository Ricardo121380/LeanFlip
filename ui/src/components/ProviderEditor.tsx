import { CommonConfigEditor } from "./CommonConfigEditor";
import { ConfigOptions } from "./ConfigOptions";
import { providerOptions, applyOption, setPath, pathParts, type ConfigPath } from "@/lib/config-options";
import { providerFieldsFor, configFieldsIssue } from "@/lib/config-fields";
import { ConfigCompatibility } from "./ConfigCompatibility";
import { ConfigFields, useFieldError } from "./ConfigFields";
import { readToml, setTomlValue } from "@/lib/toml-edit";
import { useEditorNavigation } from "@/lib/navigation";
import { useEffect, useId, useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import * as Tabs from "@radix-ui/react-tabs";
import { ArrowLeft, Check, ExternalLink, ListRestart, Loader2, Plus, RotateCcw, Search } from "lucide-react";
import { toast } from "sonner";
import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input, NativeSelect } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { CodeEditor } from "@/components/CodeEditor";
import { useConfirm } from "@/components/ConfirmDialog";
import { ConnectionFields, type EffortState } from "@/components/ConnectionFields";
import { Monogram, ToolGlyph } from "@/components/Glyph";
import { ModelRoutesEditor } from "@/components/ModelRoutesEditor";
import { ConnectionCheck, ProbeLine, type ProbeSlot } from "@/components/ProviderProbe";
import { applyRoutes, routeErrors, routesOf } from "@/lib/routes";
import { loginChoices, loginHelp } from "@/lib/login-choices";
import { api, errorMessage, type AccountInfo, type LoginSummary, type ProbeResult, type Provider, type ToolId } from "@/lib/api";
import {
  endpointOf,
  fromEditorText,
  hasRedacted,
  isOfficial,
  REDACTED,
  slugify,
  toEditorText,
  uniqueId,
  type EditorText,
} from "@/lib/cards";
import { readConnection, writeConnection, type FieldSpec } from "@/lib/connection";
import { settle } from "@/lib/probe";
import { filterPresets, loadPresets, type Preset } from "@/lib/presets";
import { toolMeta } from "@/lib/tools";
import { cn } from "@/lib/utils";
import { showWarnings, refreshAll } from "@/hooks/queries";

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
  if (source.startsWith("ref:")) meta.codexLoginRef = source.slice(4);
  const next: Provider = { ...provider, meta };
  if (Object.keys(meta).length === 0) delete next.meta;
  return next;
}


/** 复制时去掉被隐藏的登录凭据：它们只能在原供应商上还原。 */
function withoutRedacted(settingsConfig: Record<string, unknown>): Record<string, unknown> {
  const auth = settingsConfig.auth;
  if (!auth || typeof auth !== "object" || Array.isArray(auth)) return settingsConfig;
  const kept = Object.fromEntries(Object.entries(auth).filter(([, value]) => value !== REDACTED));
  return { ...settingsConfig, auth: kept };
}

function useDebounced<T>(value: T, delay: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), delay);
    return () => clearTimeout(timer);
  }, [value, delay]);
  return debounced;
}

/** 预设卡片上的地址：优先请求地址，没有就用官网。 */
function presetHost(tool: ToolId, preset: Preset): string | undefined {
  const url = endpointOf(tool, { id: "", name: preset.name, settingsConfig: preset.settingsConfig }) ?? preset.websiteUrl;
  if (!url) return undefined;
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

function Section({ title, hint, aside, children }: { title: string; hint?: ReactNode; aside?: ReactNode; children?: ReactNode }) {
  return (
    <section className="flex min-w-0 flex-col gap-3.5">
      <div className="flex min-h-8 flex-wrap items-center justify-between gap-x-3 gap-y-1">
        <div className="min-w-0">
          <h3 className="text-sm font-semibold tracking-tight">{title}</h3>
          {hint && <p className="mt-0.5 text-xs leading-5 text-muted-foreground">{hint}</p>}
        </div>
        {aside}
      </div>
      {children}
    </section>
  );
}

function Note({ tone = "muted", children }: { tone?: "muted" | "warning"; children: ReactNode }) {
  return (
    <div className={cn("rounded-md border px-3 py-2 text-xs leading-5", tone === "warning" ? "border-warning/30 bg-warning/[0.07] text-foreground" : "bg-muted/50 text-muted-foreground")}>
      {children}
    </div>
  );
}

const TAB = "relative inline-flex h-11 items-center gap-1.5 px-1 text-sm font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring data-[state=active]:text-foreground data-[state=active]:after:absolute data-[state=active]:after:inset-x-0 data-[state=active]:after:-bottom-px data-[state=active]:after:h-0.5 data-[state=active]:after:rounded-full data-[state=active]:after:bg-primary";

interface Props {
  tool: ToolId;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** 有值 = 编辑。 */
  provider?: Provider;
  /** 有值 = 以它为模板新建（复制）。 */
  template?: Provider;
  existingIds: string[];
  accounts?: AccountInfo[];
  logins?: LoginSummary[];
}

export function ProviderEditor({ tool, open, onOpenChange, provider, template, existingIds, accounts = [], logins = [] }: Props) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const descriptionId = useId();
  const meta = toolMeta(tool);
  const editing = Boolean(provider);
  const copying = !editing && Boolean(template);
  const source = provider ?? template;
  // 新建时先选一个起点（预设或空白），再进入表单。
  const [step, setStep] = useState<"start" | "form">(editing || copying ? "form" : "start");
  const [chosen, setChosen] = useState(false);
  const [commonOpen, setCommonOpen] = useState(false);
  const [tab, setTab] = useState<"form" | "config">("form");
  const [query, setQuery] = useState("");
  const [preset, setPreset] = useState<Preset | null>(null);
  const [name, setName] = useState(provider?.name ?? (template ? t("editor.copyName", { name: template.name }) : ""));
  const [id, setId] = useState(provider?.id ?? (template ? uniqueId(`${template.id}-copy`, existingIds) : ""));
  const [website, setWebsite] = useState((source?.websiteUrl as string | undefined) ?? "");
  const [notes, setNotes] = useState((provider?.notes as string | undefined) ?? "");
  const [category, setCategory] = useState<string | undefined>(source?.category);
  const [text, setText] = useState<EditorText>(() => {
    const settings = provider?.settingsConfig ?? (template ? withoutRedacted(template.settingsConfig ?? {}) : customTemplate(tool));
    return toEditorText(meta.editor, settings);
  });
  const [loginSource, setLoginSource] = useState(loginSourceOf(source));
  const [routes, setRoutes] = useState(() => routesOf(source));
  const [apply, setApply] = useState(false);
  const [checkModel, setCheckModel] = useState("");
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [confirm, confirmDialog] = useConfirm();
  const signature = JSON.stringify({ name, id, website, notes, category, text, loginSource, routes, apply });
  const [initialSignature] = useState(signature);
  const [presetSignature, setPresetSignature] = useState(signature);
  const dirty = signature !== initialSignature;
  useEditorNavigation({ dirty: open && dirty, saving: open && saving, discard: () => onOpenChange(false) });

  const presetList = useQuery({
    queryKey: ["presets", tool],
    queryFn: () => loadPresets(tool),
    enabled: !editing && !copying,
    staleTime: Infinity,
  });
  const presets = useMemo(() => filterPresets(presetList.data ?? [], query), [presetList.data, query]);
  const parsed = fromEditorText(meta.editor, text, source?.settingsConfig);
  const shortcutValue = meta.editor === "codex" ? readToml(text.toml ?? "") : parsed.ok ? parsed.value : null;
  const fieldError = useFieldError(shortcutValue ? configFieldsIssue(tool, "provider", shortcutValue) : null);
  const shortcuts = providerFieldsFor(tool, shortcutValue);
  const changeFields = (changes: [ConfigPath, string | number | undefined][]) => {
    if (!parsed.ok || !shortcutValue) return;
    try {
      if (meta.editor === "codex") {
        let toml = text.toml ?? "";
        for (const [key, value] of changes) {
          const parts = pathParts(key).map(String);
          toml = setTomlValue(toml, parts.length > 1 ? parts.slice(0, -1) : null, parts[parts.length - 1], value);
        }
        if (!readToml(toml)) throw new Error(t("config.fields.advancedToml"));
        setText((s) => ({ ...s, toml }));
      } else {
        const next = changes.reduce((value, [key, next]) => setPath(value, key, next), shortcutValue);
        setText({ json: JSON.stringify(next, null, 2) });
      }
      setSaveError(null);
    } catch (e) { setSaveError(errorMessage(e)); }
  };
  const settingsKey = parsed.ok ? JSON.stringify(parsed.value) : null;
  const official = isOfficial({ category });
  const showLogin = tool === "codex" && official;
  const loginOptions = useMemo(() => loginChoices(logins, accounts, loginSource, t), [logins, accounts, loginSource, t]);
  const showRoutes = tool === "claude-desktop" && !official;
  const routesOk = !showRoutes || routeErrors(routes).every((e) => e === null);
  const connection = readConnection(tool, text, official);
  const ready = connection.state === "ready" ? connection : null;
  const connectionFields = ready?.fields.filter((f) => f.group === "connection") ?? [];
  const modelFields = ready?.fields.filter((f) => f.group === "models" && !(tool === "grokbuild" && f.id === "contextWindow" && shortcuts.length > 0)) ?? [];

  // 只有 Claude 会过滤字段；停止输入一会儿再问后端，列表与保存时的提示一致。
  const checked = useDebounced(tool === "claude" && parsed.ok && !official ? settingsKey : null, 400);
  const ignored = useQuery({
    queryKey: ["ignored-fields", tool, checked],
    queryFn: () => api.ignoredFields(tool, JSON.parse(checked!) as Record<string, unknown>),
    enabled: checked !== null,
    placeholderData: keepPreviousData,
    staleTime: Infinity,
  });
  const ignoredFields = checked !== null ? (ignored.data ?? []) : [];

  // 检测都用编辑中的配置，不需要先保存。地址或密钥在结果出来后又改过，就标成“可能已过期”；
  // 只改模型不影响测连通和模型列表。
  const values = ready?.values ?? {};
  const reachKey = JSON.stringify([values.baseUrl, values.api, connection.state]);
  const authKey = JSON.stringify([reachKey, values.apiKey, values.authField]);
  type Run = { config: string; key: string; model?: string };
  const latencyProbe = useMutation({ mutationFn: (run: Run) => settle(() => api.probeLatency(tool, JSON.parse(run.config))) });
  const modelsProbe = useMutation({ mutationFn: (run: Run) => settle(() => api.probeModels(tool, JSON.parse(run.config))) });
  const checkProbe = useMutation({ mutationFn: (run: Run) => settle(() => api.probeCheck(tool, JSON.parse(run.config), run.model)) });
  const slot = (pending: boolean, result: ProbeResult | undefined, run: Run | undefined, key: string): ProbeSlot => ({
    pending,
    result,
    stale: Boolean(result) && run?.key !== key,
  });
  const remoteModels = modelsProbe.data?.ok ? (modelsProbe.data.models ?? []) : [];
  const candidates = (() => {
    const list: string[] = [];
    const push = (value: unknown) => {
      if (typeof value === "string" && value.trim() && !list.includes(value.trim())) list.push(value.trim());
    };
    for (const key of ["model", "sonnetModel", "haikuModel", "opusModel"] as const) push(values[key]);
    if (Array.isArray(values.models)) values.models.forEach(push);
    if (showRoutes) routes.forEach((route) => push(route.id));
    remoteModels.slice(0, 50).forEach((model) => push(model.id));
    return list;
  })();
  const selectedModel = candidates.includes(checkModel) ? checkModel : (candidates[0] ?? "");
  // 地址由别处决定（云厂商认证、Codex 本地供应商）或缺少默认模型时，检测没有意义，不显示入口。
  const canProbe = ready !== null && !official && !ready.blocked && parsed.ok;

  // 推理强度：先看模型列表自带的档位（Anthropic），再看逐档检测的结果；检测更贴近实际网关，优先采用。
  const effortField = ready?.fields.find((f) => f.kind === "effort");
  const effortModel = (["model", "sonnetModel", "haikuModel", "opusModel"] as const)
    .map((key) => values[key])
    .find((value): value is string => typeof value === "string" && value.trim() !== "")?.trim() ?? "";
  const effortsProbe = useMutation({ mutationFn: (run: Run) => settle(() => api.probeEfforts(tool, JSON.parse(run.config), run.model)) });
  const effortKey = `${authKey}${effortModel}`;
  const effort: EffortState | undefined = effortField && (() => {
    const run = effortsProbe.variables;
    const probed = effortsProbe.data && run?.model === effortModel ? effortsProbe.data : undefined;
    const listed = remoteModels.find((m) => m.id === effortModel)?.efforts;
    const support = probed?.ok && probed.levels
      ? Object.fromEntries(probed.levels.map((l) => [l.level, l.supported]))
      : listed
        ? Object.fromEntries((effortField.levels ?? []).map((level) => [level, listed.includes(level)]))
        : undefined;
    return {
      model: effortModel,
      support,
      source: probed?.ok ? "probe" : listed ? "models" : undefined,
      pending: effortsProbe.isPending,
      failure: probed && !probed.ok ? probed : undefined,
      stale: Boolean(probed) && run?.key !== effortKey,
      onDetect: canProbe && effortModel && settingsKey ? () => effortsProbe.mutate({ config: settingsKey, key: effortKey, model: effortModel }) : undefined,
    };
  })();

  async function choosePreset(p: Preset | null) {
    if (saving) return;
    setChosen(true);
    if (preset?.name === p?.name) {
      setStep("form");
      return;
    }
    if (signature !== presetSignature && !(await confirm({ title: t("editor.replaceTitle"), description: t("editor.replaceHint"), confirmLabel: t("editor.replaceAction") }))) return;
    setPreset(p);
    const nextName = p?.name ?? "";
    const nextId = copying ? id : uniqueId(slugify(nextName || "custom"), existingIds);
    const nextWebsite = p?.websiteUrl ?? "";
    const nextCategory = p?.isOfficial ? "official" : p?.category;
    const nextText = toEditorText(meta.editor, p?.settingsConfig ?? customTemplate(tool));
    setName(nextName);
    setId(nextId);
    setWebsite(nextWebsite);
    setCategory(nextCategory);
    setText(nextText);
    setSaveError(null);
    latencyProbe.reset();
    modelsProbe.reset();
    checkProbe.reset();
    effortsProbe.reset();
    setPresetSignature(JSON.stringify({ name: nextName, id: nextId, website: nextWebsite, notes, category: nextCategory, text: nextText, loginSource, routes, apply }));
    setStep("form");
  }

  async function requestClose() {
    if (saving) return;
    if (!dirty || await confirm({ title: t("editor.discardTitle"), description: t("editor.discardHint"), confirmLabel: t("editor.discardAction") })) onOpenChange(false);
  }

  async function openKeyPage(url: string) {
    try {
      await api.openUrl(url);
    } catch {
      toast.error(t("editor.openError"));
    }
  }

  async function save() {
    if (!canSave || !parsed.ok) return;
    setSaving(true);
    setSaveError(null);
    try {
      let next: Provider;
      if (editing) {
        next = { ...provider!, name: name.trim() || provider!.name, settingsConfig: parsed.value };
      } else {
        // 复制时保留模板上的其他字段（图标、扩展信息等），排序和备注不带过来。
        const { sortIndex: _sort, notes: _notes, ...rest } = template ?? ({} as Partial<Provider>);
        next = { ...rest, id: uniqueId(id.trim(), existingIds), name: name.trim() || id.trim(), settingsConfig: parsed.value, createdAt: Date.now() };
        if (category) next.category = category;
        else delete next.category;
      }
      if (website.trim()) next.websiteUrl = website.trim();
      else delete next.websiteUrl;
      if (notes.trim()) next.notes = notes.trim();
      else delete next.notes;
      if (showRoutes) next = applyRoutes(next, routes);
      if (showLogin) next = applyLoginSource(next, loginSource);

      const result = editing
        ? await api.updateProvider(tool, next)
        : await api.addProvider(tool, next, apply);
      showWarnings(result);
      toast.success(t("editor.saved"));
      onOpenChange(false);
    } catch (error) {
      setSaveError(errorMessage(error));
    } finally {
      setSaving(false);
      void refreshAll(client);
    }
  }

  const canSave = step === "form" && parsed.ok && !fieldError && routesOk && (editing ? dirty : id.trim() !== "") && !saving;
  const keyUrl = preset?.apiKeyUrl;
  const keyAction = keyUrl ? (
    <button type="button" className="inline-flex items-center gap-1 text-xs text-primary hover:underline" onClick={() => void openKeyPage(keyUrl)}>
      <ExternalLink className="h-3 w-3" aria-hidden="true" /> {t("editor.getKey")}
    </button>
  ) : undefined;
  const fieldsFor = (fields: FieldSpec[]) => ready && (
    <ConnectionFields
      tool={tool}
      fields={fields}
      values={ready.values}
      locked={ready.blocked?.fields}
      keyAction={keyAction}
      models={remoteModels}
      effort={effort}
      onChange={(field, value) => setText((current) => writeConnection(tool, current, field, value))}
    />
  );
  const title = t(editing ? "editor.titleEdit" : copying ? "editor.titleCopy" : "editor.titleAdd");

  const startStep = (
    <div className="flex min-h-0 flex-col gap-4 px-6 py-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h3 className="text-sm font-semibold tracking-tight">{t("editor.startTitle")}</h3>
        <div className="relative w-full max-w-xs">
          <Search className="pointer-events-none absolute left-2.5 top-2.5 size-4 text-muted-foreground" aria-hidden="true" />
          <Input type="search" aria-label={t("editor.searchPreset")} className="pl-8" placeholder={t("editor.searchPreset")} value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
      </div>
      {presetList.isError && (
        <div role="alert" className="flex flex-wrap items-center gap-3 rounded-md border border-destructive/30 bg-destructive/[0.06] px-3 py-2 text-xs text-destructive">
          <span>{t("editor.presetsError")}</span>
          <Button type="button" size="sm" variant="outline" onClick={() => void presetList.refetch()}><RotateCcw />{t("common.retry")}</Button>
        </div>
      )}
      <div className="grid gap-2.5 sm:grid-cols-2 lg:grid-cols-3" role="group" aria-label={t("editor.preset")}>
        {!query.trim() && (
          <StartTile
            label={t("editor.custom")}
            hint={t("editor.customHint")}
            selected={chosen && preset === null}
            icon={<Plus className="h-4 w-4" aria-hidden="true" />}
            onClick={() => void choosePreset(null)}
          />
        )}
        {presets.map((p) => (
          <StartTile
            key={p.name}
            label={p.name}
            hint={p.isOfficial ? t("editor.presetOfficial") : (presetHost(tool, p) ?? t("editor.presetFillIn"))}
            badge={p.isOfficial ? t("card.official") : undefined}
            selected={preset?.name === p.name}
            onClick={() => void choosePreset(p)}
          />
        ))}
      </div>
      {presetList.isLoading && <p role="status" className="text-xs text-muted-foreground">{t("common.loading")}</p>}
      {!presetList.isLoading && !presetList.isError && query.trim() && presets.length === 0 && (
        <div role="status" className="flex flex-col items-start gap-2 text-xs text-muted-foreground">
          <p>{t("editor.noPresets")}</p>
          <Button type="button" size="sm" variant="outline" onClick={() => void choosePreset(null)}><Plus />{t("editor.custom")}</Button>
        </div>
      )}
    </div>
  );

  const fieldControls = shortcuts.length > 0 && <Section title={t("config.quickOptions")} aside={tool === "codex" && <Button type="button" variant="outline" size="sm" disabled={saving || !parsed.ok} onClick={() => changeFields([["model_context_window", 1000000], ["model_auto_compact_token_limit", 900000]])}>{t("config.fields.contextPreset")}</Button>}>
    <ConfigFields fields={shortcuts} value={shortcutValue ?? {}} disabled={saving || !parsed.ok} onChange={(key, value) => changeFields([[key, value]])} />
    <ConfigCompatibility tool={tool} config={shortcutValue} />
  </Section>;

  const formTab = (
    <div className="flex min-w-0 flex-col gap-7 px-6 py-5">
      {!parsed.ok && (
        <Note tone="warning">
          {t("editor.fieldsInvalid")}{" "}
          <button type="button" className="font-medium text-primary hover:underline" onClick={() => setTab("config")}>{t("editor.openConfig")}</button>
        </Note>
      )}
      <Section title={t("editor.sectionBasic")}>
        <div className="grid gap-x-3 gap-y-4 sm:grid-cols-2">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="pe-name">{t("editor.name")}</Label>
            <Input id="pe-name" value={name} onChange={(e) => {
              const next = e.target.value;
              setName(next);
              if (!editing && !copying) setId(next.trim() ? uniqueId(slugify(next), existingIds) : "");
            }} />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="pe-web">{t("editor.website")}</Label>
            <Input id="pe-web" type="url" placeholder="https://" value={website} onChange={(e) => setWebsite(e.target.value)} />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="pe-notes">{t("editor.notes")}</Label>
            <Input id="pe-notes" placeholder={t("editor.notesPlaceholder")} value={notes} onChange={(e) => setNotes(e.target.value)} />
          </div>
        </div>
      </Section>

      {showLogin && (
        <Section title={t("editor.loginSource")}>
          <NativeSelect id="pe-login" aria-label={t("editor.loginSource")} aria-describedby="pe-login-help" value={loginSource} onChange={(e) => setLoginSource(e.target.value)}>
            {loginOptions.missing && <option value={loginOptions.missing.value}>{loginOptions.missing.label}</option>}
            <option value={loginOptions.native.value}>{loginOptions.native.label}</option>
            {loginOptions.local.length > 0 && (
              <optgroup label={t("editor.loginLocalGroup")}>
                {loginOptions.local.map((c) => <option key={c.value} value={c.value}>{c.label}</option>)}
              </optgroup>
            )}
            {loginOptions.managed.length > 0 && (
              <optgroup label={t("editor.loginManagedGroup")}>
                {loginOptions.managed.map((c) => <option key={c.value} value={c.value}>{c.label}</option>)}
              </optgroup>
            )}
          </NativeSelect>
          <p id="pe-login-help" className="mt-1.5 text-xs leading-relaxed text-muted-foreground">{loginHelp(loginSource, loginOptions, t)}</p>
        </Section>
      )}

      {connection.state === "official" && !showLogin && (
        <Section title={t("editor.sectionConnection")}>
          <Note>{t("editor.officialNote")}</Note>
        </Section>
      )}

      {connectionFields.length > 0 && (
        <Section title={t("editor.sectionConnection")}>
          {ready?.blocked && <Note>{t(`editor.blocked_${ready.blocked.reason}`, { id: ready.blocked.id })}</Note>}
          {fieldsFor(connectionFields)}
          {canProbe && (
            <ConnectionCheck
              latency={slot(latencyProbe.isPending, latencyProbe.data, latencyProbe.variables, reachKey)}
              check={slot(checkProbe.isPending, checkProbe.data, checkProbe.variables, `${authKey}${selectedModel}`)}
              onLatency={() => settingsKey && latencyProbe.mutate({ config: settingsKey, key: reachKey })}
              onCheck={() => settingsKey && checkProbe.mutate({ config: settingsKey, key: `${authKey}${selectedModel}`, model: selectedModel })}
              candidates={candidates}
              model={selectedModel}
              onModel={setCheckModel}
            />
          )}
        </Section>
      )}

      {(modelFields.length > 0 || showRoutes) && (
        <Section
          title={t("editor.sectionModels")}
          hint={tool === "claude" ? t("editor.claudeModelsHint") : undefined}
          aside={canProbe && (
            <Button type="button" size="sm" variant="outline" disabled={modelsProbe.isPending} onClick={() => settingsKey && modelsProbe.mutate({ config: settingsKey, key: authKey })}>
              {modelsProbe.isPending ? <Loader2 className="animate-spin" /> : <ListRestart />} {t("probe.fetchModels")}
            </Button>
          )}
        >
          {(modelsProbe.isPending || modelsProbe.data) && <ProbeLine kind="models" slot={slot(modelsProbe.isPending, modelsProbe.data, modelsProbe.variables, authKey)} />}
          {modelFields.length > 0 && fieldsFor(modelFields)}
          {showRoutes && <ModelRoutesEditor rows={routes} onChange={setRoutes} />}
        </Section>
      )}

      {fieldControls}

      {ignoredFields.length > 0 && (
        <Note tone="warning">
          {t("editor.ignoredSummary", { count: ignoredFields.length })}{" "}
          <button type="button" className="font-medium text-primary hover:underline" onClick={() => setTab("config")}>{t("editor.openConfig")}</button>
        </Note>
      )}
    </div>
  );

  const configTab = (
    <div className="flex min-w-0 flex-col gap-3 px-6 py-5">
      <div className="flex flex-wrap items-center justify-between gap-3"><h3 className="text-sm font-semibold">{t("config.providerTitle")}</h3><Button type="button" size="sm" variant="outline" disabled={saving} onClick={() => setCommonOpen(true)}>{t("config.editCommon")}</Button></div>
      <p className="text-xs leading-5 text-muted-foreground">{t("config.providerHint")}</p>
      <ConfigOptions options={providerOptions[tool] ?? []} value={(meta.editor === "codex" ? readToml(text.toml ?? "") : parsed.ok ? parsed.value : null) ?? {}} disabled={saving || !parsed.ok} onChange={(option, state) => {
        if (!parsed.ok) return;
        if (meta.editor === "codex") {
          const value = state === "default" ? undefined : state ? option.on : option.off;
          setText((s) => ({ ...s, toml: setTomlValue(s.toml ?? "", null, option.key, value as string | boolean | undefined) }));
        } else {
          try { setText({ json: JSON.stringify(applyOption(parsed.value, option, state), null, 2) }); setSaveError(null); }
          catch (e) { setSaveError(errorMessage(e)); }
        }
      }} />
      {fieldControls}
      {copying && hasRedacted(toEditorText(meta.editor, template!.settingsConfig ?? {})) && <Note>{t("editor.copyRedactedNote")}</Note>}
      {editing && hasRedacted(text) && <Note tone="warning">{t("editor.redactedNote")}</Note>}
      {meta.editor === "codex" ? (
        <>
          <Label className="text-xs text-muted-foreground">{t("editor.auth")}</Label>
          <CodeEditor readOnly={saving} language="json" minHeight={90} value={text.json} onChange={(json) => setText((s) => ({ ...s, json }))} ariaLabel={t("editor.auth")} />
          <Label className="text-xs text-muted-foreground">{t("editor.config")}</Label>
          <CodeEditor readOnly={saving} language="toml" minHeight={200} value={text.toml ?? ""} onChange={(toml) => setText((s) => ({ ...s, toml }))} ariaLabel={t("editor.config")} />
        </>
      ) : (
        <CodeEditor readOnly={saving} language="json" minHeight={260} value={text.json} onChange={(json) => setText({ json })} ariaLabel={t("editor.settings")} />
      )}
      {!parsed.ok && (
        <p role="alert" className="text-xs text-destructive">
          {t("editor.invalid")}: {parsed.error}
        </p>
      )}
      {ignoredFields.length > 0 && (
        <Note tone="warning">
          <p>{t("editor.ignoredHint", { count: ignoredFields.length })}</p>
          <ul className="mt-1.5 flex flex-wrap gap-1" aria-label={t("editor.ignoredTitle")}>
            {ignoredFields.map((field) => <li key={field}><code className="rounded bg-background/80 px-1.5 py-0.5 font-mono text-[11px]">{field}</code></li>)}
          </ul>
        </Note>
      )}
    </div>
  );

  return (
    <>
    <Dialog open={open} onOpenChange={(next) => { if (!next) void requestClose(); }}>
      <DialogContent
        aria-describedby={descriptionId}
        closeDisabled={saving}
        // 模型下拉展开时，Esc 只收起下拉。
        onEscapeKeyDown={(e) => { if (document.activeElement?.getAttribute("aria-expanded") === "true") e.preventDefault(); }}
        className="flex h-[min(780px,calc(100dvh-2rem))] w-[calc(100vw-2rem)] max-w-[52rem] flex-col gap-0 overflow-hidden p-0"
      >
        <DialogHeader className="shrink-0 gap-1 px-6 pb-3 pt-5 pr-12">
          <div className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
            <ToolGlyph tool={meta} size={18} />
            {meta.label}
          </div>
          <DialogTitle className="text-lg">{title}<span className="sr-only"> · {meta.label}</span></DialogTitle>
          <DialogDescription id={descriptionId}>
            {editing ? t("editor.editHint") : copying ? t("editor.copyHint", { name: template!.name }) : t("editor.addHint")}
          </DialogDescription>
        </DialogHeader>

        <fieldset disabled={saving} className="flex min-h-0 min-w-0 flex-1 flex-col">
          {step === "start" ? (
            <div className="min-h-0 flex-1 overflow-y-auto border-t">{startStep}</div>
          ) : (
            <Tabs.Root value={tab} onValueChange={(value) => setTab(value as "form" | "config")} className="flex min-h-0 flex-1 flex-col">
              <div className="flex shrink-0 flex-wrap items-center justify-between gap-x-4 border-b px-6">
                <Tabs.List aria-label={title} className="flex gap-5">
                  <Tabs.Trigger value="form" className={TAB}>{t("editor.tabForm")}</Tabs.Trigger>
                  <Tabs.Trigger value="config" className={TAB}>
                    {t("editor.tabConfig")}
                    {!parsed.ok && <span className="h-1.5 w-1.5 rounded-full bg-destructive" aria-label={t("editor.invalid")} />}
                  </Tabs.Trigger>
                </Tabs.List>
                {!editing && !copying && (
                  <div className="flex min-w-0 items-center gap-2 py-2 text-xs text-muted-foreground">
                    <span className="truncate">{t("editor.startFrom", { name: preset?.name ?? t("editor.custom") })}</span>
                    <Button type="button" size="sm" variant="ghost" className="h-7 px-2" onClick={() => setStep("start")}>
                      <ArrowLeft className="h-3.5 w-3.5" /> {t("editor.changeStart")}
                    </Button>
                  </div>
                )}
              </div>
              <Tabs.Content value="form" className="min-h-0 flex-1 overflow-y-auto focus-visible:outline-none">{formTab}</Tabs.Content>
              <Tabs.Content value="config" className="min-h-0 flex-1 overflow-y-auto focus-visible:outline-none">{configTab}</Tabs.Content>
            </Tabs.Root>
          )}
        </fieldset>

        <DialogFooter className="shrink-0 flex-wrap items-center gap-y-2 border-t bg-muted/40 px-6 py-3.5">
          <div className="mr-auto flex min-w-0 flex-1 basis-56 flex-col gap-1 text-xs text-muted-foreground">
            {step === "form" && !editing && (
              <label className="flex items-center gap-2 text-sm text-foreground">
                <Checkbox checked={apply} disabled={saving} onCheckedChange={(v) => setApply(v === true)} />
                {meta.kind === "coexist" ? t("editor.addToLive") : t("editor.switchAfterAdd")}
              </label>
            )}
            {fieldError || saveError ? (
              <p role="alert" className="break-words text-destructive">{fieldError ?? `${t("editor.saveError")} ${saveError}`}</p>
            ) : editing ? (
              <p role="status">{t(saving ? "editor.saving" : dirty ? "editor.saveHint" : "editor.unchanged")}</p>
            ) : null}
          </div>
          <Button variant="outline" disabled={saving} onClick={() => void requestClose()}>
            {t("editor.cancel")}
          </Button>
          {step === "form" && (
            <Button onClick={() => void save()} disabled={!canSave}>
              {saving && <Loader2 className="animate-spin" />}{t(saving ? "editor.saving" : "editor.save")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
    {commonOpen && <CommonConfigEditor tool={tool} onClose={() => setCommonOpen(false)} />}
    {confirmDialog}
    </>
  );
}

function StartTile({ label, hint, badge, selected, icon, onClick }: { label: string; hint: string; badge?: string; selected: boolean; icon?: ReactNode; onClick: () => void }) {
  const labelId = useId();
  const hintId = useId();
  return (
    <button
      type="button"
      aria-pressed={selected}
      aria-labelledby={labelId}
      aria-describedby={hintId}
      onClick={onClick}
      className={cn(
        "group flex min-h-[4.25rem] min-w-0 flex-col items-start justify-center gap-1 rounded-lg border bg-card px-3.5 py-3 text-left transition-colors hover:border-primary/50 hover:bg-accent/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        selected && "border-primary/60 bg-primary/[0.04]",
        icon && "border-dashed",
      )}
    >
      <span className="flex w-full min-w-0 items-center gap-2">
        {icon ? <span className="grid h-6 w-6 shrink-0 place-items-center rounded-md bg-muted text-muted-foreground">{icon}</span> : <Monogram name={label} size={24} />}
        <span id={labelId} className="min-w-0 truncate text-sm font-medium">{label}</span>
        {badge && <Badge variant="outline" className="shrink-0">{badge}</Badge>}
        {selected && <Check className="ml-auto h-4 w-4 shrink-0 text-primary" aria-hidden="true" />}
      </span>
      <span id={hintId} className="min-w-0 max-w-full truncate font-mono text-[11px] text-muted-foreground">{hint}</span>
    </button>
  );
}
