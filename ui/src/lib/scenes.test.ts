import { describe, expect, it } from "vitest";
import type { SceneList, SceneView, ToolPreview, ToolProviders } from "./api";
import i18n from "@/i18n";
import { applicable, currentScene, defaultEntry, isCidr, isHost, knownRule, missingIds, parseTime, ruleText, sceneShortcut, shortcutIndex, splitPreview, toggleEnabled, validRule } from "./scenes";

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

describe("scene shortcuts", () => {
  const key = (code: string, mods: Partial<Record<"metaKey" | "ctrlKey" | "shiftKey" | "altKey", boolean>>) => ({ code, metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...mods });
  it("maps ⌘⇧1–9 on macOS and Ctrl+Shift elsewhere to list positions", () => {
    expect(shortcutIndex(key("Digit1", { metaKey: true, shiftKey: true }), true)).toBe(0);
    expect(shortcutIndex(key("Digit9", { ctrlKey: true, shiftKey: true }), false)).toBe(8);
    expect(shortcutIndex(key("Digit2", { ctrlKey: true, shiftKey: true }), true)).toBe(-1);
    expect(shortcutIndex(key("Digit2", { metaKey: true }), true)).toBe(-1);
    expect(shortcutIndex(key("Digit2", { metaKey: true, shiftKey: true, altKey: true }), true)).toBe(-1);
    expect(shortcutIndex(key("Digit0", { metaKey: true, shiftKey: true }), true)).toBe(-1);
    expect(shortcutIndex(key("KeyA", { ctrlKey: true, shiftKey: true }), false)).toBe(-1);
    expect([sceneShortcut(0, true), sceneShortcut(8, false), sceneShortcut(9, true)]).toEqual(["⌘⇧1", "Ctrl+Shift+9", null]);
  });
});

describe("reminder rules", () => {
  const t = i18n.getFixedT("zh");
  it("validates times, networks and hostnames like the backend", () => {
    expect([parseTime("09:30"), parseTime("23:59"), parseTime("24:00"), parseTime("9:00")]).toEqual([570, 1439, null, null]);
    expect(["10.0.0.0/8", "192.168.1.0/24", "fd00::/8", "0.0.0.0/0"].every(isCidr)).toBe(true);
    expect(["10.0.0.0", "10.0.0.0/33", "300.0.0.0/8", "fd00::/129", "x/8"].some(isCidr)).toBe(false);
    expect(["intranet", "intranet.corp.example"].every(isHost)).toBe(true);
    expect(["", "https://intranet", "a..b", ".corp", "host:80"].some(isHost)).toBe(false);
    expect(validRule({ kind: "time", days: [1], start: "09:00", end: "18:00" })).toBe(true);
    expect(validRule({ kind: "time", days: [], start: "09:00", end: "18:00" })).toBe(false);
    expect(validRule({ kind: "time", days: [1], start: "09:00", end: "09:00" })).toBe(false);
  });

  it("describes rules in the interface language", () => {
    expect(ruleText({ kind: "time", days: [5, 1, 2, 3, 4], start: "09:00", end: "18:00" }, t)).toBe("工作日 09:00–18:00");
    expect(ruleText({ kind: "time", days: [6, 7], start: "22:00", end: "02:00" }, t)).toBe("周末 22:00–次日 02:00");
    expect(ruleText({ kind: "time", days: [1, 3], start: "08:00", end: "09:00" }, t)).toBe("周一、周三 08:00–09:00");
    expect(ruleText({ kind: "subnet", cidr: "10.0.0.0/8" }, t)).toBe("网段 10.0.0.0/8");
    expect(ruleText({ kind: "host", host: "intranet.example" }, i18n.getFixedT("en"))).toBe("intranet.example resolves");
    expect(knownRule({ kind: "bluetooth", device: "desk" })).toBeNull();
    expect(ruleText({ kind: "bluetooth", device: "desk" }, t)).toBe("新版本添加的规则");
  });
});

