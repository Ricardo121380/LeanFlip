import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
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
    onSettled: () => client.invalidateQueries(),
  });
}

/** 托盘 / 文件监听 / 后台刷新改了数据：重新拉。 */
export function useBackendEvents() {
  const client = useQueryClient();
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    import("@tauri-apps/api/event")
      .then(({ listen }) => listen("providers-changed", () => client.invalidateQueries()))
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {
        /* 不在 Tauri 里（测试）。 */
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [client]);
}
