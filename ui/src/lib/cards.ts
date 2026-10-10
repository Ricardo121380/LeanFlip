// 卡片相关的纯函数（有单元测试）。
import { parse as parseToml } from "smol-toml";
import type { AccountInfo, Provider, ToolId } from "./api";

/** 后端用来替换 ChatGPT 登录凭据的占位符（crates/app/src/redact.rs）。 */
export const REDACTED = "__ccsl_redacted__";

export function isOfficial(provider: Pick<Provider, "category">): boolean {
  return provider.category === "official";
}

function str(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

/** 卡片上显示的请求地址。 */
export function endpointOf(tool: ToolId, provider: Provider): string | undefined {
  const sc = provider.settingsConfig ?? {};
  const env = (sc.env ?? {}) as Record<string, unknown>;
  switch (tool) {
    case "claude":
    case "claude-desktop":
      return str(env.ANTHROPIC_BASE_URL);
    case "gemini":
      return str(env.GOOGLE_GEMINI_BASE_URL);
    case "codex":
    case "grokbuild": {
      const config = typeof sc.config === "string" ? sc.config : "";
      return config.match(/^\s*base_url\s*=\s*"([^"]+)"/m)?.[1];
    }
    case "opencode":
    case "mcode":
      return str((sc.options as Record<string, unknown> | undefined)?.baseURL) ?? str(sc.baseUrl);
    case "openclaw":
    case "pi":
      return str(sc.baseUrl) ?? str(sc.baseURL);
    case "hermes":
      return str(sc.base_url) ?? str(sc.baseUrl);
  }
}

/** 预设名 → 卡片 id（并存式工具里也是客户端配置里的条目名）。 */
export function slugify(name: string): string {
  const slug = name
    .normalize("NFKD")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return slug || "provider";
}

export function uniqueId(base: string, taken: Iterable<string>): string {
  const used = new Set(taken);
  if (!used.has(base)) return base;
  for (let n = 2; ; n++) {
    const candidate = `${base}-${n}`;
    if (!used.has(candidate)) return candidate;
  }
}

/** 套餐徽章：plus / team / pro / enterprise…… */
export function planBadge(plan: string | null | undefined): { label: string; variant: "plus" | "team" | "pro" | "secondary" } | null {
  const p = plan?.trim().toLowerCase();
  if (!p) return null;
  if (p === "plus") return { label: "Plus", variant: "plus" };
  if (p === "team" || p === "business") return { label: "Team", variant: "team" };
  if (p === "pro") return { label: "Pro", variant: "pro" };
  return { label: p.charAt(0).toUpperCase() + p.slice(1), variant: "secondary" };
}

export function accountLabel(account: Pick<AccountInfo, "email" | "workspace" | "id">): string {
  return account.email ?? account.workspace ?? account.id;
}

// ---- 卡片编辑器：编辑的是 settingsConfig 的文本，保存时只改动过的部分写回。

export interface EditorText {
  /** Codex / Grok：auth.json 的 JSON 文本；其他工具：整个 settingsConfig。 */
  json: string;
  /** Codex / Grok：config.toml 原文（一字不改地保存）。 */
  toml?: string;
}

export function toEditorText(editor: "json" | "codex", settingsConfig: Record<string, unknown>): EditorText {
  if (editor === "codex") {
    const { auth, config, ...rest } = settingsConfig;
    // 其他键（如果有）也放进 JSON 里，免得编辑时丢掉。
    const jsonPart = Object.keys(rest).length ? { auth: auth ?? {}, ...rest } : (auth ?? {});
    return {
      json: JSON.stringify(jsonPart, null, 2),
      toml: typeof config === "string" ? config : "",
    };
  }
  return { json: JSON.stringify(settingsConfig ?? {}, null, 2) };
}

export type ParseResult = { ok: true; value: Record<string, unknown> } | { ok: false; error: string };

export function fromEditorText(
  editor: "json" | "codex",
  text: EditorText,
  original?: Record<string, unknown>,
): ParseResult {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text.json || "{}");
  } catch (e) {
    return { ok: false, error: `JSON: ${(e as Error).message}` };
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    return { ok: false, error: "JSON: expected an object" };
  }
  if (editor !== "codex") return { ok: true, value: parsed as Record<string, unknown> };

  const toml = text.toml ?? "";
  if (toml.trim()) {
    try {
      parseToml(toml);
    } catch (e) {
      return { ok: false, error: `TOML: ${(e as Error).message.split("\n")[0]}` };
    }
  }
  const obj = parsed as Record<string, unknown>;
  const hadExtra = original && Object.keys(original).some((k) => k !== "auth" && k !== "config");
  const value: Record<string, unknown> =
    hadExtra && "auth" in obj ? { ...obj, config: toml } : { auth: obj, config: toml };
  return { ok: true, value };
}

/** 文本里还留着占位符（只能在原卡上保存，后端会还原）。 */
export function hasRedacted(text: EditorText): boolean {
  return text.json.includes(REDACTED);
}
