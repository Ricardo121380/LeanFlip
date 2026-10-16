import { describe, expect, it } from "vitest";
import i18n from "@/i18n";
import type { Provider, QuotaSnapshot, QuotaView } from "./api";
import { agoText, applyQuotaSettings, creditsText, errorText, meterWidth, moneyText, quotaBrief, quotaSettingsOf, resetText, sourceFor, sourcesByCard, windowLabel, windowLevel } from "./quota";

describe("home quota brief", () => {
  const t = i18n.getFixedT("zh");
  const at = Date.UTC(2026, 9, 15, 4, 0);
  const snap = (windows: QuotaSnapshot["windows"], balance?: QuotaSnapshot["balance"], level: QuotaSnapshot["level"] = "ok"): QuotaSnapshot => ({
    windows, balance, fetchedAt: at, origin: "network", level, limitReached: false,
  });
  const w = (minutes: number, usedPercent: number) => ({ minutes, usedPercent, resetsAt: Math.floor(at / 1000) + 3 * 86_400 + 60, reset: false });

  it("follows the tray rule: a nearly full window first, then the balance, then the fullest window", () => {
    expect(quotaBrief(snap([w(10080, 51)]), at, t, "zh")).toEqual({ text: "本周 51%", level: "ok", percent: 51, note: "3 天后重置" });
    const wallet = { unit: "USD", remaining: 31.4, limit: 50, used: 18.6, unlimited: false };
    expect(quotaBrief(snap([w(300, 30)], wallet), at, t, "zh")).toMatchObject({ text: "余 $31.40", percent: 37.2, note: "/ $50.00" });
    expect(quotaBrief(snap([w(300, 84)], wallet, "warn"), at, t, "zh")).toMatchObject({ text: "5 小时 84%", level: "warn" });
    expect(quotaBrief(snap([], { unit: "USD", unlimited: true }), at, t, "zh")?.text).toBe("不限");
    expect(quotaBrief(snap([]), at, t, "zh")).toBeNull();
  });

  it("maps every card to its source", () => {
    const view: QuotaView = {
      checkedAt: at,
      sources: [{ key: "relay:a", kind: "relay", users: [{ tool: "claude", providerId: "a", providerName: "A", active: true }, { tool: "opencode", providerId: "a2", providerName: "A", active: false }] }],
    };
    const map = sourcesByCard(view);
    expect(map.get("claude:a")?.key).toBe("relay:a");
    expect(map.get("opencode:a2")?.key).toBe("relay:a");
    expect(map.size).toBe(2);
  });
});

describe("relay quota helpers", () => {
  it("round-trips Krill settings and removes its JWT when the adapter changes", () => {
    const card: Provider = { id: "r", name: "R", settingsConfig: {} };
    const settings = { adapter: "krill" as const, refreshMinutes: 15, krillJwt: "test-login-jwt" };
    expect(quotaSettingsOf(applyQuotaSettings(card, settings))).toEqual(settings);
    expect(applyQuotaSettings(card, { ...settings, adapter: "sub2api" }).meta?.leanflipQuota).toEqual({ adapter: "sub2api", refreshMinutes: 15 });
  });

  it("formats dollars as currency and other units after the number", () => {
    expect(moneyText(31.4, "USD", "en")).toBe("$31.40");
    expect(moneyText(0, "USD", "zh")).toBe("$0.00");
    expect(moneyText(1234.5, "CNY", "en")).toBe("1,234.5 CNY");
  });

  it("reads and writes the card setting without touching other meta", () => {
    const card: Provider = { id: "r", name: "R", settingsConfig: {}, meta: { codexLoginRef: "x", leanflipQuota: { adapter: "newapi", refreshMinutes: 15, quotaPerUsd: 1e6 } } };
    expect(quotaSettingsOf(card)).toEqual({ adapter: "newapi", refreshMinutes: 15, quotaPerUsd: 1e6 });
    expect(quotaSettingsOf({ ...card, meta: { leanflipQuota: { adapter: "js", quotaPerUsd: -1 } } })).toEqual({ refreshMinutes: 0 });
    expect(quotaSettingsOf({ id: "r", name: "R", settingsConfig: {} })).toBeNull();

    expect(applyQuotaSettings(card, { adapter: "sub2api", refreshMinutes: 0 }).meta).toEqual({ codexLoginRef: "x", leanflipQuota: { adapter: "sub2api", refreshMinutes: 0 } });
    expect(applyQuotaSettings(card, { refreshMinutes: 0 }).meta).toEqual({ codexLoginRef: "x" });
    expect(applyQuotaSettings({ ...card, meta: { leanflipQuota: {} } }, null).meta).toBeUndefined();
  });
});

const zh = i18n.getFixedT("zh");
const en = i18n.getFixedT("en");
const NOW = Date.UTC(2026, 9, 15, 4, 0);
const at = (ms: number) => Math.floor((NOW + ms) / 1000);

const snapshot = (credits: QuotaSnapshot["credits"]): QuotaSnapshot => ({
  windows: [], credits, fetchedAt: NOW, origin: "local", level: "ok", limitReached: false,
});

describe("quota text", () => {
  it("names windows by their length, not their position", () => {
    expect(windowLabel(10080, zh)).toBe("本周");
    expect(windowLabel(300, zh)).toBe("5 小时");
    expect(windowLabel(1440, zh)).toBe("1 天");
    expect(windowLabel(90, zh)).toBe("90 分钟");
    expect(windowLabel(300, en)).toBe("5 h");
  });

  it("says how long until a window resets", () => {
    expect(resetText(at((3 * 24 + 4) * 3_600_000), NOW, zh)).toBe("3 天 4 小时后重置");
    expect(resetText(at(72 * 60_000), NOW, zh)).toBe("1 小时 12 分后重置");
    expect(resetText(at(2 * 3_600_000), NOW, zh)).toBe("2 小时后重置");
    expect(resetText(at(20_000), NOW, zh)).toBe("1 分钟后重置");
    expect(resetText(at(-1000), NOW, zh)).toBe("已重置，下次使用后更新");
    expect(resetText(undefined, NOW, zh)).toBeNull();
    expect(resetText(at(5 * 86_400_000), NOW, en)).toBe("Resets in 5 d");
  });

  it("says how old the data is", () => {
    expect(agoText(NOW - 10_000, NOW, zh)).toBe("刚刚");
    expect(agoText(NOW - 42 * 60_000, NOW, zh)).toBe("42 分钟前");
    expect(agoText(NOW - 5 * 3_600_000, NOW, zh)).toBe("5 小时前");
    expect(agoText(NOW - 2 * 86_400_000, NOW, en)).toBe("2 d ago");
  });

  it("formats credits and keeps small balances precise", () => {
    expect(creditsText(snapshot({ hasCredits: true, unlimited: false, balance: 60951.5625 }), "en", en)).toBe("60,952");
    expect(creditsText(snapshot({ hasCredits: true, unlimited: false, balance: 18.7907 }), "en", en)).toBe("18.79");
    expect(creditsText(snapshot({ hasCredits: true, unlimited: true }), "zh", zh)).toBe("不限");
    expect(creditsText(snapshot(undefined), "zh", zh)).toBeNull();
  });

  it("explains failures and falls back for unknown codes", () => {
    expect(errorText({ code: "unauthorized", status: 401, at: NOW }, zh)).toBe("登录已失效（401），重新登录后再查");
    expect(errorText({ code: "something-new", status: 502, at: NOW }, zh)).toBe("查询失败（HTTP 502）");
  });

  it("draws reset windows empty and uses the same thresholds as the backend", () => {
    expect(meterWidth(140, false)).toBe("100%");
    expect(meterWidth(60, true)).toBe("0%");
    expect([windowLevel(79, false), windowLevel(80, false), windowLevel(100, false), windowLevel(100, true)]).toEqual(["ok", "warn", "out", "ok"]);
  });

  it("finds the source for a managed account or a local login", () => {
    const view: QuotaView = {
      checkedAt: NOW,
      sources: [
        { key: "a", kind: "codex", accountId: "acc-1", users: [] },
        { key: "b", kind: "codex", login: "sub:x|account:y", users: [] },
      ],
    };
    expect(sourceFor(view, { accountId: "acc-1" })?.key).toBe("a");
    expect(sourceFor(view, { login: "sub:x|account:y" })?.key).toBe("b");
    expect(sourceFor(view, { accountId: "missing" })).toBeUndefined();
    expect(sourceFor(undefined, { accountId: "acc-1" })).toBeUndefined();
  });
});
