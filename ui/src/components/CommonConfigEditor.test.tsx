import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { cleanup, screen, fireEvent, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { fakeBackend, renderWithClient } from "@/test/utils";
import i18n from "@/i18n";
const backend = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => backend.invoke(...args) }));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() } }));
vi.mock("./CodeEditor", () => ({ CodeEditor: ({ value, onChange, ariaLabel, readOnly }: any) => <textarea aria-label={ariaLabel} value={value} readOnly={readOnly} onChange={(e) => onChange(e.target.value)} /> }));
import { CommonConfigEditor } from "./CommonConfigEditor";
import { ProviderEditor } from "./ProviderEditor";
import { TOOLS } from "@/lib/tools";
import { readToml } from "@/lib/toml-edit";

beforeEach(async () => { vi.clearAllMocks(); await i18n.changeLanguage("zh"); });
afterEach(cleanup);
it("shares one draft between switches and JSON, saves with revision and reloads it", async () => {
  let config: Record<string, unknown> = { env: { API_TIMEOUT_MS: "500" } };
  let revision = "r1";
  const fake = fakeBackend({
    get_common_config: () => ({ config, revision, format: "JSON", paths: ["/isolated/.claude/settings.json"] }),
    save_common_config: (args) => { expect(args!.revision).toBe("r1"); config = args!.config as Record<string, unknown>; revision = "r2"; return { changed: ["/isolated/.claude/settings.json"], warnings: [] }; },
  }); backend.invoke = fake.invoke;
  const close = vi.fn();
  renderWithClient(<CommonConfigEditor tool="claude" onClose={close} />);
  await userEvent.click(await screen.findByRole("checkbox", { name: "Teammates 模式" }));
  expect(JSON.parse((screen.getByLabelText("配置 JSON") as HTMLTextAreaElement).value).env).toEqual({ API_TIMEOUT_MS: "500", CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS: "1" });
  fireEvent.change(screen.getByLabelText("配置 JSON"), { target: { value: JSON.stringify({ env: { API_TIMEOUT_MS: "500", DISABLE_AUTOUPDATER: "1" } }) } });
  expect(screen.getByRole("checkbox", { name: "禁用自动升级" })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Teammates 模式" })).not.toBeChecked();
  await userEvent.click(screen.getByRole("button", { name: "保存并应用" }));
  await waitFor(() => expect(close).toHaveBeenCalled());
  expect(config).toEqual({ env: { API_TIMEOUT_MS: "500", DISABLE_AUTOUPDATER: "1" } });
  expect(fake.calls.filter((c) => c.cmd === "save_common_config")).toHaveLength(1);
});
it("retains invalid and conflicting drafts; cancelling does not write", async () => {
  const fake = fakeBackend({ get_common_config: () => ({ config: {}, revision: "v1", format: "TOML", paths: ["/isolated/.codex/config.toml"] }), save_common_config: () => { throw new Error("Client configuration changed"); } });
  backend.invoke = fake.invoke;
  const close = vi.fn(); renderWithClient(<CommonConfigEditor tool="codex" onClose={close} />);
  await userEvent.click(await screen.findByRole("checkbox", { name: "Shell 快照" }));
  await userEvent.click(screen.getByRole("button", { name: "保存并应用" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Client configuration changed");
  expect(within(screen.getByTestId("common-config-footer")).getByRole("alert")).toHaveTextContent("Client configuration changed");
  expect(screen.getByRole("checkbox", { name: "Shell 快照" })).toBeChecked();
  fireEvent.change(screen.getByLabelText("配置 JSON"), { target: { value: "{" } });
  expect(screen.queryByText("Client configuration changed")).not.toBeInTheDocument();
  expect(within(screen.getByTestId("common-config-footer")).getByRole("alert")).toHaveTextContent("尚未保存");
  expect(screen.getByRole("alert")).not.toHaveTextContent("顶层必须是对象");
  expect(screen.getByRole("button", { name: "保存并应用" })).toBeDisabled();
  fireEvent.change(screen.getByLabelText("配置 JSON"), { target: { value: "[]" } });
  expect(screen.getByRole("alert")).toHaveTextContent("顶层必须是对象");
  await userEvent.click(screen.getByRole("button", { name: "取消" }));
  expect(await screen.findByRole("alertdialog")).toBeInTheDocument();
  expect(close).not.toHaveBeenCalled();
  expect(fake.calls.filter((c) => c.cmd === "save_common_config")).toHaveLength(1);
});
it("Codex provider shortcuts write typed TOML without exposing ID in common settings", async () => {
  const fake = fakeBackend({ ignored_fields: () => [], update_provider: () => ({ warnings: [], changed: [] }) }); backend.invoke = fake.invoke;
  renderWithClient(<ProviderEditor tool="codex" open onOpenChange={() => {}} provider={{ id: "official", name: "Official", category: "official", settingsConfig: { auth: {}, config: "# retain comment\nmodel = 'test-model'\n" } }} existingIds={["official"]} />);
  expect(screen.queryByLabelText("ID")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: /^完整配置/ }));
  expect(screen.queryByLabelText("ID")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("checkbox", { name: "禁用响应存储" }));
  const config = (screen.getByLabelText(/config.toml/) as HTMLTextAreaElement).value;
  expect(config).toContain("disable_response_storage = true"); expect(config).toContain("# retain comment");
  await userEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() => expect(fake.calls.some((c) => c.cmd === "update_provider")).toBe(true));
});

it.each(TOOLS)("keeps unrecognized preferences in JSON, without raw-field switches for $label", async ({ id }) => {
  const config = { plugin: { enabled: true }, skipDangerousModePermissionPrompt: true, verbose: false };
  const fake = fakeBackend({ get_common_config: () => ({ config, revision: "r1", format: "JSON", paths: ["/isolated/config"] }) });
  backend.invoke = fake.invoke;
  renderWithClient(<CommonConfigEditor tool={id} onClose={() => {}} />);
  const editor = await screen.findByLabelText("配置 JSON");
  expect(JSON.parse((editor as HTMLTextAreaElement).value)).toEqual(config);
  expect(screen.queryByRole("checkbox", { name: /plugin|skipDangerous|verbose/ })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "保存并应用" })).toBeDisabled();
});

it("marks a Codex notification allowlist as custom and preserves it when editing another shortcut", async () => {
  const config = { tui: { notifications: ["agent-turn-complete"] } };
  backend.invoke = fakeBackend({ get_common_config: () => ({ config, revision: "r1", format: "TOML", paths: ["/isolated/config.toml"] }) }).invoke;
  renderWithClient(<CommonConfigEditor tool="codex" onClose={() => {}} />);
  expect(await screen.findByRole("checkbox", { name: "终端通知" })).toBePartiallyChecked();
  expect(screen.getByText("自定义")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("checkbox", { name: "Shell 快照" }));
  expect(JSON.parse((screen.getByLabelText("配置 JSON") as HTMLTextAreaElement).value).tui.notifications).toEqual(["agent-turn-complete"]);
});

it("edits and resets Claude permission mode without dropping rules or unrelated settings", async () => {
  const config = { permissions: { defaultMode: "plan", allow: ["Read"], deny: ["Read(.env)"] }, promptCacheTtl: "5m", extra: true };
  const fake = fakeBackend({ get_common_config: () => ({ config, revision: "r1", format: "JSON", paths: ["/isolated/settings.json"] }) });
  backend.invoke = fake.invoke;
  renderWithClient(<CommonConfigEditor tool="claude" onClose={() => {}} />);
  const mode = await screen.findByRole("combobox", { name: "权限模式" });
  await userEvent.selectOptions(mode, "bypassPermissions");
  expect(screen.getByText(/此模式跳过工具权限确认/)).toBeInTheDocument();
  const draft = () => JSON.parse((screen.getByLabelText("配置 JSON") as HTMLTextAreaElement).value);
  expect(draft().permissions).toEqual({ ...config.permissions, defaultMode: "bypassPermissions" });
  await userEvent.click(screen.getByRole("button", { name: "将 权限模式 恢复默认" }));
  expect(draft().permissions).toEqual({ allow: ["Read"], deny: ["Read(.env)"] });
  await userEvent.selectOptions(screen.getByRole("combobox", { name: "主会话缓存时长" }), "1h");
  expect(draft().promptCacheTtl).toBe("1h");
  expect(draft().extra).toBe(true);
  expect(fake.calls.some((c) => c.cmd === "save_common_config")).toBe(false);
});

it("keeps granular Codex approval custom and validates numeric budgets before saving", async () => {
  const config = { approval_policy: { granular: { sandbox_approval: false } }, sandbox_mode: "workspace-write" };
  const fake = fakeBackend({ get_common_config: () => ({ config, revision: "r1", format: "TOML", paths: ["/isolated/config.toml"] }) });
  backend.invoke = fake.invoke;
  renderWithClient(<CommonConfigEditor tool="codex" onClose={() => {}} />);
  expect(await screen.findByRole("combobox", { name: "命令审批策略" })).toHaveValue("__custom");
  fireEvent.change(screen.getByLabelText("工具输出预算（token）"), { target: { value: "12abc" } });
  expect(screen.getByRole("button", { name: "保存并应用" })).toBeDisabled();
  expect(within(screen.getByTestId("common-config-footer")).getByRole("alert")).toHaveTextContent("整数");
  fireEvent.change(screen.getByLabelText("工具输出预算（token）"), { target: { value: "0" } });
  const draft = JSON.parse((screen.getByLabelText("配置 JSON") as HTMLTextAreaElement).value);
  expect(draft).toEqual({ ...config, tool_output_token_limit: 0 });
  expect(screen.getByRole("button", { name: "保存并应用" })).toBeEnabled();
});

it("preserves explicit bypass restrictions and cache environment overrides", async () => {
  const config = { permissions: { disableBypassPermissionsMode: "disable" }, env: { FORCE_PROMPT_CACHING_5M: "1" }, promptCacheTtl: "1h" };
  backend.invoke = fakeBackend({ get_common_config: () => ({ config, revision: "r1", format: "JSON", paths: ["/isolated/settings.json"] }) }).invoke;
  renderWithClient(<CommonConfigEditor tool="claude" onClose={() => {}} />);
  expect(await screen.findByRole("option", { name: "Bypass · 跳过权限确认" })).toBeDisabled();
  expect(screen.getByRole("combobox", { name: "主会话缓存时长" })).toBeDisabled();
  expect(screen.getByRole("combobox", { name: "子 Agent 缓存时长" })).toBeDisabled();
  expect(JSON.parse((screen.getByLabelText("配置 JSON") as HTMLTextAreaElement).value)).toEqual(config);
});

it("shares Codex context presets and custom limits with TOML and saves numeric values", async () => {
  const fake = fakeBackend({ update_provider: () => ({ warnings: [], changed: [] }) }); backend.invoke = fake.invoke;
  renderWithClient(<ProviderEditor tool="codex" open onOpenChange={() => {}} provider={{ id: "official", name: "Official", category: "official", settingsConfig: { auth: {}, config: "# keep comment\nmodel = 'test-model'\n[features]\ncustom_flag = true\n" } }} existingIds={["official"]} />);
  await userEvent.click(screen.getByRole("button", { name: "1M 窗口 / 900K 压缩" }));
  expect(screen.getByLabelText("上下文窗口（token）")).toHaveValue("1000000");
  expect(screen.getByLabelText("自动压缩阈值（token）")).toHaveValue("900000");
  fireEvent.change(screen.getByLabelText("自动压缩阈值（token）"), { target: { value: "1200000" } });
  expect(screen.getByRole("alert")).toHaveTextContent("不能高于");
  expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
  fireEvent.change(screen.getByLabelText("自动压缩阈值（token）"), { target: { value: "750000" } });
  await userEvent.selectOptions(screen.getByRole("combobox", { name: "联网搜索模式" }), "cached");
  await userEvent.click(screen.getByRole("tab", { name: /^完整配置/ }));
  const toml = (screen.getByLabelText(/config.toml/) as HTMLTextAreaElement).value;
  expect(toml).toContain("# keep comment");
  expect(readToml(toml)).toMatchObject({ model: "test-model", model_context_window: 1000000, model_auto_compact_token_limit: 750000, web_search: "cached", features: { custom_flag: true } });
  fireEvent.change(screen.getByLabelText(/config.toml/), { target: { value: toml.replace("750000", "600000") } });
  expect(screen.getByLabelText("自动压缩阈值（token）")).toHaveValue("600000");
  await userEvent.click(screen.getByRole("button", { name: "将 上下文窗口（token） 恢复默认" }));
  expect(readToml((screen.getByLabelText(/config.toml/) as HTMLTextAreaElement).value)).not.toHaveProperty("model_context_window");
  await userEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() => expect(fake.calls.some((c) => c.cmd === "update_provider")).toBe(true));
  const saved = fake.calls.find((c) => c.cmd === "update_provider")!.args!.provider as { settingsConfig: { config: string } };
  expect(readToml(saved.settingsConfig.config)).toMatchObject({ model_auto_compact_token_limit: 600000, web_search: "cached" });
});

it("writes Claude capacity environment variables as strings and rejects partial numeric input", async () => {
  backend.invoke = fakeBackend({ ignored_fields: () => [] }).invoke;
  renderWithClient(<ProviderEditor tool="claude" open onOpenChange={() => {}} provider={{ id: "test", name: "Test", settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://qa.invalid", ANTHROPIC_AUTH_TOKEN: "fake", CUSTOM_FLAG: "keep" } } }} existingIds={["test"]} />);
  fireEvent.change(screen.getByLabelText("最大输出（token）"), { target: { value: "64000tail" } });
  expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
  fireEvent.change(screen.getByLabelText("最大输出（token）"), { target: { value: "64000" } });
  await userEvent.click(screen.getByRole("tab", { name: /^完整配置/ }));
  const jsonEditor = screen.getByRole("textbox", { name: "配置（JSON）" }) as HTMLTextAreaElement;
  expect(JSON.parse(jsonEditor.value).env).toMatchObject({ CLAUDE_CODE_MAX_OUTPUT_TOKENS: "64000", CUSTOM_FLAG: "keep", ANTHROPIC_AUTH_TOKEN: "fake" });
});
