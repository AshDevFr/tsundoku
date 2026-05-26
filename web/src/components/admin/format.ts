/// Compact byte formatter used across admin metrics views. Falls back
/// to `"0 B"` for zero / non-finite inputs so cards never render `NaN B`.
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(value >= 10 || unit === 0 ? 0 : 1)} ${units[unit]}`;
}

/// Human-readable duration in the unit that fits best. Returns `"—"`
/// for zero / null / non-finite, so callers can hand the result straight
/// to a `<Text>` without conditional rendering.
export function formatDuration(seconds: number | null | undefined): string {
  if (
    typeof seconds !== "number" ||
    !Number.isFinite(seconds) ||
    seconds <= 0
  ) {
    return "—";
  }
  if (seconds < 60) return `${Math.round(seconds)}s`;
  const m = Math.round(seconds / 60);
  if (m < 60) return `${m}m`;
  const h = Math.round(seconds / 3600);
  if (h < 24) return `${h}h`;
  const d = Math.round(seconds / 86400);
  return `${d}d`;
}
