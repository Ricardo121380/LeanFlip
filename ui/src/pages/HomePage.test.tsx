import { beforeEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Provider, ToolId, ToolProviders, ToolSummary } from "@/lib/api";
import { TOOLS } from "@/lib/tools";
import { fakeBackend, renderWithClient } from "@/test/utils";

const backend = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => backend.invoke(...args) }));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn() } }));

import { HomePage, type HomeMode } from "./HomePage";

const claude = (id: string, name: string, base?: string): Provider => ({
  id,
  name,
  settingsConfig: { env: base ? { ANTHROPIC_BASE_URL: base, ANTHROPIC_MODEL: `${id}-model` } : {} },
  ...(base ? {} : { category: "official" }),
});
const opencode = (id: string, name: string): Provider => ({ id, name, settingsConfig: { options: { baseURL: `https://${id}.example/v1` } } });

let state: Record<ToolId, ToolProviders>;

function start(mode: HomeMode = "list") {
  state = Object.fromEntries(
    TOOLS.map((tool) => [tool.id, { tool: tool.id, kind: tool.kind, providers: [], current: null, inLive: [], liveError: null, paths: [] }]),
  ) as unknown as Record<ToolId, ToolProviders>;
  state.claude.providers = [claude("kimi", "Kimi", "https://api.moonshot.cn/anthropic"), claude("deepseek", "DeepSeek", "https://api.deepseek.com/anthropic"), claude("official", "Claude 官方")];
  state.claude.current = "kimi";
  state.opencode.providers = [opencode("openrouter", "OpenRouter"), opencode("moonshot", "Moonshot")];
  state.opencode.inLive = ["openrouter"];
  const detected = new Set<ToolId>(["claude", "codex", "opencode"]);
  const overview = (): ToolSummary[] =>
    TOOLS.map((tool) => ({ tool: tool.id, kind: tool.kind, paths: [], detected: detected.has(tool.id), cardCount: state[tool.id].providers.length, current: state[tool.id].current, currentName: null, inLive: state[tool.id].inLive }));
  const fake = fakeBackend({
    get_overview: overview,
    list_providers: (a) => structuredClone(state[a!.tool as ToolId]),
    switch_provider: (a) => {
      state[a!.tool as ToolId].current = String(a!.id);
      return { changed: [], warnings: [] };
    },
    set_in_live: (a) => {
      const tool = state[a!.tool as ToolId];
      tool.inLive = a!.enabled ? [...tool.inLive, String(a!.id)] : tool.inLive.filter((id) => id !== a!.id);
      return { changed: [], warnings: [] };
    },
    probe_latency: () => ({ ok: true, status: 404, latencyMs: 252, code: null, detail: null, endpoint: null, model: null, models: null }),
  });
  backend.invoke = fake.invoke;
  const onNavigate = vi.fn();
  renderWithClient(<HomePage mode={mode} onNavigate={onNavigate} />);
  return { fake, onNavigate };
}

beforeEach(() => vi.clearAllMocks());

describe("Home list", () => {
  it("reads each tool as a sentence and gathers tools that are not installed on one line", async () => {
    start();
    const current = await screen.findByRole("button", { name: "Claude Code 正在用 Kimi，点击换一个" });
    expect(current).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByText("kimi-model · https://api.moonshot.cn/anthropic")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "OpenCode 移除 OpenRouter" })).toBeInTheDocument();
    expect(screen.getByText("Codex").closest("li")).toHaveTextContent("还没有供应商");
    const missing = screen.getByText("本机未检测到：").parentElement!;
    expect(within(missing).getByRole("link", { name: "Gemini CLI" })).toHaveAttribute("href", "#tool=gemini");
  });

  it("switches by unfolding the other providers in place", async () => {
    const { fake } = start();
    await userEvent.click(await screen.findByRole("button", { name: "Claude Code 正在用 Kimi，点击换一个" }));
    expect(screen.getByRole("button", { name: "Claude Code 正在用 Kimi，点击收起" })).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("button", { name: "Claude Code 切换到 Claude 官方" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Claude Code 切换到 DeepSeek" }));
    expect(fake.invoke).toHaveBeenCalledWith("switch_provider", { tool: "claude", id: "deepseek" });
    expect(await screen.findByRole("button", { name: "Claude Code 正在用 DeepSeek，点击换一个" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Claude Code 切换到 Kimi" })).not.toBeInTheDocument();
  });

  it("folds the providers back with Escape", async () => {
    start();
    await userEvent.click(await screen.findByRole("button", { name: "Claude Code 正在用 Kimi，点击换一个" }));
    expect(screen.getByRole("button", { name: "Claude Code 切换到 DeepSeek" })).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("button", { name: "Claude Code 切换到 DeepSeek" })).not.toBeInTheDocument();
  });

  it("connects and removes providers of a coexist tool from the sentence", async () => {
    const { fake } = start();
    await userEvent.click(await screen.findByRole("button", { name: "OpenCode 接入 Moonshot" }));
    expect(fake.invoke).toHaveBeenCalledWith("set_in_live", { tool: "opencode", id: "moonshot", enabled: true });
    await userEvent.click(await screen.findByRole("button", { name: "OpenCode 移除 OpenRouter" }));
    expect(fake.invoke).toHaveBeenCalledWith("set_in_live", { tool: "opencode", id: "openrouter", enabled: false });
    await waitFor(() => expect(screen.getByRole("button", { name: "OpenCode 接入 OpenRouter" })).toBeInTheDocument());
  });

  it("tests every provider that has an address and shows the result next to the one in use", async () => {
    const { fake } = start();
    await screen.findByRole("button", { name: "Claude Code 正在用 Kimi，点击换一个" });
    await userEvent.click(screen.getByRole("button", { name: "测连通" }));
    await waitFor(() => expect(fake.calls.filter((c) => c.cmd === "probe_latency")).toHaveLength(4));
    const current = screen.getByRole("button", { name: "Claude Code 正在用 Kimi，点击换一个" });
    await waitFor(() => expect(current).toHaveTextContent("252 ms"), { timeout: 2000 });
  });
});

describe("Home map", () => {
  it("draws one route per provider and switches when a faint route is clicked", async () => {
    const { fake } = start("map");
    const route = await screen.findByRole("button", { name: "Claude Code 切换到 DeepSeek" });
    expect(screen.getByRole("button", { name: "Claude Code 正在用 Kimi" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText("api.deepseek.com")).toBeInTheDocument();
    expect(screen.getByText("Claude Code 官方登录")).toBeInTheDocument();
    await userEvent.click(route);
    expect(fake.invoke).toHaveBeenCalledWith("switch_provider", { tool: "claude", id: "deepseek" });
  });
});
