//! Canonical release DTO and polling context.
//!
//! Every implementation of [`crate::DiscoverySource`] maps its native payload
//! into [`DiscoveredRelease`]. Persistence keys on `(source_kind, external_id)`
//! and on `link`, so the source must emit stable values for both.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One release observation from a discovery source. Source-agnostic shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredRelease {
    /// Kind of the source that produced this row (e.g. `"nyaa"`). Persisted
    /// to `releases.source_kind`.
    pub source_kind: String,
    /// Config-defined instance name of the source. Persisted to
    /// `releases.source_name` so multiple instances of the same kind stay
    /// distinguishable in the UI.
    pub source_name: String,
    /// Stable per-source identifier (e.g. a Nyaa post id). Combined with
    /// `source_kind` forms the upsert key for the `releases` row.
    pub external_id: String,
    pub title: String,
    /// Canonical page link. Also unique in the `releases` table.
    pub link: String,
    pub magnet: Option<String>,
    pub torrent_url: Option<String>,
    /// Direct download URL, if this source publishes one (e.g. DDL feeds).
    pub ddl_url: Option<String>,
    pub info_hash: Option<String>,
    pub size_bytes: Option<u64>,
    /// File names inside the torrent / archive, used by the format detector.
    /// Empty when the source does not expose a file list.
    #[serde(default)]
    pub files: Vec<String>,
    pub description_html: Option<String>,
    /// External provider links extracted from the post body / description.
    /// Empty fields mean the source could not find that provider's link.
    #[serde(default)]
    pub external_links: ExternalLinks,
    /// URL the uploader cited in the post's "Information" field, verbatim.
    /// Unlike [`ExternalLinks`], this is kept even when it points at a site
    /// we don't resolve against (a publisher page, a Discord invite, …) so
    /// the review UI can surface it. `None` when the source exposes no such
    /// field. Persisted to `releases.information_url`.
    #[serde(default)]
    pub information_url: Option<String>,
    pub posted_at: DateTime<Utc>,
}

/// External provider links a source may surface from a release. Persisted
/// to `releases.extracted_links_json` and consumed by the resolution
/// pipeline's foreign-ID lookup.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExternalLinks {
    pub mangaupdates: Option<String>,
    pub anilist: Option<String>,
    pub mal: Option<String>,
    pub mangadex: Option<String>,
    /// MangaBaka link. Carried even when MangaBaka is the active provider
    /// so the resolver can short-circuit fuzzy matching by calling
    /// `active.get(id)` directly.
    #[serde(default)]
    pub mangabaka: Option<String>,
}

impl ExternalLinks {
    pub fn is_empty(&self) -> bool {
        self.mangaupdates.is_none()
            && self.anilist.is_none()
            && self.mal.is_none()
            && self.mangadex.is_none()
            && self.mangabaka.is_none()
    }

    /// Yield `(provider, id_or_url)` pairs for every populated link. The
    /// resolver iterates this directly to drive `resolve_by_foreign_id`.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &str)> {
        [
            ("mangaupdates", self.mangaupdates.as_deref()),
            ("anilist", self.anilist.as_deref()),
            ("mal", self.mal.as_deref()),
            ("mangadex", self.mangadex.as_deref()),
            ("mangabaka", self.mangabaka.as_deref()),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.map(|s| (k, s)))
    }
}

/// Per-poll context passed to [`crate::DiscoverySource::poll`]. Sources read
/// the previous ETag / cursor / last-success marker and write back the new
/// values for the next run.
#[derive(Debug, Clone, Default)]
pub struct PollContext {
    /// ETag returned by the last successful poll, if the source supports
    /// conditional GETs. `None` means "never polled" or "ETag unsupported".
    pub etag: Option<String>,
    /// Opaque cursor for sources that paginate. Source-specific format.
    pub cursor: Option<String>,
    /// Timestamp of the last successful poll, if any. Sources may use it to
    /// short-circuit when items older than this can be assumed already
    /// observed.
    pub last_success_at: Option<DateTime<Utc>>,
    /// `external_id`s already persisted for this `(source_kind, source_name)`.
    /// Populated by the caller from the recent tail of the `releases` table;
    /// sources should drop matching items before any per-item enrichment
    /// (detail fetches, etc.) so steady-state polls stay cheap even when an
    /// ETag flake or pagination overlap re-surfaces known posts.
    pub recently_seen: HashSet<String>,
}

/// What a source produced. Distinct from `Vec<DiscoveredRelease>` so a source
/// can also return new state markers (ETag, cursor) without bundling them
/// into a side channel.
#[derive(Debug, Clone, Default)]
pub struct PollOutcome {
    pub releases: Vec<DiscoveredRelease>,
    /// New ETag observed on the upstream response, propagated to
    /// `source_state.etag` for the next run.
    pub new_etag: Option<String>,
    /// New cursor, for paginated sources.
    pub new_cursor: Option<String>,
    /// True when the upstream returned 304 Not Modified (or equivalent) and
    /// the source skipped parsing. Useful for logging.
    pub not_modified: bool,
}

impl PollOutcome {
    pub fn from_releases(releases: Vec<DiscoveredRelease>) -> Self {
        Self {
            releases,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_links_iter_yields_only_populated_entries() {
        let links = ExternalLinks {
            mangaupdates: Some("https://www.mangaupdates.com/series/abc".into()),
            anilist: None,
            mal: Some("12345".into()),
            mangadex: None,
            mangabaka: Some("https://mangabaka.org/42".into()),
        };
        let collected: Vec<(&str, &str)> = links.iter().collect();
        assert_eq!(
            collected,
            vec![
                ("mangaupdates", "https://www.mangaupdates.com/series/abc"),
                ("mal", "12345"),
                ("mangabaka", "https://mangabaka.org/42"),
            ]
        );
        assert!(!links.is_empty());
    }

    #[test]
    fn external_links_default_is_empty() {
        let links = ExternalLinks::default();
        assert!(links.is_empty());
        assert_eq!(links.iter().count(), 0);
    }
}
