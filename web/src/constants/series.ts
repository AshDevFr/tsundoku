// Canonical series vocab shared between the feed filter and the manual-series
// editor. Kept in one place so the two surfaces can't drift apart.

export const KIND_OPTIONS = [
  "manga",
  "manhwa",
  "manhua",
  "novel",
  "one_shot",
  "other",
];

export const STATUS_OPTIONS = [
  "ongoing",
  "completed",
  "hiatus",
  "cancelled",
  "unknown",
];

// Admin-only Codex presence filter. Values match the backend `codexStatus`
// param; the filter is multi-select and OR-combined. Shared by the filter
// panel (option labels) and the route validator (accepted values).
export const CODEX_STATUS_OPTIONS = [
  { value: "any", label: "On Codex (any)" },
  { value: "complete", label: "Owned — up to date" },
  { value: "behind", label: "Owned — behind" },
  { value: "present", label: "Owned — unverified" },
  { value: "ignored", label: "Owned — tracking off" },
  { value: "missing", label: "Not on Codex" },
];

export const CODEX_STATUS_VALUES = CODEX_STATUS_OPTIONS.map((o) => o.value);
