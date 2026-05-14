import { beforeEach, describe, expect, it, vi } from "vitest";

import { deleteApiToken, deleteUser, updateApiToken, updateUser } from "@/lib/auth-client";
import { useAuthStore } from "@/stores/useAuthStore";

describe("auth-client management actions", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    localStorage.clear();
    useAuthStore.getState().setSession("jwt-test", {
      id: "root",
      username: "root",
      role: "root",
      source: "env",
    });
  });

  it("updates users with bearer auth", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({
        id: "usr_1",
        username: "ana",
        role: "editor",
        active: false,
        createdAt: "2026-05-13T00:00:00Z",
        updatedAt: "2026-05-13T00:00:00Z",
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await updateUser("usr_1", { active: false });

    expect(fetchMock).toHaveBeenCalledWith(
      expect.stringContaining("/api/v1/users/usr_1"),
      expect.objectContaining({
        method: "PATCH",
        body: JSON.stringify({ active: false }),
      }),
    );
    const init = fetchMock.mock.calls[0][1] as RequestInit;
    expect(new Headers(init.headers).get("Authorization")).toBe("Bearer jwt-test");
  });

  it("deletes users and revokes api tokens", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true, status: 204 });
    vi.stubGlobal("fetch", fetchMock);

    await deleteUser("usr_1");
    await deleteApiToken("tok_1");

    expect(fetchMock).toHaveBeenCalledWith(
      expect.stringContaining("/api/v1/users/usr_1"),
      expect.objectContaining({ method: "DELETE" }),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      expect.stringContaining("/api/v1/api-tokens/tok_1"),
      expect.objectContaining({ method: "DELETE" }),
    );
  });

  it("updates api token active state", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({
        id: "tok_1",
        name: "ci",
        tokenPrefix: "pvk_test",
        role: "operator",
        active: false,
        createdByUsername: "root",
        createdAt: "2026-05-13T00:00:00Z",
        updatedAt: "2026-05-13T00:00:00Z",
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await updateApiToken("tok_1", false);

    expect(fetchMock).toHaveBeenCalledWith(
      expect.stringContaining("/api/v1/api-tokens/tok_1"),
      expect.objectContaining({
        method: "PATCH",
        body: JSON.stringify({ active: false }),
      }),
    );
  });
});
