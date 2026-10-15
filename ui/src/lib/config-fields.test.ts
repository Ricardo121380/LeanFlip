import { expect, it } from "vitest";
import { applyOption, atPath, commonOptionsFor, setPath } from "./config-options";
import { commonFieldsFor, configFieldsIssue, fieldInput, fieldPath, providerFieldsFor } from "./config-fields";

it("updates literal model identifiers and array rows without replacing siblings", () => {
  const original = { models: { "vendor/model.v1": { limit: { context: 1000 }, reasoning: true }, other: {} } };
  const field = providerFieldsFor("mcode", original)[1];
  const changed = setPath(original, fieldPath(field), 512);
  expect(atPath(changed, ["models", "vendor/model.v1", "limit", "output"])).toBe(512);
  expect(changed.models).toEqual({ ...original.models, "vendor/model.v1": { limit: { context: 1000, output: 512 }, reasoning: true } });
  expect(setPath(changed, fieldPath(field), undefined)).toEqual(original);
  const array = { models: [{ id: "a.b", contextWindow: 2000 }, { id: "c", extra: true }] };
  expect(setPath(array, ["models", 0, "contextWindow"], 4000)).toEqual({ models: [{ id: "a.b", contextWindow: 4000 }, array.models[1]] });
  expect(() => setPath({}, ["models", "__proto__", "x"], true)).toThrow();
});

it("validates ratio units and integer budgets without parseInt truncation", () => {
  const field = commonFieldsFor("gemini", {}).find((f) => f.key === "model.compressionThreshold")!;
  expect(fieldInput(field, "0.625")).toBe(0.625);
  for (const value of [0, 65, "0.5oops", "0.5"]) expect(configFieldsIssue("gemini", "common", { model: { compressionThreshold: value } })).not.toBeNull();
  expect(configFieldsIssue("gemini", "common", { model: { compressionThreshold: 0.001, maxSessionTurns: -1 } })).toBeNull();
  expect(configFieldsIssue("pi", "common", { retry: { maxRetries: 1.5 } })).not.toBeNull();
  expect(configFieldsIssue("pi", "common", { compaction: { reserveTokens: 0 } })).toBeNull();
});

it("enforces Claude compact bounds without imposing the unknown-model output cap on all models", () => {
  for (const value of ["99999", "1000001"]) expect(configFieldsIssue("claude", "provider", { env: { CLAUDE_CODE_AUTO_COMPACT_WINDOW: value } })).not.toBeNull();
  for (const value of ["100000", "1000000"]) expect(configFieldsIssue("claude", "provider", { env: { CLAUDE_CODE_AUTO_COMPACT_WINDOW: value, CLAUDE_CODE_MAX_OUTPUT_TOKENS: "256000" } })).toBeNull();
});

it("uses notify for both OpenCode generations and keeps scalar or custom permissions intact", () => {
  const option = commonOptionsFor("opencode", {}).find((o) => o.key === "autoupdate")!;
  expect(applyOption({ permission: "allow" }, option, true)).toEqual({ permission: "allow", autoupdate: "notify" });
  expect(commonFieldsFor("opencode", { permission: "allow" }).map((f) => f.key)).toContain("permission");
  expect(commonFieldsFor("opencode", { permission: "allow" }).map((f) => f.key)).not.toContain("permission.bash");
  expect(commonFieldsFor("opencode", {}, true).map((f) => f.key)).toEqual(["update", "compaction.buffer", "compaction.keep.tokens"]);
  expect(commonOptionsFor("opencode", {}, true).map((f) => f.key)).not.toContain("compaction.prune");
});

it("preserves unedited legacy values and nullable Hermes caps while validating new edits", () => {
  const before = { compression: { threshold_tokens: "legacy-auto", threshold: 0.5 } };
  expect(configFieldsIssue("hermes", "common", { ...before, memory: { memory_enabled: true } }, before)).toBeNull();
  expect(configFieldsIssue("hermes", "common", { compression: { threshold_tokens: "bad" } }, before)).not.toBeNull();
  expect(configFieldsIssue("hermes", "common", { compression: { threshold_tokens: null } })).toBeNull();
});
