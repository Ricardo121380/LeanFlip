import { beforeEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { fakeBackend, renderWithClient } from "@/test/utils";
import type { ToolId, ToolProviders } from "@/lib/api";

const backend = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => backend.invoke(...args) }));

import { CommandPalette } from "./CommandPalette";

const empty = (tool: ToolId, kind: "switch" | "coexist" = "switch"): ToolProviders => ({ tool, kind, providers: [], current: null, inLive: [], liveError: null, paths: [] });
const lists: Partial<Record<ToolId, ToolProviders>> = {
  claude: {
    ...empty("claude"),
    current: "kimi",
    providers: [
      { id: "kimi", name: "Kimi", settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://api.moonshot.cn/anthropic" } } },
      { id: "deepseek", name: "DeepSeek", settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://api.deepseek.com/anthropic" } } },
    ],
  },
  opencode: { ...empty("opencode", "coexist"), inLive: [], providers: [{ id: "router", name: "OpenRouter", settingsConfig: {} }] },
};

let fake: ReturnType<typeof fakeBackend>;
beforeEach(async () => {
  fake = fakeBackend({
    list_providers: (args) => lists[args!.tool as ToolId] ?? empty(args!.tool as ToolId, ["opencode", "openclaw", "hermes", "pi", "mcode"].includes(String(args!.tool)) ? "coexist" : "switch"),
    switch_provider: () => ({ changed: [], warnings: [] }),
    set_in_live: () => ({ changed: [], warnings: [] }),
  });
  backend.invoke = fake.invoke;
  const { default: i18n } = await import("@/i18n");
  await i18n.changeLanguage("zh");
});

describe("CommandPalette", () => {
  it("filters providers by endpoint and switches the chosen one", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    const onOpenChange = vi.fn();
    renderWithClient(<CommandPalette open onOpenChange={onOpenChange} onNavigate={onNavigate} />);
    const input = screen.getByRole("combobox");
    await screen.findByRole("option", { name: /Claude Code › DeepSeek/ });
    await user.type(input, "deepseek.com");
    const options = screen.getAllByRole("option");
    expect(options).toHaveLength(1);
    expect(options[0]).toHaveTextContent("切换");
    await user.keyboard("{Enter}");
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(onNavigate).toHaveBeenCalledWith("#tool=claude");
    await waitFor(() => expect(fake.calls.find((call) => call.cmd === "switch_provider")?.args).toEqual({ tool: "claude", id: "deepseek" }));
  });

  it("does not switch to the provider that is already in use", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    renderWithClient(<CommandPalette open onOpenChange={() => {}} onNavigate={onNavigate} />);
    await screen.findByRole("option", { name: /Claude Code › Kimi/ });
    await user.type(screen.getByRole("combobox"), "kimi");
    expect(screen.getByRole("option", { name: /Claude Code › Kimi/ })).toHaveTextContent("使用中");
    await user.keyboard("{Enter}");
    expect(onNavigate).toHaveBeenCalledWith("#tool=claude");
    expect(fake.calls.some((call) => call.cmd === "switch_provider")).toBe(false);
  });

  it("toggles a coexist provider and offers page navigation", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    renderWithClient(<CommandPalette open onOpenChange={() => {}} onNavigate={onNavigate} />);
    await user.type(screen.getByRole("combobox"), "openrouter");
    await user.click(await screen.findByRole("option", { name: /OpenCode › OpenRouter/ }));
    await waitFor(() => expect(fake.calls.find((call) => call.cmd === "set_in_live")?.args).toEqual({ tool: "opencode", id: "router", enabled: true }));

    await user.clear(screen.getByRole("combobox"));
    await user.type(screen.getByRole("combobox"), "设置");
    await user.click(screen.getByRole("option", { name: /设置/ }));
    expect(onNavigate).toHaveBeenLastCalledWith("#settings");
  });
});
