import { useState, useRef, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { BarChart3, FolderOpen, MoreVertical, Copy, Trash2, Calendar, Download, Pencil, Tag, Share2 } from "lucide-react";
import type { Project } from "@/types/project";
import { format } from "date-fns";
import { ptBR, enUS } from "date-fns/locale";

interface ProjectCardProps {
  project: Project;
  onOpen: (id: string) => void;
  onDashboard: (id: string) => void;
  onDuplicate: (id: string) => void;
  onDelete: (id: string) => void;
  onExport: (id: string) => void;
  onShare?: (id: string) => void;
  onRename?: (id: string, newName: string) => void;
  onEditTags?: (id: string) => void;
}

export function ProjectCard({ project, onOpen, onDashboard, onDuplicate, onDelete, onExport, onShare, onRename, onEditTags }: ProjectCardProps) {
  const { t, i18n } = useTranslation();
  const pipelinesCount = project.pipelines?.length || 0;
  const specsCount = project.specs?.length || 0;
  const hasSpec = specsCount > 0 || Boolean(project.spec);
  const dateLocale = i18n.language === "pt-BR" ? ptBR : enUS;

  const [editing, setEditing] = useState(false);
  const [editName, setEditName] = useState(project.name);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editing) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editing]);

  const commitRename = () => {
    const trimmed = editName.trim();
    if (trimmed && trimmed !== project.name) {
      onRename?.(project.id, trimmed);
    } else {
      setEditName(project.name);
    }
    setEditing(false);
  };

  return (
    <Card className="group relative hover:shadow-md hover:border-primary/30 hover-lift transition-all duration-300 h-[244px] flex flex-col">
      <CardHeader className="pb-3">
      <div className="flex items-start justify-between gap-2">
        <div className="flex-1 min-w-0">
        {editing ? (
          <Input
          ref={inputRef}
          value={editName}
          onChange={(e) => setEditName(e.target.value)}
          onBlur={commitRename}
          onKeyDown={(e) => {
            if (e.key === "Enter") commitRename();
            if (e.key === "Escape") { setEditName(project.name); setEditing(false); }
          }}
          className="h-7 text-lg font-semibold"
          />
        ) : (
          <CardTitle className="text-lg truncate">{project.name}</CardTitle>
        )}
        {project.description && (
          <CardDescription className="mt-1 line-clamp-2">
          {project.description}
          </CardDescription>
        )}
        {project.tags && project.tags.length > 0 && (
          <div className="mt-2 flex flex-wrap gap-1.5">
            {project.tags.slice(0, 3).map((tag) => (
              <Badge key={tag} variant="outline" className="max-w-[120px] truncate text-xs">
                {tag}
              </Badge>
            ))}
            {project.tags.length > 3 && (
              <Badge variant="secondary" className="text-xs">
                +{project.tags.length - 3}
              </Badge>
            )}
          </div>
        )}
        </div>
        
        <DropdownMenu>
        <DropdownMenuTrigger asChild>
            <Button 
            variant="ghost" 
            size="icon" 
            className="h-8 w-8 opacity-0 group-hover:opacity-100 transition-opacity"
            aria-label={`${project.name} actions`}
            >
            <MoreVertical className="h-4 w-4" />
            </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem className="gap-2.5" onClick={() => onOpen(project.id)}>
          <FolderOpen className="h-4 w-4" />
          {t("common.open")}
          </DropdownMenuItem>
          <DropdownMenuItem className="gap-2.5" onClick={() => onDashboard(project.id)}>
          <BarChart3 className="h-4 w-4" />
          {t("dashboard.title")}
          </DropdownMenuItem>
          <DropdownMenuItem className="gap-2.5" onClick={() => { setEditName(project.name); setEditing(true); }}>
          <Pencil className="h-4 w-4" />
          {t("common.rename")}
          </DropdownMenuItem>
          <DropdownMenuItem className="gap-2.5" onClick={() => onEditTags?.(project.id)}>
          <Tag className="h-4 w-4" />
          {t("projects.tags.edit")}
          </DropdownMenuItem>
          <DropdownMenuItem className="gap-2.5" onClick={() => onDuplicate(project.id)}>
          <Copy className="h-4 w-4" />
          {t("common.duplicate")}
          </DropdownMenuItem>
          <DropdownMenuItem className="gap-2.5" onClick={() => onExport(project.id)}>
          <Download className="h-4 w-4" />
          {t("common.export")}
          </DropdownMenuItem>
          <DropdownMenuItem className="gap-2.5" onClick={() => onShare?.(project.id)}>
          <Share2 className="h-4 w-4" />
          Compartilhar
          </DropdownMenuItem>
          <DropdownMenuItem 
          onClick={() => onDelete(project.id)}
          className="gap-2.5 text-destructive focus:text-destructive"
          >
          <Trash2 className="h-4 w-4" />
          {t("common.delete")}
          </DropdownMenuItem>
        </DropdownMenuContent>
        </DropdownMenu>
      </div>
      </CardHeader>

      <CardContent className="flex-1 flex flex-col justify-between">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <Calendar className="h-3 w-3" />
        <span>
          {format(new Date(project.updatedAt), "dd MMM yyyy", { locale: dateLocale })}
        </span>
        </div>

        <div className="flex gap-1.5">
        {hasSpec && (
          <Badge variant="outline" className="text-xs bg-primary/5 border-primary/20">
          {specsCount > 1 ? `${specsCount} Specs` : "Spec"}
          </Badge>
        )}
        <Badge variant="secondary" className="text-xs bg-secondary/60 ">
          {pipelinesCount} pipeline{pipelinesCount !== 1 ? "s" : ""}
        </Badge>
        </div>
      </div>

      <div className="border-border/30 pt-4 mt-4">
        <Button 
        variant="default" 
        size="sm"
        className="w-full"
        onClick={() => onOpen(project.id)}
        >
        <FolderOpen className="h-4 w-4" />
        {t("projects.open")}
        </Button>
      </div>
      </CardContent>
    </Card>
  );
}
