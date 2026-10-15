import { beforeEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Provider, QuotaSnapshot, QuotaSource, QuotaView, ToolId, ToolProviders, ToolSummary } from "@/lib/api";
import { TOOLS } from "@/lib/tools";
import { fakeBackend, renderWithClient, type Handler } from "@/test/utils";

const backend = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => backend.invoke(...args) }));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn() } }));

import { HomePage } from "./HomePage";
import { StatsPage } from "./StatsPage";

const now = Date.now();
const s = Math.floor(now / 1000);
const relay = (id: string, name: string): Provider => ({ id, name, settingsConfig: { env: { ANTHROPIC_BASE_URL: `https://${id}.example`, ANTHROPIC_AUTH_TOKEN: `sk-${id}` } } });
const balance = (remaining: number, level: QuotaSnapshot["level"] = "ok"): QuotaSnapshot => ({
  windows: [],
  balance: { unit: "USD", remaining, limit: 50, used: 50 - remaining, unlimited: false },
  fetchedAt: now - 3 * 60_000,
  origin: "network",
  level,
  usedFraction: (50 - remaining) / 50,
  limitReached: false,
});
const weekly = (used: number, level: QuotaSnapshot["level"] = "ok"): QuotaSnapshot => ({
  windows: [{ minutes: 10080, usedPercent: used, resetsAt: s + 3 * 86_400 + 600, reset: false }],
  credits: { hasCredits: true, unlimited: false, balance: 60951.56 },
  plan: "pro",
  fetchedAt: now - 42 * 60_000,
  origin: "local",
  level,
  usedFraction: used / 100,
  limitReached: false,
});

function sources(): QuotaSource[] {
  return [
    {
      key: "codex:ws:user", kind: "codex", label: "me@example.com", login: "sub:me|account:ws",
      users: [{ tool: "codex", providerId: "official", providerName: "Codex 官方", active: true }],
      snapshot: weekly(51),
    },
    {
      key: "relay:corp", kind: "relay", label: "corp.example", adapter: "newapi", refreshMinutes: 15,
      users: [
        { tool: "claude", providerId: "corp", providerName: "公司网关", active: true },
        { tool: "opencode", providerId: "corp-oc", providerName: "公司网关", active: false },
      ],
      snapshot: balance(31.4),
    },
    {
      key: "relay:spare", kind: "relay", label: "spare.example", adapter: "sub2api",
      users: [{ tool: "claude", providerId: "spare", providerName: "备用站", active: false }],
      snapshot: balance(0, "out"),
    },
  ];
}

function start(extra: Record<string, Handler> = {}) {
  const state = Object.fromEntries(
    TOOLS.map((tool) => [tool.id, { tool: tool.id, kind: tool.kind, providers: [], current: null, inLive: [], liveError: null, paths: [] }]),
  ) as unknown as Record<ToolId, ToolProviders>;
  state.claude.providers = [relay("corp", "公司网关"), relay("spare", "备用站"), relay("fresh", "新中转")];
  state.claude.current = "corp";
  state.codex.providers = [{ id: "official", name: "Codex 官方", category: "official", settingsConfig: { auth: {}, config: "" } }];
  state.codex.current = "official";
  const view: QuotaView = { checkedAt: now, busiest: "relay:corp", sources: sources() };
  const fake = fakeBackend({
    get_overview: (): ToolSummary[] =>
      TOOLS.map((tool) => ({ tool: tool.id, kind: tool.kind, paths: [], detected: ["claude", "codex"].includes(tool.id), cardCount: state[tool.id].providers.length, current: state[tool.id].current, currentName: null, inLive: state[tool.id].inLive })),
    list_providers: (a) => structuredClone(state[a!.tool as ToolId]),
    quota_overview: () => structuredClone(view),
    quota_refresh: () => structuredClone(view),
    ...extra,
  });
  backend.invoke = fake.invoke;
  return { fake, state };
}

beforeEach(() => vi.clearAllMocks());

describe("Quota on the home page", () => {
  it("puts a meter and text after the current provider and small figures on the alternatives", async () => {
    start();
    renderWithClient(<HomePage mode="list" onNavigate={vi.fn()} />);
    const claude = (await screen.findByRole("button", { name: "Claude Code 正在用 公司网关，点击换一个" })).closest("li")!;
    await within(claude).findByText("余 $31.40");
    const mark = claude.querySelector<HTMLElement>(".q")!;
    expect(mark.title).toContain("corp.example");
    expect(mark).toHaveTextContent("余 $31.40");
    expect(mark).toHaveTextContent("/ $50.00");
    expect(mark.querySelector(".quota-meter")).not.toBeNull();

    const codex = screen.getByRole("button", { name: "Codex 正在用 Codex 官方，点击换一个" }).closest("li")!;
    // 本机记录超过半小时：标出时间。
    expect(within(codex).getByText("本周 51%")).toBeInTheDocument();
    expect(within(codex).getByText("42 分钟前")).toBeInTheDocument();

    await userEvent.click(within(claude).getByRole("button", { name: "Claude Code 正在用 公司网关，点击换一个" }));
    const spare = within(claude).getByRole("button", { name: "Claude Code 切换到 备用站" });
    expect(within(spare).getByText("余 $0.00")).toHaveAttribute("data-level", "out");
  });

  it("offers a usage check for a relay that was never set up and saves what was detected", async () => {
    const { fake, state } = start({
      quota_detect: () => ({
        steps: [{ adapter: "sub2api", url: "https://fresh.example/v1/usage", outcome: "hit", status: 200 }],
        adapter: "sub2api",
        key: "relay:fresh",
        snapshot: balance(12.06),
      }),
      quota_configure: () => null,
    });
    state.claude.current = "fresh";
    renderWithClient(<HomePage mode="list" onNavigate={vi.fn()} />);
    await userEvent.click(await screen.findByRole("button", { name: "查询额度" }));
    const dialog = await screen.findByRole("dialog", { name: "识别 新中转 的额度接口" });
    await userEvent.click(await within(dialog).findByRole("button", { name: "使用 sub2api" }));
    await waitFor(() => expect(fake.calls.find((c) => c.cmd === "quota_configure")?.args).toEqual({ tool: "claude", id: "fresh", settings: { refreshMinutes: 0, adapter: "sub2api" } }));
    expect(fake.calls.find((c) => c.cmd === "quota_detect")?.args).toEqual({ tool: "claude", settingsConfig: state.claude.providers[2].settingsConfig, quotaPerUsd: null });
  });

  it("refreshes from the header without querying anything on its own", async () => {
    const { fake } = start();
    renderWithClient(<HomePage mode="list" onNavigate={vi.fn()} />);
    const button = await screen.findByRole("button", { name: "额度 · 3 分钟前" });
    expect(fake.calls.some((c) => c.cmd === "quota_refresh" || c.cmd === "quota_detect")).toBe(false);
    await userEvent.click(button);
    await waitFor(() => expect(fake.calls.find((c) => c.cmd === "quota_refresh")?.args).toEqual({ keys: null }));
  });
});

describe("Quota tab on the stats page", () => {
  it("opens on usage limits, lists what needs attention first, and only scans history when asked", async () => {
    const { fake } = start({
      usage_report: () => ({ rows: [], events: [], sources: [], scannedAt: now }),
    });
    renderWithClient(<StatsPage />);
    expect(await screen.findByRole("tab", { name: "额度" })).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByText(/^3 个额度来源/)).toBeInTheDocument();
    const attention = screen.getByRole("list", { name: "1 个需要注意" });
    // 备用站只是备选，但它用完了：切过去就会失败，所以也列出来。
    expect(within(attention).getByText("备用站")).toBeInTheDocument();
    expect(within(attention).queryByText("me@example.com")).not.toBeInTheDocument();

    const relays = screen.getByRole("region", { name: "中转余额" });
    const corp = within(relays).getByTestId("quota-relay:corp");
    expect(within(corp).getByText("NewAPI · 按 Key 查询 · corp.example")).toBeInTheDocument();
    expect(within(corp).getByText(/共用同一个 Key/)).toBeInTheDocument();
    expect(within(corp).getByText("每 15 分钟自动刷新")).toBeInTheDocument();
    expect(within(corp).getByText("$31.40")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "订阅窗口" })).getByText("Codex 官方 · Pro")).toBeInTheDocument();

    await userEvent.click(within(corp).getByRole("button", { name: "刷新 公司网关 的额度" }));
    await waitFor(() => expect(fake.calls.find((c) => c.cmd === "quota_refresh")?.args).toEqual({ keys: ["relay:corp"] }));
    expect(fake.calls.some((c) => c.cmd === "usage_report")).toBe(false);

    await userEvent.click(screen.getByRole("tab", { name: "用量记录" }));
    await waitFor(() => expect(fake.calls.some((c) => c.cmd === "usage_report")).toBe(true));
  });

  it("explains where data will come from when there is none yet", async () => {
    start({ quota_overview: () => ({ checkedAt: now, sources: [] }) });
    renderWithClient(<StatsPage initialTab="quota" />);
    expect(await screen.findByText("还没有额度数据")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /额度 ·|刷新额度/ })).not.toBeInTheDocument();
  });
});
