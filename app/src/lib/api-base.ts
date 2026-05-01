const API_PREFIX = "/api/v1";

function configuredApiBaseUrl(): string | undefined {
  const configured = import.meta.env.VITE_PREVIA_API_BASE_URL;
  return typeof configured === "string" && configured.trim() ? configured : undefined;
}

export function normalizeApiBaseUrl(rawUrl: string): string {
  return rawUrl.trim().replace(/\/api\/v1\/?$/, "").replace(/\/+$/, "");
}

export function resolveApiBaseUrl(): string {
  return normalizeApiBaseUrl(configuredApiBaseUrl() ?? window.location.origin);
}

export function getApiUrlFromBase(baseUrl = resolveApiBaseUrl()): string {
  return `${normalizeApiBaseUrl(baseUrl)}${API_PREFIX}`;
}
