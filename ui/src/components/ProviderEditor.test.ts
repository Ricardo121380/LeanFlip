import { describe, expect, it } from "vitest";
import { applyLoginSource, customTemplate, loginSourceOf } from "./ProviderEditor";
import { TOOLS } from "@/lib/tools";
import type { Provider } from "@/lib/api";

const base: Provider = { id: "official", name: "OpenAI", category: "official", settingsConfig: { auth: {}, config: "" } };

describe("Codex login source", () => {
  it("round-trips managed / local-ref / native", () => {
    expect(loginSourceOf(base)).toBe("native");
    const managed = applyLoginSource(base, "managed:acc-1");
    expect(managed.meta?.authBinding).toEqual({ authProvider: "codex_oauth", accountId: "acc-1" });
    expect(loginSourceOf(managed)).toBe("managed:acc-1");
    const def = applyLoginSource(base, "managed:");
    expect(def.meta?.authBinding).toEqual({ authProvider: "codex_oauth" });
    expect(loginSourceOf(def)).toBe("managed:");
    expect(loginSourceOf({ ...base, meta: { codexLoginRef: "sub:u|account:w" } })).toBe("ref:sub:u|account:w");
    const native = applyLoginSource({ ...managed, meta: { ...managed.meta, codexLoginRef: "x", other: 1 } }, "native");
    expect(native.meta).toEqual({ other: 1 });
    expect(applyLoginSource(managed, "native").meta).toBeUndefined();
  });

  it("has a custom template for every tool", () => {
    for (const tool of TOOLS) expect(typeof customTemplate(tool.id)).toBe("object");
  });
});
