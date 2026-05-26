// Unix-seconds → "2 hours ago" for the feed UI. The backend serializes every
// timestamp as a signed 64-bit unix-seconds integer.
export function formatRelative(
  unixSeconds: number,
  now: number = Date.now(),
): string {
  const diffSec = Math.max(0, Math.floor(now / 1000 - unixSeconds));
  if (diffSec < 60) return "just now";
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h ago`;
  const diffDay = Math.floor(diffHr / 24);
  if (diffDay < 30) return `${diffDay}d ago`;
  const diffMo = Math.floor(diffDay / 30);
  if (diffMo < 12) return `${diffMo}mo ago`;
  return `${Math.floor(diffMo / 12)}y ago`;
}

export function formatAbsolute(unixSeconds: number): string {
  return new Date(unixSeconds * 1000).toLocaleString();
}
