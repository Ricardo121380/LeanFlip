import { useEffect } from "react";
import { useMutation, useQueries, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { api, errorMessage, type OpResult, type ToolId, type ToolProviders } from "@/lib/api";
import { TOOLS } from "@/lib/tools";

export const keys = {
  overview: ["overview"] as const,
  recovery: ["recovery"] as const,
  providers: (tool: ToolId) => ["providers", tool] as const,
  accounts: ["codex-accounts"] as const,
  logins: ["codex-logins"] as const,
  settings: ["settings"] as const,
  usage: ["usage"] as const,
  quota: ["quota"] as const,
  scenes: ["scenes"] as const,
};

export function useRecoveryNotices() {
  return useQuery({ queryKey: keys.recovery, queryFn: api.recoveryNotices });
}

export function useOverview(enabled = true) {
  return useQuery({ queryKey: keys.overview, queryFn: api.overview, enabled });
}

export function useProviders(tool: ToolId) {
  return useQuery({ queryKey: keys.providers(tool), queryFn: () => api.listProviders(tool) });
}

/** 所有工具的供应商，按 `TOOLS` 的顺序。首页和命令面板共用缓存。 */
export function useAllProviders(enabled = true) {
  return useQueries({
    queries: TOOLS.map((tool) => ({ queryKey: keys.providers(tool.id), queryFn: () => api.listProviders(tool.id), enabled })),
  });
}

/** 用量统计。每次读取都会先增量扫描各工具新增的记录。 */
export function useUsage() {
  return useQuery({ queryKey: keys.usage, queryFn: api.usageReport, staleTime: 60_000 });
}

export function useAccounts(enabled = true) {
  return useQuery({ queryKey: keys.accounts, queryFn: api.codexAccounts, enabled });
}

export function useLogins(enabled = true) {
  return useQuery({ queryKey: keys.logins, queryFn: api.codexLogins, enabled });
}

/** 额度总览：本机记录加上次联网查询的结果，不联网。 */
export function useQuota(enabled = true) {
  return useQuery({ queryKey: keys.quota, queryFn: api.quotaOverview, enabled, staleTime: 30_000 });
}

/** 联网查询额度（不传来源时查全部），结果直接替换总览。 */
export function useQuotaRefresh() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (keys?: string[]) => api.quotaRefresh(keys),
    onSuccess: (view) => client.setQueryData(keys.quota, view),
    onError: (error) => toast.error(errorMessage(error)),
  });
}

/** 场景和各自的状态（是否和现在的配置一致）。只读。 */
export function useScenes(enabled = true) {
  return useQuery({ queryKey: keys.scenes, queryFn: api.listScenes, enabled });
}

export function useSettings() {
  return useQuery({ queryKey: keys.settings, queryFn: api.getSettings });
}

/**
 * 数据变了：刷新除预设外的查询。`cancelRefetch: false` 让正在进行的请求直接复用，
 * 避免「操作完成 + 后端事件 + 文件监听」接连触发时反复取消重拉。
 */
export function refreshAll(client: QueryClient) {
  return client.invalidateQueries({ predicate: (q) => q.queryKey[0] !== "presets" }, { cancelRefetch: false });
}

/** 合并短时间内的多次调用，只在最后一次之后执行。 */
export function debounce(fn: () => void, ms: number) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const run = () => {
    clearTimeout(timer);
    timer = setTimeout(fn, ms);
  };
  run.cancel = () => clearTimeout(timer);
  return run;
}

export function showWarnings(result: OpResult | void | undefined) {
  for (const warning of result?.warnings ?? []) toast.warning(warning);
}

/** 改数据的操作：成功后刷新所有查询，失败弹提示。 */
export function useAction<TArgs, TResult>(fn: (args: TArgs) => Promise<TResult>, onSuccess?: (r: TResult, a: TArgs) => void) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: fn,
    onSuccess: (result, args) => {
      if (result && typeof result === "object" && "warnings" in (result as object)) {
        showWarnings(result as unknown as OpResult);
      }
      onSuccess?.(result, args);
    },
    onError: (error) => toast.error(errorMessage(error)),
    onSettled: () => refreshAll(client),
  });
}

/** 托盘 / 文件监听 / 后台刷新改了数据：重新拉。 */
export function useBackendEvents() {
  const client = useQueryClient();
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    const refresh = debounce(() => void refreshAll(client), 150);
    import("@tauri-apps/api/event")
      .then(({ listen }) => listen("providers-changed", refresh))
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {
        /* 不在 Tauri 里（测试）。 */
      });
    return () => {
      cancelled = true;
      refresh.cancel();
      unlisten?.();
    };
  }, [client]);
}

export interface SwitchArgs {
  tool: ToolId;
  id: string;
  name: string;
  /** 切换前的当前供应商：成功提示里提供“切回”。 */
  previous?: { id: string; name: string } | null;
  undo?: boolean;
}

/** 先把缓存改成操作后的样子，界面立刻动；失败时还原。 */
async function assume(client: QueryClient, tool: ToolId, change: (data: ToolProviders) => ToolProviders) {
  await client.cancelQueries({ queryKey: keys.providers(tool) });
  const before = client.getQueryData<ToolProviders>(keys.providers(tool));
  if (before) client.setQueryData(keys.providers(tool), change(before));
  return { before };
}

/**
 * 切换供应商。工具页、首页和命令面板共用：成功后提示，并可一键切回上一个。
 * `optimistic` 让首页的动画不用等后端返回。
 */
export function useSwitchProvider({ optimistic = false }: { optimistic?: boolean } = {}) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const mutation = useMutation({
    mutationFn: ({ tool, id }: SwitchArgs) => api.switchProvider(tool, id),
    onMutate: (args) => (optimistic ? assume(client, args.tool, (data) => ({ ...data, current: args.id })) : undefined),
    onSuccess: (result, args) => {
      showWarnings(result);
      const back = args.undo ? null : args.previous;
      toast.success(
        t(args.undo ? "card.switchedBack" : "card.switched", { name: args.name }),
        back
          ? { action: { label: t("card.switchBack", { name: back.name }), onClick: () => mutation.mutate({ tool: args.tool, id: back.id, name: back.name, undo: true }) } }
          : undefined,
      );
    },
    onError: (error, args, context) => {
      if (context?.before) client.setQueryData(keys.providers(args.tool), context.before);
      toast.error(errorMessage(error));
    },
    onSettled: () => refreshAll(client),
  });
  return mutation;
}

export interface ToggleArgs {
  tool: ToolId;
  id: string;
  name: string;
  enabled: boolean;
  undo?: boolean;
}

/** 并存式工具里写入或移除一个供应商，提示里可以撤销。 */
export function useToggleLive({ optimistic = false }: { optimistic?: boolean } = {}) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const mutation = useMutation({
    mutationFn: ({ tool, id, enabled }: ToggleArgs) => api.setInLive(tool, id, enabled),
    onMutate: (args) =>
      optimistic
        ? assume(client, args.tool, (data) => ({
            ...data,
            inLive: args.enabled ? [...data.inLive.filter((id) => id !== args.id), args.id] : data.inLive.filter((id) => id !== args.id),
          }))
        : undefined,
    onSuccess: (result, args) => {
      showWarnings(result);
      toast.success(
        t(args.enabled ? "card.enabled" : "card.disabled", { name: args.name }),
        args.undo ? undefined : { action: { label: t("common.undo"), onClick: () => mutation.mutate({ ...args, enabled: !args.enabled, undo: true }) } },
      );
    },
    onError: (error, args, context) => {
      if (context?.before) client.setQueryData(keys.providers(args.tool), context.before);
      toast.error(errorMessage(error));
    },
    onSettled: () => refreshAll(client),
  });
  return mutation;
}
