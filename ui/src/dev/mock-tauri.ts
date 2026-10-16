// Browser-only stand-in for `@tauri-apps/api/core`, used by `pnpm dev:mock`.
// Vite aliases the real module to this file only when LEANFLIP_MOCK=1, so it never ships in the app.
import type { AccountInfo, LoginSummary, Provider, QuotaDetect, QuotaSnapshot, QuotaSource, QuotaView, Scene, SceneStep, SceneTool, SceneUndo, SettingsView, SwitchEvent, ToolId, ToolOutcome, ToolProviders, ToolSummary, UsageReport, UsageRow } from "@/lib/api";

type Kind = "switch" | "coexist";
interface MockTool { kind: Kind; paths: string[]; detected: boolean; current: string | null; inLive: string[]; providers: Provider[] }

const home = "/Users/demo";
const tools: Record<ToolId, MockTool> = {
  claude: {
    kind: "switch", detected: true, current: "kimi", inLive: [], paths: [`${home}/.claude/settings.json`],
    providers: [
      { id: "kimi", name: "Kimi", settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://api.moonshot.cn/anthropic", ANTHROPIC_AUTH_TOKEN: "sk-demo-kimi", ANTHROPIC_MODEL: "kimi-k2" } } },
      { id: "deepseek", name: "DeepSeek", settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://api.deepseek.com/anthropic", ANTHROPIC_AUTH_TOKEN: "sk-demo-ds", ANTHROPIC_MODEL: "deepseek-chat" } } },
      { id: "official", name: "Claude 官方", category: "official", settingsConfig: { env: {} } },
      { id: "relay", name: "公司网关", settingsConfig: { env: { ANTHROPIC_BASE_URL: "https://gw.internal.example/anthropic", ANTHROPIC_AUTH_TOKEN: "sk-demo-relay", ANTHROPIC_MODEL: "claude-sonnet-4-5" } } },
    ],
  },
  codex: {
    kind: "switch", detected: true, current: "official", inLive: [], paths: [`${home}/.codex/config.toml`, `${home}/.codex/auth.json`],
    providers: [
      // 官方卡里留着一张没被选中的表（真实配置里常见）：首页仍应显示官方登录。
      { id: "official", name: "Codex 官方", category: "official", settingsConfig: { auth: {}, config: 'model = "gpt-5"\n\n[model_providers.custom]\nbase_url = "https://gw.example.com/v1"\n' }, meta: { authBinding: { authProvider: "codex_oauth", accountId: "acc-work" } } },
      { id: "openai", name: "OpenAI API", settingsConfig: { auth: { OPENAI_API_KEY: "sk-demo" }, config: 'model = "gpt-5"\nmodel_provider = "openai"\n' } },
      { id: "gateway", name: "Responses 网关", settingsConfig: { auth: { OPENAI_API_KEY: "sk-demo" }, config: 'model = "gpt-5-mini"\nmodel_provider = "gateway"\n\n[model_providers.gateway]\nbase_url = "https://gw.example.com/v1"\n' } },
    ],
  },
  gemini: {
    kind: "switch", detected: true, current: "gapi", inLive: [], paths: [`${home}/.gemini/.env`, `${home}/.gemini/settings.json`],
    providers: [
      { id: "gapi", name: "Gemini API", settingsConfig: { env: { GEMINI_API_KEY: "demo" }, config: { model: { name: "gemini-2.5-pro" } } } },
      { id: "ggw", name: "Gemini 网关", settingsConfig: { env: { GOOGLE_GEMINI_BASE_URL: "https://gw.example.com", GEMINI_API_KEY: "demo", GEMINI_MODEL: "gemini-2.5-flash" } } },
    ],
  },
  grokbuild: { kind: "switch", detected: true, current: null, inLive: [], paths: [`${home}/.grok/config.toml`], providers: [] },
  "claude-desktop": { kind: "switch", detected: false, current: null, inLive: [], paths: [`${home}/Library/Application Support/Claude-3p/claude_desktop_config.json`], providers: [] },
  opencode: {
    kind: "coexist", detected: true, current: null, inLive: ["openrouter", "moonshot"], paths: [`${home}/.config/opencode/opencode.json`],
    providers: [
      { id: "openrouter", name: "OpenRouter", settingsConfig: { npm: "@ai-sdk/openai-compatible", options: { baseURL: "https://openrouter.ai/api/v1", apiKey: "demo" }, models: { "anthropic/claude-sonnet-4.5": {} } } },
      { id: "moonshot", name: "Moonshot", settingsConfig: { npm: "@ai-sdk/openai-compatible", options: { baseURL: "https://api.moonshot.cn/v1", apiKey: "demo" }, models: { "kimi-k2": {} } } },
      { id: "ollama", name: "本地 Ollama", settingsConfig: { npm: "@ai-sdk/openai-compatible", options: { baseURL: "http://localhost:11434/v1" }, models: { "qwen3-coder": {} } } },
    ],
  },
  openclaw: { kind: "coexist", detected: false, current: null, inLive: [], paths: [`${home}/.openclaw/openclaw.json`], providers: [] },
  hermes: {
    kind: "coexist", detected: true, current: null, inLive: ["deepseek"], paths: [`${home}/.hermes/config.yaml`],
    providers: [{ id: "deepseek", name: "DeepSeek", settingsConfig: { base_url: "https://api.deepseek.com/v1", api_key: "demo", models: [{ id: "deepseek-chat" }] } }],
  },
  pi: { kind: "coexist", detected: false, current: null, inLive: [], paths: [`${home}/.pi/agent/models.json`], providers: [] },
  mcode: { kind: "coexist", detected: false, current: null, inLive: [], paths: [`${home}/.minimax/config.yaml`], providers: [] },
};

let accounts: AccountInfo[] = [
  { id: "acc-work", email: "dev@studio.example", workspace: "Studio", plan: "team", authenticatedAt: 1791600000, reauthRequired: false, isDefault: true },
  { id: "acc-home", email: "me@example.com", workspace: null, plan: "plus", authenticatedAt: 1791100000, reauthRequired: true, isDefault: false },
];
const logins: LoginSummary[] = [{ identity: "sub:demo|account:personal", email: "me@example.com", workspace: null, plan: "plus", live: true }];
// 额度：工作账号联网查过（5 小时窗口快满了），个人登录只有本机记录（Pro 只有周窗口）。
let quotaCheckedAt = Date.now() - 3 * 60_000;
function quotaView(): QuotaView {
  const now = Date.now();
  const s = Math.floor(now / 1000);
  return {
    checkedAt: now,
    busiest: "codex:studio:user-dev",
    sources: [
      {
        key: "codex:studio:user-dev", kind: "codex", label: "dev@studio.example", accountId: "acc-work",
        users: [{ tool: "codex", providerId: "official", providerName: "Codex 官方", active: true }],
        snapshot: {
          windows: [
            { minutes: 300, usedPercent: 92, resetsAt: s + 72 * 60, reset: false },
            { minutes: 10080, usedPercent: 40, resetsAt: s + 5 * 86_400, reset: false },
          ],
          credits: { hasCredits: false, unlimited: false, balance: 0 },
          plan: "team", fetchedAt: quotaCheckedAt, origin: "network", level: "warn", usedFraction: 0.92, limitReached: false,
        },
      },
      {
        key: "codex:personal:user-me", kind: "codex", label: "me@example.com", accountId: "acc-home", login: "sub:demo|account:personal", users: [],
        snapshot: {
          windows: [{ minutes: 10080, usedPercent: 51, resetsAt: s + 3 * 86_400 + 4 * 3600, reset: false }],
          credits: { hasCredits: true, unlimited: false, balance: 60951.56 },
          plan: "pro", fetchedAt: now - 42 * 60_000, origin: "local", level: "ok", usedFraction: 0.51, limitReached: false,
        },
        error: { code: "reauth", at: quotaCheckedAt },
      },
      ...relaySources(now),
    ],
  };
}

// 中转站：卡片设置了平台类型才算一个来源，数字是示例。
function relaySnapshot(now: number): QuotaSnapshot {
  return {
    windows: [{ minutes: 300, usedPercent: 84, resetsAt: Math.floor(now / 1000) + 2 * 3600, reset: false, usedAmount: 4.2, limitAmount: 5 }],
    balance: { unit: "USD", remaining: 31.4, limit: 58, used: 26.6, unlimited: false, accountRemaining: 120 },
    plan: "work", fetchedAt: now - 3 * 60_000, origin: "network", level: "warn", usedFraction: 0.84, limitReached: false,
  };
}
function relaySources(now: number): QuotaSource[] {
  const sources: QuotaSource[] = [];
  for (const tool of Object.keys(tools) as ToolId[]) {
    const t = tools[tool];
    for (const p of t.providers) {
      const adapter = (p.meta?.leanflipQuota as { adapter?: string } | undefined)?.adapter;
      if (adapter !== "sub2api" && adapter !== "newapi") continue;
      const active = t.kind === "switch" ? t.current === p.id : t.inLive.includes(p.id);
      sources.push({
        key: `relay:demo:${tool}:${p.id}`, kind: "relay", label: "gw.example.com", adapter,
        users: [{ tool, providerId: p.id, providerName: p.name, active }], snapshot: relaySnapshot(now),
      });
    }
  }
  return sources;
}
let app: SettingsView["app"] = { language: null, theme: null, dirOverrides: {}, autostart: false, backgroundRefreshHours: 168, closeToTray: true };
const ok = { changed: [] as string[], warnings: [] as string[] };
const wait = (ms = 220) => new Promise((resolve) => setTimeout(resolve, ms));

function list(tool: ToolId): ToolProviders {
  const t = tools[tool];
  return { tool, kind: t.kind, providers: structuredClone(t.providers), current: t.current, inLive: [...t.inLive], liveError: null, paths: t.paths };
}
function overview(): ToolSummary[] {
  return (Object.keys(tools) as ToolId[]).map((tool) => {
    const t = tools[tool];
    const current = t.providers.find((p) => p.id === t.current);
    return { tool, kind: t.kind, paths: t.paths, detected: t.detected, cardCount: t.providers.length, current: t.current, currentName: current?.name ?? null, inLive: [...t.inLive], liveReadable: true };
  });
}

// 演示用量：180 天的每日汇总和 64 天前开始的切换记录，数字是虚构的。
function usageDemo(): UsageReport {
  const DAY = 86_400_000;
  const now = Date.now();
  const logStart = now - 64.4 * DAY;
  let seed = 20261013;
  const random = () => (seed = (seed * 16807) % 2147483647) / 2147483647;
  const pad = (n: number) => String(n).padStart(2, "0");
  const key = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  const hit: Record<string, number> = { kimi: 0.93, deepseek: 0.88, official: 0.96, relay: 0.41, openai: 0.9, gateway: 0.62, gapi: 0.7, ggw: 0.35, openrouter: 0.55, moonshot: 0.9, ollama: 0, "": 0.9 };
  const plans: Record<string, [string, number][]> = {
    claude: [["official", 180], ["kimi", 140], ["deepseek", 96], ["relay", 71], ["kimi", 58], ["deepseek", 33.5], ["kimi", 21], ["relay", 12.2], ["kimi", 4.6]],
    codex: [["openai", 180], ["official", 122], ["gateway", 61], ["official", 55.3], ["openai", 26], ["official", 9.8]],
    gemini: [["ggw", 180], ["gapi", 150], ["ggw", 44], ["gapi", 38.6]],
  };
  const name = (tool: ToolId, id: string) => tools[tool].providers.find((p) => p.id === id)?.name ?? id;
  const activeAt = (tool: string, ts: number) => {
    const plan = plans[tool];
    let id = plan[0][0];
    for (const [p, ago] of plan) if (now - ago * DAY <= ts) id = p;
    return id;
  };
  const events: SwitchEvent[] = [];
  for (const tool of ["claude", "codex", "gemini"] as ToolId[]) {
    events.push({ ts: logStart, tool, provider: activeAt(tool, logStart), name: name(tool, activeAt(tool, logStart)), enabled: true });
    for (const [p, ago] of plans[tool]) if (now - ago * DAY > logStart) events.push({ ts: now - ago * DAY, tool, provider: p, name: name(tool, p), enabled: true });
  }
  const coexist: [ToolId, string, number, number | null][] = [["opencode", "openrouter", 64.4, null], ["opencode", "moonshot", 61, null], ["opencode", "ollama", 20, 14], ["hermes", "deepseek", 64.4, null]];
  for (const [tool, p, from, to] of coexist) {
    events.push({ ts: now - from * DAY, tool, provider: p, name: name(tool, p), enabled: true });
    if (to !== null) events.push({ ts: now - to * DAY, tool, provider: p, name: name(tool, p), enabled: false });
  }
  events.sort((a, b) => a.ts - b.ts);
  const base: [ToolId, number, number][] = [["claude", 6.2e7, 170], ["codex", 1.5e8, 1150], ["gemini", 9e6, 45], ["grokbuild", 3.2e6, 22], ["opencode", 1.3e7, 70], ["hermes", 4e6, 30]];
  const rows: UsageRow[] = [];
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  for (let i = 179; i >= 0; i--) {
    const date = new Date(today);
    date.setDate(date.getDate() - i);
    const mid = date.getTime() + DAY / 2;
    const weekend = date.getDay() === 0 || date.getDay() === 6;
    for (const [tool, tokens, requests] of base) {
      const f = (weekend ? 0.42 : 1) * (0.62 + 0.76 * random()) * (0.78 + 0.4 * (179 - i) / 180) * (i === 0 ? (now - today.getTime()) / DAY : 1);
      const parts: [string, UsageRow["attribution"], number][] =
        tool === "opencode"
          ? ([["openrouter", 0.5], ["moonshot", mid > now - 61 * DAY ? 0.35 : 0], ["ollama", mid > now - 20 * DAY && mid < now - 14 * DAY ? 0.15 : 0]] as [string, number][])
              .filter(([, w]) => w > 0).map(([p, w], _, all) => [p, "exact", w / all.reduce((s, [, x]) => s + x, 0)])
          : tool === "hermes" ? [["deepseek", "exact", 1]]
          : tool === "grokbuild" ? [["", "none", 1]]
          : tool === "codex" ? [[activeAt(tool, mid), "exact", 1]]
          : mid < logStart ? [["", "none", 1]] : [[activeAt(tool, mid), "time", 1]];
      for (const [provider, attribution, share] of parts) {
        const total = tokens * f * share;
        const output = total * 0.007;
        const side = total - output;
        const h = hit[provider] ?? 0.85;
        rows.push({
          day: key(date), tool, provider, model: tool === "codex" ? "gpt-5" : tool === "gemini" ? "gemini-2.5-pro" : "default", attribution,
          input: Math.round(side * (1 - h - 0.02)), cacheRead: Math.round(side * h), cacheWrite: tool === "claude" ? Math.round(side * 0.02) : 0, output: Math.round(output),
          requests: Math.round(requests * f * share),
        });
      }
    }
  }
  const source = (tool: ToolId, state: UsageReport["sources"][number]["state"], path: string | null, extra: Partial<UsageReport["sources"][number]> = {}) => ({ tool, state, tested: true, experimental: false, path, retentionDays: null, detail: null, ...extra });
  return {
    rows,
    events,
    scannedAt: now,
    sources: [
      source("claude", "read", `${home}/.claude/projects`, { retentionDays: 30 }),
      source("codex", "read", `${home}/.codex/sessions`),
      source("gemini", "read", `${home}/.gemini/tmp`, { tested: false, retentionDays: 30 }),
      source("grokbuild", "read", `${home}/.grok/sessions`, { experimental: true }),
      source("claude-desktop", "unsupported", null),
      source("opencode", "read", `${home}/.local/share/opencode`),
      source("openclaw", "missing", `${home}/.openclaw/agents`, { tested: false }),
      source("hermes", "read", `${home}/.hermes/state.db`, { tested: false }),
      source("pi", "missing", `${home}/.pi/agent/sessions`, { tested: false }),
      source("mcode", "unsupported", null),
    ],
  };
}

let demoUsage: UsageReport | null = null;
/** 演示后端也记切换，统计页的时间线跟着变。 */
function noteDemo(tool: ToolId, provider: string, enabled: boolean) {
  const name = tools[tool].providers.find((p) => p.id === provider)?.name ?? provider;
  demoUsage?.events.push({ ts: Date.now(), tool, provider, name, enabled });
}

// 场景：「个人」和现在的状态一致，「公司」要切 Claude、Codex 和 OpenCode。
let scenes: Scene[] = [
  { id: "scene-home", name: "个人", tools: { claude: { current: "kimi" }, codex: { current: "official" }, opencode: { enabled: ["openrouter", "moonshot"] } } },
  {
    id: "scene-work", name: "公司",
    tools: { claude: { current: "relay" }, codex: { current: "gateway" }, opencode: { enabled: ["openrouter"] }, hermes: { enabled: [] } },
    rules: [{ kind: "time", days: [1, 2, 3, 4, 5], start: "09:00", end: "18:00" }, { kind: "subnet", cidr: "10.20.0.0/16" }],
  },
];
let lastScene: string | null = "scene-home";

function scenePlan(tool: ToolId, entry: SceneTool): { steps: SceneStep[]; missing: string[] } {
  const t = tools[tool];
  const name = (id: string) => t.providers.find((p) => p.id === id)?.name ?? id;
  const ids = [...(entry.current ? [entry.current] : []), ...(entry.enabled ?? [])];
  const missing = [...new Set(ids.filter((id) => !t.providers.some((p) => p.id === id)))];
  const steps: SceneStep[] = [];
  if (t.kind === "switch") {
    if (entry.current && !missing.includes(entry.current) && t.current !== entry.current) steps.push({ action: { kind: "switch", id: entry.current }, name: name(entry.current) });
  } else {
    for (const p of t.providers) {
      const want = entry.enabled?.includes(p.id) ?? false;
      const have = t.inLive.includes(p.id);
      if (want && !have) steps.push({ action: { kind: "enable", id: p.id }, name: p.name });
      if (!want && have) steps.push({ action: { kind: "disable", id: p.id }, name: p.name });
    }
  }
  return { steps, missing };
}

function sceneEntries(scene: { tools: Record<string, SceneTool> }) {
  return (Object.keys(tools) as ToolId[]).flatMap((tool) => (scene.tools[tool] ? [[tool, scene.tools[tool]] as const] : []));
}

function captureTool(tool: ToolId): SceneTool | null {
  const t = tools[tool];
  if (t.kind === "switch") return t.current ? { current: t.current } : null;
  return t.providers.length ? { enabled: t.providers.filter((p) => t.inLive.includes(p.id)).map((p) => p.id) } : null;
}

function findScene(key: string) {
  const scene = scenes.find((s) => s.id === key) ?? scenes.find((s) => s.name.toLowerCase() === key.trim().toLowerCase());
  if (!scene) throw `找不到场景 ${key}`;
  return scene;
}

function runScene(entries: (readonly [ToolId, SceneTool])[], last: string | null) {
  const undo: SceneUndo = { tools: {}, last: lastScene };
  for (const [tool] of entries) {
    const entry = captureTool(tool);
    if (entry) undo.tools[tool] = entry;
  }
  const results: ToolOutcome[] = entries.map(([tool, entry]) => {
    const t = tools[tool];
    const { steps, missing } = scenePlan(tool, entry);
    for (const { action } of steps) {
      if (action.kind === "switch") t.current = action.id;
      else if (action.kind === "enable") t.inLive = [...t.inLive, action.id];
      else if (action.kind === "disable") t.inLive = t.inLive.filter((id) => id !== action.id);
      noteDemo(tool, action.id, action.kind !== "disable");
    }
    return { tool, outcome: steps.length ? "applied" : "unchanged", changed: steps.length ? t.paths.slice(0, 1) : [], warnings: [], missing, error: null };
  });
  lastScene = last && scenes.some((s) => s.id === last) ? last : null;
  return { results, undo };
}

type Args = Record<string, unknown> | undefined;
const commonConfigs: Partial<Record<ToolId, { config: Record<string, unknown>; version: number }>> = {};
const handlers: Record<string, (args: Args) => unknown> = {
  get_recovery_notices: () => [],
  get_overview: overview,
  usage_report: () => {
    demoUsage ??= usageDemo();
    demoUsage.scannedAt = Date.now();
    return demoUsage;
  },
  list_providers: (a) => list(a!.tool as ToolId),
  // 演示用的检测结果：地址里带 fail 的视为不可连通，带 slow 的延迟较高。
  probe_latency: (a) => {
    const url = JSON.stringify(a!.settingsConfig);
    if (url.includes("internal.example")) return { ok: false, status: null, latencyMs: 0, code: "dns", detail: null, endpoint: "https://gw.internal.example/anthropic", model: null, models: null };
    const ms = url.includes("deepseek") ? 760 : url.includes("openrouter") ? 1320 : 120 + (url.length % 200);
    return { ok: true, status: 404, latencyMs: ms, code: null, detail: null, endpoint: "https://api.example.com/", model: null, models: null };
  },
  probe_models: () => ({
    ok: true, status: 200, latencyMs: 380, code: null, detail: null, endpoint: "https://api.example.com/v1/models", model: null,
    models: [
      { id: "claude-opus-5-5", name: "Claude Opus 5.5", efforts: ["low", "medium", "high", "xhigh", "max"] },
      { id: "claude-haiku-4-5", name: "Claude Haiku 4.5", efforts: [] },
      { id: "kimi-k2-turbo-preview", name: "Kimi K2 Turbo" }, { id: "kimi-k2-0905-preview", name: "Kimi K2" }, { id: "kimi-latest" },
      { id: "moonshot-v1-128k" }, { id: "deepseek-chat", name: "DeepSeek V3" }, { id: "deepseek-reasoner", name: "DeepSeek R1" },
      { id: "anthropic/claude-sonnet-4.5" }, { id: "openai/gpt-5.5" }, { id: "qwen3-coder-plus" }, { id: "glm-4.6" },
    ],
  }),
  probe_efforts: (a) => {
    const model = (a!.model as string | null) ?? "kimi-k2";
    const config = JSON.stringify(a!.settingsConfig);
    const levels = config.includes("ANTHROPIC") ? ["low", "medium", "high", "xhigh", "max"] : ["minimal", "low", "medium", "high", "xhigh", "max"];
    const supported = new Set(["low", "medium", "high"]);
    return {
      ok: true, status: 200, latencyMs: 2400, code: null, detail: null, endpoint: "https://api.example.com/v1/messages", model, models: null,
      levels: levels.map((level) => ({ level, supported: supported.has(level), detail: supported.has(level) ? undefined : `Unsupported value: '${level}'` })),
    };
  },
  probe_check: (a) => {
    const model = (a!.model as string | null) ?? "kimi-k2";
    const config = JSON.stringify(a!.settingsConfig);
    if (config.includes("sk-demo-ds")) return { ok: false, status: 401, latencyMs: 410, code: "auth", detail: "Authentication Fails, Your api key: **** is invalid", endpoint: "https://api.deepseek.com/anthropic/v1/messages", model, models: null };
    return { ok: true, status: 200, latencyMs: 860, code: null, detail: null, endpoint: "https://api.example.com/v1/messages", model, models: null };
  },
  // 近似后端的 Claude 字段过滤，只用于演示。
  ignored_fields: (a) => {
    if (a!.tool !== "claude") return [];
    const config = a!.settingsConfig as Record<string, unknown>;
    const env = Object.keys((config.env ?? {}) as object).filter((k) => !/^(ANTHROPIC_|AWS_|CLAUDE_CODE_)/.test(k)).map((k) => `env.${k}`);
    const top = Object.keys(config).filter((k) => !["env", "model", "apiKeyHelper", "fallbackModel"].includes(k));
    return [...top, ...env].sort();
  },
  switch_provider: (a) => {
    const t = tools[a!.tool as ToolId];
    if (t.kind === "switch") t.current = String(a!.id);
    noteDemo(a!.tool as ToolId, String(a!.id), true);
    return { ...ok, changed: t.paths.slice(0, 1) };
  },
  set_in_live: (a) => {
    const t = tools[a!.tool as ToolId];
    const id = String(a!.id);
    t.inLive = a!.enabled ? [...new Set([...t.inLive, id])] : t.inLive.filter((x) => x !== id);
    noteDemo(a!.tool as ToolId, id, Boolean(a!.enabled));
    return ok;
  },
  add_provider: (a) => {
    const t = tools[a!.tool as ToolId];
    const provider = a!.provider as Provider;
    t.providers.push(provider);
    if (a!.addToLive) {
      if (t.kind === "switch") t.current = provider.id;
      else t.inLive.push(provider.id);
    }
    return ok;
  },
  update_provider: (a) => {
    const t = tools[a!.tool as ToolId];
    const provider = a!.provider as Provider;
    t.providers = t.providers.map((p) => (p.id === provider.id ? provider : p));
    return ok;
  },
  delete_provider: (a) => {
    const t = tools[a!.tool as ToolId];
    t.providers = t.providers.filter((p) => p.id !== a!.id);
    t.inLive = t.inLive.filter((x) => x !== a!.id);
    return null;
  },
  import_live: (a) => {
    const tool = a!.tool as ToolId;
    const t = tools[tool];
    if (t.providers.length || !t.detected) return { created: [] };
    t.providers.push({ id: "current", name: "当前配置", settingsConfig: { config: '[models]\ndefault = "grok-4"\n\n[model."grok-4"]\nmodel = "grok-4"\nbase_url = "https://api.x.ai/v1"\n' } });
    if (t.kind === "switch") t.current = "current";
    return { created: ["current"] };
  },
  list_scenes: () => ({
    last: lastScene,
    scenes: scenes.map((scene) => {
      const plans = sceneEntries(scene).map(([tool, entry]) => ({ tool, ...scenePlan(tool, entry) }));
      const pending = plans.filter((p) => p.steps.length || p.missing.length).map((p) => p.tool);
      return {
        ...scene,
        status: pending.length === 0 ? "active" : scene.id === lastScene ? "drifted" : "idle",
        // 演示：「公司」的规则一直满足，没在用时建议切换。
        suggested: pending.length > 0 && (scene.rules?.length ?? 0) > 0,
        pending,
        missing: plans.flatMap((p) => p.missing.map((id) => ({ tool: p.tool, id }))),
      };
    }),
  }),
  capture_scene: (a) => {
    const wanted = (a?.tools as ToolId[] | null) ?? (Object.keys(tools) as ToolId[]);
    const out: Record<string, SceneTool> = {};
    for (const tool of wanted) {
      const entry = captureTool(tool);
      if (entry) out[tool] = entry;
    }
    return { id: "", name: "", tools: out };
  },
  save_scene: (a) => {
    const scene = { ...(a!.scene as Scene) };
    scene.name = scene.name.trim();
    if (!scene.name) throw "场景名不能为空";
    if (scenes.some((s) => s.id !== scene.id && s.name.toLowerCase() === scene.name.toLowerCase())) throw `已经有一个叫「${scene.name}」的场景`;
    if (sceneEntries(scene).length === 0) throw "场景至少要包含一个工具";
    if (!scene.id) {
      scene.id = `scene-${Math.random().toString(16).slice(2, 10)}`;
      scenes.push(scene);
    } else scenes = scenes.map((s) => (s.id === scene.id ? scene : s));
    return scene;
  },
  delete_scene: (a) => {
    scenes = scenes.filter((s) => s.id !== a!.id);
    if (lastScene === a!.id) lastScene = null;
    return null;
  },
  preview_scene: (a) => {
    const scene = findScene(String(a!.id));
    return {
      scene,
      tools: sceneEntries(scene).map(([tool, entry]) => {
        const t = tools[tool];
        return { tool, kind: t.kind, from: t.kind === "switch" ? (t.providers.find((p) => p.id === t.current)?.name ?? null) : null, ...scenePlan(tool, entry), error: null };
      }),
    };
  },
  apply_scene: (a) => {
    const scene = findScene(String(a!.id));
    const only = a!.tools as ToolId[] | null;
    return runScene(sceneEntries(scene).filter(([tool]) => !only || only.includes(tool)), scene.id);
  },
  undo_scene: (a) => {
    const undo = a!.undo as SceneUndo;
    return runScene(sceneEntries(undo), undo.last);
  },
  codex_accounts: () => accounts,
  codex_logins: () => logins,
  quota_overview: () => quotaView(),
  quota_refresh: () => { quotaCheckedAt = Date.now(); return quotaView(); },
  quota_detect: (): QuotaDetect => ({
    steps: [
      { adapter: "sub2api", url: "https://gw.example.com/v1/usage", outcome: "miss", status: 200, code: "format" },
      { adapter: "newapi", url: "https://gw.example.com/api/usage/token", outcome: "hit", status: 200 },
    ],
    adapter: "newapi",
    snapshot: relaySnapshot(Date.now() + 3 * 60_000),
    key: "relay:demo",
  }),
  quota_configure: (a) => {
    const provider = tools[a!.tool as ToolId].providers.find((p) => p.id === a!.id);
    if (!provider) throw "Provider not found";
    const meta = { ...(provider.meta ?? {}) };
    if (a!.settings) meta.leanflipQuota = a!.settings;
    else delete meta.leanflipQuota;
    provider.meta = meta;
    return null;
  },
  codex_set_default: (a) => { accounts = accounts.map((x) => ({ ...x, isDefault: x.id === a!.id })); return null; },
  codex_remove_account: (a) => { accounts = accounts.filter((x) => x.id !== a!.id); return null; },
  codex_set_login_ref: () => ok,
  codex_start_login: () => ({ deviceCode: "demo", userCode: "LEAN-FL1P", verificationUri: "https://auth.openai.com/codex/device", expiresIn: 900, interval: 5 }),
  codex_poll_login: () => ({ status: "pending" }),
  codex_cancel_login: () => true,
  get_settings: (): SettingsView => ({
    app, keepCodexLogin: false, dataDir: `${home}/.leanflip`, home,
    livePaths: Object.fromEntries((Object.keys(tools) as ToolId[]).map((tool) => [tool, tools[tool].paths])),
    dirs: {}, ccswitchDir: `${home}/.cc-switch`,
  }),
  get_common_config: (a) => { const tool = a!.tool as ToolId; const state = commonConfigs[tool] ?? { config: {}, version: 0 }; return { ...state, revision: String(state.version), paths: tools[tool].paths.slice(0, 1), format: ["codex", "grokbuild"].includes(tool) ? "TOML" : "JSON" }; },
  save_common_config: (a) => { const tool = a!.tool as ToolId; const state = commonConfigs[tool] ?? { config: {}, version: 0 }; if (String(state.version) !== a!.revision) throw new Error("Configuration changed"); commonConfigs[tool] = { config: a!.config as Record<string, unknown>, version: state.version + 1 }; return ok; },
  save_settings: (a) => { app = (a!.input as { app: SettingsView["app"] }).app; return { dirsChanged: false }; },
  ccswitch_scan: () => ({ directory: `${home}/.cc-switch`, source: null, candidates: [], accounts: 0 }),
  open_url: () => null,
};

export async function invoke<T>(cmd: string, args?: Args): Promise<T> {
  await wait();
  const handler = handlers[cmd];
  if (!handler) throw `Demo backend does not implement ${cmd}`;
  return structuredClone(handler(args)) as T;
}
