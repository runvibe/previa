import { Bot, ExternalLink, FileUp, FolderPlus } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";

interface AgentRuntimeOnboardingProps {
  onCreateStack: () => void;
  onImportStack: () => void;
}

export function AgentRuntimeOnboarding({
  onCreateStack,
  onImportStack,
}: AgentRuntimeOnboardingProps) {
  const { t } = useTranslation();

  return (
    <section className="flex flex-col items-center justify-center rounded-lg border border-dashed border-border/50 px-4 py-12 text-center sm:py-16">
      <div className="mb-4 rounded-lg border border-border/70 bg-muted p-4">
        <Bot className="h-8 w-8 text-foreground" aria-hidden="true" />
      </div>
      <h3 className="mb-2 text-lg font-semibold">{t("onboarding.agent.title")}</h3>
      <p className="mb-6 max-w-xl text-sm leading-6 text-muted-foreground sm:text-base">
        {t("onboarding.agent.description")}
      </p>
      <div className="flex flex-col gap-2 sm:flex-row">
        <Button type="button" onClick={onCreateStack}>
          <FolderPlus className="h-4 w-4" aria-hidden="true" />
          {t("onboarding.agent.create")}
        </Button>
        <Button type="button" variant="outline" onClick={onImportStack}>
          <FileUp className="h-4 w-4" aria-hidden="true" />
          {t("onboarding.agent.import")}
        </Button>
        <Button type="button" variant="ghost" asChild>
          <a
            href="https://github.com/runvibe/previa/tree/main/docs/previa"
            target="_blank"
            rel="noreferrer"
          >
            <ExternalLink className="h-4 w-4" aria-hidden="true" />
            {t("onboarding.agent.docs")}
          </a>
        </Button>
      </div>
    </section>
  );
}
