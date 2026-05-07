import type { Project } from "@/types/project";

export function normalizeProjectTags(tags: readonly string[] | undefined): string[] {
  const seen = new Set<string>();
  const normalized: string[] = [];

  for (const tag of tags ?? []) {
    const trimmed = tag.trim();
    if (!trimmed) continue;

    const key = trimmed.toLocaleLowerCase();
    if (seen.has(key)) continue;

    seen.add(key);
    normalized.push(trimmed);
  }

  return normalized;
}

export function collectProjectTags(projects: readonly Project[]): string[] {
  const byKey = new Map<string, string>();

  for (const project of projects) {
    for (const tag of normalizeProjectTags(project.tags)) {
      const key = tag.toLocaleLowerCase();
      if (!byKey.has(key)) {
        byKey.set(key, tag);
      }
    }
  }

  return Array.from(byKey.values()).sort((left, right) => left.localeCompare(right));
}

export function filterProjectsBySearchAndTags(
  projects: readonly Project[],
  searchQuery: string,
  selectedTags: readonly string[],
): Project[] {
  const query = searchQuery.trim().toLocaleLowerCase();
  const selectedKeys = selectedTags.map((tag) => tag.toLocaleLowerCase());

  return projects.filter((project) => {
    const matchesSearch = !query
      || project.name.toLocaleLowerCase().includes(query)
      || (project.description ?? "").toLocaleLowerCase().includes(query);

    if (!matchesSearch) return false;

    const projectTagKeys = new Set(
      normalizeProjectTags(project.tags).map((tag) => tag.toLocaleLowerCase()),
    );
    return selectedKeys.every((tag) => projectTagKeys.has(tag));
  });
}
