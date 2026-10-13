// 编辑器表单 ↔ 配置文本的映射。配置文本始终是唯一的数据来源：表单从文本读出，改动再写回文本。
import type { ToolId } from "./api";
import type { EditorText } from "./cards";
import { getTomlValue, readToml, setTomlValue } from "./toml-edit";

export type FieldId =
  | "baseUrl"
  | "apiKey"
  | "authField"
  | "model"
  | "haikuModel"
  | "sonnetModel"
  | "opusModel"
  | "reasoning"
  | "alwaysEffort"
  | "api"
  | "contextWindow"
  | "models";

export interface FieldSpec {
  id: FieldId;
  kind: "url" | "secret" | "text" | "select" | "number" | "list" | "effort" | "toggle";
  group: "connection" | "models";
  options?: string[];
  /** 可自由填写，只给出常见值作为补全。 */
  suggestions?: string[];
  /** effort：客户端接受的推理强度档位，从低到高。 */
  levels?: string[];
  /** 字段对应的配置键，用作输入框提示。 */
  hint?: string;
}

export type FieldValue = string | string[];

export type Connection =
  | { state: "none" }
  | { state: "official" }
  | { state: "invalid" }
  | {
      state: "ready";
      fields: FieldSpec[];
      values: Partial<Record<FieldId, FieldValue>>;
      /** 这些字段由别处决定（内置供应商 / 云厂商认证 / 缺少默认模型），表单里只读。 */
      blocked?: { reason: "builtIn" | "advancedAuth" | "grokDefault"; id?: string; fields: FieldId[] };
    };

export const AUTH_FIELDS = ["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"] as const;
/** 推理强度档位，从低到高；实际可用的取决于模型，编辑器里可以检测。 */
export const CODEX_EFFORTS = ["minimal", "low", "medium", "high", "xhigh", "max"];
/** Claude Code 的 CLAUDE_CODE_EFFORT_LEVEL 接受的档位。 */
export const CLAUDE_EFFORTS = ["low", "medium", "high", "xhigh", "max"];
/** Codex 内置的本地供应商：地址由 Codex 决定，config.toml 里不能覆盖。 */
const CODEX_LOCAL = ["ollama", "lmstudio"];

type Json = Record<string, unknown>;
const isObject = (value: unknown): value is Json => Boolean(value) && typeof value === "object" && !Array.isArray(value);
const asText = (value: unknown) => (typeof value === "string" ? value : typeof value === "number" ? String(value) : "");

function parseJson(text: string): Json | null {
  try {
    const value = JSON.parse(text || "{}");
    return isObject(value) ? value : null;
  } catch {
    return null;
  }
}

function getPath(root: Json, path: string[]): unknown {
  let node: unknown = root;
  for (const part of path) node = isObject(node) ? node[part] : undefined;
  return node;
}

function setPath(root: Json, path: string[], value: unknown) {
  let node = root;
  for (const part of path.slice(0, -1)) {
    if (!isObject(node[part])) node[part] = {};
    node = node[part] as Json;
  }
  const leaf = path[path.length - 1];
  if (value === undefined || value === "") delete node[leaf];
  else node[leaf] = value;
}

const stringify = (value: Json) => JSON.stringify(value, null, 2);

/** 行级改写遇到内联表等写法时可能产出无效 TOML；这种情况保持原文，让用户去完整配置里改。 */
function withToml(text: EditorText, toml: string): EditorText {
  return readToml(toml) === null ? text : { ...text, toml };
}

function withOptions(options: string[], current: string) {
  return current && !options.includes(current) ? [...options, current] : options;
}

/** 并存式工具的模型列表：对象（OpenCode / MCode）或数组（OpenClaw / Pi / Hermes）。 */
function modelIds(models: unknown): string[] {
  if (Array.isArray(models)) {
    return models.map((m) => (typeof m === "string" ? m : isObject(m) ? asText(m.id) || asText(m.name) : "")).filter(Boolean);
  }
  return isObject(models) ? Object.keys(models) : [];
}

function nextModels(models: unknown, ids: string[], arrayForm: boolean): unknown {
  if (arrayForm || Array.isArray(models)) {
    const existing = Array.isArray(models) ? models : [];
    return ids.map((id) => existing.find((m) => (isObject(m) ? asText(m.id) === id : m === id)) ?? { id, name: id });
  }
  const existing = isObject(models) ? models : {};
  return Object.fromEntries(ids.map((id) => [id, existing[id] ?? {}]));
}

interface JsonMap {
  baseUrl?: string[];
  apiKey?: string[];
  api?: { path: string[]; options: string[] };
  models?: { path: string[]; arrayForm: boolean };
}

const JSON_TOOLS: Partial<Record<ToolId, JsonMap>> = {
  "claude-desktop": { baseUrl: ["env", "ANTHROPIC_BASE_URL"], apiKey: ["env", "ANTHROPIC_AUTH_TOKEN"] },
  opencode: { baseUrl: ["options", "baseURL"], apiKey: ["options", "apiKey"], models: { path: ["models"], arrayForm: false } },
  mcode: { baseUrl: ["options", "baseURL"], apiKey: ["options", "apiKey"], api: { path: ["api"], options: ["openai-completions", "anthropic-messages"] }, models: { path: ["models"], arrayForm: false } },
  openclaw: { baseUrl: ["baseUrl"], apiKey: ["apiKey"], api: { path: ["api"], options: ["openai-completions", "openai-responses"] }, models: { path: ["models"], arrayForm: true } },
  pi: { baseUrl: ["baseUrl"], apiKey: ["apiKey"], api: { path: ["api"], options: ["openai-completions", "anthropic-messages"] }, models: { path: ["models"], arrayForm: true } },
  hermes: { baseUrl: ["base_url"], apiKey: ["api_key"], models: { path: ["models"], arrayForm: true } },
};

const filled = (value: unknown) => typeof value === "string" && value !== "";

/** 与后端 connection.rs 一致：只填了 ANTHROPIC_API_KEY 时用它，否则用 ANTHROPIC_AUTH_TOKEN。 */
function claudeAuthField(env: Json): (typeof AUTH_FIELDS)[number] {
  if (filled(env.ANTHROPIC_API_KEY) && !filled(env.ANTHROPIC_AUTH_TOKEN)) return "ANTHROPIC_API_KEY";
  if (env.ANTHROPIC_AUTH_TOKEN === undefined && env.ANTHROPIC_API_KEY !== undefined) return "ANTHROPIC_API_KEY";
  return "ANTHROPIC_AUTH_TOKEN";
}

/** Bedrock / Vertex 走云厂商认证，地址和密钥不在这里填。 */
function advancedAuth(env: Json) {
  return ["CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX"].some((key) => env[key] === true || env[key] === "1" || env[key] === "true");
}

/** 当前生效的表名：显式指定的优先，没指定且只有一张表时用那张。 */
function selectedTable(toml: string, group: string, selected: string): string {
  const tables = getTomlValue(toml, null, group);
  if (!isObject(tables)) return "";
  if (selected) return isObject(tables[selected]) ? selected : "";
  const names = Object.keys(tables).filter((name) => isObject(tables[name]));
  return names.length === 1 ? names[0] : "";
}

/** Codex 请求地址写在哪里：内置 openai 用顶层 openai_base_url，自定义供应商用自己的表。 */
type CodexEndpoint = { kind: "openai" } | { kind: "local"; id: string } | { kind: "custom"; id: string };
function codexEndpoint(toml: string): CodexEndpoint {
  const selected = asText(getTomlValue(toml, null, "model_provider"));
  if (selected === "openai") return { kind: "openai" };
  if (CODEX_LOCAL.includes(selected)) return { kind: "local", id: selected };
  if (selected) return { kind: "custom", id: selected };
  const only = selectedTable(toml, "model_providers", "");
  return only ? { kind: "custom", id: only } : { kind: "openai" };
}
const codexUrlKey = (endpoint: CodexEndpoint): [string[] | null, string] | null =>
  endpoint.kind === "openai" ? [null, "openai_base_url"] : endpoint.kind === "custom" ? [["model_providers", endpoint.id], "base_url"] : null;
const grokModel = (toml: string) => selectedTable(toml, "model", asText(getTomlValue(toml, ["models"], "default")));

/** Codex 的 auth.json 部分：可能是顶层对象，也可能嵌在 `{ auth, ...其他键 }` 里。 */
function codexAuth(root: Json): Json {
  return isObject(root.auth) && Object.keys(root).some((key) => key !== "auth") ? root.auth : root;
}

export function readConnection(tool: ToolId, text: EditorText, official: boolean): Connection {
  if (official) return { state: "official" };
  if (tool === "claude") {
    const root = parseJson(text.json);
    if (!root) return { state: "invalid" };
    const env = isObject(root.env) ? root.env : {};
    const authField = claudeAuthField(env);
    return {
      state: "ready",
      blocked: advancedAuth(env) ? { reason: "advancedAuth", fields: ["baseUrl", "authField", "apiKey"] } : undefined,
      fields: [
        { id: "baseUrl", kind: "url", group: "connection", hint: "ANTHROPIC_BASE_URL" },
        { id: "authField", kind: "select", group: "connection", options: [...AUTH_FIELDS] },
        { id: "apiKey", kind: "secret", group: "connection", hint: authField },
        { id: "model", kind: "text", group: "models", hint: "ANTHROPIC_MODEL" },
        { id: "haikuModel", kind: "text", group: "models", hint: "ANTHROPIC_DEFAULT_HAIKU_MODEL" },
        { id: "sonnetModel", kind: "text", group: "models", hint: "ANTHROPIC_DEFAULT_SONNET_MODEL" },
        { id: "opusModel", kind: "text", group: "models", hint: "ANTHROPIC_DEFAULT_OPUS_MODEL" },
        { id: "reasoning", kind: "effort", group: "models", levels: CLAUDE_EFFORTS, hint: "CLAUDE_CODE_EFFORT_LEVEL" },
        { id: "alwaysEffort", kind: "toggle", group: "models", hint: "CLAUDE_CODE_ALWAYS_ENABLE_EFFORT" },
      ],
      values: {
        baseUrl: asText(env.ANTHROPIC_BASE_URL),
        authField,
        apiKey: asText(env[authField]),
        model: asText(env.ANTHROPIC_MODEL),
        haikuModel: asText(env.ANTHROPIC_DEFAULT_HAIKU_MODEL),
        sonnetModel: asText(env.ANTHROPIC_DEFAULT_SONNET_MODEL),
        opusModel: asText(env.ANTHROPIC_DEFAULT_OPUS_MODEL),
        reasoning: asText(env.CLAUDE_CODE_EFFORT_LEVEL),
        alwaysEffort: asText(env.CLAUDE_CODE_ALWAYS_ENABLE_EFFORT) === "1" ? "1" : "",
      },
    };
  }
  if (tool === "gemini") {
    const root = parseJson(text.json);
    if (!root) return { state: "invalid" };
    const env = isObject(root.env) ? root.env : {};
    return {
      state: "ready",
      fields: [
        { id: "baseUrl", kind: "url", group: "connection", hint: "GOOGLE_GEMINI_BASE_URL" },
        { id: "apiKey", kind: "secret", group: "connection", hint: "GEMINI_API_KEY" },
        { id: "model", kind: "text", group: "models", hint: "GEMINI_MODEL" },
      ],
      values: {
        baseUrl: asText(env.GOOGLE_GEMINI_BASE_URL),
        apiKey: asText(env.GEMINI_API_KEY),
        model: asText(env.GEMINI_MODEL) || asText(getPath(root, ["config", "model", "name"])),
      },
    };
  }
  if (tool === "codex") {
    const auth = parseJson(text.json);
    const toml = text.toml ?? "";
    if (!auth || readToml(toml) === null) return { state: "invalid" };
    const endpoint = codexEndpoint(toml);
    const urlKey = codexUrlKey(endpoint);
    const reasoning = asText(getTomlValue(toml, null, "model_reasoning_effort"));
    return {
      state: "ready",
      fields: [
        { id: "baseUrl", kind: "url", group: "connection", hint: urlKey ? [...(urlKey[0] ?? []), urlKey[1]].join(".") : "base_url" },
        { id: "apiKey", kind: "secret", group: "connection", hint: "OPENAI_API_KEY" },
        { id: "model", kind: "text", group: "models", hint: "model" },
        { id: "reasoning", kind: "effort", group: "models", levels: CODEX_EFFORTS, hint: "model_reasoning_effort" },
      ],
      values: {
        baseUrl: urlKey ? asText(getTomlValue(toml, urlKey[0], urlKey[1])) : "",
        apiKey: asText(codexAuth(auth).OPENAI_API_KEY),
        model: asText(getTomlValue(toml, null, "model")),
        reasoning,
      },
      blocked: endpoint.kind === "local" ? { reason: "builtIn", id: endpoint.id, fields: ["baseUrl"] } : undefined,
    };
  }
  if (tool === "grokbuild") {
    const toml = text.toml ?? "";
    if (readToml(toml) === null) return { state: "invalid" };
    const name = grokModel(toml);
    const table = ["model", name];
    return {
      state: "ready",
      fields: [
        { id: "baseUrl", kind: "url", group: "connection", hint: "base_url" },
        { id: "apiKey", kind: "secret", group: "connection", hint: "api_key" },
        { id: "model", kind: "text", group: "models", hint: "model" },
        { id: "contextWindow", kind: "number", group: "models", hint: "context_window" },
      ],
      values: name
        ? {
            baseUrl: asText(getTomlValue(toml, table, "base_url")),
            apiKey: asText(getTomlValue(toml, table, "api_key")),
            model: asText(getTomlValue(toml, table, "model")),
            contextWindow: asText(getTomlValue(toml, table, "context_window")),
          }
        : {},
      blocked: name ? undefined : { reason: "grokDefault", fields: ["baseUrl", "apiKey", "model", "contextWindow"] },
    };
  }
  const map = JSON_TOOLS[tool];
  if (!map) return { state: "none" };
  const root = parseJson(text.json);
  if (!root) return { state: "invalid" };
  const fields: FieldSpec[] = [];
  const values: Partial<Record<FieldId, FieldValue>> = {};
  if (map.baseUrl) {
    fields.push({ id: "baseUrl", kind: "url", group: "connection", hint: map.baseUrl.join(".") });
    values.baseUrl = asText(getPath(root, map.baseUrl));
  }
  if (map.apiKey) {
    fields.push({ id: "apiKey", kind: "secret", group: "connection", hint: map.apiKey.join(".") });
    values.apiKey = asText(getPath(root, map.apiKey));
  }
  if (map.api) {
    const current = asText(getPath(root, map.api.path));
    fields.push({ id: "api", kind: "select", group: "connection", options: withOptions(map.api.options, current), hint: map.api.path.join(".") });
    values.api = current;
  }
  if (map.models) {
    fields.push({ id: "models", kind: "list", group: "models", hint: map.models.path.join(".") });
    values.models = modelIds(getPath(root, map.models.path));
  }
  return { state: "ready", fields, values };
}

export function writeConnection(tool: ToolId, text: EditorText, field: FieldId, value: FieldValue): EditorText {
  const str = typeof value === "string" ? value.trim() === "" ? "" : value : "";
  if (tool === "claude" || tool === "gemini") {
    const root = parseJson(text.json);
    if (!root) return text;
    const env = isObject(root.env) ? root.env : {};
    root.env = env;
    if (tool === "claude") {
      const authField = claudeAuthField(env);
      const keys: Partial<Record<FieldId, string>> = {
        baseUrl: "ANTHROPIC_BASE_URL",
        apiKey: authField,
        model: "ANTHROPIC_MODEL",
        haikuModel: "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        sonnetModel: "ANTHROPIC_DEFAULT_SONNET_MODEL",
        opusModel: "ANTHROPIC_DEFAULT_OPUS_MODEL",
        reasoning: "CLAUDE_CODE_EFFORT_LEVEL",
        alwaysEffort: "CLAUDE_CODE_ALWAYS_ENABLE_EFFORT",
      };
      if (field === "authField" && typeof value === "string" && value !== authField) {
        const secret = env[authField];
        delete env[authField];
        env[value] = secret ?? "";
      } else if (field === "apiKey") {
        // 密钥清空时保留键名，否则“认证字段”会跳回默认值。
        env[authField] = typeof value === "string" ? value : "";
      } else if (keys[field]) {
        setPath(root, ["env", keys[field]!], str);
      }
    } else {
      const keys: Partial<Record<FieldId, string>> = { baseUrl: "GOOGLE_GEMINI_BASE_URL", apiKey: "GEMINI_API_KEY", model: "GEMINI_MODEL" };
      if (keys[field]) setPath(root, ["env", keys[field]!], str);
      if (field === "model" && getPath(root, ["config", "model", "name"]) !== undefined) setPath(root, ["config", "model", "name"], str);
    }
    return { ...text, json: stringify(root) };
  }
  if (tool === "codex") {
    if (field === "apiKey") {
      const root = parseJson(text.json);
      if (!root) return text;
      codexAuth(root).OPENAI_API_KEY = str;
      if (!str) delete codexAuth(root).OPENAI_API_KEY;
      return { ...text, json: stringify(root) };
    }
    const toml = text.toml ?? "";
    if (field === "model") return withToml(text, setTomlValue(toml, null, "model", str));
    if (field === "reasoning") return withToml(text, setTomlValue(toml, null, "model_reasoning_effort", str));
    if (field === "baseUrl") {
      const urlKey = codexUrlKey(codexEndpoint(toml));
      return urlKey ? withToml(text, setTomlValue(toml, urlKey[0], urlKey[1], str)) : text;
    }
    return text;
  }
  if (tool === "grokbuild") {
    const toml = text.toml ?? "";
    const name = grokModel(toml);
    if (!name) return text;
    const keys: Partial<Record<FieldId, string>> = { baseUrl: "base_url", apiKey: "api_key", model: "model", contextWindow: "context_window" };
    const key = keys[field];
    if (!key) return text;
    const number = field === "contextWindow" && /^\d+$/.test(str) ? Number(str) : undefined;
    return withToml(text, setTomlValue(toml, ["model", name], key, number ?? str));
  }
  const map = JSON_TOOLS[tool];
  const root = parseJson(text.json);
  if (!map || !root) return text;
  if (field === "baseUrl" && map.baseUrl) setPath(root, map.baseUrl, str);
  else if (field === "apiKey" && map.apiKey) setPath(root, map.apiKey, str);
  else if (field === "api" && map.api) setPath(root, map.api.path, str);
  else if (field === "models" && map.models && Array.isArray(value)) {
    setPath(root, map.models.path, nextModels(getPath(root, map.models.path), value, map.models.arrayForm));
  }
  return { ...text, json: stringify(root) };
}
