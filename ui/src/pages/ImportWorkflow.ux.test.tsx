import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { fakeBackend, renderWithClient, type Handler } from "@/test/utils";
import type { ExportPreview, ImportScan, ProviderImportPreview } from "@/lib/api";

const backend: { invoke: ReturnType<typeof vi.fn> | null } = { invoke: null };
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: Record<string, unknown>, options?: { headers?: Record<string, string> }) =>
    backend.invoke!(cmd, options?.headers ? { body: args, headers: options.headers } : args),
}));

import { ImportWizard } from "./ImportWizard";
import { ExportImport } from "./ExportImport";

function use(handlers: Record<string, Handler>) {
  const fake = fakeBackend({
    get_settings: () => ({ app: {}, keepCodexLogin: false, dataDir: "/data", home: "/home", livePaths: {}, ccswitchDir: "/home/.cc-switch" }),
    provider_import_discard: () => null,
    ...handlers,
  });
  backend.invoke = fake.invoke;
  return fake;
}

const scan: ImportScan = {
  source: "/source/config.json", accounts: 0,
  candidates: [{ tool: "claude", provider: { id: "relay", name: "Relay", settingsConfig: {} }, skipReason: null }],
};
const preview = (rename = false): ExportPreview => ({
  format: "sql-dump", schemaVersion: 12,
  items: [
    { tool: "claude", app: "claude", sourceId: "relay", name: "Relay", status: "import", targetId: "relay", reason: null, notes: [] },
    { tool: "claude", app: "claude", sourceId: "taken", name: "Taken", status: "duplicate", targetId: rename ? "taken-2" : null, reason: null, notes: [] },
  ],
});
const file = () => new File(["-- exported data"], "backup.sql", { type: "application/sql" });

beforeEach(async () => {
  backend.invoke = null;
  const { default: i18n } = await import("@/i18n");
  await i18n.changeLanguage("zh");
});

it("clears an unlocked backup when leaving the file import tab", async () => {
  const fake = use({ backup_load: () => ({ createdAt: 100, items: [], accounts: 0, existingAccounts: 0, hasSettings: true, hasPaths: false }), backup_discard: () => null });
  renderWithClient(<ImportWizard />);
  await userEvent.click(screen.getByRole("tab", { name: "导入文件" }));
  await userEvent.upload(screen.getByTestId("export-file"), new File(["encrypted fixture"], "backup.leanflip"));
  await userEvent.type(await screen.findByLabelText("备份密码"), "fixture-passphrase");
  await userEvent.click(screen.getByRole("button", { name: "解密并预览" }));
  await screen.findByRole("button", { name: "确认恢复所选内容" });
  await userEvent.click(screen.getByRole("tab", { name: "本机数据目录" }));
  await waitFor(() => expect(fake.calls.some((call) => call.cmd === "backup_discard")).toBe(true));
  await userEvent.click(screen.getByRole("tab", { name: "导入文件" }));
  expect(screen.queryByRole("button", { name: "确认恢复所选内容" })).not.toBeInTheDocument();
});

describe("local import source consistency", () => {
  it("requires a new preview after the directory changes and applies the scanned directory", async () => {
    const user = userEvent.setup();
    const fake = use({ ccswitch_scan: () => scan, ccswitch_apply: () => ({ created: [["claude", "relay"]], skipped: [], accountsAdded: [] }) });
    renderWithClient(<ImportWizard />);
    expect(screen.queryByRole("button", { name: "选择导出文件…" })).not.toBeInTheDocument();
    await user.click(screen.getByText("手动指定数据目录"));
    const dir = screen.getByLabelText("CC Switch 数据目录");
    await user.type(dir, "/first");
    await user.click(screen.getByRole("button", { name: "检测本机 CC Switch" }));
    expect(await screen.findByRole("button", { name: "导入所选工具" })).toBeEnabled();
    await user.clear(dir);
    await user.type(dir, "/second");
    expect(screen.queryByRole("button", { name: "导入所选工具" })).not.toBeInTheDocument();
    expect(screen.queryByText("Relay")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "检测本机 CC Switch" }));
    await user.click(await screen.findByRole("button", { name: "导入所选工具" }));
    await waitFor(() => expect(fake.calls.find((call) => call.cmd === "ccswitch_apply")?.args).toEqual({ tools: ["claude"], dir: "/second", accounts: true }));
  });

  it("clears a previous scan when rescanning fails and recovers with an explicit retry", async () => {
    const user = userEvent.setup();
    let attempts = 0;
    use({ ccswitch_scan: () => { if (++attempts === 2) throw new Error("read denied"); return scan; } });
    renderWithClient(<ImportWizard />);
    await user.click(screen.getByRole("button", { name: "检测本机 CC Switch" }));
    await screen.findByRole("button", { name: "导入所选工具" });
    await user.click(screen.getByRole("button", { name: "检测本机 CC Switch" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("read denied");
    expect(screen.queryByRole("button", { name: "导入所选工具" })).not.toBeInTheDocument();
    expect(screen.queryByText("Relay")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重试" }));
    expect(await screen.findByRole("button", { name: "导入所选工具" })).toBeEnabled();
    expect(attempts).toBe(3);
  });

  it("imports accounts without a provider source and freezes its source while applying", async () => {
    const user = userEvent.setup();
    let finish!: (value: unknown) => void;
    const fake = use({ ccswitch_scan: () => ({ source: null, candidates: [], accounts: 2 }), ccswitch_apply: () => new Promise((resolve) => { finish = resolve; }) });
    renderWithClient(<ImportWizard />);
    await user.click(screen.getByText("手动指定数据目录"));
    const dir = screen.getByLabelText("CC Switch 数据目录");
    await user.type(dir, "/accounts-only");
    await user.click(screen.getByRole("button", { name: "检测本机 CC Switch" }));
    const codex = await screen.findByRole("checkbox", { name: "导入 Codex" });
    expect(codex).toBeChecked();
    expect(screen.queryByText("没有找到 cc-switch 数据。")).not.toBeInTheDocument();
    expect(screen.getByText("已选择 0 个供应商、2 个账号")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "导入所选工具" }));
    expect(dir).toBeDisabled();
    expect(codex).toBeDisabled();
    expect(screen.getByRole("tab", { name: "导入文件" })).toBeDisabled();
    expect(fake.calls.find((call) => call.cmd === "ccswitch_apply")?.args).toEqual({ tools: ["codex"], dir: "/accounts-only", accounts: true });
    await act(async () => finish({ created: [], skipped: [], accountsAdded: ["a", "b"] }));
    expect(await screen.findByText("导入完成：新增 0 个供应商、2 个账号，跳过 0 个。")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重新扫描此目录" })).toBeEnabled();
  });

  it("rejects a relative directory instead of silently previewing the default", async () => {
    const user = userEvent.setup();
    const fake = use({ ccswitch_scan: () => scan });
    renderWithClient(<ImportWizard />);
    await user.type(screen.getByLabelText("CC Switch 数据目录"), "relative/path");
    await user.click(screen.getByRole("button", { name: "检测本机 CC Switch" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("请输入绝对路径");
    expect(fake.calls.some((call) => call.cmd === "ccswitch_scan")).toBe(false);
  });
});

const providerPreview: ProviderImportPreview = {
  previewId: 21, directory: "/source/ai-toolbox", found: true,
  items: [
    { index: 0, tool: "codex", name: "Codex Relay", skipReason: null, notes: [], needsApiKey: false },
    { index: 1, tool: "claude", name: "Claude Team", skipReason: null, notes: [], needsApiKey: false },
    { index: 2, tool: "codex", name: "Existing", skipReason: { code: "duplicate", zh: "已存在", en: "Already exists" }, notes: [], needsApiKey: false },
  ],
};

describe("provider imports from Data & migration", () => {
  it("imports selected AI Toolbox providers from a custom directory without completing onboarding", async () => {
    const user = userEvent.setup();
    const fake = use({ provider_import_scan: () => providerPreview, provider_import_apply: () => ({ created: [["codex", "relay"]], skipped: [], accountsAdded: [] }) });
    renderWithClient(<ImportWizard />);
    await user.click(screen.getByRole("radio", { name: /AI Toolbox/ }));
    expect(screen.getByRole("heading", { name: "从本机 AI Toolbox 数据目录导入" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "检测本机 CC Switch" })).not.toBeInTheDocument();
    await user.click(screen.getByText("配置在其他位置？"));
    await user.type(screen.getByLabelText("来源数据目录"), "/custom/toolbox");
    await user.click(screen.getByRole("button", { name: "扫描并预览" }));
    expect(await screen.findByRole("checkbox", { name: /Existing/ })).toBeDisabled();
    await user.click(screen.getByRole("checkbox", { name: /Claude Team/ }));
    await user.click(screen.getByRole("button", { name: "导入所选 1 项" }));
    await screen.findByText("已导入 1 项，跳过 0 项");
    expect(fake.calls.find((call) => call.cmd === "provider_import_scan")?.args).toEqual({ source: "ai-toolbox", directory: "/custom/toolbox" });
    expect(fake.calls.find((call) => call.cmd === "provider_import_apply")?.args).toEqual({ previewId: 21, selected: [{ index: 0, apiKey: undefined }] });
    expect(fake.calls.some((call) => ["complete_onboarding", "ccswitch_apply"].includes(call.cmd))).toBe(false);
  });

  it("requires missing Superba keys and drops previews and private input on source or tab changes", async () => {
    const user = userEvent.setup();
    const fake = use({ provider_import_scan: () => ({ ...providerPreview, items: [{ ...providerPreview.items[0], needsApiKey: true }] }) });
    renderWithClient(<ImportWizard />);
    await user.click(screen.getByRole("radio", { name: /Superba/ }));
    await user.click(screen.getByRole("button", { name: "扫描并预览" }));
    const key = await screen.findByLabelText("Codex Relay 的 API Key");
    expect(key).toHaveAttribute("type", "password");
    expect(screen.getByRole("button", { name: "导入所选 1 项" })).toBeDisabled();
    await user.type(key, "fixture-only");
    expect(screen.getByRole("button", { name: "导入所选 1 项" })).toBeEnabled();
    await user.click(screen.getByRole("radio", { name: /AI Toolbox/ }));
    expect(screen.queryByLabelText("Codex Relay 的 API Key")).not.toBeInTheDocument();
    await waitFor(() => expect(fake.calls.filter((call) => call.cmd === "provider_import_discard")).toHaveLength(1));
    await user.click(screen.getByRole("radio", { name: /Superba/ }));
    await user.click(screen.getByRole("button", { name: "扫描并预览" }));
    expect(await screen.findByLabelText("Codex Relay 的 API Key")).toHaveValue("");
    await user.type(screen.getByLabelText("Codex Relay 的 API Key"), "fixture-only");
    await user.click(screen.getByRole("tab", { name: "导入文件" }));
    await waitFor(() => expect(fake.calls.filter((call) => call.cmd === "provider_import_discard")).toHaveLength(3));
    await user.click(screen.getByRole("tab", { name: "本机数据目录" }));
    expect(screen.queryByRole("button", { name: /导入所选/ })).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Codex Relay 的 API Key")).not.toBeInTheDocument();
  });

  it("locks migration navigation during scan and apply, then releases it after success", async () => {
    const user = userEvent.setup();
    let finishScan!: (value: ProviderImportPreview) => void;
    let finishApply!: (value: unknown) => void;
    const onBusyChange = vi.fn();
    const fake = use({
      provider_import_scan: () => new Promise<ProviderImportPreview>((resolve) => { finishScan = resolve; }),
      provider_import_apply: () => new Promise((resolve) => { finishApply = resolve; }),
    });
    renderWithClient(<ImportWizard onBusyChange={onBusyChange} />);
    await user.click(screen.getByRole("radio", { name: /Superba/ }));
    await user.click(screen.getByRole("button", { name: "扫描并预览" }));
    expect(screen.getByRole("radio", { name: /CC Switch/ })).toBeDisabled();
    expect(screen.getByRole("tab", { name: "导入文件" })).toBeDisabled();
    expect(onBusyChange).toHaveBeenLastCalledWith(true);
    await act(async () => finishScan({ ...providerPreview, items: [{ ...providerPreview.items[0], needsApiKey: true }] }));
    await user.type(await screen.findByLabelText("Codex Relay 的 API Key"), "fixture-key");
    await user.click(screen.getByRole("button", { name: "导入所选 1 项" }));
    expect(screen.getByRole("radio", { name: /AI Toolbox/ })).toBeDisabled();
    expect(screen.getByRole("tab", { name: "导出备份" })).toBeDisabled();
    expect(screen.getByLabelText("来源数据目录")).toBeDisabled();
    expect(fake.calls.find((call) => call.cmd === "provider_import_apply")?.args).toEqual({ previewId: 21, selected: [{ index: 0, apiKey: "fixture-key" }] });
    await act(async () => finishApply({ created: [["codex", "relay"]], skipped: [], accountsAdded: [] }));
    await screen.findByText("已导入 1 项，跳过 0 项");
    expect(screen.getByRole("tab", { name: "导出备份" })).toBeEnabled();
    expect(onBusyChange).toHaveBeenLastCalledWith(false);
    expect(screen.queryByLabelText("Codex Relay 的 API Key")).not.toBeInTheDocument();
  });

  it("invalidates previews after directory edits or scan failures and returns to the CC Switch flow", async () => {
    const user = userEvent.setup();
    let attempts = 0;
    const fake = use({ provider_import_scan: () => { if (++attempts === 2) throw new Error("source unavailable"); return providerPreview; }, ccswitch_scan: () => scan });
    renderWithClient(<ImportWizard />);
    await user.click(screen.getByRole("radio", { name: /AI Toolbox/ }));
    await user.click(screen.getByRole("button", { name: "扫描并预览" }));
    await screen.findByRole("button", { name: "导入所选 2 项" });
    await user.click(screen.getByText("配置在其他位置？"));
    await user.type(screen.getByLabelText("来源数据目录"), "/other");
    expect(screen.queryByRole("button", { name: /导入所选/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "扫描并预览" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("source unavailable");
    expect(screen.queryByText("Codex Relay")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "扫描并预览" }));
    await screen.findByRole("button", { name: "导入所选 2 项" });
    await user.click(screen.getByRole("radio", { name: /CC Switch/ }));
    await user.click(screen.getByRole("button", { name: "检测本机 CC Switch" }));
    expect(await screen.findByRole("button", { name: "导入所选工具" })).toBeEnabled();
    expect(fake.calls.some((call) => call.cmd === "ccswitch_scan")).toBe(true);
  });
});

describe("export preview and action ordering", () => {
  it.each(["leanflip", "age"])("opens a .%s backup in the password flow without parsing it as a CC Switch export", async (extension) => {
    const user = userEvent.setup();
    const fake = use({ backup_discard: () => null });
    renderWithClient(<ExportImport />);
    await user.upload(screen.getByTestId("export-file"), new File(["encrypted fixture"], `backup.${extension}`));
    expect(await screen.findByLabelText("备份密码")).toHaveAttribute("type", "password");
    expect(fake.calls.some((call) => call.cmd === "ccswitch_export_load")).toBe(false);
    expect(fake.calls.some((call) => call.cmd === "backup_load")).toBe(false);
  });

  it("locks source, policy and selection until preview and apply each settle", async () => {
    const user = userEvent.setup();
    let finishPreview!: (value: ExportPreview) => void;
    let finishApply!: (value: unknown) => void;
    const fake = use({
      ccswitch_export_load: () => preview(),
      ccswitch_export_preview: () => new Promise<ExportPreview>((resolve) => { finishPreview = resolve; }),
      ccswitch_export_apply: () => new Promise((resolve) => { finishApply = resolve; }),
    });
    renderWithClient(<ImportWizard />);
    await user.click(screen.getByRole("tab", { name: "导入文件" }));
    expect(screen.queryByRole("button", { name: "扫描" })).not.toBeInTheDocument();
    await user.upload(screen.getByTestId("export-file"), file());
    await screen.findByRole("button", { name: "确认导入 1 个供应商" });
    const policy = screen.getByLabelText("同 ID 的供应商");
    await user.selectOptions(policy, "rename");
    expect(policy).toBeDisabled();
    expect(screen.getByRole("tab", { name: "本机数据目录" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "选择导出文件…" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: /确认导入/ })).not.toBeInTheDocument();
    await user.selectOptions(policy, "skip");
    expect(fake.calls.filter((call) => call.cmd === "ccswitch_export_preview")).toHaveLength(1);
    await act(async () => finishPreview(preview(true)));
    await user.click(await screen.findByRole("button", { name: "确认导入 2 个供应商" }));
    expect(screen.getByRole("checkbox", { name: "导入 Claude Code" })).toBeDisabled();
    expect(policy).toBeDisabled();
    expect(screen.getByTestId("export-file")).toBeDisabled();
    expect(screen.getByRole("tab", { name: "本机数据目录" })).toBeDisabled();
    expect(fake.calls.find((call) => call.cmd === "ccswitch_export_apply")?.args).toEqual({ tools: ["claude"], policy: "rename" });
    await act(async () => finishApply({ created: [["claude", "relay", "relay"], ["claude", "taken", "taken-2"]], skipped: 0 }));
    expect(await screen.findByRole("button", { name: "导入另一个文件" })).toBeEnabled();
    expect(screen.getByRole("tab", { name: "本机数据目录" })).toBeEnabled();
  });

  it("never applies an old preview after a policy refresh fails and can retry it", async () => {
    const user = userEvent.setup();
    let attempts = 0;
    const fake = use({
      ccswitch_export_load: () => preview(),
      ccswitch_export_preview: () => { if (++attempts === 1) throw new Error("preview unavailable"); return preview(true); },
    });
    renderWithClient(<ExportImport />);
    await user.upload(screen.getByTestId("export-file"), file());
    await screen.findByRole("button", { name: "确认导入 1 个供应商" });
    await user.selectOptions(screen.getByLabelText("同 ID 的供应商"), "rename");
    expect(await screen.findByRole("alert")).toHaveTextContent("preview unavailable");
    expect(screen.queryByRole("button", { name: /确认导入/ })).not.toBeInTheDocument();
    expect(screen.queryByTestId("export-group-claude")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重试" }));
    expect(await screen.findByRole("button", { name: "确认导入 2 个供应商" })).toBeEnabled();
    expect(fake.calls.filter((call) => call.cmd === "ccswitch_export_preview").map((call) => call.args)).toEqual([{ policy: "rename" }, { policy: "rename" }]);
  });

  it("retries the selected file after a load error without retaining an old preview", async () => {
    const user = userEvent.setup();
    let attempts = 0;
    use({ ccswitch_export_load: () => { if (++attempts === 1) throw new Error("parse failed"); return preview(); } });
    renderWithClient(<ExportImport />);
    await user.upload(screen.getByTestId("export-file"), file());
    expect(await screen.findByRole("alert")).toHaveTextContent("parse failed");
    expect(screen.queryByRole("button", { name: /确认导入/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重试" }));
    expect(await screen.findByRole("button", { name: "确认导入 1 个供应商" })).toBeEnabled();
    expect(attempts).toBe(2);
  });

  it("waits for staged-file discard before allowing another file to replace it", async () => {
    const user = userEvent.setup();
    let finish!: () => void;
    const fake = use({ ccswitch_export_load: () => preview(), ccswitch_export_discard: () => new Promise<void>((resolve) => { finish = resolve; }) });
    renderWithClient(<ExportImport />);
    await user.upload(screen.getByTestId("export-file"), file());
    await user.click(await screen.findByRole("button", { name: "放弃此文件" }));
    expect(screen.getByRole("button", { name: "选择导出文件…" })).toBeDisabled();
    await user.upload(screen.getByTestId("export-file"), file());
    expect(fake.calls.filter((call) => call.cmd === "ccswitch_export_load")).toHaveLength(1);
    await act(async () => finish());
    expect(await screen.findByRole("button", { name: "选择导出文件…" })).toBeEnabled();
    expect(screen.queryByText("文件：backup.sql")).not.toBeInTheDocument();
  });
});
