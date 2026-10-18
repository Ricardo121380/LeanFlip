// 同步到其他 Agent：用一张卡的地址和 Key 给其他工具添加供应商。默认都不勾选，只添加卡片；
// 勾了「同时设为当前」才切换。协议不同的工具标「需要检测」，可以先获取模型、检测再添加。
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Check, Loader2, Search, X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ToolGlyph } from "@/components/Glyph";
import { QueryState } from "@/components/QueryState";
import { refreshAll, showWarnings, useAllProviders } from "@/hooks/queries";
import { api, errorMessage, type ProbeResult, type ToolId } from "@/lib/api";
import { endpointOf } from "@/lib/cards";
import { loadPresets, type Preset } from "@/lib/presets";
import { failureText } from "@/lib/probe";
import { linkOf, modelOfCard, newLink, planSync, withLink, withModel, type SyncSource, type SyncTarget } from "@/lib/sync";
import { TOOLS, toolMeta } from "@/lib/tools";
import { cn } from "@/lib/utils";

interface RowState {
  checked: boolean;
  model: string;
  activate: boolean;
  models?: string[];
  check?: ProbeResult | "pending";
  listing?: boolean;
  /** 这次对话框里已经添加成功，重试时跳过。 */
  added?: boolean;
}

type RowUpdate = Partial<RowState> | ((current: RowState) => Partial<RowState>);

const STATUS_VARIANT = { ready: "success", check: "warning", exists: "outline", unavailable: "outline" } as const;

function SyncRow({ target, state, onChange }: { target: SyncTarget; state: RowState; onChange: (next: RowUpdate) => void }) {
  const { t } = useTranslation();
  const meta = toolMeta(target.tool);
  const selectable = Boolean(target.provider) && !state.added;
  const id = `sync-${target.tool}`;
  const card = target.provider ? withModel(target.tool, target.provider, state.model) : undefined;
  const missingModel = target.needsModel && !state.model.trim();
  const check = state.check;

  const listModels = async () => {
    if (!card) return;
    onChange({ listing: true });
    try {
      const result = await api.probeModels(target.tool, card.settingsConfig);
      if (result.ok && result.models?.length) onChange({ models: result.models.map((m) => m.id), listing: false });
      else {
        toast.error(t("sync.modelsFailed", { tool: meta.label, reason: failureText(t, result, "models") }));
        onChange({ listing: false });
      }
    } catch (error) {
      toast.error(errorMessage(error));
      onChange({ listing: false });
    }
  };
  const runCheck = async () => {
    if (!card) return;
    const model = state.model.trim();
    // 检测期间改了模型：结果属于旧模型，丢掉。
    const settle = (check: RowState["check"]) => onChange((current) => (current.model.trim() === model ? { check } : {}));
    onChange({ check: "pending" });
    try {
      settle(await api.probeCheck(target.tool, card.settingsConfig, model || undefined));
    } catch (error) {
      toast.error(errorMessage(error));
      settle(undefined);
    }
  };

  return (
    <li className="grid gap-2 px-3 py-2.5">
      <div className="flex items-start gap-3">
        <Checkbox id={id} className="mt-0.5" checked={selectable && state.checked} disabled={!selectable} onCheckedChange={(value) => onChange({ checked: value === true })} aria-label={meta.label} />
        <ToolGlyph tool={meta} size={20} />
        <label htmlFor={id} className={cn("min-w-0 flex-1 text-sm", !selectable && "text-muted-foreground")}>
          <span className="flex flex-wrap items-center gap-1.5">
            <span className="font-medium">{meta.label}</span>
            {state.added ? (
              <Badge variant="success">{t("sync.status.added")}</Badge>
            ) : (
              <Badge variant={STATUS_VARIANT[target.status]}>
                {target.status === "exists" ? t("sync.status.exists", { name: target.existing?.name }) : t(`sync.status.${target.status}`)}
              </Badge>
            )}
          </span>
          {card && <span className="block truncate font-mono text-xs text-muted-foreground">{endpointOf(target.tool, card)}</span>}
          {target.status === "check" && !state.added && <span className="block text-xs text-muted-foreground">{t(target.family === "gemini" ? "sync.geminiHint" : "sync.checkHint", { protocol: t(`sync.family.${target.family}`) })}</span>}
        </label>
      </div>
      {selectable && state.checked && (
        <div className="ml-7 grid gap-2">
          <div className="flex flex-wrap items-center gap-2">
            <Input
              className="h-8 w-56"
              list={`${id}-models`}
              aria-label={t("sync.model", { tool: meta.label })}
              placeholder={t(target.needsModel ? "sync.modelRequired" : "sync.modelOptional")}
              value={state.model}
              aria-invalid={missingModel}
              onChange={(event) => onChange({ model: event.target.value, check: undefined })}
            />
            {state.models && (
              <datalist id={`${id}-models`}>
                {state.models.map((model) => (
                  <option key={model} value={model} />
                ))}
              </datalist>
            )}
            <Button type="button" size="sm" variant="outline" disabled={state.listing} onClick={() => void listModels()}>
              {state.listing ? <Loader2 className="animate-spin" /> : <Search />}
              {t("sync.listModels")}
            </Button>
            <Button type="button" size="sm" variant="outline" disabled={check === "pending" || missingModel} onClick={() => void runCheck()}>
              {check === "pending" && <Loader2 className="animate-spin" />}
              {t("sync.check")}
            </Button>
            {check && check !== "pending" && (
              <span role="status" className={cn("flex items-center gap-1 text-xs", check.ok ? "text-success" : "text-destructive")}>
                {check.ok ? <Check className="h-3.5 w-3.5" aria-hidden="true" /> : <X className="h-3.5 w-3.5" aria-hidden="true" />}
                {check.ok ? t("sync.checkOk", { ms: check.latencyMs }) : failureText(t, check, "check")}
              </span>
            )}
          </div>
          <label className="flex items-center gap-2 text-[13px] text-muted-foreground">
            <Checkbox checked={state.activate} onCheckedChange={(value) => onChange({ activate: value === true })} />
            {t(meta.kind === "coexist" ? "sync.enableToo" : "sync.activateToo")}
          </label>
        </div>
      )}
    </li>
  );
}

export function SyncDialog({ source, onClose }: { source: SyncSource; onClose: () => void }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const results = useAllProviders();
  const [presets, setPresets] = useState<Partial<Record<ToolId, Preset[]>> | null>(null);
  const [rows, setRows] = useState<Partial<Record<ToolId, RowState>>>({});
  const [saving, setSaving] = useState(false);
  // 一个对话框只用一个关联 ID：部分失败后重试，新加的卡和之前加的、和来源都在同一组。
  const link = useRef(linkOf(source.provider) ?? newLink());

  useEffect(() => {
    let alive = true;
    void Promise.all(TOOLS.map(async (tool) => [tool.id, await loadPresets(tool.id).catch(() => [])] as const)).then((loaded) => {
      if (alive) setPresets(Object.fromEntries(loaded));
    });
    return () => {
      alive = false;
    };
  }, []);

  const loading = !presets || results.some((r) => r.isPending);
  const targets = useMemo(() => {
    if (loading) return [];
    const providers = Object.fromEntries(TOOLS.map((tool, index) => [tool.id, results[index]?.data?.providers ?? []]));
    return planSync(source, presets!, providers);
    // 只在数据齐了之后算一次：之后刷新查询不应该打乱用户已经填的内容。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loading, source]);

  const initial = (target: SyncTarget): RowState => ({ checked: false, activate: false, model: target.provider ? modelOfCard(target.tool, target.provider) : "" });
  const stateOf = (target: SyncTarget): RowState => rows[target.tool] ?? initial(target);
  // 按最新状态合并：异步请求回来时不能把用户在等待期间改的内容改回去。
  const update = (target: SyncTarget, next: RowUpdate) =>
    setRows((all) => {
      const current = all[target.tool] ?? initial(target);
      return { ...all, [target.tool]: { ...current, ...(typeof next === "function" ? next(current) : next) } };
    });

  const chosen = targets.filter((target) => target.provider && !stateOf(target).added && stateOf(target).checked);
  const blocked = chosen.some((target) => target.needsModel && !stateOf(target).model.trim());

  const save = async () => {
    setSaving(true);
    let added = 0;
    try {
      await api.linkProvider(source.tool, source.provider, link.current);
      for (const target of chosen) {
        const state = stateOf(target);
        const card = withLink(withModel(target.tool, target.provider!, state.model), link.current);
        try {
          showWarnings(await api.addProvider(target.tool, card, state.activate));
          added += 1;
          update(target, { added: true, checked: false });
        } catch (error) {
          toast.error(t("sync.addFailed", { tool: toolMeta(target.tool).label, reason: errorMessage(error) }));
        }
      }
      if (added > 0) toast.success(t("sync.added", { count: added }));
      if (added === chosen.length) onClose();
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setSaving(false);
      void refreshAll(client);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && !saving && onClose()}>
      <DialogContent className="max-w-2xl" closeDisabled={saving}>
        <DialogHeader>
          <DialogTitle>{t("sync.title")}</DialogTitle>
          <DialogDescription>{t("sync.hint", { name: source.provider.name, tool: toolMeta(source.tool).label })}</DialogDescription>
        </DialogHeader>
        {loading ? (
          <QueryState state="loading" title={t("common.loading")} />
        ) : (
          <ul className="max-h-[min(60vh,32rem)] divide-y overflow-y-auto rounded-lg border" aria-label={t("sync.title")}>
            {targets.map((target) => (
              <SyncRow key={target.tool} target={target} state={stateOf(target)} onChange={(next) => update(target, next)} />
            ))}
          </ul>
        )}
        <DialogFooter>
          <Button variant="outline" disabled={saving} onClick={onClose}>
            {t("sync.skip")}
          </Button>
          <Button disabled={saving || chosen.length === 0 || blocked} onClick={() => void save()}>
            {saving && <Loader2 className="animate-spin" />}
            {t("sync.add", { count: chosen.length })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
