import { beforeEach, describe, expect, it, vi } from "vitest";
import { StrictMode, useState } from "react";
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { fakeBackend, renderWithClient, type Handler } from "@/test/utils";
import type { Provider, QuotaDetect } from "@/lib/api";

const mock = vi.hoisted(() => ({ invoke: vi.fn(), loadPresets: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => mock.invoke(...args) }));
vi.mock("@/lib/presets", async (original) => ({ ...await original<object>(), loadPresets: () => mock.loadPresets() }));
import { ProviderEditor } from "./ProviderEditor";
import { QuotaDetectDialog } from "./QuotaDetectDialog";

const relay: Provider = {
  id: "corp",
  name: "公司网关",
  settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://gw.example.com", ANTHROPIC_AUTH_TOKEN: "sk-corp" } },
};
const krill: Provider = { ...relay, settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://api.krill-code.net/codex/v1", ANTHROPIC_AUTH_TOKEN: "sk-corp" } } };
const now = Date.now();
const found: QuotaDetect = {
  steps: [
    { adapter: "sub2api", url: "https://gw.example.com/v1/usage", outcome: "miss", status: 200, code: "format" },
    { adapter: "newapi", url: "https://gw.example.com/api/usage/token/", outcome: "hit", status: 200 },
  ],
  adapter: "newapi",
  key: "relay:https://gw.example.com:abc",
  snapshot: {
    windows: [{ minutes: 300, usedPercent: 84, resetsAt: Math.floor(now / 1000) + 2 * 3600 + 90, reset: false, usedAmount: 4.2, limitAmount: 5 }],
    balance: { unit: "USD", remaining: 31.4, limit: 58, used: 26.6, unlimited: false, accountRemaining: 120 },
    fetchedAt: now,
    origin: "network",
    level: "warn",
    usedFraction: 0.84,
    limitReached: false,
  },
};

function mount(provider: Provider, handlers: Record<string, Handler> = {}) {
  const backend = fakeBackend({ ignored_fields: () => [], ...handlers });
  mock.invoke = backend.invoke;
  function Host() {
    const [open, setOpen] = useState(true);
    return open ? <ProviderEditor tool="claude" open onOpenChange={setOpen} provider={provider} existingIds={[provider.id]} /> : <p>Editor closed</p>;
  }
  return { ...renderWithClient(<StrictMode><Host /></StrictMode>), backend };
}

beforeEach(() => {
  vi.clearAllMocks();
  mock.loadPresets.mockResolvedValue([]);
});

describe("Relay usage lookup in the provider editor", () => {
  it("switches the connection UI by Base URL, with no editable platform selector", async () => {
    const { backend } = mount({ ...relay, meta: { leanflipQuota: { adapter: "newapi", refreshMinutes: 15, accountToken: "__leanflip_redacted__" } } });
    expect(screen.queryByLabelText("平台类型")).not.toBeInTheDocument();
    expect(screen.getByText("已自动适配 NewAPI")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("请求地址"), { target: { value: "https://api-slb.krill-ai.net/codex/v1" } });
    expect(screen.getByText("已自动适配 Krill")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "登录 Krill 获取额度" })).toBeEnabled();
    expect(screen.queryByLabelText("用户访问令牌（可选）")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("请求地址"), { target: { value: "https://other.example/v1" } });
    expect(screen.getByText("查询时自动识别当前 Base URL 的平台")).toBeInTheDocument();
    expect(screen.getByLabelText("用户访问令牌（可选）")).toHaveValue("__leanflip_redacted__");
    expect(backend.calls.some((c) => c.cmd.startsWith("quota_"))).toBe(false);
  });

  it("preserves a connected Krill account when checks are turned off and on", async () => {
    mount({ ...krill, meta: { leanflipQuota: { adapter: "krill", refreshMinutes: 15, krillJwt: "__leanflip_redacted__" } } });
    await userEvent.click(screen.getByRole("checkbox", { name: "启用额度查询" }));
    expect(screen.getByRole("button", { name: "查询额度" })).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox", { name: "启用额度查询" }));
    expect(screen.getByText("已连接 Krill 账户")).toBeInTheDocument();
    expect(screen.getByLabelText("自动刷新")).toHaveValue("15");
  });

  it("cancels sign-in on an address change and ignores a late completion", async () => {
    let complete!: (result: unknown) => void;
    const { backend } = mount(krill, {
      quota_krill_login: () => new Promise((resolve) => { complete = resolve; }),
      quota_krill_login_cancel: () => {},
    });
    await userEvent.click(screen.getByRole("button", { name: "登录 Krill 获取额度" }));
    fireEvent.change(screen.getByLabelText("请求地址"), { target: { value: "https://other.example/v1" } });
    expect(backend.calls.some((c) => c.cmd === "quota_krill_login_cancel")).toBe(true);
    await act(async () => complete({ credential: "__leanflip_krill_login_late", detected: found }));
    expect(screen.queryByText("已连接 Krill 账户")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "查询额度" })).toBeEnabled();
    expect(screen.getByText("查询时自动识别当前 Base URL 的平台")).toBeInTheDocument();
  });

  it("queries with a separate user access token and saves it without changing the model key", async () => {
    const account = { ...found, snapshot: { ...found.snapshot!, windows: [], balance: { unit: "USD", unlimited: false, accountRemaining: 123.45 } } };
    const { backend } = mount(relay, { quota_account_detect: () => account, update_provider: () => ({ changed: [], warnings: [] }) });
    const field = screen.getByLabelText("用户访问令牌（可选）");
    expect(field).toHaveAttribute("type", "password");
    await userEvent.type(field, "personal-access");
    await userEvent.type(screen.getByLabelText("NewAPI 用户 ID（仅部分旧站点需要）"), "7");
    await userEvent.click(screen.getByRole("button", { name: "查询额度" }));
    const dialog = await screen.findByRole("dialog", { name: "识别 公司网关 的额度接口" });
    expect(await within(dialog).findByText("$123.45")).toBeInTheDocument();
    expect(within(dialog).queryByText("Key 额度")).not.toBeInTheDocument();
    expect(backend.calls.find((c) => c.cmd === "quota_account_detect")?.args).toEqual({tool:"claude", settingsConfig: relay.settingsConfig, providerId:"corp", settings:{refreshMinutes:0,accountToken:"personal-access",accountUserId:7}});
    expect(backend.calls.some((c) => c.cmd === "quota_detect")).toBe(false);
    await userEvent.click(within(dialog).getByRole("button", { name: "完成" }));
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(backend.calls.some((c) => c.cmd === "update_provider")).toBe(true));
    const saved = backend.calls.find((c) => c.cmd === "update_provider")!.args!.provider as Provider;
    expect(saved.settingsConfig).toEqual(relay.settingsConfig);
    expect(saved.meta?.leanflipQuota).toEqual({adapter:"newapi", refreshMinutes:0, accountToken:"personal-access", accountUserId:7});
  });

  it("lets users cancel website sign-in and retry without losing the existing card", async () => {
    let rejectLogin: (e: unknown) => void = () => {};
    const { backend } = mount(krill, {
      quota_krill_login: () => new Promise((_resolve,reject) => { rejectLogin=reject; }),
      quota_krill_login_cancel: () => { rejectLogin(new Error("Krill 登录已取消")); },
    });
    expect(screen.queryByLabelText("平台类型")).not.toBeInTheDocument();
    expect(screen.getByText("已自动适配 Krill")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "登录 Krill 获取额度" }));
    const status = await screen.findByText("等待完成官网登录…");
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    await userEvent.click(within(status.closest('[role="status"]')!).getByRole("button", { name: "取消" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Krill 登录已取消");
    expect(screen.getByRole("button", { name: "登录 Krill 获取额度" })).toBeEnabled();
    expect(backend.calls.some((c) => c.cmd === "update_provider")).toBe(false);
  });

  it("applies a detected platform once and lets a failed save retry without re-querying", async () => {
    const backend = fakeBackend({ quota_detect: () => found });
    mock.invoke = backend.invoke;
    const apply = vi.fn().mockRejectedValueOnce(new Error("save failed")).mockResolvedValue(undefined);
    renderWithClient(<StrictMode><QuotaDetectDialog tool="claude" name="公司网关" settingsConfig={relay.settingsConfig} onUse={apply} onClose={vi.fn()} /></StrictMode>);
    expect(await screen.findByRole("alert")).toHaveTextContent("save failed");
    expect(apply).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByRole("button", { name: "重试" }));
    await screen.findByText(/已自动应用识别结果/);
    expect(apply).toHaveBeenCalledTimes(2);
    expect(backend.calls.filter((c) => c.cmd === "quota_detect")).toHaveLength(1);
  });

  it("explains missing account balance without displaying an unlimited or negative wallet", async () => {
    const result = structuredClone(found);
    result.snapshot!.balance = { unit: "USD", unlimited: true, remaining: -6665 };
    mount(relay, { quota_detect: () => result });
    await userEvent.click(screen.getByRole("button", { name: "查询额度" }));
    const dialog = await screen.findByRole("dialog", { name: "识别 公司网关 的额度接口" });
    expect(await within(dialog).findByText("余额未提供")).toBeInTheDocument();
    expect(within(dialog).getByText("Key 未设限")).toBeInTheDocument();
    expect(within(dialog).getByText(/允许查询用户额度/)).toBeInTheDocument();
    expect(within(dialog).queryByText("不限")).not.toBeInTheDocument();
    expect(dialog.textContent).not.toContain("6665");
  });

  it("connects Krill through website sign-in, displays plans and saves without exposing a JWT", async () => {
    const krillFound: QuotaDetect = {
      steps: [{ adapter: "krill", url: "https://www.krill-code.com/api/subscription", outcome: "hit", status: 200 }],
      adapter: "krill",
      snapshot: {
        windows: [
          { minutes: 10080, label: "Claude Weekly", usedPercent: 20, reset: false, usedAmount: 20, limitAmount: 100 },
          { minutes: 10080, label: "Requests", unit: "requests", usedPercent: 50, reset: false, usedAmount: 10, limitAmount: 20 },
        ],
        balance: { unit: "USD", accountRemaining: 0, unlimited: false },
        fetchedAt: now, origin: "network", level: "ok", limitReached: false,
      },
    };
    const { backend } = mount(krill, { quota_krill_login: () => ({ credential: "__leanflip_krill_login_test", detected: krillFound }), quota_krill_detect: () => krillFound, update_provider: () => ({ changed: [], warnings: [] }) });
    expect(screen.queryByLabelText("平台类型")).not.toBeInTheDocument();
    expect(screen.getByText("已自动适配 Krill")).toBeInTheDocument();
    expect(screen.queryByLabelText("Krill 登录 JWT")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "登录 Krill 获取额度" }));
    await screen.findByText("已连接 Krill 账户");
    expect(backend.calls.find((call) => call.cmd === "quota_krill_login")?.args).toEqual({ tool: "claude" });
    await userEvent.selectOptions(screen.getByLabelText("自动刷新"), "15");
    expect(screen.queryByLabelText("多少额度算 $1")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "查询额度" }));
    const dialog = await screen.findByRole("dialog", { name: "识别 公司网关 的额度接口" });
    expect(await within(dialog).findByText("识别为 Krill")).toBeInTheDocument();
    expect(within(dialog).getByText("Claude Weekly · 本周")).toBeInTheDocument();
    expect(within(dialog).getByText("$20.00 / $100.00")).toBeInTheDocument();
    expect(within(dialog).getByText("10 requests / 20 requests")).toBeInTheDocument();
    expect(within(dialog).getByText("账户余额")).toBeInTheDocument();
    expect(within(dialog).queryByText("余额")).not.toBeInTheDocument();
    expect(backend.calls.find((call) => call.cmd === "quota_krill_detect")?.args).toEqual({ tool: "claude", providerId: "corp", jwt: "__leanflip_krill_login_test" });
    expect(backend.calls.some((call) => call.cmd === "quota_detect")).toBe(false);
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "完成" })).toBeEnabled());
    expect(within(dialog).queryByRole("button", { name: /^使用/ })).not.toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "完成" }));
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(backend.calls.some((call) => call.cmd === "update_provider")).toBe(true));
    const saved = backend.calls.find((call) => call.cmd === "update_provider")!.args!.provider as Provider;
    expect(saved.settingsConfig).toEqual(krill.settingsConfig);
    expect(saved.meta?.leanflipQuota).toEqual({ adapter: "krill", refreshMinutes: 15, krillJwt: "__leanflip_krill_login_test" });
  });

  it("keeps a saved Krill credential hidden and explains how to replace an expired JWT", async () => {
    const configured: Provider = { ...krill, meta: { leanflipQuota: { adapter: "krill", refreshMinutes: 0, krillJwt: "__leanflip_redacted__" } } };
    const { backend } = mount(configured, {
      quota_krill_detect: () => ({ steps: [{ adapter: "krill", url: "https://www.krill-code.com/api/subscription", outcome: "miss", status: 401, code: "krillAuth" }] }),
      update_provider: () => ({ changed: [], warnings: [] }),
    });
    expect(screen.queryByLabelText("Krill 登录 JWT")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重新登录 Krill" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "查询额度" }));
    const dialog = await screen.findByRole("dialog", { name: "识别 公司网关 的额度接口" });
    expect(await within(dialog).findByText("Krill 登录已过期或无权限，请在卡片额度设置中重新登录")).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: "使用 Krill" })).not.toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    await userEvent.click(screen.getByRole("checkbox", { name: "启用额度查询" }));
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(backend.calls.some((call) => call.cmd === "update_provider")).toBe(true));
    const saved = backend.calls.find((call) => call.cmd === "update_provider")!.args!.provider as Provider;
    expect(saved.meta?.leanflipQuota).toEqual({ adapter: "off", refreshMinutes: 0, krillJwt: "__leanflip_redacted__" });
  });

  it("detects the platform only when asked, shows what it found, and saves the choice on the card", async () => {
    const { backend } = mount(relay, { quota_detect: () => found, update_provider: () => ({ changed: [], warnings: [] }) });
    expect(screen.getByRole("heading", { name: "额度查询" })).toBeInTheDocument();
    expect(screen.queryByLabelText("平台类型")).not.toBeInTheDocument();
    expect(screen.getByText("查询时自动识别当前 Base URL 的平台")).toBeInTheDocument();
    expect(backend.calls.some((c) => c.cmd === "quota_detect")).toBe(false);

    await userEvent.click(screen.getByRole("button", { name: "查询额度" }));
    const dialog = await screen.findByRole("dialog", { name: "识别 公司网关 的额度接口" });
    expect(await within(dialog).findByText("识别为 NewAPI")).toBeInTheDocument();
    const steps = within(dialog).getByRole("list", { name: "尝试过的接口" });
    expect(within(steps).getByText("不是")).toBeInTheDocument();
    expect(within(steps).getByText("认出来了")).toBeInTheDocument();
    expect(within(steps).getByText("GET https://gw.example.com/api/usage/token/")).toBeInTheDocument();
    expect(within(dialog).getByText("$31.40")).toBeInTheDocument();
    expect(within(dialog).getByText("共 $58.00")).toBeInTheDocument();
    expect(within(dialog).getByText("$120.00")).toBeInTheDocument();
    expect(within(dialog).getByText("84%")).toHaveClass("text-warning");
    expect(within(dialog).getByText("$4.20 / $5.00 · 2 小时 1 分后重置")).toBeInTheDocument();
    expect(backend.calls.filter((c) => c.cmd === "quota_detect")).toEqual([
      { cmd: "quota_detect", args: { tool: "claude", settingsConfig: relay.settingsConfig, quotaPerUsd: null } },
    ]);

    await waitFor(() => expect(within(dialog).getByRole("button", { name: "完成" })).toBeEnabled());
    expect(within(dialog).queryByRole("button", { name: /^使用/ })).not.toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "完成" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "识别 公司网关 的额度接口" })).not.toBeInTheDocument());
    expect(screen.getByRole("button", { name: "查询额度" })).toHaveFocus();
    expect(screen.getByText("已自动适配 NewAPI")).toBeInTheDocument();
    await userEvent.type(screen.getByLabelText("多少额度算 $1"), "1000000");
    expect(screen.getByLabelText("自动刷新")).toHaveValue("0");
    await userEvent.selectOptions(screen.getByLabelText("自动刷新"), "15");

    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(backend.calls.some((c) => c.cmd === "update_provider")).toBe(true));
    const saved = backend.calls.find((c) => c.cmd === "update_provider")!.args!.provider as Provider;
    expect(saved.meta?.leanflipQuota).toEqual({ refreshMinutes: 15, adapter: "newapi", quotaPerUsd: 1000000 });
  });

  it("says when nothing matched or the card has no key, and lets the user try again", async () => {
    let calls = 0;
    mount(relay, {
      quota_detect: () => {
        calls += 1;
        return calls === 1
          ? { steps: [
              { adapter: "sub2api", url: "https://gw.example.com/v1/usage", outcome: "miss", status: 404, code: "http" },
              { adapter: "newapi", url: "https://gw.example.com/api/usage/token/", outcome: "error", code: "network" },
            ] }
          : { steps: [], error: "noEndpoint" };
      },
    });
    await userEvent.click(screen.getByRole("button", { name: "查询额度" }));
    const dialog = await screen.findByRole("dialog", { name: "识别 公司网关 的额度接口" });
    expect(await within(dialog).findByText(/没有认出平台/)).toBeInTheDocument();
    expect(within(dialog).getByText("不是（HTTP 404）")).toBeInTheDocument();
    expect(within(dialog).getByText("连不上")).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: /^使用/ })).not.toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "再试一次" }));
    expect(await within(dialog).findByText("这张卡还没有地址或 Key，填好后再识别。")).toBeInTheDocument();
    expect(calls).toBe(2);
  });

  it("keeps an existing setting, can turn lookups off, and leaves official cards alone", async () => {
    const configured: Provider = { ...relay, meta: { leanflipQuota: { adapter: "sub2api", refreshMinutes: 0 } } };
    const { backend, unmount } = mount(configured, { update_provider: () => ({ changed: [], warnings: [] }) });
    expect(screen.getByText("已自动适配 sub2api")).toBeInTheDocument();
    expect(screen.queryByLabelText("多少额度算 $1")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("checkbox", { name: "启用额度查询" }));
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(backend.calls.some((c) => c.cmd === "update_provider")).toBe(true));
    const saved = backend.calls.find((c) => c.cmd === "update_provider")!.args!.provider as Provider;
    expect(saved.meta?.leanflipQuota).toEqual({ refreshMinutes: 0, adapter: "off" });
    unmount();

    mount({ id: "official", name: "Claude 官方", category: "official", settingsConfig: { env: {} } });
    expect(screen.queryByRole("heading", { name: "额度查询" })).not.toBeInTheDocument();
  });
});

it("uses and saves an explicit quota prefix without changing the model connection", async () => {
  const { backend } = mount(relay, { quota_detect: () => found, update_provider: () => ({ changed: [], warnings: [] }) });
  await userEvent.type(screen.getByLabelText("额度查询基址（可选）"), "https://gw.example.com/billing/");
  await userEvent.click(screen.getByRole("button", { name: "查询额度" }));
  const dialog = await screen.findByRole("dialog", { name: "识别 公司网关 的额度接口" });
  await within(dialog).findByRole("button", { name: "完成" });
  expect(backend.calls.find((call) => call.cmd === "quota_detect")?.args).toMatchObject({ queryBaseUrl: "https://gw.example.com/billing/", settingsConfig: relay.settingsConfig });
  await userEvent.click(within(dialog).getByRole("button", { name: "完成" }));
  await userEvent.click(screen.getByRole("button", { name: "保存" }));
  await screen.findByText("Editor closed");
  const saved = backend.calls.find((call) => call.cmd === "update_provider")?.args?.provider as Provider;
  expect(saved.meta?.leanflipQuota).toMatchObject({ queryBaseUrl: "https://gw.example.com/billing/" });
  expect(saved.settingsConfig).toEqual(relay.settingsConfig);
});
