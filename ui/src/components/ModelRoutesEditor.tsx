import { useTranslation } from "react-i18next";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { routeErrors, type RouteRow } from "@/lib/routes";
import { cn } from "@/lib/utils";

interface Props {
  rows: RouteRow[];
  onChange: (rows: RouteRow[]) => void;
}

/** Claude Desktop 模型菜单：模型名、显示名、1M 上下文。 */
export function ModelRoutesEditor({ rows, onChange }: Props) {
  const { t } = useTranslation();
  const errors = routeErrors(rows);
  const update = (i: number, patch: Partial<RouteRow>) => onChange(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center justify-between">
        <Label>{t("editor.routes")}</Label>
        <Button
          variant="outline"
          size="sm"
          onClick={() => onChange([...rows, { id: "claude-", label: "", supports1m: false }])}
        >
          <Plus /> {t("editor.routeAdd")}
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">{t("editor.routesHint")}</p>
      {rows.length > 0 && (
        <div className="flex flex-col gap-1.5 rounded-md border p-2">
          {rows.map((row, i) => (
            <div key={i} className="flex flex-col gap-0.5">
              <div className="flex items-center gap-2">
                <Input
                  aria-label={t("editor.routeId")}
                  placeholder="claude-sonnet-4-5"
                  value={row.id}
                  aria-invalid={errors[i] !== null}
                  className={cn("flex-[3] font-mono text-xs", errors[i] && "border-destructive")}
                  onChange={(e) => update(i, { id: e.target.value })}
                />
                <Input
                  aria-label={t("editor.routeLabel")}
                  placeholder={t("editor.routeLabel")}
                  value={row.label}
                  className="flex-[2] text-xs"
                  onChange={(e) => update(i, { label: e.target.value })}
                />
                <label className="flex shrink-0 items-center gap-1 text-xs">
                  <Checkbox
                    checked={row.supports1m}
                    aria-label={t("editor.route1m")}
                    onCheckedChange={(v) => update(i, { supports1m: v === true })}
                  />
                  1M
                </label>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("editor.routeRemove")}
                  onClick={() => onChange(rows.filter((_, j) => j !== i))}
                >
                  <Trash2 />
                </Button>
              </div>
              {errors[i] && (
                <p role="alert" className="text-xs text-destructive">
                  {t(errors[i] === "duplicate" ? "editor.routeDuplicate" : "editor.routeInvalid")}
                </p>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
