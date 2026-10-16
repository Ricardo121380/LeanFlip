import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { toast } from "sonner";
import type { SceneList, SceneReport, ScenePreview, ToolId, ToolProviders } from "@/lib/api";
import { TOOLS } from "@/lib/tools";
import { fakeBackend, renderWithClient } from "@/test/utils";

const backend = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => backend.invoke(...args) }));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn() } }));
const events = vi.hoisted(() => ({ handlers: new Map<string, (event: { payload: unknown }) => void>() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
    events.handlers.set(name, handler);
    // 只取消自己的订阅：上一个测试卸载后的订阅可能晚到。
    return () => events.handlers.get(name) === handler && events.handlers.delete(name);
  },
}));

import { ScenesProvider } from "./Scenes";
import { CommandPalette } from "./CommandPalette";
import { SceneMenu } from "@/pages/home/SceneMenu";

const lists = (): Record<ToolId, ToolProviders> => {
  const out = Object.fromEntries(
    TOOLS.map((tool) => [tool.id, { tool: tool.id, kind: tool.kind, providers: [], current: null, inLive: [], liveError: null, paths: [] }]),
  ) as unknown as Record<ToolId, ToolProviders>;
  out.claude.providers = [
    { id: "kimi", name: "Kimi", settingsConfig: {} },
    { id: "deepseek", name: "DeepSeek", settingsConfig: {} },
    { id: "relay", name: "公司网关", settingsConfig: {} },
  ];
  out.claude.current = "kimi";
  out.opencode.providers = [
    { id: "openrouter", name: "OpenRouter", settingsConfig: {} },
    { id: "moonshot", name: "Moonshot", settingsConfig: {} },
  ];
  out.opencode.inLive = ["openrouter"];
  return out;
};

const workHours = { kind: "time" as const, days: [1, 2, 3, 4, 5], start: "09:00", end: "18:00" };

const sceneList = (): SceneList => ({
  last: "scene-home",
  scenes: [
    { id: "scene-home", name: "个人", tools: { claude: { current: "kimi" }, opencode: { enabled: ["openrouter"] } }, status: "drifted", pending: ["claude"], missing: [] },
    {
      id: "scene-work",
      name: "公司",
      tools: { claude: { current: "relay" }, codex: { current: "official" }, opencode: { enabled: ["moonshot"] }, gemini: { current: "gapi" } },
      rules: [workHours, { kind: "subnet", cidr: "10.20.0.0/16" }],
      status: "idle",
      pending: ["claude", "opencode"],
      missing: [],
      suggested: true,
    },
  ],
});

const workPreview = (): ScenePreview => ({
  scene: sceneList().scenes[1],
  tools: [
    { tool: "claude", kind: "switch", from: "Kimi", steps: [{ action: { kind: "switch", id: "relay" }, name: "公司网关" }], missing: [], error: null },
    { tool: "codex", kind: "switch", from: "Codex 官方", steps: [], missing: [], error: null },
    { tool: "gemini", kind: "switch", from: null, steps: [{ action: { kind: "switch", id: "gapi" }, name: "Gemini API" }], missing: [], error: "配置读不了" },
    {
      tool: "opencode",
      kind: "coexist",
      from: null,
      steps: [
        { action: { kind: "enable", id: "moonshot" }, name: "Moonshot" },
        { action: { kind: "disable", id: "openrouter" }, name: "OpenRouter" },
      ],
      missing: [],
      error: null,
    },
  ],
});

const undoSnapshot = { tools: { claude: { current: "kimi" } }, last: "scene-home" };
let applyResult: SceneReport;

function start(ui = <SceneMenu />) {
  applyResult = {
    results: [{ tool: "claude", outcome: "applied", changed: ["/home/.claude/settings.json"], warnings: [], missing: [], error: null }],
    undo: undoSnapshot,
  };
  const state = lists();
  const fake = fakeBackend({
    list_scenes: () => sceneList(),
    list_providers: (a) => structuredClone(state[a!.tool as ToolId]),
    preview_scene: (a) => {
      expect(a!.id).toBe("scene-work");
      return workPreview();
    },
    apply_scene: () => applyResult,
    undo_scene: () => ({ results: [{ tool: "claude", outcome: "applied", changed: [], warnings: [], missing: [], error: null }], undo: { tools: {}, last: "scene-work" } }),
    capture_scene: () => ({ id: "", name: "", tools: { claude: { current: "kimi" }, opencode: { enabled: ["openrouter"] } } }),
    save_scene: (a) => ({ ...(a!.scene as object), id: "scene-new" }),
    delete_scene: () => null,
  });
  backend.invoke = fake.invoke;
  renderWithClient(<ScenesProvider>{ui}</ScenesProvider>);
  return fake;
}

const calls = (fake: ReturnType<typeof fakeBackend>, cmd: string) => fake.calls.filter((call) => call.cmd === cmd).map((call) => call.args);

beforeEach(() => vi.clearAllMocks());

describe("Scene menu and apply dialog", () => {
  it("shows the drifted last scene and applies a previewed scene with undo", async () => {
    const user = userEvent.setup();
    const fake = start();
    const button = await screen.findByRole("button", { name: "场景：个人，已偏离，建议切换到「公司」" });
    await user.click(button);
    const menu = screen.getByRole("menu");
    expect(within(menu).getAllByRole("menuitem").map((item) => item.textContent)).toEqual(["个人已偏离Ctrl+Shift+1", "公司建议Ctrl+Shift+2", "把当前状态存为场景…", "管理场景…"]);
    await user.click(within(menu).getByRole("menuitem", { name: /公司/ }));
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();

    const dialog = await screen.findByRole("dialog", { name: "应用场景「公司」" });
    const rows = await within(dialog).findByRole("list");
    expect(within(rows).getByText("Kimi → 公司网关")).toBeInTheDocument();
    expect(within(rows).getByText("接入 Moonshot · 移出 OpenRouter")).toBeInTheDocument();
    expect(within(rows).getByText(/读不了配置/)).toBeInTheDocument();
    expect(within(dialog).getByText("已一致：Codex")).toBeInTheDocument();
    expect(within(dialog).getByRole("checkbox", { name: "Gemini CLI" })).toBeDisabled();

    await user.click(within(dialog).getByRole("checkbox", { name: "OpenCode" }));
    await user.click(within(dialog).getByRole("button", { name: "应用" }));
    await waitFor(() => expect(calls(fake, "apply_scene")).toEqual([{ id: "scene-work", tools: ["claude"] }]));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());

    const [message, options] = vi.mocked(toast.success).mock.calls[0] as [string, { action: { label: string; onClick: () => void } }];
    expect(message).toBe("已切换到场景「公司」");
    expect(options.action.label).toBe("撤销");
    options.action.onClick();
    await waitFor(() => expect(calls(fake, "undo_scene")).toEqual([{ undo: undoSnapshot }]));
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("已撤销场景切换"));
  });

  it("keeps the results open when a tool fails and can undo the rest", async () => {
    const user = userEvent.setup();
    const fake = start();
    applyResult.results.push({ tool: "opencode", outcome: "failed", changed: [], warnings: [], missing: [], error: "opencode.json 格式错误", partial: true });
    await user.click(await screen.findByRole("button", { name: /场景：个人/ }));
    await user.click(screen.getByRole("menuitem", { name: /公司/ }));
    const dialog = await screen.findByRole("dialog", { name: "应用场景「公司」" });
    await within(dialog).findByText("Kimi → 公司网关");
    await user.click(within(dialog).getByRole("button", { name: "应用" }));

    const result = await screen.findByRole("dialog", { name: "场景「公司」只应用了一部分" });
    expect(within(result).getByText("opencode.json 格式错误")).toBeInTheDocument();
    expect(within(result).getByText("失败")).toBeInTheDocument();
    expect(within(result).getByText("没能改回原样，可能改了一部分")).toBeInTheDocument();
    expect(toast.success).not.toHaveBeenCalled();
    await user.click(within(result).getByRole("button", { name: "撤销" }));
    await waitFor(() => expect(calls(fake, "undo_scene")).toHaveLength(1));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("opens from the keyboard and returns focus on Escape", async () => {
    const user = userEvent.setup();
    start();
    const button = await screen.findByRole("button", { name: /场景：个人/ });
    button.focus();
    await user.keyboard("{ArrowDown}");
    const items = screen.getAllByRole("menuitem");
    await waitFor(() => expect(items[0]).toHaveFocus());
    await user.keyboard("{ArrowUp}");
    expect(items[3]).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(button).toHaveFocus();
  });

  it("applies a scene from the command palette after the preview", async () => {
    const user = userEvent.setup();
    start(<CommandPalette open onOpenChange={() => {}} onNavigate={() => {}} />);
    const option = await screen.findByRole("option", { name: /应用场景：公司/ });
    expect(screen.getByRole("option", { name: /应用场景：个人/ })).toHaveTextContent("应用");
    await user.click(option);
    expect(await screen.findByRole("dialog", { name: "应用场景「公司」" })).toBeInTheDocument();
  });
});

describe("Scenes applied from the tray", () => {
  it("shows an undoable notice on success, the results on failure and the error otherwise", async () => {
    const fake = start();
    await waitFor(() => expect(events.handlers.has("scene-applied")).toBe(true));
    const emit = (payload: unknown) => act(() => events.handlers.get("scene-applied")!({ payload }));
    await waitFor(() => expect(events.handlers.has("scene-suggested")).toBe(true));
    const report: SceneReport = { results: [{ tool: "claude", outcome: "applied", changed: [], warnings: ["注意"], missing: [], error: null }], undo: undoSnapshot };

    emit({ id: "scene-work", name: "公司", report, error: null });
    expect(toast.warning).toHaveBeenCalledWith("Claude Code: 注意");
    const [message, options] = vi.mocked(toast.success).mock.calls[0] as unknown as [string, { action: { onClick: () => void } }];
    expect(message).toBe("已切换到场景「公司」");
    options.action.onClick();
    await waitFor(() => expect(calls(fake, "undo_scene")).toEqual([{ undo: undoSnapshot }]));

    emit({ id: "scene-work", name: "公司", report: { ...report, results: [...report.results, { tool: "opencode", outcome: "failed", changed: [], warnings: [], missing: [], error: "坏了" }] }, error: null });
    const result = await screen.findByRole("dialog", { name: "场景「公司」只应用了一部分" });
    expect(within(result).getByText("坏了")).toBeInTheDocument();

    emit({ id: "scene-work", name: "公司", report: null, error: "找不到场景" });
    expect(toast.error).toHaveBeenCalledWith("找不到场景");
  });
});

describe("Scene shortcuts and reminders", () => {
  it("applies the scene at the pressed position directly, with undo", async () => {
    const fake = start();
    applyResult.irreversible = ["gemini"];
    applyResult.results[0].missing = ["gone"];
    await screen.findByRole("button", { name: /场景：个人/ });
    fireEvent.keyDown(window, { code: "Digit2", ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(calls(fake, "apply_scene")).toEqual([{ id: "scene-work", tools: null }]));
    await waitFor(() =>
      expect(toast.success).toHaveBeenCalledWith(
        "已切换到场景「公司」",
        expect.objectContaining({
          description: "跳过了已删除的供应商：Claude Code（gone） 撤销不会改回：Gemini CLI（应用前没有选供应商）",
          action: expect.objectContaining({ label: "撤销" }),
        }),
      ),
    );
    fireEvent.keyDown(window, { code: "Digit3", ctrlKey: true, shiftKey: true });
    fireEvent.keyDown(window, { code: "Digit1", shiftKey: true });
    expect(calls(fake, "apply_scene")).toHaveLength(1);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("reminds once per backend event and previews the suggested scene", async () => {
    start();
    await waitFor(() => expect(events.handlers.has("scene-suggested")).toBe(true));
    const emit = (payload: unknown) => act(() => events.handlers.get("scene-suggested")!({ payload }));
    emit({ id: "scene-work", name: "公司", rule: workHours });
    const [message, options] = vi.mocked(toast.info).mock.calls[0] as unknown as [string, { action: { label: string; onClick: () => void } }];
    expect(message).toBe("建议切换到场景「公司」：工作日 09:00–18:00");
    expect(options.action.label).toBe("预览");
    act(() => options.action.onClick());
    expect(await screen.findByRole("dialog", { name: "应用场景「公司」" })).toBeInTheDocument();
    emit(null);
    expect(toast.info).toHaveBeenCalledTimes(1);
  });

  it("edits reminder rules with inline validation", async () => {
    const user = userEvent.setup();
    const fake = start();
    await user.click(await screen.findByRole("button", { name: /场景：个人/ }));
    await user.click(screen.getByRole("menuitem", { name: "管理场景…" }));
    const dialog = await screen.findByRole("dialog", { name: "管理场景" });
    expect(within(dialog).getByText("工作日 09:00–18:00、网段 10.20.0.0/16")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "编辑场景「公司」" }));
    const rules = await within(dialog).findByRole("list", { name: "提醒规则" });
    expect(within(rules).getAllByRole("listitem").map((item) => item.textContent)).toEqual(["工作日 09:00–18:00", "网段 10.20.0.0/16"]);
    await user.click(within(rules).getByRole("button", { name: "删除规则：网段 10.20.0.0/16" }));

    await user.selectOptions(within(dialog).getByRole("combobox", { name: "规则类型" }), "host");
    const add = within(dialog).getByRole("button", { name: "添加规则" });
    expect(add).toBeDisabled();
    await user.type(within(dialog).getByRole("textbox", { name: "域名" }), "http://x");
    expect(within(dialog).getByRole("alert")).toHaveTextContent("不带 http://");
    expect(add).toBeDisabled();
    await user.clear(within(dialog).getByRole("textbox", { name: "域名" }));
    await user.type(within(dialog).getByRole("textbox", { name: "域名" }), "intranet.corp");
    await user.click(add);

    await user.selectOptions(within(dialog).getByRole("combobox", { name: "规则类型" }), "time");
    await user.click(within(dialog).getByRole("button", { name: "周六", pressed: false }));
    await user.click(within(dialog).getByRole("button", { name: "添加规则" }));
    expect(within(rules).getAllByRole("listitem").map((item) => item.textContent)).toEqual(["工作日 09:00–18:00", "intranet.corp 能解析", "周一、周二、周三、周四、周五、周六 09:00–18:00"]);

    await user.click(within(dialog).getByRole("button", { name: "保存" }));
    await waitFor(() => expect(calls(fake, "save_scene")).toHaveLength(1));
    const saved = calls(fake, "save_scene")[0]!.scene as { rules: unknown[]; suggested?: boolean };
    expect(saved.rules).toEqual([workHours, { kind: "host", host: "intranet.corp" }, { ...workHours, days: [1, 2, 3, 4, 5, 6] }]);
    expect("suggested" in saved).toBe(false);
  });
});

describe("Scene manager", () => {
  it("saves the current setup as a new scene after adjusting a tool", async () => {
    const user = userEvent.setup();
    const fake = start();
    await user.click(await screen.findByRole("button", { name: /场景：个人/ }));
    await user.click(screen.getByRole("menuitem", { name: "把当前状态存为场景…" }));
    const dialog = await screen.findByRole("dialog", { name: "新建场景" });
    const save = within(dialog).getByRole("button", { name: "保存" });
    const claude = await within(dialog).findByRole("combobox", { name: "Claude Code 的供应商" });
    expect(claude).toHaveValue("kimi");
    expect(save).toBeDisabled();
    await user.type(within(dialog).getByLabelText("名称"), "出差");
    await user.selectOptions(claude, "deepseek");
    await user.click(within(dialog).getByRole("checkbox", { name: "Moonshot" }));
    await user.click(save);
    await waitFor(() =>
      expect(calls(fake, "save_scene")).toEqual([
        { scene: { id: "", name: "出差", tools: { claude: { current: "deepseek" }, opencode: { enabled: ["openrouter", "moonshot"] } } } },
      ]),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(toast.success).toHaveBeenCalledWith("已保存场景「出差」");
  });

  it("edits without saving list status and deletes after confirmation", async () => {
    const user = userEvent.setup();
    const fake = start();
    await user.click(await screen.findByRole("button", { name: /场景：个人/ }));
    await user.click(screen.getByRole("menuitem", { name: "管理场景…" }));
    const dialog = await screen.findByRole("dialog", { name: "管理场景" });
    await user.click(await within(dialog).findByRole("button", { name: "编辑场景「个人」" }));
    expect(await within(dialog).findByLabelText("名称")).toHaveValue("个人");
    await user.click(within(dialog).getByRole("checkbox", { name: "包含 OpenCode" }));
    await user.click(within(dialog).getByRole("button", { name: "保存" }));
    await waitFor(() => expect(calls(fake, "save_scene")).toEqual([{ scene: { id: "scene-home", name: "个人", tools: { claude: { current: "kimi" } } } }]));
    expect(await within(dialog).findByRole("button", { name: "删除场景「公司」" })).toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: "删除场景「公司」" }));
    const confirm = await screen.findByRole("alertdialog", { name: "删除场景「公司」？" });
    await user.click(within(confirm).getByRole("button", { name: "删除" }));
    await waitFor(() => expect(calls(fake, "delete_scene")).toEqual([{ id: "scene-work" }]));
    expect(toast.success).toHaveBeenCalledWith("已删除场景「公司」");
  });
});
