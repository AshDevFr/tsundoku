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
//!
//! MangaUpdates migrated from numeric `series.html?id=NNN` IDs to base36
//! alphanumeric `series/{slug}/` IDs in 2022. Both shapes still appear in
//! uploader-pasted URLs. We classify the legacy shape with the synthetic
//! provider tag `mangaupdates-legacy`; a downstream normalization step
//! translates those into modern `mangaupdates` IDs before the resolver
//! pipeline consumes them.

use td_source::ExternalLinks;

/// Synthetic provider tag for unresolved MangaUpdates legacy numeric IDs.
/// Never persisted; only flows through the resolver's normalization step,
/// which swaps each entry for a real `("mangaupdates", modern_id)` pair
/// or drops it on tombstone.
pub const MANGAUPDATES_LEGACY: &str = "mangaupdates-legacy";

/// `(canonical-provider-id, foreign-id, original-url-if-any)` triples for
/// every populated link, in the order the resolver should try them.
pub fn pairs(links: &ExternalLinks) -> Vec<(&'static str, String, Option<String>)> {
    links
        .iter()
        .filter_map(|(provider, url)| {
            let provider = if provider == "mangaupdates" && is_mangaupdates_legacy(url) {
                MANGAUPDATES_LEGACY
            } else {
                provider
            };
            extract_id(provider, url).map(|id| (provider, id, Some(url.to_string())))
        })
        .collect()
}

/// Detect the provider and extract the id from a single pasted string that
/// may be a full provider URL. Returns `None` when no known provider host is
/// present (the caller should then treat the input as a bare id for an
/// explicitly chosen provider, or as a native id).
///
/// The host→provider mapping mirrors `td_source_nyaa::links` (the Nyaa link
/// extractor); keep the two in sync when adding a provider. Legacy
/// MangaUpdates URLs classify as [`MANGAUPDATES_LEGACY`] so callers that
/// can't translate them (e.g. the synchronous review-search path) can tell
/// the operator to use the modern `/series/<slug>` link instead.
pub fn detect(input: &str) -> Option<(&'static str, String)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let provider = if lower.contains("mangaupdates.com") {
        if is_mangaupdates_legacy(trimmed) {
            MANGAUPDATES_LEGACY
        } else {
            "mangaupdates"
        }
    } else if lower.contains("anilist.co") {
        "anilist"
    } else if lower.contains("myanimelist.net") {
        "mal"
    } else if lower.contains("mangadex.org") {
        "mangadex"
    } else if lower.contains("mangabaka.org") || lower.contains("mangabaka.dev") {
        "mangabaka"
    } else {
        return None;
    };
    extract_id(provider, trimmed).map(|id| (provider, id))
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
        MANGAUPDATES_LEGACY => extract_legacy_mu_id(trimmed),
        "anilist" => extract_after_segment(trimmed, "/manga/"),
        "mal" => extract_after_segment(trimmed, "/manga/"),
        "mangadex" => extract_after_segment(trimmed, "/title/"),
        "mangabaka" => extract_mangabaka_id(trimmed),
        _ => None,
    }
}

/// Pull the numeric series id from a MangaBaka URL of the shape
/// `mangabaka.(org|dev)/{id}[/...][?...][#...]`. The site's series pages
/// live at the domain root (no `/series/` segment), so we can't reuse
/// [`extract_after_segment`]; we strip the scheme + host and read the
/// first path token.
fn extract_mangabaka_id(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    let host_idx = lower.find("mangabaka.")?;
    // Advance past "mangabaka.<tld>" by locating the path-start slash.
    let after_host = url.get(host_idx..)?;
    let slash = after_host.find('/')?;
    let path = &after_host[slash + 1..];
    let end = path.find(['/', '?', '#']).unwrap_or(path.len());
    let id = &path[..end];
    if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
        Some(id.to_string())
    } else {
        None
    }
}

/// True when `url` is a legacy MangaUpdates reference: either the
/// `series.html?id=NNN` web shape, or a bare numeric id passed by hand.
fn is_mangaupdates_legacy(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return false;
    }
    if !trimmed.contains('/') {
        // Bare numeric id (e.g. "151349") looks like a legacy MU id.
        return trimmed.chars().all(|c| c.is_ascii_digit());
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.contains("/series.html") && lower.contains("?id=")
}

/// Pull the numeric `id` query parameter from a legacy MU URL.
/// Tolerates extra parameters and any ordering; rejects non-numeric IDs.
fn extract_legacy_mu_id(url: &str) -> Option<String> {
    let q_start = url.find('?')?;
    let query = &url[q_start + 1..];
    for pair in query.split('&') {
        let Some((key, val)) = pair.split_once('=') else {
            continue;
        };
        if key.eq_ignore_ascii_case("id") {
            let val = val.split('#').next().unwrap_or(val);
            if !val.is_empty() && val.chars().all(|c| c.is_ascii_digit()) {
                return Some(val.to_string());
            }
        }
    }
    None
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
    fn mangaupdates_legacy_url_classified_with_synthetic_provider() {
        let links = ExternalLinks {
            mangaupdates: Some("https://www.mangaupdates.com/series.html?id=151349".into()),
            ..ExternalLinks::default()
        };
        let got = pairs(&links);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, MANGAUPDATES_LEGACY);
        assert_eq!(got[0].1, "151349");
    }

    #[test]
    fn mangaupdates_modern_url_keeps_modern_provider() {
        let links = ExternalLinks {
            mangaupdates: Some("https://www.mangaupdates.com/series/6z1uqw7/solo-leveling".into()),
            ..ExternalLinks::default()
        };
        let got = pairs(&links);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "mangaupdates");
        assert_eq!(got[0].1, "6z1uqw7");
    }

    #[test]
    fn mangaupdates_bare_numeric_is_legacy() {
        let links = ExternalLinks {
            mangaupdates: Some("151349".into()),
            ..ExternalLinks::default()
        };
        let got = pairs(&links);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, MANGAUPDATES_LEGACY);
        assert_eq!(got[0].1, "151349");
    }

    #[test]
    fn mangaupdates_bare_alphanumeric_stays_modern() {
        let links = ExternalLinks {
            mangaupdates: Some("6z1uqw7".into()),
            ..ExternalLinks::default()
        };
        let got = pairs(&links);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "mangaupdates");
        assert_eq!(got[0].1, "6z1uqw7");
    }

    #[test]
    fn mangaupdates_legacy_id_extracted_from_query_string() {
        let id = extract_id(
            MANGAUPDATES_LEGACY,
            "https://www.mangaupdates.com/series.html?id=151349",
        );
        assert_eq!(id.as_deref(), Some("151349"));
    }

    #[test]
    fn mangaupdates_legacy_handles_extra_query_params_in_any_order() {
        let id = extract_id(
            MANGAUPDATES_LEGACY,
            "https://www.mangaupdates.com/series.html?foo=1&id=70263&bar=baz",
        );
        assert_eq!(id.as_deref(), Some("70263"));
    }

    #[test]
    fn mangaupdates_legacy_strips_fragment() {
        let id = extract_id(
            MANGAUPDATES_LEGACY,
            "https://www.mangaupdates.com/series.html?id=70263#reviews",
        );
        assert_eq!(id.as_deref(), Some("70263"));
    }

    #[test]
    fn mangaupdates_legacy_rejects_alphanumeric_query_value() {
        // Defensive: the legacy path is for pure-numeric IDs only.
        let id = extract_id(
            MANGAUPDATES_LEGACY,
            "https://www.mangaupdates.com/series.html?id=abc123",
        );
        assert_eq!(id, None);
    }

    #[test]
    fn mangabaka_id_extracted_from_org_url_with_query() {
        let id = extract_id(
            "mangabaka",
            "https://mangabaka.org/35296?utm_source=nyaa&utm_id=oakminati",
        );
        assert_eq!(id.as_deref(), Some("35296"));
    }

    #[test]
    fn mangabaka_id_extracted_from_dev_and_www_variants() {
        for url in [
            "https://mangabaka.dev/12345",
            "https://www.mangabaka.org/67890/report",
            "http://mangabaka.org/1#about",
        ] {
            let id = extract_id("mangabaka", url).expect(url);
            // First numeric path segment.
            assert!(id.chars().all(|c| c.is_ascii_digit()), "non-numeric: {id}");
        }
    }

    #[test]
    fn mangabaka_rejects_non_numeric_first_segment() {
        assert_eq!(
            extract_id("mangabaka", "https://mangabaka.org/search"),
            None
        );
        assert_eq!(
            extract_id("mangabaka", "https://mangabaka.org/api/v1/series/123"),
            None
        );
    }

    #[test]
    fn detect_identifies_each_provider_from_full_url() {
        assert_eq!(
            detect("https://www.mangaupdates.com/series/ylx5wzn/chainsaw-man"),
            Some(("mangaupdates", "ylx5wzn".to_string()))
        );
        assert_eq!(
            detect("https://anilist.co/manga/105778/Chainsaw-Man"),
            Some(("anilist", "105778".to_string()))
        );
        assert_eq!(
            detect("https://myanimelist.net/manga/116778/Chainsaw_Man"),
            Some(("mal", "116778".to_string()))
        );
        assert_eq!(
            detect("https://mangadex.org/title/a77742b1-befd-49a4-bff5-1ad4e6b0ef7b/x"),
            Some((
                "mangadex",
                "a77742b1-befd-49a4-bff5-1ad4e6b0ef7b".to_string()
            ))
        );
        assert_eq!(
            detect("https://mangabaka.org/35296?utm_source=nyaa"),
            Some(("mangabaka", "35296".to_string()))
        );
    }

    #[test]
    fn detect_classifies_legacy_mangaupdates_url() {
        assert_eq!(
            detect("https://www.mangaupdates.com/series.html?id=151349"),
            Some((MANGAUPDATES_LEGACY, "151349".to_string()))
        );
    }

    #[test]
    fn detect_returns_none_for_bare_id_or_unknown_host() {
        // Bare ids carry no host, so the caller decides the provider.
        assert!(detect("151349").is_none());
        assert!(detect("ylx5wzn").is_none());
        assert!(detect("https://example.com/series/1").is_none());
        assert!(detect("   ").is_none());
    }

    #[test]
    fn detect_returns_none_for_non_series_mangabaka_paths() {
        assert!(detect("https://mangabaka.org/search?q=foo").is_none());
    }

    #[test]
    fn pairs_drops_unparseable_links_but_keeps_others() {
        let links = ExternalLinks {
            mangaupdates: Some("https://www.mangaupdates.com/series/abc/foo".into()),
            anilist: Some("not-a-url-but-bare-id".into()),
            mal: Some("https://example.invalid/wrong-shape".into()),
            mangadex: None,
            mangabaka: None,
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
