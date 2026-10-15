import { describe, expect, it } from "vitest";
import type { SceneList, SceneView, ToolPreview, ToolProviders } from "./api";
import { applicable, currentScene, defaultEntry, missingIds, splitPreview, toggleEnabled } from "./scenes";

const view = (id: string, status: SceneView["status"]): SceneView => ({ id, name: id, tools: {}, status, pending: [], missing: [] });
const preview = (tool: ToolPreview["tool"], extra: Partial<ToolPreview> = {}): ToolPreview => ({ tool, kind: "switch", from: null, steps: [], missing: [], error: null, ...extra });

describe("currentScene", () => {
  it("prefers the last applied scene, flags drift, and falls back to any matching scene", () => {
    const list = (last: string | null, ...scenes: SceneView[]): SceneList => ({ last, scenes });
    expect(currentScene(undefined)).toBeNull();
    expect(currentScene(list("b", view("a", "active"), view("b", "active")))).toEqual({ scene: view("b", "active"), drifted: false });
    expect(currentScene(list("b", view("a", "active"), view("b", "drifted")))).toEqual({ scene: view("b", "drifted"), drifted: true });
    expect(currentScene(list("b", view("a", "active"), view("b", "idle")))?.scene.id).toBe("a");
    expect(currentScene(list(null, view("a", "idle")))).toBeNull();
  });
});

describe("splitPreview", () => {
  it("keeps tools with steps, missing cards or errors apart from matching ones", () => {
    const step = { action: { kind: "switch" as const, id: "b" }, name: "B" };
    const tools = [preview("claude", { steps: [step] }), preview("codex"), preview("gemini", { missing: ["x"] }), preview("opencode", { error: "broken", steps: [step] })];
    const { changing, inSync } = splitPreview(tools);
    expect(changing.map((t) => t.tool)).toEqual(["claude", "gemini", "opencode"]);
    expect(inSync.map((t) => t.tool)).toEqual(["codex"]);
    expect(changing.map(applicable)).toEqual([true, false, false]);
  });
});

describe("scene drafts", () => {
  const data = (extra: Partial<ToolProviders>): ToolProviders => ({
    tool: "opencode",
    kind: "coexist",
    providers: [
      { id: "x", name: "X", settingsConfig: {} },
      { id: "y", name: "Y", settingsConfig: {} },
    ],
    current: null,
    inLive: ["y", "mine"],
    liveError: null,
    paths: [],
    ...extra,
  });

  it("starts a newly included tool from its current state", () => {
    expect(defaultEntry("coexist", data({}))).toEqual({ enabled: ["y"] });
    expect(defaultEntry("switch", data({ current: "y" }))).toEqual({ current: "y" });
    expect(defaultEntry("switch", data({ selected: "x", current: null }))).toEqual({ current: "x" });
    expect(defaultEntry("switch", data({}))).toEqual({ current: "x" });
    expect(defaultEntry("switch", data({ providers: [] }))).toBeNull();
    expect(defaultEntry("switch", undefined)).toBeNull();
  });

  it("toggles enabled cards and drops a Hermes default that is no longer enabled", () => {
    expect(toggleEnabled({ enabled: ["x"] }, "y", true)).toEqual({ enabled: ["x", "y"] });
    expect(toggleEnabled({ enabled: ["x", "y"], current: "y" }, "y", false)).toEqual({ enabled: ["x"] });
    expect(toggleEnabled({ enabled: ["x"], current: "x" }, "x", true)).toEqual({ enabled: ["x"], current: "x" });
  });

  it("lists referenced cards that no longer exist", () => {
    expect(missingIds({ current: "gone", enabled: ["x", "gone"] }, data({}).providers)).toEqual(["gone"]);
    expect(missingIds(undefined, [])).toEqual([]);
  });
});
