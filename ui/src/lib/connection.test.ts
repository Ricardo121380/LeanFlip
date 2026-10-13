import { describe, expect, it } from "vitest";
import { readConnection, writeConnection, type Connection } from "./connection";
import { toEditorText } from "./cards";
import { getTomlValue } from "./toml-edit";

function ready(connection: Connection) {
  if (connection.state !== "ready") throw new Error(`expected ready, got ${connection.state}`);
  return connection;
}

describe("connection fields", () => {
  it("reads and writes Claude connection and model fields", () => {
    let text = toEditorText("json", { env: { ANTHROPIC_BASE_URL: "https://a.example.com", ANTHROPIC_AUTH_TOKEN: "t" }, permissions: { allow: [] } });
    const before = ready(readConnection("claude", text, false));
    expect(before.values).toMatchObject({ baseUrl: "https://a.example.com", authField: "ANTHROPIC_AUTH_TOKEN", apiKey: "t", model: "" });

    text = writeConnection("claude", text, "sonnetModel", "my-sonnet");
    text = writeConnection("claude", text, "baseUrl", "");
    const value = JSON.parse(text.json);
    expect(value.env).toEqual({ ANTHROPIC_AUTH_TOKEN: "t", ANTHROPIC_DEFAULT_SONNET_MODEL: "my-sonnet" });
    expect(value.permissions).toEqual({ allow: [] });
  });

  it("moves the secret when the Claude auth field changes and keeps the field when the key is cleared", () => {
    let text = toEditorText("json", { env: { ANTHROPIC_AUTH_TOKEN: "secret" } });
    text = writeConnection("claude", text, "authField", "ANTHROPIC_API_KEY");
    expect(JSON.parse(text.json).env).toEqual({ ANTHROPIC_API_KEY: "secret" });
    text = writeConnection("claude", text, "apiKey", "");
    expect(ready(readConnection("claude", text, false)).values.authField).toBe("ANTHROPIC_API_KEY");
  });

  it("locks Claude connection fields for Bedrock", () => {
    const text = toEditorText("json", { env: { CLAUDE_CODE_USE_BEDROCK: "1", AWS_REGION: "us-east-1" } });
    expect(ready(readConnection("claude", text, false)).blocked).toMatchObject({ reason: "advancedAuth" });
  });

  it("edits Codex TOML in place and only allows base_url on custom providers", () => {
    const config = 'model_provider = "leanflip"\nmodel = "m1"\n\n[model_providers.leanflip]\nname = "My connection"\nbase_url = "https://old/v1"\n';
    let text = toEditorText("codex", { auth: { OPENAI_API_KEY: "k" }, config });
    expect(ready(readConnection("codex", text, false)).values).toMatchObject({ baseUrl: "https://old/v1", apiKey: "k", model: "m1" });

    text = writeConnection("codex", text, "baseUrl", "https://new/v1");
    text = writeConnection("codex", text, "reasoning", "high");
    text = writeConnection("codex", text, "apiKey", "k2");
    expect(text.toml).toBe('model_provider = "leanflip"\nmodel = "m1"\nmodel_reasoning_effort = "high"\n\n[model_providers.leanflip]\nname = "My connection"\nbase_url = "https://new/v1"\n');
    expect(JSON.parse(text.json)).toEqual({ OPENAI_API_KEY: "k2" });

    const builtIn = toEditorText("codex", { auth: {}, config: 'model_provider = "ollama"\nmodel = "m"\n' });
    expect(ready(readConnection("codex", builtIn, false)).blocked).toMatchObject({ reason: "builtIn", id: "ollama" });
    expect(writeConnection("codex", builtIn, "baseUrl", "http://x")).toEqual(builtIn);
  });

  it("edits openai_base_url for the built-in OpenAI provider", () => {
    let text = toEditorText("codex", { auth: { OPENAI_API_KEY: "k" }, config: 'model = "m"\n' });
    expect(ready(readConnection("codex", text, false)).blocked).toBeUndefined();
    text = writeConnection("codex", text, "baseUrl", "https://proxy.example/v1");
    expect(text.toml).toBe('model = "m"\nopenai_base_url = "https://proxy.example/v1"\n');
    expect(ready(readConnection("codex", text, false)).values.baseUrl).toBe("https://proxy.example/v1");
  });

  it("does not treat a leftover Codex provider table as the one in use", () => {
    const config = 'model = "m"\n\n[model_providers.custom]\nbase_url = "https://unused.example/v1"\n';
    let text = toEditorText("codex", { auth: {}, config });
    const connection = ready(readConnection("codex", text, false));
    expect(connection.values.baseUrl).toBe("");
    expect(connection.fields.find((field) => field.id === "baseUrl")?.hint).toBe("openai_base_url");
    text = writeConnection("codex", text, "baseUrl", "https://proxy.example/v1");
    expect(text.toml).toBe('model = "m"\nopenai_base_url = "https://proxy.example/v1"\n\n[model_providers.custom]\nbase_url = "https://unused.example/v1"\n');
  });

  it("writes the Codex key into the nested auth object when extra keys exist", () => {
    let text = toEditorText("codex", { auth: { OPENAI_API_KEY: "" }, config: "", extra: true });
    text = writeConnection("codex", text, "apiKey", "k");
    expect(JSON.parse(text.json)).toEqual({ auth: { OPENAI_API_KEY: "k" }, extra: true });
  });

  it("edits the default Grok model table", () => {
    const config = '[models]\ndefault = "connection"\n\n[model.connection]\nmodel = "m"\nbase_url = ""\napi_key = ""\ncontext_window = 32768\n';
    let text = toEditorText("codex", { config });
    text = writeConnection("grokbuild", text, "contextWindow", "500000");
    text = writeConnection("grokbuild", text, "apiKey", "xai");
    expect(getTomlValue(text.toml!, ["model", "connection"], "context_window")).toBe(500000);
    expect(ready(readConnection("grokbuild", text, false)).values).toMatchObject({ apiKey: "xai", contextWindow: "500000" });
  });

  it("keeps per-model settings when the model list of a coexist tool changes", () => {
    let text = toEditorText("json", { options: { baseURL: "", apiKey: "" }, models: { a: { limit: 1 }, b: {} } });
    text = writeConnection("opencode", text, "models", ["a", "c"]);
    expect(JSON.parse(text.json).models).toEqual({ a: { limit: 1 }, c: {} });

    let pi = toEditorText("json", { baseUrl: "", models: [{ id: "a", name: "A", reasoning: true }] });
    pi = writeConnection("pi", pi, "models", ["a", "b"]);
    expect(JSON.parse(pi.json).models).toEqual([{ id: "a", name: "A", reasoning: true }, { id: "b", name: "b" }]);
    expect(ready(readConnection("pi", pi, false)).values.models).toEqual(["a", "b"]);
  });

  it("reports official and unparsable configurations", () => {
    expect(readConnection("claude", { json: "{}" }, true).state).toBe("official");
    expect(readConnection("gemini", { json: "{" }, false).state).toBe("invalid");
    expect(writeConnection("gemini", { json: "{" }, "apiKey", "x")).toEqual({ json: "{" });
  });

  it("writes Claude Code effort settings into env", () => {
    let text = toEditorText("json", { env: { ANTHROPIC_AUTH_TOKEN: "t" } });
    text = writeConnection("claude", text, "reasoning", "xhigh");
    text = writeConnection("claude", text, "alwaysEffort", "1");
    expect(JSON.parse(text.json).env).toEqual({ ANTHROPIC_AUTH_TOKEN: "t", CLAUDE_CODE_EFFORT_LEVEL: "xhigh", CLAUDE_CODE_ALWAYS_ENABLE_EFFORT: "1" });
    expect(ready(readConnection("claude", text, false)).values).toMatchObject({ reasoning: "xhigh", alwaysEffort: "1" });
    text = writeConnection("claude", text, "reasoning", "");
    text = writeConnection("claude", text, "alwaysEffort", "");
    expect(JSON.parse(text.json).env).toEqual({ ANTHROPIC_AUTH_TOKEN: "t" });
  });
});
