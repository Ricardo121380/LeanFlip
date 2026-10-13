import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fakeBackend, renderWithClient, type Handler } from "@/test/utils";
import type { ToolProviders } from "@/lib/api";

const backend: { invoke: ReturnType<typeof vi.fn> | null } = { invoke: null };
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => backend.invoke!(cmd, args),
}));

import { ToolPage } from "./ToolPage";

const providers: ToolProviders = {
  tool: "claude", kind: "switch", current: "official", inLive: [], liveError: null,
  paths: ["/home/me/.claude/settings.json"],
  providers: [
    { id: "official", name: "Official", settingsConfig: {} },
    { id: "backup-relay", name: "Relay", notes: "team quota", settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://moon.example/v1" } } },
    { id: "other", name: "Another", settingsConfig: {} },
  ],
};

function use(handlers: Record<string, Handler>) {
  const fake = fakeBackend(handlers);
  backend.invoke = fake.invoke;
  return fake;
}

beforeEach(async () => {
  backend.invoke = null;
  const { default: i18n } = await import("@/i18n");
  await i18n.changeLanguage("zh");
});

describe("ToolPage task states", () => {
  it("searches name, ID, endpoint and notes, with a recoverable no-results state", async () => {
    const user = userEvent.setup();
    const fake = use({ list_providers: () => providers });
    renderWithClient(<ToolPage tool="claude" />);
    const search = await screen.findByRole("searchbox", { name: "搜索名称、ID、地址、模型或备注" });
    expect(screen.getByRole("status")).toHaveTextContent("3 个供应商");
    expect(within(screen.getByTestId("row-backup-relay")).getByText("team quota")).toBeInTheDocument();
    for (const query of ["Relay", "BACKUP-RELAY", "moon.example", "quota"]) {
      await user.clear(search);
      await user.type(search, query);
      expect(screen.getByTestId("row-backup-relay")).toBeInTheDocument();
      expect(screen.queryByTestId("row-official")).not.toBeInTheDocument();
      expect(screen.getByRole("status")).toHaveTextContent("显示 1 / 3 个供应商");
    }
    await user.clear(search);
    await user.type(search, "missing");
    expect(screen.getByRole("heading", { name: "没有匹配的供应商" })).toBeInTheDocument();
    expect(screen.queryByText("还没有 Claude Code 供应商")).not.toBeInTheDocument();
    await user.click(screen.getByText("清除搜索"));
    expect(search).toHaveValue("");
    expect(search).toHaveFocus();
    expect(screen.getByTestId("row-official")).toBeInTheDocument();
    expect(fake.calls.every((call) => call.cmd === "list_providers")).toBe(true);
  });

  it("shows the client config paths and copies the exact path", async () => {
    const user = userEvent.setup();
    const copy = vi.spyOn(navigator.clipboard, "writeText");
    use({ list_providers: () => providers });
    renderWithClient(<ToolPage tool="claude" />);
    const paths = await screen.findByRole("list", { name: "配置文件" });
    expect(within(paths).getByText(providers.paths[0])).toBeInTheDocument();
    await user.click(within(paths).getByRole("button", { name: `复制路径：${providers.paths[0]}` }));
    expect(copy).toHaveBeenCalledWith(providers.paths[0]);
  });

  it("makes first-run import and add actions available without duplicate controls", async () => {
    const user = userEvent.setup();
    const fake = use({
      list_providers: () => ({ ...providers, current: null, providers: [] }),
      get_overview: () => [{ tool: "claude", kind: "switch", paths: providers.paths, detected: true, cardCount: 0, current: null, currentName: null, inLive: [] }],
      import_live: () => ({ created: [], warnings: [] }),
    });
    renderWithClient(<ToolPage tool="claude" />);
    expect(await screen.findByRole("heading", { name: "还没有 Claude Code 供应商" })).toBeInTheDocument();
    expect(screen.getByText(`检测到 ${providers.paths[0]}。先导入它，之后就能在这里切换。`)).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "从预设添加" })).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "添加供应商" })).not.toBeInTheDocument();
    expect(screen.queryByRole("searchbox")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "导入当前配置" }));
    await waitFor(() => expect(fake.calls.some((call) => call.cmd === "import_live")).toBe(true));
  });

  it("identifies the active provider and explains why it cannot be deleted", async () => {
    use({ list_providers: () => providers });
    renderWithClient(<ToolPage tool="claude" />);
    const current = await screen.findByTestId("row-official");
    expect(within(current).queryByRole("button", { name: /切换/ })).not.toBeInTheDocument();
    expect(within(current).getByText("使用中")).toBeInTheDocument();
    const remove = within(current).getByRole("button", { name: "删除 Official" });
    expect(remove).toBeDisabled();
    expect(remove).toHaveAttribute("title", "切换到其他供应商后才能删除");
  });

  it("locks conflicting actions during a switch and labels only its pending row", async () => {
    const user = userEvent.setup();
    let finish!: (result: { changed: string[]; warnings: string[] }) => void;
    use({ list_providers: () => providers, switch_provider: () => new Promise((resolve) => { finish = resolve; }) });
    renderWithClient(<ToolPage tool="claude" />);
    const relay = await screen.findByTestId("row-backup-relay");
    await user.click(within(relay).getByRole("button", { name: "切换到 Relay" }));
    expect(within(relay).getByRole("button", { name: "正在切换…" })).toBeDisabled();
    expect(within(screen.getByTestId("row-other")).getByRole("button", { name: "切换到 Another" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "编辑 Another" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "删除 Another" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "添加供应商" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "从当前配置导入" })).toBeDisabled();
    finish({ changed: [], warnings: [] });
    await waitFor(() => expect(screen.getByRole("button", { name: "编辑 Another" })).toBeEnabled());
  });

  it("explains that deleting a coexist provider also removes its client entry", async () => {
    const user = userEvent.setup();
    const fake = use({
      list_providers: () => ({ ...providers, tool: "opencode", kind: "coexist", current: null, inLive: ["backup-relay"] }),
      delete_provider: () => null,
    });
    renderWithClient(<ToolPage tool="opencode" />);
    await user.click(await screen.findByRole("button", { name: "删除 Relay" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText("删除这个供应商，并从客户端配置里移除对应条目（如果有）。")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(fake.calls.some((call) => call.cmd === "delete_provider")).toBe(false);
  });

  it("shows a labeled loading state and retries a failed query", async () => {
    const user = userEvent.setup();
    let reject!: (error: Error) => void;
    let attempts = 0;
    use({ list_providers: () => ++attempts === 1 ? new Promise((_resolve, fail) => { reject = fail; }) : providers });
    renderWithClient(<ToolPage tool="claude" />);
    expect(screen.getByRole("status")).toHaveAttribute("aria-busy", "true");
    expect(screen.getByRole("status")).toHaveTextContent("正在读取 Claude Code 配置…");
    reject(new Error("read denied"));
    expect(await screen.findByRole("heading", { name: "无法加载 Claude Code 供应商" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重试" }));
    expect(await screen.findByTestId("row-official")).toBeInTheDocument();
    expect(attempts).toBe(2);
  });

  it("preserves the open editor draft across background refresh failure and retry", async () => {
    const user = userEvent.setup();
    let attempts = 0;
    use({ list_providers: () => {
      attempts++;
      if (attempts === 2) throw new Error("temporary read failure");
      return providers;
    } });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(<QueryClientProvider client={client}><ToolPage tool="claude" /></QueryClientProvider>);
    await user.click(await screen.findByRole("button", { name: "编辑 Relay" }));
    const name = await screen.findByLabelText("名称");
    await user.clear(name);
    await user.type(name, "Unsaved draft");

    await act(async () => { await client.refetchQueries({ queryKey: ["providers", "claude"] }); });
    expect(await screen.findByText("无法刷新供应商列表")).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByLabelText("名称")).toBe(name);
    expect(name).toHaveValue("Unsaved draft");

    await act(async () => { await client.refetchQueries({ queryKey: ["providers", "claude"] }); });
    await waitFor(() => expect(screen.queryByText("无法刷新供应商列表")).not.toBeInTheDocument());
    expect(screen.getByLabelText("名称")).toBe(name);
    expect(name).toHaveValue("Unsaved draft");
    expect(attempts).toBe(3);
  });

  it("keeps cached providers visible and retries a failed background refresh in place", async () => {
    const user = userEvent.setup();
    let attempts = 0;
    use({ list_providers: () => {
      attempts++;
      if (attempts === 2) throw new Error("temporary read failure");
      return providers;
    } });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(<QueryClientProvider client={client}><ToolPage tool="claude" /></QueryClientProvider>);
    const card = await screen.findByTestId("row-backup-relay");
    await act(async () => { await client.refetchQueries({ queryKey: ["providers", "claude"] }); });
    expect(await screen.findByText("无法刷新供应商列表")).toBeInTheDocument();
    expect(screen.getByTestId("row-backup-relay")).toBe(card);
    await user.click(screen.getByRole("button", { name: "重试" }));
    await waitFor(() => expect(screen.queryByText("无法刷新供应商列表")).not.toBeInTheDocument());
    expect(screen.getByTestId("row-backup-relay")).toBe(card);
    expect(attempts).toBe(3);
  });

  it("counts enabled saved providers without including unimported client entries", async () => {
    use({ list_providers: () => ({ ...providers, tool: "opencode", kind: "coexist", current: null, inLive: ["backup-relay", "unimported"] }) });
    renderWithClient(<ToolPage tool="opencode" />);
    await screen.findByTestId("row-backup-relay");
    expect(screen.getByRole("status")).toHaveTextContent("3 个供应商 · 已写入 1 / 3");
  });

  it("does not present unreadable live state as a verified enabled count", async () => {
    use({ list_providers: () => ({ ...providers, tool: "opencode", kind: "coexist", current: null, liveError: "bad json" }) });
    renderWithClient(<ToolPage tool="opencode" />);
    expect(await screen.findByText(/配置读取失败，当前状态待确认/)).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("bad json");
    expect(screen.getByRole("button", { name: "重试" })).toBeInTheDocument();
  });

  it("duplicates a provider into a new editor", async () => {
    const user = userEvent.setup();
    use({ list_providers: () => providers, ignored_fields: () => [] });
    renderWithClient(<ToolPage tool="claude" />);
    await user.click(await screen.findByRole("button", { name: "复制 Relay" }));
    const dialog = await screen.findByRole("dialog", { name: "复制供应商 · Claude Code" });
    expect(within(dialog).getByLabelText("名称")).toHaveValue("Relay 副本");
    expect(within(dialog).getByLabelText("备注")).toHaveValue("");
    expect(within(dialog).getByLabelText("请求地址")).toHaveValue("https://moon.example/v1");
  });

  it("tests reachability for every non-official provider without sending keys to the list", async () => {
    const user = userEvent.setup();
    const fake = use({
      list_providers: () => providers,
      probe_latency: (args) => {
        const env = ((args!.settingsConfig as { env?: Record<string, string> }).env ?? {});
        return env.ANTHROPIC_BASE_URL
          ? { ok: true, status: 404, latencyMs: 820, code: null, detail: null, endpoint: env.ANTHROPIC_BASE_URL, model: null, models: null }
          : { ok: false, status: null, latencyMs: 0, code: "dns", detail: null, endpoint: null, model: null, models: null };
      },
    });
    renderWithClient(<ToolPage tool="claude" />);
    await user.click(await screen.findByRole("button", { name: "测连通" }));
    expect(await within(screen.getByTestId("row-backup-relay")).findByText("820 ms")).toHaveClass("text-warning");
    expect(within(screen.getByTestId("row-other")).getByText("不可连通")).toBeInTheDocument();
    expect(within(screen.getByTestId("row-other")).getByText("不可连通").closest("[title]")).toHaveAttribute("title", expect.stringContaining("域名解析失败"));
    expect(fake.calls.filter((c) => c.cmd === "probe_latency")).toHaveLength(3);
  });

  it("shows the current route from the config file to the active provider", async () => {
    use({ list_providers: () => ({ ...providers, current: "backup-relay" }) });
    renderWithClient(<ToolPage tool="claude" />);
    const route = await screen.findByRole("region", { name: "当前路由" });
    expect(within(route).getByRole("heading", { name: "Relay" })).toBeInTheDocument();
    expect(within(route).getByText(providers.paths[0])).toBeInTheDocument();
    expect(within(route).getByText("https://moon.example/v1")).toBeInTheDocument();
  });
});
