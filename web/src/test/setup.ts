import "@testing-library/jest-dom/vitest";
import { afterAll, afterEach, beforeAll, vi } from "vitest";
import { server } from "@/mocks/server";

// jsdom doesn't implement these browser APIs, but Mantine (and many UI libs)
// call them on mount. Provide minimal stubs so component tests can render.
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: vi.fn().mockImplementation((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })),
});

window.ResizeObserver = vi.fn().mockImplementation(() => ({
  observe: vi.fn(),
  unobserve: vi.fn(),
  disconnect: vi.fn(),
})) as unknown as typeof ResizeObserver;

window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;

// Mantine's internal state updates (Tooltip mount, PasswordInput focus, etc.)
// settle after fireEvent returns. The tests still await the visible outcome
// via findBy*/waitFor, so the act() warnings are noise rather than real bugs.
// Surface anything else through the original console.error.
const originalConsoleError = console.error;
console.error = (...args: unknown[]) => {
  const first = args[0];
  if (typeof first === "string" && first.includes("not wrapped in act(")) {
    return;
  }
  originalConsoleError(...args);
};

// MSW: intercept API calls for the whole test run.
beforeAll(() => server.listen({ onUnhandledRequest: "bypass" }));
afterEach(() => server.resetHandlers());
afterAll(() => server.close());
