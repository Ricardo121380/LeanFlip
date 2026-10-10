import { describe, expect, it } from "vitest";
import { parse } from "smol-toml";
import { TOOLS } from "./tools";
import { presetsFor, searchPresets } from "./presets";
import { slugify } from "./cards";

describe("trimmed cc-switch presets", () => {
  it("exist for every tool and are well-formed", () => {
    for (const tool of TOOLS) {
      const list = presetsFor(tool.id);
      expect(list.length, tool.id).toBeGreaterThan(0);
      for (const p of list) {
        expect(typeof p.name).toBe("string");
        expect(p.settingsConfig && typeof p.settingsConfig === "object", `${tool.id}/${p.name}`).toBe(true);
        const raw = p as unknown as Record<string, unknown>;
        for (const dropped of ["providerType", "requiresOAuth", "hidden", "apiFormat", "partnerPromotionKey", "theme"]) {
          expect(raw[dropped], `${tool.id}/${p.name}.${dropped}`).toBeUndefined();
        }
      }
      expect(new Set(list.map((p) => p.name)).size).toBe(list.length);
    }
  });

  it("Codex and Grok presets carry valid TOML", () => {
    for (const tool of ["codex", "grokbuild"] as const) {
      for (const p of presetsFor(tool)) {
        const config = p.settingsConfig.config as string;
        expect(typeof config).toBe("string");
        if (config.trim()) expect(() => parse(config), `${tool}/${p.name}`).not.toThrow();
      }
    }
  });

  it("official presets come first-class and searchable", () => {
    expect(presetsFor("codex").some((p) => p.isOfficial)).toBe(true);
    expect(presetsFor("claude").some((p) => p.isOfficial)).toBe(true);
    expect(searchPresets("claude", "kimi").every((p) => p.name.toLowerCase().includes("kimi") || (p.websiteUrl ?? "").includes("kimi"))).toBe(true);
    expect(slugify(presetsFor("opencode")[0].name)).toMatch(/^[a-z0-9-]+$/);
  });
});

describe("preset links", () => {
  it("carry no referral or tracking parameters", () => {
    const tracking = /[?&](aff|track_id|ref|invite\w*|ic|code|source|from|ch|ytag|ac|rc|utm_\w+)=/i;
    for (const tool of TOOLS) {
      for (const p of presetsFor(tool.id)) {
        for (const url of [p.websiteUrl, p.apiKeyUrl]) {
          if (url) expect(url, `${tool.id}/${p.name}`).not.toMatch(tracking);
        }
      }
    }
  });
});
