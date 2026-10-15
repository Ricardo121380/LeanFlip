// 管理场景：列表（应用、编辑、删除）和编辑器。新建场景从当前状态开始，再按工具微调。
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ArrowLeft, Loader2, Pencil, Play, Plus, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input, NativeSelect } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ToolGlyph } from "@/components/Glyph";
import { QueryState } from "@/components/QueryState";
import { useConfirm } from "@/components/ConfirmDialog";
import { refreshAll, useAllProviders, useScenes } from "@/hooks/queries";
import { api, errorMessage, type Scene, type SceneTool, type SceneView, type ToolProviders } from "@/lib/api";
import { defaultEntry, missingIds, sceneTools, toggleEnabled } from "@/lib/scenes";
import { TOOLS, type ToolMeta } from "@/lib/tools";
import { cn } from "@/lib/utils";

/** 列表项带着状态字段；保存时只交回场景本身，免得状态被存进场景里。 */
function sceneOf({ status: _status, pending: _pending, missing: _missing, ...scene }: SceneView): Scene {
  return structuredClone(scene);
}

function SceneRow({ scene, onApply, onEdit, onDelete }: { scene: SceneView; onApply: () => void; onEdit: () => void; onDelete: () => void }) {
  const { t } = useTranslation();
  const tools = sceneTools(scene, TOOLS).map((id) => TOOLS.find((tool) => tool.id === id)!);
  return (
    <li className="flex items-center gap-3 px-3 py-2.5">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium">{scene.name}</span>
          {scene.status === "active" && <Badge>{t("scene.active")}</Badge>}
          {scene.status === "drifted" && <Badge variant="outline">{t("scene.drifted")}</Badge>}
          {scene.missing.length > 0 && <Badge variant="warning">{t("scene.incomplete")}</Badge>}
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-1" aria-label={tools.map((tool) => tool.label).join(t("common.separator"))}>
          {tools.map((tool) => (
            <ToolGlyph key={tool.id} tool={tool} size={18} />
          ))}
        </div>
      </div>
      <Button size="icon" variant="ghost" aria-label={t("scene.applyNamed", { name: scene.name })} title={t("scene.apply")} onClick={onApply}>
        <Play />
      </Button>
      <Button size="icon" variant="ghost" aria-label={t("scene.editNamed", { name: scene.name })} title={t("scene.edit")} onClick={onEdit}>
        <Pencil />
      </Button>
      <Button size="icon" variant="ghost" aria-label={t("scene.deleteNamed", { name: scene.name })} title={t("scene.delete")} onClick={onDelete}>
        <Trash2 />
      </Button>
    </li>
  );
}

function ToolEntry({ tool, data, entry, onChange }: { tool: ToolMeta; data: ToolProviders | undefined; entry: SceneTool | undefined; onChange: (entry: SceneTool | undefined) => void }) {
  const { t } = useTranslation();
  const providers = data?.providers ?? [];
  const missing = missingIds(entry, providers);
  const id = `scene-include-${tool.id}`;
  return (
    <li className="grid gap-2 px-3 py-2.5">
      <div className="flex items-center gap-3">
        <Checkbox
          id={id}
          checked={entry !== undefined}
          disabled={entry === undefined && providers.length === 0}
          aria-label={t("scene.include", { tool: tool.label })}
          onCheckedChange={(checked) => onChange(checked === true ? (defaultEntry(tool.kind, data) ?? undefined) : undefined)}
        />
        <ToolGlyph tool={tool} size={20} />
        <label htmlFor={id} className="flex-1 text-sm font-medium">
          {tool.label}
        </label>
        {entry && tool.kind === "switch" && (
          <NativeSelect
            className="h-8 w-56"
            aria-label={t("scene.providerFor", { tool: tool.label })}
            value={entry.current ?? ""}
            onChange={(event) => onChange({ ...entry, current: event.target.value })}
          >
            {providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
            {missing.map((missingId) => (
              <option key={missingId} value={missingId}>
                {t("scene.deletedCard", { id: missingId })}
              </option>
            ))}
          </NativeSelect>
        )}
      </div>
      {entry && tool.kind === "coexist" && (
        <div className="ml-7 grid gap-2">
          <div role="group" aria-label={t("scene.enabledFor", { tool: tool.label })} className="flex flex-wrap gap-x-4 gap-y-1.5">
            {providers.map((p) => {
              const box = `scene-${tool.id}-${p.id}`;
              return (
                <label key={p.id} htmlFor={box} className="flex items-center gap-1.5 text-[13px]">
                  <Checkbox id={box} checked={entry.enabled?.includes(p.id) ?? false} onCheckedChange={(checked) => onChange(toggleEnabled(entry, p.id, checked === true))} />
                  {p.name}
                </label>
              );
            })}
          </div>
          {tool.id === "hermes" && (entry.enabled?.length ?? 0) > 0 && (
            <label className="flex items-center gap-2 text-[13px] text-muted-foreground">
              {t("scene.hermesDefault")}
              <NativeSelect
                className="h-8 w-48"
                value={entry.current ?? ""}
                onChange={(event) => {
                  const next = { ...entry };
                  if (event.target.value) next.current = event.target.value;
                  else delete next.current;
                  onChange(next);
                }}
              >
                <option value="">{t("scene.keepDefault")}</option>
                {providers
                  .filter((p) => entry.enabled?.includes(p.id))
                  .map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name}
                    </option>
                  ))}
              </NativeSelect>
            </label>
          )}
        </div>
      )}
      {entry && missing.length > 0 && <p className="ml-7 text-xs text-warning">{t("scene.missing", { ids: missing.join(t("common.separator")) })}</p>}
    </li>
  );
}

function SceneEditor({ draft, onChange }: { draft: Scene; onChange: (scene: Scene) => void }) {
  const { t } = useTranslation();
  const results = useAllProviders();
  const rows = TOOLS.map((tool, index) => ({ tool, data: results[index]?.data })).filter(
    ({ tool, data }) => (data?.providers.length ?? 0) > 0 || draft.tools[tool.id] !== undefined,
  );
  const loading = results.some((r) => r.isPending);
  return (
    <div className="grid gap-4">
      <div className="grid gap-1.5">
        <Label htmlFor="scene-name">{t("scene.name")}</Label>
        <Input id="scene-name" autoFocus maxLength={60} placeholder={t("scene.namePlaceholder")} value={draft.name} onChange={(event) => onChange({ ...draft, name: event.target.value })} />
      </div>
      <fieldset className="grid gap-1.5">
        <legend className="text-sm font-medium">{t("scene.tools")}</legend>
        <p className="text-xs text-muted-foreground">{t("scene.toolsHint")}</p>
        {loading ? (
          <QueryState state="loading" title={t("common.loading")} />
        ) : rows.length === 0 ? (
          <p className="py-2 text-sm text-muted-foreground">{t("scene.noTools")}</p>
        ) : (
          <ul className="mt-1 divide-y rounded-lg border">
            {rows.map(({ tool, data }) => (
              <ToolEntry
                key={tool.id}
                tool={tool}
                data={data}
                entry={draft.tools[tool.id]}
                onChange={(entry) => {
                  const tools = { ...draft.tools };
                  if (entry) tools[tool.id] = entry;
                  else delete tools[tool.id];
                  onChange({ ...draft, tools });
                }}
              />
            ))}
          </ul>
        )}
      </fieldset>
    </div>
  );
}

export function SceneManager({ create, onApply, onClose }: { create: boolean; onApply: (id: string) => void; onClose: () => void }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const scenes = useScenes();
  const [draft, setDraft] = useState<Scene | null>(null);
  const [confirm, confirmDialog] = useConfirm();

  const capture = useMutation({
    mutationFn: () => api.captureScene(),
    onSuccess: (scene) => setDraft({ ...scene, name: "" }),
    onError: (error) => {
      toast.error(errorMessage(error));
      if (create) onClose();
    },
  });
  const save = useMutation({
    mutationFn: (scene: Scene) => api.saveScene(scene),
    onSuccess: (scene) => {
      toast.success(t("scene.saved", { name: scene.name }));
      if (create) onClose();
      else setDraft(null);
    },
    onError: (error) => toast.error(errorMessage(error)),
    onSettled: () => refreshAll(client),
  });
  const remove = useMutation({
    mutationFn: (scene: SceneView) => api.deleteScene(scene.id),
    onSuccess: (_result, scene) => toast.success(t("scene.deleted", { name: scene.name })),
    onError: (error) => toast.error(errorMessage(error)),
    onSettled: () => refreshAll(client),
  });

  const started = useRef(false);
  useEffect(() => {
    if (create && !started.current) {
      started.current = true;
      capture.mutate();
    }
  }, [create, capture]);

  const onDelete = async (scene: SceneView) => {
    if (await confirm({ title: t("scene.deleteTitle", { name: scene.name }), description: t("scene.deleteHint"), confirmLabel: t("scene.delete"), destructive: true })) {
      remove.mutate(scene);
    }
  };
  const back = () => (create ? onClose() : setDraft(null));
  const editing = draft !== null || (create && capture.isPending);
  const canSave = draft !== null && draft.name.trim() !== "" && Object.keys(draft.tools).length > 0;

  return (
    <Dialog open onOpenChange={(open) => !open && !save.isPending && onClose()}>
      <DialogContent className="max-w-xl" closeDisabled={save.isPending}>
        <DialogHeader>
          <DialogTitle>{editing ? t(draft?.id ? "scene.editTitle" : "scene.createTitle") : t("scene.manageTitle")}</DialogTitle>
          <DialogDescription>{editing ? t("scene.editHint") : t("scene.manageHint")}</DialogDescription>
        </DialogHeader>
        {editing ? (
          draft ? (
            <SceneEditor draft={draft} onChange={setDraft} />
          ) : (
            <QueryState state="loading" title={t("common.loading")} />
          )
        ) : scenes.isPending ? (
          <QueryState state="loading" title={t("common.loading")} />
        ) : (scenes.data?.scenes.length ?? 0) === 0 ? (
          <p className="py-2 text-sm text-muted-foreground">{scenes.isError ? errorMessage(scenes.error) : t("scene.emptyList")}</p>
        ) : (
          <ul className="divide-y rounded-lg border" aria-label={t("scene.manageTitle")}>
            {scenes.data!.scenes.map((scene) => (
              <SceneRow key={scene.id} scene={scene} onApply={() => onApply(scene.id)} onEdit={() => setDraft(sceneOf(scene))} onDelete={() => void onDelete(scene)} />
            ))}
          </ul>
        )}
        <DialogFooter className={cn(!editing && "sm:justify-between")}>
          {editing ? (
            <>
              <Button variant="outline" disabled={save.isPending} onClick={back}>
                {!create && <ArrowLeft />}
                {create ? t("common.cancel") : t("scene.back")}
              </Button>
              <Button disabled={!canSave || save.isPending} onClick={() => draft && save.mutate(draft)}>
                {save.isPending && <Loader2 className="animate-spin" />}
                {save.isPending ? t("scene.saving") : t("scene.save")}
              </Button>
            </>
          ) : (
            <>
              <Button variant="outline" disabled={capture.isPending} onClick={() => capture.mutate()}>
                {capture.isPending ? <Loader2 className="animate-spin" /> : <Plus />}
                {t("scene.newScene")}
              </Button>
              <Button onClick={onClose}>{t("common.close")}</Button>
            </>
          )}
        </DialogFooter>
        {confirmDialog}
      </DialogContent>
    </Dialog>
  );
}
