import { AlertCircle, RotateCcw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";

export function QueryState({ state, title, description, onRetry }: {
  state: "loading" | "error";
  title: string;
  description?: string;
  onRetry?: () => void;
}) {
  const { t } = useTranslation();
  if (state === "loading") return (
    <div className="space-y-6 p-6" role="status" aria-busy="true">
      <span className="sr-only">{title}</span>
      <div className="space-y-3" aria-hidden="true">
        <div className="h-7 w-48 rounded bg-muted" />
        <div className="h-4 w-72 max-w-full rounded bg-muted" />
      </div>
      <div className="grid gap-3 lg:grid-cols-2" aria-hidden="true">
        {[0, 1].map((key) => <div key={key} className="h-36 rounded-xl border bg-muted/30" />)}
      </div>
    </div>
  );
  return (
    <div className="p-6" role="alert">
      <div className="flex max-w-xl items-start gap-3 rounded-xl border border-destructive/25 bg-destructive/5 p-5">
        <AlertCircle className="mt-0.5 h-5 w-5 shrink-0 text-destructive" aria-hidden="true" />
        <div className="min-w-0 space-y-3">
          <h2 className="font-semibold">{title}</h2>
          {description && <p className="break-words text-sm text-muted-foreground">{description}</p>}
          {onRetry && <Button type="button" size="sm" variant="outline" onClick={onRetry}><RotateCcw aria-hidden="true" />{t("common.retry")}</Button>}
        </div>
      </div>
    </div>
  );
}
