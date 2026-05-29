//! Volume / chapter span detection from a release's file list (or title).
//!
//! Mirrors [`crate::format`]'s philosophy: purely lexical, no I/O, no
//! archive peeking. We scan filenames for volume and chapter markers
//! (`v01`, `vol. 3`, `c012`, `chapter 5`, ranges like `v01-03`) and reduce
//! every number found into a `(min, max)` [`Span`] per dimension.
//!
//! The series catalog uses the `max` of these spans to track the highest
//! volume / chapter that has actually surfaced in a release — distinct from
//! the provider's *published* total (`series.total_volumes` /
//! `total_chapters`). A user can then tell at a glance how much of a series
//! is realistically downloadable versus how long it ultimately runs.
//!
//! When the file list yields nothing (no files, or only cover art / info
//! text with no numbering) we fall back to parsing the release title, which
//! almost always carries the volume/chapter range for single-pack uploads.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// An inclusive numeric range `[start, end]` observed across a release.
/// `start == end` for a single-volume / single-chapter release. Numbers are
/// `f64` because chapter numbering routinely includes decimals (e.g. the
/// classic "chapter 10.5" omake), and the `series.highest_*` columns are
/// `REAL` to match.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}

impl Span {
    fn from_numbers(nums: &[f64]) -> Option<Span> {
        let mut iter = nums.iter().copied();
        let first = iter.next()?;
        let mut min = first;
        let mut max = first;
        for n in iter {
            if n < min {
                min = n;
            }
            if n > max {
                max = n;
            }
        }
        Some(Span {
            start: min,
            end: max,
        })
    }
}

/// Volume and chapter spans detected for one release. Either field is
/// `None` when no marker of that kind was found anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ReleaseSpans {
    pub volumes: Option<Span>,
    pub chapters: Option<Span>,
}

impl ReleaseSpans {
    fn is_empty(&self) -> bool {
        self.volumes.is_none() && self.chapters.is_none()
    }
}

/// Detect volume / chapter spans for a release. Scans `files` first; if that
/// produces nothing usable, falls back to the `title` (covers single-pack
/// uploads whose file list is just `cover.jpg` + one archive, or sources
/// that never expose a file list at all).
pub fn detect_spans(files: &[String], title: &str) -> ReleaseSpans {
    let from_files = scan(files.iter().map(String::as_str));
    if !from_files.is_empty() {
        return from_files;
    }
    scan(std::iter::once(title))
}

/// Collect every volume and chapter number across `texts` into one span per
/// dimension. Range markers (`v01-03`) contribute both endpoints.
fn scan<'a>(texts: impl Iterator<Item = &'a str>) -> ReleaseSpans {
    let mut vols: Vec<f64> = Vec::new();
    let mut chaps: Vec<f64> = Vec::new();
    for text in texts {
        push_matches(volume_re(), text, &mut vols);
        push_matches(chapter_re(), text, &mut chaps);
    }
    ReleaseSpans {
        volumes: Span::from_numbers(&vols),
        chapters: Span::from_numbers(&chaps),
    }
}

/// Push both capture groups (range start + optional range end) of every
/// match into `out`. A leading-zero token like `01` parses as `1.0`.
fn push_matches(re: &Regex, text: &str, out: &mut Vec<f64>) {
    for caps in re.captures_iter(text) {
        for idx in [1usize, 2usize] {
            if let Some(m) = caps.get(idx)
                && let Ok(n) = m.as_str().parse::<f64>()
            {
                out.push(n);
            }
        }
    }
}

/// `v01`, `vol 3`, `volume 12`, `v01-03`, `v1-v5`. The `v`/`vol` prefix is
/// required so bare numbers (years, resolution, group counts) never read as
/// volumes. Word-boundary anchored so it won't fire mid-word (`tv01`).
fn volume_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)\bv(?:ol(?:ume)?)?\.?\s*(\d+(?:\.\d+)?)(?:\s*[-–]\s*(?:v(?:ol(?:ume)?)?\.?\s*)?(\d+(?:\.\d+)?))?",
        )
        .unwrap()
    })
}

/// `c012`, `ch 5`, `chapter 10.5`, `#001`, `c001-050`. Alternation lists the
/// longer keywords first so `chapter`/`chap` win over the bare `ch`/`c`. The
/// keyword branch is `\b`-anchored so the bare `c` can't match inside words
/// like `Disc`; the `#` branch needs no anchor (it's already non-word). The
/// `#` issue/chapter convention is borrowed from Codex's filename parser.
fn chapter_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)(?:\b(?:chapter|chap|ch|c)|#)\.?\s*(\d+(?:\.\d+)?)(?:\s*[-–]\s*(?:(?:\b(?:chapter|chap|ch|c)|#)\.?\s*)?(\d+(?:\.\d+)?))?",
        )
        .unwrap()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|x| x.to_string()).collect()
    }

    fn span(start: f64, end: f64) -> Option<Span> {
        Some(Span { start, end })
    }

    #[test]
    fn single_volume_file() {
        let got = detect_spans(&s(&["Some Series v01.cbz"]), "ignored");
        assert_eq!(got.volumes, span(1.0, 1.0));
        assert_eq!(got.chapters, None);
    }

    #[test]
    fn multiple_volume_files_form_a_span() {
        let files = s(&["Series v01.cbz", "Series v02.cbz", "Series v03.cbz"]);
        let got = detect_spans(&files, "ignored");
        assert_eq!(got.volumes, span(1.0, 3.0));
    }

    #[test]
    fn volume_range_in_one_filename() {
        let got = detect_spans(&s(&["Series v01-05 (Digital).cbz"]), "ignored");
        assert_eq!(got.volumes, span(1.0, 5.0));
    }

    #[test]
    fn chapter_markers() {
        let got = detect_spans(&s(&["Series c001.cbz", "Series c012.cbz"]), "ignored");
        assert_eq!(got.chapters, span(1.0, 12.0));
        assert_eq!(got.volumes, None);
    }

    #[test]
    fn decimal_chapter() {
        let got = detect_spans(&s(&["Series chapter 10.5.cbz"]), "ignored");
        assert_eq!(got.chapters, span(10.5, 10.5));
    }

    #[test]
    fn mixed_volume_and_chapter() {
        let got = detect_spans(&s(&["Series v01-03 c001-050.cbz"]), "ignored");
        assert_eq!(got.volumes, span(1.0, 3.0));
        assert_eq!(got.chapters, span(1.0, 50.0));
    }

    #[test]
    fn falls_back_to_title_when_files_have_no_markers() {
        let files = s(&["cover.jpg", "info.txt"]);
        let got = detect_spans(&files, "Some Series v01-12 (2021)");
        assert_eq!(got.volumes, span(1.0, 12.0));
    }

    #[test]
    fn falls_back_to_title_when_no_files() {
        let got = detect_spans(&[], "Solo Leveling Vol. 3");
        assert_eq!(got.volumes, span(3.0, 3.0));
    }

    #[test]
    fn year_is_not_read_as_a_volume_or_chapter() {
        let got = detect_spans(&[], "Berserk (2016) Deluxe");
        assert_eq!(got.volumes, None);
        assert_eq!(got.chapters, None);
    }

    #[test]
    fn bare_c_does_not_match_inside_words() {
        // "Disc" must not surface chapter 1, and "Comic" / "CBZ" carry no
        // numbering — none of these should produce a span.
        let got = detect_spans(&s(&["Comic Disc Archive.cbz"]), "Comic Disc Archive");
        assert_eq!(got.chapters, None);
        assert_eq!(got.volumes, None);
    }

    #[test]
    fn nothing_parseable_yields_empty() {
        let got = detect_spans(&s(&["Artbook (Digital).cbz"]), "Artbook (Digital)");
        assert_eq!(got, ReleaseSpans::default());
    }

    #[test]
    fn hash_prefix_reads_as_a_chapter() {
        // Comic-style issue numbering, as Codex's filename parser handles.
        let got = detect_spans(&s(&["Batman #001.cbz", "Batman #042.cbz"]), "ignored");
        assert_eq!(got.chapters, span(1.0, 42.0));
        assert_eq!(got.volumes, None);
    }

    #[test]
    fn leading_zero_tokens_parse_as_their_value() {
        let got = detect_spans(&s(&["Series v007.cbz"]), "ignored");
        assert_eq!(got.volumes, span(7.0, 7.0));
    }
}
