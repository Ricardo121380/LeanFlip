// 供应商编辑器里的结构化字段：地址、密钥、模型。值都来自配置文本，改动立即写回文本。
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { CircleCheck, Eye, EyeOff, Info, Loader2, Plus, ScanSearch, TriangleAlert, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input, NativeSelect } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ModelPicker } from "@/components/ModelPicker";
import type { ProbeResult, RemoteModel, ToolId } from "@/lib/api";
import type { FieldId, FieldSpec, FieldValue } from "@/lib/connection";
import { failureText, providerDetail } from "@/lib/probe";
import { cn } from "@/lib/utils";

const WIDE: FieldSpec["kind"][] = ["url", "secret", "list", "effort", "toggle"];
// 有“认证方式”时，它和 API Key 并排一行。
const isWide = (field: FieldSpec, fields: FieldSpec[]) =>
  WIDE.includes(field.kind) && !(field.kind === "secret" && fields.some((f) => f.id === "authField"));

interface Props {
  tool: ToolId;
  fields: FieldSpec[];
  values: Partial<Record<FieldId, FieldValue>>;
  locked?: FieldId[];
  onChange: (field: FieldId, value: FieldValue) => void;
  /** 显示在 API Key 标签旁，例如“获取 API Key”。 */
  keyAction?: ReactNode;
  /** 从供应商获取到的模型，用作模型输入框的候选。 */
  models?: RemoteModel[];
  /** 推理强度：哪些档位可用，以及检测入口。 */
  effort?: EffortState;
}

export interface EffortState {
  /** 每一档是否可用；还不知道时为空。 */
  support?: Record<string, boolean>;
  source?: "models" | "probe";
  /** 判断依据的模型。 */
  model: string;
  pending: boolean;
  /** 检测失败时的结果。 */
  failure?: ProbeResult;
  stale?: boolean;
  /** 没有时不显示“检测可用档位”。 */
  onDetect?: () => void;
}

export function ConnectionFields({ tool, fields, values, locked = [], onChange, keyAction, models = [], effort }: Props) {
  const { t } = useTranslation();
  const label = (field: FieldSpec) => {
    if (field.id === "model") return t(tool === "claude" ? "editor.claudeModel" : "editor.model");
    return t(`editor.field.${field.id}`);
  };
  return (
    <div className="grid gap-x-3 gap-y-4 sm:grid-cols-2">
      {fields.map((field) => {
        const inputId = `pe-field-${field.id}`;
        const value = values[field.id] ?? (field.kind === "list" ? [] : "");
        const disabled = locked.includes(field.id);
        return (
          <div key={field.id} className={cn("flex min-w-0 flex-col gap-1.5", isWide(field, fields) && "sm:col-span-2")}>
            <div className="flex min-h-5 items-center justify-between gap-2">
              <Label id={`${inputId}-label`} htmlFor={inputId}>{label(field)}</Label>
              {field.id === "apiKey" && keyAction
                ? keyAction
                : field.hint && <code className="min-w-0 truncate font-mono text-[11px] text-muted-foreground" title={field.hint}>{field.hint}</code>}
            </div>
            {field.kind === "effort" ? (
              <EffortPicker id={inputId} value={String(value)} levels={field.levels ?? []} effort={effort} disabled={disabled} onChange={(next) => onChange(field.id, next)} />
            ) : field.kind === "toggle" ? (
              <label className="flex items-start gap-2.5 text-sm">
                <Checkbox id={inputId} className="mt-0.5" checked={value === "1"} disabled={disabled} onCheckedChange={(checked) => onChange(field.id, checked === true ? "1" : "")} />
                <span className="text-xs leading-5 text-muted-foreground">{t(`editor.hint_${field.id}`)}</span>
              </label>
            ) : field.kind === "secret" ? (
              <SecretInput id={inputId} value={String(value)} disabled={disabled} placeholder={field.hint} onChange={(next) => onChange(field.id, next)} />
            ) : field.kind === "select" ? (
              <NativeSelect id={inputId} value={String(value)} disabled={disabled} className="disabled:bg-muted disabled:opacity-70" onChange={(e) => onChange(field.id, e.target.value)}>
                {value === "" && <option value="">{t("editor.unset")}</option>}
                {field.options?.map((option) => (
                  <option key={option} value={option}>{field.id === "authField" ? t(`editor.auth_${option}`) : option}</option>
                ))}
              </NativeSelect>
            ) : field.kind === "list" ? (
              <ModelList id={inputId} models={Array.isArray(value) ? value : []} suggestions={models} disabled={disabled} onChange={(next) => onChange(field.id, next)} />
            ) : field.kind === "text" ? (
              <ModelPicker
                id={inputId}
                value={String(value)}
                disabled={disabled}
                suggestions={field.suggestions ? field.suggestions.map((id) => ({ id })) : models}
                placeholder={t("editor.optional")}
                onChange={(next) => onChange(field.id, next)}
              />
            ) : (
              <Input
                id={inputId}
                value={String(value)}
                disabled={disabled}
                type={field.kind === "url" ? "url" : "text"}
                inputMode={field.kind === "number" ? "numeric" : undefined}
                spellCheck={false}
                autoComplete="off"
                placeholder={field.kind === "url" ? "https://" : t("editor.optional")}
                className={cn(field.kind === "url" && "font-mono text-[13px]")}
                onChange={(e) => onChange(field.id, field.kind === "number" ? e.target.value.replace(/\D/g, "") : e.target.value)}
              />
            )}
          </div>
        );
      })}
    </div>
  );
}

function SecretInput({ id, value, disabled, placeholder, onChange }: { id: string; value: string; disabled: boolean; placeholder?: string; onChange: (value: string) => void }) {
  const { t } = useTranslation();
  const [visible, setVisible] = useState(false);
  return (
    <div className="relative">
      <Input
        id={id}
        value={value}
        disabled={disabled}
        type={visible ? "text" : "password"}
        autoComplete="off"
        spellCheck={false}
        placeholder={placeholder}
        className="pr-10 font-mono text-[13px]"
        onChange={(e) => onChange(e.target.value)}
      />
      <Button
        type="button"
        size="icon"
        variant="ghost"
        className="absolute right-0.5 top-0.5 text-muted-foreground"
        aria-label={t(visible ? "editor.hideKey" : "editor.showKey")}
        aria-pressed={visible}
        onClick={() => setVisible((v) => !v)}
      >
        {visible ? <EyeOff /> : <Eye />}
      </Button>
    </div>
  );
}

function ModelList({ id, models, suggestions, disabled, onChange }: { id: string; models: string[]; suggestions: RemoteModel[]; disabled: boolean; onChange: (models: string[]) => void }) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState("");
  const add = (value = draft) => {
    const next = value.trim();
    if (!next) return;
    if (!models.includes(next)) onChange([...models, next]);
    setDraft("");
  };
  return (
    <div className="flex flex-col gap-2">
      {models.length > 0 ? (
        <ul className="flex flex-wrap gap-1.5" aria-label={t("editor.field.models")}>
          {models.map((model) => (
            <li key={model} className="inline-flex max-w-full items-center gap-1 rounded-md border bg-muted/60 py-0.5 pl-2 pr-0.5 font-mono text-xs">
              <span className="min-w-0 break-all">{model}</span>
              <button
                type="button"
                disabled={disabled}
                className="grid h-5 w-5 shrink-0 place-items-center rounded text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-50"
                aria-label={t("editor.modelRemove", { model })}
                onClick={() => onChange(models.filter((m) => m !== model))}
              >
                <X className="h-3 w-3" />
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="text-xs text-muted-foreground">{t("editor.modelsEmpty")}</p>
      )}
      <div className="flex gap-2">
        <div className="min-w-0 flex-1">
          <ModelPicker
            id={id}
            value={draft}
            disabled={disabled}
            suggestions={suggestions.filter((m) => !models.includes(m.id))}
            placeholder={t(suggestions.length ? "editor.modelPickPlaceholder" : "editor.modelPlaceholder")}
            onChange={setDraft}
            onPick={(model) => add(model)}
            onEnter={() => add()}
          />
        </div>
        <Button type="button" variant="outline" disabled={disabled || !draft.trim()} onClick={() => add()}>
          <Plus /> {t("editor.modelAdd")}
        </Button>
      </div>
    </div>
  );
}

function EffortPicker({ id, value, levels, effort, disabled, onChange }: { id: string; value: string; levels: string[]; effort?: EffortState; disabled: boolean; onChange: (value: string) => void }) {
  const { t } = useTranslation();
  const custom = value !== "" && !levels.includes(value);
  const [editing, setEditing] = useState(custom);
  const support = effort?.support;
  const anySupported = support && Object.values(support).some(Boolean);
  const chosenUnsupported = value !== "" && support?.[value] === false;
  const pill = (key: string, label: string, checked: boolean, onSelect: () => void, unsupported = false) => (
    <button
      key={key}
      type="button"
      role="radio"
      aria-checked={checked}
      disabled={disabled}
      title={unsupported ? t("editor.effortUnsupported") : undefined}
      onClick={onSelect}
      className={cn(
        "h-7 rounded-md px-2.5 font-mono text-xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50",
        checked ? "bg-card text-foreground shadow-sm ring-1 ring-border" : "text-muted-foreground hover:text-foreground",
        unsupported && !checked && "text-muted-foreground/50 line-through decoration-muted-foreground/40",
      )}
    >
      {label}
    </button>
  );
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <div role="radiogroup" aria-labelledby={`${id}-label`} className="inline-flex w-fit max-w-full flex-wrap gap-0.5 rounded-lg bg-muted p-0.5">
        {pill("default", t("editor.effortDefault"), value === "" && !editing, () => { setEditing(false); onChange(""); })}
        {levels.map((level) => pill(level, level, value === level && !editing, () => { setEditing(false); onChange(level); }, support?.[level] === false))}
        {pill("custom", t("editor.effortCustom"), editing || custom, () => setEditing(true))}
      </div>
      {(editing || custom) && (
        <Input id={id} value={value} disabled={disabled} spellCheck={false} autoComplete="off" placeholder={t("editor.effortCustomPlaceholder")} className="max-w-xs font-mono text-[13px]" onChange={(e) => onChange(e.target.value)} />
      )}
      {effort && (
        <div className={cn("flex flex-wrap items-center gap-x-3 gap-y-1.5 text-xs leading-5 text-muted-foreground", effort.stale && "opacity-60")}>
          {effort.pending ? (
            <span role="status" className="inline-flex items-center gap-1.5"><Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />{t("probe.running_efforts")}</span>
          ) : effort.failure ? (
            <span role="status" className="inline-flex min-w-0 items-start gap-1.5 text-destructive">
              <TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
              <span className="min-w-0 break-words">{failureText(t, effort.failure, "check")}{providerDetail(effort.failure) ? ` · ${providerDetail(effort.failure)}` : ""}</span>
            </span>
          ) : support ? (
            <span role="status" className="inline-flex items-center gap-1.5">
              <CircleCheck className="h-3.5 w-3.5 shrink-0 text-success" aria-hidden="true" />
              {anySupported ? t(effort.source === "probe" ? "editor.effortFromProbe" : "editor.effortFromModels", { model: effort.model }) : t("editor.effortNoSupport", { model: effort.model })}
            </span>
          ) : (
            <span className="inline-flex items-center gap-1.5"><Info className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />{t("editor.effortUnknown")}</span>
          )}
          {effort.onDetect && (
            <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs" disabled={disabled || effort.pending} onClick={effort.onDetect} title={t("editor.effortDetectNote", { model: effort.model, count: levels.length + 1 })}>
              <ScanSearch className="h-3.5 w-3.5" /> {t(support ? "editor.effortRedetect" : "editor.effortDetect")}
            </Button>
          )}
        </div>
      )}
      {effort?.stale && <p className="text-xs text-muted-foreground">{t("probe.stale")}</p>}
      {chosenUnsupported && (
        <p className="text-xs text-warning">{t("editor.effortChosenUnsupported", { level: value, model: effort?.model ?? "" })}</p>
      )}
    </div>
  );
}
