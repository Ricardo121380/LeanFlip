import { beforeEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { toast } from "sonner";
import type { Provider, ToolId, ToolProviders } from "@/lib/api";
import { syncSource } from "@/lib/sync";
import { TOOLS } from "@/lib/tools";
import { fakeBackend, renderWithClient } from "@/test/utils";

const backend = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => backend.invoke(...args) }));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn() } }));

import { SyncDialog } from "./SyncDialog";

const relay: Provider = {
  id: "relay",
  name: "拼车站",
  settingsConfig: {
    auth: { OPENAI_API_KEY: "sk-relay" },
    config: 'model = "gpt-5"\nmodel_provider = "relay"\n\n[model_providers.relay]\nbase_url = "https://relay.example.com/v1"\nwire_api = "responses"\n',
  },
};
const sameEndpoint: Provider = { id: "old", name: "旧中转", settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://relay.example.com", ANTHROPIC_AUTH_TOKEN: "sk-relay" } } };

function start() {
  const lists = Object.fromEntries(
    TOOLS.map((tool) => [tool.id, { tool: tool.id, kind: tool.kind, providers: [], current: null, inLive: [], liveError: null, paths: [] }]),
  ) as unknown as Record<ToolId, ToolProviders>;
  lists.claude.providers = [sameEndpoint];
  lists.codex.providers = [relay];
  const fake = fakeBackend({
    list_providers: (a) => lists[a!.tool as ToolId],
    probe_models: () => ({ ok: true, status: 200, latencyMs: 90, code: null, detail: null, endpoint: null, model: null, models: [{ id: "deepseek-chat" }, { id: "kimi-k2" }] }),
    probe_check: () => ({ ok: false, status: 404, latencyMs: 80, code: "not_found", detail: null, endpoint: null, model: "deepseek-chat", models: null }),
    add_provider: () => ({ changed: [], warnings: [] }),
    update_provider: () => ({ changed: [], warnings: [] }),
  });
  backend.invoke = fake.invoke;
  const onClose = vi.fn();
  renderWithClient(<SyncDialog source={syncSource("codex", relay)!} onClose={onClose} />);
  return { fake, onClose };
}

const calls = (fake: ReturnType<typeof fakeBackend>, cmd: string) => fake.calls.filter((call) => call.cmd === cmd).map((call) => call.args!);

beforeEach(() => vi.clearAllMocks());

describe("SyncDialog", () => {
  it("lists every other tool with its status and adds only the checked ones", async () => {
    const user = userEvent.setup();
    const { fake, onClose } = start();
    const dialog = await screen.findByRole("dialog", { name: "同步到其他 Agent" });
    const list = await within(dialog).findByRole("list");
    const row = (label: string) => within(list).getByRole("checkbox", { name: label }).closest("li")!;
    expect(within(list).queryByRole("checkbox", { name: "Codex" })).not.toBeInTheDocument();
    expect(within(row("Claude Code")).getByText("已存在：旧中转")).toBeInTheDocument();
    expect(within(list).getByRole("checkbox", { name: "Claude Code" })).toBeDisabled();
    expect(within(row("OpenClaw")).getByText("直接可用")).toBeInTheDocument();
    expect(within(row("OpenCode")).getByText("需要检测")).toBeInTheDocument();
    expect(within(row("OpenCode")).getByText(/OpenAI Chat Completions/)).toBeInTheDocument();
    expect(within(list).getAllByRole("checkbox").every((box) => box.getAttribute("aria-checked") === "false")).toBe(true);
    const add = within(dialog).getByRole("button", { name: /添加到 0 个工具/ });
    expect(add).toBeDisabled();

    await user.click(within(list).getByRole("checkbox", { name: "OpenClaw" }));
    expect(within(row("OpenClaw")).getByRole("combobox", { name: "OpenClaw 的模型" })).toHaveValue("gpt-5");
    await user.click(within(row("OpenClaw")).getByRole("checkbox", { name: "同时启用" }));

    await user.click(within(list).getByRole("checkbox", { name: "OpenCode" }));
    const model = within(row("OpenCode")).getByRole("combobox", { name: "OpenCode 的模型" });
    expect(model).toHaveValue("");
    expect(within(dialog).getByRole("button", { name: "添加到 2 个工具" })).toBeDisabled();
    await user.click(within(row("OpenCode")).getByRole("button", { name: "获取模型" }));
    await waitFor(() => expect(calls(fake, "probe_models")).toHaveLength(1));
    expect(calls(fake, "probe_models")[0].tool).toBe("opencode");
    await user.type(model, "deepseek-chat");
    await user.click(within(row("OpenCode")).getByRole("button", { name: "检测" }));
    expect(await within(row("OpenCode")).findByRole("status")).toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: "添加到 2 个工具" }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    const added = calls(fake, "add_provider") as { tool: ToolId; provider: Provider; addToLive: boolean }[];
    expect(added.map((call) => [call.tool, call.addToLive])).toEqual([
      ["opencode", false],
      ["openclaw", true],
    ]);
    const link = added[0].provider.meta?.leanflipLink;
    expect(link).toMatch(/^link-/);
    expect(added[1].provider.meta?.leanflipLink).toBe(link);
    expect(added[0].provider.settingsConfig).toMatchObject({ options: { baseURL: "https://relay.example.com/v1", apiKey: "sk-relay" }, models: { "deepseek-chat": {} } });
    const updated = calls(fake, "update_provider") as { tool: ToolId; provider: Provider }[];
    expect(updated).toHaveLength(1);
    expect(updated[0].tool).toBe("codex");
    expect(updated[0].provider.meta?.leanflipLink).toBe(link);
    expect(toast.success).toHaveBeenCalledWith("已添加到 2 个工具");
  });

  it("keeps the dialog open when a tool fails to add", async () => {
    const user = userEvent.setup();
    const { fake, onClose } = start();
    backend.invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "add_provider") throw "OpenClaw 配置坏了";
      return fake.invoke(cmd, args);
    });
    const list = await screen.findByRole("list");
    await user.click(within(list).getByRole("checkbox", { name: "OpenClaw" }));
    await user.click(screen.getByRole("button", { name: "添加到 1 个工具" }));
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("OpenClaw 添加失败：OpenClaw 配置坏了"));
    expect(onClose).not.toHaveBeenCalled();
    expect(calls(fake, "update_provider")).toHaveLength(0);
  });
});
