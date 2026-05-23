import { useTranslation } from "react-i18next";
import { Checkbox } from "@/components/ui/checkbox";
import { CheckCircle2, XCircle, Pencil, X, Copy, GripVertical, Clock3, Share2 } from "lucide-react";
import { DotsLoader } from "@/components/DotsLoader";
import { SidebarItemActionBar } from "@/components/SidebarItemActionBar";
import type { Pipeline } from "@/types/pipeline";

interface PipelineListItemProps {
  pipeline: Pipeline;
  index: number;
  isSelected: boolean;
  status?: "success" | "error" | "running" | "queued";
  onSelect: (index: number, event?: React.MouseEvent) => void;
  onEdit?: (index: number) => void;
  onDuplicate?: (index: number) => void;
  onShare?: (index: number) => void;
  onDelete: (index: number) => void;
  onDragStart?: (index: number) => void;
  onDragOver?: (index: number) => void;
  onDrop?: (index: number) => void;
  isDragTarget?: boolean;
  isChecked?: boolean;
  onToggleCheck?: (index: number) => void;
}

export function PipelineListItem({ pipeline, index, isSelected, status, onSelect, onEdit, onDuplicate, onShare, onDelete, onDragStart, onDragOver, onDrop, isDragTarget, isChecked, onToggleCheck }: PipelineListItemProps) {
  const { t } = useTranslation();

  return (
    <div
      className={`group relative flex px-3 py-2.5 text-sm transition-all duration-150 cursor-pointer border-border/20
        ${isSelected 
          ? "bg-primary/10 shadow-ring-primary" 
          : isChecked && onToggleCheck
            ? "bg-accent/30"
            : "hover:bg-accent/40"
        }
        ${status === "running" ? "border-l-2 border-l-primary" :
          status === "queued" ? "border-l-2 border-l-amber-500" :
          status === "success" ? "border-l-2 border-l-emerald-500" :
          status === "error" ? "border-l-2 border-l-red-500" : ""
        }
        ${isDragTarget ? "border-t-2 border-t-primary" : ""}`}
      onClick={(e) => onSelect(index, e)}
      draggable={!!onDragStart}
      onDragStart={(e) => {
        e.dataTransfer.effectAllowed = "move";
        onDragStart?.(index);
      }}
      onDragOver={(e) => {
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        onDragOver?.(index);
      }}
      onDrop={(e) => {
        e.preventDefault();
        onDrop?.(index);
      }}
    >
      {/* Batch checkbox */}
      {onToggleCheck && (
        <div className="flex items-center mr-1.5" onClick={(e) => e.stopPropagation()}>
          <Checkbox
            checked={isChecked}
            onCheckedChange={() => onToggleCheck(index)}
            className="h-3.5 w-3.5 border-muted-foreground/50"
          />
        </div>
      )}

      {/* Drag handle */}
      {onDragStart && (
        <div className="flex items-center mr-1.5 opacity-0 group-hover:opacity-50 cursor-grab active:cursor-grabbing">
          <GripVertical className="h-3.5 w-3.5 text-muted-foreground" />
        </div>
      )}

      <SidebarItemActionBar
        label={`${pipeline.name} actions`}
        actions={[
          ...(onEdit ? [{ label: t("pipeline.editPipeline"), icon: <Pencil className="h-3 w-3" />, onClick: () => onEdit(index) }] : []),
          ...(onDuplicate ? [{ label: t("pipeline.duplicatePipeline"), icon: <Copy className="h-3 w-3" />, onClick: () => onDuplicate(index) }] : []),
          ...(onShare ? [{ label: "Compartilhar", icon: <Share2 className="h-3 w-3" />, onClick: () => onShare(index) }] : []),
          { label: t("common.delete"), icon: <X className="h-3 w-3" />, onClick: () => onDelete(index) },
        ]}
      />

      {/* Content */}
      <div className="min-w-0 flex-1">
        <div className="inline-block text-xs">
          {pipeline.name}
        </div>
        <div className="flex items-center gap-1.5">
          <p className="truncate text-xs text-muted-foreground">{pipeline.steps.length} steps</p>
          {pipeline.updatedAt && (
            <span className="text-[10px] text-muted-foreground/70">
              {new Date(pipeline.updatedAt).toLocaleString(undefined, { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" })}
            </span>
          )}
          {status === "running" && <DotsLoader className="text-primary" />}
          {status === "queued" && <Clock3 className="h-3 w-3 text-amber-500 shrink-0" />}
          {status === "success" && <CheckCircle2 className="h-3 w-3 text-success shrink-0" />}
          {status === "error" && <XCircle className="h-3 w-3 text-destructive shrink-0" />}
        </div>
      </div>
    </div>
  );
}
