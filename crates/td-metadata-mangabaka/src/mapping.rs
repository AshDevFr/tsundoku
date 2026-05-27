//! Map MangaBaka payloads onto canonical [`td_metadata`] types.
//!
//! Translation rules:
//! - Provider source keys → canonical provider ids:
//!   - `manga_updates` → `mangaupdates`
//!   - `my_anime_list` → `mal`
//!   - others pass through unchanged (`anilist`, `mangadex`, `kitsu`,
//!     `anime_planet`, `anime_news_network`, `shikimori`, ...).
//! - `type` strings → [`SeriesKind`] (unknown values land in `Other(_)`).
//! - `status` strings → [`SeriesStatus`] (MangaBaka uses `releasing`, which
//!   we normalize to `Ongoing`; `unknown` and missing both land in `Unknown`).
//! - `content_hash` is a SHA-1 of the canonical JSON serialization of the
//!   raw payload. Resolver uses this to skip writes on no-op refreshes.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};
use td_metadata::{ForeignId, SearchHit, SeriesKind, SeriesMetadata, SeriesStatus};

use crate::client::{MbScaledImage, MbSeries};

/// Map a MangaBaka series payload to the canonical [`SeriesMetadata`].
pub fn series_to_canonical(series: MbSeries) -> SeriesMetadata {
    let external_id = series.id.to_string();
    let external_url = Some(format!("https://mangabaka.dev/series/{external_id}"));
    let alternate_titles = collect_alternates(&series);
    let kind = series.kind.as_deref().map(parse_kind);
    let status = series.status.as_deref().map(parse_status);
    let cover_url = pick_cover(series.cover.as_ref());
    let foreign_ids = source_to_foreign_ids(series.source.as_ref());

    // `raw` is the input payload re-serialized so the hash is deterministic
    // across runs regardless of map ordering in MangaBaka's response.
    let raw = serde_json::to_value(&series).expect("MbSeries always serializes");
    let content_hash = hash_value(&raw);

    SeriesMetadata {
        external_id,
        canonical_title: series.title,
        alternate_titles,
        kind,
        status,
        year: series.year,
        cover_url,
        external_url,
        description: series.description.filter(|s| !s.is_empty()),
        genres: series.genres.unwrap_or_default(),
        tags: series.tags.unwrap_or_default(),
        foreign_ids,
        raw,
        content_hash,
    }
}

/// Map a MangaBaka search-result row to a canonical [`SearchHit`].
pub fn series_to_search_hit(series: &MbSeries) -> SearchHit {
    SearchHit {
        external_id: series.id.to_string(),
        title: series.title.clone(),
        year: series.year,
        cover_url: pick_cover(series.cover.as_ref()),
        score: None, // MangaBaka doesn't return a relevance score.
    }
}

fn parse_kind(s: &str) -> SeriesKind {
    match s {
        "manga" => SeriesKind::Manga,
        "manhwa" => SeriesKind::Manhwa,
        "manhua" => SeriesKind::Manhua,
        "novel" => SeriesKind::Novel,
        "one_shot" | "oneshot" => SeriesKind::OneShot,
        "oel" => SeriesKind::Oel,
        other => SeriesKind::Other(other.to_string()),
    }
}

fn parse_status(s: &str) -> SeriesStatus {
    match s {
        // MangaBaka uses "releasing" for ongoing series.
        "releasing" | "ongoing" => SeriesStatus::Ongoing,
        "completed" => SeriesStatus::Completed,
        "hiatus" => SeriesStatus::Hiatus,
        "cancelled" => SeriesStatus::Cancelled,
        "upcoming" => SeriesStatus::Upcoming,
        _ => SeriesStatus::Unknown,
    }
}

/// Best-effort cover URL: prefer the 350-wide variant (x2 retina), fall
/// back through smaller scales, then to the raw upload.
fn pick_cover(cover: Option<&crate::client::MbCover>) -> Option<String> {
    let cover = cover?;
    pick_scaled(cover.x350.as_ref())
        .or_else(|| pick_scaled(cover.x250.as_ref()))
        .or_else(|| pick_scaled(cover.x150.as_ref()))
        .or_else(|| cover.raw.as_ref().and_then(|r| r.url.clone()))
}

fn pick_scaled(scaled: Option<&MbScaledImage>) -> Option<String> {
    let s = scaled?;
    s.x2.clone()
        .or_else(|| s.x1.clone())
        .or_else(|| s.x3.clone())
}

/// Flatten `secondary_titles` (keyed by language) and the explicit
/// `native_title` / `romanized_title` fields into a deduplicated alternate
/// list, with the canonical title excluded.
fn collect_alternates(series: &MbSeries) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: Option<&str>| {
        if let Some(value) = s
            && !value.is_empty()
            && value != series.title
            && !out.iter().any(|existing| existing == value)
        {
            out.push(value.to_string());
        }
    };
    push(series.native_title.as_deref());
    push(series.romanized_title.as_deref());

    if let Some(obj) = series.secondary_titles.as_ref().and_then(|v| v.as_object()) {
        for entries in obj.values() {
            let Some(arr) = entries.as_array() else {
                continue;
            };
            for entry in arr {
                if let Some(t) = entry.get("title").and_then(|t| t.as_str()) {
                    push(Some(t));
                }
            }
        }
    }
    out
}

/// Map MangaBaka's `source` payload onto canonical [`ForeignId`] rows.
fn source_to_foreign_ids(source: Option<&serde_json::Value>) -> Vec<ForeignId> {
    let Some(obj) = source.and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    // Iterate deterministically so `content_hash` is stable: BTreeMap by key.
    let sorted: BTreeMap<&String, &serde_json::Value> = obj.iter().collect();
    let mut out = Vec::with_capacity(sorted.len());
    for (mb_key, value) in sorted {
        let provider = match mb_key.as_str() {
            "manga_updates" => "mangaupdates",
            "my_anime_list" => "mal",
            other => other,
        };
        let id_val = value.get("id");
        let Some(id) = id_val
            .and_then(|v| {
                v.as_str()
                    .map(str::to_string)
                    .or_else(|| v.as_i64().map(|n| n.to_string()))
                    .or_else(|| v.as_u64().map(|n| n.to_string()))
            })
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        out.push(ForeignId {
            provider: provider.to_string(),
            id,
            url: None,
        });
    }
    out
}

pub(crate) fn hash_value(value: &serde_json::Value) -> String {
    // serde_json::to_vec emits keys in insertion order; for stability we
    // pass through canonical_json by sorting via a BTreeMap when objects
    // appear. Simpler: serialize to a string then SHA-256 it; ordering is
    // already deterministic for the values we construct here (BTreeMap in
    // `source_to_foreign_ids`, struct order for `MbSeries`).
    let bytes = serde_json::to_vec(value).expect("Value always serializes");
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_payload() -> serde_json::Value {
        serde_json::json!({
            "id": 1,
            "title": "Chainsaw Man",
            "native_title": "チェンソーマン",
            "romanized_title": "Chensō Man",
            "year": 2018,
            "type": "manga",
            "status": "releasing",
            "secondary_titles": {
                "en": [{"type": "alternative", "title": "Chainsaw-Man"}],
                "ja": [{"type": "native", "title": "チェンソーマン"}]
            },
            "cover": {
                "raw": {"url": "https://mangabaka/raw.jpg"},
                "x350": {"x1": "https://mangabaka/350.jpg", "x2": "https://mangabaka/350@2x.jpg"}
            },
            "source": {
                "anilist": {"id": 105778},
                "my_anime_list": {"id": 116778},
                "manga_updates": {"id": 174610},
                "mangadex": {"id": "a77742b1-befd-49a4-bff5-1ad4375089ee"}
            },
            "genres": ["action", "horror"]
        })
    }

    #[test]
    fn maps_canonical_fields() {
        let s: MbSeries = serde_json::from_value(sample_payload()).unwrap();
        let m = series_to_canonical(s);
        assert_eq!(m.external_id, "1");
        assert_eq!(m.canonical_title, "Chainsaw Man");
        assert_eq!(m.year, Some(2018));
        assert_eq!(m.kind, Some(SeriesKind::Manga));
        assert_eq!(m.status, Some(SeriesStatus::Ongoing));
        assert_eq!(m.cover_url.as_deref(), Some("https://mangabaka/350@2x.jpg"));
        assert_eq!(
            m.external_url.as_deref(),
            Some("https://mangabaka.dev/series/1")
        );
        assert!(m.genres.contains(&"action".to_string()));
    }

    #[test]
    fn collects_alternate_titles_dedup_and_skip_canonical() {
        let s: MbSeries = serde_json::from_value(sample_payload()).unwrap();
        let m = series_to_canonical(s);
        // The canonical title ("Chainsaw Man") must not appear in alternates.
        assert!(!m.alternate_titles.contains(&"Chainsaw Man".to_string()));
        assert!(m.alternate_titles.contains(&"Chainsaw-Man".to_string()));
        assert!(m.alternate_titles.contains(&"チェンソーマン".to_string()));
        // Dedup: チェンソーマン appears both in native_title and secondary_titles.ja.
        let count = m
            .alternate_titles
            .iter()
            .filter(|t| t.as_str() == "チェンソーマン")
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn maps_source_keys_to_canonical_provider_ids() {
        let s: MbSeries = serde_json::from_value(sample_payload()).unwrap();
        let m = series_to_canonical(s);

        let mut providers: Vec<&str> = m.foreign_ids.iter().map(|f| f.provider.as_str()).collect();
        providers.sort();
        assert_eq!(
            providers,
            vec!["anilist", "mal", "mangadex", "mangaupdates"]
        );

        // Numeric ids stringified.
        let anilist = m
            .foreign_ids
            .iter()
            .find(|f| f.provider == "anilist")
            .unwrap();
        assert_eq!(anilist.id, "105778");
        // String ids preserved.
        let mangadex = m
            .foreign_ids
            .iter()
            .find(|f| f.provider == "mangadex")
            .unwrap();
        assert_eq!(mangadex.id, "a77742b1-befd-49a4-bff5-1ad4375089ee");
    }

    #[test]
    fn falls_back_to_raw_cover_when_no_scaled_variant_present() {
        let mut payload = sample_payload();
        payload["cover"]["x350"] = serde_json::Value::Null;
        let s: MbSeries = serde_json::from_value(payload).unwrap();
        let m = series_to_canonical(s);
        assert_eq!(m.cover_url.as_deref(), Some("https://mangabaka/raw.jpg"));
    }

    #[test]
    fn unknown_kind_lands_in_other_variant() {
        let mut payload = sample_payload();
        payload["type"] = serde_json::Value::String("doujinshi".into());
        let s: MbSeries = serde_json::from_value(payload).unwrap();
        let m = series_to_canonical(s);
        assert_eq!(m.kind, Some(SeriesKind::Other("doujinshi".into())));
    }

    #[test]
    fn content_hash_is_stable_across_repeated_mapping() {
        let s1: MbSeries = serde_json::from_value(sample_payload()).unwrap();
        let s2: MbSeries = serde_json::from_value(sample_payload()).unwrap();
        let h1 = series_to_canonical(s1).content_hash;
        let h2 = series_to_canonical(s2).content_hash;
        assert_eq!(h1, h2);
        assert!(h1.starts_with("sha256:"));
    }

    #[test]
    fn search_hit_uses_canonical_title_and_cover() {
        let s: MbSeries = serde_json::from_value(sample_payload()).unwrap();
        let hit = series_to_search_hit(&s);
        assert_eq!(hit.external_id, "1");
        assert_eq!(hit.title, "Chainsaw Man");
        assert_eq!(hit.year, Some(2018));
        assert!(hit.cover_url.is_some());
    }
}
