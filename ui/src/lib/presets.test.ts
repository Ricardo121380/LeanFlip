import { beforeAll, describe, expect, it } from "vitest";
import { parse } from "smol-toml";
import { TOOLS } from "./tools";
import { filterPresets, loadPresets, type Preset } from "./presets";
import { slugify } from "./cards";
import officialVendors from "./official-vendor-domains.json";

const all = new Map<string, Preset[]>();
const presetsFor = (tool: string) => all.get(tool) ?? [];
const searchPresets = (tool: string, q: string) => filterPresets(presetsFor(tool), q);
beforeAll(async () => {
  for (const tool of TOOLS) all.set(tool.id, await loadPresets(tool.id));
});

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

  it("custom Grok presets use Grok's own [models] / [model.<name>] shape", () => {
    for (const p of presetsFor("grokbuild").filter((p) => !p.isOfficial && p.category !== "official")) {
      const doc = parse(p.settingsConfig.config as string) as { models?: { default?: string }; model?: Record<string, Record<string, unknown>> };
      const name = doc.models?.default ?? "";
      const table = doc.model?.[name];
      expect(table, p.name).toBeDefined();
      expect(table!.base_url, p.name).toMatch(/^https:\/\//);
      expect(table!.api_backend).toBe("responses");
      expect(Number(table!.context_window)).toBeGreaterThan(0);
    }
  });

  it("loads lazily and caches per tool", async () => {
    const a = loadPresets("claude");
    expect(loadPresets("claude")).toBe(a);
    expect((await a).length).toBeGreaterThan(0);
  });

  it("official presets come first-class and searchable", () => {
    expect(presetsFor("codex").some((p) => p.isOfficial)).toBe(true);
    expect(presetsFor("claude").some((p) => p.isOfficial)).toBe(true);
    expect(searchPresets("claude", "kimi").every((p) => p.name.toLowerCase().includes("kimi") || (p.websiteUrl ?? "").includes("kimi"))).toBe(true);
    expect(slugify(presetsFor("opencode")[0].name)).toMatch(/^[a-z0-9-]+$/);
  });
});

describe("preset links", () => {
  const SHORT_CODE = /^\/(?=[A-Za-z0-9]*\d)(?=[A-Za-z0-9]*[A-Z])[A-Za-z0-9]{5,10}\/?$/;
  const RELAYS = ["packyapi.ai", "aigocode.app", "zetaapi.ai", "siliconflow.cn", "ppio.com", "s.qiniu.com", "openrouter.ai", "aihubmix.com", "apinebula.ai"];
  const officialDomains = (officialVendors as { vendors: Record<string, string[]> }).vendors;
  const vendorOf = (host: string) =>
    Object.entries(officialDomains).find(([, domains]) => domains.some((d) => host === d || host.endsWith(`.${d}`)))?.[0];

  it("only point at allowlisted official model-vendor domains, without referral or tracking parts", () => {
    const tracking = /[?&](aff|track_id|ref|invite\w*|ic|code|source|from|ch|ytag|ac|rc|utm_\w+)=/i;
    let count = 0;
    for (const tool of TOOLS) {
      for (const p of presetsFor(tool.id)) {
        for (const url of [p.websiteUrl, p.apiKeyUrl]) {
          if (!url) continue;
          count++;
          const label = `${tool.id}/${p.name}: ${url}`;
          const parsed = new URL(url);
          expect(parsed.protocol, label).toBe("https:");
          expect(vendorOf(parsed.host), label).toBeDefined();
          expect(RELAYS.some((r) => parsed.host === r || parsed.host.endsWith(`.${r}`)), label).toBe(false);
          expect(url, label).not.toMatch(tracking);
          expect(parsed.pathname, label).not.toMatch(/^\/(i|invite|r|ref|go|aff|s)\/|cc-?switch|\/register\/[A-Za-z0-9_-]{6,}/i);
          expect(parsed.pathname, label).not.toMatch(SHORT_CODE);
        }
      }
    }
    expect(count).toBeGreaterThan(100);
  });

  it("keep official vendor links (Kimi, DeepSeek, GLM, Qwen, Volcengine, MiniMax …)", () => {
    const has = (tool: string, name: string) => Boolean(presetsFor(tool).find((p) => p.name === name)?.websiteUrl);
    for (const name of ["Kimi", "DeepSeek", "Zhipu GLM", "MiniMax", "Volcengine Doubao"]) expect(has("claude", name), name).toBe(true);
    for (const name of ["OpenRouter", "PackyCode", "AIGoCode", "SiliconFlow", "PPIO", "Qiniu"]) {
      const p = presetsFor("claude").find((x) => x.name === name);
      if (p) expect([p.websiteUrl, p.apiKeyUrl], name).toEqual([undefined, undefined]);
    }
  });

  it("drop every link field listed in the removal manifest (partner / promo presets)", async () => {
    const removed = (await import("./removed-preset-links.json")).default as { tool: string; name: string; field: "websiteUrl" | "apiKeyUrl" }[];
    expect(removed.length).toBeGreaterThan(0);
    for (const r of removed) {
      const preset = presetsFor(r.tool).find((p) => p.name === r.name);
      expect(preset, `${r.tool}/${r.name}`).toBeDefined();
      expect(preset![r.field], `${r.tool}/${r.name}.${r.field}`).toBeUndefined();
    }
  });
});
