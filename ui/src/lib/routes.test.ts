import { describe, expect, it } from "vitest";
import { applyRoutes, isClaudeSafeModelId, routeErrors, routesOf } from "./routes";

describe("Claude Desktop model routes", () => {
  it("validates ids like the core", () => {
    expect(isClaudeSafeModelId("claude-sonnet-4-5")).toBe(true);
    expect(isClaudeSafeModelId("anthropic/claude-opus-4-1")).toBe(true);
    expect(isClaudeSafeModelId("claude-sonnet-")).toBe(false);
    expect(isClaudeSafeModelId("claude-sonnet-4-5[1m]")).toBe(false);
    expect(isClaudeSafeModelId("gpt-5")).toBe(false);
  });

  it("round-trips meta and drops upstream mapping", () => {
    const provider = {
      meta: {
        other: 1,
        claudeDesktopModelRoutes: {
          "claude-sonnet-4-5": { model: "", labelOverride: "Sonnet" },
          "claude-opus-4-1": { supports1m: true },
        },
      },
    };
    const rows = routesOf(provider);
    expect(rows).toEqual([
      { id: "claude-sonnet-4-5", label: "Sonnet", supports1m: false },
      { id: "claude-opus-4-1", label: "", supports1m: true },
    ]);
    const next = applyRoutes(provider, [...rows, { id: "  ", label: "", supports1m: false }]);
    expect(next.meta).toEqual({
      other: 1,
      claudeDesktopModelRoutes: { "claude-sonnet-4-5": { labelOverride: "Sonnet" }, "claude-opus-4-1": { supports1m: true } },
    });
    expect(applyRoutes({ meta: { claudeDesktopModelRoutes: {} } }, []).meta).toBeUndefined();
    expect(routesOf({})).toEqual([]);
  });

  it("flags invalid and duplicate rows", () => {
    const row = (id: string) => ({ id, label: "", supports1m: false });
    expect(routeErrors([row("claude-haiku-4-5"), row("claude-haiku-4-5"), row("x")])).toEqual([null, "duplicate", "invalid"]);
  });
});
