import { useState, useEffect, useRef, useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { useAppHeader } from "@/components/AppShell";
import { ProjectCard } from "@/components/ProjectCard";
import { ExportDialog } from "@/components/ExportDialog";
import { ProjectSettingsDialog } from "@/components/ProjectSettingsDialog";
import { Button } from "@/components/ui/button";
import { Plus, FolderOpen, Upload } from "lucide-react";
import { useProjectStore } from "@/stores/useProjectStore";
import { useOrchestratorStore } from "@/stores/useOrchestratorStore";
import { importProjectFile } from "@/lib/project-io";
import type { Project } from "@/types/project";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { toast } from "sonner";

export default function ProjectsPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { projects, loading, loadProjects, loadProject, createProject, updateProject, deleteProject, duplicateProject } = useProjectStore();
  const orchUrl = useOrchestratorStore((s) => s.url);
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);
  const [projectToDelete, setProjectToDelete] = useState<string | null>(null);
  const [exportProject, setExportProject] = useState<Project | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    loadProjects();
    if (orchUrl) useOrchestratorStore.getState().fetchInfo();
  }, [loadProjects, orchUrl]);

  const headerActions = useMemo(() => <ProjectSettingsDialog />, []);
  useAppHeader({ headerActions });

  const handleCreateProject = async () => {
    try {
      const projectNumber = projects.length + 1;
      const newProject = await createProject({ name: t("projects.defaultName", { number: projectNumber }) });
      navigate(`/projects/${newProject.id}`);
    } catch {
      // Error already toasted by store
    }
  };

  const handleOpenProject = (id: string) => {
    navigate(`/projects/${id}`);
  };

  const handleDuplicateProject = async (id: string) => {
    const newProject = await duplicateProject(id);
    if (newProject) {
      toast.success(t("projects.duplicated"));
    }
  };

  const handleDeleteClick = (id: string) => {
    setProjectToDelete(id);
    setDeleteDialogOpen(true);
  };

  const handleExportClick = async (id: string) => {
    const project = await loadProject(id);
    if (project) setExportProject(project);
  };

  const handleImportFile = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    try {
      await importProjectFile(file);
      await loadProjects();
      toast.success(t("projects.imported"));
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : t("projects.importError"));
    }
    e.target.value = "";
  };

  const handleConfirmDelete = async () => {
    if (projectToDelete) {
      await deleteProject(projectToDelete);
      toast.success(t("projects.deleted"));
    }
    setDeleteDialogOpen(false);
    setProjectToDelete(null);
  };

  const projectToDeleteName = projects.find(p => p.id === projectToDelete)?.name;

  return (
    <main className="flex-1 p-4 sm:p-6">
      <div className="mx-auto max-w-6xl">
        <div className="mb-6 sm:mb-8 flex flex-col sm:flex-row sm:items-center justify-between gap-4">
            <div>
              <h2 className="text-xl sm:text-2xl font-bold">{t("projects.title")}</h2>
              <p className="text-muted-foreground mt-1 text-sm sm:text-base">
                {t("projects.subtitle")}
              </p>
            </div>
            <div className="flex flex-wrap gap-2">
              <input
                ref={fileInputRef}
                type="file"
                accept=".json,.sqlite,.sqlite3,.db,application/vnd.sqlite3,application/x-sqlite3"
                className="hidden"
                onChange={handleImportFile}
              />
              <Button variant="outline" size="sm" className="sm:size-default" onClick={() => fileInputRef.current?.click()}>
                <Upload className="h-4 w-4" />
                {t("common.import")}
              </Button>
              <Button size="sm" className="sm:size-default" onClick={handleCreateProject}>
                <Plus className="h-4 w-4" />
                {t("projects.new")}
              </Button>
            </div>
          </div>

          {loading ? (
            <div className="flex items-center justify-center py-16">
              <p className="text-muted-foreground">{t("projects.loading")}</p>
            </div>
          ) : projects.length > 0 ? (
            <div className="grid gap-4 grid-cols-1 sm:grid-cols-2 lg:grid-cols-3">
              {projects.map((project, i) => (
                <div key={project.id} className="animate-slide-up" style={{ animationDelay: `${i * 80}ms`, opacity: 0 }}>
                  <ProjectCard
                    project={project}
                    onOpen={handleOpenProject}
                    onDuplicate={handleDuplicateProject}
                    onDelete={handleDeleteClick}
                    onExport={handleExportClick}
                    onRename={async (id, newName) => {
                      await updateProject(id, { name: newName });
                      toast.success(t("projects.renamed"));
                    }}
                  />
                </div>
              ))}
            </div>
          ) : (
            <div className="flex flex-col items-center justify-center rounded-lg border border-dashed border-border/50 py-12 sm:py-16 px-4 animate-fade-in">
              <div className="rounded-xl bg-gradient-to-br from-primary/10 to-primary/5 p-5 shadow-primary-glow mb-4">
                <FolderOpen className="h-8 w-8 text-muted-foreground" />
              </div>
              <h3 className="text-lg font-semibold mb-2">{t("projects.empty.title")}</h3>
              <p className="text-muted-foreground mb-6 text-center max-w-sm text-sm sm:text-base">
                {t("projects.empty.description")}
              </p>
              <Button onClick={handleCreateProject}>
                <Plus className="h-4 w-4" />
                {t("projects.empty.button")}
              </Button>
            </div>
          )}
      </div>

      <ConfirmDialog
        open={deleteDialogOpen}
        onOpenChange={setDeleteDialogOpen}
        title={t("projects.deleteConfirm.title")}
        description={t("projects.deleteConfirm.description", { name: projectToDeleteName })}
        confirmLabel={t("common.delete")}
        variant="destructive"
        onConfirm={handleConfirmDelete}
      />

      <ExportDialog
        project={exportProject}
        open={!!exportProject}
        onOpenChange={(open) => { if (!open) setExportProject(null); }}
      />
    </main>
  );
}
