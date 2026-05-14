import { render, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

import AccessManagementPage from "@/pages/AccessManagementPage";
import { TooltipProvider } from "@/components/ui/tooltip";

const useAppHeaderMock = vi.hoisted(() => vi.fn());
const listUsersMock = vi.hoisted(() => vi.fn());
const listApiTokensMock = vi.hoisted(() => vi.fn());

vi.mock("@/components/AppShell", () => ({
  useAppHeader: useAppHeaderMock,
}));

vi.mock("@/components/ProjectSettingsDialog", () => ({
  ProjectSettingsDialog: () => <button type="button">Settings</button>,
}));

vi.mock("@/stores/useAuthStore", () => ({
  useAuthStore: (selector: (state: unknown) => unknown) => selector({
    user: {
      id: "root",
      username: "root",
      role: "root",
      source: "env",
    },
  }),
}));

vi.mock("@/lib/auth-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/auth-client")>();
  return {
    ...actual,
    listUsers: listUsersMock,
    listApiTokens: listApiTokensMock,
    createUser: vi.fn(),
    updateUser: vi.fn(),
    deleteUser: vi.fn(),
    createApiToken: vi.fn(),
    updateApiToken: vi.fn(),
    deleteApiToken: vi.fn(),
  };
});

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    i18n: { language: "en" },
    t: (key: string) => key,
  }),
}));

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
  },
}));

function renderPage() {
  return render(
    <TooltipProvider>
      <MemoryRouter future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
        <AccessManagementPage />
      </MemoryRouter>
    </TooltipProvider>,
  );
}

describe("AccessManagementPage", () => {
  beforeEach(() => {
    useAppHeaderMock.mockReset();
    listUsersMock.mockResolvedValue([]);
    listApiTokensMock.mockResolvedValue([]);
  });

  it("keeps the project settings action in the app header", async () => {
    renderPage();

    await waitFor(() => {
      expect(useAppHeaderMock).toHaveBeenCalledWith(expect.objectContaining({
        headerActions: expect.anything(),
      }));
    });
  });
});
