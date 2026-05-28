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

// Backend cover-proxy URL for a persisted series. Used everywhere the
// frontend would otherwise put `series.coverUrl` straight into an
// `<img>`: routing through the API gives a local on-disk cache and an
// operator-visible "drop cache" lever (admin maintenance page).
// Callers should gate on `series.coverUrl != null` to avoid issuing a
// guaranteed-404 request for cover-less rows.
export function coverProxyForSeries(seriesId: number): string {
  return `/api/v1/covers/${seriesId}`;
}

// Backend cover-proxy URL for an arbitrary upstream cover. Used by the
// review/search UI where the candidate is not (yet) a persisted series.
// The backend rejects URLs outside its host allowlist with 400.
export function coverProxyForUrl(coverUrl: string): string {
  return `/api/v1/covers/by-url?url=${encodeURIComponent(coverUrl)}`;
}

// Build the public-facing URL for a (provider, externalId) pair. Returns
// null for providers we don't know how to link to. Persisted URLs are not
// stored server-side because they're fully derivable from these two fields
// and decouple us from upstream URL-scheme changes (e.g. mangabaka.dev →
// mangabaka.org).
export function providerUrl(
  provider: string,
  externalId: string,
): string | null {
  const id = encodeURIComponent(externalId);
  switch (provider) {
    case "mangabaka":
      return `https://mangabaka.org/${id}`;
    case "mangaupdates":
      return `https://www.mangaupdates.com/series/${id}`;
    case "mal":
      return `https://myanimelist.net/manga/${id}`;
    case "anilist":
      return `https://anilist.co/manga/${id}`;
    case "mangadex":
      return `https://mangadex.org/title/${id}`;
    case "kitsu":
      return `https://kitsu.io/manga/${id}`;
    case "anime_planet":
      return `https://www.anime-planet.com/manga/${id}`;
    case "anime_news_network":
      return `https://www.animenewsnetwork.com/encyclopedia/manga.php?id=${id}`;
    case "shikimori":
      return `https://shikimori.one/mangas/${id}`;
    default:
      return null;
  }
}
