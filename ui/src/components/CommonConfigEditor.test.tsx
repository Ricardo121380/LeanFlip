import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { cleanup, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { fakeBackend, renderWithClient } from "@/test/utils";
import i18n from "@/i18n";
const backend = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => backend.invoke(...args) }));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() } }));
vi.mock("./CodeEditor", () => ({ CodeEditor: ({ value, onChange, ariaLabel, readOnly }: any) => <textarea aria-label={ariaLabel} value={value} readOnly={readOnly} onChange={(e) => onChange(e.target.value)} /> }));
import { CommonConfigEditor } from "./CommonConfigEditor";
import { ProviderEditor } from "./ProviderEditor";

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
  expect(screen.getByRole("checkbox", { name: "Shell 快照" })).toBeChecked();
  fireEvent.change(screen.getByLabelText("配置 JSON"), { target: { value: "{" } });
  expect(screen.getByRole("button", { name: "保存并应用" })).toBeDisabled();
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
  await userEvent.click(screen.getByRole("checkbox", { name: "禁用响应存储" }));
  const config = (screen.getByLabelText(/config.toml/) as HTMLTextAreaElement).value;
  expect(config).toContain("disable_response_storage = true"); expect(config).toContain("# retain comment");
  await userEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() => expect(fake.calls.some((c) => c.cmd === "update_provider")).toBe(true));
});
