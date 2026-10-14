import { lazy, Suspense, useCallback, useEffect, useRef, useState, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import { ChartColumn, Download, House, RotateCcw, Search, Settings, UsersRound } from "lucide-react";
import { toast } from "sonner";
import { useBackendEvents, useOverview, useSettings } from "@/hooks/queries";
import type { ToolId } from "@/lib/api";
import { TOOLS } from "@/lib/tools";
import { cn } from "@/lib/utils";
import { applyLanguage } from "@/i18n";
import { applyTheme } from "@/lib/theme";
import { CommandPalette } from "@/components/CommandPalette";
import { ToolGlyph } from "@/components/Glyph";
import { Morph } from "@/components/Morph";
import { RecoveryNotices } from "@/components/RecoveryNotices";
import { QueryState } from "@/components/QueryState";
import { useConfirm } from "@/components/ConfirmDialog";
import { HomePage, type HomeMode } from "@/pages/HomePage";
import { ToolPage } from "@/pages/ToolPage";
import { withViewTransition } from "@/lib/motion";
import type { SettingsLeaveState } from "@/pages/SettingsPage";
import { EditorNavigationContext, type EditorLeaveState } from "@/lib/navigation";
import { BrandMark } from "@/components/BrandMark";

// 不常用的页面按需加载。
const CodexAccountsPage = lazy(() => import("@/pages/CodexAccountsPage").then((m) => ({ default: m.CodexAccountsPage })));
const ImportWizard = lazy(() => import("@/pages/ImportWizard").then((m) => ({ default: m.ImportWizard })));
const SettingsPage = lazy(() => import("@/pages/SettingsPage").then((m) => ({ default: m.SettingsPage })));
const StatsPage = lazy(() => import("@/pages/StatsPage").then((m) => ({ default: m.StatsPage })));

export type View = { type: "home"; mode: HomeMode } | { type: "stats" } | { type: "tool"; tool: ToolId } | { type: "codex" } | { type: "import" } | { type: "settings" };

function viewFromHash(hash: string): View {
  // #home（默认）、#map、#stats、#tool=codex、#codex、#import、#settings
  const value = hash.replace(/^#/, "");
  const tool = TOOLS.find((item) => value === `tool=${item.id}`);
  if (tool) return { type: "tool", tool: tool.id };
  if (value === "codex" || value === "import" || value === "settings" || value === "stats") return { type: value };
  return { type: "home", mode: value === "map" ? "map" : "list" };
}

const shortcutLabel = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘K" : "Ctrl K";

const HISTORY_INDEX = "leanflipNavigationIndex";
type NavigationEntry = { hash: string; index: number; managed: boolean };
type NavigationRequest = { kind: "link"; hash: string } | { kind: "history" };

function historyIndex(): number | null {
  const value = window.history.state?.[HISTORY_INDEX];
  return Number.isInteger(value) ? value : null;
}

export default function App() {
  const { t } = useTranslation();
  useBackendEvents();
  const overviewQuery = useOverview();
  const overview = overviewQuery.data ?? [];
  const settings = useSettings().data;
  const [view, setView] = useState<View>(() => viewFromHash(window.location.hash));
  const [confirm, confirmDialog] = useConfirm();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const main = useRef<HTMLElement>(null);
  const accepted = useRef<NavigationEntry>({ hash: window.location.hash, index: historyIndex() ?? 0, managed: true });
  const observed = useRef(accepted.current);
  const pending = useRef<{ request: NavigationRequest; resolve: (allowed: boolean) => void } | null>(null);
  const editorGuards = useRef(new Set<{ current: EditorLeaveState }>());
  const restoring = useRef(false);
  const settingsLeave = useRef<SettingsLeaveState>({ dirty: false, saving: false });
  const importBusy = useRef(false);
  const onImportBusyChange = useCallback((busy: boolean) => { importBusy.current = busy; }, []);
  const onSettingsLeaveState = useCallback((state: SettingsLeaveState) => { settingsLeave.current = state; }, []);

  useEffect(() => {
    // Keep our existing history entries intact when a guarded Back / Forward is cancelled.
    window.history.replaceState({ ...window.history.state, [HISTORY_INDEX]: accepted.current.index }, "");
  }, []);

  const acceptNavigation = useCallback((entry: NavigationEntry) => {
    accepted.current = entry;
    observed.current = entry;
    setView(viewFromHash(entry.hash));
    if (main.current) main.current.scrollTop = 0;
  }, []);

  const restoreAccepted = useCallback(() => {
    const destination = accepted.current;
    const delta = destination.index - observed.current.index;
    if (observed.current.managed && delta !== 0) {
      restoring.current = true;
      window.history.go(delta);
    } else {
      // A manually changed hash has no reliable traversal index.
      window.history.replaceState({ ...window.history.state, [HISTORY_INDEX]: destination.index }, "", destination.hash || window.location.pathname + window.location.search);
      observed.current = destination;
      restoring.current = false;
    }
  }, []);

  const requestNavigation = useCallback((request: NavigationRequest): Promise<boolean> => {
    if (restoring.current) return Promise.resolve(false);
    return new Promise<boolean>((resolve) => {
      const alreadyPending = pending.current !== null;
      pending.current?.resolve(false);
      pending.current = { request, resolve };
      if (alreadyPending) return;
      void (async () => {
        const busy = () => settingsLeave.current.saving || importBusy.current || [...editorGuards.current].some((g) => g.current.saving);
        const editorDirty = [...editorGuards.current].some((g) => g.current.dirty);
        let allowed = true;
        if (busy()) {
          toast.info(t("nav.waitForOperation"), { id: "pending-operation-navigation" });
          allowed = false;
        } else if (settingsLeave.current.dirty || editorDirty) {
          allowed = await confirm({
            title: t(editorDirty ? "editor.discardTitle" : "settings.leaveTitle"),
            description: t(editorDirty ? "editor.discardHint" : "settings.leaveHint"),
            confirmLabel: t(editorDirty ? "editor.discardAction" : "settings.leaveAction"),
            destructive: true,
          });
        }
        const next = pending.current;
        pending.current = null;
        if (!allowed || busy()) {
          restoreAccepted();
          next?.resolve(false);
          return;
        }
        for (const guard of editorGuards.current) guard.current.discard();
        if (next?.request.kind === "link") {
          const hash = next.request.hash;
          if (hash === window.location.hash) acceptNavigation(observed.current);
          else {
            const index = observed.current.index + 1;
            window.history.pushState({ ...window.history.state, [HISTORY_INDEX]: index }, "", hash);
            acceptNavigation({ hash, index, managed: true });
          }
        } else acceptNavigation(observed.current);
        next?.resolve(true);
      })();
    });
  }, [acceptNavigation, confirm, restoreAccepted, t]);

  useEffect(() => {
    const navigate = () => {
      const hash = window.location.hash;
      const index = historyIndex();
      // A traversal can emit both events; same-hash traversals emit only popstate.
      if (hash === observed.current.hash && index === observed.current.index) return;
      const managed = index !== null && (index !== observed.current.index || hash === observed.current.hash);
      const entry = { hash, index: managed ? index! : observed.current.index + 1, managed };
      if (!managed) window.history.replaceState({ ...window.history.state, [HISTORY_INDEX]: entry.index }, "");
      observed.current = entry;
      if (restoring.current) {
        if (entry.hash === accepted.current.hash && entry.index === accepted.current.index) restoring.current = false;
        else restoreAccepted();
        return;
      }
      if (entry.hash !== accepted.current.hash) void requestNavigation({ kind: "history" });
      else if (!pending.current) accepted.current = entry;
    };
    window.addEventListener("popstate", navigate);
    window.addEventListener("hashchange", navigate);
    return () => {
      window.removeEventListener("popstate", navigate);
      window.removeEventListener("hashchange", navigate);
    };
  }, [requestNavigation, restoreAccepted]);

  const onNavigationClick = (event: MouseEvent<HTMLAnchorElement>) => {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    void requestNavigation({ kind: "link", hash: event.currentTarget.hash });
  };

  useEffect(() => {
    if (!settings) return;
    applyLanguage(settings.app.language);
    applyTheme(settings.app.theme);
  }, [settings]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && !event.altKey && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const navButton = (active: boolean) =>
    cn(
      "flex min-h-8 w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left text-[13px] text-foreground/80 transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
      active && "bg-card font-medium text-foreground shadow-[0_0_0_1px_hsl(var(--border)),0_1px_2px_hsl(var(--overlay)/0.06)] hover:bg-card",
    );

  return (
    <EditorNavigationContext.Provider value={editorGuards.current}>
    <div className="flex h-screen overflow-hidden">
      <aside className="flex w-60 shrink-0 flex-col border-r bg-sidebar px-2.5 pb-2.5 pt-3">
        <div className="flex shrink-0 items-center gap-2.5 px-2 pb-4 pt-1">
          <BrandMark className="h-7 w-7 shrink-0" />
          <div className="min-w-0">
            <p className="text-[15px] font-semibold leading-tight tracking-tight">{t("app.title")}</p>
            <p className="truncate text-[11px] text-muted-foreground">{t("app.subtitle")}</p>
          </div>
        </div>
        <button
          type="button"
          onClick={() => setPaletteOpen(true)}
          aria-label={t("nav.searchLabel")}
          className="mx-0.5 mb-4 flex h-8 shrink-0 items-center gap-2 rounded-md border bg-card px-2.5 text-[13px] text-muted-foreground transition-colors hover:border-input hover:text-foreground"
        >
          <Search className="h-3.5 w-3.5" aria-hidden="true" />
          <span className="flex-1 text-left">{t("nav.search")}</span>
          <kbd className="rounded border bg-muted px-1.5 font-mono text-[10px] leading-4">{shortcutLabel}</kbd>
        </button>
        <div className="mb-3 flex shrink-0 flex-col gap-px">
          <a className={navButton(view.type === "home")} href="#home" onClick={onNavigationClick} aria-current={view.type === "home" ? "page" : undefined}>
            <House className="h-4 w-4 text-muted-foreground" /> {t("nav.home")}
          </a>
          <a className={navButton(view.type === "stats")} href="#stats" onClick={onNavigationClick} aria-current={view.type === "stats" ? "page" : undefined}>
            <ChartColumn className="h-4 w-4 text-muted-foreground" /> {t("nav.stats")}
          </a>
        </div>
        <nav aria-label={t("nav.tools")} className="min-h-0 flex-1 space-y-4 overflow-y-auto pb-3">
          {(["switch", "coexist"] as const).map((kind) => (
            <div key={kind}>
              <h2 className="flex items-baseline justify-between px-2.5 pb-1 text-[11px] font-medium text-muted-foreground">
                <span>{t(`nav.${kind}`)}</span>
                <span className="font-normal">{t(`nav.${kind}Hint`)}</span>
              </h2>
              <div className="space-y-px">
                {TOOLS.filter((tool) => tool.kind === kind).map((tool) => {
                  const summary = overview.find((s) => s.tool === tool.id);
                  const active = view.type === "tool" && view.tool === tool.id;
                  const missing = summary ? !summary.detected && summary.cardCount === 0 : false;
                  const sub = missing
                    ? t("tool.notDetected")
                    : tool.kind === "coexist"
                      ? summary && summary.inLive.length > 0
                        ? t("tool.inLiveCount", { count: summary.inLive.length })
                        : ""
                      : (summary?.currentName ?? "");
                  // 切换式工具的当前供应商写成“→ 名称”：这个工具现在路由到哪里。
                  const routed = !missing && tool.kind === "switch" && Boolean(summary?.currentName);
                  return (
                    <a key={tool.id} className={navButton(active)} href={`#tool=${tool.id}`} onClick={onNavigationClick} aria-current={active ? "page" : undefined}>
                      <ToolGlyph tool={tool} size={24} dim={missing} />
                      <span className="flex min-w-0 flex-1 flex-col">
                        <span className={cn("truncate leading-5", missing && "text-muted-foreground")}>{tool.label}</span>
                        {sub && (
                          <span className="flex min-w-0 items-center gap-1 text-[11px] font-normal leading-4 text-muted-foreground" title={sub}>
                            {routed && <span aria-hidden="true" className="text-primary/70">→</span>}
                            <Morph text={sub} className="min-w-0" />
                          </span>
                        )}
                      </span>
                      {summary && summary.cardCount > 0 && (
                        <span className="text-[11px] font-normal tabular-nums text-muted-foreground">{summary.cardCount}</span>
                      )}
                    </a>
                  );
                })}
              </div>
            </div>
          ))}
        </nav>
        {overviewQuery.isError && (
          <div role="alert" className="mb-3 flex shrink-0 items-center gap-2 rounded-md border border-warning/30 bg-warning/[0.07] px-2 py-2 text-xs text-muted-foreground">
            <span className="flex-1">{t("nav.overviewError")}</span>
            <button type="button" aria-label={t("common.retry")} disabled={overviewQuery.isFetching} onClick={() => void overviewQuery.refetch()} className="flex size-7 shrink-0 items-center justify-center rounded hover:bg-accent disabled:opacity-50">
              <RotateCcw className="size-4" />
            </button>
          </div>
        )}
        <nav aria-label={t("nav.manage")} className="flex shrink-0 flex-col gap-px border-t pt-2">
          <a className={navButton(view.type === "codex")} href="#codex" onClick={onNavigationClick} aria-current={view.type === "codex" ? "page" : undefined}>
            <UsersRound className="h-4 w-4 text-muted-foreground" /> {t("nav.codexAccounts")}
          </a>
          <a className={navButton(view.type === "import")} href="#import" onClick={onNavigationClick} aria-current={view.type === "import" ? "page" : undefined}>
            <Download className="h-4 w-4 text-muted-foreground" /> {t("nav.import")}
          </a>
          <a className={navButton(view.type === "settings")} href="#settings" onClick={onNavigationClick} aria-current={view.type === "settings" ? "page" : undefined}>
            <Settings className="h-4 w-4 text-muted-foreground" /> {t("nav.settings")}
          </a>
        </nav>
      </aside>
      <main ref={main} className="min-w-0 flex-1 overflow-y-auto">
        <RecoveryNotices />
        <Suspense fallback={<div className="p-6"><QueryState state="loading" title={t("common.loading")} /></div>}>
          {view.type === "home" && (
            <HomePage
              mode={view.mode}
              onNavigate={(hash, options) => {
                const go = () => void requestNavigation({ kind: "link", hash });
                if (options?.transition) withViewTransition(go);
                else go();
              }}
            />
          )}
          {view.type === "stats" && <StatsPage />}
          {view.type === "tool" && <ToolPage key={view.tool} tool={view.tool} />}
          {view.type === "codex" && <CodexAccountsPage />}
          {view.type === "import" && <ImportWizard onBusyChange={onImportBusyChange} />}
          {view.type === "settings" && <SettingsPage onLeaveStateChange={onSettingsLeaveState} />}
        </Suspense>
      </main>
      <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} onNavigate={(hash) => requestNavigation({ kind: "link", hash })} />
      {confirmDialog}
    </div>
    </EditorNavigationContext.Provider>
  );
}
