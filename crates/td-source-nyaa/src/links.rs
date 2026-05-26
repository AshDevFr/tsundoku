//! External provider link extraction from Nyaa description HTML / post detail.
//!
//! The strategy is regex-over-href: collect every `href="..."` value, then
//! match the URL host against the providers we know how to resolve. Avoiding
//! a DOM walk keeps the implementation tolerant of malformed descriptions
//! (the RSS feed's CDATA blocks rarely close their tags properly).

use std::sync::OnceLock;

use regex::Regex;
use td_source::ExternalLinks;

/// Scan an HTML/text blob for known external provider links. Honors the
/// first occurrence of each provider; later duplicates are ignored.
pub fn extract_external_links(html: &str) -> ExternalLinks {
    let mut out = ExternalLinks::default();
    for url in iter_urls(html) {
        absorb_url(&mut out, url);
        if out.mangaupdates.is_some()
            && out.anilist.is_some()
            && out.mal.is_some()
            && out.mangadex.is_some()
        {
            break;
        }
    }
    out
}

fn iter_urls(html: &str) -> impl Iterator<Item = &str> {
    static HREF_RE: OnceLock<Regex> = OnceLock::new();
    static BARE_RE: OnceLock<Regex> = OnceLock::new();
    let href = HREF_RE.get_or_init(|| Regex::new(r#"href\s*=\s*["']([^"']+)["']"#).unwrap());
    // Also catch bare URLs (Nyaa descriptions often include them without
    // anchor tags). We restrict to https? + a couple of providers so we
    // don't slurp arbitrary tokens.
    let bare = BARE_RE.get_or_init(|| {
        Regex::new(
            r#"(?i)https?://(?:www\.)?(?:mangaupdates|anilist|myanimelist|mangadex)[^\s<>"']+"#,
        )
        .unwrap()
    });
    let from_href = href
        .captures_iter(html)
        .filter_map(|c| c.get(1).map(|m| m.as_str()));
    let from_bare = bare.find_iter(html).map(|m| m.as_str());
    from_href.chain(from_bare)
}

fn absorb_url(out: &mut ExternalLinks, raw: &str) {
    let url = raw.trim();
    if url.is_empty() {
        return;
    }
    let lower = url.to_ascii_lowercase();
    if out.mangaupdates.is_none()
        && (lower.contains("mangaupdates.com/series/")
            || lower.contains("mangaupdates.com/series.html"))
    {
        out.mangaupdates = Some(url.to_string());
        return;
    }
    if out.anilist.is_none() && lower.contains("anilist.co/manga/") {
        out.anilist = Some(url.to_string());
        return;
    }
    if out.mal.is_none() && lower.contains("myanimelist.net/manga/") {
        out.mal = Some(url.to_string());
        return;
    }
    if out.mangadex.is_none() && lower.contains("mangadex.org/title/") {
        out.mangadex = Some(url.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_each_provider_from_anchor_tags() {
        let html = r#"
            See more:
            <a href="https://www.mangaupdates.com/series/abc123/foo-bar">MU</a> |
            <a href="https://anilist.co/manga/123456">AL</a> |
            <a href="https://myanimelist.net/manga/789">MAL</a> |
            <a href="https://mangadex.org/title/abcd-efgh">MD</a>
        "#;
        let links = extract_external_links(html);
        assert!(
            links
                .mangaupdates
                .as_deref()
                .unwrap()
                .contains("mangaupdates.com/series/abc123")
        );
        assert!(
            links
                .anilist
                .as_deref()
                .unwrap()
                .contains("anilist.co/manga/123456")
        );
        assert!(
            links
                .mal
                .as_deref()
                .unwrap()
                .contains("myanimelist.net/manga/789")
        );
        assert!(
            links
                .mangadex
                .as_deref()
                .unwrap()
                .contains("mangadex.org/title/abcd-efgh")
        );
    }

    #[test]
    fn extracts_bare_urls_without_anchors() {
        let html = "https://www.mangaupdates.com/series/xyz999/random-title is the source";
        let links = extract_external_links(html);
        assert!(links.mangaupdates.is_some());
    }

    #[test]
    fn first_occurrence_wins() {
        let html = r#"
            <a href="https://anilist.co/manga/1">first</a>
            <a href="https://anilist.co/manga/2">second</a>
        "#;
        let links = extract_external_links(html);
        assert_eq!(links.anilist.as_deref(), Some("https://anilist.co/manga/1"));
    }

    #[test]
    fn ignores_links_to_unrelated_sites() {
        let html =
            r#"<a href="https://discord.gg/foo">discord</a><a href="https://example.com">x</a>"#;
        let links = extract_external_links(html);
        assert!(links.is_empty());
    }

    #[test]
    fn ignores_anilist_anime_routes() {
        // Only manga routes are useful for MetadataProvider resolution.
        let html = r#"<a href="https://anilist.co/anime/123">AL anime</a>"#;
        let links = extract_external_links(html);
        assert!(links.anilist.is_none());
    }
}
