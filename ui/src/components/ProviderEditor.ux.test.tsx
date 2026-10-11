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
function mount(provider: Provider | null = saved, handlers: Record<string, Handler> = {}) {
  mock.invoke = fakeBackend(handlers).invoke;
  function Host() {
    const [open, setOpen] = useState(true);
    return open ? <ProviderEditor tool="claude" open onOpenChange={setOpen} provider={provider ?? undefined} existingIds={["existing"]} /> : <p>Editor closed</p>;
  }
  return renderWithClient(<StrictMode><Host /></StrictMode>);
}

beforeEach(() => {
  vi.clearAllMocks();
  mock.loadPresets.mockResolvedValue(presets);
});

describe("Provider editor workflow", () => {
  it("preserves edits on Escape and only discards after explicit confirmation", async () => {
    mount();
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

  it("confirms replacement by another preset and reports unmatched searches", async () => {
    mount(null);
    const first = await screen.findByRole("button", { name: "First" });
    await userEvent.click(first);
    await userEvent.type(screen.getByLabelText("名称"), " draft");
    await userEvent.click(screen.getByRole("button", { name: "Second" }));
    await userEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "取消" }));
    expect(screen.getByLabelText("名称")).toHaveValue("First draft");
    await userEvent.click(screen.getByRole("button", { name: "Second" }));
    await userEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "替换为所选预设" }));
    expect(screen.getByLabelText("名称")).toHaveValue("Second");
    expect(screen.getByRole("button", { name: "Second" })).toHaveAttribute("aria-pressed", "true");
    await userEvent.type(screen.getByRole("searchbox"), "unmatched");
    expect(screen.getByText(/没有匹配的预设/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "自定义" })).toBeInTheDocument();
  });

  it("locks inputs and CodeMirror while saving, then retains the draft after failure", async () => {
    let fail!: (error: Error) => void;
    const pending = new Promise<never>((_, reject) => { fail = reject; });
    mount(saved, { update_provider: () => pending });
    await userEvent.type(screen.getByLabelText("名称"), " draft");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    expect(screen.getByLabelText("名称")).toBeDisabled();
    expect(screen.getByRole("button", { name: "关闭" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "取消" })).toBeDisabled();
    expect(screen.getByTestId("editor-json").querySelector(".cm-content")).toHaveAttribute("contenteditable", "false");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    await act(async () => fail(new Error("write denied")));
    expect(await screen.findByRole("alert")).toHaveTextContent("保存失败，修改已保留。 write denied");
    expect(screen.getByLabelText("名称")).toHaveValue("Existing draft");
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
    expect(screen.getByTestId("editor-json").querySelector(".cm-content")).toHaveAttribute("contenteditable", "true");
  });

  it("offers preset retry and explains duplicate IDs without disabling custom editing", async () => {
    mock.loadPresets.mockRejectedValue(new Error("offline"));
    mount(null);
    expect(await screen.findByRole("alert")).toHaveTextContent("预设加载失败");
    mock.loadPresets.mockResolvedValue(presets);
    await userEvent.click(screen.getByRole("button", { name: "重试" }));
    expect(await screen.findByRole("button", { name: "First" })).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("ID"), { target: { value: "existing" } });
    expect(screen.getByRole("alert")).toHaveTextContent("这个 ID 已存在");
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
  });
});
