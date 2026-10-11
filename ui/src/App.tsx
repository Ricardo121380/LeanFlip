import { lazy, Suspense, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Download, RotateCcw, Settings, UsersRound } from "lucide-react";
import { useBackendEvents, useOverview, useSettings } from "@/hooks/queries";
import type { ToolId } from "@/lib/api";
import { TOOLS } from "@/lib/tools";
import { cn } from "@/lib/utils";
import { applyLanguage } from "@/i18n";
import { RecoveryNotices } from "@/components/RecoveryNotices";
import { QueryState } from "@/components/QueryState";
import { ToolPage } from "@/pages/ToolPage";

// 不常用的页面按需加载。
const CodexAccountsPage = lazy(() => import("@/pages/CodexAccountsPage").then((m) => ({ default: m.CodexAccountsPage })));
const ImportWizard = lazy(() => import("@/pages/ImportWizard").then((m) => ({ default: m.ImportWizard })));
const SettingsPage = lazy(() => import("@/pages/SettingsPage").then((m) => ({ default: m.SettingsPage })));

export type View = { type: "tool"; tool: ToolId } | { type: "codex" } | { type: "import" } | { type: "settings" };

function initialView(): View {
  // 截图 / 调试：#tool=codex、#codex、#import、#settings
  const hash = typeof location !== "undefined" ? location.hash.slice(1) : "";
  const tool = TOOLS.find((item) => hash === `tool=${item.id}`);
  if (tool) return { type: "tool", tool: tool.id };
  if (hash === "codex" || hash === "import" || hash === "settings") return { type: hash };
  return { type: "tool", tool: "claude" };
}

export default function App() {
  const { t } = useTranslation();
  useBackendEvents();
  const overviewQuery = useOverview();
  const overview = overviewQuery.data ?? [];
  const settings = useSettings().data;
  const [view, setView] = useState<View>(initialView);

  useEffect(() => {
    const navigate = () => setView(initialView());
    window.addEventListener("hashchange", navigate);
    return () => window.removeEventListener("hashchange", navigate);
  }, []);

  useEffect(() => {
    if (settings) applyLanguage(settings.app.language);
  }, [settings]);

  const navButton = (active: boolean) =>
    cn(
      "flex min-h-9 w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-sm transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
      active && "bg-accent font-semibold text-foreground",
    );

  return (
    <div className="flex h-screen overflow-hidden">
      <aside className="flex w-56 shrink-0 flex-col border-r bg-muted/40 p-3">
        <div className="shrink-0 px-2 pb-5 pt-1">
          <p className="font-semibold">{t("app.title")}</p>
          <p className="text-xs text-muted-foreground">{t("app.subtitle")}</p>
        </div>
        <nav aria-label={t("nav.tools")} className="min-h-0 flex-1 space-y-4 overflow-y-auto pb-3">
          {(["switch", "coexist"] as const).map((kind) => (
            <div key={kind}>
              <h2 className="px-2.5 pb-1 text-xs font-medium text-muted-foreground">{t(`nav.${kind}`)}</h2>
              <div className="space-y-0.5">
                {TOOLS.filter((tool) => tool.kind === kind).map((tool) => {
                  const summary = overview.find((s) => s.tool === tool.id);
                  const active = view.type === "tool" && view.tool === tool.id;
                  const sub =
                    tool.kind === "coexist"
                      ? summary && summary.inLive.length > 0
                        ? t("tool.inLiveCount", { count: summary.inLive.length })
                        : ""
                      : (summary?.currentName ?? "");
                  return (
                    <a key={tool.id} className={navButton(active)} href={`#tool=${tool.id}`} aria-current={active ? "page" : undefined}>
                      <span
                        className={cn("h-2 w-2 shrink-0 rounded-full", summary && !summary.detected && "opacity-30")}
                        style={{ background: tool.color }}
                        title={summary && !summary.detected ? t("tool.notDetected") : undefined}
                      />
                      <span className="flex min-w-0 flex-1 flex-col">
                        <span className="truncate">{tool.label}</span>
                        {sub && <span className="truncate text-xs font-normal text-muted-foreground" title={sub}>{sub}</span>}
                      </span>
                      {summary && summary.cardCount > 0 && (
                        <span className="text-xs font-normal tabular-nums text-muted-foreground">{summary.cardCount}</span>
                      )}
                    </a>
                  );
                })}
              </div>
            </div>
          ))}
        </nav>
        {overviewQuery.isError && (
          <div role="alert" className="mb-3 flex shrink-0 items-center gap-2 rounded-md border px-2 py-2 text-xs text-muted-foreground">
            <span className="flex-1">{t("nav.overviewError")}</span>
            <button type="button" aria-label={t("common.retry")} disabled={overviewQuery.isFetching} onClick={() => void overviewQuery.refetch()} className="flex size-8 shrink-0 items-center justify-center rounded hover:bg-accent disabled:opacity-50">
              <RotateCcw className="size-4" />
            </button>
          </div>
        )}
        <nav aria-label={t("nav.manage")} className="flex shrink-0 flex-col gap-0.5 border-t pt-2">
          <a className={navButton(view.type === "codex")} href="#codex" aria-current={view.type === "codex" ? "page" : undefined}>
            <UsersRound className="h-4 w-4" /> {t("nav.codexAccounts")}
          </a>
          <a className={navButton(view.type === "import")} href="#import" aria-current={view.type === "import" ? "page" : undefined}>
            <Download className="h-4 w-4" /> {t("nav.import")}
          </a>
          <a className={navButton(view.type === "settings")} href="#settings" aria-current={view.type === "settings" ? "page" : undefined}>
            <Settings className="h-4 w-4" /> {t("nav.settings")}
          </a>
        </nav>
      </aside>
      <main className="min-w-0 flex-1 overflow-y-auto">
        <RecoveryNotices />
        <Suspense fallback={<div className="p-6"><QueryState state="loading" title={t("common.loading")} /></div>}>
          {view.type === "tool" && <ToolPage key={view.tool} tool={view.tool} />}
          {view.type === "codex" && <CodexAccountsPage />}
          {view.type === "import" && <ImportWizard />}
          {view.type === "settings" && <SettingsPage />}
        </Suspense>
      </main>
    </div>
  );
}
