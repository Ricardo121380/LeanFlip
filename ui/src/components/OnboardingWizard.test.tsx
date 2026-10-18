import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderWithClient, fakeBackend } from "@/test/utils";
import { OnboardingWizard } from "./OnboardingWizard";

const backend = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => backend.invoke(...args) }));
const preview = { previewId: 42, directory: "/fixture/source", found: true, items: [
  { index: 0, tool: "codex", name: "Relay", skipReason: null, notes: [], needsApiKey: true },
  { index: 1, tool: "claude", name: "Team", skipReason: null, notes: [], needsApiKey: false },
  { index: 2, tool: "codex", name: "Login", skipReason: { code: "login", zh: "需要登录", en: "Login required" }, notes: [], needsApiKey: false },
] };

beforeEach(() => {
  backend.invoke = fakeBackend({
    provider_import_scan: () => preview,
    provider_import_apply: () => ({ created: [["codex", "relay"]], skipped: [], accountsAdded: [] }),
    complete_onboarding: () => null,
    provider_import_discard: () => null,
  }).invoke;
});
afterEach(cleanup);

async function openImport() {
  await userEvent.click(screen.getByRole("button", { name: "下一步" }));
  await userEvent.click(screen.getByRole("button", { name: "扫描并预览" }));
  await screen.findByText("Relay");
}

describe("first-run guide", () => {
  it("imports only selected providers with explicit missing keys, then completes", async () => {
    const close = vi.fn();
    renderWithClient(<OnboardingWizard onClose={close} />);
    expect(screen.getByRole("dialog")).toHaveAccessibleName("欢迎使用 LeanFlip");
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("radio", { name: /Superba/ }));
    await userEvent.click(screen.getByRole("button", { name: "扫描并预览" }));
    await screen.findByText("Relay");
    expect(backend.invoke).toHaveBeenCalledWith("provider_import_scan", { source: "superba", directory: null });
    expect(screen.getByRole("checkbox", { name: /Login/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: "导入所选 2 项" })).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox", { name: /Team/ }));
    await userEvent.type(screen.getByLabelText("Relay 的 API Key"), "fixture-key");
    await userEvent.click(screen.getByRole("button", { name: "导入所选 1 项" }));
    await screen.findByText("已导入 1 项，跳过 0 项");
    expect(backend.invoke).toHaveBeenCalledWith("provider_import_apply", { previewId: 42, selected: [{ index: 0, apiKey: "fixture-key" }] });
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "开始使用" }));
    await waitFor(() => expect(close).toHaveBeenCalledOnce());
    expect(backend.invoke).toHaveBeenCalledWith("complete_onboarding");
  });

  it("clears previews and keys when the source or path changes", async () => {
    renderWithClient(<OnboardingWizard onClose={vi.fn()} />);
    await openImport();
    await userEvent.type(screen.getByLabelText("Relay 的 API Key"), "fixture-key");
    await userEvent.click(screen.getByRole("radio", { name: /AI Toolbox/ }));
    expect(screen.queryByText("Relay")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "扫描并预览" }));
    expect(await screen.findByLabelText("Relay 的 API Key")).toHaveValue("");
    await userEvent.click(screen.getByText("配置在其他位置？"));
    await userEvent.type(screen.getByLabelText("来源数据目录"), "/other");
    expect(screen.queryByText("Relay")).not.toBeInTheDocument();
  });

  it("keeps selections on an import failure and permits retry", async () => {
    let failed = true;
    backend.invoke = fakeBackend({ provider_import_scan: () => ({ ...preview, items: [preview.items[1]] }), provider_import_apply: () => {
      if (failed) throw new Error("fixture import failed");
      return { created: [["claude", "team"]], skipped: [], accountsAdded: [] };
    } }).invoke;
    renderWithClient(<OnboardingWizard onClose={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "扫描并预览" }));
    await screen.findByText("Team");
    await userEvent.click(screen.getByRole("button", { name: "导入所选 1 项" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("fixture import failed");
    expect(screen.getByRole("checkbox", { name: /Team/ })).toBeChecked();
    failed = false;
    await userEvent.click(screen.getByRole("button", { name: "导入所选 1 项" }));
    await screen.findByText("已导入 1 项，跳过 0 项");
  });

  it("prevents close and source changes while importing", async () => {
    let resolve!: (r: unknown) => void;
    backend.invoke = fakeBackend({ provider_import_scan: () => ({ ...preview, items: [preview.items[1]] }), provider_import_apply: () => new Promise((r) => { resolve = r; }) }).invoke;
    const close = vi.fn();
    renderWithClient(<OnboardingWizard onClose={close} />);
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "扫描并预览" }));
    await screen.findByText("Team");
    await userEvent.click(screen.getByRole("button", { name: "导入所选 1 项" }));
    expect(screen.getByRole("button", { name: "关闭" })).toBeDisabled();
    expect(screen.getByRole("radio", { name: /Superba/ })).toBeDisabled();
    await userEvent.keyboard("{Escape}");
    expect(close).not.toHaveBeenCalled();
    resolve({ created: [], skipped: [], accountsAdded: [] });
    await screen.findByText("已导入 0 项，跳过 0 项");
  });

  it("persists skipping and keeps the dialog open when completion cannot be saved", async () => {
    let failed = true;
    backend.invoke = fakeBackend({ complete_onboarding: () => { if (failed) throw new Error("cannot save"); } }).invoke;
    const close = vi.fn();
    renderWithClient(<OnboardingWizard onClose={close} />);
    await userEvent.click(screen.getByRole("button", { name: "跳过向导" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("cannot save");
    expect(close).not.toHaveBeenCalled();
    failed = false;
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(close).toHaveBeenCalledOnce());
  });

  it("explains empty sources and scan failures without blocking the guide", async () => {
    let failed = true;
    backend.invoke = fakeBackend({ provider_import_scan: () => {
      if (failed) throw new Error("cannot read source");
      return { ...preview, found: false, items: [] };
    } }).invoke;
    renderWithClient(<OnboardingWizard onClose={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await userEvent.click(screen.getByRole("button", { name: "扫描并预览" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("cannot read source");
    failed = false;
    await userEvent.click(screen.getByRole("button", { name: "扫描并预览" }));
    await screen.findByText(/此目录没有找到来源配置/);
    await userEvent.click(screen.getByRole("button", { name: "暂不导入，继续" }));
    expect(screen.getByRole("heading", { name: "准备好了，开始第一次切换" })).toHaveFocus();
  });
});


it("returns keyboard focus to the settings entry after closing", async () => {
  function SettingsEntry() {
    const [open, setOpen] = useState(false);
    return <><button onClick={() => setOpen(true)}>使用向导</button>{open && <OnboardingWizard onClose={() => setOpen(false)} />}</>;
  }
  renderWithClient(<SettingsEntry />);
  const entry = screen.getByRole("button", { name: "使用向导" });
  await userEvent.click(entry);
  await screen.findByRole("dialog");
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  await waitFor(() => expect(entry).toHaveFocus());
});
