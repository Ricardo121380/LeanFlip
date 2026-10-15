// 额度的文字：窗口名称、重置时间、数据时间、错误说明。数值和等级由后端算好。
import type { TFunction } from "i18next";
import type { QuotaError, QuotaSnapshot, QuotaSource, QuotaView } from "./api";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** 按窗口长度命名：周窗口叫“本周”，其余按天 / 小时。 */
export function windowLabel(minutes: number, t: TFunction): string {
  if (minutes === 7 * 24 * 60) return t("quota.week");
  if (minutes % (24 * 60) === 0) return t("quota.days", { count: minutes / (24 * 60) });
  if (minutes % 60 === 0) return t("quota.hours", { count: minutes / 60 });
  return t("quota.minutes", { count: minutes });
}

function duration(ms: number, t: TFunction): string {
  const d = Math.floor(ms / DAY);
  const h = Math.floor((ms % DAY) / HOUR);
  const m = Math.floor((ms % HOUR) / MINUTE);
  if (d > 0) return h > 0 ? t("quota.durDaysHours", { d, h }) : t("quota.durDays", { d });
  if (h > 0) return m > 0 ? t("quota.durHoursMinutes", { h, m }) : t("quota.durHours", { h });
  return t("quota.durMinutes", { m: Math.max(1, Math.ceil(ms / MINUTE)) });
}

/** “3 天 4 小时后重置”。`resetsAt` 是 Unix 秒。 */
export function resetText(resetsAt: number | undefined, now: number, t: TFunction): string | null {
  if (resetsAt === undefined) return null;
  const left = resetsAt * 1000 - now;
  if (left <= 0) return t("quota.resetDone");
  return t("quota.resetsIn", { time: duration(left, t) });
}

/** 数据有多旧：“刚刚”“3 分钟前”。 */
export function agoText(at: number, now: number, t: TFunction): string {
  const ms = Math.max(0, now - at);
  if (ms < MINUTE) return t("quota.justNow");
  if (ms < HOUR) return t("quota.minutesAgo", { count: Math.floor(ms / MINUTE) });
  if (ms < DAY) return t("quota.hoursAgo", { count: Math.floor(ms / HOUR) });
  return t("quota.daysAgo", { count: Math.floor(ms / DAY) });
}

export function percentText(value: number): string {
  return `${Math.round(value)}%`;
}

export function creditsText(snapshot: QuotaSnapshot, language: string, t: TFunction): string | null {
  const credits = snapshot.credits;
  if (!credits) return null;
  if (credits.unlimited) return t("quota.unlimited");
  if (credits.balance === undefined) return null;
  return new Intl.NumberFormat(language, { maximumFractionDigits: credits.balance < 100 ? 2 : 0 }).format(credits.balance);
}

export function errorText(error: QuotaError, t: TFunction): string {
  const known = ["reauth", "unauthorized", "rateLimited", "http", "timeout", "network", "format", "token"];
  return t(`quota.error.${known.includes(error.code) ? error.code : "http"}`, { status: error.status ?? "?" });
}

/** 托管账号或本机登录对应的额度来源。 */
export function sourceFor(view: QuotaView | undefined, match: { accountId?: string; login?: string }): QuotaSource | undefined {
  return view?.sources.find((s) => (match.accountId && s.accountId === match.accountId) || (match.login && s.login === match.login));
}

/** 宽度用的已用比例；已重置的窗口按 0 画。 */
export function meterWidth(usedPercent: number, reset: boolean): string {
  return reset ? "0%" : `${Math.min(100, Math.max(0, usedPercent))}%`;
}

/** 单个窗口的等级，和后端的阈值一致。 */
export function windowLevel(usedPercent: number, reset: boolean): "ok" | "warn" | "out" {
  if (reset) return "ok";
  if (usedPercent >= 100) return "out";
  return usedPercent >= 80 ? "warn" : "ok";
}
