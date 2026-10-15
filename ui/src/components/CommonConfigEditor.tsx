import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { api, errorMessage, type CommonConfig, type ToolId } from "@/lib/api";
import { toolMeta } from "@/lib/tools";
import { commonOptionsFor, applyOption, setPath } from "@/lib/config-options";
import { commonFieldsFor, configFieldsIssue } from "@/lib/config-fields";
import { ConfigCompatibility } from "./ConfigCompatibility";
import { ConfigFields, useFieldError } from "./ConfigFields";
import { useEditorNavigation } from "@/lib/navigation";
import { refreshAll, showWarnings } from "@/hooks/queries";
import { CodeEditor } from "./CodeEditor";
import { ConfigOptions } from "./ConfigOptions";
import { useConfirm } from "./ConfirmDialog";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from "./ui/dialog";

export function CommonConfigEditor({ tool, onClose }: { tool: ToolId; onClose: () => void }) {
  const { t } = useTranslation();
  const [opener] = useState(() => document.activeElement instanceof HTMLElement ? document.activeElement : null);
  const closing = useRef(false);
  const close = () => { closing.current = true; onClose(); };
  const restoreFocus = (event: Event) => { event.preventDefault(); if (closing.current && opener?.isConnected) opener.focus(); };
  const [source, setSource] = useState<CommonConfig | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [attempt, retry] = useState(0);
  useEffect(() => {
    let active = true;
    setError(null);
    api.getCommonConfig(tool).then((value) => { if (active) setSource(value); }, (error) => { if (active) setError(error); });
    return () => { active = false; };
  }, [tool, attempt]);
  if (source) return <Loaded tool={tool} initial={source} onClose={close} restoreFocus={restoreFocus} />;
  return <Dialog open onOpenChange={(v) => !v && close()}><DialogContent onCloseAutoFocus={restoreFocus}><DialogHeader><DialogTitle>{t("config.commonTitle")}</DialogTitle><DialogDescription>{error ? errorMessage(error) : t("common.loading")}</DialogDescription></DialogHeader>{Boolean(error) && <Button onClick={() => retry((v) => v + 1)}>{t("common.retry")}</Button>}</DialogContent></Dialog>;
}
function Loaded({ tool, initial, onClose, restoreFocus }: { tool: ToolId; initial: CommonConfig; onClose: () => void; restoreFocus: (event: Event) => void }) {
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
  let parseError: string | null = null;
  try {
    const result: unknown = JSON.parse(text);
    if (result && typeof result === "object" && !Array.isArray(result)) parsed = result as Record<string, unknown>;
    else parseError = t("editor.notObject");
  } catch (e) { parseError = `${t("editor.invalid")}: ${errorMessage(e)}`; }
  const v2 = source.variant === "opencode-v2";
  const options = commonOptionsFor(tool, parsed, v2);
  const fields = commonFieldsFor(tool, parsed, v2);
  const fieldError = useFieldError(parsed ? configFieldsIssue(tool, "common", parsed, source.config, v2) : null);
  const problem = parseError ?? fieldError ?? error;
  const discard = async () => !dirty || await confirm({ title: t("editor.discardTitle"), description: t("editor.discardHint"), confirmLabel: t("editor.discardAction") });
  const close = async () => { if (!saving && await discard()) onClose(); };
  const save = async () => {
    if (!parsed || fieldError || saving) return;
    setSaving(true); setError(null);
    try {
      showWarnings(await api.saveCommonConfig(tool, source.revision, parsed));
      await client.invalidateQueries({ queryKey: ["commonConfig", tool] });
      await refreshAll(client);
      toast.success(t("config.saved")); onClose();
    } catch (e) { setError(errorMessage(e)); } finally { setSaving(false); }
  };
  return <><Dialog open onOpenChange={(v) => !v && void close()}><DialogContent className="flex max-w-[52rem] max-h-[90vh] flex-col overflow-hidden" closeDisabled={saving} onCloseAutoFocus={restoreFocus} onEscapeKeyDown={(e) => { if (saving) e.preventDefault(); }}>
    <DialogHeader className="shrink-0"><DialogTitle>{toolMeta(tool).label} · {t("config.commonTitle")}</DialogTitle><DialogDescription>{t("config.commonHint")}</DialogDescription></DialogHeader>
    <div className="min-h-0 flex-1 space-y-4 overflow-y-auto py-2">
      {(options.length > 0 || fields.length > 0) && <section className="space-y-3" aria-label={t("config.quickOptions")}>
        <h3 className="text-sm font-medium">{t("config.quickOptions")}</h3>
        <ConfigOptions options={options} value={parsed ?? {}} disabled={!parsed || saving} onChange={(option, state) => { try { setText(JSON.stringify(applyOption(parsed!, option, state), null, 2)); setError(null); } catch (e) { setError(errorMessage(e)); } }} />
        <ConfigFields fields={fields} value={parsed ?? {}} before={source.config} disabled={!parsed || saving} onChange={(key, value) => { try { setText(JSON.stringify(setPath(parsed!, key, value), null, 2)); setError(null); } catch (e) { setError(errorMessage(e)); } }} />
        <p className="text-xs leading-5 text-muted-foreground">{t("config.quickHint")}</p>
        <ConfigCompatibility tool={tool} config={parsed} v2={v2} />
      </section>}
      <div className="space-y-1">
        <div className="flex items-center justify-between"><h3 className="text-sm font-medium">{t("config.json")}</h3><Button size="sm" variant="ghost" disabled={!parsed || saving} onClick={() => setText(JSON.stringify(parsed, null, 2))}>{t("config.format")}</Button></div>
        <p className="text-xs text-muted-foreground">{t("config.jsonHint", { format: source.format })}</p>
      </div>
      <CodeEditor value={text} onChange={(value) => { setText(value); setError(null); }} language="json" minHeight={260} readOnly={saving} ariaLabel={t("config.json")} />
      <details className="text-xs text-muted-foreground"><summary className="w-fit cursor-pointer rounded focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{t("config.writesTo", { format: source.format })}</summary>{source.paths.map((path) => <code key={path} className="block break-all pt-2">{path}</code>)}</details>
      <p className="text-xs text-muted-foreground">{t("config.restartHint")}</p>
    </div>
    <div className="shrink-0 space-y-3 border-t pt-4" data-testid="common-config-footer">
      {problem && <div role="alert" className="max-h-28 overflow-y-auto rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs leading-5 text-destructive break-words"><p className="font-semibold">{t("config.notSaved")}</p><p>{problem}</p></div>}
      <DialogFooter className="flex-wrap"><Button variant="ghost" disabled={saving} onClick={async () => { if (await discard()) {
      setSaving(true);
      try { const next = await api.getCommonConfig(tool); setSource(next); setText(JSON.stringify(next.config, null, 2)); setError(null); }
      catch (e) { setError(errorMessage(e)); }
      finally { setSaving(false); }
    } }}>{t("config.reload")}</Button><Button variant="outline" disabled={saving} onClick={() => void close()}>{t("common.cancel")}</Button><Button disabled={!dirty || !parsed || Boolean(fieldError) || saving} onClick={() => void save()}>{t(saving ? "editor.saving" : "config.apply")}</Button></DialogFooter>
    </div>
  </DialogContent></Dialog>{confirmDialog}</>;
}
