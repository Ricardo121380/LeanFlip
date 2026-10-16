// 同步到其他 Agent：把一张第三方卡的地址、Key 和模型，按目标工具自己的预设生成新卡。
// 难点是协议不同，不是配置格式：同一个中转站在 Codex 里走 Responses，在 Claude Code 里走
// Anthropic Messages。目标支持来源的协议时「直接可用」并带上模型；不支持时「需要检测」，
// 中转站得同时支持另一种协议，模型也要重新选。只生成卡片，不自动切换。
import type { Provider, ToolId } from "./api";
import { isOfficial, slugify, toEditorText, uniqueId, fromEditorText, type EditorText } from "./cards";
import { readConnection, writeConnection } from "./connection";
import type { Preset } from "./presets";
import { getTomlValue, setTomlValue } from "./toml-edit";
import { TOOLS, toolMeta } from "./tools";

/** 请求协议。 */
export type Family = "anthropic" | "responses" | "chat" | "gemini";

/** 卡片元数据里的关联 ID：同一个地址和 Key 同步出来的卡共用一个。 */
export const LINK_META = "leanflipLink";

/** 每个工具支持的协议和对应的预设（按优先顺序）。 */
const TARGETS: Record<ToolId, [Family, string][]> = {
  claude: [["anthropic", "Anthropic Gateway"]],
  "claude-desktop": [["anthropic", "Claude Desktop Gateway"]],
  codex: [["responses", "Responses Gateway"]],
  grokbuild: [["responses", "Responses Gateway"]],
  gemini: [["gemini", "Gemini Gateway"]],
  opencode: [["chat", "OpenAI Compatible"], ["anthropic", "Anthropic Compatible"]],
  mcode: [["chat", "OpenAI Compatible"], ["anthropic", "Anthropic Compatible"]],
  pi: [["chat", "OpenAI Compatible"], ["anthropic", "Anthropic Compatible"]],
  openclaw: [["chat", "OpenAI Compatible"], ["responses", "Responses Compatible"]],
  hermes: [["chat", "OpenAI Compatible"]],
};

type Json = Record<string, unknown>;
const text = (value: unknown) => (typeof value === "string" ? value : "");

/** 这张卡走哪种协议。 */
export function familyOf(tool: ToolId, settings: Json): Family {
  switch (tool) {
    case "claude":
    case "claude-desktop":
      return "anthropic";
    case "gemini":
      return "gemini";
    case "codex":
    case "grokbuild": {
      const toml = text(settings.config);
      if (tool === "grokbuild") {
        const name = text(getTomlValue(toml, ["models"], "default"));
        return text(getTomlValue(toml, ["model", name], "api_backend")) === "chat" ? "chat" : "responses";
      }
      const provider = text(getTomlValue(toml, null, "model_provider"));
      return text(getTomlValue(toml, ["model_providers", provider], "wire_api")) === "chat" ? "chat" : "responses";
    }
    case "opencode":
      return text(settings.npm) === "@ai-sdk/anthropic" ? "anthropic" : "chat";
    case "openclaw":
      return text(settings.api) === "openai-responses" ? "responses" : "chat";
    default:
      return text(settings.api) === "anthropic-messages" ? "anthropic" : "chat";
  }
}

/** 站点根地址：去掉结尾的 `/` 和 `/v1`，再按目标预设的习惯补回。 */
export function rootUrl(url: string): string {
  return url.trim().replace(/\/+$/, "").replace(/\/v1$/i, "");
}

export interface SyncSource {
  tool: ToolId;
  provider: Provider;
  family: Family;
  root: string;
  apiKey: string;
  model: string;
}

/** 能同步的卡：第三方、有地址也有 Key。官方登录、内置供应商、读不出的配置都不行。 */
export function syncSource(tool: ToolId, provider: Provider): SyncSource | null {
  if (isOfficial(provider)) return null;
  const meta = toolMeta(tool);
  const connection = readConnection(tool, toEditorText(meta.editor, provider.settingsConfig), false);
  if (connection.state !== "ready" || connection.blocked) return null;
  const { baseUrl, apiKey, model, models } = connection.values;
  const url = typeof baseUrl === "string" ? baseUrl : "";
  const key = typeof apiKey === "string" ? apiKey : "";
  if (!/^https?:\/\//i.test(url) || !key) return null;
  const first = typeof model === "string" && model ? model : Array.isArray(models) ? (models[0] ?? "") : "";
  return { tool, provider, family: familyOf(tool, provider.settingsConfig), root: rootUrl(url), apiKey: key, model: first };
}

export type SyncStatus = "ready" | "check" | "exists" | "unavailable";

export interface SyncTarget {
  tool: ToolId;
  status: SyncStatus;
  family: Family;
  /** 生成的卡（`exists` / `unavailable` 时没有）。 */
  provider?: Provider;
  /** 已经有同样地址和 Key 的卡。 */
  existing?: Provider;
  /** 这个工具的卡必须带模型（模型写在列表里）。 */
  needsModel: boolean;
}

/** 两张卡是不是同一个地址和 Key。 */
function sameEndpoint(tool: ToolId, provider: Provider, source: SyncSource): boolean {
  const other = syncSource(tool, provider);
  return Boolean(other && other.root === source.root && other.apiKey === source.apiKey);
}

/** 目标工具按哪种协议、用哪个预设。 */
function pick(tool: ToolId, family: Family): { family: Family; preset: string; compatible: boolean } {
  const options = TARGETS[tool];
  const match = options.find(([candidate]) => candidate === family);
  const [chosen, preset] = match ?? options[0];
  return { family: chosen, preset, compatible: Boolean(match) };
}

/** 在一份配置文本上写模型（写进模型列表的工具会替换整个列表）。 */
function writeModel(tool: ToolId, editor: EditorText, model: string): EditorText {
  const connection = readConnection(tool, editor, false);
  if (connection.state !== "ready") return editor;
  if (connection.fields.some((field) => field.id === "models")) return writeConnection(tool, editor, "models", model ? [model] : []);
  if (connection.fields.some((field) => field.id === "model")) return writeConnection(tool, editor, "model", model);
  return editor;
}

/** 按目标的预设生成一张卡，地址、Key、模型和名字来自来源。 */
function build(tool: ToolId, preset: Preset, source: SyncSource, model: string, id: string): Provider | null {
  const meta = toolMeta(tool);
  let editor = toEditorText(meta.editor, structuredClone(preset.settingsConfig));
  const template = readConnection(tool, editor, false);
  if (template.state !== "ready") return null;
  // 预设的示例地址带 /v1 的，生成的也带。
  const example = typeof template.values.baseUrl === "string" ? template.values.baseUrl : "";
  const url = /\/v1\/?$/i.test(example) ? `${source.root}/v1` : source.root;
  editor = writeConnection(tool, editor, "baseUrl", url);
  editor = writeConnection(tool, editor, "apiKey", source.apiKey);
  editor = writeModel(tool, editor, model);
  // Codex / Grok 预设里的连接名叫「My connection」，换成卡片名。
  if (tool === "codex") editor = { ...editor, toml: setTomlValue(editor.toml ?? "", ["model_providers", "leanflip"], "name", source.provider.name) };
  if (tool === "grokbuild") editor = { ...editor, toml: setTomlValue(editor.toml ?? "", ["model", "connection"], "name", source.provider.name) };
  const parsed = fromEditorText(meta.editor, editor);
  if (!parsed.ok) return null;
  const provider: Provider = { id, name: source.provider.name, settingsConfig: parsed.value, createdAt: Date.now() };
  if (source.provider.websiteUrl) provider.websiteUrl = source.provider.websiteUrl;
  return provider;
}

/** 把卡的模型换成 `model`（同步对话框里改模型时用）。 */
export function withModel(tool: ToolId, provider: Provider, model: string): Provider {
  const meta = toolMeta(tool);
  const parsed = fromEditorText(meta.editor, writeModel(tool, toEditorText(meta.editor, provider.settingsConfig), model.trim()));
  return parsed.ok ? { ...provider, settingsConfig: parsed.value } : provider;
}

/** 这张卡现在的模型（列表里的第一个）。 */
export function modelOfCard(tool: ToolId, provider: Provider): string {
  const connection = readConnection(tool, toEditorText(toolMeta(tool).editor, provider.settingsConfig), false);
  if (connection.state !== "ready") return "";
  const { model, models } = connection.values;
  return typeof model === "string" ? model : Array.isArray(models) ? (models[0] ?? "") : "";
}

/** 算出其他每个工具能不能同步、生成什么卡。 */
export function planSync(
  source: SyncSource,
  presets: Partial<Record<ToolId, Preset[]>>,
  providers: Partial<Record<ToolId, Provider[]>>,
): SyncTarget[] {
  return TOOLS.filter((tool) => tool.id !== source.tool).map(({ id: tool }) => {
    const cards = providers[tool] ?? [];
    const { family, preset: presetName, compatible } = pick(tool, source.family);
    const needsModel = toolMeta(tool).kind === "coexist";
    const existing = cards.find((card) => sameEndpoint(tool, card, source));
    if (existing) return { tool, status: "exists", family, existing, needsModel };
    const preset = presets[tool]?.find((candidate) => candidate.name === presetName);
    const id = uniqueId(source.provider.id || slugify(source.provider.name), cards.map((card) => card.id));
    const provider = preset ? build(tool, preset, source, compatible ? source.model : "", id) : null;
    if (!provider) return { tool, status: "unavailable", family, needsModel };
    return { tool, status: compatible && source.family !== "gemini" ? "ready" : "check", family, provider, needsModel };
  });
}

/** 新的关联 ID。 */
export function newLink(): string {
  const bytes = new Uint8Array(6);
  crypto.getRandomValues(bytes);
  return `link-${[...bytes].map((b) => b.toString(16).padStart(2, "0")).join("")}`;
}

export function linkOf(provider: Provider): string | undefined {
  const value = provider.meta?.[LINK_META];
  return typeof value === "string" && value ? value : undefined;
}

export function withLink(provider: Provider, link: string): Provider {
  return { ...provider, meta: { ...(provider.meta ?? {}), [LINK_META]: link } };
}

/** 和这张卡关联、而且还在用同一个旧 Key 的其他卡（改 Key 时提示一起改）。 */
export function linkedCards(
  tool: ToolId,
  provider: Provider,
  oldKey: string,
  providers: Partial<Record<ToolId, Provider[]>>,
): { tool: ToolId; provider: Provider }[] {
  const link = linkOf(provider);
  if (!link || !oldKey) return [];
  return TOOLS.flatMap(({ id }) =>
    (providers[id] ?? [])
      .filter((card) => !(id === tool && card.id === provider.id) && linkOf(card) === link && syncSource(id, card)?.apiKey === oldKey)
      .map((card) => ({ tool: id, provider: card })),
  );
}

/** 把一张卡的 Key 换掉。 */
export function withApiKey(tool: ToolId, provider: Provider, apiKey: string): Provider {
  const meta = toolMeta(tool);
  const parsed = fromEditorText(meta.editor, writeConnection(tool, toEditorText(meta.editor, provider.settingsConfig), "apiKey", apiKey), provider.settingsConfig);
  return parsed.ok ? { ...provider, settingsConfig: parsed.value } : provider;
}
