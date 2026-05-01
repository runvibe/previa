import { useEffect, useMemo, useState } from "react";
import { Plus, Pencil, Trash2, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SectionHeader } from "@/components/SectionHeader";
import type { ProjectEnvEntry, ProjectEnvGroup } from "@/types/project";
import type { ProjectEnvGroupUpsertRequest } from "@/lib/api-client";

interface ProjectEnvGroupsPanelProps {
  envGroups: ProjectEnvGroup[];
  onCreate: (data: ProjectEnvGroupUpsertRequest) => Promise<ProjectEnvGroup | null>;
  onUpdate: (id: string, data: ProjectEnvGroupUpsertRequest) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
}

const emptyEntry = (): ProjectEnvEntry => ({ name: "", url: "", description: null });

function slugify(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

export function ProjectEnvGroupsPanel({ envGroups, onCreate, onUpdate, onDelete }: ProjectEnvGroupsPanelProps) {
  const [editingGroup, setEditingGroup] = useState<ProjectEnvGroup | null>(null);
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [entries, setEntries] = useState<ProjectEnvEntry[]>([emptyEntry()]);
  const [saving, setSaving] = useState(false);

  const usedSlugs = useMemo(
    () => new Set(envGroups.filter((group) => group.id !== editingGroup?.id).map((group) => group.slug)),
    [envGroups, editingGroup?.id],
  );

  useEffect(() => {
    if (!open) return;
    if (editingGroup) {
      setName(editingGroup.name);
      setSlug(editingGroup.slug);
      setEntries(editingGroup.entries.length > 0 ? editingGroup.entries : [emptyEntry()]);
      return;
    }
    setName("");
    setSlug("");
    setEntries([emptyEntry()]);
  }, [open, editingGroup]);

  const openCreate = () => {
    setEditingGroup(null);
    setOpen(true);
  };

  const openEdit = (group: ProjectEnvGroup) => {
    setEditingGroup(group);
    setOpen(true);
  };

  const updateEntry = (index: number, patch: Partial<ProjectEnvEntry>) => {
    setEntries((current) => current.map((entry, i) => i === index ? { ...entry, ...patch } : entry));
  };

  const removeEntry = (index: number) => {
    setEntries((current) => current.length === 1 ? [emptyEntry()] : current.filter((_, i) => i !== index));
  };

  const normalizedSlug = slugify(slug || name);
  const normalizedEntries = entries
    .map((entry) => ({
      name: slugify(entry.name),
      url: entry.url.trim(),
      description: entry.description?.trim() || null,
    }))
    .filter((entry) => entry.name && entry.url);
  const canSave = !!name.trim() && !!normalizedSlug && normalizedSlug !== "current" && !usedSlugs.has(normalizedSlug) && normalizedEntries.length > 0;

  const save = async () => {
    if (!canSave || saving) return;
    setSaving(true);
    try {
      const payload = {
        name: name.trim(),
        slug: normalizedSlug,
        entries: normalizedEntries,
      };
      if (editingGroup) {
        await onUpdate(editingGroup.id, payload);
      } else {
        await onCreate(payload);
      }
      setOpen(false);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="border-border/50 px-4 py-3">
      <SectionHeader title="Env Groups">
        <Button variant="ghost" size="icon" className="h-7 w-7" onClick={openCreate} title="Add env group">
          <Plus className="h-3.5 w-3.5" />
        </Button>
      </SectionHeader>

      <div className="mt-1 max-h-36 space-y-1 overflow-y-auto pr-1" aria-label="Env groups list">
        {envGroups.length === 0 ? (
          <p className="px-1.5 py-1 text-xs text-muted-foreground">Nenhum env group</p>
        ) : envGroups.map((group) => (
          <div key={group.id} className="group flex items-center gap-1.5 rounded-md px-1.5 py-1 text-xs hover:bg-accent/50">
            <div className="min-w-0 flex-1">
              <div className="truncate font-medium">{group.name}</div>
              <div className="truncate font-mono text-[10px] text-muted-foreground">{group.slug}</div>
            </div>
            <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">{group.entries.length}</span>
            <Button variant="ghost" size="icon" className="h-6 w-6 opacity-70 hover:opacity-100" onClick={() => openEdit(group)} title="Edit env group">
              <Pencil className="h-3 w-3" />
            </Button>
            <Button variant="ghost" size="icon" className="h-6 w-6 opacity-70 hover:opacity-100" onClick={() => onDelete(group.id)} title="Delete env group">
              <Trash2 className="h-3 w-3" />
            </Button>
          </div>
        ))}
      </div>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>{editingGroup ? "Edit Env Group" : "New Env Group"}</DialogTitle>
          </DialogHeader>

          <div className="grid gap-4">
            <div className="grid gap-2 sm:grid-cols-2">
              <div className="space-y-1.5">
                <Label>Name</Label>
                <Input value={name} onChange={(event) => setName(event.target.value)} placeholder="Homolog" />
              </div>
              <div className="space-y-1.5">
                <Label>Slug</Label>
                <Input value={slug} onChange={(event) => setSlug(event.target.value)} placeholder="hml" />
              </div>
            </div>

            <div className="space-y-2">
              <Label>Entries</Label>
              <div className="space-y-2">
                {entries.map((entry, index) => (
                  <div key={index} className="grid gap-2 sm:grid-cols-[120px_minmax(0,1fr)_32px]">
                    <Input value={entry.name} onChange={(event) => updateEntry(index, { name: event.target.value })} placeholder="api" />
                    <Input value={entry.url} onChange={(event) => updateEntry(index, { url: event.target.value })} placeholder="https://api.example.com" />
                    <Button type="button" variant="ghost" size="icon" className="h-9 w-9" onClick={() => removeEntry(index)} title="Remove entry">
                      <X className="h-3.5 w-3.5" />
                    </Button>
                  </div>
                ))}
              </div>
              <Button type="button" variant="outline" className="w-full" onClick={() => setEntries((current) => [...current, emptyEntry()])}>
                <Plus className="h-3.5 w-3.5" /> Entry
              </Button>
            </div>
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setOpen(false)}>Cancel</Button>
            <Button type="button" onClick={save} disabled={!canSave || saving}>Save</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
