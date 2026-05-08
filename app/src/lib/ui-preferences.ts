/**
 * Browser-local UI preferences only.
 *
 * Project, pipeline and execution data do not live here. Remote mode uses the
 * backend as the source of truth, and offline project persistence lives in
 * `project-db.ts`.
 */

const KEYS = {
  THEME: "api-pipeline-studio:theme",
  PALETTE: "api-pipeline-studio:palette",
  REDUCE_GLASS: "api-pipeline-studio:reduce-glass",
  TEST_HISTORY_COLLAPSED: "api-pipeline-studio:test-history-collapsed",
  TEST_MODE_SIDEBAR_COLLAPSED: "api-pipeline-studio:test-mode-sidebar-collapsed",
};

export type TestHistoryKind = "integration" | "loadtest";

export function getTheme(): "dark" | "light" {
  return (localStorage.getItem(KEYS.THEME) as "dark" | "light") || "dark";
}

export function setTheme(theme: "dark" | "light"): void {
  localStorage.setItem(KEYS.THEME, theme);
}

export function getPalette(): string {
  return localStorage.getItem(KEYS.PALETTE) || "default";
}

export function setPalette(palette: string): void {
  localStorage.setItem(KEYS.PALETTE, palette);
}

export function getGlassLevel(): number {
  const value = localStorage.getItem(KEYS.REDUCE_GLASS);
  if (value === "true") return 4;
  const parsed = Number(value);
  return Number.isNaN(parsed) ? 5 : parsed;
}

export function setGlassLevel(level: number): void {
  localStorage.setItem(KEYS.REDUCE_GLASS, String(level));
}

export function getTestHistoryCollapsed(kind: TestHistoryKind): boolean {
  return localStorage.getItem(`${KEYS.TEST_HISTORY_COLLAPSED}:${kind}`) === "true";
}

export function setTestHistoryCollapsed(kind: TestHistoryKind, collapsed: boolean): void {
  localStorage.setItem(`${KEYS.TEST_HISTORY_COLLAPSED}:${kind}`, String(collapsed));
}

export function getTestModeSidebarCollapsed(): boolean {
  return localStorage.getItem(KEYS.TEST_MODE_SIDEBAR_COLLAPSED) === "true";
}

export function setTestModeSidebarCollapsed(collapsed: boolean): void {
  localStorage.setItem(KEYS.TEST_MODE_SIDEBAR_COLLAPSED, String(collapsed));
}
