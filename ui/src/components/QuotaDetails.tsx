// 一个 ChatGPT 账号的额度：每个窗口一行（已用比例、重置时间），credits，数据来源和时间。
import { useTranslation } from "react-i18next";
import { AlertTriangle, Loader2, RefreshCw } from "lucide-react";
import type { QuotaSource } from "@/lib/api";
import { agoText, creditsText, errorText, meterWidth, percentText, resetText, windowLabel, windowLevel } from "@/lib/quota";
import { cn } from "@/lib/utils";

/** 进度条：越满用得越多；80% 以上变黄，用完变红。 */
export function QuotaMeter({ usedPercent, reset, className }: { usedPercent: number; reset: boolean; className?: string }) {
  return (
    <span className={cn("quota-meter", className)} data-level={windowLevel(usedPercent, reset)} aria-hidden="true">
      <i style={{ width: meterWidth(usedPercent, reset) }} />
    </span>
  );
}

export function QuotaDetails({
  source,
  label,
  now,
  canRefresh,
  refreshing = false,
  onRefresh,
  showError = true,
}: {
  source: QuotaSource | undefined;
  /** 账号名，用在读屏标签里。 */
  label: string;
  now: number;
  /** 托管账号可以联网查询；本机登录只有本机记录。 */
  canRefresh: boolean;
  refreshing?: boolean;
  onRefresh?: () => void;
  /** 联网查询失败的说明。本机登录那一节不显示：那是托管账号的事。 */
  showError?: boolean;
}) {
  const { t, i18n } = useTranslation();
  const snap = source?.snapshot;
  const credits = snap ? creditsText(snap, i18n.language, t) : null;

  return (
    <div className="mt-2.5 flex flex-col gap-1.5 text-xs" role="group" aria-label={t("quota.windowsLabel", { label })}>
      {snap && (snap.windows.length > 0 || credits) && (
        <div className="grid grid-cols-[auto_minmax(4rem,10rem)_auto_minmax(0,1fr)] items-center gap-x-2.5 gap-y-1.5">
          {snap.windows.map((w) => {
            const level = windowLevel(w.usedPercent, w.reset);
            return (
              <div key={w.minutes} className="contents">
                <span className="text-muted-foreground">{windowLabel(w.minutes, t)}</span>
                <QuotaMeter usedPercent={w.usedPercent} reset={w.reset} className="w-full" />
                <span
                  className={cn(
                    "text-right font-mono font-semibold tabular-nums",
                    level === "warn" && "text-warning",
                    level === "out" && "text-destructive",
                  )}
                >
                  {w.reset ? "—" : percentText(w.usedPercent)}
                </span>
                <span className="min-w-0 truncate text-muted-foreground">{w.reset ? t("quota.resetDone") : resetText(w.resetsAt, now, t)}</span>
              </div>
            );
          })}
          {credits && (
            <div className="contents">
              <span className="text-muted-foreground">{t("quota.credits")}</span>
              <span />
              <span className="text-right font-mono font-semibold tabular-nums">{credits}</span>
              <span />
            </div>
          )}
        </div>
      )}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-muted-foreground">
        {snap ? (
          <span title={snap.origin === "local" ? t("quota.localHint") : undefined}>
            {t(snap.origin === "local" ? "quota.local" : "quota.network")} · {agoText(snap.fetchedAt, now, t)}
          </span>
        ) : (
          <span>{t(canRefresh ? "quota.noData" : "quota.noDataLocal")}</span>
        )}
        {snap?.limitReached && <span className="font-medium text-destructive">{t("quota.limitReached")}</span>}
        {canRefresh && onRefresh && (
          <button
            type="button"
            className="inline-flex items-center gap-1 rounded font-medium text-primary hover:underline disabled:opacity-60 disabled:no-underline"
            disabled={refreshing}
            aria-label={t("quota.refreshFor", { label })}
            onClick={onRefresh}
          >
            {refreshing ? <Loader2 className="h-3 w-3 animate-spin" /> : <RefreshCw className="h-3 w-3" />}
            {t(refreshing ? "quota.refreshing" : "quota.refresh")}
          </button>
        )}
      </div>
      {showError && source?.error && (
        <p role="status" className="flex items-center gap-1.5 text-warning">
          <AlertTriangle className="h-3.5 w-3.5 shrink-0" /> {errorText(source.error, t)}
        </p>
      )}
    </div>
  );
}
