// 工具和供应商的字母标识：用颜色区分，又不抢正文的注意力。
import type { CSSProperties } from "react";
import type { ToolMeta } from "@/lib/tools";
import { cn } from "@/lib/utils";

/** 供应商没有自己的图标，按名称稳定地取一个低饱和的颜色。 */
const HUES = ["#5B6BA8", "#3E8E7E", "#B07A3A", "#8A5DA8", "#3F7FB0", "#A85D6B", "#6B8E3E", "#7A7F8E"];

/** 供应商的颜色：首页的名字、下划线和光路都用它。 */
export function providerColor(name: string) {
  let hash = 0;
  for (const ch of name) hash = (hash * 31 + ch.codePointAt(0)!) >>> 0;
  return HUES[hash % HUES.length];
}

function initial(name: string) {
  const first = Array.from(name.trim())[0] ?? "?";
  return first.toUpperCase();
}

const box = (size: number): CSSProperties => ({ width: size, height: size, fontSize: Math.round(size * 0.42), borderRadius: Math.round(size * 0.28) });

/** `shared` 给首页两个视图之间的过渡用：同一个工具的标识从旧位置滑到新位置。 */
export function ToolGlyph({ tool, size = 22, dim = false, shared = false, className }: { tool: ToolMeta; size?: number; dim?: boolean; shared?: boolean; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn("glyph", dim && "glyph-dim", className)}
      style={{ ...box(size), ["--glyph" as string]: tool.color, ...(shared ? { ["viewTransitionName" as string]: `tool-${tool.id}` } : {}) }}
    >
      {tool.glyph}
    </span>
  );
}

export function Monogram({ name, size = 28, className }: { name: string; size?: number; className?: string }) {
  return (
    <span aria-hidden="true" className={cn("glyph", className)} style={{ ...box(size), ["--glyph" as string]: providerColor(name) }}>
      {initial(name)}
    </span>
  );
}
