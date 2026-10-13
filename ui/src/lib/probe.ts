// 检测结果的展示：延迟分档和错误说明。
import type { TFunction } from "i18next";
import { errorMessage, type ProbeResult } from "./api";

export type Tone = "good" | "fair" | "slow";

/** 与常见测速面板一致：500 ms 以内为好，1 秒以内一般，再慢为慢。 */
export function latencyTone(ms: number): Tone {
  if (ms < 500) return "good";
  if (ms < 1000) return "fair";
  return "slow";
}

export const toneClass: Record<Tone, string> = {
  good: "text-success",
  fair: "text-warning",
  slow: "text-destructive",
};

export type ProbeKind = "latency" | "models" | "check";

/** 失败原因的一句话说明；供应商自己的说明另外显示。 */
export function failureText(t: TFunction, result: ProbeResult, kind: ProbeKind): string {
  const code = result.code ?? "network";
  if (code === "unsupported") return t(`probe.code.unsupported_${result.detail ?? "other"}`);
  if (kind === "models" && code === "not_found") return t("probe.modelsNotFound", { status: result.status });
  return t(`probe.code.${code}`, { status: result.status, detail: result.detail ?? "" });
}

/** 只有供应商返回的说明才值得另起一行；环境变量名等已经写进主句。 */
export function providerDetail(result: ProbeResult): string | null {
  if (!result.detail || ["unsupported", "key_env", "invalid_config"].includes(result.code ?? "")) return null;
  return result.detail;
}

/** 检测从不抛错：调用失败也折算成一条失败结果，界面只处理一种形状。 */
export function settle(run: () => Promise<ProbeResult>): Promise<ProbeResult> {
  return run().catch((error) => ({ ok: false, status: null, latencyMs: 0, code: "network", detail: errorMessage(error), endpoint: null, model: null, models: null }));
}
