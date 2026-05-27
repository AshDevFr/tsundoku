//! Nyaa RSS parser.
//!
//! Nyaa publishes a standard RSS 2.0 feed with a `xmlns:nyaa` namespace for
//! per-post torrent metadata: info hash, size string ("11.6 MiB"), trusted
//! flag, etc. We map the namespaced fields by element local name to keep
//! the parser tolerant of namespace prefix changes.

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use quick_xml::Reader;
use quick_xml::escape::{resolve_predefined_entity, unescape};
use quick_xml::events::Event;
use quick_xml::name::QName;
use td_source::DiscoveredRelease;

use crate::SOURCE_KIND;
use crate::links::extract_external_links;

/// Parse the body of a Nyaa RSS response into [`DiscoveredRelease`] rows.
///
/// `source_name` is the config-defined instance name; the parser stamps it
/// on every row alongside `source_kind = "nyaa"`. The post id (parsed from
/// the `guid` link) is used as `external_id`.
pub fn parse_feed(body: &str, source_name: &str) -> Result<Vec<DiscoveredRelease>> {
    let mut reader = Reader::from_str(body);
    // Don't ask quick_xml to trim text segments. When an element contains
    // an entity reference (`<title>Foo &amp; Bar</title>`), quick_xml
    // splits it into Text("Foo ") + GeneralRef("amp") + Text(" Bar") and
    // trims each Text individually — the spaces adjacent to the entity
    // collapse and we get "Foo&Bar". Every field this parser cares about
    // is explicitly trimmed in `build`, so leaving inner whitespace alone
    // is harmless.
    reader.config_mut().trim_text(false);

    let mut releases = Vec::new();
    let mut depth: u32 = 0;
    let mut in_channel = false;
    let mut in_item = false;
    let mut current_field: Option<Field> = None;
    let mut item = ItemBuilder::default();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name_buf = local_name_owned(e.name());
                depth += 1;
                if !in_channel && name_buf == "channel" {
                    in_channel = true;
                    continue;
                }
                if in_channel && !in_item && name_buf == "item" {
                    in_item = true;
                    item = ItemBuilder::default();
                    continue;
                }
                if in_item {
                    current_field = Field::from_local(&name_buf);
                }
            }
            Ok(Event::End(e)) => {
                let name_buf = local_name_owned(e.name());
                depth = depth.saturating_sub(1);
                if in_item && name_buf == "item" {
                    in_item = false;
                    current_field = None;
                    match item.take().build(source_name) {
                        Ok(r) => releases.push(r),
                        Err(err) => {
                            tracing::warn!(error = %err, "skipping malformed nyaa item");
                        }
                    }
                    continue;
                }
                if !in_item && in_channel && name_buf == "channel" {
                    in_channel = false;
                    continue;
                }
                current_field = None;
                let _ = depth;
            }
            Ok(Event::Empty(_)) => {
                // <atom:link .../> and similar self-closing tags don't carry
                // data we need; just ignore.
            }
            Ok(Event::Text(t)) => {
                if let Some(field) = current_field.as_ref() {
                    let raw = t.decode().map_err(|e| anyhow!("decode text: {e}"))?;
                    let unescaped = unescape(&raw)
                        .map_err(|e| anyhow!("unescape: {e}"))?
                        .to_string();
                    item.apply(field, &unescaped);
                }
            }
            Ok(Event::CData(c)) => {
                if let Some(field) = current_field.as_ref() {
                    let s = std::str::from_utf8(c.as_ref())
                        .map_err(|e| anyhow!("non-utf8 cdata: {e}"))?
                        .to_string();
                    item.apply(field, &s);
                }
            }
            // quick_xml emits entity references (`&amp;`, `&#39;`, ...) as a
            // separate event instead of inlining them into the surrounding
            // Text. Without this arm they'd fall into the wildcard below and
            // get silently dropped — Nyaa post titles routinely encode `'`
            // as `&#39;`, so we'd be losing characters mid-word.
            Ok(Event::GeneralRef(e)) => {
                if let Some(field) = current_field.as_ref() {
                    let resolved: Option<String> = if e.is_char_ref() {
                        e.resolve_char_ref()
                            .map_err(|err| anyhow!("resolve char ref: {err}"))?
                            .map(|ch| ch.to_string())
                    } else {
                        let name = e.decode().map_err(|err| anyhow!("decode entity: {err}"))?;
                        resolve_predefined_entity(&name).map(|s| s.to_string())
                    };
                    if let Some(s) = resolved {
                        item.apply(field, &s);
                    } else {
                        tracing::debug!(
                            entity = ?e.decode().ok(),
                            "dropping unresolvable XML entity reference in nyaa feed"
                        );
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => {
                return Err(anyhow!(
                    "xml read error at pos {}: {e}",
                    reader.error_position()
                ));
            }
        }
    }

    Ok(releases)
}

fn local_name_owned(name: QName<'_>) -> String {
    let local = name.local_name();
    std::str::from_utf8(local.into_inner())
        .unwrap_or("")
        .to_string()
}

#[derive(Debug, Clone, Copy)]
enum Field {
    Title,
    Link,
    Guid,
    PubDate,
    Description,
    Category,
    Size,
    InfoHash,
}

impl Field {
    fn from_local(name: &str) -> Option<Self> {
        match name {
            "title" => Some(Field::Title),
            "link" => Some(Field::Link),
            "guid" => Some(Field::Guid),
            "pubDate" => Some(Field::PubDate),
            "description" => Some(Field::Description),
            // namespaced (`nyaa:*`) fields are normalized by local_name; the
            // raw `category` channel-level tag also lands here but we only
            // populate `item` builder while `in_item` is true.
            "category" => Some(Field::Category),
            "size" => Some(Field::Size),
            "infoHash" => Some(Field::InfoHash),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
struct ItemBuilder {
    title: String,
    /// Torrent download URL (RSS `<link>`).
    link: String,
    /// Permalink to the post (`<guid isPermaLink="true">`). Source of the
    /// numeric Nyaa post id.
    guid: String,
    pub_date: String,
    description: String,
    category: String,
    size: String,
    info_hash: String,
}

impl ItemBuilder {
    fn take(&mut self) -> ItemBuilder {
        std::mem::take(self)
    }

    fn apply(&mut self, field: &Field, value: &str) {
        let dest = match field {
            Field::Title => &mut self.title,
            Field::Link => &mut self.link,
            Field::Guid => &mut self.guid,
            Field::PubDate => &mut self.pub_date,
            Field::Description => &mut self.description,
            Field::Category => &mut self.category,
            Field::Size => &mut self.size,
            Field::InfoHash => &mut self.info_hash,
        };
        if dest.is_empty() {
            *dest = value.to_string();
        } else {
            // RSS allows text + CDATA siblings inside one element; concat
            // them so we don't drop text adjacent to CDATA description blobs.
            dest.push_str(value);
        }
    }

    fn build(self, source_name: &str) -> Result<DiscoveredRelease> {
        let title = self.title.trim().to_string();
        if title.is_empty() {
            return Err(anyhow!("item missing <title>"));
        }
        let page_link = if !self.guid.is_empty() {
            self.guid.trim().to_string()
        } else {
            return Err(anyhow!("item missing <guid>"));
        };
        let external_id = extract_post_id(&page_link)
            .with_context(|| format!("parsing post id from {page_link}"))?;
        let torrent_url = if self.link.is_empty() {
            None
        } else {
            Some(self.link.trim().to_string())
        };
        let info_hash = if self.info_hash.is_empty() {
            None
        } else {
            Some(self.info_hash.trim().to_ascii_lowercase())
        };
        let magnet = info_hash.as_ref().map(|h| build_magnet(h, &title));
        let size_bytes = parse_size(self.size.trim());
        let description_html = {
            let trimmed = self.description.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        };
        let external_links = description_html
            .as_deref()
            .map(extract_external_links)
            .unwrap_or_default();

        let posted_at = parse_rfc2822_date(self.pub_date.trim())
            .with_context(|| format!("parsing pubDate: {:?}", self.pub_date))?;

        Ok(DiscoveredRelease {
            source_kind: SOURCE_KIND.into(),
            source_name: source_name.into(),
            external_id,
            title,
            link: page_link,
            magnet,
            torrent_url,
            ddl_url: None,
            info_hash,
            size_bytes,
            files: Vec::new(),
            description_html,
            external_links,
            posted_at,
        })
    }
}

/// Extract `2111533` from `https://nyaa.si/view/2111533`. Tolerates the
/// trailing-slash and query-string variants.
pub(crate) fn extract_post_id(url: &str) -> Result<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let Some(idx) = trimmed.rfind('/') else {
        return Err(anyhow!("no '/' in {url}"));
    };
    let candidate = &trimmed[idx + 1..];
    let cut = candidate
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(candidate.len());
    let id = &candidate[..cut];
    if id.is_empty() {
        return Err(anyhow!("no numeric id in {url}"));
    }
    Ok(id.to_string())
}

fn build_magnet(info_hash: &str, title: &str) -> String {
    let dn = urlencode(title);
    format!("magnet:?xt=urn:btih:{info_hash}&dn={dn}")
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Parse Nyaa's RSS "11.6 MiB" / "1.6 GiB" size strings into bytes. Returns
/// `None` on unknown units rather than erroring — the field is informational.
pub(crate) fn parse_size(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (num_part, unit_part) = raw.rsplit_once(' ')?;
    let n: f64 = num_part.replace(',', "").parse().ok()?;
    let mult: f64 = match unit_part.trim().to_ascii_lowercase().as_str() {
        "b" => 1.0,
        "kib" => 1024.0,
        "kb" => 1_000.0,
        "mib" => 1024.0 * 1024.0,
        "mb" => 1_000_000.0,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        "gb" => 1_000_000_000.0,
        "tib" => 1024f64.powi(4),
        "tb" => 1_000_000_000_000.0,
        _ => return None,
    };
    Some((n * mult) as u64)
}

fn parse_rfc2822_date(raw: &str) -> Result<DateTime<Utc>> {
    let dt = chrono::DateTime::parse_from_rfc2822(raw)?;
    Ok(dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/nyaa_rss_sample.xml");

    #[test]
    fn extract_post_id_parses_view_url() {
        assert_eq!(
            extract_post_id("https://nyaa.si/view/2111533").unwrap(),
            "2111533"
        );
        assert_eq!(
            extract_post_id("https://nyaa.si/view/2111533/").unwrap(),
            "2111533"
        );
        assert_eq!(
            extract_post_id("https://nyaa.si/view/2111533?some=arg").unwrap(),
            "2111533"
        );
    }

    #[test]
    fn parse_size_handles_common_units() {
        assert_eq!(
            parse_size("11.6 MiB"),
            Some((11.6_f64 * 1024.0 * 1024.0) as u64)
        );
        assert_eq!(
            parse_size("1.6 GiB"),
            Some((1.6_f64 * 1024f64.powi(3)) as u64)
        );
        assert_eq!(
            parse_size("3.5 MiB"),
            Some((3.5_f64 * 1024.0 * 1024.0) as u64)
        );
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("nonsense"), None);
    }

    #[test]
    fn parse_feed_extracts_every_item_from_fixture() {
        let releases = parse_feed(FIXTURE, "trusted").unwrap();
        assert!(
            !releases.is_empty(),
            "fixture should contain at least one item"
        );

        let first = &releases[0];
        assert_eq!(first.source_kind, "nyaa");
        assert_eq!(first.source_name, "trusted");
        assert!(first.title.starts_with("ReZero"));
        assert_eq!(first.external_id, "2111533");
        assert_eq!(first.link, "https://nyaa.si/view/2111533");
        assert_eq!(
            first.torrent_url.as_deref(),
            Some("https://nyaa.si/download/2111533.torrent")
        );
        assert_eq!(
            first.info_hash.as_deref(),
            Some("3cce2a1b1dd491be89a5a2461250b1f7ee6700c7")
        );
        // Magnet is synthesized from the info hash + title.
        let magnet = first.magnet.as_deref().unwrap();
        assert!(magnet.starts_with("magnet:?xt=urn:btih:3cce2a1b1dd491be89a5a2461250b1f7ee6700c7"));
        assert_eq!(first.size_bytes, Some((11.6_f64 * 1024.0 * 1024.0) as u64));
        assert_eq!(first.posted_at.timestamp(), 1_779_147_296);
        // Description is preserved (CDATA) so the link extractor has data
        // to work with.
        assert!(
            first
                .description_html
                .as_deref()
                .unwrap()
                .contains("11.6 MiB")
        );
    }

    #[test]
    fn parse_feed_is_idempotent_on_repeated_runs() {
        let a = parse_feed(FIXTURE, "trusted").unwrap();
        let b = parse_feed(FIXTURE, "trusted").unwrap();
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.external_id, y.external_id);
            assert_eq!(x.title, y.title);
            assert_eq!(x.info_hash, y.info_hash);
        }
    }

    #[test]
    fn numeric_entity_in_title_is_resolved_inline() {
        // Nyaa encodes apostrophes as `&#39;` in `<title>` (the CDATA
        // description gets the same treatment). quick_xml emits entity
        // refs as `Event::GeneralRef`, separate from the surrounding text;
        // if the parser doesn't resolve them, the character is dropped
        // mid-word and the title silently loses punctuation.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:nyaa="https://nyaa.si/xmlns/nyaa">
<channel>
<title>Nyaa</title>
<item>
<title>The Skull Dragon&#39;s Precious Daughter v05-06 (2025-2026) (Digital) (Ushi)</title>
<link>https://nyaa.si/download/2104023.torrent</link>
<guid isPermaLink="true">https://nyaa.si/view/2104023</guid>
<pubDate>Wed, 29 Apr 2026 02:51:07 -0000</pubDate>
<nyaa:size>543.7 MiB</nyaa:size>
<nyaa:infoHash>5b7b9f287c30bfe097f0621cffcb7e3e2e8638b3</nyaa:infoHash>
</item>
</channel>
</rss>"#;
        let releases = parse_feed(xml, "test").unwrap();
        assert_eq!(
            releases[0].title,
            "The Skull Dragon's Precious Daughter v05-06 (2025-2026) (Digital) (Ushi)"
        );
    }

    #[test]
    fn predefined_named_entities_in_title_are_resolved() {
        // The five XML-predefined entities. `&amp;` is the realistic one
        // (titles like "Foo &amp; Bar"); the others round-trip safely.
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0" xmlns:nyaa="https://nyaa.si/xmlns/nyaa">
<channel>
<item>
<title>Foo &amp; Bar &lt;v1&gt; &quot;Special&quot; &apos;Edition&apos;</title>
<link>https://nyaa.si/download/1.torrent</link>
<guid isPermaLink="true">https://nyaa.si/view/1</guid>
<pubDate>Mon, 18 May 2026 23:34:56 -0000</pubDate>
<nyaa:infoHash>aaaa</nyaa:infoHash>
<nyaa:size>1.0 MiB</nyaa:size>
</item>
</channel>
</rss>"#;
        let releases = parse_feed(xml, "test").unwrap();
        assert_eq!(releases[0].title, "Foo & Bar <v1> \"Special\" 'Edition'");
    }

    #[test]
    fn malformed_items_are_skipped_not_fatal() {
        let xml = r#"<?xml version="1.0"?>
        <rss version="2.0" xmlns:nyaa="https://nyaa.si/xmlns/nyaa">
          <channel>
            <title>Test</title>
            <item>
              <title>good item</title>
              <link>https://nyaa.si/download/1.torrent</link>
              <guid isPermaLink="true">https://nyaa.si/view/1</guid>
              <pubDate>Mon, 18 May 2026 23:34:56 -0000</pubDate>
              <nyaa:infoHash>aaaa</nyaa:infoHash>
              <nyaa:size>1.0 MiB</nyaa:size>
            </item>
            <item>
              <title></title>
              <guid isPermaLink="true">https://nyaa.si/view/2</guid>
              <pubDate>Mon, 18 May 2026 23:34:56 -0000</pubDate>
            </item>
          </channel>
        </rss>"#;
        let releases = parse_feed(xml, "trusted").unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].external_id, "1");
    }
}
