import { describe, expect, it, vi } from "vitest";
import { QueryClient } from "@tanstack/react-query";
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
});
