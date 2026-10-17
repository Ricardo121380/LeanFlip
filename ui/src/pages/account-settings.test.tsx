import { beforeEach, describe, expect, it, vi } from "vitest";
import { StrictMode } from "react";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { toast } from "sonner";
import i18n from "@/i18n";
import { keys } from "@/hooks/queries";
import type { SettingsView } from "@/lib/api";
import { fakeBackend, type Handler } from "@/test/utils";

const backend: { invoke: ReturnType<typeof vi.fn> | null } = { invoke: null };
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => backend.invoke!(cmd, args),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn() } }));

import { SettingsPage } from "./SettingsPage";
import { CodexAccountsPage } from "./CodexAccountsPage";

function mount(element: React.ReactElement, handlers: Record<string, Handler>) {
  const fake = fakeBackend(handlers);
  backend.invoke = fake.invoke;
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  render(<QueryClientProvider client={client}>{element}</QueryClientProvider>);
  return { ...fake, client };
}

function settings(): SettingsView {
  return {
    app: { language: "zh", dirOverrides: {}, autostart: false, backgroundRefreshHours: 168, closeToTray: true },
    keepCodexLogin: false,
    dataDir: "/test/.leanflip",
    home: "/test",
    livePaths: {},
    dirs: { codexDir: "/test/.codex" },
    ccswitchDir: "/test/.cc-switch",
  };
}

const account = { id: "account-1", email: "me@example.com", workspace: null, plan: "plus", authenticatedAt: 1, reauthRequired: false, isDefault: false };

beforeEach(async () => {
  vi.clearAllMocks();
  await i18n.changeLanguage("zh");
});

describe("Settings editing states", () => {
  it("accepts pristine backend refreshes without inventing unsaved changes under StrictMode", async () => {
    let stored = settings();
    const { client } = mount(<StrictMode><SettingsPage /></StrictMode>, { get_settings: () => stored });
    expect(await screen.findByRole("spinbutton")).toHaveValue(168);
    stored = { ...stored, app: { ...stored.app, backgroundRefreshHours: 24, autostart: true } };
    await act(async () => { await client.invalidateQueries({ queryKey: keys.settings }); });
    await waitFor(() => expect(screen.getByRole("spinbutton")).toHaveValue(24));
    expect(screen.getByRole("switch", { name: "开机自动启动" })).toBeChecked();
    expect(screen.getByRole("status")).toHaveTextContent("没有未保存的修改");
    expect(screen.getByRole("button", { name: "保存设置" })).toBeDisabled();
  });

  it("shows a load failure with retry instead of loading forever", async () => {
    let failed = true;
    mount(<SettingsPage />, { get_settings: () => { if (failed) throw new Error("settings read failed"); return settings(); } });
    expect(await screen.findByRole("alert")).toHaveTextContent("无法读取设置");
    expect(screen.getByRole("alert")).toHaveTextContent("settings read failed");
    failed = false;
    await userEvent.click(screen.getByRole("button", { name: "重试" }));
    expect(await screen.findByRole("button", { name: "保存设置" })).toBeDisabled();
  });

  it("preserves an unsaved draft through backend refreshes and resets to the latest settings", async () => {
    let stored = settings();
    const { client, calls } = mount(<StrictMode><SettingsPage /></StrictMode>, { get_settings: () => stored });
    expect(await screen.findByRole("button", { name: "保存设置" })).toBeDisabled();
    await userEvent.type(screen.getByLabelText("Codex (~/.codex)"), "/my/draft");
    expect(screen.getByRole("status")).toHaveTextContent("有未保存的修改");
    stored = { ...stored, app: { ...stored.app, dirOverrides: { codexDir: "/external/change" }, autostart: true } };
    await act(async () => { await client.invalidateQueries({ queryKey: keys.settings }); });
    expect(screen.getByLabelText("Codex (~/.codex)")).toHaveValue("/my/draft");
    expect(screen.getByRole("button", { name: "保存设置" })).toBeEnabled();
    expect(calls.some((call) => call.cmd === "save_settings")).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "放弃修改" }));
    expect(screen.getByLabelText("Codex (~/.codex)")).toHaveValue("/external/change");
    expect(screen.getByRole("switch", { name: "开机自动启动" })).toBeChecked();
    expect(screen.getByRole("button", { name: "保存设置" })).toBeDisabled();
  });

  it("validates refresh hours without turning invalid input into an accidental zero", async () => {
    const { calls } = mount(<SettingsPage />, { get_settings: settings });
    const hours = await screen.findByRole("spinbutton");
    for (const value of ["", "-1", "1.5", "4294967296"]) {
      fireEvent.change(hours, { target: { value } });
      expect(hours).toHaveAttribute("aria-invalid", "true");
      expect(screen.getByRole("alert")).toHaveTextContent("整数小时数");
      expect(screen.getByRole("button", { name: "保存设置" })).toBeDisabled();
    }
    fireEvent.change(hours, { target: { value: "0" } });
    expect(hours).toHaveAttribute("aria-invalid", "false");
    expect(screen.getByRole("button", { name: "保存设置" })).toBeEnabled();
    expect(calls.some((call) => call.cmd === "save_settings")).toBe(false);
  });

  it("locks conflicting edits while saving and applies the submitted language only on success", async () => {
    let stored = settings();
    let finish!: () => void;
    const pending = new Promise<void>((resolve) => { finish = resolve; });
    const { calls } = mount(<SettingsPage />, {
      get_settings: () => stored,
      save_settings: async (args) => {
        await pending;
        const input = args!.input as Pick<SettingsView, "app" | "keepCodexLogin">;
        stored = { ...stored, ...input };
        return { dirsChanged: false };
      },
    });
    await screen.findByRole("button", { name: "保存设置" });
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "界面语言" }), "en");
    expect(i18n.language).toBe("zh");
    await userEvent.click(screen.getByRole("button", { name: "保存设置" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "正在保存…" })).toBeDisabled());
    expect(screen.getByRole("combobox", { name: "界面语言" })).toBeDisabled();
    expect(screen.getByRole("switch", { name: "开机自动启动" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "放弃修改" })).toBeDisabled();
    expect(toast.success).not.toHaveBeenCalled();
    await act(async () => finish());
    expect(await screen.findByRole("button", { name: "Save settings" })).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent("No unsaved changes");
    expect((calls.find((call) => call.cmd === "save_settings")!.args!.input as SettingsView).app.language).toBe("en");
    expect(toast.success).toHaveBeenCalledTimes(1);
  });

  it("retains changes after a failed save and never announces success", async () => {
    mount(<StrictMode><SettingsPage /></StrictMode>, {
      get_settings: settings,
      save_settings: () => { throw new Error("write denied"); },
    });
    await screen.findByRole("button", { name: "保存设置" });
    await userEvent.type(screen.getByLabelText("Codex (~/.codex)"), "/my/draft");
    await userEvent.click(screen.getByRole("button", { name: "保存设置" }));
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("write denied"));
    expect(screen.getByLabelText("Codex (~/.codex)")).toHaveValue("/my/draft");
    expect(screen.getByRole("button", { name: "保存设置" })).toBeEnabled();
    expect(screen.getByRole("status")).toHaveTextContent("有未保存的修改");
    expect(toast.success).not.toHaveBeenCalled();
  });
});

describe("Menu-bar usage setting", () => {
  it("is off by default and saves the switch with the other app settings", async () => {
    const { calls } = mount(<SettingsPage />, {
      get_settings: () => settings(),
      save_settings: () => ({ dirsChanged: false }),
    });
    const toggle = await screen.findByRole("switch", { name: "在状态栏显示额度" });
    expect(toggle).not.toBeChecked();
    expect(toggle).toHaveAccessibleDescription(/macOS 上会显示/);
    await userEvent.click(toggle);
    await userEvent.click(screen.getByRole("button", { name: "保存设置" }));
    await waitFor(() => expect(calls.some((call) => call.cmd === "save_settings")).toBe(true));
    expect((calls.find((call) => call.cmd === "save_settings")!.args!.input as SettingsView).app.trayQuotaTitle).toBe(true);
  });
});

describe("Account feedback", () => {
  it("distinguishes failed account and local-login queries from empty data, and retries separately", async () => {
    let failed = true;
    mount(<CodexAccountsPage />, {
      codex_accounts: () => { if (failed) throw new Error("account read denied"); return []; },
      codex_logins: () => { throw new Error("local read denied"); },
    });
    await screen.findByText("无法读取托管账号");
    expect(screen.getByText("无法读取本机登录")).toBeInTheDocument();
    expect(screen.queryByText("还没有托管账号")).not.toBeInTheDocument();
    expect(screen.queryByText("没有发现本机登录。")).not.toBeInTheDocument();
    failed = false;
    const accountAlert = screen.getAllByRole("alert").find((alert) => alert.textContent?.includes("account read denied"))!;
    await userEvent.click(within(accountAlert).getByRole("button", { name: "重试" }));
    expect(await screen.findByText("还没有托管账号")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "登录新账号" })).toBeEnabled();
    expect(screen.getByText("无法读取本机登录")).toBeInTheDocument();
  });

  it("disables conflicting account controls and announces the pending operation", async () => {
    let finish!: () => void;
    const pending = new Promise<void>((resolve) => { finish = resolve; });
    mount(<CodexAccountsPage />, { codex_accounts: () => [account], codex_logins: () => [], codex_set_default: () => pending });
    await userEvent.click(await screen.findByRole("button", { name: "将 me@example.com 设为默认账号" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "移除 me@example.com" })).toBeDisabled());
    expect(screen.getByRole("button", { name: "重新登录 me@example.com" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "登录新账号" })).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent("正在设置默认账号…");
    await act(async () => finish());
    await waitFor(() => expect(screen.getByRole("button", { name: "登录新账号" })).toBeEnabled());
    expect(toast.success).toHaveBeenCalledWith("默认账号已更新");
  });

  it("reports real clipboard and browser failures during device sign-in", async () => {
    mount(<CodexAccountsPage />, {
      codex_accounts: () => [],
      codex_logins: () => [],
      codex_start_login: () => ({ deviceCode: "device", userCode: "ABCD-1234", verificationUri: "https://auth.openai.com/codex/device", expiresIn: 900, interval: 60 }),
      codex_cancel_login: () => true,
      open_url: () => { throw new Error("browser denied"); },
    });
    await userEvent.click(await screen.findByRole("button", { name: "登录新账号" }));
    await screen.findByTestId("user-code");
    const writeText = vi.fn().mockRejectedValue(new Error("clipboard denied"));
    const previous = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    try {
      await userEvent.click(screen.getByRole("button", { name: "复制登录代码" }));
      await waitFor(() => expect(toast.error).toHaveBeenCalledWith("复制失败，请手动选择并复制登录代码。"));
      expect(toast.success).not.toHaveBeenCalled();
      await userEvent.click(screen.getByRole("button", { name: "打开登录页面" }));
      await waitFor(() => expect(toast.error).toHaveBeenCalledWith(expect.stringContaining("browser denied")));
    } finally {
      if (previous) Object.defineProperty(navigator, "clipboard", previous);
      else Reflect.deleteProperty(navigator, "clipboard");
    }
  });
});

describe("Account quota", () => {
  const now = Date.now();
  const view = (used: number, fetchedAt: number, error?: { code: string; status?: number; at: number }) => ({
    checkedAt: now,
    sources: [
      {
        key: "codex:ws:user",
        kind: "codex",
        accountId: "account-1",
        users: [],
        snapshot: {
          windows: [{ minutes: 10080, usedPercent: used, resetsAt: Math.floor(now / 1000) + 3 * 86_400 + 60, reset: false }],
          credits: { hasCredits: true, unlimited: false, balance: 18.79 },
          fetchedAt,
          origin: "network",
          level: used >= 80 ? "warn" : "ok",
          usedFraction: used / 100,
          limitReached: false,
        },
        error,
      },
    ],
  });

  it("shows each window by its length with reset time, credits and data age", async () => {
    mount(<CodexAccountsPage />, { codex_accounts: () => [account], codex_logins: () => [], quota_overview: () => view(51, now - 3 * 60_000) });
    const quota = await screen.findByRole("group", { name: "me@example.com的额度" });
    expect(within(quota).getByText("本周")).toBeInTheDocument();
    expect(within(quota).getByText("51%")).toBeInTheDocument();
    expect(within(quota).getByText("3 天后重置")).toBeInTheDocument();
    expect(within(quota).getByText("credits 余额")).toBeInTheDocument();
    expect(within(quota).getByText("18.79")).toBeInTheDocument();
    expect(within(quota).getByText("联网查询 · 3 分钟前")).toBeInTheDocument();
  });

  it("refreshes one account online and shows a newer failure without dropping old data", async () => {
    const { calls } = mount(<CodexAccountsPage />, {
      codex_accounts: () => [account],
      codex_logins: () => [],
      quota_overview: () => view(51, now - 3 * 60_000),
      quota_refresh: () => view(51, now - 3 * 60_000, { code: "unauthorized", status: 401, at: Date.now() }),
    });
    expect(calls.some((c) => c.cmd === "quota_refresh")).toBe(false);
    await userEvent.click(await screen.findByRole("button", { name: "刷新 me@example.com 的额度" }));
    expect(await screen.findByText("登录已失效（401），重新登录后再查")).toBeInTheDocument();
    expect(calls.find((c) => c.cmd === "quota_refresh")?.args).toEqual({ keys: ["codex:ws:user"] });
    expect(screen.getByText("51%")).toBeInTheDocument();
  });

  it("refreshes every account from the header and never queries online on its own", async () => {
    const { calls } = mount(<CodexAccountsPage />, {
      codex_accounts: () => [account],
      codex_logins: () => [],
      quota_overview: () => ({ checkedAt: now, sources: [{ key: "k", kind: "codex", accountId: "account-1", users: [] }] }),
      quota_refresh: () => view(92, Date.now()),
    });
    expect(await screen.findByText("最近 8 天没有这个账号的记录。点“刷新额度”联网查询。")).toBeInTheDocument();
    expect(calls.some((c) => c.cmd === "quota_refresh")).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "刷新额度" }));
    expect(await screen.findByText("92%")).toHaveClass("text-warning");
    expect(calls.find((c) => c.cmd === "quota_refresh")?.args).toEqual({ keys: null });
    expect(screen.getByRole("button", { name: "额度 · 刚刚" })).toBeEnabled();
  });
});

it("changes Claude onboarding only after reviewing and confirming its exact path", async () => {
  const { calls } = mount(<SettingsPage />, {
    get_settings: () => settings(),
    claude_onboarding_path: () => "/test/.claude.json",
    complete_claude_onboarding: () => ({ changed: ["/test/.claude.json"], warnings: [] }),
  });
  const button = await screen.findByRole("button", { name: "标记首次启动已完成" });
  expect(calls.some((call) => call.cmd === "complete_claude_onboarding")).toBe(false);
  await userEvent.click(button);
  let dialog = await screen.findByRole("alertdialog");
  expect(dialog).toHaveTextContent("/test/.claude.json");
  await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
  expect(calls.some((call) => call.cmd === "complete_claude_onboarding")).toBe(false);
  await userEvent.click(button);
  dialog = await screen.findByRole("alertdialog");
  await userEvent.click(within(dialog).getByRole("button", { name: "标记首次启动已完成" }));
  await waitFor(() => expect(calls.find((call) => call.cmd === "complete_claude_onboarding")?.args).toEqual({ expectedPath: "/test/.claude.json" }));
});
