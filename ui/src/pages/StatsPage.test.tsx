import { beforeEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ToolId, ToolProviders, UsageReport, UsageRow } from "@/lib/api";
import { dayKey } from "@/lib/usage";
import { TOOLS } from "@/lib/tools";
import { fakeBackend, renderWithClient } from "@/test/utils";

const backend = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => backend.invoke(...args) }));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn() } }));

import { StatsPage } from "./StatsPage";

const DAY = 86_400_000;
const now = new Date(2026, 9, 13, 12).getTime();

function row(daysAgo: number, tool: ToolId, provider: string, input: number, cacheRead: number, attribution: UsageRow["attribution"] = "time"): UsageRow {
  return { day: dayKey(new Date(now - daysAgo * DAY)), tool, provider, model: "m", attribution, input, cacheRead, cacheWrite: 0, output: 10, requests: 2 };
}

function report(): UsageReport {
  return {
    scannedAt: now,
    rows: [row(1, "claude", "kimi", 100, 900), row(2, "claude", "relay", 600, 400), row(40, "claude", "", 50, 50, "none"), row(1, "opencode", "openrouter", 20, 0, "exact")],
    events: [
      { ts: now - 20 * DAY, tool: "claude", provider: "relay", name: "Relay", enabled: true },
      { ts: now - 1.5 * DAY, tool: "claude", provider: "kimi", name: "Kimi", enabled: true },
      { ts: now - 20 * DAY, tool: "opencode", provider: "openrouter", name: "OpenRouter", enabled: true },
    ],
    sources: [
      { tool: "claude", state: "read", tested: true, experimental: false, path: "/home/.claude/projects", retentionDays: 30, detail: null },
      { tool: "hermes", state: "missing", tested: false, experimental: false, path: "/home/.hermes/state.db", retentionDays: null, detail: null },
      { tool: "opencode", state: "read", tested: true, experimental: false, path: "/home/.local/share/opencode", retentionDays: null, detail: null },
    ],
  };
}

function start() {
  const lists = Object.fromEntries(TOOLS.map((t) => [t.id, { tool: t.id, kind: t.kind, providers: [], current: null, inLive: [], liveError: null, paths: [] }])) as unknown as Record<ToolId, ToolProviders>;
  lists.claude.providers = [{ id: "kimi", name: "Kimi", settingsConfig: {} }, { id: "relay", name: "公司网关", settingsConfig: {} }];
  lists.claude.current = "kimi";
  lists.opencode.providers = [{ id: "openrouter", name: "OpenRouter", settingsConfig: {} }];
  lists.opencode.inLive = ["openrouter"];
  const fake = fakeBackend({
    usage_report: report,
    list_providers: (a) => structuredClone(lists[a!.tool as ToolId]),
    get_overview: () => [],
    switch_provider: () => ({ changed: [], warnings: [] }),
  });
  backend.invoke = fake.invoke;
  renderWithClient(<StatsPage initialTab="usage" />);
  return fake;
}

beforeEach(() => vi.clearAllMocks());

describe("Stats page", () => {
  it("sums the selected range and flags providers whose cache barely hits", async () => {
    start();
    expect(await screen.findByRole("heading", { name: "统计" })).toBeInTheDocument();
    // 30 days: claude 1010 + 1010 tokens, opencode 30; the 40-day-old row is outside.
    const kpi = (await screen.findByText("全部 token", { selector: ".kpi-label" })).parentElement!;
    expect(kpi).toHaveTextContent("2,050");
    const relay = screen.getByText("公司网关", { selector: ".who span" }).closest("tr")!;
    expect(relay).toHaveTextContent("40.0%");
    expect(within(relay).getByText("偏低，缓存可能没生效")).toBeInTheDocument();
    expect(screen.getByText("Kimi", { selector: ".who span" }).closest("tr")).toHaveTextContent("90.0%");
    // OpenRouter used no cache at all: no hit rate, no warning.
    expect(screen.getByText("OpenRouter", { selector: ".who span" }).closest("tr")).not.toHaveTextContent("偏低");
  });

  it("widens the range and filters to one tool", async () => {
    start();
    await screen.findByText("公司网关", { selector: ".who span" });
    await userEvent.click(screen.getByRole("button", { name: "90 天" }));
    expect(screen.getByText("全部 token", { selector: ".kpi-label" }).parentElement!).toHaveTextContent("2,160");
    expect(screen.getByText("归不到供应商", { selector: ".who span" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Claude Code" }));
    expect(screen.getByRole("heading", { name: "每日用量 · Claude Code 按供应商" })).toBeInTheDocument();
    expect(screen.queryByText("OpenRouter", { selector: ".who span" })).not.toBeInTheDocument();
  });

  it("switches back when a past timeline segment is clicked", async () => {
    const fake = start();
    const past = await screen.findByRole("button", { name: /^Claude Code：公司网关，.*点击切回这个供应商$/ });
    await userEvent.click(past);
    await waitFor(() => expect(fake.invoke).toHaveBeenCalledWith("switch_provider", { tool: "claude", id: "relay" }));
  });

  it("lists every data source and marks untested ones", async () => {
    start();
    const sources = await screen.findByRole("region", { name: "数据来源" });
    expect(within(sources).getByText("Hermes").parentElement).toHaveTextContent("本机没有记录 · 未实测");
    expect(within(sources).getByText("工具默认只保留 30 天的原始记录")).toBeInTheDocument();
  });
});
