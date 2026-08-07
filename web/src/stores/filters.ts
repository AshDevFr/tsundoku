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
  /// Selected kind values (e.g. `manga`, `manhwa`), OR-combined: a series
  /// is kept if its kind matches any selected value. Sent as a CSV the
  /// backend re-splits and matches via `IN`.
  kind?: string[];
  /// Selected status values (e.g. `ongoing`, `completed`), OR-combined.
  /// See [[kind]].
  status?: string[];
  owned?: boolean;
  /// Wishlist filter: `true` keeps only wishlisted series, `false` keeps only
  /// non-wishlisted, absent means "no constraint". Admin-only — the control is
  /// hidden for non-admins and the backend ignores it without a valid admin
  /// token.
  wishlisted?: boolean;
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
  /// When `true`, the free-text [[q]] search also matches series descriptions,
  /// not just titles. Absent/`false` means titles only. A search *mode* rather
  /// than a filter, so it's deliberately excluded from [[countActiveFilters]];
  /// it is still captured in saved presets and reset by "clear all".
  searchDescriptions?: boolean;
  /// Codex presence filter, OR-combined: `any` (on Codex), `missing` (not on
  /// Codex), `complete`, `behind`, `present`, or `ignored` (completion tracking
  /// off). Selecting several keeps series matching any of them (e.g. `missing`
  /// + `behind`). Admin-only — the control is hidden for non-admins and the
  /// backend ignores it without a valid admin token.
  codexStatus?: string[];
  /// Selected discovery-source names (the release `source_name`), OR-combined:
  /// a series is kept if it has ≥1 linked release from any selected source.
  /// Admin-only — the control is hidden for non-admins and the backend ignores
  /// it without a valid admin token.
  sources?: string[];
}

/// Count the active filter constraints in a search (sort/order/page are
/// presentation, not filters, so they don't count). Single source of truth for
/// both the FilterPanel's "any active?" check and the mobile Filters button's
/// count badge. When adding a field here, see the filter-field audit checklist.
export function countActiveFilters(search: FilterSearch): number {
  let n = 0;
  if ((search.kind?.length ?? 0) > 0) n++;
  if ((search.status?.length ?? 0) > 0) n++;
  if ((search.genres?.length ?? 0) > 0) n++;
  if ((search.tags?.length ?? 0) > 0) n++;
  if (search.q) n++;
  if (typeof search.owned === "boolean") n++;
  if (typeof search.wishlisted === "boolean") n++;
  if (typeof search.hasReleases === "boolean") n++;
  if (search.metadataSource) n++;
  if ((search.codexStatus?.length ?? 0) > 0) n++;
  if ((search.sources?.length ?? 0) > 0) n++;
  return n;
}

/// Presentation order for the presets menu. Case-insensitive so "default
/// search" and "Default Search" don't split, and numeric-aware so "Isekai 2"
/// sorts before "Isekai 10".
///
/// Returns a new array and leaves the store in insertion order: order here is
/// a display concern, so nothing persisted has to change. Callers own
/// memoization. Do not call this inside a zustand selector, which would hand
/// React a fresh array identity on every snapshot read and trip the
/// "getSnapshot should be cached" check in useSyncExternalStore.
export function sortPresets(presets: FilterPreset[]): FilterPreset[] {
  return [...presets].sort((a, b) =>
    a.name.localeCompare(b.name, undefined, {
      sensitivity: "base",
      numeric: true,
    }),
  );
}

interface PresetState {
  presets: FilterPreset[];
  /// Id of the preset most recently applied, so the save modal can offer to
  /// write back to it. Session-scoped and deliberately not persisted: after a
  /// reload the filters on screen come from the URL, and a stale id would
  /// prefill a name that has nothing to do with them.
  activePresetId?: string;
  setActivePreset: (id: string | undefined) => void;
  savePreset: (name: string, search: FilterSearch) => FilterPreset;
  /// Replace an existing preset's filters, keyed on id and keeping its name.
  /// Distinct from [[savePreset]], which looks presets up by name: id-keying
  /// is what lets the save modal write back to the preset the operator
  /// loaded without having to reproduce its name. Returns undefined without
  /// writing when the id is unknown.
  updatePreset: (id: string, search: FilterSearch) => FilterPreset | undefined;
  deletePreset: (id: string) => void;
}

export const useFilterPresets = create<PresetState>()(
  persist(
    (set, get) => ({
      presets: [],
      activePresetId: undefined,
      setActivePreset: (id) => set({ activePresetId: id }),
      savePreset: (name, search) => {
        // Names are unique case-insensitively: saving over an existing name
        // overwrites that preset (keeping its id) rather than adding a clone.
        const existing = get().presets.find(
          (p) => p.name.toLowerCase() === name.toLowerCase(),
        );
        const preset: FilterPreset = {
          id: existing?.id ?? crypto.randomUUID(),
          name,
          search,
        };
        set((state) => ({
          presets: existing
            ? state.presets.map((p) => (p.id === existing.id ? preset : p))
            : [...state.presets, preset],
        }));
        return preset;
      },
      updatePreset: (id, search) => {
        const { presets } = get();
        const target = presets.find((p) => p.id === id);
        if (!target) return undefined;
        const updated: FilterPreset = { ...target, search };
        set({ presets: presets.map((p) => (p.id === id ? updated : p)) });
        return updated;
      },
      deletePreset: (id) =>
        set((state) => ({
          presets: state.presets.filter((p) => p.id !== id),
          activePresetId:
            state.activePresetId === id ? undefined : state.activePresetId,
        })),
    }),
    {
      name: "tsundoku.filter-presets.v1",
      // Only the presets themselves are durable. See activePresetId above.
      partialize: (state) => ({ presets: state.presets }),
    },
  ),
);
