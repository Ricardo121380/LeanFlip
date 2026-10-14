import { expect, it } from "vitest";
import { applyOption, commonOptions, optionState } from "./config-options";

it("distinguishes absent, explicit and custom values without changing them", () => {
  const option = commonOptions.codex![2];
  expect(optionState({}, option)).toBe("default");
  expect(optionState({ tui: { notifications: false } }, option)).toBe("off");
  expect(optionState({ tui: { notifications: true } }, option)).toBe("on");
  expect(optionState({ tui: { notifications: ["approval-requested"] } }, option)).toBe("custom");
});

it("treats partial attribution settings as custom and resets both fields while preserving siblings", () => {
  const option = commonOptions.claude![0];
  const config = { attribution: { pr: "", other: true }, permissions: { allow: ["Read"] } };
  expect(optionState(config, option)).toBe("custom");
  expect(optionState(applyOption(config, option, true), option)).toBe("on");
  expect(applyOption(config, option, "default")).toEqual({ attribution: { other: true }, permissions: { allow: ["Read"] } });
  expect(config.attribution.pr).toBe("");
});
