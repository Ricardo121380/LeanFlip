import { describe, expect, it } from "vitest";
import {
  REDACTED,
  endpointOf,
  fromEditorText,
  hasRedacted,
  modelOf,
  planBadge,
  slugify,
  toEditorText,
  uniqueId,
} from "./cards";
import type { Provider } from "./api";

const card = (settingsConfig: Record<string, unknown>): Provider => ({ id: "x", name: "x", settingsConfig });

describe("endpointOf", () => {
  it("reads each tool's base url field", () => {
    expect(endpointOf("claude", card({ env: { ANTHROPIC_BASE_URL: "https://a" } }))).toBe("https://a");
    expect(endpointOf("gemini", card({ env: { GOOGLE_GEMINI_BASE_URL: "https://g" } }))).toBe("https://g");
    expect(
      endpointOf("codex", card({ config: 'model = "m"\n[model_providers.x]\nbase_url = "https://c/v1"\n' })),
    ).toBe("https://c/v1");
    expect(endpointOf("opencode", card({ options: { baseURL: "https://o" } }))).toBe("https://o");
    expect(endpointOf("mcode", card({ options: { baseURL: "https://m" } }))).toBe("https://m");
    expect(endpointOf("openclaw", card({ baseUrl: "https://oc" }))).toBe("https://oc");
    expect(endpointOf("hermes", card({ base_url: "https://h" }))).toBe("https://h");
    expect(endpointOf("claude", card({ env: {} }))).toBeUndefined();
  });
});

describe("modelOf", () => {
  it("reads the model each client will use", () => {
    expect(modelOf("claude", card({ env: { ANTHROPIC_MODEL: "kimi-k2" } }))).toBe("kimi-k2");
    expect(modelOf("gemini", card({ env: {}, config: { model: { name: "gemini-2.5-pro" } } }))).toBe("gemini-2.5-pro");
    expect(modelOf("codex", card({ config: 'model = "gpt-5"\nmodel_provider = "x"\n' }))).toBe("gpt-5");
    expect(modelOf("grokbuild", card({ config: '[models]\ndefault = "grok-4"\n' }))).toBe("grok-4");
    expect(modelOf("opencode", card({ models: { "qwen3-coder": {} } }))).toBe("qwen3-coder");
    expect(modelOf("hermes", card({ models: [{ id: "deepseek-chat" }] }))).toBe("deepseek-chat");
    expect(modelOf("claude", card({ env: {} }))).toBeUndefined();
  });
});

describe("ids", () => {
  it("slugifies names and avoids collisions", () => {
    expect(slugify("Kimi Global")).toBe("kimi-global");
    expect(slugify("智谱 GLM")).toBe("glm");
    expect(slugify("！！")).toBe("provider");
    expect(uniqueId("kimi", ["kimi", "kimi-2"])).toBe("kimi-3");
    expect(uniqueId("new", [])).toBe("new");
  });
});

describe("planBadge", () => {
  it("maps plans to Plus / Team badges", () => {
    expect(planBadge("plus")).toEqual({ label: "Plus", variant: "plus" });
    expect(planBadge("team")).toEqual({ label: "Team", variant: "team" });
    expect(planBadge("pro")?.label).toBe("Pro");
    expect(planBadge("enterprise")?.label).toBe("Enterprise");
    expect(planBadge(null)).toBeNull();
  });
});

describe("card editor text", () => {
  it("keeps Codex config.toml byte-for-byte, including comments and odd spacing", () => {
    const config = '# my comment\nmodel   =  "gpt-5"   # trailing\n\n[model_providers.r]\nbase_url = "https://r"\n';
    const sc = { auth: { OPENAI_API_KEY: "sk" }, config };
    const text = toEditorText("codex", sc);
    expect(text.toml).toBe(config);
    const back = fromEditorText("codex", text, sc);
    expect(back).toEqual({ ok: true, value: sc });
  });

  it("keeps key order and unknown fields for JSON cards", () => {
    const sc = { zeta: 1, env: { B: "2", A: "1" }, alpha: { nested: true } };
    const text = toEditorText("json", sc);
    const back = fromEditorText("json", text);
    expect(back.ok && JSON.stringify(back.value)).toBe(JSON.stringify(sc));
  });

  it("keeps extra top-level Codex keys", () => {
    const sc = { auth: {}, config: "", extra: 1 };
    const text = toEditorText("codex", sc);
    expect(fromEditorText("codex", text, sc)).toEqual({ ok: true, value: { auth: {}, extra: 1, config: "" } });
  });

  it("reports JSON and TOML errors", () => {
    expect(fromEditorText("json", { json: "{" }).ok).toBe(false);
    expect(fromEditorText("json", { json: "[]" }).ok).toBe(false);
    const bad = fromEditorText("codex", { json: "{}", toml: "model = " });
    expect(bad.ok).toBe(false);
    expect(!bad.ok && bad.error).toMatch(/^TOML/);
  });

  it("detects redacted placeholders", () => {
    expect(hasRedacted(toEditorText("codex", { auth: { tokens: REDACTED }, config: "" }))).toBe(true);
    expect(hasRedacted(toEditorText("codex", { auth: {}, config: "" }))).toBe(false);
  });
});
