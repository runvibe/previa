import { useState, useRef, useCallback } from "react";
import { Slider } from "@/components/ui/slider";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import { Settings, FileCode2, Sparkles, Eye, EyeOff, PanelLeft, PanelRight, Globe, Moon, Sun, Workflow, List, MousePointerClick, Palette } from "lucide-react";
import { useEditorFormatStore } from "@/stores/useEditorFormatStore";
import { useOpenAIKeyStore, OPENAI_MODELS } from "@/stores/useOpenAIKeyStore";
import { useChatPositionStore } from "@/stores/useChatPositionStore";
import type { OpenAIModel } from "@/stores/useOpenAIKeyStore";
import { useThemeStore } from "@/stores/useThemeStore";
import { useStepViewStore } from "@/stores/useStepViewStore";
import type { StepViewMode } from "@/stores/useStepViewStore";
import { useAutoScrollStore } from "@/stores/useAutoScrollStore";
import { setExperimentalFeaturesEnabled, useExperimentalFeaturesEnabled } from "@/stores/useExperimentalFeaturesStore";
import { Switch } from "@/components/ui/switch";
import type { FormatType } from "@/lib/pipeline-schema";
import { ALL_PALETTES } from "@/lib/theme-palettes";
import type { PaletteId } from "@/lib/theme-palettes";
import { isComplexPalette } from "@/lib/theme-palettes";
import { cn } from "@/lib/utils";

export function ProjectSettingsDialog() {
  const { t, i18n } = useTranslation();
  const [open, setOpen] = useState(false);
  const { format, setFormat } = useEditorFormatStore();
  const { apiKey, setApiKey, model, setModel } = useOpenAIKeyStore();
  const { position: chatPosition, setPosition: setChatPosition } = useChatPositionStore();
  const { theme, setTheme: applyTheme, palette, setPalette: applyPalette, glassLevel, setGlassLevel } = useThemeStore();
  const { mode: stepViewMode, setMode: setStepViewMode } = useStepViewStore();
  const { enabled: autoScrollEnabled, setEnabled: setAutoScrollEnabled } = useAutoScrollStore();
  const experimentalFeaturesEnabled = useExperimentalFeaturesEnabled();
  const [showKey, setShowKey] = useState(false);

  const isComplex = isComplexPalette(palette);

  // Debounce timer for API key input
  const apiKeyTimer = useRef<ReturnType<typeof setTimeout>>();

  const handleOpen = (isOpen: boolean) => {
    setOpen(isOpen);
    if (isOpen) {
      setShowKey(false);
    }
  };

  const handleApiKeyChange = useCallback((value: string) => {
    clearTimeout(apiKeyTimer.current);
    apiKeyTimer.current = setTimeout(() => {
      setApiKey(value.trim() || null);
    }, 600);
  }, [setApiKey]);

  return (
    <Dialog open={open} onOpenChange={handleOpen}>
      <DialogTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8"
          title={t("settings.tooltip")}
        >
          <Settings className="h-4 w-4" />
        </Button>
      </DialogTrigger>
      <DialogContent className="sm:max-w-3xl max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Settings className="h-4 w-4" />
            {t("settings.title")}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-6 py-2">
          {/* ── AI Assistant ── */}
          {experimentalFeaturesEnabled && (
            <>
              <section className="space-y-4">
                <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground flex items-center gap-2">
                  <Sparkles className="h-3.5 w-3.5" />
                  AI Assistant
                </h3>
                <div className="grid grid-cols-2 gap-x-8 gap-y-4">
                  <div className="space-y-2">
                    <Label htmlFor="openai-key" className="text-sm font-medium">
                      {t("settings.openai.label")}
                    </Label>
                    <div className="relative">
                      <Input
                        id="openai-key"
                        type={showKey ? "text" : "password"}
                        placeholder="sk-..."
                        defaultValue={apiKey || ""}
                        onChange={(e) => handleApiKeyChange(e.target.value)}
                        className="font-mono text-xs pr-9"
                      />
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="absolute right-0 top-0 h-full w-9"
                        onClick={() => setShowKey(!showKey)}
                      >
                        {showKey ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
                      </Button>
                    </div>
                    <p className="text-xs text-muted-foreground">{t("settings.openai.description")}</p>
                  </div>

                  <div className="space-y-2">
                    <Label className="text-sm font-medium">{t("settings.model.label")}</Label>
                    <ToggleGroup
                      type="single"
                      value={model}
                      onValueChange={(value) => {
                        if (value) setModel(value as OpenAIModel);
                      }}
                      className="justify-start flex-wrap"
                    >
                      {OPENAI_MODELS.map((m) => (
                        <ToggleGroupItem key={m.value} value={m.value} className="text-xs px-3" title={m.description}>
                          {m.label}
                        </ToggleGroupItem>
                      ))}
                    </ToggleGroup>
                    <p className="text-xs text-muted-foreground">{t("settings.model.description")}</p>
                  </div>
                </div>
              </section>

              <hr className="border-border" />
            </>
          )}

          {/* ── Editor & Layout ── */}
          <section className="space-y-4">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground flex items-center gap-2">
              <FileCode2 className="h-3.5 w-3.5" />
              {t("settings.editorFormat.label", "Editor & Layout")}
            </h3>
            <div className="grid grid-cols-2 gap-x-8 gap-y-4">
              <div className="space-y-2">
                <Label className="text-sm font-medium">{t("settings.editorFormat.label")}</Label>
                <ToggleGroup
                  type="single"
                  value={format}
                  onValueChange={(value) => {
                    if (value) setFormat(value as FormatType);
                  }}
                  className="justify-start"
                >
                  <ToggleGroupItem value="json" className="text-xs px-3">JSON</ToggleGroupItem>
                  <ToggleGroupItem value="yaml" className="text-xs px-3">YAML</ToggleGroupItem>
                </ToggleGroup>
                <p className="text-xs text-muted-foreground">{t("settings.editorFormat.description")}</p>
              </div>

              {experimentalFeaturesEnabled && (
                <div className="space-y-2">
                  <Label className="flex items-center gap-2 text-sm font-medium">
                    <PanelRight className="h-3.5 w-3.5" />
                    {t("settings.chatPosition.label")}
                  </Label>
                  <ToggleGroup
                    type="single"
                    value={chatPosition}
                    onValueChange={(value) => {
                      if (value) setChatPosition(value as "left" | "right");
                    }}
                    className="justify-start"
                  >
                    <ToggleGroupItem value="left" className="text-xs px-3 gap-1.5">
                      <PanelLeft className="h-3.5 w-3.5" />
                      {t("settings.chatPosition.left")}
                    </ToggleGroupItem>
                    <ToggleGroupItem value="right" className="text-xs px-3 gap-1.5">
                      <PanelRight className="h-3.5 w-3.5" />
                      {t("settings.chatPosition.right")}
                    </ToggleGroupItem>
                  </ToggleGroup>
                </div>
              )}

              <div className="space-y-2">
                <Label className="flex items-center gap-2 text-sm font-medium">
                  <Workflow className="h-3.5 w-3.5" />
                  {t("settings.stepView.label", "Step View")}
                </Label>
                <ToggleGroup
                  type="single"
                  value={stepViewMode}
                  onValueChange={(value) => {
                    if (value) setStepViewMode(value as StepViewMode);
                  }}
                  className="justify-start"
                >
                  <ToggleGroupItem value="graph" className="text-xs px-3 gap-1.5">
                    <Workflow className="h-3.5 w-3.5" />
                    {t("settings.stepView.graph", "Graph")}
                  </ToggleGroupItem>
                  <ToggleGroupItem value="list" className="text-xs px-3 gap-1.5">
                    <List className="h-3.5 w-3.5" />
                    {t("settings.stepView.list", "List")}
                  </ToggleGroupItem>
                </ToggleGroup>
              </div>

              <div className="space-y-2">
                <Label className="flex items-center gap-2 text-sm font-medium">
                  <MousePointerClick className="h-3.5 w-3.5" />
                  {t("settings.autoScroll.label", "Auto-scroll")}
                </Label>
                <div className="flex items-center gap-3">
                  <Switch checked={autoScrollEnabled} onCheckedChange={setAutoScrollEnabled} />
                  <span className="text-xs text-muted-foreground">
                    {autoScrollEnabled ? t("settings.autoScroll.on", "On") : t("settings.autoScroll.off", "Off")}
                  </span>
                </div>
              </div>
            </div>
          </section>

          <hr className="border-border" />

          {/* ── Appearance ── */}
          <section className="space-y-4">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground flex items-center gap-2">
              <Palette className="h-3.5 w-3.5" />
              {t("settings.theme.label", "Appearance")}
            </h3>
            <div className="grid grid-cols-2 gap-x-8 gap-y-4">
              <div className="space-y-2">
                <Label className="flex items-center gap-2 text-sm font-medium">
                  {theme === "dark" ? <Moon className="h-3.5 w-3.5" /> : <Sun className="h-3.5 w-3.5" />}
                  Mode
                </Label>
                <ToggleGroup
                  type="single"
                  value={theme}
                  onValueChange={(value) => {
                    if (value) applyTheme(value as "dark" | "light");
                  }}
                  className={cn("justify-start", isComplex && "opacity-40 pointer-events-none")}
                >
                  <ToggleGroupItem value="light" className="text-xs px-3 gap-1.5">
                    <Sun className="h-3.5 w-3.5" />
                    Light
                  </ToggleGroupItem>
                  <ToggleGroupItem value="dark" className="text-xs px-3 gap-1.5">
                    <Moon className="h-3.5 w-3.5" />
                    Dark
                  </ToggleGroupItem>
                </ToggleGroup>
                {isComplex && (
                  <p className="text-xs text-muted-foreground">Locked by editor theme</p>
                )}
              </div>

              <div className="space-y-2">
                <Label className="flex items-center gap-2 text-sm font-medium">
                  <Eye className="h-3.5 w-3.5" />
                  {t("settings.glassLevel.label", "Glass Opacity")}
                </Label>
                <Slider
                  value={[glassLevel]}
                  onValueChange={([v]) => setGlassLevel(v)}
                  min={0}
                  max={5}
                  step={1}
                  className="w-full"
                  thumbTooltip={glassLevel === 0 ? "Transparent" : glassLevel === 5 ? "Opaque" : `Level ${glassLevel}`}
                />
              </div>

              <div className="space-y-2">
                <Label className="flex items-center gap-2 text-sm font-medium">
                  <Globe className="h-3.5 w-3.5" />
                  {t("settings.language.label")}
                </Label>
                <ToggleGroup
                  type="single"
                  value={i18n.language}
                  onValueChange={(value) => {
                    if (value) i18n.changeLanguage(value);
                  }}
                  className="justify-start flex-wrap"
                >
                  <ToggleGroupItem value="en" className="text-xs px-3">EN</ToggleGroupItem>
                  <ToggleGroupItem value="es" className="text-xs px-3">ES</ToggleGroupItem>
                  <ToggleGroupItem value="fr" className="text-xs px-3">FR</ToggleGroupItem>
                  <ToggleGroupItem value="de" className="text-xs px-3">DE</ToggleGroupItem>
                  <ToggleGroupItem value="pt-BR" className="text-xs px-3">PT</ToggleGroupItem>
                  <ToggleGroupItem value="ja" className="text-xs px-3">JA</ToggleGroupItem>
                  <ToggleGroupItem value="ko" className="text-xs px-3">KO</ToggleGroupItem>
                  <ToggleGroupItem value="zh-CN" className="text-xs px-3">ZH</ToggleGroupItem>
                </ToggleGroup>
              </div>

              {/* Themes */}
              <div className="space-y-2">
                <Label className="flex items-center gap-2 text-sm font-medium">
                  <Palette className="h-3.5 w-3.5" />
                  {t("settings.theme.label", "Themes")}
                </Label>
                <Select value={palette} onValueChange={(v) => applyPalette(v as PaletteId)}>
                  <SelectTrigger className="w-full max-w-xs">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {ALL_PALETTES.map((p) => (
                      <SelectItem key={p.id} value={p.id}>
                        <span className="flex items-center gap-2">
                          <span
                            className="h-4 w-4 rounded shrink-0 border border-white/10 inline-flex items-center justify-center gap-px"
                            style={{ background: p.bgPreview ?? p.primaryPreview }}
                          >
                            <span className="h-1 w-1 rounded-full" style={{ background: p.primaryPreview }} />
                            <span className="h-1 w-1 rounded-full" style={{ background: p.secondaryPreview }} />
                          </span>
                          <span>{p.label}</span>
                        </span>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
          </section>

          <hr className="border-border" />

          {/* ── Experimental Features ── */}
          <section className="space-y-4">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground flex items-center gap-2">
              <Sparkles className="h-3.5 w-3.5" />
              {t("settings.experimental.label", "Experimental Features")}
            </h3>
            <div className="flex items-center justify-between gap-4 rounded-md border border-border/60 px-3 py-2.5">
              <div className="space-y-1">
                <Label htmlFor="experimental-features" className="text-sm font-medium">
                  {t("settings.experimental.toggle", "Enable experimental features")}
                </Label>
                <p className="text-xs text-muted-foreground">
                  {t("settings.experimental.description", "Shows API Specs, AI Assistant settings, and AI actions across the project.")}
                </p>
              </div>
              <Switch
                id="experimental-features"
                checked={experimentalFeaturesEnabled}
                onCheckedChange={setExperimentalFeaturesEnabled}
              />
            </div>
          </section>
        </div>
      </DialogContent>
    </Dialog>
  );
}
