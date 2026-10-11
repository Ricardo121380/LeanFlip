import { useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { useRecoveryNotices } from "@/hooks/queries";

/** 持续显示到用户关闭；之后新发生的恢复仍会显示。读取后端不会清空提示。 */
export function RecoveryNotices() {
  const { t } = useTranslation();
  const { data = [], error } = useRecoveryNotices();
  const [dismissed, setDismissed] = useState(0);
  const notices = data.slice(dismissed);
  if (!notices.length && !error) return null;

  return (
    <section role="alert" className="m-6 rounded-md border border-amber-500/40 bg-amber-500/10 p-4 text-sm">
      <div className="flex items-start justify-between gap-4">
        <h2 className="font-medium">{t("recovery.title")}</h2>
        {!error && (
          <button type="button" aria-label={t("recovery.dismiss")} onClick={() => setDismissed(data.length)}>
            <X className="h-4 w-4" />
          </button>
        )}
      </div>
      {error && <p className="mt-2">{t("recovery.loadError")}</p>}
      <ul className="mt-2 space-y-2">
        {notices.map((notice, index) => <li key={dismissed + index} className="whitespace-pre-wrap break-all">{notice}</li>)}
      </ul>
    </section>
  );
}
