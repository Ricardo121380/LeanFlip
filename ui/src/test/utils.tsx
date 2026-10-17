import type { ReactElement } from "react";
import { render } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { vi } from "vitest";

export type Handler = (args: Record<string, unknown> | undefined) => unknown;

/** 按命令名回答的假 invoke；记录所有调用。 */
export function fakeBackend(handlers: Record<string, Handler>) {
  const calls: { cmd: string; args?: Record<string, unknown> }[] = [];
  const invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args });
    const handler = handlers[cmd] ?? (cmd === "credential_commands"
      ? () => ({ commands: {}, destinations: {}, fingerprint: "fixture-empty", approved: true })
      : undefined);
    if (!handler) throw new Error(`unexpected command ${cmd}`);
    return handler(args);
  });
  return { invoke, calls };
}

export function renderWithClient(ui: ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}
