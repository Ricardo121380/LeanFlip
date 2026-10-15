// 首次查询中转站额度：用卡片里的地址和 Key 依次试 sub2api、NewAPI 的额度接口，认出来后
// 由调用方决定怎么保存（编辑器写进草稿，列表直接写回卡片）。打开就会联网，只能由用户点击打开。
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Check, CircleSlash, Loader2, RotateCcw, WifiOff } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { QuotaDetails } from "@/components/QuotaDetails";
import { api, errorMessage, type DetectStep, type RelayAdapter, type ToolId } from "@/lib/api";
import { adapterName, RELAY_ADAPTERS } from "@/lib/quota";

function StepRow({ step }: { step: DetectStep }) {
  const { t } = useTranslation();
  const icon = {
    hit: <Check className="h-3.5 w-3.5 text-success" />,
    miss: <CircleSlash className="h-3.5 w-3.5 text-muted-foreground" />,
    error: <WifiOff className="h-3.5 w-3.5 text-warning" />,
    skipped: <span className="h-3.5 w-3.5" />,
  }[step.outcome];
  const outcome =
    step.outcome === "miss" && step.status && step.status !== 200
      ? t("quotaDetect.missStatus", { status: step.status })
      : t(`quotaDetect.${step.outcome}`);
  return (
    <li className="grid grid-cols-[1rem_minmax(0,1fr)_auto] items-center gap-2.5 px-3 py-2 text-xs" data-outcome={step.outcome}>
      {icon}
      <span className="min-w-0">
        <span className="font-medium">{adapterName(step.adapter)}</span>
        <span className="block truncate font-mono text-[11px] text-muted-foreground">GET {step.url}</span>
      </span>
      <span className={step.outcome === "hit" ? "font-medium text-success" : "text-muted-foreground"}>{outcome}</span>
    </li>
  );
}

export function QuotaDetectDialog({
  tool,
  name,
  settingsConfig,
  quotaPerUsd,
  onUse,
  onClose,
}: {
  tool: ToolId;
  name: string;
  settingsConfig: Record<string, unknown>;
  quotaPerUsd?: number;
  onUse: (adapter: RelayAdapter) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const detect = useQuery({
    queryKey: ["quota-detect", tool, JSON.stringify(settingsConfig), quotaPerUsd ?? null],
    queryFn: () => api.quotaDetect(tool, settingsConfig, quotaPerUsd),
    retry: false,
    gcTime: 0,
    staleTime: Infinity,
    refetchOnWindowFocus: false,
  });
  const result = detect.data;
  const found = result?.adapter;

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("quotaDetect.title", { name })}</DialogTitle>
          <DialogDescription>{t("quotaDetect.hint")}</DialogDescription>
        </DialogHeader>

        {detect.isFetching ? (
          <p role="status" className="flex items-center gap-2 py-3 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" /> {t("quotaDetect.running", { list: RELAY_ADAPTERS.map(adapterName).join("、") })}
          </p>
        ) : detect.isError ? (
          <p role="alert" className="py-2 text-sm text-destructive">{errorMessage(detect.error)}</p>
        ) : result?.error === "noEndpoint" ? (
          <p role="status" className="py-2 text-sm text-muted-foreground">{t("quotaDetect.noEndpoint")}</p>
        ) : result ? (
          <div className="flex flex-col gap-3">
            <ol aria-label={t("quotaDetect.steps")} className="divide-y overflow-hidden rounded-lg border">
              {result.steps.map((step) => <StepRow key={step.adapter} step={step} />)}
            </ol>
            {found && result.snapshot ? (
              <div className="rounded-lg bg-muted px-3.5 py-3">
                <p className="text-sm font-medium">{t("quotaDetect.found", { name: adapterName(found) })}</p>
                <QuotaDetails
                  source={{ key: result.key ?? "", kind: "relay", adapter: found, users: [], snapshot: result.snapshot }}
                  label={name}
                  now={Date.now()}
                  canRefresh={false}
                />
              </div>
            ) : (
              <p role="status" className="text-sm text-muted-foreground">{t("quotaDetect.notFound")}</p>
            )}
          </div>
        ) : null}

        <DialogFooter>
          {!detect.isFetching && !found && (
            <Button type="button" variant="ghost" onClick={() => void detect.refetch()}>
              <RotateCcw /> {t("quotaDetect.retry")}
            </Button>
          )}
          <Button type="button" variant="outline" onClick={onClose}>{t("common.cancel")}</Button>
          {found && (
            <Button type="button" onClick={() => { onUse(found); onClose(); }}>
              {t("quotaDetect.use", { name: adapterName(found) })}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
