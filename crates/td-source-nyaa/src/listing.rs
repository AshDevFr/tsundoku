//! Nyaa HTML listing parser, used by the backfill path.
//!
//! Unlike the RSS feed (which silently ignores `&p=N` and always returns
//! the most-recent 75 items), the HTML listing pages at
//! `https://nyaa.si/?c=...&p=N` paginate properly. Each row carries the
//! same fields RSS surfaces — title, view link, magnet, size, posted-at —
//! so we map listing rows into the same [`DiscoveredRelease`] shape and
//! send them through the normal enrich + persist + resolve pipeline.
//!
//! Description HTML and external links are *not* in the listing markup;
//! they only appear on the per-post detail page. The backfill loop is
//! expected to run [`crate::source::NyaaSource`]'s `enrich` after parsing
//! to fill those in when `fetch_details = true`.

use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use scraper::{ElementRef, Html, Selector};
use td_source::{DiscoveredRelease, ExternalLinks};

use crate::SOURCE_KIND;
use crate::parser::{extract_post_id, parse_size};

/// Parse one listing page's HTML into [`DiscoveredRelease`] rows.
///
/// `source_name` is stamped onto every row. `site_base_url` is prepended
/// to relative `/view/N` and `/download/N.torrent` hrefs. Malformed rows
/// (missing title, unparseable timestamp, etc.) are logged and skipped
/// rather than aborting the whole page — a single broken row in a 75-item
/// page must not lose the other 74.
pub fn parse_listing(
    html: &str,
    source_name: &str,
    site_base_url: &str,
) -> Result<Vec<DiscoveredRelease>> {
    let doc = Html::parse_document(html);

    let row_sel = row_selector();
    let mut out = Vec::new();
    for row in doc.select(row_sel) {
        match parse_row(&row, source_name, site_base_url) {
            Ok(r) => out.push(r),
            Err(e) => tracing::warn!(error = %e, "skipping malformed nyaa listing row"),
        }
    }
    Ok(out)
}

fn parse_row(
    row: &ElementRef<'_>,
    source_name: &str,
    site_base_url: &str,
) -> Result<DiscoveredRelease> {
    let title_anchor = row
        .select(title_selector())
        .next()
        .ok_or_else(|| anyhow!("row missing title anchor"))?;
    let view_href = title_anchor
        .value()
        .attr("href")
        .ok_or_else(|| anyhow!("title anchor missing href"))?;
    let external_id =
        extract_post_id(view_href).with_context(|| format!("parsing post id from {view_href}"))?;
    let title = title_anchor
        .value()
        .attr("title")
        .map(str::to_string)
        .unwrap_or_else(|| collect_text(&title_anchor));
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(anyhow!("row has empty title"));
    }
    let link = absolutize(site_base_url, &format!("/view/{external_id}"));

    let magnet = row
        .select(magnet_selector())
        .next()
        .and_then(|a| a.value().attr("href"))
        .map(html_decode_entities);
    let info_hash = magnet.as_deref().and_then(extract_info_hash);

    let torrent_url = row
        .select(torrent_selector())
        .next()
        .and_then(|a| a.value().attr("href"))
        .map(|h| absolutize(site_base_url, h));

    let posted_at = row
        .select(date_selector())
        .next()
        .and_then(|td| td.value().attr("data-timestamp"))
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0))
        .ok_or_else(|| anyhow!("row missing parseable data-timestamp"))?;

    let size_bytes = row
        .select(size_candidate_selector())
        .filter_map(|td| parse_size(&collect_text(&td)))
        .next();

    Ok(DiscoveredRelease {
        source_kind: SOURCE_KIND.into(),
        source_name: source_name.into(),
        external_id,
        title,
        link,
        magnet,
        torrent_url,
        ddl_url: None,
        info_hash,
        size_bytes,
        files: Vec::new(),
        description_html: None,
        external_links: ExternalLinks::default(),
        posted_at,
    })
}

fn row_selector() -> &'static Selector {
    static SEL: OnceLock<Selector> = OnceLock::new();
    SEL.get_or_init(|| Selector::parse("table.torrent-list tbody tr").expect("static selector"))
}

fn title_selector() -> &'static Selector {
    static SEL: OnceLock<Selector> = OnceLock::new();
    SEL.get_or_init(|| {
        Selector::parse(r#"a[href^="/view/"]:not(.comments)"#).expect("static selector")
    })
}

fn magnet_selector() -> &'static Selector {
    static SEL: OnceLock<Selector> = OnceLock::new();
    SEL.get_or_init(|| Selector::parse(r#"a[href^="magnet:"]"#).expect("static selector"))
}

fn torrent_selector() -> &'static Selector {
    static SEL: OnceLock<Selector> = OnceLock::new();
    SEL.get_or_init(|| Selector::parse(r#"a[href^="/download/"]"#).expect("static selector"))
}

fn date_selector() -> &'static Selector {
    static SEL: OnceLock<Selector> = OnceLock::new();
    SEL.get_or_init(|| Selector::parse("td[data-timestamp]").expect("static selector"))
}

/// Cells that *might* hold the size string. The size column has no unique
/// class, but it's the only `td.text-center` whose text parses as a size
/// (the others are seeders / leechers / completed integer counts and the
/// date cell, which never matches the unit suffix).
fn size_candidate_selector() -> &'static Selector {
    static SEL: OnceLock<Selector> = OnceLock::new();
    SEL.get_or_init(|| Selector::parse("td.text-center").expect("static selector"))
}

fn collect_text(el: &ElementRef<'_>) -> String {
    el.text().collect::<String>()
}

fn absolutize(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    let base = base.trim_end_matches('/');
    if href.starts_with('/') {
        format!("{base}{href}")
    } else {
        format!("{base}/{href}")
    }
}

/// Pull `HEX` out of a `magnet:?xt=urn:btih:HEX&...` URL, lowercased.
/// Returns `None` if the magnet doesn't carry a btih hash.
fn extract_info_hash(magnet: &str) -> Option<String> {
    let lower = magnet;
    let needle = "xt=urn:btih:";
    let idx = lower.find(needle)?;
    let rest = &lower[idx + needle.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(rest.len());
    let hash = &rest[..end];
    if hash.is_empty() {
        None
    } else {
        Some(hash.to_ascii_lowercase())
    }
}

fn html_decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/nyaa_listing_page2.html");

    #[test]
    fn parses_75_rows_from_real_listing_page() {
        let rows = parse_listing(FIXTURE, "trusted", "https://nyaa.si").unwrap();
        assert_eq!(rows.len(), 75, "Nyaa listing pages serve 75 rows");
        for r in &rows {
            assert_eq!(r.source_kind, "nyaa");
            assert_eq!(r.source_name, "trusted");
            assert!(!r.external_id.is_empty());
            assert!(r.link.starts_with("https://nyaa.si/view/"));
            assert!(!r.title.is_empty());
        }
    }

    #[test]
    fn first_row_fields_match_fixture() {
        let rows = parse_listing(FIXTURE, "trusted", "https://nyaa.si").unwrap();
        let first = &rows[0];
        assert_eq!(first.external_id, "1180952");
        assert_eq!(first.link, "https://nyaa.si/view/1180952");
        assert!(first.title.starts_with("[Doki] Kabe ni Mary.com"));
        assert_eq!(
            first.torrent_url.as_deref(),
            Some("https://nyaa.si/download/1180952.torrent")
        );
        // Magnet should be entity-decoded (no `&amp;`) and start with btih.
        let magnet = first.magnet.as_deref().unwrap();
        assert!(magnet.starts_with("magnet:?xt=urn:btih:"));
        assert!(!magnet.contains("&amp;"));
        assert_eq!(
            first.info_hash.as_deref(),
            Some("f1feaa5781a34eb29ec8dba5285982f9b337f2d8")
        );
        assert_eq!(first.size_bytes, Some((14.6_f64 * 1024.0 * 1024.0) as u64));
        // 1570119440 = 2019-10-03 16:17:20 UTC.
        assert_eq!(first.posted_at.timestamp(), 1_570_119_440);
        // Listing rows don't carry descriptions or external links —
        // those come from the detail page during `enrich`.
        assert!(first.description_html.is_none());
        assert!(first.external_links.is_empty());
        assert!(first.files.is_empty());
    }

    #[test]
    fn rows_with_comments_still_parse_title_anchor() {
        // 17 of the 75 rows in the fixture carry a comments link in the
        // same cell as the title. The `:not(.comments)` selector should
        // skip the comments anchor and pick the title anchor.
        let rows = parse_listing(FIXTURE, "trusted", "https://nyaa.si").unwrap();
        let with_comments = rows
            .iter()
            .find(|r| r.external_id == "1179240")
            .expect("fixture should contain the row with a comments link");
        // The title is the post title, not the comments-anchor text ("1").
        assert!(
            with_comments.title.len() > 3,
            "title should be the post title, got {:?}",
            with_comments.title
        );
    }

    #[test]
    fn extract_info_hash_lowercases_and_truncates_at_delimiter() {
        assert_eq!(
            extract_info_hash("magnet:?xt=urn:btih:ABCDEF1234567890&dn=foo"),
            Some("abcdef1234567890".into())
        );
        assert_eq!(extract_info_hash("not a magnet"), None);
    }

    #[test]
    fn absolutize_keeps_absolute_urls_and_prefixes_relative() {
        assert_eq!(
            absolutize("https://nyaa.si", "/view/42"),
            "https://nyaa.si/view/42"
        );
        assert_eq!(
            absolutize("https://nyaa.si/", "/view/42"),
            "https://nyaa.si/view/42"
        );
        assert_eq!(
            absolutize("https://nyaa.si", "https://other.test/view/42"),
            "https://other.test/view/42"
        );
    }
}
