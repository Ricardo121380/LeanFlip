// 一个额度来源的明细：每个窗口一行（已用比例、重置时间），余额、credits，数据来源和时间。
// ChatGPT 账号和中转站共用：中转站的窗口是金额窗口，另有余额和账户余额。
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Loader2, RefreshCw } from "lucide-react";
import type { QuotaSource } from "@/lib/api";
import { agoText, creditsText, errorText, meterWidth, moneyText, percentText, resetText, windowLabel, windowLevel } from "@/lib/quota";
import { cn } from "@/lib/utils";

/** 进度条：越满用得越多；80% 以上变黄，用完变红。 */
export function QuotaMeter({ usedPercent, reset, className }: { usedPercent: number; reset: boolean; className?: string }) {
  return (
    <span className={cn("quota-meter", className)} data-level={windowLevel(usedPercent, reset)} aria-hidden="true">
      <i style={{ width: meterWidth(usedPercent, reset) }} />
    </span>
  );
}

function Row({ name, meter, value, level = "ok", note }: { name: string; meter?: ReactNode; value: string; level?: "ok" | "warn" | "out"; note?: string | null }) {
  return (
    <div className="contents">
      <span className="text-muted-foreground">{name}</span>
      {meter ?? <span />}
      <span className={cn("text-right font-mono font-semibold tabular-nums", level === "warn" && "text-warning", level === "out" && "text-destructive")}>{value}</span>
      <span className="min-w-0 break-words text-muted-foreground">{note}</span>
    </div>
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
  emptyText,
}: {
  source: QuotaSource | undefined;
  /** 来源名，用在读屏标签里。 */
  label: string;
  now: number;
  /** 托管账号和中转站可以联网查询；本机登录只有本机记录。 */
  canRefresh: boolean;
  refreshing?: boolean;
  onRefresh?: () => void;
  /** 联网查询失败的说明。本机登录那一节不显示：那是托管账号的事。 */
  showError?: boolean;
  /** 还没有数据时的说明，默认按 ChatGPT 账号写。 */
  emptyText?: string;
}) {
  const { t, i18n } = useTranslation();
  const language = i18n.language;
  const snap = source?.snapshot;
  const credits = snap ? creditsText(snap, language, t) : null;
  const balance = snap?.balance;
  const money = (amount: number) => moneyText(amount, balance?.unit ?? "USD", language);
  const balanceUsed = balance && !balance.unlimited && balance.used !== undefined && balance.limit ? (balance.used / balance.limit) * 100 : undefined;
  const balanceOut = balance && !balance.unlimited && balance.remaining !== undefined && balance.remaining <= 0;
  const rows = snap && (snap.windows.length > 0 || credits || balance);

  return (
    <div className="mt-2.5 flex flex-col gap-1.5 text-xs" role="group" aria-label={t("quota.windowsLabel", { label })}>
      {rows && (
        <div className="grid grid-cols-[auto_minmax(4rem,10rem)_auto_minmax(0,1fr)] items-center gap-x-2.5 gap-y-1.5">
          {balance && (balance.unlimited || balance.remaining !== undefined || balance.limit !== undefined) && (
            <Row
              name={t(source?.adapter === "newapi" || balance.unlimited ? "quota.keyBalance" : "quota.balance")}
              meter={balanceUsed !== undefined ? <QuotaMeter usedPercent={balanceUsed} reset={false} className="w-full" /> : undefined}
              value={balance.unlimited ? t("quota.keyUnlimited") : balance.remaining !== undefined ? money(balance.remaining) : "—"}
              level={balanceOut ? "out" : balanceUsed !== undefined ? windowLevel(balanceUsed, false) : "ok"}
              note={balance.limit !== undefined && !balance.unlimited ? t("quota.ofTotal", { total: money(balance.limit) }) : null}
            />
          )}
          {snap.windows.map((w, index) => (
            <Row
              key={`${w.label ?? ""}:${w.minutes}:${index}`}
              name={w.label ? `${w.label} · ${windowLabel(w.minutes, t)}` : windowLabel(w.minutes, t)}
              meter={<QuotaMeter usedPercent={w.usedPercent} reset={w.reset} className="w-full" />}
              value={w.reset ? "—" : percentText(w.usedPercent)}
              level={windowLevel(w.usedPercent, w.reset)}
              note={[
                w.usedAmount !== undefined && w.limitAmount !== undefined ? `${moneyText(w.usedAmount, w.unit ?? balance?.unit ?? "USD", language)} / ${moneyText(w.limitAmount, w.unit ?? balance?.unit ?? "USD", language)}` : null,
                w.reset ? t("quota.resetDone") : resetText(w.resetsAt, now, t),
              ].filter(Boolean).join(" · ")}
            />
          ))}
          {balance?.accountRemaining !== undefined && <Row name={t("quota.accountBalance")} value={money(balance.accountRemaining)} level={snap.level === "out" && balance.accountRemaining <= 0 ? "out" : "ok"} />}
          {balance?.unlimited && balance.accountRemaining === undefined && <Row name={t("quota.accountBalance")} value={t("quota.balanceUnavailable")} />}
          {credits && <Row name={t("quota.credits")} value={credits} />}
        </div>
      )}
      {balance?.unlimited && balance.accountRemaining === undefined && <p className="leading-relaxed text-muted-foreground">{t("quota.accountBalanceHint")}</p>}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-muted-foreground">
        {snap ? (
          <span title={snap.origin === "local" ? t("quota.localHint") : undefined}>
            {t(snap.origin === "local" ? "quota.local" : "quota.network")} · {agoText(snap.fetchedAt, now, t)}
          </span>
        ) : (
          <span>{emptyText ?? t(canRefresh ? "quota.noData" : "quota.noDataLocal")}</span>
        )}
        {balance?.expiresAt && <span>{t("quota.expiresOn", { date: new Date(balance.expiresAt * 1000).toLocaleDateString(language) })}</span>}
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
