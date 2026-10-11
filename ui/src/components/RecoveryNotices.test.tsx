import { StrictMode } from "react";
import { expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { keys } from "@/hooks/queries";

const backend = vi.hoisted(() => ({ notices: [] as string[] }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => [...backend.notices]),
}));

import { RecoveryNotices } from "./RecoveryNotices";

it("keeps recovery results visible through refetches until dismissed and shows later notices", async () => {
  backend.notices = ["claude: 仅部分恢复\n/tmp/home/.claude/settings.json", "codex: 恢复失败，请检查客户端配置后重试。"];
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <StrictMode><QueryClientProvider client={client}><RecoveryNotices /></QueryClientProvider></StrictMode>,
  );
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent("/tmp/home/.claude/settings.json");
  expect(alert).toHaveTextContent("恢复失败");
  await client.invalidateQueries({ queryKey: keys.recovery });
  expect(screen.getByRole("alert")).toHaveTextContent("仅部分恢复");

  await userEvent.click(screen.getByRole("button", { name: "关闭恢复提示" }));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  await client.invalidateQueries({ queryKey: keys.recovery });
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();

  backend.notices.push("gemini: 已补完上次未完成的写入。");
  await client.invalidateQueries({ queryKey: keys.recovery });
  await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("gemini:"));
  expect(screen.getByRole("alert")).not.toHaveTextContent("claude:");
  client.clear();
});
