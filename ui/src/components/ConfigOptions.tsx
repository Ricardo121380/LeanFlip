import { useTranslation } from "react-i18next";
import { Checkbox } from "./ui/checkbox";
import { atPath, optionValue, type ConfigOption } from "@/lib/config-options";

export function ConfigOptions({ options, value, disabled, onChange }: { options: ConfigOption[]; value: Record<string, unknown>; disabled?: boolean; onChange: (option: ConfigOption, checked: boolean | "default") => void }) {
  const { t } = useTranslation();
  if (!options.length) return null;
  return <div className="flex flex-wrap gap-x-5 gap-y-3" role="group" aria-label={t("config.quickOptions")}>
    {options.map((option) => {
      const label = t(`config.options.${option.label}`, { defaultValue: option.label });
      const unset = atPath(value, option.key) === undefined;
      return <div key={option.key} className="flex items-center gap-1.5 text-xs">
        <label className="flex cursor-pointer items-center gap-2"><Checkbox disabled={disabled} checked={optionValue(value, option)} onCheckedChange={(v) => onChange(option, Boolean(v))} />{label}</label>
        {unset ? <span className="text-[10px] text-muted-foreground">{t("config.default")}</span> : <button disabled={disabled} type="button" className="text-[10px] text-muted-foreground underline decoration-dotted" aria-label={t("config.resetOption", { name: label })} onClick={() => onChange(option, "default")}>{t("config.reset")}</button>}
      </div>;
    })}
  </div>;
}
