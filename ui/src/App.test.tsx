import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { fakeBackend, renderWithClient } from "@/test/utils";

const backend = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => backend.invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@/pages/ToolPage", () => ({ ToolPage: ({ tool }: { tool: string }) => <h1>{tool}</h1> }));
vi.mock("@/pages/SettingsPage", () => ({ SettingsPage: () => <h1>Settings content</h1> }));

import App from "./App";

beforeEach(() => {
  window.history.replaceState(null, "", "/");
  backend.invoke = fakeBackend({
    get_overview: () => [],
    get_settings: () => ({ app: { language: "zh" } }),
    get_recovery_notices: () => [],
  }).invoke;
});
afterEach(() => window.history.replaceState(null, "", "/"));

describe("App navigation", () => {
  it("falls back from an unknown tool instead of rendering a broken page", async () => {
    window.history.replaceState(null, "", "/#tool=unknown");
    renderWithClient(<App />);
    expect(screen.getByRole("heading", { name: "claude" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Claude Code" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("navigation", { name: "管理" })).toBeInTheDocument();
    await waitFor(() => expect(backend.invoke).toHaveBeenCalledWith("get_overview"));
  });

  it("keeps the selected navigation item in sync with clicks and hash history", async () => {
    renderWithClient(<App />);
    await userEvent.click(screen.getByRole("link", { name: "设置" }));
    expect(await screen.findByRole("heading", { name: "Settings content" })).toBeInTheDocument();
    expect(window.location.hash).toBe("#settings");
    expect(screen.getByRole("link", { name: "设置" })).toHaveAttribute("aria-current", "page");
    act(() => {
      window.history.replaceState(null, "", "/#tool=gemini");
      window.dispatchEvent(new HashChangeEvent("hashchange"));
    });
    expect(screen.getByRole("heading", { name: "gemini" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Gemini CLI" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("link", { name: "设置" })).not.toHaveAttribute("aria-current");
  });

  it("offers a retry when the overview fails without blocking tool navigation", async () => {
    let failed = true;
    backend.invoke = fakeBackend({
      get_overview: () => { if (failed) throw new Error("offline"); return []; },
      get_settings: () => ({ app: { language: "zh" } }),
      get_recovery_notices: () => [],
    }).invoke;
    renderWithClient(<App />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("工具状态暂不可用");
    failed = false;
    await userEvent.click(within(alert).getByRole("button", { name: "重试" }));
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
    expect(screen.getByRole("heading", { name: "claude" })).toBeInTheDocument();
  });
});
