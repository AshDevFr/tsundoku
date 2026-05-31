//! Nyaa post detail page parser.
//!
//! Pulls the file list, magnet link, description, and (if present) any
//! external provider links from a single rendered HTML page. Used by
//! `NyaaSource` when `fetch_details = true`.

use std::sync::OnceLock;

use anyhow::Result;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use td_source::ExternalLinks;

use crate::links::extract_external_links;

/// Bundle of fields the detail page produces. Each is `None` / empty if the
/// page does not expose it, leaving the RSS-derived values intact in the
/// merge step inside `NyaaSource::poll`.
#[derive(Debug, Default)]
pub struct DetailFields {
    /// File list (folders flattened away — only leaf file names are kept).
    pub files: Vec<String>,
    pub magnet: Option<String>,
    pub description_html: Option<String>,
    pub external_links: ExternalLinks,
    /// Provider links found in the post's *comment* section. Untrusted (any
    /// user can comment), so the resolver never consumes them; the review UI
    /// surfaces them as operator-confirmable suggestions. Empty when there are
    /// no comments or none carry a recognizable link.
    pub comment_suggested_links: ExternalLinks,
    /// URL from the post's "Information" row, verbatim. Captured whether or
    /// not it points at a provider we resolve against — the review UI shows
    /// it as the uploader's cited source. `None` when the row is absent or
    /// holds plain text (e.g. Nyaa's "No information.").
    pub information_url: Option<String>,
}

pub fn parse_detail(html: &str, _site_base_url: &str) -> Result<DetailFields> {
    let doc = Html::parse_document(html);

    let mut out = DetailFields::default();
    out.files = parse_file_list(&doc);
    out.magnet = parse_magnet(html);
    out.description_html = parse_description_html(&doc);
    // Comment links first: the no-description fallback below scans the whole
    // page (comments included), so we need these to subtract them out and keep
    // an untrusted commenter link from ever reaching `external_links` (which
    // feeds the resolver).
    out.comment_suggested_links = parse_comment_links(&doc);
    out.external_links = match out.description_html.as_deref() {
        Some(desc) => extract_external_links(desc),
        None => subtract_links(extract_external_links(html), &out.comment_suggested_links),
    };
    out.information_url = parse_information_url(&doc);
    Ok(out)
}

/// Extract provider links from the post's comment section (`#comments`),
/// kept apart from the uploader's [`DetailFields::external_links`]. Returns
/// an empty set when there is no comment panel.
fn parse_comment_links(doc: &Html) -> ExternalLinks {
    static COMMENTS_SEL: OnceLock<Selector> = OnceLock::new();
    let sel = COMMENTS_SEL.get_or_init(|| Selector::parse("#comments").expect("static selector"));
    match doc.select(sel).next() {
        Some(comments) => extract_external_links(&comments.inner_html()),
        None => ExternalLinks::default(),
    }
}

/// Drop from `base` any provider link that is identical to the one in
/// `remove`. Used by the no-description fallback, which scans the whole page
/// (comments included): this guarantees a comment-sourced link can never
/// masquerade as an uploader link in `external_links`, which is the only
/// links field the resolver consumes.
fn subtract_links(mut base: ExternalLinks, remove: &ExternalLinks) -> ExternalLinks {
    if base.mangaupdates.is_some() && base.mangaupdates == remove.mangaupdates {
        base.mangaupdates = None;
    }
    if base.anilist.is_some() && base.anilist == remove.anilist {
        base.anilist = None;
    }
    if base.mal.is_some() && base.mal == remove.mal {
        base.mal = None;
    }
    if base.mangadex.is_some() && base.mangadex == remove.mangadex {
        base.mangadex = None;
    }
    if base.mangabaka.is_some() && base.mangabaka == remove.mangabaka {
        base.mangabaka = None;
    }
    base
}

/// Pull the URL out of the post's "Information" row. The detail page lays
/// each field as a `col-md-1` label followed by its `col-md-5` value cell;
/// we find the label whose text is "Information:" and read the first http(s)
/// link from the sibling cell (anchor `href` first, then bare-URL text).
/// Returns `None` when the row is missing or its value is not a URL.
fn parse_information_url(doc: &Html) -> Option<String> {
    static LABEL_SEL: OnceLock<Selector> = OnceLock::new();
    static A_SEL: OnceLock<Selector> = OnceLock::new();
    let label_sel =
        LABEL_SEL.get_or_init(|| Selector::parse("div.col-md-1").expect("static selector"));
    let a_sel = A_SEL.get_or_init(|| Selector::parse("a").expect("static selector"));
    for label in doc.select(label_sel) {
        if label.text().collect::<String>().trim() != "Information:" {
            continue;
        }
        // The value lives in the next sibling element (a `col-md-5`).
        let value = label.next_siblings().find_map(ElementRef::wrap)?;
        if let Some(href) = value
            .select(a_sel)
            .next()
            .and_then(|a| a.value().attr("href"))
        {
            let href = href.trim();
            if is_http_url(href) {
                return Some(href.to_string());
            }
        }
        let text = value.text().collect::<String>();
        let text = text.trim();
        return is_http_url(text).then(|| text.to_string());
    }
    None
}

fn is_http_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

fn parse_file_list(doc: &Html) -> Vec<String> {
    static FILE_LI_SEL: OnceLock<Selector> = OnceLock::new();
    let sel = FILE_LI_SEL
        .get_or_init(|| Selector::parse("div.torrent-file-list li").expect("static selector"));
    let mut out = Vec::new();
    for li in doc.select(sel) {
        // Skip folder rows: they have an anchor.folder child.
        if li
            .select(&Selector::parse("a.folder").expect("static selector"))
            .next()
            .is_some()
        {
            continue;
        }
        // The leaf text holds "filename (size)"; trim the size span away.
        let mut text = String::new();
        for child in li.children() {
            if let Some(t) = child.value().as_text() {
                text.push_str(t);
            }
        }
        let cleaned = text.trim().to_string();
        if !cleaned.is_empty() {
            out.push(cleaned);
        }
    }
    out
}

fn parse_magnet(html: &str) -> Option<String> {
    static MAGNET_RE: OnceLock<Regex> = OnceLock::new();
    let re =
        MAGNET_RE.get_or_init(|| Regex::new(r#"href="(magnet:\?[^"]+)""#).expect("static regex"));
    let captured = re.captures(html)?.get(1)?.as_str().to_string();
    Some(html_decode_entities(&captured))
}

fn parse_description_html(doc: &Html) -> Option<String> {
    static DESC_SEL: OnceLock<Selector> = OnceLock::new();
    let sel =
        DESC_SEL.get_or_init(|| Selector::parse("#torrent-description").expect("static selector"));
    doc.select(sel).next().map(|el| el.inner_html())
}

/// Decode the handful of HTML entities Nyaa uses in href attributes
/// (`&amp;`). The full set is overkill; magnet URLs only contain `&` after
/// escape.
fn html_decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINGLE_FILE: &str = include_str!("../tests/fixtures/nyaa_detail_single_file.html");
    const MULTI_FILE: &str = include_str!("../tests/fixtures/nyaa_detail_multi_file.html");

    #[test]
    fn parses_single_file_detail_page() {
        let detail = parse_detail(SINGLE_FILE, "https://nyaa.si").unwrap();
        assert_eq!(detail.files.len(), 1);
        assert!(
            detail.files[0].ends_with(".epub"),
            "expected single .epub file; got {:?}",
            detail.files
        );
        let magnet = detail.magnet.as_deref().unwrap();
        assert!(magnet.starts_with("magnet:?xt=urn:btih:"));
        // Magnet should be entity-decoded.
        assert!(!magnet.contains("&amp;"));
        // Description block exists for this post.
        assert!(detail.description_html.is_some());
        // The "Information" row holds a non-provider link (Discord); we
        // still capture it verbatim for display.
        assert_eq!(
            detail.information_url.as_deref(),
            Some("https://discord.gg/r9gyPwJeqW")
        );
    }

    #[test]
    fn parses_multi_file_detail_page_flattening_folders() {
        let detail = parse_detail(MULTI_FILE, "https://nyaa.si").unwrap();
        // Folder rows skipped; only leaf files kept.
        assert!(detail.files.len() > 1);
        for f in &detail.files {
            assert!(
                !f.contains("Tsundere Service Providers"),
                "folder rows leaked into file list: {f:?}"
            );
        }
        assert!(detail.magnet.is_some());
        assert_eq!(
            detail.information_url.as_deref(),
            Some("https://tsundere.services/")
        );
    }

    #[test]
    fn information_url_absent_when_row_is_plain_text() {
        // Nyaa renders "No information." as plain text when the uploader
        // left the field empty; that must not be captured as a URL.
        let html = r#"
            <div class="panel-body">
              <div class="row">
                <div class="col-md-1">Information:</div>
                <div class="col-md-5">No information.</div>
              </div>
            </div>
        "#;
        let detail = parse_detail(html, "https://nyaa.si").unwrap();
        assert_eq!(detail.information_url, None);
    }

    #[test]
    fn comment_links_are_separated_from_uploader_links() {
        // A post whose body has no provider link, but a commenter pasted a
        // MangaUpdates link. It must land in comment_suggested_links (a
        // review-only hint), never in external_links (the resolver's input).
        let html = r#"
            <div id="torrent-description">Uploader notes, nothing linked here.</div>
            <div id="comments">
              <div class="panel comment-panel">
                <div class="comment-content">
                  this is https://www.mangaupdates.com/series/ylx5wzn/chainsaw-man
                </div>
              </div>
            </div>
        "#;
        let detail = parse_detail(html, "https://nyaa.si").unwrap();
        assert!(
            detail.external_links.is_empty(),
            "comment link leaked into uploader links: {:?}",
            detail.external_links
        );
        assert_eq!(
            detail.comment_suggested_links.mangaupdates.as_deref(),
            Some("https://www.mangaupdates.com/series/ylx5wzn/chainsaw-man")
        );
    }

    #[test]
    fn comment_links_empty_when_no_comment_panel() {
        let html = r#"<div id="torrent-description">no comments section</div>"#;
        let detail = parse_detail(html, "https://nyaa.si").unwrap();
        assert!(detail.comment_suggested_links.is_empty());
    }

    #[test]
    fn comment_link_does_not_leak_into_external_links_without_description() {
        // No `#torrent-description` block, so external_links falls back to a
        // whole-page scan. The only provider link is in a comment — it must
        // NOT leak into external_links (the resolver's input), only into the
        // suggestions.
        let html = r#"
            <div class="panel-body">file list etc, no description block</div>
            <div id="comments">
              <div class="comment-content">
                see https://www.mangaupdates.com/series/ylx5wzn/x
              </div>
            </div>
        "#;
        let detail = parse_detail(html, "https://nyaa.si").unwrap();
        assert!(
            detail.external_links.is_empty(),
            "comment link leaked into external_links: {:?}",
            detail.external_links
        );
        assert_eq!(
            detail.comment_suggested_links.mangaupdates.as_deref(),
            Some("https://www.mangaupdates.com/series/ylx5wzn/x")
        );
    }

    #[test]
    fn uploader_link_survives_when_no_description_and_comment_differs() {
        // No description block: a bare uploader link in the body plus a
        // different comment link. The uploader link must remain in
        // external_links; only the matching comment link is subtracted.
        let html = r#"
            <div class="panel-body">
              https://anilist.co/manga/123 by the uploader
            </div>
            <div id="comments">
              <div class="comment-content">
                https://www.mangaupdates.com/series/ylx5wzn/x
              </div>
            </div>
        "#;
        let detail = parse_detail(html, "https://nyaa.si").unwrap();
        assert_eq!(
            detail.external_links.anilist.as_deref(),
            Some("https://anilist.co/manga/123"),
            "uploader link was wrongly dropped: {:?}",
            detail.external_links
        );
        assert!(detail.external_links.mangaupdates.is_none());
        assert_eq!(
            detail.comment_suggested_links.mangaupdates.as_deref(),
            Some("https://www.mangaupdates.com/series/ylx5wzn/x")
        );
    }
}
