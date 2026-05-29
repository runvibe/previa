import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Copy, ExternalLink } from "lucide-react";
import { toast } from "sonner";

import { PreviaLogo } from "@/components/PreviaLogo";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

const ONBOARDING_STORAGE_KEY = "previa:onboarding:v1";

interface OnboardingModalProps {
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}

type GuideSectionId = "inicio" | "instalacao" | "primeira-execucao" | "mais-informacoes";

interface GuideSection {
  id: GuideSectionId;
  title: string;
  summary: string;
}

function CodeBlock({ children }: { children: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(children);
      setCopied(true);
      toast.success(t("guide.copySuccess"));
      window.setTimeout(() => setCopied(false), 2000);
    } catch (error) {
      console.error(t("guide.copyError"), error);
    }
  };

  return (
    <div className="relative rounded-xl border border-border/70 bg-muted px-3 py-3 pr-14 sm:px-4 sm:py-3 sm:pr-16">
      <Button
        type="button"
        variant="ghost"
        size="sm"
        onClick={handleCopy}
        className="absolute right-2 top-2 h-7 gap-1.5 px-2 sm:right-3 sm:top-[0.4rem] sm:h-8 sm:gap-2"
      >
        {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
        <span className="hidden sm:inline">{copied ? t("guide.copied") : t("guide.copy")}</span>
      </Button>
      <pre className="overflow-x-auto text-xs text-foreground sm:text-sm">
        <code>{children}</code>
      </pre>
    </div>
  );
}

export function OnboardingModal({ open: controlledOpen, onOpenChange }: OnboardingModalProps) {
  const { t } = useTranslation();
  const [internalOpen, setInternalOpen] = useState(false);
  const [activeSection, setActiveSection] = useState<GuideSectionId>("inicio");
  const isControlled = controlledOpen !== undefined;
  const open = isControlled ? controlledOpen : internalOpen;

  const guideSections = useMemo<GuideSection[]>(
    () => [
      {
        id: "inicio",
        title: t("guide.sections.inicio.title"),
        summary: t("guide.sections.inicio.summary"),
      },
      {
        id: "instalacao",
        title: t("guide.sections.instalacao.title"),
        summary: t("guide.sections.instalacao.summary"),
      },
      {
        id: "primeira-execucao",
        title: t("guide.sections.primeiraExecucao.title"),
        summary: t("guide.sections.primeiraExecucao.summary"),
      },
      {
        id: "mais-informacoes",
        title: t("guide.sections.maisInformacoes.title"),
        summary: t("guide.sections.maisInformacoes.summary"),
      },
    ],
    [t],
  );

  useEffect(() => {
    if (isControlled) return;

    const hasSeenOnboarding = window.localStorage.getItem(ONBOARDING_STORAGE_KEY) === "seen";
    if (!hasSeenOnboarding) {
      setInternalOpen(true);
    }
  }, [isControlled]);

  useEffect(() => {
    if (open) {
      setActiveSection("inicio");
    }
  }, [open]);

  const handleOpenChange = (nextOpen: boolean) => {
    if (!isControlled) {
      setInternalOpen(nextOpen);
    }

    onOpenChange?.(nextOpen);

    if (!nextOpen) {
      window.localStorage.setItem(ONBOARDING_STORAGE_KEY, "seen");
    }
  };

  const activeGuide = useMemo(
    () => guideSections.find((section) => section.id === activeSection) ?? guideSections[0],
    [activeSection, guideSections],
  );

  const content = useMemo(() => {
    switch (activeSection) {
      case "inicio":
        return (
          <div className="space-y-5">
            <div className="space-y-2">
              <p className="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground sm:text-sm">
                {t("guide.title")}
              </p>
              <h3 className="text-2xl font-semibold tracking-tight text-foreground sm:text-3xl">
                {t("guide.start.title")}
              </h3>
              <p className="max-w-2xl text-sm leading-6 text-muted-foreground">{t("guide.start.description")}</p>
            </div>

            <div className="grid gap-3 sm:grid-cols-2">
              <div className="rounded-2xl border border-border/70 bg-muted/40 p-4">
                <p className="text-sm font-medium text-foreground">{t("guide.start.flowTitle")}</p>
                <p className="mt-1 text-sm leading-6 text-muted-foreground">{t("guide.start.flowDescription")}</p>
              </div>
              <div className="rounded-2xl border border-border/70 bg-muted/40 p-4">
                <p className="text-sm font-medium text-foreground">{t("guide.start.whenTitle")}</p>
                <p className="mt-1 text-sm leading-6 text-muted-foreground">{t("guide.start.whenDescription")}</p>
              </div>
            </div>
          </div>
        );

      case "instalacao":
        return (
          <div className="space-y-5">
            <div className="space-y-2">
              <p className="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground sm:text-sm">
                {t("guide.install.badge")}
              </p>
              <h3 className="text-2xl font-semibold tracking-tight text-foreground sm:text-3xl">
                {t("guide.install.title")}
              </h3>
              <p className="max-w-2xl text-sm leading-6 text-muted-foreground">{t("guide.install.description")}</p>
            </div>

            <CodeBlock>curl -fsSL https://raw.githubusercontent.com/runvibe/previa/main/install.sh | sh</CodeBlock>

            <ul className="space-y-2 text-sm leading-6 text-muted-foreground">
              {[1, 2, 3].map((item) => (
                <li key={item}>• {t(`guide.install.tip${item}`)}</li>
              ))}
            </ul>
          </div>
        );

      case "primeira-execucao":
        return (
          <div className="space-y-5">
            <div className="space-y-2">
              <p className="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground sm:text-sm">
                {t("guide.firstRun.badge")}
              </p>
              <h3 className="text-2xl font-semibold tracking-tight text-foreground sm:text-3xl">
                {t("guide.firstRun.title")}
              </h3>
              <p className="max-w-2xl text-sm leading-6 text-muted-foreground">{t("guide.firstRun.description")}</p>
            </div>

            <div className="space-y-4">
              <div className="space-y-2">
                <p className="text-sm font-medium text-foreground">{t("guide.firstRun.step1")}</p>
                <CodeBlock>previa up -d</CodeBlock>
              </div>

              <div className="space-y-2">
                <p className="text-sm font-medium text-foreground">{t("guide.firstRun.step2")}</p>
                <CodeBlock>previa open</CodeBlock>
              </div>

              <div className="space-y-2">
                <p className="text-sm font-medium text-foreground">{t("guide.firstRun.step3")}</p>
                <CodeBlock>previa mcp install codex --scope project</CodeBlock>
              </div>
            </div>
          </div>
        );

      case "mais-informacoes":
        return (
          <div className="space-y-5">
            <div className="space-y-2">
              <p className="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground sm:text-sm">
                {t("guide.more.badge")}
              </p>
              <h3 className="text-2xl font-semibold tracking-tight text-foreground sm:text-3xl">
                {t("guide.more.title")}
              </h3>
              <p className="max-w-2xl text-sm leading-6 text-muted-foreground">{t("guide.more.description")}</p>
            </div>

            <div className="space-y-3">
              <a
                href="https://github.com/runvibe/previa"
                target="_blank"
                rel="noreferrer"
                className="flex items-center justify-between rounded-2xl border border-border/70 bg-muted/40 px-4 py-3 text-sm text-foreground transition-colors hover:bg-accent"
              >
                <span>{t("guide.more.github")}</span>
                <ExternalLink className="h-4 w-4 text-muted-foreground" />
              </a>
              <div className="rounded-2xl border border-dashed border-border/70 bg-accent px-4 py-3 text-sm leading-6 text-muted-foreground">
                {t("guide.more.tip")}
              </div>
            </div>
          </div>
        );

      default:
        return null;
    }
  }, [activeSection, t]);

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="max-h-[calc(100vh-1rem)] w-[calc(100vw-1rem)] max-w-5xl overflow-hidden border-border/80 p-0 shadow-lg sm:max-h-[85vh]">
        <div className="flex h-full min-h-0 flex-col bg-card">
          <DialogHeader className="border-b border-border/70 px-4 py-4 text-left sm:px-6">
            <div className="flex items-center gap-3 pr-8">
              <PreviaLogo className="h-8 w-8 shrink-0 sm:h-9 sm:w-9" />
              <div>
                <DialogTitle className="text-xl font-semibold tracking-tight sm:text-2xl">{t("guide.title")}</DialogTitle>
                <DialogDescription className="sr-only">{t("guide.start.description")}</DialogDescription>
              </div>
            </div>
          </DialogHeader>

          <div className="flex min-h-0 flex-1 flex-col md:grid md:grid-cols-[224px_minmax(0,1fr)]">
            <aside className="border-b border-border/70 bg-muted/20 md:border-b-0 md:border-r" aria-label={t("guide.categories")}
            >
              <div className="flex h-full flex-col p-3 sm:p-4">
                <p className="px-1 pb-2 text-[11px] font-medium uppercase tracking-[0.18em] text-muted-foreground sm:px-2 sm:pb-3 sm:text-xs">
                  {t("guide.categories")}
                </p>
                <nav className="flex gap-2 overflow-x-auto pb-1 md:block md:space-y-2 md:overflow-visible md:pb-0">
                  {guideSections.map((section, index) => {
                    const isActive = section.id === activeGuide.id;

                    return (
                      <button
                        key={section.id}
                        type="button"
                        onClick={() => setActiveSection(section.id)}
                        aria-current={isActive ? "page" : undefined}
                        className={[
                          "min-w-[172px] shrink-0 rounded-xl border px-3 py-2 text-left transition-colors md:min-w-0 md:rounded-2xl md:px-4 md:py-3",
                          isActive
                            ? "border-border bg-accent text-foreground"
                            : "border-transparent bg-transparent text-muted-foreground hover:border-border/70 hover:bg-muted/40 hover:text-foreground",
                        ].join(" ")}
                      >
                        <p className="text-[10px] font-medium uppercase tracking-[0.16em] text-muted-foreground md:text-xs">
                          {index + 1}
                        </p>
                        <p className="mt-1 text-sm font-medium">{section.title}</p>
                        <p className="mt-1 hidden text-xs leading-5 md:block">{section.summary}</p>
                      </button>
                    );
                  })}
                </nav>
              </div>
            </aside>

            <section className="min-h-0 overflow-y-auto px-4 py-4 sm:px-6 sm:py-6">
              <div className="space-y-6">
                {content}

                <div className="border-t border-border/70 pt-4">
                  <Button onClick={() => handleOpenChange(false)}>{t("guide.close")}</Button>
                </div>
              </div>
            </section>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
