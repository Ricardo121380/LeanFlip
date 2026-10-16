// 场景的入口和应用流程。首页下拉、命令面板都通过 `useSceneActions()` 打开同一个确认框和管理框。
// 应用前先预览（只读），确认后一次应用；全部成功时提示里可以撤销，有失败时留在框里列出结果。
// 托盘里应用的场景通过 `scene-applied` 事件回到这里，用同样的提示和结果框。
// 提醒规则满足时后端发 `scene-suggested`，这里提示一次，点「预览」进入确认框。
// 窗口在前台时 ⌘⇧1…9（其他系统 Ctrl+Shift）直接应用列表里对应的场景，之后可以撤销。
import { createContext, lazy, Suspense, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { AlertTriangle, Check, Loader2, Minus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ToolGlyph } from "@/components/Glyph";
import { keys, refreshAll, useScenes } from "@/hooks/queries";
import { api, errorMessage, type SceneReport, type SceneSuggestion, type SceneUndo, type SceneView, type ToolId, type ToolOutcome, type ToolPreview } from "@/lib/api";
import { applicable, ruleText, shortcutIndex, splitPreview } from "@/lib/scenes";
import { toolMeta } from "@/lib/tools";
import { cn } from "@/lib/utils";

const SceneManager = lazy(() => import("@/components/SceneManager").then((m) => ({ default: m.SceneManager })));

export interface SceneActions {
  /** 打开应用确认框。 */
  apply: (id: string) => void;
  /** 打开管理框；`create` 时直接从当前状态新建。 */
  manage: (options?: { create?: boolean }) => void;
}

const SceneContext = createContext<SceneActions>({ apply: () => {}, manage: () => {} });

export function useSceneActions() {
  return useContext(SceneContext);
}

function toolLabel(tool: ToolId) {
  return toolMeta(tool).label;
}

/** 失败的工具名，提示里用。 */
function failedTools(report: SceneReport) {
  return report.results.filter((r) => r.outcome === "failed").map((r) => toolLabel(r.tool));
}

function showResultWarnings(report: SceneReport) {
  for (const result of report.results) for (const warning of result.warnings) toast.warning(`${toolLabel(result.tool)}: ${warning}`);
}

/** 撤销一次场景切换（把应用前的快照再应用一次）。 */
export function useUndoScene() {
  const { t } = useTranslation();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (undo: SceneUndo) => api.undoScene(undo),
    onSuccess: (report) => {
      showResultWarnings(report);
      const failed = failedTools(report);
      if (failed.length) toast.error(t("scene.undoFailed", { tools: failed.join(t("common.separator")) }));
      else toast.success(t("scene.undone"));
    },
    onError: (error) => toast.error(errorMessage(error)),
    onSettled: () => refreshAll(client),
  });
}

/** 托盘应用场景后发来的结果（crates/app/src/tray.rs 的 `SceneEvent`）。 */
interface SceneEvent {
  id: string;
  name: string;
  report: SceneReport | null;
  error: string | null;
}

/** 应用完成：提示警告；全部成功时弹带撤销的提示并返回 true，有失败时返回 false。 */
function useAppliedNotice() {
  const { t } = useTranslation();
  const undo = useUndoScene();
  return useCallback(
    (name: string, report: SceneReport) => {
      showResultWarnings(report);
      if (failedTools(report).length) return false;
      const separator = t("common.separator");
      const irreversible = (report.irreversible ?? []).map(toolLabel);
      // 快捷键和托盘没有预览：跳过的已删除供应商要在这里说出来。
      const missing = report.results
        .filter((result) => result.missing.length > 0)
        .map((result) => t("scene.missingItem", { tool: toolLabel(result.tool), ids: result.missing.join(separator) }));
      const notes = [
        ...(missing.length ? [t("scene.skippedMissing", { items: missing.join(separator) })] : []),
        ...(irreversible.length ? [t("scene.irreversible", { tools: irreversible.join(separator) })] : []),
      ];
      toast.success(t("scene.applied", { name }), {
        description: notes.length ? notes.join(" ") : undefined,
        action: { label: t("common.undo"), onClick: () => undo.mutate(report.undo) },
      });
      return true;
    },
    [t, undo],
  );
}

let eventApi: Promise<typeof import("@tauri-apps/api/event")> | undefined;

/** 订阅后端事件；不在 Tauri 里（测试、演示）时什么都不做。`handler` 总是用最新的那个。 */
function useBackendEvent<T>(name: string, handler: (payload: T) => void) {
  const latest = useRef(handler);
  latest.current = handler;
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (eventApi ??= import("@tauri-apps/api/event"))
      .then(({ listen }) => listen<T>(name, (event) => latest.current(event.payload)))
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [name]);
}

export function ScenesProvider({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const scenes = useScenes();
  const [applying, setApplying] = useState<string | null>(null);
  const [managing, setManaging] = useState<{ create: boolean } | null>(null);
  const [result, setResult] = useState<{ name: string; report: SceneReport } | null>(null);
  const notice = useAppliedNotice();
  useBackendEvent<SceneEvent>("scene-applied", (event) => {
    void refreshAll(client);
    if (event.error) toast.error(event.error);
    else if (event.report && !notice(event.name, event.report)) setResult({ name: event.name, report: event.report });
  });
  useBackendEvent<SceneSuggestion | null>("scene-suggested", (suggestion) => {
    void client.invalidateQueries({ queryKey: keys.scenes });
    if (!suggestion) return;
    toast.info(t("scene.suggestion", { name: suggestion.name, reason: ruleText(suggestion.rule, t) }), {
      id: "scene-suggestion",
      duration: 15_000,
      action: { label: t("scene.preview"), onClick: () => setApplying(suggestion.id) },
    });
  });

  // 快捷键直接应用，不经过预览；全部成功时提示里可以撤销。
  const quick = useMutation({
    mutationFn: (scene: SceneView) => api.applyScene(scene.id),
    onSuccess: (report, scene) => {
      if (!notice(scene.name, report)) setResult({ name: scene.name, report });
    },
    onError: (error) => toast.error(errorMessage(error)),
    onSettled: () => refreshAll(client),
  });
  const shortcut = useRef<(event: KeyboardEvent) => void>(() => {});
  shortcut.current = (event) => {
    const index = shortcutIndex(event);
    const scene = index >= 0 ? scenes.data?.scenes[index] : undefined;
    if (!scene) return;
    event.preventDefault();
    if (quick.isPending || applying || managing) return;
    if (scene.status === "active") toast.info(t("scene.alreadyActive", { name: scene.name }));
    else quick.mutate(scene);
  };
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => shortcut.current(event);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const apply = useCallback((id: string) => setApplying(id), []);
  const manage = useCallback((options?: { create?: boolean }) => setManaging({ create: options?.create ?? false }), []);
  const value = useMemo(() => ({ apply, manage }), [apply, manage]);
  return (
    <SceneContext.Provider value={value}>
      {children}
      {applying && <SceneApplyDialog key={applying} id={applying} onClose={() => setApplying(null)} />}
      {result && <SceneResultDialog name={result.name} report={result.report} onClose={() => setResult(null)} />}
      {managing && (
        <Suspense fallback={null}>
          <SceneManager
            create={managing.create}
            onApply={(id) => {
              setManaging(null);
              setApplying(id);
            }}
            onClose={() => setManaging(null)}
          />
        </Suspense>
      )}
    </SceneContext.Provider>
  );
}

/** 一个工具要做的事，一句话。 */
function stepText(tool: ToolPreview, t: (key: string, options?: Record<string, unknown>) => string) {
  const names = (kind: string) => tool.steps.filter((s) => s.action.kind === kind).map((s) => s.name).join(t("common.separator"));
  const parts: string[] = [];
  const target = tool.steps.find((s) => s.action.kind === "switch");
  if (target) parts.push(t("scene.switchTo", { from: tool.from ?? t("scene.noCurrent"), to: target.name }));
  const activate = tool.steps.find((s) => s.action.kind === "activate");
  if (activate) parts.push(t("scene.activate", { name: activate.name }));
  if (names("enable")) parts.push(t("scene.enable", { names: names("enable") }));
  if (names("disable")) parts.push(t("scene.disable", { names: names("disable") }));
  return parts.join(" · ");
}

function PreviewRow({ tool, checked, onChecked }: { tool: ToolPreview; checked: boolean; onChecked: (checked: boolean) => void }) {
  const { t } = useTranslation();
  const meta = toolMeta(tool.tool);
  const enabled = applicable(tool);
  const id = `scene-tool-${tool.tool}`;
  return (
    <li className="flex items-start gap-3 px-3 py-2.5">
      <Checkbox id={id} className="mt-0.5" checked={enabled && checked} disabled={!enabled} onCheckedChange={(value) => onChecked(value === true)} aria-label={meta.label} />
      <ToolGlyph tool={meta} size={20} />
      <label htmlFor={id} className={cn("min-w-0 flex-1 text-sm", !enabled && "text-muted-foreground")}>
        <span className="font-medium">{meta.label}</span>
        {tool.steps.length > 0 && <span className="block text-[13px] text-muted-foreground">{stepText(tool, t)}</span>}
        {tool.missing.length > 0 && <span className="block text-xs text-warning">{t("scene.missing", { ids: tool.missing.join(t("common.separator")) })}</span>}
        {tool.error && <span className="block text-xs text-destructive">{t("scene.previewError", { error: tool.error })}</span>}
      </label>
    </li>
  );
}

function OutcomeRow({ result }: { result: ToolOutcome }) {
  const { t } = useTranslation();
  const meta = toolMeta(result.tool);
  const icon = {
    applied: <Check className="h-4 w-4 text-success" aria-hidden="true" />,
    unchanged: <Minus className="h-4 w-4 text-muted-foreground" aria-hidden="true" />,
    failed: <X className="h-4 w-4 text-destructive" aria-hidden="true" />,
  }[result.outcome];
  return (
    <li className="flex items-start gap-3 px-3 py-2.5" data-outcome={result.outcome}>
      <ToolGlyph tool={meta} size={20} />
      <span className="min-w-0 flex-1 text-sm">
        <span className="font-medium">{meta.label}</span>
        {result.error && <span className="block break-words text-xs text-destructive">{result.error}</span>}
        {result.partial && <span className="block text-xs font-medium text-warning">{t("scene.partialTool")}</span>}
        {result.missing.length > 0 && <span className="block text-xs text-warning">{t("scene.missing", { ids: result.missing.join(t("common.separator")) })}</span>}
      </span>
      <span className="flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground">
        {icon}
        {t(`scene.outcome.${result.outcome}`)}
      </span>
    </li>
  );
}

/** 有工具失败时的结果：列出每个工具，可以撤销已经切换的部分。 */
export function SceneResultDialog({ name, report, onClose }: { name: string; report: SceneReport; onClose: () => void }) {
  const { t } = useTranslation();
  const undo = useUndoScene();
  const busy = undo.isPending;
  return (
    <Dialog open onOpenChange={(open) => !open && !busy && onClose()}>
      <DialogContent className="max-w-lg" closeDisabled={busy}>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <AlertTriangle className="h-5 w-5 text-warning" aria-hidden="true" />
            {t("scene.partialTitle", { name })}
          </DialogTitle>
          <DialogDescription>{t("scene.partialHint")}</DialogDescription>
        </DialogHeader>
        <ul className="divide-y rounded-lg border" aria-label={t("scene.tools")}>
          {report.results.map((result) => (
            <OutcomeRow key={result.tool} result={result} />
          ))}
        </ul>
        <DialogFooter>
          <Button variant="outline" disabled={busy} onClick={() => undo.mutate(report.undo, { onSuccess: onClose })}>
            {busy && <Loader2 className="animate-spin" />}
            {t("common.undo")}
          </Button>
          <Button disabled={busy} onClick={onClose}>
            {t("common.close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function SceneApplyDialog({ id, onClose }: { id: string; onClose: () => void }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const preview = useQuery({ queryKey: ["scene-preview", id], queryFn: () => api.previewScene(id), gcTime: 0, staleTime: 0, retry: false });
  const [skipped, setSkipped] = useState<Set<ToolId>>(() => new Set());
  const [report, setReport] = useState<SceneReport | null>(null);
  const name = preview.data?.scene.name ?? "";
  const { changing, inSync } = splitPreview(preview.data?.tools ?? []);
  const chosen = changing.filter((tool) => applicable(tool) && !skipped.has(tool.tool));

  const notice = useAppliedNotice();
  const apply = useMutation({
    mutationFn: (tools: ToolId[]) => api.applyScene(id, tools),
    onSuccess: (result) => {
      if (notice(name, result)) onClose();
      else setReport(result);
    },
    onError: (error) => toast.error(errorMessage(error)),
    onSettled: () => refreshAll(client),
  });

  const busy = apply.isPending;
  const close = (open: boolean) => !open && !busy && onClose();

  if (report) return <SceneResultDialog name={name} report={report} onClose={onClose} />;

  return (
    <Dialog open onOpenChange={close}>
      <DialogContent className="max-w-lg" closeDisabled={busy}>
        <DialogHeader>
          <DialogTitle>{name ? t("scene.applyTitle", { name }) : t("scene.menu")}</DialogTitle>
          <DialogDescription>{t("scene.applyHint")}</DialogDescription>
        </DialogHeader>
        {preview.isPending ? (
          <p className="flex items-center gap-2 py-4 text-sm text-muted-foreground" role="status">
            <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
            {t("scene.loadingPreview")}
          </p>
        ) : preview.isError ? (
          <p className="py-2 text-sm text-destructive" role="alert">
            {errorMessage(preview.error)}
          </p>
        ) : (
          <div className="grid gap-2">
            {changing.length > 0 ? (
              <ul className="divide-y rounded-lg border" aria-label={t("scene.tools")}>
                {changing.map((tool) => (
                  <PreviewRow
                    key={tool.tool}
                    tool={tool}
                    checked={!skipped.has(tool.tool)}
                    onChecked={(checked) =>
                      setSkipped((current) => {
                        const next = new Set(current);
                        if (checked) next.delete(tool.tool);
                        else next.add(tool.tool);
                        return next;
                      })
                    }
                  />
                ))}
              </ul>
            ) : (
              <p className="py-2 text-sm text-muted-foreground" role="status">
                {t("scene.nothing")}
              </p>
            )}
            {changing.length > 0 && inSync.length > 0 && (
              <p className="text-xs text-muted-foreground">{t("scene.inSync", { tools: inSync.map((tool) => toolLabel(tool.tool)).join(t("common.separator")) })}</p>
            )}
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" disabled={busy} onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button disabled={busy || chosen.length === 0} onClick={() => apply.mutate(chosen.map((tool) => tool.tool))}>
            {apply.isPending && <Loader2 className="animate-spin" />}
            {apply.isPending ? t("scene.applying") : t("scene.apply")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
