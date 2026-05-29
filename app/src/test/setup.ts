import "@testing-library/jest-dom";

const originalConsole = {
  error: console.error.bind(console),
  info: console.info.bind(console),
  log: console.log.bind(console),
  warn: console.warn.bind(console),
};

const suppressedConsolePrefixes = [
  "[DEBUG]",
  "[SpecSyncNotifier]",
  "Failed to delete from backend:",
  "Reconnect load test error:",
  "Remote load test error:",
  "🌐 i18next is maintained",
];

const suppressedConsoleFragments = [
  "React Router Future Flag Warning",
  "i18next is maintained",
];

function shouldSuppressConsole(args: unknown[]): boolean {
  const message = args
    .map((arg) => (typeof arg === "string" ? arg : ""))
    .join(" ");

  return (
    suppressedConsolePrefixes.some((prefix) => message.startsWith(prefix)) ||
    suppressedConsoleFragments.some((fragment) => message.includes(fragment))
  );
}

console.log = (...args: unknown[]) => {
  if (!shouldSuppressConsole(args)) {
    originalConsole.log(...args);
  }
};

console.info = (...args: unknown[]) => {
  if (!shouldSuppressConsole(args)) {
    originalConsole.info(...args);
  }
};

console.warn = (...args: unknown[]) => {
  if (!shouldSuppressConsole(args)) {
    originalConsole.warn(...args);
  }
};

console.error = (...args: unknown[]) => {
  if (!shouldSuppressConsole(args)) {
    originalConsole.error(...args);
  }
};

Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => {},
  }),
});

Object.defineProperty(window, "ResizeObserver", {
  writable: true,
  value: class ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  },
});

const storage = new Map<string, string>();

Object.defineProperty(window, "localStorage", {
  writable: true,
  value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => {
      storage.set(key, value);
    },
    removeItem: (key: string) => {
      storage.delete(key);
    },
    clear: () => {
      storage.clear();
    },
    key: (index: number) => Array.from(storage.keys())[index] ?? null,
    get length() {
      return storage.size;
    },
  },
});
