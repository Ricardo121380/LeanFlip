import { describe, expect, it, vi, beforeEach } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { fakeBackend, renderWithClient, type Handler } from "@/test/utils";

const backend: { invoke: ReturnType<typeof vi.fn> | null } = { invoke: null };
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: Record<string, unknown>, options?: { headers?: Record<string, string> }) =>
    backend.invoke!(cmd, options?.headers ? { body: args, headers: options.headers } : args),
}));

import { ToolPage } from "./ToolPage";
import { CodexAccountsPage } from "./CodexAccountsPage";
import { ImportWizard } from "./ImportWizard";
import { SettingsPage, cleanOverrides } from "./SettingsPage";

function use(handlers: Record<string, Handler>) {
  const fake = fakeBackend(handlers);
  backend.invoke = fake.invoke;
  return fake;
}

const accounts = [
  { id: "acc-plus", email: "me@example.com", workspace: "Personal", plan: "plus", authenticatedAt: 1, reauthRequired: false, isDefault: true },
  { id: "acc-team", email: "me@corp.com", workspace: "Corp", plan: "team", authenticatedAt: 1, reauthRequired: true, isDefault: false },
];

beforeEach(() => {
  backend.invoke = null;
});

describe("ToolPage (switch tool)", () => {
  it("shows cards with the current one marked and switches on click", async () => {
    let current = "official";
    const fake = use({
      list_providers: () => ({
        tool: "codex",
        kind: "switch",
        current,
        inLive: [],
        liveError: null,
        paths: ["/tmp/home/.codex/config.toml"],
        providers: [
          { id: "official", name: "OpenAI", category: "official", settingsConfig: { auth: {}, config: "" }, meta: { authBinding: { authProvider: "codex_oauth", accountId: "acc-team" } } },
          { id: "relay", name: "Relay", settingsConfig: { auth: { OPENAI_API_KEY: "sk" }, config: 'base_url = "https://relay/v1"\n' } },
        ],
      }),
      codex_accounts: () => accounts,
      codex_logins: () => [],
      switch_provider: (args) => {
        current = String(args!.id);
        return { changed: [], warnings: ["auth.json 读不了"] };
      },
    });
    renderWithClient(<ToolPage tool="codex" />);

    const official = await screen.findByTestId("card-official");
    expect(within(official).getByText("当前")).toBeInTheDocument();
    expect(within(official).getByText("官方")).toBeInTheDocument();
    expect(within(official).getByText("me@corp.com")).toBeInTheDocument();
    const relay = screen.getByTestId("card-relay");
    expect(within(relay).getByText("https://relay/v1")).toBeInTheDocument();
    expect(within(official).getByRole("button", { name: /切换/ })).toBeDisabled();

    await userEvent.click(within(relay).getByRole("button", { name: /切换/ }));
    await waitFor(() => expect(fake.calls.some((c) => c.cmd === "switch_provider")).toBe(true));
    expect(fake.calls.find((c) => c.cmd === "switch_provider")!.args).toEqual({ tool: "codex", id: "relay" });
    await waitFor(() => expect(within(screen.getByTestId("card-relay")).getByText("当前")).toBeInTheDocument());
  });
});

describe("ToolPage (coexist tool)", () => {
  it("toggles entries in and out of the live config", async () => {
    const inLive = new Set(["a"]);
    const fake = use({
      list_providers: () => ({
        tool: "opencode",
        kind: "coexist",
        current: null,
        inLive: [...inLive],
        liveError: null,
        paths: [],
        providers: [
          { id: "a", name: "A", settingsConfig: { options: { baseURL: "https://a" } } },
          { id: "b", name: "B", settingsConfig: {} },
        ],
      }),
      codex_accounts: () => [],
      set_in_live: (args) => {
        if (args!.enabled) inLive.add(String(args!.id));
        else inLive.delete(String(args!.id));
        return { changed: [], warnings: [] };
      },
    });
    renderWithClient(<ToolPage tool="opencode" />);
    const toggleA = await screen.findByRole("switch", { name: "toggle a" });
    const toggleB = screen.getByRole("switch", { name: "toggle b" });
    expect(toggleA).toBeChecked();
    expect(toggleB).not.toBeChecked();
    await userEvent.click(toggleB);
    await waitFor(() => expect(screen.getByRole("switch", { name: "toggle b" })).toBeChecked());
    expect(fake.calls.find((c) => c.cmd === "set_in_live")!.args).toEqual({ tool: "opencode", id: "b", enabled: true });
  });

  it("disables toggles when the live config cannot be read", async () => {
    use({
      list_providers: () => ({
        tool: "opencode",
        kind: "coexist",
        current: null,
        inLive: [],
        liveError: "bad json",
        paths: [],
        providers: [{ id: "a", name: "A", settingsConfig: {} }],
      }),
      codex_accounts: () => [],
    });
    renderWithClient(<ToolPage tool="opencode" />);
    expect(await screen.findByText(/bad json/)).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "toggle a" })).toBeDisabled();
  });
});

describe("ProviderEditor via ToolPage", () => {
  it("adds a card from a preset with a slug id", async () => {
    const fake = use({
      list_providers: () => ({ tool: "claude", kind: "switch", current: null, inLive: [], liveError: null, paths: [], providers: [] }),
      codex_accounts: () => [],
      add_provider: () => ({ changed: [], warnings: [] }),
    });
    renderWithClient(<ToolPage tool="claude" />);
    await userEvent.click(await screen.findByRole("button", { name: /添加供应商/ }));
    await userEvent.type(await screen.findByPlaceholderText("搜索预设…"), "Kimi");
    const option = (await screen.findAllByRole("option"))[0];
    await userEvent.click(option);
    expect((screen.getByLabelText("ID") as HTMLInputElement).value).toMatch(/^kimi/);
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(fake.calls.some((c) => c.cmd === "add_provider")).toBe(true));
    const args = fake.calls.find((c) => c.cmd === "add_provider")!.args as { provider: { id: string; settingsConfig: { env: Record<string, string> } }; addToLive: boolean };
    expect(args.provider.id).toMatch(/^kimi/);
    expect(args.provider.settingsConfig.env.ANTHROPIC_BASE_URL).toMatch(/^https:\/\//);
    expect(args.addToLive).toBe(false);
  });

  it("edits the Claude Desktop model menu visually and blocks invalid ids", async () => {
    const fake = use({
      list_providers: () => ({ tool: "claude-desktop", kind: "switch", current: null, inLive: [], liveError: null, paths: [], providers: [] }),
      codex_accounts: () => [],
      add_provider: () => ({ changed: [], warnings: [] }),
    });
    renderWithClient(<ToolPage tool="claude-desktop" />);
    await userEvent.click(await screen.findByRole("button", { name: /添加供应商/ }));
    await userEvent.click(await screen.findByRole("button", { name: /添加模型/ }));
    const id = screen.getByLabelText("模型名");
    await userEvent.clear(id);
    await userEvent.type(id, "gpt-5");
    expect(screen.getByRole("alert")).toHaveTextContent(/claude-sonnet/);
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    await userEvent.clear(id);
    await userEvent.type(id, "claude-sonnet-4-5");
    await userEvent.type(screen.getByLabelText("显示名（可选）"), "Sonnet");
    await userEvent.click(screen.getByRole("checkbox", { name: "支持 1M 上下文" }));
    await userEvent.type(screen.getByLabelText("ID"), "x");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(fake.calls.some((c) => c.cmd === "add_provider")).toBe(true));
    const args = fake.calls.find((c) => c.cmd === "add_provider")!.args as { provider: { meta?: Record<string, unknown> } };
    expect(args.provider.meta?.claudeDesktopModelRoutes).toEqual({ "claude-sonnet-4-5": { labelOverride: "Sonnet", supports1m: true } });
  });
});

describe("delete confirmation", () => {
  it("asks in an in-app alert dialog; cancel and Esc keep the card, confirm deletes", async () => {
    const confirmSpy = vi.spyOn(window, "confirm");
    const fake = use({
      list_providers: () => ({
        tool: "claude",
        kind: "switch",
        current: null,
        inLive: [],
        liveError: null,
        paths: [],
        providers: [{ id: "relay", name: "Relay", settingsConfig: { env: {} } }],
      }),
      codex_accounts: () => [],
      delete_provider: () => null,
    });
    renderWithClient(<ToolPage tool="claude" />);
    const del = await screen.findByRole("button", { name: "删除 Relay" });

    await userEvent.click(del);
    let dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText("确定删除「Relay」？")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "取消" })).toHaveFocus();
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());

    await userEvent.click(del);
    await screen.findByRole("alertdialog");
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    expect(fake.calls.some((c) => c.cmd === "delete_provider")).toBe(false);

    await userEvent.click(del);
    dialog = await screen.findByRole("alertdialog");
    const action = within(dialog).getByRole("button", { name: "删除" });
    expect(action.className).toMatch(/bg-destructive/);
    await userEvent.click(action);
    await waitFor(() => expect(fake.calls.find((c) => c.cmd === "delete_provider")?.args).toEqual({ tool: "claude", id: "relay" }));
    expect(confirmSpy).not.toHaveBeenCalled();
    confirmSpy.mockRestore();
  });

  it("removes a Codex account only after confirming", async () => {
    const fake = use({
      codex_accounts: () => accounts,
      codex_logins: () => [],
      codex_remove_account: () => null,
    });
    renderWithClient(<CodexAccountsPage />);
    await userEvent.click(await screen.findByRole("button", { name: /移除 me@example.com/ }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(/绑定它的卡将无法切换/)).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "移除" }));
    await waitFor(() => expect(fake.calls.find((c) => c.cmd === "codex_remove_account")?.args).toEqual({ id: "acc-plus" }));
  });
});

describe("CodexAccountsPage", () => {
  it("shows Plus / Team badges, default and re-auth state", async () => {
    use({
      codex_accounts: () => accounts,
      codex_logins: () => [{ identity: "sub:u|account:w", email: "local@example.com", workspace: null, plan: "pro", live: true }],
    });
    renderWithClient(<CodexAccountsPage />);
    const plus = await screen.findByTestId("account-acc-plus");
    expect(within(plus).getByText("Plus")).toBeInTheDocument();
    expect(within(plus).getByText("默认")).toBeInTheDocument();
    const team = screen.getByTestId("account-acc-team");
    expect(within(team).getByText("Team")).toBeInTheDocument();
    expect(within(team).getByText("需要重新登录")).toBeInTheDocument();
    expect(await screen.findByText("local@example.com")).toBeInTheDocument();
    expect(screen.getByText("Pro")).toBeInTheDocument();
  });

  it("runs the device login flow and cancels nothing once added", async () => {
    let polls = 0;
    const fake = use({
      codex_accounts: () => [],
      codex_logins: () => [],
      codex_start_login: () => ({ deviceCode: "dev-1", userCode: "ABCD-1234", verificationUri: "https://auth.openai.com/codex/device", expiresIn: 900, interval: 1 }),
      codex_poll_login: () => (++polls < 2 ? { status: "pending" } : { status: "added", account: accounts[0] }),
      codex_cancel_login: () => true,
    });
    renderWithClient(<CodexAccountsPage />);
    await userEvent.click(await screen.findByRole("button", { name: /登录新账号/ }));
    expect(await screen.findByTestId("user-code")).toHaveTextContent("ABCD-1234");
    await waitFor(() => expect(polls).toBe(2), { timeout: 4000 });
    await waitFor(() => expect(screen.queryByTestId("user-code")).not.toBeInTheDocument());
    expect(fake.calls.some((c) => c.cmd === "codex_cancel_login")).toBe(false);
  });
});

describe("ImportWizard", () => {
  it("scans read-only, preselects importable tools and applies", async () => {
    const fake = use({
      get_settings: () => ({ app: {}, keepCodexLogin: false, dataDir: "/d", home: "/h", livePaths: {}, ccswitchDir: "/h/.cc-switch" }),
      ccswitch_scan: () => ({
        source: "/h/.cc-switch/cc-switch.db",
        accounts: 1,
        candidates: [
          { tool: "claude", provider: { id: "k", name: "Kimi", settingsConfig: {} }, skipReason: null },
          { tool: "claude", provider: { id: "c", name: "Copilot", settingsConfig: {} }, skipReason: "需要本地路由" },
          { tool: "gemini", provider: { id: "g", name: "G", settingsConfig: {} }, skipReason: "id 已存在" },
        ],
      }),
      ccswitch_apply: () => ({ created: [["claude", "k"]], skipped: [["claude", "c", "需要本地路由"]], accountsAdded: [] }),
    });
    renderWithClient(<ImportWizard />);
    await userEvent.click(screen.getByRole("button", { name: /扫描/ }));
    expect(await screen.findByText(/需要本地路由/)).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "select claude" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "select gemini" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: /导入所选工具/ }));
    expect(await screen.findByText(/新增 1 张卡/)).toBeInTheDocument();
    expect(fake.calls.find((c) => c.cmd === "ccswitch_apply")!.args).toEqual({ tools: ["claude"], dir: null });
  });
});

describe("SettingsPage", () => {
  it("drops empty overrides", () => {
    expect(cleanOverrides({ claudeDir: "  ", codexDir: "/x/codex ", geminiDir: null })).toEqual({ codexDir: "/x/codex" });
  });

  it("saves the keep-Codex-login toggle with app settings", async () => {
    const fake = use({
      get_settings: () => ({
        app: { language: null, dirOverrides: {}, autostart: false, backgroundRefreshHours: 168, closeToTray: true },
        keepCodexLogin: false,
        dataDir: "/h/.cc-switch-lite",
        home: "/h",
        livePaths: { claude: ["/h/.claude/settings.json"] },
        ccswitchDir: "/h/.cc-switch",
      }),
      save_settings: () => ({ dirsChanged: true }),
    });
    renderWithClient(<SettingsPage />);
    await userEvent.click(await screen.findByRole("switch", { name: /保留 Codex/ }));
    await userEvent.type(screen.getByLabelText(/Codex \(~\/.codex\)/), "/alt/codex");
    await userEvent.click(screen.getByRole("button", { name: "保存设置" }));
    await waitFor(() => expect(fake.calls.some((c) => c.cmd === "save_settings")).toBe(true));
    const input = (fake.calls.find((c) => c.cmd === "save_settings")!.args as { input: { app: { dirOverrides: unknown }; keepCodexLogin: boolean } }).input;
    expect(input.keepCodexLogin).toBe(true);
    expect(input.app.dirOverrides).toEqual({ codexDir: "/alt/codex" });
  });
});

describe("ExportImport", () => {
  // 前面的设置页测试可能切过界面语言。
  beforeEach(async () => {
    const { default: i18n } = await import("@/i18n");
    await i18n.changeLanguage("zh");
  });
  const reason = (code: string, zh: string) => ({ code, zh, en: zh });
  const preview = (policy: string) => ({
    format: "sql-dump",
    schemaVersion: 12,
    items: [
      { app: "claude", tool: "claude", sourceId: "relay", name: "Relay", status: "import", targetId: "relay", reason: null, notes: [] },
      {
        app: "claude",
        tool: "claude",
        sourceId: "taken",
        name: "Taken",
        status: "duplicate",
        targetId: policy === "rename" ? "taken-2" : null,
        reason: policy === "rename" ? reason("renamed", "已有同 id 的卡，改名为 taken-2 导入") : reason("same_id", "已有同 id 的卡，跳过"),
        notes: [],
      },
      { app: "codex", tool: "codex", sourceId: "copilot", name: "Copilot", status: "skip", targetId: null, reason: reason("routing", "需要 cc-switch 的本地路由 / 反代（github_copilot）"), notes: [] },
      { app: "copilot-cli", tool: null, sourceId: "x", name: "X", status: "skip", targetId: null, reason: reason("unsupported_app", "ccswitch-lite 不支持这个应用（copilot-cli）"), notes: [] },
    ],
  });

  it("picks a file, previews per tool with reasons, switches duplicate policy and confirms", async () => {
    const fake = use({
      ccswitch_export_load: () => preview("skip"),
      ccswitch_export_preview: (args) => preview((args as { policy: string }).policy),
      ccswitch_export_apply: () => ({ created: [["claude", "relay", "relay"], ["claude", "taken", "taken-2"]], skipped: 2 }),
    });
    const { ExportImport } = await import("./ExportImport");
    renderWithClient(<ExportImport />);
    const file = new File(["-- CC Switch SQLite 导出\n"], "cc-switch-export-20261001_080000.sql", { type: "application/sql" });
    await userEvent.upload(screen.getByTestId("export-file"), file);

    await waitFor(() => expect(fake.calls.some((c) => c.cmd === "ccswitch_export_load")).toBe(true));
    expect(await screen.findByText(/可导入 1 · 重复 1 · 跳过 2/, undefined, { timeout: 4000 })).toBeInTheDocument();
    const load = fake.calls.find((c) => c.cmd === "ccswitch_export_load")!.args as { body: Uint8Array; headers: Record<string, string> };
    expect(new TextDecoder().decode(load.body)).toContain("CC Switch SQLite");
    expect(load.headers["x-duplicate-policy"]).toBe("skip");
    expect(within(screen.getByTestId("export-item-codex-copilot")).getByText(/本地路由/)).toBeInTheDocument();
    expect(within(screen.getByTestId("export-group-other")).getByText(/不支持这个应用/)).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "导入 Codex" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "确认导入 1 张卡" })).toBeEnabled();

    await userEvent.selectOptions(screen.getByLabelText("同 id 的卡"), "rename");
    expect(await screen.findByText(/taken → taken-2/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "确认导入 2 张卡" }));
    await waitFor(() => expect(screen.getByText(/新增 2 张卡，跳过 2 张/)).toBeInTheDocument());
    const apply = fake.calls.find((c) => c.cmd === "ccswitch_export_apply")!.args as { tools: string[]; policy: string };
    expect(apply).toEqual({ tools: ["claude"], policy: "rename" });
  });

  it("shows the backend error for a malformed file and stays idle", async () => {
    const fake = use({
      ccswitch_export_load: () => {
        throw "不是 cc-switch 的导出文件（支持 SQL 导出、.db 备份、旧版 JSON 配置）";
      },
    });
    const { ExportImport } = await import("./ExportImport");
    renderWithClient(<ExportImport />);
    await userEvent.upload(screen.getByTestId("export-file"), new File(["hello"], "notes.sql"));
    await waitFor(() => expect(fake.calls.some((c) => c.cmd === "ccswitch_export_load")).toBe(true));
    expect(screen.queryByRole("button", { name: /确认导入/ })).not.toBeInTheDocument();
  });
});
