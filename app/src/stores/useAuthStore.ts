import { create } from "zustand";

const STORAGE_KEY = "previa.auth";

export interface AuthUser {
  id: string;
  username: string;
  role: "root" | "admin" | "editor" | "operator" | "viewer" | "anonymous";
  source: "env" | "database" | "anonymous" | "api_token";
}

interface StoredAuth {
  token: string | null;
  user: AuthUser | null;
}

interface AuthState extends StoredAuth {
  setSession: (token: string | null, user: AuthUser | null) => void;
  clearSession: () => void;
}

function readStoredAuth(): StoredAuth {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { token: null, user: null };
    const parsed = JSON.parse(raw) as StoredAuth;
    return {
      token: typeof parsed.token === "string" ? parsed.token : null,
      user: parsed.user ?? null,
    };
  } catch {
    return { token: null, user: null };
  }
}

function writeStoredAuth(auth: StoredAuth) {
  if (!auth.token) {
    localStorage.removeItem(STORAGE_KEY);
    return;
  }
  localStorage.setItem(STORAGE_KEY, JSON.stringify(auth));
}

const initial = readStoredAuth();

export const useAuthStore = create<AuthState>((set) => ({
  ...initial,
  setSession: (token, user) => {
    writeStoredAuth({ token, user });
    set({ token, user });
  },
  clearSession: () => {
    writeStoredAuth({ token: null, user: null });
    set({ token: null, user: null });
  },
}));

export function getAuthToken(): string | null {
  return useAuthStore.getState().token;
}

export function clearAuthSession() {
  useAuthStore.getState().clearSession();
}
