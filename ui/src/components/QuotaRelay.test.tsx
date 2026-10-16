import { beforeEach, describe, expect, it, vi } from "vitest";
import { StrictMode, useState } from "react";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { fakeBackend, renderWithClient, type Handler } from "@/test/utils";
import type { Provider, QuotaDetect } from "@/lib/api";

const mock = vi.hoisted(() => ({ invoke: vi.fn(), loadPresets: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => mock.invoke(...args) }));
vi.mock("@/lib/presets", async (original) => ({ ...await original<object>(), loadPresets: () => mock.loadPresets() }));
import { ProviderEditor } from "./ProviderEditor";

const relay: Provider = {
  id: "corp",
  name: "公司网关",
  settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://gw.example.com", ANTHROPIC_AUTH_TOKEN: "sk-corp" } },
};
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
  it("queries Krill with a separate JWT, displays named plans and saves without changing the model key", async () => {
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
    const { backend } = mount(relay, { quota_krill_detect: () => krillFound, update_provider: () => ({ changed: [], warnings: [] }) });
    await userEvent.selectOptions(screen.getByLabelText("平台类型"), "krill");
    const jwt = screen.getByLabelText("Krill 登录 JWT");
    expect(jwt).toHaveAttribute("type", "password");
    await userEvent.type(jwt, "test-login-jwt");
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
    expect(backend.calls.find((call) => call.cmd === "quota_krill_detect")?.args).toEqual({ tool: "claude", providerId: "corp", jwt: "test-login-jwt" });
    expect(backend.calls.some((call) => call.cmd === "quota_detect")).toBe(false);
    await userEvent.click(within(dialog).getByRole("button", { name: "使用 Krill" }));
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(backend.calls.some((call) => call.cmd === "update_provider")).toBe(true));
    const saved = backend.calls.find((call) => call.cmd === "update_provider")!.args!.provider as Provider;
    expect(saved.settingsConfig).toEqual(relay.settingsConfig);
    expect(saved.meta?.leanflipQuota).toEqual({ adapter: "krill", refreshMinutes: 15, krillJwt: "test-login-jwt" });
  });

  it("keeps a saved Krill credential hidden and explains how to replace an expired JWT", async () => {
    const configured: Provider = { ...relay, meta: { leanflipQuota: { adapter: "krill", refreshMinutes: 0, krillJwt: "__leanflip_redacted__" } } };
    const { backend } = mount(configured, {
      quota_krill_detect: () => ({ steps: [{ adapter: "krill", url: "https://www.krill-code.com/api/subscription", outcome: "miss", status: 401, code: "krillAuth" }] }),
      update_provider: () => ({ changed: [], warnings: [] }),
    });
    expect(screen.getByLabelText("Krill 登录 JWT")).toHaveValue("__leanflip_redacted__");
    await userEvent.click(screen.getByRole("button", { name: "查询额度" }));
    const dialog = await screen.findByRole("dialog", { name: "识别 公司网关 的额度接口" });
    expect(await within(dialog).findByText("Krill 登录 JWT 已失效或无权限，请在卡片额度设置中更新")).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: "使用 Krill" })).not.toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    await userEvent.selectOptions(screen.getByLabelText("平台类型"), "off");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(backend.calls.some((call) => call.cmd === "update_provider")).toBe(true));
    const saved = backend.calls.find((call) => call.cmd === "update_provider")!.args!.provider as Provider;
    expect(saved.meta?.leanflipQuota).toEqual({ adapter: "off", refreshMinutes: 0 });
  });

  it("detects the platform only when asked, shows what it found, and saves the choice on the card", async () => {
    const { backend } = mount(relay, { quota_detect: () => found, update_provider: () => ({ changed: [], warnings: [] }) });
    expect(screen.getByRole("heading", { name: "额度查询" })).toBeInTheDocument();
    expect(screen.getByLabelText("平台类型")).toHaveValue("");
    expect(backend.calls.some((c) => c.cmd === "quota_detect")).toBe(false);

    await userEvent.click(screen.getByRole("button", { name: "识别平台" }));
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

    await userEvent.click(within(dialog).getByRole("button", { name: "使用 NewAPI" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "识别 公司网关 的额度接口" })).not.toBeInTheDocument());
    expect(screen.getByLabelText("平台类型")).toHaveValue("newapi");
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
    await userEvent.click(screen.getByRole("button", { name: "识别平台" }));
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
    expect(screen.getByLabelText("平台类型")).toHaveValue("sub2api");
    expect(screen.queryByLabelText("多少额度算 $1")).not.toBeInTheDocument();
    await userEvent.selectOptions(screen.getByLabelText("平台类型"), "off");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(backend.calls.some((c) => c.cmd === "update_provider")).toBe(true));
    const saved = backend.calls.find((c) => c.cmd === "update_provider")!.args!.provider as Provider;
    expect(saved.meta?.leanflipQuota).toEqual({ refreshMinutes: 0, adapter: "off" });
    unmount();

    mount({ id: "official", name: "Claude 官方", category: "official", settingsConfig: { env: {} } });
    expect(screen.queryByRole("heading", { name: "额度查询" })).not.toBeInTheDocument();
  });
});
