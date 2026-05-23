import { CheckCircle2, XCircle, Zap } from "lucide-react";
import { DotsLoader } from "@/components/DotsLoader";
import { formatDistanceToNow } from "date-fns";
import { ptBR, enUS } from "date-fns/locale";
import { useTranslation } from "react-i18next";
import type { LoadTestRunRecord } from "@/lib/load-test-store";

interface LoadTestRunHistoryItemProps {
  run: LoadTestRunRecord;
  isActive: boolean;
  onClick: () => void;
}

export function LoadTestRunHistoryItem({ run, isActive, onClick }: LoadTestRunHistoryItemProps) {
  const { t, i18n } = useTranslation();
  const dateLocale = i18n.language === "pt-BR" ? ptBR : enUS;
  const localeStr = i18n.language === "pt-BR" ? "pt-BR" : "en-US";
  const isRunning = run.state === "running" || run.state === "provisioning";
  const hasErrors = run.metrics.totalError > 0;
  const successRate = run.metrics.totalSent > 0
    ? Math.round((run.metrics.totalSuccess / run.metrics.totalSent) * 100)
    : 0;

  return (
    <button
      onClick={onClick}
      className={`flex flex-col gap-0.5 rounded-none px-2.5 py-2 text-[11px] font-medium transition-all duration-200 w-full text-left active:scale-[0.98] ${
        isRunning
          ? isActive
            ? "bg-primary/15 text-primary shadow-ring-primary "
            : "bg-primary/10 text-primary"
          : isActive
            ? !hasErrors
              ? "bg-success/15 text-success shadow-ring-success "
              : "bg-destructive/15 text-destructive shadow-ring-error "
            : "text-muted-foreground hover:bg-accent/60 hover:"
      }`}
    >
      <div className="flex items-center gap-1.5">
        {isRunning ? (
          <DotsLoader className="text-primary" />
        ) : !hasErrors ? (
          <CheckCircle2 className="h-3 w-3 text-success shrink-0" />
        ) : (
          <XCircle className="h-3 w-3 text-destructive shrink-0" />
        )}
        <span className="truncate">{isRunning ? t("common.inProgress") : formatDistanceToNow(new Date(run.timestamp), { addSuffix: true, locale: dateLocale })}</span>
      </div>
      <div className="flex flex-col pl-[18px] text-[10px] text-muted-foreground gap-0.5">
        <span>{new Date(run.timestamp).toLocaleDateString(localeStr)} {t("runHistory.at")} {new Date(run.timestamp).toLocaleTimeString(localeStr, { hour: "2-digit", minute: "2-digit", second: "2-digit" })}</span>
        <div className="flex items-center gap-2">
          <span>{run.metrics.totalSent} reqs</span>
          <span>{successRate}% ok</span>
          <span className="flex items-center gap-0.5"><Zap className="h-2.5 w-2.5" />{run.metrics.rps} rps</span>
        </div>
        <span>{Math.round(run.metrics.elapsedMs / 1000)}s • avg {run.metrics.avgLatency}ms</span>
      </div>
    </button>
  );
}
