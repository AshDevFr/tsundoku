import { create } from "zustand";
import { persist } from "zustand/middleware";

// Active filters live in the URL (so they're shareable). This store only
// persists the set of saved presets the user has named.
export interface FilterPreset {
  id: string;
  name: string;
  search: FilterSearch;
}

export interface FilterSearch {
  kind?: string;
  status?: string;
  owned?: boolean;
  /// Has-releases filter: `true` keeps only series with ≥1 linked
  /// release, `false` keeps only orphans (zero releases, typically the
  /// residue of a manual re-link). Absent means "no constraint".
  hasReleases?: boolean;
  /// Single genre name (case-insensitive against the backend).
  genre?: string;
  /// Single tag name (case-insensitive against the backend).
  tag?: string;
  sort?: string;
  order?: string;
  page?: number;
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
