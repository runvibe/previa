import { resolveApiBaseUrl } from "@/lib/api-base";
import { getAuthToken, type AuthUser } from "@/stores/useAuthStore";

export type AccessRole = AuthUser["role"];

export interface LoginResponse {
  tokenKind: "jwt";
  token: string;
  user: AuthUser;
}

export interface ManagedUser {
  id: string;
  username: string;
  role: AccessRole;
  active: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface ApiTokenRecord {
  id: string;
  name: string;
  tokenPrefix: string;
  role: AccessRole;
  active: boolean;
  expiresAt?: string | null;
  createdByUsername: string;
  lastUsedAt?: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface ApiTokenCreateResponse {
  token: string;
  record: ApiTokenRecord;
}

export async function login(username: string, password: string): Promise<LoginResponse> {
  const response = await fetch(`${resolveApiBaseUrl()}/api/v1/auth/login`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ username, password, clientKind: "app" }),
  });
  if (!response.ok) {
    throw new Error(await response.text());
  }
  return response.json();
}

export async function listUsers(): Promise<ManagedUser[]> {
  const response = await authFetch("/api/v1/users");
  return response.json();
}

export async function createUser(input: {
  username: string;
  password: string;
  role: AccessRole;
  active: boolean;
}): Promise<ManagedUser> {
  const response = await authFetch("/api/v1/users", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(input),
  });
  return response.json();
}

export async function updateUser(
  userId: string,
  input: Partial<{
    username: string;
    password: string;
    role: AccessRole;
    active: boolean;
  }>,
): Promise<ManagedUser> {
  const response = await authFetch(`/api/v1/users/${encodeURIComponent(userId)}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(input),
  });
  return response.json();
}

export async function deleteUser(userId: string): Promise<void> {
  await authFetch(`/api/v1/users/${encodeURIComponent(userId)}`, {
    method: "DELETE",
  });
}

export async function listApiTokens(): Promise<ApiTokenRecord[]> {
  const response = await authFetch("/api/v1/api-tokens");
  return response.json();
}

export async function createApiToken(input: {
  name: string;
  role: AccessRole;
}): Promise<ApiTokenCreateResponse> {
  const response = await authFetch("/api/v1/api-tokens", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(input),
  });
  return response.json();
}

export async function updateApiToken(tokenId: string, active: boolean): Promise<ApiTokenRecord> {
  const response = await authFetch(`/api/v1/api-tokens/${encodeURIComponent(tokenId)}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ active }),
  });
  return response.json();
}

export async function deleteApiToken(tokenId: string): Promise<void> {
  await authFetch(`/api/v1/api-tokens/${encodeURIComponent(tokenId)}`, {
    method: "DELETE",
  });
}

export async function fetchCurrentUser(): Promise<AuthUser> {
  const token = getAuthToken();
  const response = await fetch(`${resolveApiBaseUrl()}/api/v1/auth/me`, {
    headers: token ? { Authorization: `Bearer ${token}` } : undefined,
  });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }
  return response.json();
}

async function authFetch(path: string, init?: RequestInit): Promise<Response> {
  const token = getAuthToken();
  const headers = new Headers(init?.headers);
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
  }
  const response = await fetch(`${resolveApiBaseUrl()}${path}`, {
    ...init,
    headers,
  });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }
  return response;
}
