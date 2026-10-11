import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { StrictMode } from "react";
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { toast } from "sonner";
import i18n from "@/i18n";
import type { SettingsView } from "@/lib/api";
import { fakeBackend, renderWithClient, type Handler } from "@/test/utils";

const backend = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => backend.invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@/pages/ToolPage", () => ({ ToolPage: ({ tool }: { tool: string }) => <h1>{tool}</h1> }));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn() } }));

import App from "./App";

function settings(): SettingsView {
  return {
    app: { language: "zh", dirOverrides: {}, autostart: false, backgroundRefreshHours: 168, closeToTray: true },
    keepCodexLogin: false,
    dataDir: "/test/.cc-switch-lite",
    home: "/test",
    livePaths: {},
    dirs: { codexDir: "/test/.codex" },
    ccswitchDir: "/test/.cc-switch",
  };
}

function start(handlers: Record<string, Handler> = {}) {
  const fake = fakeBackend({ get_overview: () => [], get_settings: settings, get_recovery_notices: () => [], ...handlers });
  backend.invoke = fake.invoke;
  renderWithClient(<StrictMode><App /></StrictMode>);
  return fake;
}

async function editSettings() {
  await userEvent.click(screen.getByRole("link", { name: "设置" }));
  const directory = await screen.findByLabelText("Codex (~/.codex)");
  await userEvent.type(directory, "/my/draft");
  return directory;
}

beforeEach(async () => {
  vi.clearAllMocks();
  window.history.replaceState(null, "", "/#tool=claude");
  await i18n.changeLanguage("zh");
});
afterEach(() => window.history.replaceState(null, "", "/"));

describe("Settings navigation protection", () => {
  it("keeps the draft, URL, selection and scroll position when a link is cancelled, then discards on confirmation", async () => {
    start();
    const directory = await editSettings();
    const main = screen.getByRole("main");
    main.scrollTop = 180;
    await userEvent.click(screen.getByRole("link", { name: "Gemini CLI" }));
    let dialog = await screen.findByRole("alertdialog");
    expect(window.location.hash).toBe("#settings");
    expect(within(dialog).getByRole("button", { name: "取消" })).toHaveFocus();
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(directory).toHaveValue("/my/draft");
    expect(screen.getByRole("link", { name: "设置" })).toHaveAttribute("aria-current", "page");
    expect(main.scrollTop).toBe(180);
    await userEvent.click(screen.getByRole("link", { name: "Gemini CLI" }));
    dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "放弃修改并离开" }));
    expect(await screen.findByRole("heading", { name: "gemini" })).toBeInTheDocument();
    expect(window.location.hash).toBe("#tool=gemini");
    expect(main.scrollTop).toBe(0);
    await userEvent.click(screen.getByRole("link", { name: "设置" }));
    expect(await screen.findByLabelText("Codex (~/.codex)")).toHaveValue("");
  });

  it("restores a cancelled Back without rewriting history, and enters the Back target after confirmation", async () => {
    start();
    await editSettings();
    act(() => window.history.back());
    let dialog = await screen.findByRole("alertdialog");
    expect(window.location.hash).toBe("#tool=claude");
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    await waitFor(() => expect(window.location.hash).toBe("#settings"));
    expect(screen.getByLabelText("Codex (~/.codex)")).toHaveValue("/my/draft");
    expect(screen.getByRole("link", { name: "设置" })).toHaveAttribute("aria-current", "page");
    act(() => window.history.back());
    dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "放弃修改并离开" }));
    expect(await screen.findByRole("heading", { name: "claude" })).toBeInTheDocument();
    expect(window.location.hash).toBe("#tool=claude");
    act(() => window.history.forward());
    expect(await screen.findByLabelText("Codex (~/.codex)")).toHaveValue("");
    expect(window.location.hash).toBe("#settings");
  });

  it("protects Forward while preserving its destination after cancellation", async () => {
    start();
    await userEvent.click(screen.getByRole("link", { name: "设置" }));
    await screen.findByRole("button", { name: "保存设置" });
    await userEvent.click(screen.getByRole("link", { name: "Gemini CLI" }));
    await screen.findByRole("heading", { name: "gemini" });
    act(() => window.history.back());
    await userEvent.type(await screen.findByLabelText("Codex (~/.codex)"), "/my/draft");
    act(() => window.history.forward());
    let dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    await waitFor(() => expect(window.location.hash).toBe("#settings"));
    expect(screen.getByLabelText("Codex (~/.codex)")).toHaveValue("/my/draft");
    act(() => window.history.forward());
    dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "放弃修改并离开" }));
    expect(await screen.findByRole("heading", { name: "gemini" })).toBeInTheDocument();
  });

  it("keeps the correct history position after a traversal between identical settings hashes", async () => {
    start();
    await userEvent.click(screen.getByRole("link", { name: "设置" }));
    await screen.findByRole("button", { name: "保存设置" });
    await userEvent.click(screen.getByRole("link", { name: "Gemini CLI" }));
    await screen.findByRole("heading", { name: "gemini" });
    await editSettings();
    const sameHashTraversal = new Promise<void>((resolve) => window.addEventListener("popstate", () => resolve(), { once: true }));
    await act(async () => { window.history.go(-2); await sameHashTraversal; });
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(screen.getByLabelText("Codex (~/.codex)")).toHaveValue("/my/draft");
    act(() => window.history.back());
    let dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    await waitFor(() => expect(window.location.hash).toBe("#settings"));
    act(() => window.history.forward());
    dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "放弃修改并离开" }));
    expect(await screen.findByRole("heading", { name: "gemini" })).toBeInTheDocument();
  });

  it("rejects link and history navigation during save, and retains the draft after failure", async () => {
    let reject!: (error: Error) => void;
    const pending = new Promise((_resolve, rejectPromise) => { reject = rejectPromise; });
    start({ save_settings: () => pending });
    await editSettings();
    await userEvent.click(screen.getByRole("button", { name: "保存设置" }));
    await screen.findByRole("button", { name: "正在保存…" });
    await userEvent.click(screen.getByRole("link", { name: "Gemini CLI" }));
    expect(window.location.hash).toBe("#settings");
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(toast.info).toHaveBeenCalledWith("操作正在进行，请完成后再切换页面。", expect.anything());
    act(() => window.history.back());
    await waitFor(() => expect(toast.info).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(window.location.hash).toBe("#settings"));
    expect(screen.getByRole("link", { name: "设置" })).toHaveAttribute("aria-current", "page");
    await act(async () => reject(new Error("write denied")));
    await waitFor(() => expect(screen.getByRole("button", { name: "保存设置" })).toBeEnabled());
    expect(screen.getByLabelText("Codex (~/.codex)")).toHaveValue("/my/draft");
    expect(window.location.hash).toBe("#settings");
  });

  it("navigates without confirmation when settings are unchanged or reset", async () => {
    start();
    await userEvent.click(screen.getByRole("link", { name: "设置" }));
    await screen.findByRole("button", { name: "保存设置" });
    await userEvent.click(screen.getByRole("link", { name: "Gemini CLI" }));
    expect(await screen.findByRole("heading", { name: "gemini" })).toBeInTheDocument();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    await editSettings();
    await userEvent.click(screen.getByRole("button", { name: "放弃修改" }));
    await userEvent.click(screen.getByRole("link", { name: "Claude Code" }));
    expect(await screen.findByRole("heading", { name: "claude" })).toBeInTheDocument();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  it("uses one confirmation for repeated navigation attempts and only accepts the latest destination", async () => {
    start();
    await editSettings();
    await userEvent.click(screen.getByRole("link", { name: "Gemini CLI" }));
    const dialog = await screen.findByRole("alertdialog");
    // Browser history remains usable while a modal is open.
    act(() => window.history.back());
    await waitFor(() => expect(window.location.hash).toBe("#tool=claude"));
    expect(screen.getAllByRole("alertdialog")).toHaveLength(1);
    await userEvent.click(within(dialog).getByRole("button", { name: "放弃修改并离开" }));
    expect(await screen.findByRole("heading", { name: "claude" })).toBeInTheDocument();
    expect(window.location.hash).toBe("#tool=claude");
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  it("restores an unmanaged hash change on cancel without dropping the draft", async () => {
    start();
    await editSettings();
    act(() => { window.location.hash = "#tool=gemini"; });
    const dialog = await screen.findByRole("alertdialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    await waitFor(() => expect(window.location.hash).toBe("#settings"));
    expect(screen.getByLabelText("Codex (~/.codex)")).toHaveValue("/my/draft");
    expect(screen.getByRole("link", { name: "设置" })).toHaveAttribute("aria-current", "page");
  });
});


describe("Import navigation protection", () => {
  it("keeps an in-flight file preview mounted until its backend staging is complete", async () => {
    let finish!: () => void;
    const pending = new Promise<void>((resolve) => { finish = resolve; });
    start({ ccswitch_export_load: async () => { await pending; return { format: "json", items: [] }; } });
    await userEvent.click(screen.getByRole("link", { name: "数据与迁移" }));
    await userEvent.click(await screen.findByRole("tab", { name: "导入文件" }));
    await userEvent.upload(screen.getByTestId("export-file"), new File(["{}"], "providers.json", { type: "application/json" }));
    await waitFor(() => expect(screen.getByRole("tab", { name: "本机数据目录" })).toBeDisabled());
    await userEvent.click(screen.getByRole("link", { name: "Gemini CLI" }));
    expect(window.location.hash).toBe("#import");
    expect(screen.getByTestId("export-file")).toBeInTheDocument();
    expect(toast.info).toHaveBeenCalledWith("操作正在进行，请完成后再切换页面。", expect.anything());
    await act(async () => finish());
    await waitFor(() => expect(screen.getByRole("tab", { name: "本机数据目录" })).toBeEnabled());
    await userEvent.click(screen.getByRole("link", { name: "Gemini CLI" }));
    expect(await screen.findByRole("heading", { name: "gemini" })).toBeInTheDocument();
  });
});
