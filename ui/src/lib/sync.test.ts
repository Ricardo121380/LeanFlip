import { describe, expect, it } from "vitest";
import type { Provider, ToolId } from "./api";
import type { Preset } from "./presets";
import { linkedCards, LINK_META, planSync, rootUrl, syncSource, withApiKey, withLink, withModel } from "./sync";
import { TOOLS } from "./tools";

const presetFiles = import.meta.glob<{ default: Preset[] }>("../presets/*.json", { eager: true });
const presets = Object.fromEntries(TOOLS.map((tool) => [tool.id, presetFiles[`../presets/${tool.id}.json`]?.default ?? []])) as Record<ToolId, Preset[]>;

const codexRelay: Provider = {
  id: "relay",
  name: "拼车站",
  websiteUrl: "https://relay.example.com",
  settingsConfig: {
    auth: { OPENAI_API_KEY: "sk-relay" },
    config: 'model = "gpt-5"\nmodel_provider = "relay"\n\n[model_providers.relay]\nname = "拼车站"\nbase_url = "https://relay.example.com/v1/"\nwire_api = "responses"\n',
  },
};
const claudeRelay: Provider = {
  id: "kimi",
  name: "Kimi",
  settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://api.moonshot.cn/anthropic", ANTHROPIC_AUTH_TOKEN: "sk-kimi", ANTHROPIC_MODEL: "kimi-k2" } },
};

const byTool = (targets: ReturnType<typeof planSync>) => Object.fromEntries(targets.map((target) => [target.tool, target]));

describe("sync sources", () => {
  it("reads the address, key, model and protocol of third-party cards only", () => {
    expect(rootUrl("https://relay.example.com/v1/")).toBe("https://relay.example.com");
    expect(syncSource("codex", codexRelay)).toMatchObject({ family: "responses", root: "https://relay.example.com", apiKey: "sk-relay", model: "gpt-5" });
    expect(syncSource("claude", claudeRelay)).toMatchObject({ family: "anthropic", root: "https://api.moonshot.cn/anthropic", model: "kimi-k2" });
    expect(syncSource("codex", { ...codexRelay, category: "official" })).toBeNull();
    expect(syncSource("claude", { id: "x", name: "X", settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://x.example" } } })).toBeNull();
  });

  it("skips keys that reference environment variables", () => {
    const opencode = (apiKey: string): Provider => ({ id: "o", name: "O", settingsConfig: { npm: "@ai-sdk/openai-compatible", options: { baseURL: "https://o.example/v1", apiKey }, models: { m: {} } } });
    for (const key of ["{env:RELAY_KEY}", "${RELAY_KEY}", "$RELAY_KEY"]) expect(syncSource("opencode", opencode(key))).toBeNull();
    expect(syncSource("opencode", opencode("sk-real"))).not.toBeNull();
  });

  it("keeps MCode's Responses protocol", () => {
    const mcode: Provider = { id: "m", name: "M", settingsConfig: { api: "openai-responses", options: { baseURL: "https://m.example/v1", apiKey: "sk-m" }, models: { "gpt-5": {} } } };
    const source = syncSource("mcode", mcode);
    expect(source?.family).toBe("responses");
    const openclaw = byTool(planSync(source!, presets, {})).openclaw;
    expect(openclaw.status).toBe("ready");
    expect(openclaw.provider!.settingsConfig).toMatchObject({ api: "openai-responses" });
  });
});

describe("planSync", () => {
  it("builds cards from each tool's preset and marks protocol mismatches for checking", () => {
    const targets = byTool(planSync(syncSource("codex", codexRelay)!, presets, {}));
    expect(Object.keys(targets)).not.toContain("codex");

    const claude = targets.claude;
    expect(claude.status).toBe("check");
    expect(claude.provider!.settingsConfig).toEqual({ env: { ANTHROPIC_BASE_URL: "https://relay.example.com", ANTHROPIC_AUTH_TOKEN: "sk-relay" } });
    expect(claude.provider).toMatchObject({ id: "relay", name: "拼车站", websiteUrl: "https://relay.example.com" });

    const openclaw = targets.openclaw;
    expect(openclaw.status).toBe("ready");
    expect(openclaw.provider!.settingsConfig).toMatchObject({ baseUrl: "https://relay.example.com/v1", api: "openai-responses", apiKey: "sk-relay", models: [{ id: "gpt-5", name: "gpt-5" }] });

    const grok = targets.grokbuild;
    expect(grok.status).toBe("ready");
    const toml = grok.provider!.settingsConfig.config as string;
    expect(toml).toContain('base_url = "https://relay.example.com/v1"');
    expect(toml).toContain('model = "gpt-5"');
    expect(toml).toContain('name = "拼车站"');

    expect(targets.opencode.status).toBe("check");
    expect(targets.opencode.needsModel).toBe(true);
    expect(targets.opencode.provider!.settingsConfig).toMatchObject({ npm: "@ai-sdk/openai-compatible", options: { baseURL: "https://relay.example.com/v1", apiKey: "sk-relay" }, models: {} });
    expect(targets.gemini.status).toBe("check");
  });

  it("keeps Anthropic sources on Anthropic presets and uses each preset's /v1 convention", () => {
    const targets = byTool(planSync(syncSource("claude", claudeRelay)!, presets, {}));
    expect(targets.opencode.status).toBe("ready");
    expect(targets.opencode.provider!.settingsConfig).toMatchObject({ npm: "@ai-sdk/anthropic", options: { baseURL: "https://api.moonshot.cn/anthropic/v1" }, models: { "kimi-k2": {} } });
    expect(targets.pi.status).toBe("ready");
    expect(targets.pi.provider!.settingsConfig).toMatchObject({ api: "anthropic-messages", baseUrl: "https://api.moonshot.cn/anthropic" });
    expect(targets["claude-desktop"].status).toBe("ready");
    expect(targets.codex.status).toBe("check");
    // Grok Build 的后端要求模型；协议不同时模型被清空，必须重新填。
    expect(targets.grokbuild).toMatchObject({ status: "check", needsModel: true });
    expect(targets.codex.needsModel).toBe(false);
    const codexToml = targets.codex.provider!.settingsConfig.config as string;
    expect(codexToml).toContain('base_url = "https://api.moonshot.cn/anthropic/v1"');
    expect(codexToml).not.toContain("your-model-id");
  });

  it("points out cards that already use the same address and key", () => {
    const existing: Provider = { id: "kimi-old", name: "Old", settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://relay.example.com/", ANTHROPIC_AUTH_TOKEN: "sk-relay" } } };
    const taken: Provider = { id: "relay", name: "Other", settingsConfig: { npm: "@ai-sdk/openai-compatible", options: { baseURL: "https://other.example/v1", apiKey: "k" }, models: {} } };
    const targets = byTool(planSync(syncSource("codex", codexRelay)!, presets, { claude: [existing], opencode: [taken] }));
    expect(targets.claude).toMatchObject({ status: "exists", existing: { id: "kimi-old" } });
    expect(targets.claude.provider).toBeUndefined();
    expect(targets.opencode.provider!.id).toBe("relay-2");
  });
});

describe("linked cards", () => {
  it("changes models and keys and finds cards that share a link and the old key", () => {
    const targets = byTool(planSync(syncSource("codex", codexRelay)!, presets, {}));
    const withPick = withModel("opencode", targets.opencode.provider!, "deepseek-chat");
    expect(withPick.settingsConfig.models).toEqual({ "deepseek-chat": {} });

    const source = withLink(codexRelay, "link-1");
    const claude = withLink(targets.claude.provider!, "link-1");
    const stale = withLink(withApiKey("opencode", withPick, "sk-other"), "link-1");
    const unrelated = targets.openclaw.provider!;
    const found = linkedCards("codex", source, "sk-relay", { codex: [source], claude: [claude], opencode: [stale], openclaw: [unrelated] });
    expect(found.map((item) => item.tool)).toEqual(["claude"]);
    expect(source.meta?.[LINK_META]).toBe("link-1");

    const rotated = withApiKey("codex", source, "sk-new");
    expect((rotated.settingsConfig.auth as Record<string, string>).OPENAI_API_KEY).toBe("sk-new");
    expect(rotated.settingsConfig.config).toBe(codexRelay.settingsConfig.config);
  });
});
