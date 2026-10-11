// 把 cc-switch 的预设（MIT，Copyright (c) 2025 Jason Young）裁剪成 JSON，放到
// ui/src/presets/。用法：node scripts/build-presets.mjs [cc-switch 源码目录]
// 只在更新预设时跑一次，生成的 JSON 提交进仓库；构建不依赖 cc-switch 源码。
//
// 裁剪规则：去掉 hidden、requiresOAuth、带 providerType（Copilot / 托管 OAuth 等需要
// 代理或专用登录的）、Claude / Claude Desktop 里 apiFormat 不是 anthropic（需要本地
// 格式转换代理）的预设；去掉合作伙伴推广、主题、图标等界面字段。链接只留模型厂商
// 官方平台的（去掉返利 / 追踪参数），中转站 / 聚合平台的链接整条删除。
import { build } from "esbuild";
import { mkdirSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const upstream = path.resolve(process.argv[2] ?? "../ccswitch");
const outDir = path.resolve("ui/src/presets");
const tmp = path.resolve("node_modules/.cache/ccsl-presets");

const sources = {
  claude: ["claudeProviderPresets.ts", "providerPresets"],
  codex: ["codexProviderPresets.ts", "codexProviderPresets"],
  gemini: ["geminiProviderPresets.ts", "geminiProviderPresets"],
  grokbuild: ["grokBuildProviderPresets.ts", "grokBuildProviderPresets"],
  "claude-desktop": ["claudeDesktopProviderPresets.ts", "claudeDesktopProviderPresets"],
  opencode: ["opencodeProviderPresets.ts", "opencodeProviderPresets"],
  openclaw: ["openclawProviderPresets.ts", "openclawProviderPresets"],
  hermes: ["hermesProviderPresets.ts", "hermesProviderPresets"],
  pi: ["piProviderPresets.ts", "piProviderPresets"],
  mcode: ["mcodeProviderPresets.ts", "mcodeProviderPresets"],
};

const KEEP = ["name", "websiteUrl", "apiKeyUrl", "settingsConfig", "isOfficial", "category", "apiKeyField"];

// Codex / Grok 预设是顶层 auth + config；Claude Desktop 是 baseUrl + 模式。统一成卡片的
// settingsConfig。
// Grok Build 的 cc-switch 预设是 Codex 风格的载体（model_provider / model_providers），
// cc-switch 保存时才转成 Grok 自己的 [models] / [model.<name>]。这里直接转好，
// 和 cc-switch 库里存的形状一致（api_key 留空让用户填）。
export function grokNativeConfig(carrier, fallbackName) {
  const line = (key) => carrier.match(new RegExp(`^${key}\\s*=\\s*"([^"]*)"`, "m"))?.[1] ?? "";
  const model = line("model") || "grok-4.5";
  const q = JSON.stringify;
  return [
    "[models]",
    `default = ${q(model)}`,
    "",
    `[model.${q(model)}]`,
    `model = ${q(model)}`,
    `base_url = ${q(line("base_url"))}`,
    `name = ${q(line("name") || fallbackName)}`,
    'api_key = ""',
    'api_backend = "responses"',
    "context_window = 500000",
    "",
  ].join("\n");
}

function normalize(tool, preset) {
  if (tool === "grokbuild" && !preset.settingsConfig) {
    const official = preset.category === "official";
    return { ...preset, settingsConfig: { config: official ? preset.config ?? "" : grokNativeConfig(preset.config ?? "", preset.name) } };
  }
  if (tool === "codex" && !preset.settingsConfig) {
    return { ...preset, settingsConfig: { auth: preset.auth ?? {}, config: preset.config ?? "" } };
  }
  if (tool === "claude-desktop" && !preset.settingsConfig) {
    const official = preset.category === "official";
    const env = official
      ? {}
      : { ANTHROPIC_BASE_URL: preset.baseUrl, [preset.apiKeyField ?? "ANTHROPIC_AUTH_TOKEN"]: "" };
    return { ...preset, isOfficial: official || undefined, settingsConfig: { env } };
  }
  return preset;
}

function keep(tool, preset) {
  // 官方 Codex 卡：既能用本机登录，也能绑定托管账号（providerType 只是 cc-switch 的标记）。
  const officialCodex = tool === "codex" && preset.category === "official";
  if (preset.hidden || preset.requiresOAuth || (preset.providerType && !officialCodex)) return false;
  // Claude Desktop：只有直连模式（proxy 模式要本地路由）。
  if (tool === "claude-desktop" && preset.mode === "proxy") return false;
  // Codex / Grok：只有 Responses 直连（Chat / Anthropic 格式要本地转换）。
  if ((tool === "codex" || tool === "grokbuild") && preset.apiFormat && preset.apiFormat !== "openai_responses") {
    return false;
  }
  if ((tool === "claude" || tool === "claude-desktop") && preset.apiFormat && preset.apiFormat !== "anthropic") {
    return false;
  }
  if (!preset.settingsConfig || typeof preset.settingsConfig !== "object") return false;
  return true;
}

// 预设链接只留模型厂商自己的官方平台（白名单见 ui/src/lib/official-vendor-domains.json）：
// - 中转站 / 聚合平台 / 第三方推理云的链接整条删掉，不留域名（包括 cc-switch 的合作伙伴）。
// - 官方链接去掉返利 / 追踪参数（?aff=、?track_id=、utm_* …）；路径形式的邀请码只留域名。
// 只动 websiteUrl / apiKeyUrl；settingsConfig 里的 API 地址是配置，不动。
const officialList = JSON.parse(readFileSync(path.resolve("ui/src/lib/official-vendor-domains.json"), "utf8"));
const OFFICIAL = Object.entries(officialList.vendors).flatMap(([vendor, domains]) => domains.map((d) => [d, vendor]));
const TRACKING_PARAMS = new Set([
  "aff", "track_id", "ref", "invite", "invite_code", "invitecode", "ic", "code",
  "source", "from", "ch", "ytag", "ac", "rc",
]);
const REFERRAL_PATH = [
  /^\/(i|invite|r|ref|go|aff|s)\/[^/]+\/?$/i,
  /^\/(agent\/)?register\/[A-Za-z0-9_-]{6,}\/?$/,
  /cc-?switch/i,
];
const SHORT_CODE = /^\/(?=[A-Za-z0-9]*\d)(?=[A-Za-z0-9]*[A-Z])[A-Za-z0-9]{5,10}\/?$/;

/** 官方厂商名；不在白名单里返回 null。 */
export function officialVendor(host) {
  const h = host.toLowerCase();
  return OFFICIAL.find(([d]) => h === d || h.endsWith(`.${d}`))?.[1] ?? null;
}

/** 去掉返利 / 追踪参数和路径邀请码。 */
export function cleanOfficialUrl(value) {
  const url = new URL(value);
  for (const key of [...url.searchParams.keys()]) {
    const k = key.toLowerCase();
    if (TRACKING_PARAMS.has(k) || k.startsWith("utm_")) url.searchParams.delete(key);
  }
  if (REFERRAL_PATH.some((re) => re.test(url.pathname)) || SHORT_CODE.test(url.pathname)) return `${url.origin}/`;
  return url.toString().replace(/\?(?=#|$)/, "");
}

export const removedLinks = [];
export const keptLinks = [];

function trim(tool, preset) {
  const out = {};
  for (const key of KEEP) if (preset[key] !== undefined) out[key] = preset[key];
  for (const field of ["websiteUrl", "apiKeyUrl"]) {
    if (typeof out[field] !== "string" || !out[field]) continue;
    let url;
    try {
      url = new URL(out[field]);
    } catch {
      url = null;
    }
    const vendor = url && url.protocol === "https:" ? officialVendor(url.host) : null;
    if (vendor) {
      const cleaned = cleanOfficialUrl(out[field]);
      keptLinks.push({ tool, name: preset.name, field, vendor, cleaned: cleaned !== out[field] });
      out[field] = cleaned;
    } else {
      removedLinks.push({ tool, name: preset.name, field, reason: "not-official-vendor" });
      delete out[field];
    }
  }
  return out;
}

mkdirSync(outDir, { recursive: true });
mkdirSync(tmp, { recursive: true });
const counts = {};
for (const [tool, [file, exportName]] of Object.entries(sources)) {
  const outfile = path.join(tmp, `${tool}.mjs`);
  await build({
    entryPoints: [path.join(upstream, "src/config", file)],
    bundle: true,
    format: "esm",
    platform: "node",
    outfile,
    alias: { "@": path.join(upstream, "src") },
    nodePaths: [path.resolve("node_modules")],
    logLevel: "error",
  });
  const mod = await import(pathToFileURL(outfile).href);
  const extra = tool === "grokbuild" && mod.grokBuildOfficialPreset ? [mod.grokBuildOfficialPreset] : [];
  const all = [...extra, ...(mod[exportName] ?? [])].map((p) => normalize(tool, p));
  if (all.length === extra.length) throw new Error(`${file}: ${exportName} is empty`);
  const seen = new Set();
  const presets = all
    .filter((p) => keep(tool, p))
    .map((p) => trim(tool, p))
    .filter((p) => !seen.has(p.name) && seen.add(p.name));
  writeFileSync(path.join(outDir, `${tool}.json`), JSON.stringify(presets, null, 2) + "\n");
  counts[tool] = `${presets.length}/${all.length}`;
}
rmSync(tmp, { recursive: true, force: true });
console.log("presets kept/total:", counts);
// 记录删掉了哪些字段（不写原链接，免得把返利码又带回仓库）；presets.test.ts 用它做守卫。
const kept = new Set(Object.keys(counts));
const seenRemoved = new Set();
const removed = removedLinks
  .filter((r) => kept.has(r.tool))
  .filter((r) => {
    const key = `${r.tool}\u0000${r.name}\u0000${r.field}`;
    return !seenRemoved.has(key) && seenRemoved.add(key);
  });
writeFileSync(path.join(outDir, "..", "lib", "removed-preset-links.json"), JSON.stringify(removed, null, 1) + "\n");
const kept2 = keptLinks.filter((r) => kept.has(r.tool));
const vendors = {};
for (const r of kept2) vendors[r.vendor] = (vendors[r.vendor] ?? 0) + 1;
console.log(`link fields kept (official vendors): ${kept2.length}, cleaned of params: ${kept2.filter((r) => r.cleaned).length}`, vendors);
console.log(`link fields removed (relay / aggregator / non-vendor): ${removed.length}`);
