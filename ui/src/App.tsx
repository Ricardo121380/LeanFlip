import { lazy, Suspense, useCallback, useEffect, useRef, useState, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import { Download, RotateCcw, Settings, UsersRound } from "lucide-react";
import { toast } from "sonner";
import { useBackendEvents, useOverview, useSettings } from "@/hooks/queries";
import type { ToolId } from "@/lib/api";
import { TOOLS } from "@/lib/tools";
import { cn } from "@/lib/utils";
import { applyLanguage } from "@/i18n";
import { RecoveryNotices } from "@/components/RecoveryNotices";
import { QueryState } from "@/components/QueryState";
import { useConfirm } from "@/components/ConfirmDialog";
import { ToolPage } from "@/pages/ToolPage";
import type { SettingsLeaveState } from "@/pages/SettingsPage";

// 不常用的页面按需加载。
const CodexAccountsPage = lazy(() => import("@/pages/CodexAccountsPage").then((m) => ({ default: m.CodexAccountsPage })));
const ImportWizard = lazy(() => import("@/pages/ImportWizard").then((m) => ({ default: m.ImportWizard })));
const SettingsPage = lazy(() => import("@/pages/SettingsPage").then((m) => ({ default: m.SettingsPage })));

export type View = { type: "tool"; tool: ToolId } | { type: "codex" } | { type: "import" } | { type: "settings" };

function viewFromHash(hash: string): View {
  // 截图 / 调试：#tool=codex、#codex、#import、#settings
  const value = hash.replace(/^#/, "");
  const tool = TOOLS.find((item) => value === `tool=${item.id}`);
  if (tool) return { type: "tool", tool: tool.id };
  if (value === "codex" || value === "import" || value === "settings") return { type: value };
  return { type: "tool", tool: "claude" };
}

const HISTORY_INDEX = "ccslNavigationIndex";
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
  const main = useRef<HTMLElement>(null);
  const accepted = useRef<NavigationEntry>({ hash: window.location.hash, index: historyIndex() ?? 0, managed: true });
  const observed = useRef(accepted.current);
  const pending = useRef<NavigationRequest | null>(null);
  const restoring = useRef(false);
  const settingsLeave = useRef<SettingsLeaveState>({ dirty: false, saving: false });
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

  const requestNavigation = useCallback(async (request: NavigationRequest) => {
    if (restoring.current) return;
    if (request.kind === "link" && request.hash === accepted.current.hash && !pending.current) return;
    // One confirmation covers the latest destination if history changes while it is open.
    const alreadyPending = pending.current !== null;
    pending.current = request;
    if (alreadyPending) return;

    let allowed = true;
    if (settingsLeave.current.saving) {
      toast.info(t("settings.waitForSave"), { id: "settings-save-navigation" });
      allowed = false;
    } else if (settingsLeave.current.dirty) {
      allowed = await confirm({
        title: t("settings.leaveTitle"),
        description: t("settings.leaveHint"),
        confirmLabel: t("settings.leaveAction"),
        destructive: true,
      });
    }

    const next = pending.current;
    pending.current = null;
    if (!allowed || settingsLeave.current.saving) {
      restoreAccepted();
      return;
    }
    if (next?.kind === "link") {
      if (next.hash === window.location.hash) acceptNavigation(observed.current);
      else {
        const index = observed.current.index + 1;
        window.history.pushState({ ...window.history.state, [HISTORY_INDEX]: index }, "", next.hash);
        acceptNavigation({ hash: next.hash, index, managed: true });
      }
    } else {
      acceptNavigation(observed.current);
    }
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
                    <a key={tool.id} className={navButton(active)} href={`#tool=${tool.id}`} onClick={onNavigationClick} aria-current={active ? "page" : undefined}>
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
          <a className={navButton(view.type === "codex")} href="#codex" onClick={onNavigationClick} aria-current={view.type === "codex" ? "page" : undefined}>
            <UsersRound className="h-4 w-4" /> {t("nav.codexAccounts")}
          </a>
          <a className={navButton(view.type === "import")} href="#import" onClick={onNavigationClick} aria-current={view.type === "import" ? "page" : undefined}>
            <Download className="h-4 w-4" /> {t("nav.import")}
          </a>
          <a className={navButton(view.type === "settings")} href="#settings" onClick={onNavigationClick} aria-current={view.type === "settings" ? "page" : undefined}>
            <Settings className="h-4 w-4" /> {t("nav.settings")}
          </a>
        </nav>
      </aside>
      <main ref={main} className="min-w-0 flex-1 overflow-y-auto">
        <RecoveryNotices />
        <Suspense fallback={<div className="p-6"><QueryState state="loading" title={t("common.loading")} /></div>}>
          {view.type === "tool" && <ToolPage key={view.tool} tool={view.tool} />}
          {view.type === "codex" && <CodexAccountsPage />}
          {view.type === "import" && <ImportWizard />}
          {view.type === "settings" && <SettingsPage onLeaveStateChange={onSettingsLeaveState} />}
        </Suspense>
      </main>
      {confirmDialog}
    </div>
  );
}
