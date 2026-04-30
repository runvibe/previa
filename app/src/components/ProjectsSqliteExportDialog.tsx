import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Download } from "lucide-react";
import type { Project } from "@/types/project";
import { exportProjectsSqlite } from "@/lib/project-io";
import { toast } from "sonner";

interface ProjectsSqliteExportDialogProps {
  projects: Project[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function ProjectsSqliteExportDialog({
  projects,
  open,
  onOpenChange,
}: ProjectsSqliteExportDialogProps) {
  const { t } = useTranslation();
  const [includeHistory, setIncludeHistory] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set());

  useEffect(() => {
    if (open) {
      setSelectedIds(new Set());
      setIncludeHistory(false);
      setExporting(false);
    }
  }, [open]);

  const allSelected = projects.length > 0 && selectedIds.size === projects.length;
  const selectedProjectIds = useMemo(() => Array.from(selectedIds), [selectedIds]);

  const toggleProject = (projectId: string, checked: boolean) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (checked) {
        next.add(projectId);
      } else {
        next.delete(projectId);
      }
      return next;
    });
  };

  const toggleAll = (checked: boolean) => {
    setSelectedIds(checked ? new Set(projects.map((project) => project.id)) : new Set());
  };

  const handleExport = async () => {
    if (selectedIds.size === 0) return;
    setExporting(true);
    try {
      await exportProjectsSqlite(selectedProjectIds, allSelected, includeHistory);
      toast.success(t("export.sqlite.success"));
      onOpenChange(false);
    } catch {
      toast.error(t("export.sqlite.error"));
    } finally {
      setExporting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("export.sqlite.title")}</DialogTitle>
          <DialogDescription>
            {t("export.sqlite.description")}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-2">
          <div className="flex items-center justify-between gap-3 rounded-md border border-border/50 px-3 py-2">
            <div className="flex items-center gap-2">
              <Checkbox
                id="select-all-projects"
                checked={allSelected}
                onCheckedChange={(value) => toggleAll(value === true)}
              />
              <Label htmlFor="select-all-projects">{t("common.selectAll")}</Label>
            </div>
            <span className="text-xs text-muted-foreground">
              {t("export.sqlite.selectedCount", { count: selectedIds.size, total: projects.length })}
            </span>
          </div>

          <div className="max-h-64 space-y-2 overflow-y-auto pr-1">
            {projects.map((project) => (
              <label
                key={project.id}
                htmlFor={`export-project-${project.id}`}
                className="flex cursor-pointer items-center gap-3 rounded-md border border-border/50 px-3 py-2 transition-colors hover:bg-accent/40"
              >
                <Checkbox
                  id={`export-project-${project.id}`}
                  checked={selectedIds.has(project.id)}
                  onCheckedChange={(value) => toggleProject(project.id, value === true)}
                />
                <span className="min-w-0 flex-1 truncate text-sm font-medium">{project.name}</span>
              </label>
            ))}
          </div>

          <div className="flex items-center space-x-2">
            <Checkbox
              id="include-history-sqlite"
              checked={includeHistory}
              onCheckedChange={(value) => setIncludeHistory(value === true)}
            />
            <Label htmlFor="include-history-sqlite">{t("export.includeHistory")}</Label>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button onClick={handleExport} disabled={exporting || selectedIds.size === 0}>
            <Download className="h-4 w-4" />
            {exporting ? t("export.exporting") : t("common.export")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
