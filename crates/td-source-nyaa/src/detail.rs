//! Nyaa post detail page parser.
//!
//! Pulls the file list, magnet link, description, and (if present) any
//! external provider links from a single rendered HTML page. Used by
//! `NyaaSource` when `fetch_details = true`.

use std::sync::OnceLock;

use anyhow::Result;
use regex::Regex;
use scraper::{Html, Selector};
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
}

pub fn parse_detail(html: &str, _site_base_url: &str) -> Result<DetailFields> {
    let doc = Html::parse_document(html);

    let mut out = DetailFields::default();
    out.files = parse_file_list(&doc);
    out.magnet = parse_magnet(html);
    out.description_html = parse_description_html(&doc);
    out.external_links = match out.description_html.as_deref() {
        Some(desc) => extract_external_links(desc),
        None => extract_external_links(html),
    };
    Ok(out)
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
    }
}
