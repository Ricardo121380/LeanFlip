import { describe, expect, it, vi, beforeEach } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { fakeBackend, renderWithClient, type Handler } from "@/test/utils";

const backend: { invoke: ReturnType<typeof vi.fn> | null } = { invoke: null };
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => backend.invoke!(cmd, args),
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
    await userEvent.type(screen.getByPlaceholderText("搜索预设…"), "Kimi");
    const option = screen.getAllByRole("option")[0];
    await userEvent.click(option);
    expect((screen.getByLabelText("ID") as HTMLInputElement).value).toMatch(/^kimi/);
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(fake.calls.some((c) => c.cmd === "add_provider")).toBe(true));
    const args = fake.calls.find((c) => c.cmd === "add_provider")!.args as { provider: { id: string; settingsConfig: { env: Record<string, string> } }; addToLive: boolean };
    expect(args.provider.id).toMatch(/^kimi/);
    expect(args.provider.settingsConfig.env.ANTHROPIC_BASE_URL).toMatch(/^https:\/\//);
    expect(args.addToLive).toBe(false);
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
      save_settings: () => ({ restartRequired: true }),
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
