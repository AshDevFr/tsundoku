import "@testing-library/jest-dom/vitest";
import { afterAll, afterEach, beforeAll, vi } from "vitest";
import { server } from "@/mocks/server";
import { useUiPrefs } from "@/stores/uiPrefs";

// Persisted Zustand stores are module-level singletons, so a preference set in
// one test (e.g. flipping to list view) would otherwise bleed into the next.
// Snapshot the defaults once and restore them — plus clear localStorage — after
// each test.
const uiPrefsDefaults = useUiPrefs.getState();
afterEach(() => {
  useUiPrefs.setState(uiPrefsDefaults, true);
  localStorage.clear();
});

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

// Class form (not vi.fn) so callers using `new ResizeObserver(...)` work.
// Mantine's FloatingIndicator inside SegmentedControl exercises this path.
class MockResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
window.ResizeObserver = MockResizeObserver as unknown as typeof ResizeObserver;

window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;

// Minimal EventSource stand-in for jsdom. Tracks instances so tests can
// dispatch synthetic frames; auto-reconnect / readyState transitions
// are not modelled (tests that need them should drive .listeners
// directly).
class MockEventSource extends EventTarget {
  static instances: MockEventSource[] = [];
  url: string;
  readyState = 1;
  onmessage: ((evt: MessageEvent) => void) | null = null;
  onerror: ((evt: Event) => void) | null = null;
  constructor(url: string) {
    super();
    this.url = url;
    MockEventSource.instances.push(this);
  }
  close() {
    this.readyState = 2;
  }
  /// Test helper: dispatch a `message` event carrying the given JSON
  /// payload, matching what the real backend would push.
  emit(data: unknown) {
    const evt = new MessageEvent("message", { data: JSON.stringify(data) });
    this.dispatchEvent(evt);
    this.onmessage?.(evt);
  }
}
(globalThis as unknown as { EventSource: typeof MockEventSource }).EventSource =
  MockEventSource;
// Re-exported for tests that want to assert / drive instances.
(
  globalThis as unknown as { __mockEventSources: typeof MockEventSource }
).__mockEventSources = MockEventSource;

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

// jsdom doesn't compute layout, so Mantine's focusable() check (which relies
// on element dimensions) can't find a focusable child inside a Popover whose
// dropdown is still off-screen mid-transition. The trap settles correctly in
// a real browser; suppress the dev-only warning here.
const originalConsoleWarn = console.warn;
console.warn = (...args: unknown[]) => {
  const first = args[0];
  if (
    typeof first === "string" &&
    first.includes("[@mantine/hooks/use-focus-trap]")
  ) {
    return;
  }
  originalConsoleWarn(...args);
};

// MSW: intercept API calls for the whole test run.
beforeAll(() => server.listen({ onUnhandledRequest: "bypass" }));
afterEach(() => server.resetHandlers());
afterAll(() => server.close());
