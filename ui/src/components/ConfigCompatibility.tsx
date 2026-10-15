import { useTranslation } from "react-i18next";
import type { ToolId } from "@/lib/api";
import { openCodeV2 } from "@/lib/config-options";

export function ConfigCompatibility({ tool, config, v2 = false }: { tool: ToolId; config: unknown; v2?: boolean }) {
  const { t } = useTranslation();
  if (tool === "claude-desktop") return null;
  return <details className="text-xs leading-5 text-muted-foreground">
    <summary className="w-fit cursor-pointer rounded focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{t("config.compatibilityTitle")}</summary>
    <p className="pt-1">{t(`config.compatibility.${tool === "opencode" && (v2 || openCodeV2(config)) ? "opencodeV2" : tool}`)}</p>
    <p>{t("config.compatibilityHint")}</p>
  </details>;
}
