// 界面外观：`light` / `dark` 强制覆盖，`null` 跟随系统（CSS 里由 prefers-color-scheme 处理）。
import { useSyncExternalStore } from "react";

export type ThemePreference = "light" | "dark" | null;

let preference: ThemePreference = null;
const listeners = new Set<() => void>();
const media = typeof window !== "undefined" && typeof window.matchMedia === "function"
  ? window.matchMedia("(prefers-color-scheme: dark)")
  : null;

export function resolvedTheme(): "light" | "dark" {
  return preference ?? (media?.matches ? "dark" : "light");
}

export function applyTheme(next: ThemePreference | undefined) {
  preference = next ?? null;
  const root = document.documentElement;
  root.classList.toggle("dark", preference === "dark");
  root.classList.toggle("light", preference === "light");
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  media?.addEventListener?.("change", listener);
  return () => {
    listeners.delete(listener);
    media?.removeEventListener?.("change", listener);
  };
}

export function useResolvedTheme(): "light" | "dark" {
  return useSyncExternalStore(subscribe, resolvedTheme, () => "light");
}
