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
//! Both the file list *and* the release title are scanned and the results
//! unioned. The title is not a mere fallback: a pack often names its files
//! with bare numbers (`One Piece 1134 (2024).cbz`) while only the title
//! carries the explicit range and the volume/chapter split
//! (`v001-111 + 1134-1176`). Scanning only the files would miss the
//! chapters; scanning only the title would miss a pack whose title is
//! generic. Taking the max across both is the most complete signal.

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

/// Detect volume / chapter spans for a release by scanning every file name
/// *and* the release title, taking the widest span found across all of them
/// per dimension. See the module docs for why the title is unioned in rather
/// than used only as a fallback.
pub fn detect_spans(files: &[String], title: &str) -> ReleaseSpans {
    scan(
        files
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(title)),
    )
}

/// Collect every volume and chapter number across `texts` into one span per
/// dimension. Range markers (`v01-03`) contribute both endpoints.
fn scan<'a>(texts: impl Iterator<Item = &'a str>) -> ReleaseSpans {
    let mut vols: Vec<f64> = Vec::new();
    let mut chaps: Vec<f64> = Vec::new();
    for text in texts {
        // Strip bracketed / parenthesized metadata first. Those groups hold
        // release group, year, format, language, and CRC hashes — none of
        // which are content numbers, and several of which actively mislead:
        // `(2003-2026)` is a year range, and a CRC like `[C0045813]` would
        // otherwise read as "chapter 45813" via the bare `c` prefix. The
        // real volume/chapter markers always live in the bare stem.
        let stem = strip_metadata_groups(text);
        push_matches(volume_re(), &stem, &mut vols);
        // The spelled-out keyword form (`Volumes 1 - 7`) is an unambiguous
        // human-readable range even with a spaced hyphen, so it's matched
        // separately. The short `v` form deliberately does not allow a spaced
        // bare endpoint (see `volume_re`), because in per-file naming the ` - `
        // in `v18 - 1935-A Deep Marble` is a title separator, not a range.
        push_matches(volume_keyword_range_re(), &stem, &mut vols);
        push_matches(chapter_re(), &stem, &mut chaps);
        // Long-runner convention: `v001-111 + 1134-1176` means "volumes
        // 1-111, plus loose chapters 1134-1176 not yet collected into a
        // volume". The trailing range carries no `c` prefix, so a bare
        // number range introduced by `+` is treated as chapters. Gated on
        // the literal `+` so title numbers (`Mob Psycho 100`) and bare
        // years stay untouched.
        push_matches(plus_chapter_re(), &stem, &mut chaps);
    }
    ReleaseSpans {
        volumes: Span::from_numbers(&vols),
        chapters: Span::from_numbers(&chaps),
    }
}

/// Replace every `(...)` and `[...]` group with a space so metadata inside
/// them can't be mistaken for content numbering. Non-nested (Nyaa titles
/// don't nest), which is all the real data needs.
fn strip_metadata_groups(text: &str) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| Regex::new(r"\([^)]*\)|\[[^\]]*\]").unwrap());
    re.replace_all(text, " ").into_owned()
}

/// Push every numeric capture group of every match into `out` (the range
/// start plus whichever range-end alternative fired). Non-numeric or
/// non-participating groups are skipped. A leading-zero token like `01`
/// parses as `1.0`.
fn push_matches(re: &Regex, text: &str, out: &mut Vec<f64>) {
    for caps in re.captures_iter(text) {
        for idx in 1..caps.len() {
            if let Some(m) = caps.get(idx)
                && let Ok(n) = m.as_str().parse::<f64>()
            {
                out.push(n);
            }
        }
    }
}

/// `v01`, `vol 3`, `volume 12`, `v01-03`, `v1-v5`, `v18 - v22`. The
/// `v`/`vol`/`volume(s)` prefix is required so bare numbers (years,
/// resolution, group counts) never read as volumes. Word-boundary anchored
/// so it won't fire mid-word (`tv01`).
///
/// A range endpoint is accepted two ways: a **tight** hyphen (`v18-22`,
/// `v01-v05`) or a **spaced** hyphen whose endpoint carries its own `v`/`vol`
/// prefix (`v18 - v22`). A spaced hyphen followed by a *bare* number is NOT a
/// range: in real file names `v18 - 1935-A Deep Marble` the ` - ` separates
/// the volume marker from a subtitle that happens to start with a year, and
/// `v20 - 1931 Winter` is the same shape. Treating those as ranges produced
/// nonsense spans like "v18-1935". The spelled-out `Volumes 1 - 7` form, which
/// legitimately uses a spaced bare endpoint, is matched by
/// [`volume_keyword_range_re`] instead.
fn volume_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)\bv(?:ol(?:ume)?s?)?\.?\s*(\d+(?:\.\d+)?)(?:[-–](?:v(?:ol(?:ume)?s?)?\.?)?(\d+(?:\.\d+)?)|\s*[-–]\s*v(?:ol(?:ume)?s?)?\.?\s*(\d+(?:\.\d+)?))?",
        )
        .unwrap()
    })
}

/// The spelled-out spaced volume range `Volumes 1 - 7` / `vol. 3 - 9`. Kept
/// separate from [`volume_re`] because it requires the literal `vol` keyword
/// (and whitespace before the number), so it can never fire on the short
/// `v18 - <subtitle>` per-file form that the spaced-bare endpoint would
/// otherwise mis-read as a range.
fn volume_keyword_range_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)\bvol(?:ume)?s?\.?\s+(\d+(?:\.\d+)?)\s*[-–]\s*(?:v(?:ol(?:ume)?s?)?\.?\s*)?(\d+(?:\.\d+)?)",
        )
        .unwrap()
    })
}

/// `c012`, `ch 5`, `chapter 10.5`, `chapters 1-7`, `#001`, `c001-050`.
/// Alternation lists the longer keywords first so `chapter`/`chap` win over
/// the bare `ch`/`c`. The keyword branch is `\b`-anchored so the bare `c`
/// can't match inside words like `Disc`; the `#` branch needs no anchor
/// (it's already non-word). The `#` issue/chapter convention is borrowed
/// from Codex's filename parser.
///
/// Range endpoints follow the same rule as [`volume_re`]: a tight hyphen
/// (`c001-050`) or a spaced hyphen with a re-stated prefix. A spaced bare
/// endpoint (`ch 5 - 2019 Special`) is a title separator, not a range.
fn chapter_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)(?:\b(?:chapters?|chaps?|ch|c)|#)\.?\s*(\d+(?:\.\d+)?)(?:[-–](?:(?:chapters?|chaps?|ch|c|#)\.?)?(\d+(?:\.\d+)?)|\s*[-–]\s*(?:\b(?:chapters?|chaps?|ch|c)|#)\.?\s*(\d+(?:\.\d+)?))?",
        )
        .unwrap()
    })
}

/// A bare number range introduced by `+`, e.g. the `+ 1134-1176` in
/// `One Piece v001-111 + 1134-1176`. Treated as chapters (see [`scan`]).
fn plus_chapter_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\+\s*(\d+(?:\.\d+)?)(?:\s*[-–]\s*(\d+(?:\.\d+)?))?").unwrap())
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
    fn crc_hash_in_brackets_is_not_read_as_a_chapter() {
        // Regression: the bare `c` prefix used to match `C0045813` inside a
        // CRC hash as "chapter 45813", swamping the real "Chapter 90".
        let got = detect_spans(
            &[],
            "[Doki] Hitoribocchi no OO Seikatsu - Chapter 90 [C0045813].zip",
        );
        assert_eq!(got.chapters, span(90.0, 90.0));
        assert_eq!(got.volumes, None);
    }

    #[test]
    fn year_range_in_parens_is_not_read_as_content() {
        // Only the bare `v001-111` and the `+` chapter range count; the
        // parenthesized `(2003-2026)` year range and `(1r0n)` group tag are
        // stripped before parsing.
        let got = detect_spans(
            &[],
            "One Piece v001-111 + 1134-1176 (2003-2026) (Digital) (1r0n)",
        );
        assert_eq!(got.volumes, span(1.0, 111.0));
        assert_eq!(got.chapters, span(1134.0, 1176.0));
    }

    #[test]
    fn real_long_runner_pack_unions_volume_files_with_title_chapter_range() {
        // The actual One Piece pack: per-volume files (v001..v111), per-issue
        // files named with BARE numbers ("One Piece 1134 (2024)..."), and a
        // title that carries the volume/chapter split via `+`. Files alone
        // give only volumes (the issue files are bare → ignored); the title's
        // `+ 1134-1176` supplies the chapters. The union must yield both.
        let files = s(&[
            "One Piece v001 (2003) (Digital) (1r0n) (f).cbz",
            "One Piece v111 (2026) (Digital) (1r0n).cbz",
            "One Piece 1134 (2024) (Digital) (1r0n).cbz",
            "One Piece 1176 (2026) (Digital) (1r0n).cbz",
        ]);
        let got = detect_spans(
            &files,
            "One Piece v001-111 + 1134-1176 (2003-2026) (Digital) (1r0n)",
        );
        assert_eq!(got.volumes, span(1.0, 111.0));
        assert_eq!(got.chapters, span(1134.0, 1176.0));
    }

    #[test]
    fn bare_chapter_range_after_plus_is_chapters() {
        let got = detect_spans(&[], "Some Series v01-10 + 95-130");
        assert_eq!(got.volumes, span(1.0, 10.0));
        assert_eq!(got.chapters, span(95.0, 130.0));
    }

    #[test]
    fn title_number_is_not_mistaken_for_a_chapter() {
        // The `100` in the title must not become a chapter; only the `v`
        // prefix counts, and there is no `+` range.
        let got = detect_spans(&[], "Mob Psycho 100 v01-16 (Digital)");
        assert_eq!(got.volumes, span(1.0, 16.0));
        assert_eq!(got.chapters, None);
    }

    #[test]
    fn plural_volume_keyword() {
        let got = detect_spans(&[], "Some Series, Volumes 1 - 7, Parts 1-198");
        assert_eq!(got.volumes, span(1.0, 7.0));
    }

    #[test]
    fn bare_chapter_prefix_survives_metadata_stripping() {
        let got = detect_spans(
            &[],
            "Summertime Rendering 2026 - c001-002 (OneShot) (web) [MANGA Plus].zip",
        );
        assert_eq!(got.chapters, span(1.0, 2.0));
        assert_eq!(got.volumes, None);
    }

    #[test]
    fn hash_prefix_reads_as_a_chapter() {
        // Comic-style issue numbering, as Codex's filename parser handles.
        let got = detect_spans(&s(&["Batman #001.cbz", "Batman #042.cbz"]), "ignored");
        assert_eq!(got.chapters, span(1.0, 42.0));
        assert_eq!(got.volumes, None);
    }

    #[test]
    fn spaced_bare_endpoint_after_volume_is_a_subtitle_not_a_range() {
        // Regression: the real Baccano! pack. Each file is `vNN - <Subtitle>`
        // where the subtitle starts with a year-letter token (`1935-A`) or a
        // bare year (`1931 Winter`). The ` - ` is a title separator, so each
        // file contributes only its own volume; the title's `v18-22` (tight)
        // supplies the range. The endpoint must never read as `1935`/`1931`.
        let files = s(&[
            "Baccano! v18 - 1935-A Deep Marble [Yen Press] [Stick].epub",
            "Baccano! v19 - 1935-B Dr. Feelgreed [Yen Press] [Stick].epub",
            "Baccano! v20 - 1931 Winter - The Time of the Oasis [Yen Press] [Stick].epub",
            "Baccano! v21 - 1935-C The Grateful Bet [Yen Press] [Stick].epub",
            "Baccano! v22 - 1935-D Luckstreet Boys [Yen Press] [Stick].epub",
        ]);
        let got = detect_spans(&files, "Baccano! v18-22 [Yen Press] [Stick]");
        assert_eq!(got.volumes, span(18.0, 22.0));
        assert_eq!(got.chapters, None);
    }

    #[test]
    fn spaced_range_with_restated_prefix_is_a_range() {
        // `v18 - v22` (spaced, but the endpoint re-states the `v`) is an
        // unambiguous range and must still parse as one.
        let got = detect_spans(&[], "Baccano! v18 - v22");
        assert_eq!(got.volumes, span(18.0, 22.0));
    }

    #[test]
    fn spaced_bare_endpoint_after_chapter_is_a_subtitle_not_a_range() {
        let got = detect_spans(&s(&["Some Series ch 5 - 2019 Special.cbz"]), "ignored");
        assert_eq!(got.chapters, span(5.0, 5.0));
        assert_eq!(got.volumes, None);
    }

    #[test]
    fn leading_zero_tokens_parse_as_their_value() {
        let got = detect_spans(&s(&["Series v007.cbz"]), "ignored");
        assert_eq!(got.volumes, span(7.0, 7.0));
    }
}
