import { useTranslation } from "react-i18next";
import { RotateCcw } from "lucide-react";
import { Checkbox } from "./ui/checkbox";
import { optionState, type ConfigOption } from "@/lib/config-options";

export function ConfigOptions({ options, value, disabled, onChange }: { options: ConfigOption[]; value: Record<string, unknown>; disabled?: boolean; onChange: (option: ConfigOption, checked: boolean | "default") => void }) {
  const { t } = useTranslation();
  if (!options.length) return null;
  return <div className="flex flex-wrap gap-x-5 gap-y-3" role="group" aria-label={t("config.quickOptions")}>
    {options.map((option) => {
      const label = t(`config.options.${option.label}`, { defaultValue: option.label });
      const state = optionState(value, option);
      return <div key={option.key} className="flex items-center gap-1.5 text-xs">
        <label className="flex cursor-pointer items-center gap-2"><Checkbox disabled={disabled} checked={state === "custom" ? "indeterminate" : state === "on"} onCheckedChange={(v) => onChange(option, v === true)} />{label}</label>
        {(state === "default" || state === "custom") && <span className="text-[10px] text-muted-foreground">{t(`config.${state}`)}</span>}
        {state !== "default" && <button disabled={disabled} type="button" className="grid size-6 place-items-center rounded text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-40" aria-label={t("config.resetOption", { name: label })} title={t("config.resetOption", { name: label })} onClick={() => onChange(option, "default")}><RotateCcw className="size-3" aria-hidden="true" /></button>}
      </div>;
    })}
  </div>;
}
