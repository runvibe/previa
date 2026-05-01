import { beforeEach, describe, expect, it, vi } from "vitest";

import { getApiUrlFromBase, normalizeApiBaseUrl, resolveApiBaseUrl } from "@/lib/api-base";

describe("api base resolution", () => {
  beforeEach(() => {
    vi.unstubAllEnvs();
    window.history.replaceState({}, "", "/");
  });

  it("uses window.location.origin when VITE_PREVIA_API_BASE_URL is not defined", () => {
    expect(resolveApiBaseUrl()).toBe(window.location.origin);
  });

  it("uses VITE_PREVIA_API_BASE_URL when it is defined", () => {
    vi.stubEnv("VITE_PREVIA_API_BASE_URL", "http://127.0.0.1:5588/");

    expect(resolveApiBaseUrl()).toBe("http://127.0.0.1:5588");
  });

  it("does not duplicate the api prefix when the configured base already includes it", () => {
    vi.stubEnv("VITE_PREVIA_API_BASE_URL", "http://127.0.0.1:5588/api/v1/");

    expect(getApiUrlFromBase(resolveApiBaseUrl())).toBe("http://127.0.0.1:5588/api/v1");
  });

  it("ignores legacy context query parameters while resolving the api base", () => {
    window.history.replaceState({}, "", "/?add_context=http%3A%2F%2F127.0.0.1%3A5588&context=old");

    expect(resolveApiBaseUrl()).toBe(window.location.origin);
  });

  it("normalizes trailing slashes and api suffixes", () => {
    expect(normalizeApiBaseUrl("http://localhost:5588///")).toBe("http://localhost:5588");
    expect(normalizeApiBaseUrl("http://localhost:5588/api/v1/")).toBe("http://localhost:5588");
  });
});
