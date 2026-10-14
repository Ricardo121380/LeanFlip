// 后端命令的类型化封装（命令定义见 crates/app/src/commands.rs）。
import { invoke } from "@tauri-apps/api/core";

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
  items: { tool: ToolId; id: string; name: string; duplicate: boolean }[];
  accounts: number;
  existingAccounts: number;
  hasSettings: boolean;
  hasPaths: boolean;
}

export interface RestoreOptions {
  tools: ToolId[];
  accounts: boolean;
  settings: boolean;
  paths: boolean;
  duplicate: DuplicatePolicy;
}

export interface RestoreReport {
  providersAdded: number;
  providersSkipped: number;
  accountsAdded: number;
  settingsRestored: boolean;
  dirsChanged: boolean;
  failures: string[];
}

export interface ImportReport {
  created: [ToolId, string][];
  skipped: [ToolId, string, Reason][];
  accountsAdded: string[];
}

export type DuplicatePolicy = "skip" | "rename";

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
  backupSave: (password: string) => invoke<string | null>("backup_save", { password }),
  backupLoad: (bytes: Uint8Array, password: string) => invoke<BackupPreview>("backup_load", { bytes: Array.from(bytes), password }),
  backupApply: (options: RestoreOptions) => invoke<RestoreReport>("backup_apply", { options }),
  backupDiscard: () => invoke<void>("backup_discard"),
  recoveryNotices: () => invoke<string[]>("get_recovery_notices"),
  overview: () => invoke<ToolSummary[]>("get_overview"),
  listProviders: (tool: ToolId) => invoke<ToolProviders>("list_providers", { tool }),
  /** 配置里不会写入客户端的字段（目前只有 Claude 会过滤）。 */
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
  updateProvider: (tool: ToolId, provider: Provider) => invoke<OpResult>("update_provider", { tool, provider }),
  deleteProvider: (tool: ToolId, id: string) => invoke<void>("delete_provider", { tool, id }),
  setInLive: (tool: ToolId, id: string, enabled: boolean) =>
    invoke<OpResult>("set_in_live", { tool, id, enabled }),
  importLive: (tool: ToolId) => invoke<{ created: string[] }>("import_live", { tool }),
  codexAccounts: () => invoke<AccountInfo[]>("codex_accounts"),
  codexStartLogin: (target?: string) => invoke<DeviceFlow>("codex_start_login", { target: target ?? null }),
  codexPollLogin: (deviceCode: string) => invoke<PollResult>("codex_poll_login", { deviceCode }),
  codexCancelLogin: (deviceCode: string) => invoke<boolean>("codex_cancel_login", { deviceCode }),
  codexSetDefault: (id: string) => invoke<void>("codex_set_default", { id }),
  codexRemoveAccount: (id: string) => invoke<void>("codex_remove_account", { id }),
  codexLogins: () => invoke<LoginSummary[]>("codex_logins"),
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

/** 后端错误是一个字符串（`ApiError`）。 */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return JSON.stringify(error);
}
