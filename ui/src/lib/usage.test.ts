import { describe, expect, it } from "vitest";
import type { SwitchEvent } from "./api";
import { axisLabel, coexistLanes, compact, dayKey, days, formatCompact, hitRate, niceTicks, switchCount, switchSegments } from "./usage";

const event = (ts: number, tool: SwitchEvent["tool"], provider: string, enabled = true): SwitchEvent => ({ ts, tool, provider, name: provider.toUpperCase(), enabled });

describe("usage helpers", () => {
  it("lists the days of a range and the range before it", () => {
    const now = new Date(2026, 9, 13, 15, 0);
    expect(days(7, now)).toEqual(["2026-10-07", "2026-10-08", "2026-10-09", "2026-10-10", "2026-10-11", "2026-10-12", "2026-10-13"]);
    expect(days(7, now, 1)[6]).toBe("2026-10-06");
    expect(dayKey(new Date(2026, 0, 2))).toBe("2026-01-02");
  });

  it("formats large numbers in 万 / 亿 for Chinese and K / M / B for English", () => {
    expect(formatCompact(437_000_000, "zh")).toBe("4.37 亿");
    expect(formatCompact(57_450_000, "zh")).toBe("5745 万");
    expect(formatCompact(8_820, "zh")).toBe("8,820");
    expect(formatCompact(1_480_000, "en")).toBe("1.48M");
    expect(compact(12_900, "en")).toEqual({ value: "12.9", unit: "K" });
    expect(axisLabel(150_000_000, "zh")).toBe("1.5 亿");
  });

  it("picks round axis steps", () => {
    expect(niceTicks(3.7e8)).toEqual({ top: 4e8, step: 1e8 });
    expect(niceTicks(7, true)).toEqual({ top: 8, step: 2 });
    expect(niceTicks(0)).toEqual({ top: 1, step: 1 });
  });

  it("computes the cache hit rate from all input-side tokens", () => {
    expect(hitRate({ input: 10, cacheRead: 80, cacheWrite: 10, output: 5, requests: 1 })).toBe(0.8);
    expect(hitRate({ input: 0, cacheRead: 0, cacheWrite: 0, output: 5, requests: 1 })).toBeNull();
  });

  it("turns switch events into segments and skips times with nothing selected", () => {
    const { since, segments } = switchSegments([event(30, "claude", "b"), event(10, "claude", "a"), event(50, "claude", ""), event(60, "claude", "a")], 100);
    expect(since).toBe(10);
    expect(segments.map((s) => [s.provider, s.start, s.end, s.live])).toEqual([["a", 10, 30, false], ["b", 30, 50, false], ["a", 60, 100, true]]);
  });

  it("turns enable and disable events into lanes per provider", () => {
    const { lanes } = coexistLanes([event(10, "opencode", "a"), event(20, "opencode", "b"), event(40, "opencode", "b", false), event(70, "opencode", "b")], 100);
    expect(lanes.get("a")!.map((s) => [s.start, s.end, s.live])).toEqual([[10, 100, true]]);
    expect(lanes.get("b")!.map((s) => [s.start, s.end, s.live])).toEqual([[20, 40, false], [70, 100, true]]);
  });

  it("does not count the first recorded state of each tool as a switch", () => {
    const events = [event(10, "claude", "a"), event(10, "opencode", "x"), event(20, "claude", "b"), event(30, "opencode", "y"), event(40, "claude", "a")];
    expect(switchCount(events, 0, 100)).toBe(3);
    expect(switchCount(events, 25, 100)).toBe(2);
  });
});
