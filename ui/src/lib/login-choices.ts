// Codex 官方卡“登录方式”的选项。同一个 ChatGPT 账号可能同时是本机登录和托管账号，
// 所以每个选项都写明会用到谁、凭据从哪来，不显示工作区 UUID 这类内部标识。
import type { TFunction } from "i18next";
import type { AccountInfo, LoginSummary } from "./api";
import { planBadge } from "./cards";

export interface LoginChoice {
  value: string;
  label: string;
}

export interface LoginChoices {
  native: LoginChoice;
  local: LoginChoice[];
  managed: LoginChoice[];
  /** 卡上存的登录已经不在了：保留这个值，免得下拉框悄悄显示成别的选项。 */
  missing?: LoginChoice;
}

function short(id: string): string {
  return id.length > 10 ? `${id.slice(0, 8)}…` : id;
}

/** 邮箱有重复时才带上工作区，用来区分同一个人的不同工作区。 */
function describe(
  who: { email: string | null; workspace: string | null; plan: string | null },
  duplicated: boolean,
  extra: (string | null)[],
  fallback: string,
  t: TFunction,
): string {
  return [
    who.email ?? (who.workspace ? t("editor.loginWorkspace", { id: short(who.workspace) }) : fallback),
    duplicated && who.email && who.workspace ? t("editor.loginWorkspace", { id: short(who.workspace) }) : null,
    planBadge(who.plan)?.label ?? null,
    ...extra,
  ]
    .filter(Boolean)
    .join(" · ");
}

function duplicates(emails: (string | null)[]): Set<string> {
  const seen = new Set<string>();
  const twice = new Set<string>();
  for (const email of emails) {
    if (!email) continue;
    if (seen.has(email)) twice.add(email);
    seen.add(email);
  }
  return twice;
}

export function loginChoices(logins: LoginSummary[], accounts: AccountInfo[], current: string, t: TFunction): LoginChoices {
  const live = logins.find((l) => l.live);
  const sameLocal = duplicates(logins.map((l) => l.email));
  const sameManaged = duplicates(accounts.map((a) => a.email));
  const local = logins.map((l) => ({
    value: `ref:${l.identity}`,
    label: describe(l, Boolean(l.email && sameLocal.has(l.email)), [l.live ? t("editor.loginLive") : null], short(l.identity), t),
  }));
  const account = (a: AccountInfo) =>
    describe(a, Boolean(a.email && sameManaged.has(a.email)), [a.reauthRequired ? t("editor.loginReauth") : null], short(a.id), t);
  const fallback = accounts.find((a) => a.isDefault);
  // 没有托管账号时不提供“默认账号”，除非这张卡已经选了它。
  const managed =
    accounts.length === 0 && current !== "managed:"
      ? []
      : [
          {
            value: "managed:",
            label: fallback ? t("editor.loginManagedDefaultWho", { label: account(fallback) }) : t("editor.loginManagedDefault"),
          },
          ...accounts.map((a) => ({ value: `managed:${a.id}`, label: account(a) })),
        ];
  const native = {
    value: "native",
    label: live ? t("editor.loginNativeWho", { label: live.email ?? t("editor.loginNoEmail") }) : t("editor.loginNative"),
  };
  const known = [native, ...local, ...managed].some((c) => c.value === current);
  const missing = known
    ? undefined
    : { value: current, label: t(current.startsWith("ref:") ? "editor.loginMissingRef" : "editor.loginMissingManaged") };
  return { native, local, managed, missing };
}

/** 选中项下面的一句说明：切换时凭据从哪来。 */
export function loginHelp(current: string, choices: LoginChoices, t: TFunction): string {
  if (choices.missing?.value === current) return t("editor.loginHelpMissing");
  if (current === "native") return t("editor.loginHelpNative");
  if (current === "managed:") return t("editor.loginHelpManagedDefault");
  if (current.startsWith("managed:")) return t("editor.loginHelpManaged");
  return t("editor.loginHelpRef");
}
