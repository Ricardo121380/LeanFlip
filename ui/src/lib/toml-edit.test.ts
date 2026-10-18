import { describe, expect, it } from "vitest";
import { getTomlValue, readToml, setTomlValue } from "./toml-edit";

const CODEX = `# my gateway
model_provider = "custom"
model = "gpt-5.5"

[model_providers.custom]
name = "custom"
base_url = "https://old.example.com/v1" # keep me
wire_api = "responses"

[[profiles]]
name = "x"
`;

describe("toml-edit", () => {
  it("reads values from top level and nested tables", () => {
    expect(getTomlValue(CODEX, null, "model")).toBe("gpt-5.5");
    expect(getTomlValue(CODEX, ["model_providers", "custom"], "base_url")).toBe("https://old.example.com/v1");
    expect(getTomlValue("not = = toml", null, "model")).toBeUndefined();
  });

  it("replaces one line and keeps comments, order and other tables", () => {
    const next = setTomlValue(CODEX, ["model_providers", "custom"], "base_url", "https://new.example.com/v1");
    expect(next).toBe(CODEX.replace('base_url = "https://old.example.com/v1" # keep me', 'base_url = "https://new.example.com/v1"'));
    expect(readToml(next)).not.toBeNull();
  });

  it("adds a top-level key before the first table", () => {
    const next = setTomlValue(CODEX, null, "model_reasoning_effort", "high");
    expect(next.split("\n").slice(0, 4)).toEqual(["# my gateway", 'model_provider = "custom"', 'model = "gpt-5.5"', 'model_reasoning_effort = "high"']);
    expect(getTomlValue(next, ["model_providers", "custom"], "name")).toBe("custom");
  });

  it("removes a key when the value is cleared", () => {
    const next = setTomlValue(CODEX, null, "model", "");
    expect(next).not.toContain('model = "gpt-5.5"');
    expect(getTomlValue(next, null, "model_provider")).toBe("custom");
  });

  it("finds quoted table names and creates missing tables", () => {
    const grok = '[models]\ndefault = "grok-4.5"\n\n[model."grok-4.5"]\nmodel = "grok-4.5"\ncontext_window = 500000\n';
    const updated = setTomlValue(grok, ["model", "grok-4.5"], "context_window", 32768);
    expect(getTomlValue(updated, ["model", "grok-4.5"], "context_window")).toBe(32768);
    const created = setTomlValue("", ["model_providers", "custom"], "base_url", "https://api.example.com");
    expect(created).toBe('[model_providers.custom]\nbase_url = "https://api.example.com"\n');
  });

  it("escapes strings", () => {
    const next = setTomlValue("", null, "model", 'a "quoted" \\ name');
    expect(getTomlValue(next, null, "model")).toBe('a "quoted" \\ name');
  });
});

it.each(['"""', "'''"])("ignores keys and headers inside %s strings", (quote) => {
  const instructions = `Example:\nmodel = "example"\n[model_providers.custom]\nbase_url = "https://example.invalid"\n`;
  const source = `developer_instructions = ${quote}\n${instructions}${quote}\nmodel = "real"\n[model_providers.custom]\nbase_url = "https://old.invalid"\n`;
  const model = setTomlValue(source, null, "model", "new");
  expect(getTomlValue(model, null, "model")).toBe("new");
  expect(getTomlValue(model, null, "developer_instructions")).toBe(instructions);
  const url = setTomlValue(model, ["model_providers", "custom"], "base_url", "https://new.invalid");
  expect(getTomlValue(url, ["model_providers", "custom"], "base_url")).toBe("https://new.invalid");
  expect(getTomlValue(url, null, "developer_instructions")).toBe(instructions);
});

it("inserts after complete multiline values and removes an entire multiline assignment", () => {
  const source = `developer_instructions = """\nmodel = "example"\n"""\nmodels = [\n  "one",\n  "two",\n]\n`;
  const next = setTomlValue(source, null, "model", "actual");
  expect(getTomlValue(next, null, "model")).toBe("actual");
  expect(getTomlValue(next, null, "models")).toEqual(["one", "two"]);
  const removed = setTomlValue(next, null, "developer_instructions", undefined);
  expect(readToml(removed)).toMatchObject({ model: "actual", models: ["one", "two"] });
  expect(removed).not.toContain("example");
});

it("does not interpret escaped quotes and comments as multiline delimiters", () => {
  const source = '# """ comment\nlabel = "escaped \\" quote"\nmodel = "real"\n';
  const next = setTomlValue(source, null, "model", "new");
  expect(getTomlValue(next, null, "model")).toBe("new");
  expect(getTomlValue(next, null, "label")).toBe('escaped " quote');
});
