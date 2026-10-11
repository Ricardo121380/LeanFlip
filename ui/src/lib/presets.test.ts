import { describe, expect, it } from "vitest";
import { parse } from "smol-toml";
import { TOOLS } from "./tools";
import { loadPresets, filterPresets } from "./presets";

describe("LeanFlip connection templates", () => {
  it("offers editable templates for every supported tool", async () => {
    for (const tool of TOOLS) {
      const list = await loadPresets(tool.id);
      expect(list.length, tool.id).toBeGreaterThan(0);
      expect(new Set(list.map(p => p.name)).size).toBe(list.length);
      for (const item of list) {
        expect(item.settingsConfig).toBeTypeOf("object");
        if (tool.id === "codex" || tool.id === "grokbuild") expect(() => parse(item.settingsConfig.config as string)).not.toThrow();
        for (const address of [item.websiteUrl, item.apiKeyUrl]) {
          if (!address) continue;
          const url = new URL(address);
          expect(url.protocol).toBe("https:");
          expect(url.search).toBe("");
        }
      }
    }
  });
  it("keeps official sign-in available and reuses lazy imports", async () => {
    for (const tool of ["claude", "codex", "gemini", "grokbuild", "claude-desktop"] as const) {
      const request = loadPresets(tool);
      expect(loadPresets(tool)).toBe(request);
      const list = await request;
      expect(list.some(p => p.isOfficial)).toBe(true);
      expect(filterPresets(list, "  OFFICIAL ").every(p => p.isOfficial)).toBe(true);
    }
  });
});
