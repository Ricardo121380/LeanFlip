import { describe, expect, it, vi } from "vitest";
import { createElement, type ReactNode } from "react";
import { act, renderHook } from "@testing-library/react";
import { toast } from "sonner";
import { QueryClientProvider, QueryClient, QueryObserver } from "@tanstack/react-query";
import { debounce, refreshAll, useAction } from "./queries";

describe("refresh coalescing", () => {
  it("debounce runs once after a burst", () => {
    vi.useFakeTimers();
    const fn = vi.fn();
    const run = debounce(fn, 150);
    run();
    run();
    vi.advanceTimersByTime(100);
    run();
    vi.advanceTimersByTime(149);
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(fn).toHaveBeenCalledTimes(1);
    run();
    run.cancel();
    vi.advanceTimersByTime(500);
    expect(fn).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });

  it("refreshAll leaves presets alone and reuses in-flight fetches", async () => {
    const client = new QueryClient();
    client.setQueryData(["presets", "claude"], []);
    client.setQueryData(["providers", "claude"], {});
    await refreshAll(client);
    expect(client.getQueryState(["presets", "claude"])?.isInvalidated).toBe(false);
    expect(client.getQueryState(["providers", "claude"])?.isInvalidated).toBe(true);
  });

  it.each(["quota-detect", "quota-krill-detect"])("does not repeat active %s after its cache write invalidates views", async (kind) => {
    const client = new QueryClient();
    const detect = vi.fn().mockResolvedValue({ adapter: "newapi" });
    const observer = new QueryObserver(client, { queryKey: [kind, "dialog"], queryFn: detect, staleTime: Infinity });
    const unsubscribe = observer.subscribe(() => {});
    await observer.refetch();
    expect(detect).toHaveBeenCalledTimes(1);
    client.setQueryData(["providers", "claude"], {});
    await refreshAll(client);
    await refreshAll(client);
    expect(detect).toHaveBeenCalledTimes(1);
    expect(observer.getCurrentResult().isFetching).toBe(false);
    expect(client.getQueryState(["providers", "claude"])?.isInvalidated).toBe(true);
    await observer.refetch();
    expect(detect).toHaveBeenCalledTimes(2);
    unsubscribe();
    client.clear();
  });
});

it("shows import warnings before the page success callback for partial and empty imports", async () => {
  const warning = vi.spyOn(toast, "warning").mockImplementation(() => 1);
  const client = new QueryClient();
  const success = vi.fn();
  const { result } = renderHook(() => useAction(async (created: string[]) => ({ created, warnings: ["Invalid field: providers.bad.models.m.capabilities.tools"] }), success), {
    wrapper: ({ children }: { children: ReactNode }) => createElement(QueryClientProvider, { client }, children),
  });
  for (const created of [["good"], []]) {
    await act(async () => { await result.current.mutateAsync(created); });
    expect(warning).toHaveBeenLastCalledWith("Invalid field: providers.bad.models.m.capabilities.tools");
    expect(success).toHaveBeenLastCalledWith(expect.objectContaining({ created }), created);
  }
  expect(warning).toHaveBeenCalledTimes(2);
  warning.mockRestore();
  client.clear();
});
