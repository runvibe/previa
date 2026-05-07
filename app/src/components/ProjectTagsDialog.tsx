import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Tag, X } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { normalizeProjectTags } from "@/lib/project-tags";

interface ProjectTagsDialogProps {
  open: boolean;
  projectName?: string;
  tags: string[];
  onOpenChange: (open: boolean) => void;
  onSave: (tags: string[]) => void;
}

export function ProjectTagsDialog({
  open,
  projectName,
  tags,
  onOpenChange,
  onSave,
}: ProjectTagsDialogProps) {
  const { t } = useTranslation();
  const [draftTags, setDraftTags] = useState<string[]>(tags);
  const [tagName, setTagName] = useState("");

  useEffect(() => {
    if (open) {
      setDraftTags(normalizeProjectTags(tags));
      setTagName("");
    }
  }, [open, tags]);

  const normalizedDraftTags = useMemo(() => normalizeProjectTags(draftTags), [draftTags]);

  const addTag = () => {
    const next = normalizeProjectTags([...normalizedDraftTags, tagName]);
    setDraftTags(next);
    setTagName("");
  };

  const removeTag = (tagToRemove: string) => {
    const key = tagToRemove.toLocaleLowerCase();
    setDraftTags(normalizedDraftTags.filter((tag) => tag.toLocaleLowerCase() !== key));
  };

  const saveTags = () => {
    onSave(normalizedDraftTags);
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("projects.tags.title")}</DialogTitle>
          {projectName && <DialogDescription>{projectName}</DialogDescription>}
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="project-tag-name">{t("projects.tags.inputLabel")}</Label>
            <div className="flex gap-2">
              <Input
                id="project-tag-name"
                value={tagName}
                onChange={(event) => setTagName(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    addTag();
                  }
                }}
              />
              <Button type="button" variant="outline" onClick={addTag}>
                <Tag className="h-4 w-4" />
                {t("projects.tags.add")}
              </Button>
            </div>
          </div>

          <div className="flex min-h-9 flex-wrap gap-2">
            {normalizedDraftTags.map((tag) => (
              <Badge key={tag} variant="secondary" className="gap-1.5">
                {tag}
                <button
                  type="button"
                  className="rounded-sm text-muted-foreground hover:text-foreground"
                  aria-label={`Remove ${tag}`}
                  onClick={() => removeTag(tag)}
                >
                  <X className="h-3 w-3" />
                </button>
              </Badge>
            ))}
          </div>
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button type="button" onClick={saveTags}>
            {t("projects.tags.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
