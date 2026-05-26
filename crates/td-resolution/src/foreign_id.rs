//! Extract a provider-canonical ID from the URLs that
//! [`td_source::ExternalLinks`] hands us.
//!
//! `ExternalLinks` stores full URLs (the source crate didn't want to make
//! assumptions about ID shape per-provider), but
//! [`td_metadata::MetadataProvider::resolve_by_foreign_id`] takes the raw
//! provider ID. This module is the single place that knows the URL shape
//! of every external provider we currently surface.
//!
//! Returns the original `id_or_url` as a fallback for inputs that look
//! like a bare ID already, so callers can pass strings from either
//! source-extracted URLs or hand-supplied IDs uniformly.

use td_source::ExternalLinks;

/// `(canonical-provider-id, foreign-id, original-url-if-any)` triples for
/// every populated link, in the order the resolver should try them.
pub fn pairs(links: &ExternalLinks) -> Vec<(&'static str, String, Option<String>)> {
    links
        .iter()
        .filter_map(|(provider, url)| {
            extract_id(provider, url).map(|id| (provider, id, Some(url.to_string())))
        })
        .collect()
}

/// Pull the provider-side external ID from a known-shape URL. Returns
/// `None` only when the URL doesn't look like one we can map; for bare
/// IDs (no `/`, no scheme) it returns the input unchanged.
pub fn extract_id(provider: &str, url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }
    // If there's no scheme and no slash, treat as a bare ID already.
    if !trimmed.contains('/') {
        return Some(trimmed.to_string());
    }
    match provider {
        "mangaupdates" => extract_after_segment(trimmed, "/series/"),
        "anilist" => extract_after_segment(trimmed, "/manga/"),
        "mal" => extract_after_segment(trimmed, "/manga/"),
        "mangadex" => extract_after_segment(trimmed, "/title/"),
        _ => None,
    }
}

/// Find `segment` in `url` and return the next path token, stripped of
/// trailing query / fragment / additional path. Returns `None` if `segment`
/// is absent or the token is empty.
fn extract_after_segment(url: &str, segment: &str) -> Option<String> {
    let idx = url.find(segment)?;
    let rest = &url[idx + segment.len()..];
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let id = &rest[..end];
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mangaupdates_slug_extracted_from_full_url() {
        let id = extract_id(
            "mangaupdates",
            "https://www.mangaupdates.com/series/ylx5wzn/chainsaw-man",
        );
        assert_eq!(id.as_deref(), Some("ylx5wzn"));
    }

    #[test]
    fn anilist_numeric_id_extracted() {
        let id = extract_id("anilist", "https://anilist.co/manga/105778/Chainsaw-Man");
        assert_eq!(id.as_deref(), Some("105778"));
    }

    #[test]
    fn mal_numeric_id_extracted_with_trailing_path() {
        let id = extract_id(
            "mal",
            "https://myanimelist.net/manga/116778/Chainsaw_Man?q=foo",
        );
        assert_eq!(id.as_deref(), Some("116778"));
    }

    #[test]
    fn mangadex_uuid_extracted() {
        let id = extract_id(
            "mangadex",
            "https://mangadex.org/title/a77742b1-befd-49a4-bff5-1ad4e6b0ef7b/chainsaw-man",
        );
        assert_eq!(id.as_deref(), Some("a77742b1-befd-49a4-bff5-1ad4e6b0ef7b"));
    }

    #[test]
    fn bare_id_passes_through_unchanged() {
        assert_eq!(extract_id("anilist", "105778").as_deref(), Some("105778"));
        assert_eq!(
            extract_id("mangaupdates", "ylx5wzn").as_deref(),
            Some("ylx5wzn")
        );
    }

    #[test]
    fn unknown_provider_returns_none_for_urls() {
        assert!(extract_id("unknown", "https://example.com/series/1").is_none());
    }

    #[test]
    fn unknown_provider_passes_bare_id_through() {
        // Bare-ID heuristic doesn't depend on the provider table, so a
        // future provider whose ID arrived as a raw token still resolves.
        assert_eq!(extract_id("unknown", "abc123").as_deref(), Some("abc123"));
    }

    #[test]
    fn empty_input_yields_none() {
        assert!(extract_id("anilist", "").is_none());
        assert!(extract_id("mangaupdates", "  ").is_none());
    }

    #[test]
    fn pairs_drops_unparseable_links_but_keeps_others() {
        let links = ExternalLinks {
            mangaupdates: Some("https://www.mangaupdates.com/series/abc/foo".into()),
            anilist: Some("not-a-url-but-bare-id".into()),
            mal: Some("https://example.invalid/wrong-shape".into()),
            mangadex: None,
        };
        let got = pairs(&links);
        // mangaupdates: parsed; anilist: bare passthrough; mal: contains /, no /manga/ segment → dropped.
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "mangaupdates");
        assert_eq!(got[0].1, "abc");
        assert_eq!(got[1].0, "anilist");
        assert_eq!(got[1].1, "not-a-url-but-bare-id");
    }
}
