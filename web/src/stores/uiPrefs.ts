import { create } from "zustand";
import { persist } from "zustand/middleware";

/// Page-size choices offered by the feed selector. 25 (not 24) so the
/// widescreen 5-column grid fills whole rows instead of leaving a stub.
export const PAGE_SIZE_OPTIONS = [25, 50, 100, 200] as const;
export const DEFAULT_PAGE_SIZE = 25;

/// Page-size choices for the review queue. Smaller than the feed's because the
/// queue is a vertical list of expandable cards, not a grid: 20 keeps the
/// default page scannable while still allowing larger batches for bulk linking.
export const REVIEW_PAGE_SIZE_OPTIONS = [20, 50, 100, 200] as const;
export const DEFAULT_REVIEW_PAGE_SIZE = 20;

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
  /// Results per page for the review queue. Separate from the feed's so the
  /// two views (grid vs. list) can be sized independently.
  reviewPageSize: number;
  /// Feed results layout: `card` grid (default) or compact `list` rows.
  view: "card" | "list";
  /// Whether source cards show their config block (cron, feed_url, …).
  /// Defaults **off**: twenty-odd cards with a five-row config block each run
  /// several screens deep, and the config is reference material you look up
  /// occasionally rather than scan. Persisted so the choice survives a reload.
  sourceCardDetails: boolean;
  toggleWideMode: () => void;
  setPageSize: (size: number) => void;
  setReviewPageSize: (size: number) => void;
  setView: (view: "card" | "list") => void;
  toggleSourceCardDetails: () => void;
}

export const useUiPrefs = create<UiPrefsState>()(
  persist(
    (set) => ({
      wideMode: false,
      pageSize: DEFAULT_PAGE_SIZE,
      reviewPageSize: DEFAULT_REVIEW_PAGE_SIZE,
      view: "card",
      sourceCardDetails: false,
      toggleWideMode: () => set((state) => ({ wideMode: !state.wideMode })),
      setPageSize: (size) => set({ pageSize: size }),
      setReviewPageSize: (size) => set({ reviewPageSize: size }),
      setView: (view) => set({ view }),
      toggleSourceCardDetails: () =>
        set((state) => ({ sourceCardDetails: !state.sourceCardDetails })),
    }),
    {
      name: "tsundoku.ui-prefs.v1",
    },
  ),
);
