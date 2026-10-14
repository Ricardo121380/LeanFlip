import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { api, errorMessage, type CommonConfig, type ToolId } from "@/lib/api";
import { toolMeta } from "@/lib/tools";
import { commonOptions, existingOptions, applyOption } from "@/lib/config-options";
import { useEditorNavigation } from "@/lib/navigation";
import { refreshAll, showWarnings } from "@/hooks/queries";
import { CodeEditor } from "./CodeEditor";
import { ConfigOptions } from "./ConfigOptions";
import { useConfirm } from "./ConfirmDialog";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from "./ui/dialog";

export function CommonConfigEditor({ tool, onClose }: { tool: ToolId; onClose: () => void }) {
  const { t } = useTranslation();
  const [source, setSource] = useState<CommonConfig | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [attempt, retry] = useState(0);
  useEffect(() => {
    let active = true;
    setError(null);
    api.getCommonConfig(tool).then((value) => { if (active) setSource(value); }, (error) => { if (active) setError(error); });
    return () => { active = false; };
  }, [tool, attempt]);
  if (source) return <Loaded tool={tool} initial={source} onClose={onClose} />;
  return <Dialog open onOpenChange={(v) => !v && onClose()}><DialogContent><DialogHeader><DialogTitle>{t("config.commonTitle")}</DialogTitle><DialogDescription>{error ? errorMessage(error) : t("common.loading")}</DialogDescription></DialogHeader>{Boolean(error) && <Button onClick={() => retry((v) => v + 1)}>{t("common.retry")}</Button>}</DialogContent></Dialog>;
}
function Loaded({ tool, initial, onClose }: { tool: ToolId; initial: CommonConfig; onClose: () => void }) {
  const [source, setSource] = useState(initial);
  const { t } = useTranslation();
  const client = useQueryClient();
  const [text, setText] = useState(JSON.stringify(source.config, null, 2));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirm, confirmDialog] = useConfirm();
  const dirty = text !== JSON.stringify(source.config, null, 2);
  useEditorNavigation({ dirty, saving, discard: onClose });
  let parsed: Record<string, unknown> | null = null;
  try { const result: unknown = JSON.parse(text); if (result && typeof result === "object" && !Array.isArray(result)) parsed = result as Record<string, unknown>; } catch { /* shown inline */ }
  const known = commonOptions[tool] ?? [];
  const options = [...known, ...existingOptions(parsed ?? {}, known)];
  const discard = async () => !dirty || await confirm({ title: t("editor.discardTitle"), description: t("editor.discardHint"), confirmLabel: t("editor.discardAction") });
  const close = async () => { if (!saving && await discard()) onClose(); };
  const save = async () => {
    if (!parsed || saving) return;
    setSaving(true); setError(null);
    try {
      showWarnings(await api.saveCommonConfig(tool, source.revision, parsed));
      await client.invalidateQueries({ queryKey: ["commonConfig", tool] });
      await refreshAll(client);
      toast.success(t("config.saved")); onClose();
    } catch (e) { setError(errorMessage(e)); } finally { setSaving(false); }
  };
  return <><Dialog open onOpenChange={(v) => !v && void close()}><DialogContent className="flex max-w-[52rem] max-h-[90vh] flex-col overflow-hidden" onEscapeKeyDown={(e) => { if (saving) e.preventDefault(); }}>
    <DialogHeader className="shrink-0"><DialogTitle>{toolMeta(tool).label} · {t("config.commonTitle")}</DialogTitle><DialogDescription>{t("config.commonHint")}</DialogDescription></DialogHeader>
    <div className="min-h-0 flex-1 space-y-4 overflow-y-auto py-2">
      <div className="rounded-md border bg-muted/30 px-3 py-2 text-xs text-muted-foreground"><p>{t("config.writesTo", { format: source.format })}</p>{source.paths.map((path) => <code key={path} className="block break-all pt-1">{path}</code>)}</div>
      <ConfigOptions options={options} value={parsed ?? {}} disabled={!parsed || saving} onChange={(option, state) => { try { setText(JSON.stringify(applyOption(parsed!, option, state), null, 2)); setError(null); } catch (e) { setError(errorMessage(e)); } }} />
      <div className="flex items-center justify-between"><b className="text-sm">{t("config.json")}</b><Button size="sm" variant="ghost" disabled={!parsed || saving} onClick={() => setText(JSON.stringify(parsed, null, 2))}>{t("config.format")}</Button></div>
      <CodeEditor value={text} onChange={setText} language="json" minHeight={260} readOnly={saving} ariaLabel={t("config.json")} />
      {!parsed && <p role="alert" className="text-xs text-destructive">{t("editor.notObject")}</p>}
      {error && <p role="alert" className="text-xs text-destructive break-words">{error}</p>}
      <p className="text-xs text-muted-foreground">{t("config.restartHint")}</p>
    </div>
    <DialogFooter className="shrink-0 flex-wrap"><Button variant="ghost" disabled={saving} onClick={async () => { if (await discard()) {
      setSaving(true);
      try { const next = await api.getCommonConfig(tool); setSource(next); setText(JSON.stringify(next.config, null, 2)); setError(null); }
      catch (e) { setError(errorMessage(e)); }
      finally { setSaving(false); }
    } }}>{t("config.reload")}</Button><Button variant="outline" disabled={saving} onClick={() => void close()}>{t("common.cancel")}</Button><Button disabled={!dirty || !parsed || saving} onClick={() => void save()}>{t(saving ? "editor.saving" : "config.apply")}</Button></DialogFooter>
  </DialogContent></Dialog>{confirmDialog}</>;
}
