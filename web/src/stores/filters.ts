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
  /// Metadata provenance filter: `manual` keeps only operator-authored
  /// series, `auto` keeps only provider-backed ones. Absent means "no
  /// constraint". Unrecognized values are ignored by the backend.
  metadataSource?: "manual" | "auto";
  sort?: string;
  order?: string;
  page?: number;
  /// Free-text search query. Whitespace-only treated as absent.
  q?: string;
  /// Codex presence filter: `any` (on Codex), `missing` (not on Codex),
  /// `complete`, `behind`, or `present`. Admin-only — the control is hidden
  /// for non-admins and the backend ignores it without a valid admin token.
  codexStatus?: string;
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
