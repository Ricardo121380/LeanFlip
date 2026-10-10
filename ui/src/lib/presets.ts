// cc-switch 的预设（MIT，Copyright (c) 2025 Jason Young），由 scripts/build-presets.mjs
// 裁剪生成：去掉了需要本地路由 / 专用登录的条目。
import type { ToolId } from "./api";
import claude from "@/presets/claude.json";
import claudeDesktop from "@/presets/claude-desktop.json";
import codex from "@/presets/codex.json";
import gemini from "@/presets/gemini.json";
import grokbuild from "@/presets/grokbuild.json";
import hermes from "@/presets/hermes.json";
import mcode from "@/presets/mcode.json";
import openclaw from "@/presets/openclaw.json";
import opencode from "@/presets/opencode.json";
import pi from "@/presets/pi.json";

export interface Preset {
  name: string;
  websiteUrl?: string;
  apiKeyUrl?: string;
  settingsConfig: Record<string, unknown>;
  isOfficial?: boolean;
  category?: string;
  apiKeyField?: string;
}

const PRESETS: Record<ToolId, Preset[]> = {
  claude: claude as Preset[],
  "claude-desktop": claudeDesktop as Preset[],
  codex: codex as Preset[],
  gemini: gemini as Preset[],
  grokbuild: grokbuild as Preset[],
  hermes: hermes as Preset[],
  mcode: mcode as Preset[],
  openclaw: openclaw as Preset[],
  opencode: opencode as Preset[],
  pi: pi as Preset[],
};

export function presetsFor(tool: ToolId): Preset[] {
  return PRESETS[tool] ?? [];
}

export function searchPresets(tool: ToolId, query: string): Preset[] {
  const q = query.trim().toLowerCase();
  const list = presetsFor(tool);
  if (!q) return list;
  return list.filter(
    (p) => p.name.toLowerCase().includes(q) || (p.websiteUrl ?? "").toLowerCase().includes(q),
  );
}
