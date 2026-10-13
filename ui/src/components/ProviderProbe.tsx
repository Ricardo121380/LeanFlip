// 编辑器里的检测区：测连通、检查模型，以及结果的展示。
import { useTranslation } from "react-i18next";
import { CircleCheck, CircleX, Gauge, Loader2, Stethoscope } from "lucide-react";
import { Button } from "@/components/ui/button";
import { NativeSelect } from "@/components/ui/input";
import type { ProbeResult } from "@/lib/api";
import { failureText, latencyTone, providerDetail, toneClass, type ProbeKind } from "@/lib/probe";
import { cn } from "@/lib/utils";

export interface ProbeSlot {
  pending: boolean;
  result?: ProbeResult;
  /** 结果出来之后配置又改过。 */
  stale?: boolean;
}

export function ProbeLine({ kind, slot }: { kind: ProbeKind; slot: ProbeSlot }) {
  const { t } = useTranslation();
  if (slot.pending) {
    return (
      <p role="status" className="flex items-center gap-2 text-xs text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
        {t(`probe.running_${kind}`)}
      </p>
    );
  }
  const result = slot.result;
  if (!result) return null;
  const detail = providerDetail(result);
  const headline = result.ok
    ? kind === "latency"
      ? t("probe.reachable")
      : kind === "check"
        ? t("probe.checkOk", { model: result.model ?? "" })
        : t(result.models?.length ? "probe.modelsOk" : "probe.modelsEmpty", { count: result.models?.length ?? 0 })
    : failureText(t, result, kind);
  return (
    <div role="status" data-testid={`probe-${kind}`} className={cn("flex min-w-0 items-start gap-2 text-xs leading-5", slot.stale && "opacity-60")}>
      {result.ok ? (
        <CircleCheck className="mt-[3px] h-3.5 w-3.5 shrink-0 text-success" aria-hidden="true" />
      ) : (
        <CircleX className="mt-[3px] h-3.5 w-3.5 shrink-0 text-destructive" aria-hidden="true" />
      )}
      <div className="min-w-0 flex-1">
        <p className="break-words [overflow-wrap:anywhere]">
          <span className="font-medium">{headline}</span>
          {result.ok && kind !== "models" && (
            <span className={cn("ml-1.5 font-medium tabular-nums", toneClass[latencyTone(result.latencyMs)])}>{t("probe.ms", { ms: result.latencyMs })}</span>
          )}
          {result.ok && kind === "latency" && result.status !== null && <span className="ml-1.5 text-muted-foreground">HTTP {result.status}</span>}
        </p>
        {detail && <p className="break-words text-muted-foreground [overflow-wrap:anywhere]">{detail}</p>}
        {result.endpoint && <p className="truncate font-mono text-[11px] text-muted-foreground" title={result.endpoint}>{result.endpoint}</p>}
        {slot.stale && <p className="text-muted-foreground">{t("probe.stale")}</p>}
      </div>
    </div>
  );
}

interface CheckProps {
  latency: ProbeSlot;
  check: ProbeSlot;
  onLatency: () => void;
  onCheck: () => void;
  /** 可用于检查的模型：已填写的在前，获取到的在后。 */
  candidates: string[];
  model: string;
  onModel: (model: string) => void;
}

export function ConnectionCheck({ latency, check, onLatency, onCheck, candidates, model, onModel }: CheckProps) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-2.5 rounded-lg bg-muted/50 px-3.5 py-3">
      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" size="sm" variant="outline" className="bg-card" disabled={latency.pending} onClick={onLatency}>
          {latency.pending ? <Loader2 className="animate-spin" /> : <Gauge />} {t("probe.latency")}
        </Button>
        <Button type="button" size="sm" variant="outline" className="bg-card" disabled={check.pending || !model} onClick={onCheck}>
          {check.pending ? <Loader2 className="animate-spin" /> : <Stethoscope />} {t("probe.check")}
        </Button>
        {candidates.length > 1 ? (
          <NativeSelect aria-label={t("probe.checkWith")} value={model} disabled={check.pending} onChange={(e) => onModel(e.target.value)} className="h-8 w-auto min-w-0 max-w-[16rem] bg-card font-mono text-xs">
            {candidates.map((candidate) => <option key={candidate} value={candidate}>{candidate}</option>)}
          </NativeSelect>
        ) : model ? (
          <code className="min-w-0 truncate font-mono text-xs text-muted-foreground" title={t("probe.checkWith")}>{model}</code>
        ) : (
          <span className="text-xs text-muted-foreground">{t("probe.noModel")}</span>
        )}
      </div>
      <ProbeLine kind="latency" slot={latency} />
      <ProbeLine kind="check" slot={check} />
      <p className="text-[11px] leading-4 text-muted-foreground">{t("probe.note")}</p>
    </div>
  );
}
