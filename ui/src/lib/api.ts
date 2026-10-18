// 后端命令的类型化封装（命令定义见 crates/app/src/commands.rs）。
import { invoke } from "@tauri-apps/api/core";
import i18n from "@/i18n";

export type ToolId =
  | "claude"
  | "codex"
  | "gemini"
  | "grokbuild"
  | "claude-desktop"
  | "opencode"
  | "openclaw"
  | "hermes"
  | "pi"
  | "mcode";

export type ToolKind = "switch" | "coexist";

export interface AuthBinding {
  authProvider: string;
  accountId?: string;
}

export interface Provider {
  id: string;
  name: string;
  category?: string;
  settingsConfig: Record<string, unknown>;
  meta?: { authBinding?: AuthBinding; codexLoginRef?: string; [key: string]: unknown };
  sortIndex?: number;
  createdAt?: number;
  websiteUrl?: string;
  notes?: string;
  [key: string]: unknown;
}

export interface RemoteModel {
  id: string;
  name?: string;
  /** 模型列表自带的推理强度档位（Anthropic）。 */
  efforts?: string[];
}

export interface LevelResult {
  level: string;
  supported: boolean;
  detail?: string;
}

/** 测连通 / 获取模型列表 / 检查模型的结果（crates/service/src/probe.rs）。 */
export interface ProbeResult {
  ok: boolean;
  status: number | null;
  latencyMs: number;
  code: string | null;
  detail: string | null;
  endpoint: string | null;
  model: string | null;
  models: RemoteModel[] | null;
  levels?: LevelResult[] | null;
}

export interface ToolSummary {
  tool: ToolId;
  kind: ToolKind;
  paths: string[];
  detected: boolean;
  cardCount: number;
  current: string | null;
  selected?: string | null;
  currentName: string | null;
  inLive: string[];
  liveReadable?: boolean;
}

export interface ToolProviders {
  tool: ToolId;
  kind: ToolKind;
  providers: Provider[];
  current: string | null;
  selected?: string | null;
  inLive: string[];
  liveError: string | null;
  paths: string[];
}

export interface CommonConfig { config: Record<string, unknown>; paths: string[]; revision: string; format: string; variant?: "opencode-v2" }

export interface OpResult {
  changed: string[];
  warnings: string[];
}

export interface AccountInfo {
  id: string;
  email: string | null;
  workspace: string | null;
  plan: string | null;
  authenticatedAt: number;
  reauthRequired: boolean;
  isDefault: boolean;
}

export interface DeviceFlow {
  deviceCode: string;
  userCode: string;
  verificationUri: string;
  expiresIn: number;
  interval: number;
}

export type PollResult = { status: "pending" } | { status: "added"; account: AccountInfo };

export interface LoginSummary {
  identity: string;
  email: string | null;
  workspace: string | null;
  plan: string | null;
  live: boolean;
}

/** 一个限额窗口。窗口长度要按 `minutes` 标注：有的套餐只有周窗口。 */
export interface QuotaWindow {
  minutes: number;
  label?: string;
  unit?: string;
  usedPercent: number;
  /** Unix 秒。 */
  resetsAt?: number;
  /** 记录之后已过重置时间，用量要等下一条记录更新。 */
  reset: boolean;
  /** 中转站的金额窗口：已用和上限，和余额同一单位。 */
  usedAmount?: number;
  limitAmount?: number;
}

/** 中转站的余额。 */
export interface QuotaBalance {
  unit: string;
  remaining?: number;
  /** 总额度；钱包和订阅没有。 */
  limit?: number;
  used?: number;
  unlimited: boolean;
  /** Key 的过期时间，Unix 秒。 */
  expiresAt?: number;
  /** 账户余额（NewAPI 允许时随 Key 返回）。 */
  accountRemaining?: number;
}

export interface QuotaCredits {
  hasCredits: boolean;
  unlimited: boolean;
  balance?: number;
}

export type QuotaLevel = "ok" | "warn" | "out";

export interface QuotaSnapshot {
  windows: QuotaWindow[];
  credits?: QuotaCredits;
  balance?: QuotaBalance;
  plan?: string;
  /** 毫秒。本机记录是那次请求的时间。 */
  fetchedAt: number;
  origin: "local" | "network";
  level: QuotaLevel;
  usedFraction?: number;
  limitReached: boolean;
}

export interface QuotaError {
  code: "reauth" | "unauthorized" | "rateLimited" | "http" | "timeout" | "network" | "format" | "token" | string;
  status?: number;
  at: number;
}

export interface QuotaUser {
  tool: ToolId;
  providerId: string;
  providerName: string;
  active: boolean;
}

export type RelayAdapter = "sub2api" | "newapi" | "krill";

export interface QuotaSource {
  key: string;
  kind: "codex" | "relay";
  label?: string;
  /** 托管账号 id：有它才能联网查询。 */
  accountId?: string;
  /** 本机登录（`LoginSummary.identity`）。 */
  login?: string;
  /** 中转站的平台类型。 */
  adapter?: RelayAdapter;
  /** 中转站的自动刷新间隔（分钟）；不自动刷新时为空。 */
  refreshMinutes?: number;
  users: QuotaUser[];
  snapshot?: QuotaSnapshot;
  error?: QuotaError;
}

export interface QuotaView {
  sources: QuotaSource[];
  busiest?: string;
  checkedAt: number;
}

/** 卡片上的额度设置（`meta.leanflipQuota`）。 */
export interface QuotaSettings {
  /** 没有时表示还没识别过；`off` 表示不查。 */
  adapter?: RelayAdapter | "off";
  /** 自动刷新间隔（分钟），0 表示只在点刷新时查。 */
  refreshMinutes: number;
  /** NewAPI 多少额度算 $1，默认 500000。 */
  quotaPerUsd?: number;
  /** Krill 网站登录 JWT；后端读取时返回占位符。 */
  krillJwt?: string;
  accountToken?: string;
  accountUserId?: number;
  queryBaseUrl?: string;
}

export interface DetectStep {
  adapter: RelayAdapter;
  /** 不含 Key。 */
  url: string;
  outcome: "hit" | "miss" | "error" | "skipped";
  status?: number;
  code?: string;
}

export interface QuotaDetect {
  steps: DetectStep[];
  adapter?: RelayAdapter;
  snapshot?: QuotaSnapshot;
  key?: string;
  /** `noEndpoint`：卡片里没有地址或 Key。 */
  error?: string;
}

export type DirOverrides = Partial<
  Record<
    | "claudeDir"
    | "codexDir"
    | "geminiDir"
    | "grokDir"
    | "claudeDesktopRoot"
    | "opencodeDir"
    | "openclawDir"
    | "hermesDir"
    | "piAgentDir"
    | "mcodeDir",
    string | null
  >
>;

export interface AppSettings {
  language: "zh" | "en" | null;
  theme?: "light" | "dark" | null;
  dirOverrides: DirOverrides;
  autostart: boolean;
  backgroundRefreshHours: number;
  closeToTray: boolean;
  /** 在状态栏图标旁显示最紧张的额度，默认关闭。 */
  trayQuotaTitle?: boolean;
  /** 每隔这么多分钟联网查一次托管 Codex 账号的额度；0 关闭。 */
  codexQuotaRefreshMinutes?: number;
}

export interface SettingsView {
  app: AppSettings;
  keepCodexLogin: boolean;
  dataDir: string;
  home: string;
  livePaths: Record<string, string[]>;
  dirs: Record<string, string>;
  ccswitchDir: string;
}

export interface ImportCandidate {
  tool: ToolId;
  provider: Provider;
  skipReason: Reason | null;
  notes?: Reason[];
}

export interface ImportScan {
  directory?: string;
  source: string | null;
  candidates: ImportCandidate[];
  accounts: number;
}

export interface BackupPreview {
  createdAt: number;
  items: { tool: ToolId; id: string; name: string; duplicate: boolean; accountId?: string | null }[];
  accounts: number;
  existingAccounts: number;
  hasSettings: boolean;
  hasPaths: boolean;
  paths: { tool: ToolId; current: string[]; target: string[]; resolvedTarget: string[]; outsideHome: boolean }[];
  pathsFingerprint: string;
  /** 备份里的场景个数。 */
  scenes?: number;
}

export interface RestoreOptions {
  tools: ToolId[];
  accounts: boolean;
  settings: boolean;
  paths: boolean;
  pathsConfirmation?: string;
  /** 恢复备份里的场景（引用的供应商改了 ID 时跟着改）。 */
  scenes: boolean;
  duplicate: DuplicatePolicy;
}

export interface RestoreReport {
  providersAdded: number;
  providersSkipped: number;
  accountsAdded: number;
  settingsRestored: boolean;
  dirsChanged: boolean;
  scenesAdded?: number;
  scenesSkipped?: number;
  failures: string[];
}

export interface ImportReport {
  created: [ToolId, string][];
  skipped: [ToolId, string, Reason][];
  accountsAdded: string[];
}

export type DuplicatePolicy = "skip" | "rename";

/** 场景里一个工具的选择：单选式工具用 `current`，并存式工具用 `enabled`（Hermes 还可以有 `current`）。 */
export interface SceneTool {
  current?: string;
  enabled?: string[];
  [key: string]: unknown;
}

/** 场景的提醒规则（crates/core/src/scene_rules.rs）。`days`：1 = 周一 … 7 = 周日。 */
export type KnownSceneRule =
  | { kind: "time"; days: number[]; start: string; end: string }
  | { kind: "subnet"; cidr: string }
  | { kind: "host"; host: string };
/** 新版本写的、不认识的规则原样保留。 */
export type SceneRule = KnownSceneRule | { kind: string; [key: string]: unknown };

/** 场景（crates/core/src/scene.rs）。`tools` 的键是工具 id。 */
export interface Scene {
  id: string;
  name: string;
  tools: Record<string, SceneTool>;
  /** 满足任意一条、而场景还没在用时提醒切换；不会自动切换。 */
  rules?: SceneRule[];
  [key: string]: unknown;
}

/** 提醒规则满足时建议切换到的场景。 */
export interface SceneSuggestion {
  id: string;
  name: string;
  rule: KnownSceneRule;
}

export type SceneStatus = "active" | "drifted" | "idle";

export interface SceneView extends Scene {
  status: SceneStatus;
  /** 和场景不一致的工具。 */
  pending: ToolId[];
  /** 场景引用、但已经被删掉的卡。 */
  missing: { tool: ToolId; id: string }[];
  /** 提醒规则满足，建议切换到它。 */
  suggested?: boolean;
}

export interface SceneList {
  scenes: SceneView[];
  /** 最近一次应用的场景 id。 */
  last: string | null;
}

export type SceneAction = { kind: "switch" | "enable" | "disable" | "activate"; id: string };

export interface SceneStep {
  action: SceneAction;
  name: string;
}

export interface ToolPreview {
  tool: ToolId;
  kind: ToolKind;
  /** 单选式工具现在用的卡（名字）。 */
  from: string | null;
  steps: SceneStep[];
  missing: string[];
  error: string | null;
}

export interface ScenePreview {
  scene: Scene;
  tools: ToolPreview[];
}

export interface ToolOutcome {
  tool: ToolId;
  outcome: "applied" | "unchanged" | "failed";
  changed: string[];
  warnings: string[];
  missing: string[];
  error: string | null;
  /** 失败后没能改回原样，可能只改了一部分。 */
  partial?: boolean;
}

/** 应用前的状态，撤销时原样交回去。 */
export interface SceneUndo {
  tools: Record<string, SceneTool>;
  last: string | null;
}

export interface SceneReport {
  results: ToolOutcome[];
  undo: SceneUndo;
  /** 这次改了、但撤销改不回的工具（应用前没有选供应商）。 */
  irreversible?: ToolId[];
}

/** 后端给的中英双语说明。 */
export interface Reason {
  code: string;
  zh: string;
  en: string;
}

/** 按界面语言挑后端双语说明。 */
export function reasonText(reason: Reason, language: string): string {
  return language.startsWith("en") ? reason.en : reason.zh;
}

export interface ExportItem {
  /** cc-switch 的 app_type（不支持的应用也原样给出）。 */
  app: string;
  tool: ToolId | null;
  sourceId: string;
  name: string;
  status: "import" | "skip" | "duplicate";
  targetId: string | null;
  reason: Reason | null;
  notes: Reason[];
}

export interface ExportPreview {
  format: "sql-dump" | "sqlite-db" | "legacy-json";
  schemaVersion: number | null;
  items: ExportItem[];
}

export interface ExportReport {
  created: [ToolId, string, string][];
  skipped: number;
}

/** 一天、一个工具、一个供应商、一个模型的用量（crates/service/src/usage.rs）。 */
export interface UsageRow {
  day: string;
  tool: ToolId;
  /** LeanFlip 的供应商 ID，或客户端自己的供应商名；空字符串表示归不到供应商。 */
  provider: string;
  model: string;
  /** exact：记录里自带供应商；time：按切换记录推算；none：记录开始之前。 */
  attribution: "exact" | "time" | "route" | "none";
  input: number;
  cacheRead: number;
  cacheWrite: number;
  output: number;
  requests: number;
}

/** 切换记录：切换式工具是“当前供应商变成了谁”（provider 为空表示没有选择），并存式是写入 / 移出。 */
export interface SwitchEvent {
  ts: number;
  tool: ToolId;
  provider: string;
  name: string;
  enabled: boolean;
}

export interface SourceStatus {
  tool: ToolId;
  state: "read" | "missing" | "unsupported" | "failed";
  /** 格式在真实数据上验证过。 */
  tested: boolean;
  /** 格式没有公开文档。 */
  experimental: boolean;
  path: string | null;
  retentionDays: number | null;
  detail: string | null;
}

export interface UsageReport {
  rows: UsageRow[];
  events: SwitchEvent[];
  sources: SourceStatus[];
  scannedAt: number;
  coverageStart?: Partial<Record<ToolId, string>>;
}

export const api = {
  getCommonConfig: (tool: ToolId) => invoke<CommonConfig>("get_common_config", { tool }),
  saveCommonConfig: (tool: ToolId, revision: string, config: Record<string, unknown>) => invoke<OpResult>("save_common_config", { tool, revision, config }),
  backupSave: (password: string) => invoke<string | null>("backup_save", { password }),
  backupLoad: (bytes: Uint8Array, password: string) => invoke<BackupPreview>("backup_load", { bytes: Array.from(bytes), password }),
  backupApply: (options: RestoreOptions) => invoke<RestoreReport>("backup_apply", { options }),
  backupDiscard: () => invoke<void>("backup_discard"),
  recoveryNotices: () => invoke<string[]>("get_recovery_notices"),
  overview: () => invoke<ToolSummary[]>("get_overview"),
  listProviders: (tool: ToolId) => invoke<ToolProviders>("list_providers", { tool }),
  /** 配置里不会写入客户端的字段（目前只有 Claude 会过滤）。 */
  claudeOnboardingPath: () => invoke<string>("claude_onboarding_path"),
  completeClaudeOnboarding: (expectedPath: string) => invoke<OpResult>("complete_claude_onboarding", { expectedPath }),
  credentialCommands: (tool: ToolId, settingsConfig: Record<string, unknown>) => invoke<{ commands: Record<string, unknown>; destinations: Record<string, unknown>; fingerprint: string; approved: boolean }>("credential_commands", { tool, settingsConfig }),
  approveCredentialCommands: (tool: ToolId, settingsConfig: Record<string, unknown>, fingerprint: string) => invoke<void>("approve_credential_commands", { tool, settingsConfig, fingerprint }),
  ignoredFields: (tool: ToolId, settingsConfig: Record<string, unknown>) => invoke<string[]>("ignored_fields", { tool, settingsConfig }),
  // 以下三个会访问供应商，只在用户点击时调用。
  probeLatency: (tool: ToolId, settingsConfig: Record<string, unknown>) => invoke<ProbeResult>("probe_latency", { tool, settingsConfig }),
  probeModels: (tool: ToolId, settingsConfig: Record<string, unknown>) => invoke<ProbeResult>("probe_models", { tool, settingsConfig }),
  probeCheck: (tool: ToolId, settingsConfig: Record<string, unknown>, model?: string) => invoke<ProbeResult>("probe_check", { tool, settingsConfig, model: model ?? null }),
  probeEfforts: (tool: ToolId, settingsConfig: Record<string, unknown>, model?: string) => invoke<ProbeResult>("probe_efforts", { tool, settingsConfig, model: model ?? null }),
  /** 扫描各工具新增的会话记录，返回最近 180 天的用量和切换记录。只读数字，不读对话内容。 */
  usageReport: () => invoke<UsageReport>("usage_report"),
  switchProvider: (tool: ToolId, id: string) => invoke<OpResult>("switch_provider", { tool, id }),
  addProvider: (tool: ToolId, provider: Provider, addToLive: boolean) =>
    invoke<OpResult>("add_provider", { tool, provider, addToLive }),
  linkProvider: (tool: ToolId, expected: Provider, link: string) => invoke<OpResult>("link_provider", { tool, expected, link }),
  updateProvider: (tool: ToolId, provider: Provider) => invoke<OpResult>("update_provider", { tool, provider }),
  deleteProvider: (tool: ToolId, id: string) => invoke<void>("delete_provider", { tool, id }),
  setInLive: (tool: ToolId, id: string, enabled: boolean) =>
    invoke<OpResult>("set_in_live", { tool, id, enabled }),
  importLive: (tool: ToolId) => invoke<{ created: string[]; warnings?: string[] }>("import_live", { tool }),
  listScenes: () => invoke<SceneList>("list_scenes"),
  /** 把现在的状态记成一个还没保存的场景；不传 `tools` 时取所有有卡的工具。 */
  captureScene: (tools?: ToolId[]) => invoke<Scene>("capture_scene", { tools: tools ?? null }),
  saveScene: (scene: Scene) => invoke<Scene>("save_scene", { scene }),
  deleteScene: (id: string) => invoke<void>("delete_scene", { id }),
  previewScene: (id: string) => invoke<ScenePreview>("preview_scene", { id }),
  /** 按 id 或名字应用场景；给了 `tools` 时只应用其中的工具。 */
  applyScene: (id: string, tools?: ToolId[]) => invoke<SceneReport>("apply_scene", { id, tools: tools ?? null }),
  undoScene: (undo: SceneUndo) => invoke<SceneReport>("undo_scene", { undo }),
  codexAccounts: () => invoke<AccountInfo[]>("codex_accounts"),
  codexStartLogin: (target?: string) => invoke<DeviceFlow>("codex_start_login", { target: target ?? null }),
  codexPollLogin: (deviceCode: string) => invoke<PollResult>("codex_poll_login", { deviceCode }),
  codexCancelLogin: (deviceCode: string) => invoke<boolean>("codex_cancel_login", { deviceCode }),
  codexSetDefault: (id: string) => invoke<void>("codex_set_default", { id }),
  codexRemoveAccount: (id: string) => invoke<void>("codex_remove_account", { id }),
  codexLogins: () => invoke<LoginSummary[]>("codex_logins"),
  quotaOverview: () => invoke<QuotaView>("quota_overview"),
  /** 联网查询托管账号的额度；不传 `accounts` 查全部。 */
  quotaRefresh: (keys?: string[]) => invoke<QuotaView>("quota_refresh", { keys: keys ?? null }),
  /** 用这份配置里的地址和 Key 识别中转站平台（联网，只在用户点击时调用）。 */
  quotaDetect: (tool: ToolId, settingsConfig: Record<string, unknown>, quotaPerUsd?: number, queryBaseUrl?: string) =>
    invoke<QuotaDetect>("quota_detect", { tool, settingsConfig, quotaPerUsd: quotaPerUsd ?? null, ...(queryBaseUrl ? { queryBaseUrl } : {}) }),
  quotaKrillDetect: (tool: ToolId, jwt?: string, providerId?: string) =>
    invoke<QuotaDetect>("quota_krill_detect", { tool, jwt: jwt ?? null, providerId: providerId ?? null }),
  quotaKrillLogin: (tool: ToolId) => invoke<{ credential: string; detected: QuotaDetect }>("quota_krill_login", { tool }),
  quotaKrillLoginCancel: () => invoke<void>("quota_krill_login_cancel"),
  quotaAccountDetect: (tool: ToolId, settingsConfig: Record<string, unknown>, settings: QuotaSettings, providerId?: string) =>
    invoke<QuotaDetect>("quota_account_detect", { tool, settingsConfig, settings, providerId: providerId ?? null }),
  quotaConfigure: (tool: ToolId, id: string, settings: QuotaSettings | null) => invoke<null>("quota_configure", { tool, id, settings }),
  codexSetLoginRef: (id: string, identity: string | null) =>
    invoke<OpResult>("codex_set_login_ref", { id, identity }),
  getSettings: () => invoke<SettingsView>("get_settings"),
  saveSettings: (app: AppSettings, keepCodexLogin: boolean) =>
    invoke<{ dirsChanged: boolean }>("save_settings", { input: { app, keepCodexLogin } }),
  ccswitchScan: (dir?: string) => invoke<ImportScan>("ccswitch_scan", { dir: dir ?? null }),
  ccswitchApply: (tools: ToolId[], dir?: string, accounts = true) =>
    invoke<ImportReport>("ccswitch_apply", { tools, dir: dir ?? null, accounts }),
  /** 文件原始字节作为请求体发给后端（不经 JSON），后端暂存等确认。 */
  exportLoad: (bytes: Uint8Array, policy: DuplicatePolicy) =>
    invoke<ExportPreview>("ccswitch_export_load", bytes, { headers: { "x-duplicate-policy": policy } }),
  exportPreview: (policy: DuplicatePolicy) => invoke<ExportPreview>("ccswitch_export_preview", { policy }),
  exportApply: (tools: ToolId[], policy: DuplicatePolicy) =>
    invoke<ExportReport>("ccswitch_export_apply", { tools, policy }),
  exportDiscard: () => invoke<void>("ccswitch_export_discard"),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
};

/** Structured backend errors use recovery copy; legacy and local errors stay readable. */
export function errorMessage(error: unknown): string {
  if (error && typeof error === "object" && "code" in error && "message" in error) {
    const key = typeof error.code === "string" ? `errors.${error.code}` : "";
    if (key && i18n.exists(key)) return i18n.t(key);
    if (typeof error.message === "string") return error.message;
  }
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return JSON.stringify(error) ?? i18n.t("errors.unknown");
}

/** Original details are available for support without putting them in the primary toast. */
export function errorDetails(error: unknown): string {
  if (error && typeof error === "object" && "message" in error && typeof error.message === "string") return error.message;
  return typeof error === "string" ? error : error instanceof Error ? error.message : JSON.stringify(error) ?? "";
}
