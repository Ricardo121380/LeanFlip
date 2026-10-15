import { describe, expect, it } from "vitest";
import i18n from "@/i18n";
import type { AccountInfo, LoginSummary } from "./api";
import { loginChoices, loginHelp } from "./login-choices";

const t = i18n.getFixedT("zh");
const workspace = "baba242c-83d7-4d86-8f93-b56e249fa5f9";

const login = (over: Partial<LoginSummary> = {}): LoginSummary => ({
  identity: `sub:me|account:${workspace}`, email: "me@example.com", workspace, plan: "pro", live: true, ...over,
});
const account = (over: Partial<AccountInfo> = {}): AccountInfo => ({
  id: "acc-1", email: "me@example.com", workspace, plan: "pro", authenticatedAt: 1, reauthRequired: false, isDefault: true, ...over,
});

describe("login choices", () => {
  it("names who each option means when one person is both a local login and a managed account", () => {
    const choices = loginChoices([login()], [account()], "native", t);
    expect(choices.native.label).toBe("跟随本机当前登录（me@example.com）");
    expect(choices.local.map((c) => c.label)).toEqual(["me@example.com · Pro · 当前生效"]);
    expect(choices.managed.map((c) => [c.value, c.label])).toEqual([
      ["managed:", "默认账号（me@example.com · Pro）"],
      ["managed:acc-1", "me@example.com · Pro"],
    ]);
    const labels = [choices.native, ...choices.local, ...choices.managed].map((c) => c.label);
    expect(labels.join("\n")).not.toContain("baba242c");
    expect(choices.missing).toBeUndefined();
  });

  it("adds a short workspace only to tell apart the same email", () => {
    const other = "ffff0000-1111-2222-3333-444455556666";
    const choices = loginChoices(
      [login(), login({ identity: `sub:me|account:${other}`, workspace: other, plan: "team", live: false })],
      [account({ reauthRequired: true })],
      "native",
      t,
    );
    expect(choices.local.map((c) => c.label)).toEqual([
      "me@example.com · 工作区 baba242c… · Pro · 当前生效",
      "me@example.com · 工作区 ffff0000… · Team",
    ]);
    expect(choices.managed[1].label).toBe("me@example.com · Pro · 需要重新登录");
  });

  it("keeps a stored login that is gone instead of silently showing another option", () => {
    const ref = loginChoices([login()], [], "ref:sub:gone|account:x", t);
    expect(ref.missing).toEqual({ value: "ref:sub:gone|account:x", label: "本机登录（已不在本机）" });
    expect(loginHelp("ref:sub:gone|account:x", ref, t)).toBe("选中的登录已经不在了，切换前换一个。");
    const managed = loginChoices([], [account()], "managed:acc-removed", t);
    expect(managed.missing?.label).toBe("托管账号（已移除）");
  });

  it("offers the default managed account only when there is one or the card already uses it", () => {
    expect(loginChoices([], [], "native", t).managed).toEqual([]);
    expect(loginChoices([], [], "managed:", t).managed).toEqual([{ value: "managed:", label: "默认账号" }]);
    expect(loginChoices([login({ live: false })], [], "native", t).native.label).toBe("跟随本机当前登录");
  });

  it("explains where the credentials come from for the selected option", () => {
    const choices = loginChoices([login()], [account()], "native", t);
    expect(loginHelp("native", choices, t)).toContain("~/.codex/auth.json");
    expect(loginHelp(`ref:${login().identity}`, choices, t)).toBe("切换时固定还原这份本机登录，不管 Codex 当前登录的是谁。");
    expect(loginHelp("managed:", choices, t)).toContain("默认托管账号");
    expect(loginHelp("managed:acc-1", choices, t)).toContain("token 由 LeanFlip 保存和刷新");
  });
});
