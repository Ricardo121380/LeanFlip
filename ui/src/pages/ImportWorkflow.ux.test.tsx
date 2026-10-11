import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { fakeBackend, renderWithClient, type Handler } from "@/test/utils";
import type { ExportPreview, ImportScan } from "@/lib/api";

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

describe("local import source consistency", () => {
  it("requires a new preview after the directory changes and applies the scanned directory", async () => {
    const user = userEvent.setup();
    const fake = use({ ccswitch_scan: () => scan, ccswitch_apply: () => ({ created: [["claude", "relay"]], skipped: [], accountsAdded: [] }) });
    renderWithClient(<ImportWizard />);
    expect(screen.queryByRole("button", { name: "选择导出文件…" })).not.toBeInTheDocument();
    const dir = screen.getByLabelText("cc-switch 数据目录");
    await user.type(dir, "/first");
    await user.click(screen.getByRole("button", { name: "扫描" }));
    expect(await screen.findByRole("button", { name: "导入所选工具" })).toBeEnabled();
    await user.clear(dir);
    await user.type(dir, "/second");
    expect(screen.queryByRole("button", { name: "导入所选工具" })).not.toBeInTheDocument();
    expect(screen.queryByText("Relay")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "扫描" }));
    await user.click(await screen.findByRole("button", { name: "导入所选工具" }));
    await waitFor(() => expect(fake.calls.find((call) => call.cmd === "ccswitch_apply")?.args).toEqual({ tools: ["claude"], dir: "/second" }));
  });

  it("clears a previous scan when rescanning fails and recovers with an explicit retry", async () => {
    const user = userEvent.setup();
    let attempts = 0;
    use({ ccswitch_scan: () => { if (++attempts === 2) throw new Error("read denied"); return scan; } });
    renderWithClient(<ImportWizard />);
    await user.click(screen.getByRole("button", { name: "扫描" }));
    await screen.findByRole("button", { name: "导入所选工具" });
    await user.click(screen.getByRole("button", { name: "扫描" }));
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
    const dir = screen.getByLabelText("cc-switch 数据目录");
    await user.type(dir, "/accounts-only");
    await user.click(screen.getByRole("button", { name: "扫描" }));
    const codex = await screen.findByRole("checkbox", { name: "导入 Codex" });
    expect(codex).not.toBeChecked();
    expect(screen.queryByText("没有找到 cc-switch 数据。")).not.toBeInTheDocument();
    await user.click(codex);
    expect(screen.getByText("已选择 0 张卡、2 个账号")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "导入所选工具" }));
    expect(dir).toBeDisabled();
    expect(codex).toBeDisabled();
    expect(screen.getByRole("tab", { name: "导出文件" })).toBeDisabled();
    expect(fake.calls.find((call) => call.cmd === "ccswitch_apply")?.args).toEqual({ tools: ["codex"], dir: "/accounts-only" });
    await act(async () => finish({ created: [], skipped: [], accountsAdded: ["a", "b"] }));
    expect(await screen.findByText("导入完成：新增 0 张卡、2 个账号，跳过 0 张。")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重新扫描此目录" })).toBeEnabled();
  });

  it("rejects a relative directory instead of silently previewing the default", async () => {
    const user = userEvent.setup();
    const fake = use({ ccswitch_scan: () => scan });
    renderWithClient(<ImportWizard />);
    await user.type(screen.getByLabelText("cc-switch 数据目录"), "relative/path");
    await user.click(screen.getByRole("button", { name: "扫描" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("请输入绝对路径");
    expect(fake.calls.some((call) => call.cmd === "ccswitch_scan")).toBe(false);
  });
});

describe("export preview and action ordering", () => {
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
    await user.click(screen.getByRole("tab", { name: "导出文件" }));
    expect(screen.queryByRole("button", { name: "扫描" })).not.toBeInTheDocument();
    await user.upload(screen.getByTestId("export-file"), file());
    await screen.findByRole("button", { name: "确认导入 1 张卡" });
    const policy = screen.getByLabelText("同 id 的卡");
    await user.selectOptions(policy, "rename");
    expect(policy).toBeDisabled();
    expect(screen.getByRole("tab", { name: "本机数据目录" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "选择导出文件…" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: /确认导入/ })).not.toBeInTheDocument();
    await user.selectOptions(policy, "skip");
    expect(fake.calls.filter((call) => call.cmd === "ccswitch_export_preview")).toHaveLength(1);
    await act(async () => finishPreview(preview(true)));
    await user.click(await screen.findByRole("button", { name: "确认导入 2 张卡" }));
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
    await screen.findByRole("button", { name: "确认导入 1 张卡" });
    await user.selectOptions(screen.getByLabelText("同 id 的卡"), "rename");
    expect(await screen.findByRole("alert")).toHaveTextContent("preview unavailable");
    expect(screen.queryByRole("button", { name: /确认导入/ })).not.toBeInTheDocument();
    expect(screen.queryByTestId("export-group-claude")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重试" }));
    expect(await screen.findByRole("button", { name: "确认导入 2 张卡" })).toBeEnabled();
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
    expect(await screen.findByRole("button", { name: "确认导入 1 张卡" })).toBeEnabled();
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
