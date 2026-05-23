import { useTranslation } from "react-i18next";
import { ServerCog } from "lucide-react";

import { Progress } from "@/components/ui/progress";
import type { LoadProvisioningStatus } from "@/types/load-test";

interface LoadProvisioningStatusPanelProps {
  status: LoadProvisioningStatus | null;
  startedAt: number | null;
}

function formatElapsed(startedAt: number | null) {
  if (!startedAt) return "0s";
  const seconds = Math.max(0, Math.floor((Date.now() - startedAt) / 1000));
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return minutes > 0 ? `${minutes}m ${rest}s` : `${rest}s`;
}

export function LoadProvisioningStatusPanel({
  status,
  startedAt,
}: LoadProvisioningStatusPanelProps) {
  const { t } = useTranslation();
  const requested = Math.max(0, status?.requestedRunnerCount ?? 0);
  const ready = Math.max(0, status?.readyRunnerCount ?? 0);
  const progress = requested > 0 ? Math.min(100, Math.round((ready / requested) * 100)) : 0;
  const elapsed = formatElapsed(startedAt);

  return (
    <section
      data-testid="load-provisioning-status"
      className="rounded-lg border border-border bg-card p-4 shadow-sm"
    >
      <div className="flex items-start gap-3">
        <div className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
          <ServerCog className="h-5 w-5" />
        </div>
        <div className="min-w-0 flex-1 space-y-3">
          <div className="min-w-0">
            <h3 className="text-sm font-semibold text-foreground">
              {t("loadTest.provisioning.title")}
            </h3>
            <p className="text-xs text-muted-foreground">
              {status?.unavailable
                ? t("loadTest.provisioning.unavailable")
                : requested > 0
                  ? t("loadTest.provisioning.subtitle", { ready, requested })
                  : t("loadTest.provisioning.waiting")}
            </p>
          </div>

          <Progress value={progress} className="h-2.5" />

          <div className="grid gap-2 text-xs text-muted-foreground sm:grid-cols-2">
            <span>{t("loadTest.provisioning.status")}: {status?.reservationStatus ?? "pending"}</span>
            <span>{t("loadTest.provisioning.elapsed")}: {elapsed}</span>
            {status?.reservationId && (
              <span className="truncate">{t("loadTest.provisioning.reservation")}: {status.reservationId}</span>
            )}
            {status?.targetRps ? (
              <span>{t("loadTest.provisioning.targetRps")}: {status.targetRps}</span>
            ) : null}
            {status?.nodeProfile && (
              <span>{t("loadTest.provisioning.nodeProfile")}: {status.nodeProfile}</span>
            )}
          </div>

          {status?.message && (
            <p className="text-xs text-muted-foreground">{status.message}</p>
          )}
        </div>
      </div>
    </section>
  );
}
