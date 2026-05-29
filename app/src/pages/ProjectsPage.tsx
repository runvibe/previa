import { useState, useEffect, useRef, useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { AgentRuntimeOnboarding } from "@/components/AgentRuntimeOnboarding";
import { useAppHeader } from "@/components/AppShell";
import { ProjectCard } from "@/components/ProjectCard";
import { ProjectsSqliteExportDialog } from "@/components/ProjectsSqliteExportDialog";
import { ProjectSettingsDialog } from "@/components/ProjectSettingsDialog";
import { ProjectSharingDialog } from "@/components/ProjectSharingDialog";
import { ProjectTagsDialog } from "@/components/ProjectTagsDialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ChevronLeft, ChevronRight, Plus, Upload, Download, Search, X } from "lucide-react";
import { useProjectStore } from "@/stores/useProjectStore";
import { useOrchestratorStore } from "@/stores/useOrchestratorStore";
import { exportProjectsSqlite, importProjectFile } from "@/lib/project-io";
import { collectProjectTags, filterProjectsBySearchAndTags } from "@/lib/project-tags";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { toast } from "sonner";

const PROJECTS_PER_PAGE = 10;

export default function ProjectsPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { projects, loading, loadProjects, createProject, updateProject, deleteProject, duplicateProject } = useProjectStore();
  const orchUrl = useOrchestratorStore((s) => s.url);
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);
  const [sqliteExportOpen, setSqliteExportOpen] = useState(false);
  const [projectToDelete, setProjectToDelete] = useState<string | null>(null);
  const [projectToEditTags, setProjectToEditTags] = useState<string | null>(null);
  const [projectToShare, setProjectToShare] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedTags, setSelectedTags] = useState<string[]>([]);
  const [currentPage, setCurrentPage] = useState(1);
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

  const handleOpenProjectDashboard = (id: string) => {
    navigate(`/projects/${id}/dashboard`);
  };

  const handleDuplicateProject = async (id: string) => {
    const newProject = await duplicateProject(id);
    if (newProject) {
      await loadProjects();
      toast.success(t("projects.duplicated"));
    }
  };

  const handleDeleteClick = (id: string) => {
    setProjectToDelete(id);
    setDeleteDialogOpen(true);
  };

  const handleExportClick = async (id: string) => {
    try {
      await exportProjectsSqlite([id], false, false);
      toast.success(t("export.sqlite.success"));
    } catch {
      toast.error(t("export.sqlite.error"));
    }
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
    try {
      if (projectToDelete) {
        await deleteProject(projectToDelete);
        toast.success(t("projects.deleted"));
      }
    } catch {
      // The store reports the error toast and keeps the stack in state.
    } finally {
      setDeleteDialogOpen(false);
      setProjectToDelete(null);
    }
  };

  const projectToDeleteName = projects.find(p => p.id === projectToDelete)?.name;
  const tagProject = projects.find((project) => project.id === projectToEditTags);
  const shareProject = projects.find((project) => project.id === projectToShare) ?? null;
  const availableTags = useMemo(() => collectProjectTags(projects), [projects]);
  const filteredProjects = useMemo(
    () => filterProjectsBySearchAndTags(projects, searchQuery, selectedTags),
    [projects, searchQuery, selectedTags],
  );
  const hasFilters = searchQuery.trim().length > 0 || selectedTags.length > 0;
  const totalPages = Math.max(1, Math.ceil(filteredProjects.length / PROJECTS_PER_PAGE));
  const safeCurrentPage = Math.min(currentPage, totalPages);
  const pageStartIndex = (safeCurrentPage - 1) * PROJECTS_PER_PAGE;
  const paginatedProjects = filteredProjects.slice(pageStartIndex, pageStartIndex + PROJECTS_PER_PAGE);
  const visibleStart = filteredProjects.length === 0 ? 0 : pageStartIndex + 1;
  const visibleEnd = Math.min(pageStartIndex + paginatedProjects.length, filteredProjects.length);

  useEffect(() => {
    setCurrentPage(1);
  }, [searchQuery, selectedTags]);

  useEffect(() => {
    setCurrentPage((page) => Math.min(page, totalPages));
  }, [totalPages]);

  const toggleTagFilter = (tag: string) => {
    setSelectedTags((current) => (
      current.includes(tag)
        ? current.filter((item) => item !== tag)
        : [...current, tag]
    ));
  };

  const clearFilters = () => {
    setSearchQuery("");
    setSelectedTags([]);
  };

  return (
    <main className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden p-4 sm:p-6">
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
              <Button
                variant="outline"
                size="sm"
                className="sm:size-default"
                onClick={() => setSqliteExportOpen(true)}
                disabled={loading || projects.length === 0}
              >
                <Download className="h-4 w-4" />
                {t("common.export")}
              </Button>
              <Button size="sm" className="sm:size-default" onClick={handleCreateProject}>
                <Plus className="h-4 w-4" />
                {t("projects.new")}
              </Button>
            </div>
          </div>

          {projects.length > 0 && (
            <div className="mb-5 space-y-3">
              <div className="relative">
                <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
                <Input
                  value={searchQuery}
                  onChange={(event) => setSearchQuery(event.target.value)}
                  placeholder={t("projects.filters.searchPlaceholder")}
                  className="pl-9"
                />
              </div>

              {availableTags.length > 0 && (
                <div className="flex flex-wrap gap-2">
                  {availableTags.map((tag) => {
                    const selected = selectedTags.includes(tag);
                    return (
                      <Button
                        key={tag}
                        type="button"
                        variant={selected ? "default" : "outline"}
                        size="sm"
                        className="h-7 px-2 text-xs"
                        onClick={() => toggleTagFilter(tag)}
                      >
                        {tag}
                      </Button>
                    );
                  })}
                  {hasFilters && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-7 px-2 text-xs"
                      onClick={clearFilters}
                    >
                      <X className="h-3.5 w-3.5" />
                      {t("projects.filters.clear")}
                    </Button>
                  )}
                </div>
              )}
            </div>
          )}

          {loading ? (
            <div className="flex items-center justify-center py-16">
              <p className="text-muted-foreground">{t("projects.loading")}</p>
            </div>
          ) : projects.length > 0 && filteredProjects.length > 0 ? (
            <>
              <div className="grid gap-4 grid-cols-1 sm:grid-cols-2 lg:grid-cols-3">
                {paginatedProjects.map((project, i) => (
                  <div key={project.id} className="animate-slide-up" style={{ animationDelay: `${i * 80}ms`, opacity: 0 }}>
                    <ProjectCard
                      project={project}
                      onOpen={handleOpenProject}
                      onDashboard={handleOpenProjectDashboard}
                      onDuplicate={handleDuplicateProject}
                      onDelete={handleDeleteClick}
                      onExport={handleExportClick}
                      onShare={setProjectToShare}
                      onEditTags={setProjectToEditTags}
                      onRename={async (id, newName) => {
                        await updateProject(id, { name: newName });
                        toast.success(t("projects.renamed"));
                      }}
                    />
                  </div>
                ))}
              </div>

              {totalPages > 1 && (
                <div className="mt-5 flex flex-col items-center justify-between gap-3 border-t border-border/50 pt-4 text-sm text-muted-foreground sm:flex-row">
                  <span>
                    {t("projects.pagination.summary", {
                      start: visibleStart,
                      end: visibleEnd,
                      total: filteredProjects.length,
                    })}
                  </span>
                  <div className="flex items-center gap-2">
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => setCurrentPage((page) => Math.max(1, page - 1))}
                      disabled={safeCurrentPage === 1}
                    >
                      <ChevronLeft className="h-4 w-4" />
                      {t("projects.pagination.previous")}
                    </Button>
                    <span className="min-w-24 text-center text-xs font-medium text-foreground">
                      {t("projects.pagination.pageLabel", { page: safeCurrentPage, total: totalPages })}
                    </span>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => setCurrentPage((page) => Math.min(totalPages, page + 1))}
                      disabled={safeCurrentPage === totalPages}
                    >
                      {t("projects.pagination.next")}
                      <ChevronRight className="h-4 w-4" />
                    </Button>
                  </div>
                </div>
              )}
            </>
          ) : projects.length > 0 ? (
            <div className="flex flex-col items-center justify-center rounded-lg border border-dashed border-border/50 py-12 sm:py-16 px-4 animate-fade-in">
              {hasFilters && (
                <Badge variant="outline" className="mb-4">
                  {selectedTags.join(", ") || searchQuery}
                </Badge>
              )}
              <h3 className="text-lg font-semibold mb-2">{t("projects.filters.noResults.title")}</h3>
              <p className="text-muted-foreground mb-6 text-center max-w-sm text-sm sm:text-base">
                {t("projects.filters.noResults.description")}
              </p>
              <Button type="button" variant="outline" onClick={clearFilters}>
                <X className="h-4 w-4" />
                {t("projects.filters.clear")}
              </Button>
            </div>
          ) : (
            <AgentRuntimeOnboarding
              onCreateStack={handleCreateProject}
              onImportStack={() => fileInputRef.current?.click()}
            />
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

      <ProjectsSqliteExportDialog
        projects={projects}
        open={sqliteExportOpen}
        onOpenChange={setSqliteExportOpen}
      />

      <ProjectSharingDialog
        open={Boolean(projectToShare)}
        baseUrl={orchUrl}
        project={shareProject}
        onOpenChange={(open) => {
          if (!open) setProjectToShare(null);
        }}
      />

      <ProjectTagsDialog
        open={Boolean(tagProject)}
        projectName={tagProject?.name}
        tags={tagProject?.tags ?? []}
        onOpenChange={(open) => {
          if (!open) setProjectToEditTags(null);
        }}
        onSave={async (tags) => {
          if (!tagProject) return;
          await updateProject(tagProject.id, { tags });
          toast.success(t("projects.tags.updated"));
        }}
      />
    </main>
  );
}
