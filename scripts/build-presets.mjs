// LeanFlip's own starter templates. No external repository or copied catalogue.
// Format references: docs/engineering/independent-core.md.
import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
const destination = new URL("../ui/src/presets/", import.meta.url);
const gateway = "https://api.example.com/v1";
const model = "your-model-id";
const template = (name, settingsConfig, extra = {}) => ({ name, settingsConfig, ...extra });
const official = (name, settingsConfig, websiteUrl) => template(name, settingsConfig, { isOfficial: true, category: "official", websiteUrl });
const codex = (endpoint) => ({ auth: { OPENAI_API_KEY: "" }, config: `model = "${model}"\nmodel_provider = "leanflip"\n\n[model_providers.leanflip]\nname = "My connection"\nbase_url = "${endpoint}"\nwire_api = "responses"\n` });
const grok = (endpoint) => ({ config: `# Set model and context_window to values supported by your endpoint.\n[models]\ndefault = "connection"\n\n[model.connection]\nname = "My connection"\nmodel = "${model}"\nbase_url = "${endpoint}"\napi_backend = "responses"\napi_key = ""\ncontext_window = 32768\n` });
const namedModel = { id: model, name: "My model" };
const templates = {
  claude: [
    official("Claude Official", { env: {} }, "https://claude.ai"),
    template("Anthropic API", { env: { ANTHROPIC_API_KEY: "" } }, { websiteUrl: "https://platform.claude.com", apiKeyField: "ANTHROPIC_API_KEY" }),
    template("Anthropic Gateway", { env: { ANTHROPIC_BASE_URL: "https://api.example.com", ANTHROPIC_AUTH_TOKEN: "", ANTHROPIC_MODEL: model } }),
    template("AWS Bedrock", { env: { CLAUDE_CODE_USE_BEDROCK: "1", AWS_REGION: "us-east-1" } }, { websiteUrl: "https://aws.amazon.com/bedrock/" }),
  ],
  codex: [
    official("Codex Official", { auth: {}, config: "" }, "https://chatgpt.com"),
    template("OpenAI Responses API", codex("https://api.openai.com/v1"), { websiteUrl: "https://platform.openai.com" }),
    template("Responses Gateway", codex(gateway)),
    template("Ollama Local", { auth: {}, config: `model_provider = "ollama"\nmodel = "${model}"\n` }),
    template("LM Studio Local", { auth: {}, config: `model_provider = "lmstudio"\nmodel = "${model}"\n` }),
  ],
  gemini: [
    official("Google Official", { env: {} }, "https://gemini.google.com"),
    template("Google Gemini API", { env: { GEMINI_API_KEY: "" } }, { websiteUrl: "https://aistudio.google.com", apiKeyField: "GEMINI_API_KEY" }),
    template("Gemini Gateway", { env: { GOOGLE_GEMINI_BASE_URL: "https://api.example.com", GEMINI_API_KEY: "", GEMINI_MODEL: model } }),
  ],
  grokbuild: [official("Grok Official", { config: "" }, "https://x.ai"), template("xAI Responses API", grok("https://api.x.ai/v1"), { websiteUrl: "https://console.x.ai" }), template("Responses Gateway", grok(gateway))],
  "claude-desktop": [official("Claude Desktop Official", { env: {} }, "https://claude.ai/download"), template("Claude Desktop Gateway", { env: { ANTHROPIC_BASE_URL: "https://api.example.com", ANTHROPIC_AUTH_TOKEN: "" } })],
  opencode: [template("OpenAI Compatible", { npm: "@ai-sdk/openai-compatible", options: { baseURL: gateway, apiKey: "" }, models: { [model]: { name: "My model" } } }), template("Anthropic Compatible", { npm: "@ai-sdk/anthropic", options: { baseURL: "https://api.example.com/v1", apiKey: "" }, models: { [model]: { name: "My model" } } })],
  openclaw: [template("OpenAI Compatible", { baseUrl: gateway, api: "openai-completions", apiKey: "", models: [namedModel] }), template("Responses Compatible", { baseUrl: gateway, api: "openai-responses", apiKey: "", models: [namedModel] })],
  hermes: [template("OpenAI Compatible", { base_url: gateway, api_key: "", models: [namedModel] })],
  pi: [template("OpenAI Compatible", { baseUrl: gateway, api: "openai-completions", apiKey: "", models: [namedModel] }), template("Anthropic Compatible", { baseUrl: "https://api.example.com", api: "anthropic-messages", apiKey: "", models: [namedModel] })],
  mcode: [template("OpenAI Compatible", { api: "openai-completions", options: { baseURL: gateway, apiKey: "" }, models: { [model]: {} } }), template("Anthropic Compatible", { api: "anthropic-messages", options: { baseURL: "https://api.example.com", apiKey: "" }, models: { [model]: {} } })],
};
await mkdir(destination, { recursive: true });
for (const [tool, rows] of Object.entries(templates)) {
  await writeFile(new URL(`${tool}.json`, destination), `${JSON.stringify(rows, null, 2)}\n`);
}
console.log(`Generated ${Object.values(templates).flat().length} LeanFlip templates in ${fileURLToPath(destination)}`);
