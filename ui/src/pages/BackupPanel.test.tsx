import { beforeEach, expect, it, vi } from "vitest";
import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { fakeBackend, renderWithClient } from "@/test/utils";
import { BackupExport, BackupRestore } from "./BackupPanel";

const backend = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: backend.invoke }));
const preview = { createdAt: 100, items: [{ tool: "claude", id: "work", name: "Work", duplicate: false }], accounts: 2, existingAccounts: 1, hasSettings: true, hasPaths: true };
const restored = { providersAdded: 0, providersSkipped: 0, accountsAdded: 1, settingsRestored: true, dirsChanged: false, failures: [] };
beforeEach(async () => {
  backend.invoke.mockReset();
  const { default: i18n } = await import("@/i18n"); await i18n.changeLanguage("zh");
});

it("discards a late unlock result after the restore panel has closed", async () => {
  let finish!: (value: typeof preview) => void;
  const fake = fakeBackend({ backup_load: () => new Promise<typeof preview>((resolve) => { finish = resolve; }), backup_discard: () => null });
  backend.invoke.mockImplementation(fake.invoke);
  const view = renderWithClient(<BackupRestore bytes={new Uint8Array([1])} fileName="fixture.leanflip" onBack={() => {}} />);
  await userEvent.type(screen.getByLabelText("备份密码"), "fixture-passphrase");
  await userEvent.click(screen.getByRole("button", { name: "解密并预览" }));
  view.unmount();
  await waitFor(() => expect(fake.calls.filter((call) => call.cmd === "backup_discard")).toHaveLength(1));
  await act(async () => finish(preview));
  await waitFor(() => expect(fake.calls.filter((call) => call.cmd === "backup_discard")).toHaveLength(2));
});

it("exports with matching masked passwords, freezes input while saving, and clears secrets after success", async () => {
  const user = userEvent.setup(); let finish!: (value: string) => void;
  const fake = fakeBackend({ backup_save: () => new Promise<string>((resolve) => { finish = resolve; }) });
  backend.invoke.mockImplementation(fake.invoke);
  renderWithClient(<BackupExport />);
  const password = screen.getByLabelText("备份密码"); const confirm = screen.getByLabelText("再次输入密码");
  expect(password).toHaveAttribute("type", "password");
  await user.type(password, "fixture-passphrase"); await user.type(confirm, "different-value");
  expect(screen.getByRole("button", { name: "选择位置并导出" })).toBeDisabled();
  await user.clear(confirm); await user.type(confirm, "fixture-passphrase");
  await user.click(screen.getByRole("button", { name: "选择位置并导出" }));
  expect(password).toBeDisabled(); expect(fake.calls).toHaveLength(1);
  await act(async () => finish("/backup.leanflip"));
  expect(await screen.findByRole("status")).toHaveTextContent("加密备份已保存");
  expect(password).toHaveValue(""); expect(confirm).toHaveValue("");
});

it("retries a wrong password, previews without writing, and restores only selected scopes", async () => {
  const user = userEvent.setup(); let attempts = 0;
  const fake = fakeBackend({ backup_load: () => { if (++attempts === 1) throw new Error("Incorrect password"); return preview; }, backup_apply: () => restored, backup_discard: () => null });
  backend.invoke.mockImplementation(fake.invoke);
  renderWithClient(<BackupRestore bytes={new Uint8Array([1,2])} fileName="fixture.leanflip" onBack={() => {}} />);
  await user.type(screen.getByLabelText("备份密码"), "wrong-password");
  await user.click(screen.getByRole("button", { name: "解密并预览" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Incorrect password");
  expect(screen.getByLabelText("备份密码")).toHaveValue("");
  await user.type(screen.getByLabelText("备份密码"), "correct-fixture");
  await user.click(screen.getByRole("button", { name: "解密并预览" }));
  const paths = await screen.findByRole("checkbox", { name: /同时恢复本机目录/ }); expect(paths).not.toBeChecked();
  expect(fake.calls.filter((call) => call.cmd === "backup_apply")).toHaveLength(0);
  await user.click(screen.getByRole("checkbox", { name: /Claude Code/ }));
  await user.click(screen.getByRole("button", { name: "确认恢复所选内容" }));
  await waitFor(() => expect(fake.calls.find((call) => call.cmd === "backup_apply")?.args).toEqual({ options: { tools: [], accounts: true, settings: true, paths: false, duplicate: "skip" } }));
  expect(await screen.findByRole("status")).toHaveTextContent("恢复完成");
  expect(screen.queryByRole("button", { name: "确认恢复所选内容" })).not.toBeInTheDocument();
});

it("keeps a partial restore visible and retryable; directory restore requires preferences", async () => {
  const user = userEvent.setup();
  const fake = fakeBackend({ backup_load: () => preview, backup_apply: () => ({ ...restored, failures: ["preferences:io"] }), backup_discard: () => null });
  backend.invoke.mockImplementation(fake.invoke);
  renderWithClient(<BackupRestore bytes={new Uint8Array()} fileName="fixture.leanflip" onBack={() => {}} />);
  await user.type(screen.getByLabelText("备份密码"), "fixture-password"); await user.click(screen.getByRole("button", { name: "解密并预览" }));
  const paths = await screen.findByRole("checkbox", { name: /同时恢复本机目录/ });
  await user.click(paths); await user.click(screen.getByRole("checkbox", { name: "恢复应用偏好" }));
  expect(paths).toBeDisabled(); expect(paths).not.toBeChecked();
  await user.click(screen.getByRole("button", { name: "确认恢复所选内容" }));
  expect(await screen.findByRole("status")).toHaveTextContent("部分内容已恢复");
  expect(screen.getByRole("button", { name: "确认恢复所选内容" })).toBeEnabled();
});
