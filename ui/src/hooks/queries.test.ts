import { describe, expect, it, vi } from "vitest";
import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { debounce, refreshAll } from "./queries";

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
