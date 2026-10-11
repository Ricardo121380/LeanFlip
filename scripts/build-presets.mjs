// 把 cc-switch 的预设（MIT，Copyright (c) 2025 Jason Young）裁剪成 JSON，放到
// ui/src/presets/。用法：node scripts/build-presets.mjs [cc-switch 源码目录]
// 只在更新预设时跑一次，生成的 JSON 提交进仓库；构建不依赖 cc-switch 源码。
//
// 裁剪规则：去掉 hidden、requiresOAuth、带 providerType（Copilot / 托管 OAuth 等需要
// 代理或专用登录的）、Claude / Claude Desktop 里 apiFormat 不是 anthropic（需要本地
// 格式转换代理）的预设；去掉合作伙伴推广、主题、图标等界面字段，以及链接里的返利 /
// 追踪参数。
import { build } from "esbuild";
import { mkdirSync, writeFileSync, rmSync } from "node:fs";
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
function normalize(tool, preset) {
  if ((tool === "codex" || tool === "grokbuild") && !preset.settingsConfig) {
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

// 推广链接（返利 / 邀请 / 赞助 / 合作伙伴）整条删掉，不留域名：
// - 查询参数里带返利 / 追踪码（?aff=、?ref=、?invite=、utm_* …）
// - 路径形式的邀请码：/i/CODE、/invite/CODE、/r/CODE、/go/CODE、/register/CODE、
//   /agent/register/CODE、路径里带 ccswitch 的活动页、单段随机短码（/VjM74M）
// - 短链接服务（跳转目标未知）
// - cc-switch 标成商业合作伙伴的预设（isPartner / partnerPromotionKey）的所有链接
// 只动 websiteUrl / apiKeyUrl；settingsConfig 里的 API 地址是配置，不动。
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
const SHORTENER_HOSTS = new Set(["s.qiniu.com", "bit.ly", "t.cn", "dwz.cn", "tinyurl.com", "url.cn"]);

/** 推广链接返回原因，普通链接返回 null。 */
export function promoReason(value, preset = {}) {
  if (preset.isPartner || preset.primePartner || preset.partnerPromotionKey) return "partner";
  let url;
  try {
    url = new URL(value);
  } catch {
    return null;
  }
  if (SHORTENER_HOSTS.has(url.host)) return "short-link";
  for (const key of url.searchParams.keys()) {
    if (TRACKING_PARAMS.has(key.toLowerCase()) || key.toLowerCase().startsWith("utm_")) return `query:${key}`;
  }
  if (REFERRAL_PATH.some((re) => re.test(url.pathname)) || SHORT_CODE.test(url.pathname)) return "referral-path";
  return null;
}

export const removedLinks = [];

function trim(tool, preset) {
  const out = {};
  for (const key of KEEP) if (preset[key] !== undefined) out[key] = preset[key];
  for (const field of ["websiteUrl", "apiKeyUrl"]) {
    if (typeof out[field] !== "string" || !out[field]) continue;
    const reason = promoReason(out[field], preset);
    if (reason) {
      removedLinks.push({ tool, name: preset.name, field, reason: reason.startsWith("query:") ? "tracking-query" : reason });
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
const byReason = {};
for (const r of removed) byReason[r.reason] = (byReason[r.reason] ?? 0) + 1;
console.log(`promo link fields removed: ${removed.length}`, byReason);
