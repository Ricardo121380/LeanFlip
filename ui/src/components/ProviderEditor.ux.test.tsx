import { beforeEach, describe, expect, it, vi } from "vitest";
import { StrictMode, useState } from "react";
import { act, fireEvent, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { fakeBackend, renderWithClient, type Handler } from "@/test/utils";
import type { Provider } from "@/lib/api";

const mock = vi.hoisted(() => ({ invoke: vi.fn(), loadPresets: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => mock.invoke(...args) }));
vi.mock("@/lib/presets", async (original) => ({ ...await original<object>(), loadPresets: () => mock.loadPresets() }));
import { ProviderEditor } from "./ProviderEditor";

const saved: Provider = { id: "existing", name: "Existing", settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://example.com" } } };
const presets = [{ name: "First", settingsConfig: { env: {} } }, { name: "Second", settingsConfig: { env: {} } }];
function mount(provider: Provider | null = saved, handlers: Record<string, Handler> = {}, extra: { template?: Provider; tool?: "claude" | "codex" } = {}) {
  const backend = fakeBackend({ ignored_fields: () => [], ...handlers });
  mock.invoke = backend.invoke;
  function Host() {
    const [open, setOpen] = useState(true);
    return open ? <ProviderEditor tool={extra.tool ?? "claude"} open onOpenChange={setOpen} provider={provider ?? undefined} template={extra.template} existingIds={["existing"]} /> : <p>Editor closed</p>;
  }
  return { ...renderWithClient(<StrictMode><Host /></StrictMode>), backend };
}
const editorText = () => screen.getByTestId("editor-json").querySelector(".cm-content")?.textContent ?? "";
const openTab = (name: "常用设置" | "完整配置") => userEvent.click(screen.getByRole("tab", { name: new RegExp(`^${name}`) }));
const result = (extra: Partial<Record<string, unknown>>) => ({ ok: true, status: 200, latencyMs: 120, code: null, detail: null, endpoint: "https://example.com/", model: null, models: null, ...extra });

beforeEach(() => {
  vi.clearAllMocks();
  mock.loadPresets.mockResolvedValue(presets);
});

describe("Provider editor workflow", () => {
  it("preserves edits on Escape and only discards after explicit confirmation", async () => {
    mount();
    expect(screen.getByRole("dialog")).toHaveAccessibleDescription("修改这个供应商。如果它正在使用，保存后会同步更新客户端配置。");
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    await userEvent.type(screen.getByLabelText("名称"), " draft");
    await userEvent.keyboard("{Escape}");
    const confirmation = await screen.findByRole("alertdialog");
    await userEvent.click(within(confirmation).getByRole("button", { name: "取消" }));
    expect(screen.getByLabelText("名称")).toHaveValue("Existing draft");
    await userEvent.click(screen.getByRole("button", { name: "关闭" }));
    await userEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "放弃修改" }));
    expect(screen.getByText("Editor closed")).toBeInTheDocument();
  });

  it("starts from a preset, confirms replacing edits and reports unmatched searches", async () => {
    mount(null);
    expect(screen.queryByRole("button", { name: "保存" })).not.toBeInTheDocument();
    await userEvent.click(await screen.findByRole("button", { name: "First" }));
    await userEvent.type(screen.getByLabelText("名称"), " draft");
    await userEvent.click(screen.getByRole("button", { name: "更换" }));
    await userEvent.click(screen.getByRole("button", { name: "Second" }));
    await userEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "取消" }));
    expect(screen.getByRole("button", { name: "First" })).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(screen.getByRole("button", { name: "First" }));
    expect(screen.getByLabelText("名称")).toHaveValue("First draft");
    await userEvent.click(screen.getByRole("button", { name: "更换" }));
    await userEvent.click(screen.getByRole("button", { name: "Second" }));
    await userEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "替换为所选预设" }));
    expect(screen.getByLabelText("名称")).toHaveValue("Second");
    expect(screen.getByText("起点：Second")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "更换" }));
    expect(screen.getByRole("button", { name: "Second" })).toHaveAttribute("aria-pressed", "true");
    await userEvent.type(screen.getByRole("searchbox"), "unmatched");
    expect(screen.getByText(/没有匹配的预设/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "空白配置" })).toBeInTheDocument();
  });

  it("locks inputs and CodeMirror while saving, then retains the draft after failure", async () => {
    let fail!: (error: Error) => void;
    const pending = new Promise<never>((_, reject) => { fail = reject; });
    mount(saved, { update_provider: () => pending });
    await userEvent.type(screen.getByLabelText("名称"), " draft");
    await openTab("完整配置");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    expect(screen.getByRole("tab", { name: "常用设置" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "关闭" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "取消" })).toBeDisabled();
    expect(screen.getByTestId("editor-json").querySelector(".cm-content")).toHaveAttribute("contenteditable", "false");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    await act(async () => fail(new Error("write denied")));
    expect(await screen.findByRole("alert")).toHaveTextContent("保存失败，修改已保留。 write denied");
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
    expect(screen.getByTestId("editor-json").querySelector(".cm-content")).toHaveAttribute("contenteditable", "true");
    await openTab("常用设置");
    expect(screen.getByLabelText("名称")).toHaveValue("Existing draft");
  });

  it("offers preset retry and generates a unique ID without exposing it in either tab", async () => {
    mock.loadPresets.mockRejectedValue(new Error("offline"));
    const { backend } = mount(null, { add_provider: () => ({ changed: [], warnings: [] }) });
    expect(await screen.findByRole("alert")).toHaveTextContent("预设加载失败");
    mock.loadPresets.mockResolvedValue(presets);
    await userEvent.click(screen.getByRole("button", { name: "重试" }));
    expect(await screen.findByRole("button", { name: "First" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "空白配置" }));
    fireEvent.change(screen.getByLabelText("名称"), { target: { value: "Existing" } });
    expect(screen.queryByLabelText("ID")).not.toBeInTheDocument();
    await openTab("完整配置");
    expect(screen.queryByLabelText("ID")).not.toBeInTheDocument();
    expect(screen.queryByText("内部标识（高级）")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    const created = backend.calls.find((call) => call.cmd === "add_provider")?.args?.provider as Provider;
    expect(created.id).toBe("existing-2");
    expect(created.name).toBe("Existing");
  });

  it("keeps structured fields and the full configuration in sync", async () => {
    const { backend } = mount(saved, { update_provider: () => ({ changed: [], warnings: [] }) });
    const url = screen.getByLabelText("请求地址");
    expect(url).toHaveValue("https://example.com");
    await userEvent.clear(url);
    await userEvent.type(url, "https://gw.example.com");
    await userEvent.type(screen.getByLabelText("API Key"), "sk-test");
    await userEvent.selectOptions(screen.getByLabelText("认证方式"), "ANTHROPIC_API_KEY");
    await userEvent.type(screen.getByLabelText("Sonnet 模型"), "my-sonnet");
    expect(screen.getByLabelText("API Key")).toHaveAttribute("type", "password");
    await userEvent.click(screen.getByRole("button", { name: "显示 API Key" }));
    expect(screen.getByLabelText("API Key")).toHaveAttribute("type", "text");
    await userEvent.type(screen.getByLabelText("备注"), "work");
    await openTab("完整配置");
    expect(editorText()).toContain("https://gw.example.com");
    expect(editorText()).toContain("ANTHROPIC_API_KEY");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await screen.findByText("Editor closed");
    const update = backend.calls.find((call) => call.cmd === "update_provider")!.args!.provider as Provider;
    expect(update.notes).toBe("work");
    expect(update.settingsConfig).toEqual({ env: { ANTHROPIC_BASE_URL: "https://gw.example.com", ANTHROPIC_API_KEY: "sk-test", ANTHROPIC_DEFAULT_SONNET_MODEL: "my-sonnet" } });
  });

  it("lists fields that switching will not write to Claude", async () => {
    mount({ ...saved, settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://example.com", API_TIMEOUT_MS: "1" }, permissions: {} } }, { ignored_fields: () => ["env.API_TIMEOUT_MS", "permissions"] });
    expect(await screen.findByText(/有 2 个字段不随切换生效/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "去完整配置查看" }));
    const list = await screen.findByRole("list", { name: "不随切换生效的字段" });
    expect(within(list).getAllByRole("listitem").map((item) => item.textContent)).toEqual(["env.API_TIMEOUT_MS", "permissions"]);
  });

  it("duplicates a provider without its notes and keeps other fields", async () => {
    const template: Provider = { ...saved, notes: "private", websiteUrl: "https://site.example", icon: "kimi", sortIndex: 3 };
    const { backend } = mount(null, { add_provider: () => ({ changed: [], warnings: [] }) }, { template });
    expect(screen.getByRole("dialog")).toHaveAccessibleName("复制供应商 · Claude Code");
    expect(screen.queryByRole("group", { name: "预设" })).not.toBeInTheDocument();
    expect(screen.getByLabelText("名称")).toHaveValue("Existing 副本");
    expect(screen.queryByLabelText("ID")).not.toBeInTheDocument();
    expect(screen.getByLabelText("备注")).toHaveValue("");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await screen.findByText("Editor closed");
    const added = backend.calls.find((call) => call.cmd === "add_provider")!.args!.provider as Provider;
    expect(added).toMatchObject({ id: "existing-copy", name: "Existing 副本", websiteUrl: "https://site.example", icon: "kimi", settingsConfig: saved.settingsConfig });
    expect(added).not.toHaveProperty("notes");
    expect(added).not.toHaveProperty("sortIndex");
  });

  it("explains when a Codex built-in local provider decides the base URL", async () => {
    mount({ id: "local", name: "Local", settingsConfig: { auth: {}, config: 'model_provider = "ollama"\nmodel = "qwen"\n' } }, {}, { tool: "codex" });
    expect(screen.getByLabelText("请求地址")).toBeDisabled();
    expect(screen.getByText(/Codex 内置的本地供应商 “ollama”/)).toBeInTheDocument();
    expect(screen.getByLabelText("模型")).toHaveValue("qwen");
    expect(screen.queryByRole("button", { name: "测连通" })).not.toBeInTheDocument();
  });

  it("tests the draft connection and reports failures without saving", async () => {
    const { backend } = mount(saved, {
      probe_latency: () => result({ status: 404, latencyMs: 180 }),
      probe_check: () => result({ ok: false, status: 401, code: "auth", detail: "invalid key ***", model: "draft-model" }),
    });
    await userEvent.type(screen.getByLabelText("默认模型"), "draft-model");
    await userEvent.click(screen.getByRole("button", { name: "测连通" }));
    expect(await screen.findByTestId("probe-latency")).toHaveTextContent("可连通180 msHTTP 404");
    await userEvent.click(screen.getByRole("button", { name: "检查模型" }));
    const check = await screen.findByTestId("probe-check");
    expect(check).toHaveTextContent("认证失败（HTTP 401）");
    expect(check).toHaveTextContent("invalid key ***");
    const call = backend.calls.find((c) => c.cmd === "probe_check")!.args!;
    expect(call).toMatchObject({ tool: "claude", model: "draft-model", settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://example.com", ANTHROPIC_MODEL: "draft-model" } } });
    expect(backend.calls.some((c) => c.cmd === "update_provider")).toBe(false);
    await userEvent.clear(screen.getByLabelText("请求地址"));
    expect(screen.getByTestId("probe-latency")).toHaveTextContent("配置已修改");
  });

  it("offers fetched models in the model fields", async () => {
    mount(saved, { probe_models: () => result({ models: [{ id: "alpha-large", name: "Alpha" }, { id: "beta-small" }] }) });
    await userEvent.click(screen.getByRole("button", { name: "获取模型列表" }));
    expect(await screen.findByTestId("probe-models")).toHaveTextContent("已获取 2 个模型");
    const sonnet = screen.getByLabelText("Sonnet 模型");
    await userEvent.type(sonnet, "beta");
    const options = within(screen.getByRole("listbox")).getAllByRole("option");
    expect(options.map((o) => o.textContent)).toEqual(["beta-small"]);
    await userEvent.keyboard("{Enter}");
    expect(sonnet).toHaveValue("beta-small");
    await userEvent.click(screen.getByLabelText("Opus 模型"));
    await userEvent.keyboard("{ArrowDown}");
    expect(screen.getByRole("listbox")).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("narrows effort levels from the model list and from detection", async () => {
    const { backend } = mount({ ...saved, settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://example.com", ANTHROPIC_MODEL: "opus-x" } } }, {
      probe_models: () => result({ models: [{ id: "opus-x", efforts: ["low", "medium", "high"] }] }),
      probe_efforts: () => result({ model: "opus-x", levels: [{ level: "low", supported: true }, { level: "medium", supported: true }, { level: "high", supported: true }, { level: "xhigh", supported: true }, { level: "max", supported: false, detail: "nope" }] }),
      update_provider: () => ({ changed: [], warnings: [] }),
    });
    const levels = screen.getByRole("radiogroup", { name: "推理强度" });
    expect(screen.getByText("可用档位取决于模型，尚未确认")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "获取模型列表" }));
    expect(await screen.findByText("已按模型列表确认 opus-x 的可用档位")).toBeInTheDocument();
    expect(within(levels).getByRole("radio", { name: "xhigh" })).toHaveAttribute("title", "这个模型不支持该档位");
    await userEvent.click(within(levels).getByRole("radio", { name: "xhigh" }));
    expect(screen.getByText(/所选档位 xhigh 在 opus-x 上不可用/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "重新检测" }));
    expect(await screen.findByText("已检测 opus-x 的可用档位")).toBeInTheDocument();
    expect(screen.queryByText(/所选档位 xhigh/)).not.toBeInTheDocument();
    expect(within(levels).getByRole("radio", { name: "max" })).toHaveAttribute("title", "这个模型不支持该档位");
    expect(backend.calls.find((c) => c.cmd === "probe_efforts")!.args).toMatchObject({ model: "opus-x" });
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await screen.findByText("Editor closed");
    const update = backend.calls.find((c) => c.cmd === "update_provider")!.args!.provider as Provider;
    expect((update.settingsConfig.env as Record<string, string>).CLAUDE_CODE_EFFORT_LEVEL).toBe("xhigh");
  });
});
