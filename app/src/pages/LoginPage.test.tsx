import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

import LoginPage from "@/pages/LoginPage";
import { useAuthStore } from "@/stores/useAuthStore";

const fetchCurrentUserMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/auth-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/auth-client")>();
  return {
    ...actual,
    fetchCurrentUser: fetchCurrentUserMock,
  };
});

function renderLogin() {
  return render(
    <MemoryRouter initialEntries={["/login"]} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
      <Routes>
        <Route path="/login" element={<LoginPage />} />
        <Route path="/" element={<div>projects home</div>} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("LoginPage", () => {
  beforeEach(() => {
    localStorage.clear();
    useAuthStore.getState().clearSession();
    fetchCurrentUserMock.mockReset();
  });

  it("leaves the login page when the server allows anonymous access", async () => {
    fetchCurrentUserMock.mockResolvedValue({
      id: "anonymous",
      username: "anonymous",
      role: "anonymous",
      source: "anonymous",
    });

    renderLogin();

    await waitFor(() => {
      expect(screen.getByText("projects home")).toBeInTheDocument();
    });
  });
});
