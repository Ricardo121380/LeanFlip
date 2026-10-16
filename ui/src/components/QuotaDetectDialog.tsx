// 首次查询中转站额度：用卡片里的地址和 Key 依次试 sub2api、NewAPI 的额度接口，认出来后
// 由调用方决定怎么保存（编辑器写进草稿，列表直接写回卡片）。打开就会联网，只能由用户点击打开。
import { useQuery } from "@tanstack/react-query";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { Check, CircleSlash, Loader2, RotateCcw, WifiOff } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { QuotaDetails } from "@/components/QuotaDetails";
import { api, errorMessage, type DetectStep, type RelayAdapter, type ToolId } from "@/lib/api";
import { adapterName, errorText, RELAY_ADAPTERS } from "@/lib/quota";

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
  krill,
  onUse,
  onDisable,
  onClose,
}: {
  tool: ToolId;
  name: string;
  settingsConfig: Record<string, unknown>;
  quotaPerUsd?: number;
  krill?: { jwt?: string; providerId?: string };
  onUse: (adapter: RelayAdapter) => void;
  /** 没认出平台时提供“不再提示”：把这张卡设为不查询。 */
  onDisable?: () => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const queryId = useId();
  const detect = useQuery({
    queryKey: krill ? ["quota-krill-detect", queryId] : ["quota-detect", tool, JSON.stringify(settingsConfig), quotaPerUsd ?? null],
    queryFn: () => krill ? api.quotaKrillDetect(tool, krill.jwt, krill.providerId) : api.quotaDetect(tool, settingsConfig, quotaPerUsd),
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
          <DialogDescription>{t(krill ? "quotaDetect.krillHint" : "quotaDetect.hint")}</DialogDescription>
        </DialogHeader>

        {detect.isFetching ? (
          <p role="status" className="flex items-center gap-2 py-3 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" /> {t("quotaDetect.running", { list: krill ? "Krill" : RELAY_ADAPTERS.filter((a) => a !== "krill").map(adapterName).join("、") })}
          </p>
        ) : detect.isError ? (
          <p role="alert" className="py-2 text-sm text-destructive">{errorMessage(detect.error)}</p>
        ) : result?.error === "noEndpoint" || result?.error === "noKrillJwt" ? (
          <p role="status" className="py-2 text-sm text-muted-foreground">{t(result.error === "noKrillJwt" ? "quotaDetect.noKrillJwt" : "quotaDetect.noEndpoint")}</p>
        ) : result ? (
          <div className="flex flex-col gap-3">
            <ol aria-label={t("quotaDetect.steps")} className="divide-y overflow-hidden rounded-lg border">
                {result.steps.map((step) => <StepRow key={`${step.adapter}:${step.url}`} step={step} />)}
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
              <p role="status" className="text-sm text-muted-foreground">{krill && result.steps[0]?.code
                ? errorText({ code: result.steps[0].code, status: result.steps[0].status, at: Date.now() }, t)
                : t("quotaDetect.notFound")}</p>
            )}
          </div>
        ) : null}

        <DialogFooter>
          {!detect.isFetching && !found && (
            <Button type="button" variant="ghost" onClick={() => void detect.refetch()}>
              <RotateCcw /> {t("quotaDetect.retry")}
            </Button>
          )}
          {!detect.isFetching && result && !found && onDisable && (
            <Button type="button" variant="outline" onClick={() => { onDisable(); onClose(); }}>{t("quotaDetect.disable")}</Button>
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
