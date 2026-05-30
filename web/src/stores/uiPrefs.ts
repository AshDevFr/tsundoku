import { create } from "zustand";
import { persist } from "zustand/middleware";

/// Page-size choices offered by the feed selector. 25 (not 24) so the
/// widescreen 5-column grid fills whole rows instead of leaving a stub.
export const PAGE_SIZE_OPTIONS = [25, 50, 100, 200] as const;
export const DEFAULT_PAGE_SIZE = 25;

// Display preferences tied to the user's device/taste rather than to a
// particular query. Filters live in the URL (so they're shareable); these
// don't — you'd never want to force wide mode onto someone on a small screen,
// and resetting the page size on every fresh visit is just annoying. Persisted
// to localStorage so both survive a reload.
interface UiPrefsState {
  /// Stretch the feed layout to the full viewport width (fixed-width sidebar +
  /// fluid card grid) instead of the centered max-width container.
  wideMode: boolean;
  /// Results per page for the feed. Persisted so it doesn't reset to the
  /// default each time the feed is opened without a remembered selection.
  pageSize: number;
  /// Feed results layout: `card` grid (default) or compact `list` rows.
  view: "card" | "list";
  toggleWideMode: () => void;
  setPageSize: (size: number) => void;
  setView: (view: "card" | "list") => void;
}

export const useUiPrefs = create<UiPrefsState>()(
  persist(
    (set) => ({
      wideMode: false,
      pageSize: DEFAULT_PAGE_SIZE,
      view: "card",
      toggleWideMode: () => set((state) => ({ wideMode: !state.wideMode })),
      setPageSize: (size) => set({ pageSize: size }),
      setView: (view) => set({ view }),
    }),
    {
      name: "tsundoku.ui-prefs.v1",
    },
  ),
);
