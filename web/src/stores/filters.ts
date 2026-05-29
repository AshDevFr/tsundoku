import { create } from "zustand";
import { persist } from "zustand/middleware";

// Active filters live in the URL (so they're shareable). This store only
// persists the set of saved presets the user has named.
export interface FilterPreset {
  id: string;
  name: string;
  search: FilterSearch;
}

export type TagFilterMode = "any" | "all";

/// Page-size choices offered by the feed selector. 25 (not 24) so the
/// widescreen 5-column grid fills whole rows instead of leaving a stub.
export const PAGE_SIZE_OPTIONS = [25, 50, 100, 200] as const;
export const DEFAULT_PAGE_SIZE = 25;

export interface FilterSearch {
  kind?: string;
  status?: string;
  owned?: boolean;
  /// Has-releases filter: `true` keeps only series with ≥1 linked
  /// release, `false` keeps only orphans (zero releases, typically the
  /// residue of a manual re-link). Absent means "no constraint".
  hasReleases?: boolean;
  /// Selected genre names. Case-insensitive on the backend.
  genres?: string[];
  /// `any` (default) keeps series matching at least one selected genre;
  /// `all` requires every selected genre.
  genresMode?: TagFilterMode;
  /// Selected tag names. Case-insensitive on the backend.
  tags?: string[];
  /// `any` (default) or `all`. See [[genresMode]].
  tagsMode?: TagFilterMode;
  sort?: string;
  order?: string;
  page?: number;
  /// Results per page. One of the values offered by the feed's page-size
  /// selector; absent means the default. Backend caps at 200.
  pageSize?: number;
  /// Free-text search query. Whitespace-only treated as absent.
  q?: string;
  /// View mode for the results grid: `card` (default) or `list`.
  view?: "card" | "list";
}

interface PresetState {
  presets: FilterPreset[];
  savePreset: (name: string, search: FilterSearch) => FilterPreset;
  deletePreset: (id: string) => void;
}

export const useFilterPresets = create<PresetState>()(
  persist(
    (set) => ({
      presets: [],
      savePreset: (name, search) => {
        const id = crypto.randomUUID();
        const preset: FilterPreset = { id, name, search };
        set((state) => ({ presets: [...state.presets, preset] }));
        return preset;
      },
      deletePreset: (id) =>
        set((state) => ({ presets: state.presets.filter((p) => p.id !== id) })),
    }),
    {
      name: "tsundoku.filter-presets.v1",
    },
  ),
);
