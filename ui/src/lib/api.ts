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

export interface ToolSummary {
  tool: ToolId;
  kind: ToolKind;
  paths: string[];
  detected: boolean;
  cardCount: number;
  current: string | null;
  currentName: string | null;
  inLive: string[];
}

export interface ToolProviders {
  tool: ToolId;
  kind: ToolKind;
  providers: Provider[];
  current: string | null;
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
  skipReason: string | null;
}

export interface ImportScan {
  source: string | null;
  candidates: ImportCandidate[];
  accounts: number;
}

export interface ImportReport {
  created: [ToolId, string][];
  skipped: [ToolId, string, string][];
  accountsAdded: string[];
}

export type DuplicatePolicy = "skip" | "rename";

/** 后端给的中英双语说明。 */
export interface Reason {
  code: string;
  zh: string;
  en: string;
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

export const api = {
  overview: () => invoke<ToolSummary[]>("get_overview"),
  listProviders: (tool: ToolId) => invoke<ToolProviders>("list_providers", { tool }),
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
  ccswitchApply: (tools: ToolId[], dir?: string) =>
    invoke<ImportReport>("ccswitch_apply", { tools, dir: dir ?? null }),
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
