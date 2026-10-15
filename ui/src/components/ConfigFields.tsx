import { useId } from "react";
import { useTranslation } from "react-i18next";
import { RotateCcw } from "lucide-react";
import { atPath, type ConfigPath } from "@/lib/config-options";
import { fieldInput, fieldIssue, fieldPath, type ConfigField, type FieldIssue } from "@/lib/config-fields";
import { Input, NativeSelect } from "./ui/input";
import { Label } from "./ui/label";

export function useFieldError(issue: FieldIssue | null): string | null {
  const { t } = useTranslation();
  if (!issue) return null;
  return t(`config.fields.${issue.reason === "integer" && issue.max !== undefined ? "integerRange" : issue.reason}`, { name: t(`config.fields.${issue.label}`), min: issue.min, max: issue.max });
}

export function ConfigFields({ fields, value, before, disabled, onChange }: {
  fields: ConfigField[];
  value: Record<string, unknown>;
  before?: Record<string, unknown>;
  disabled?: boolean;
  onChange: (key: ConfigPath, value: string | number | undefined) => void;
}) {
  const { t } = useTranslation();
  const prefix = useId();
  if (!fields.length) return null;
  return <div className="grid gap-4 sm:grid-cols-2">
    {fields.map((field) => {
      const path = fieldPath(field);
      const id = `${prefix}-${field.key}`, raw = atPath(value, path);
      const label = `${t(`config.fields.${field.label}`)}${field.model ? ` · ${field.model}` : ""}`;
      const blockedBypass = field.key === "permissions.defaultMode" && atPath(value, "permissions.disableBypassPermissionsMode") === "disable";
      const blockedAuto = field.key === "permissions.defaultMode" && atPath(value, "permissions.disableAutoMode") === "disable";
      const cacheEnv = field.key === "promptCacheTtl" ? "CLAUDE_CODE_PROMPT_CACHE_TTL" : field.key === "subagentPromptCacheTtl" ? "CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL" : null;
      const override = cacheEnv && (["1", "true", "yes", "on"].includes(String(atPath(value, "env.FORCE_PROMPT_CACHING_5M") ?? "").toLowerCase())
        ? "FORCE_PROMPT_CACHING_5M" : ["5m", "1h"].includes(String(atPath(value, `env.${cacheEnv}`))) ? cacheEnv : undefined);
      const locked = disabled || Boolean(override);
      const warning = ["permissions.defaultMode", "permissionMode"].includes(field.key) && raw === "bypassPermissions" ? "bypassHint" : ((field.key === "sandbox_mode" && raw === "danger-full-access") || (field.key === "sandbox.profile" && raw === "off")) ? "fullAccessHint" : null;
      const selected = raw === undefined ? "__default" : field.kind === "select" && field.choices.some(([v]) => v === raw) ? String(raw) : "__custom";
      return <div key={field.key} className="min-w-0 space-y-1.5">
        <div className="flex min-h-6 items-center justify-between gap-2">
          <Label htmlFor={id} className="min-w-0 break-words">{label}</Label>
          {raw !== undefined && <button type="button" disabled={locked} onClick={() => onChange(path, undefined)} aria-label={t("config.resetOption", { name: label })} title={t("config.resetOption", { name: label })} className="grid size-6 shrink-0 place-items-center rounded text-muted-foreground hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-40"><RotateCcw className="size-3" aria-hidden="true" /></button>}
        </div>
        {field.kind === "select" ? <NativeSelect id={id} value={selected} disabled={locked} aria-describedby={`${id}-hint`} onChange={(e) => onChange(path, e.target.value === "__default" ? undefined : e.target.value)}>
          <option value="__default">{t("config.default")}</option>
          {selected === "__custom" && <option value="__custom" disabled>{t("config.fields.customValue")}</option>}
          {field.choices.map(([v, text]) => <option key={v} value={v} disabled={(v === "bypassPermissions" && blockedBypass) || (v === "auto" && blockedAuto)}>{t(`config.fields.${text}`)}</option>)}
        </NativeSelect> : <Input id={id} inputMode={field.decimal ? "decimal" : "numeric"} placeholder={t("config.default")} value={raw === undefined ? "" : typeof raw === "object" ? JSON.stringify(raw) : String(raw)} disabled={disabled} aria-invalid={Boolean(fieldIssue(field, value, before))} aria-describedby={`${id}-hint`} onChange={(e) => onChange(path, fieldInput(field, e.target.value))} />}
        <div id={`${id}-hint`} className="space-y-1 text-xs leading-5 text-muted-foreground">
          {field.hint && <p>{t(`config.fields.${field.hint}`)}</p>}
          {warning && <p className="text-amber-700 dark:text-amber-300">{t(`config.fields.${warning}`)}</p>}
          {blockedBypass && <p>{t("config.fields.bypassDisabled")}</p>}
          {blockedAuto && <p>{t("config.fields.autoDisabled")}</p>}
          {override && <p>{t("config.fields.overridden", { key: override })}</p>}
        </div>
      </div>;
    })}
  </div>;
}
