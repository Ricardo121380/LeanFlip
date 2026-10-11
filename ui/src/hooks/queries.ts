import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { api, errorMessage, type OpResult, type ToolId } from "@/lib/api";

export const keys = {
  overview: ["overview"] as const,
  providers: (tool: ToolId) => ["providers", tool] as const,
  accounts: ["codex-accounts"] as const,
  logins: ["codex-logins"] as const,
  settings: ["settings"] as const,
};

export function useOverview() {
  return useQuery({ queryKey: keys.overview, queryFn: api.overview });
}

export function useProviders(tool: ToolId) {
  return useQuery({ queryKey: keys.providers(tool), queryFn: () => api.listProviders(tool) });
}

export function useAccounts() {
  return useQuery({ queryKey: keys.accounts, queryFn: api.codexAccounts });
}

export function useLogins(enabled = true) {
  return useQuery({ queryKey: keys.logins, queryFn: api.codexLogins, enabled });
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
