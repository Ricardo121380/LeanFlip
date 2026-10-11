import { lazy, Suspense, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Download, Settings, UsersRound } from "lucide-react";
import { useBackendEvents, useOverview, useSettings } from "@/hooks/queries";
import type { ToolId } from "@/lib/api";
import { TOOLS } from "@/lib/tools";
import { cn } from "@/lib/utils";
import { applyLanguage } from "@/i18n";
import { RecoveryNotices } from "@/components/RecoveryNotices";
import { ToolPage } from "@/pages/ToolPage";

// 不常用的页面按需加载。
const CodexAccountsPage = lazy(() => import("@/pages/CodexAccountsPage").then((m) => ({ default: m.CodexAccountsPage })));
const ImportWizard = lazy(() => import("@/pages/ImportWizard").then((m) => ({ default: m.ImportWizard })));
const SettingsPage = lazy(() => import("@/pages/SettingsPage").then((m) => ({ default: m.SettingsPage })));

export type View = { type: "tool"; tool: ToolId } | { type: "codex" } | { type: "import" } | { type: "settings" };

function initialView(): View {
  // 截图 / 调试：#tool=codex、#codex、#import、#settings
  const hash = typeof location !== "undefined" ? location.hash.slice(1) : "";
  if (hash.startsWith("tool=")) return { type: "tool", tool: hash.slice(5) as ToolId };
  if (hash === "codex" || hash === "import" || hash === "settings") return { type: hash };
  return { type: "tool", tool: "claude" };
}

export default function App() {
  const { t } = useTranslation();
  useBackendEvents();
  const overview = useOverview().data ?? [];
  const settings = useSettings().data;
  const [view, setView] = useState<View>(initialView);

  useEffect(() => {
    if (settings) applyLanguage(settings.app.language);
  }, [settings]);

  const navButton = (active: boolean) =>
    cn(
      "flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition-colors hover:bg-accent",
      active && "bg-accent font-medium",
    );

  return (
    <div className="flex h-screen overflow-hidden">
      <aside className="flex w-60 shrink-0 flex-col gap-1 border-r bg-muted/40 p-3">
        <div className="px-2 pb-3 pt-1">
          <p className="font-semibold">{t("app.title")}</p>
          <p className="text-xs text-muted-foreground">{t("app.subtitle")}</p>
        </div>
        <p className="px-2 pb-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">{t("nav.tools")}</p>
        <nav className="flex flex-col gap-0.5 overflow-y-auto">
          {TOOLS.map((tool) => {
            const summary = overview.find((s) => s.tool === tool.id);
            const active = view.type === "tool" && view.tool === tool.id;
            const sub =
              tool.kind === "coexist"
                ? summary && summary.inLive.length > 0
                  ? t("tool.inLiveCount", { count: summary.inLive.length })
                  : ""
                : (summary?.currentName ?? "");
            return (
              <button key={tool.id} className={navButton(active)} onClick={() => setView({ type: "tool", tool: tool.id })}>
                <span
                  className={cn("h-2 w-2 shrink-0 rounded-full", !summary?.detected && "opacity-30")}
                  style={{ background: tool.color }}
                  title={summary?.detected ? undefined : t("tool.notDetected")}
                />
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate">{tool.label}</span>
                  {sub && <span className="truncate text-[11px] text-muted-foreground">{sub}</span>}
                </span>
                {summary && summary.cardCount > 0 && (
                  <span className="text-[11px] text-muted-foreground">{summary.cardCount}</span>
                )}
              </button>
            );
          })}
        </nav>
        <div className="mt-auto flex flex-col gap-0.5 border-t pt-2">
          <button className={navButton(view.type === "codex")} onClick={() => setView({ type: "codex" })}>
            <UsersRound className="h-4 w-4" /> {t("nav.codexAccounts")}
          </button>
          <button className={navButton(view.type === "import")} onClick={() => setView({ type: "import" })}>
            <Download className="h-4 w-4" /> {t("nav.import")}
          </button>
          <button className={navButton(view.type === "settings")} onClick={() => setView({ type: "settings" })}>
            <Settings className="h-4 w-4" /> {t("nav.settings")}
          </button>
        </div>
      </aside>
      <main className="min-w-0 flex-1 overflow-y-auto">
        <RecoveryNotices />
        <Suspense fallback={<p className="p-8 text-sm text-muted-foreground">{t("common.loading")}</p>}>
          {view.type === "tool" && <ToolPage key={view.tool} tool={view.tool} />}
          {view.type === "codex" && <CodexAccountsPage />}
          {view.type === "import" && <ImportWizard />}
          {view.type === "settings" && <SettingsPage />}
        </Suspense>
      </main>
    </div>
  );
}
