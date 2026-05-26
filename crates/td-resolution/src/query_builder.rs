//! Turn a raw release title into one or more search queries for the
//! active metadata provider.
//!
//! Nyaa post titles bury the actual series name under volume markers,
//! scanlator brackets, year ranges, edition labels, and uploader tags.
//! Feeding that string straight into MangaBaka's FTS5 index AND-requires
//! every token to appear in the row, so any parenthesized noise causes a
//! complete miss (this is the bug that put Solo Leveling in the review
//! queue).
//!
//! The cleaner applies a fixed-order rule pipeline (safest first), then
//! splits on multi-title separators (`|`, ` / `) so romaji and English
//! titles for the same series each become their own search query. The
//! pipeline searches each query, dedupes hits by `external_id`, and
//! Dice-rescores against whichever query-half the candidate matches
//! best.
//!
//! Rule order is part of the contract — each rule has a tiny job and
//! gets out of the way. The order is documented inline so future readers
//! can audit it without reading the regex.

use std::sync::OnceLock;

use anyhow::{Result, anyhow};
use regex::Regex;

/// The built-in list of trailing keywords often stamped on by uploaders
/// or scanlator groups. Operators can extend this via
/// `ingestion.cleanup.extra_format_keywords` but cannot shrink it.
pub const BUILT_IN_FORMAT_WORDS: &[&str] = &[
    "Digital",
    "Raw",
    "Color",
    "Colored",
    "Omnibus",
    "Premium",
    "Complete",
    "Decensored",
    "Uncensored",
    "Webtoon",
    "WN",
    "LN",
];

/// Result of cleaning one raw release title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanedQuery {
    /// Non-empty list of search strings to try. Multi-title separators
    /// expand into multiple entries, ordered longest-first.
    pub queries: Vec<String>,
    /// Stable names of every rule that fired while cleaning. Used by the
    /// review UI to show what surgery happened (badge chips per rule).
    pub rules_applied: Vec<String>,
}

impl CleanedQuery {
    /// The query the resolver should treat as "primary" — used to fill
    /// the search modal's title input and to render the inline pill on
    /// review cards. Always present (the cleaner never returns an empty
    /// `queries` vector).
    pub fn primary(&self) -> &str {
        &self.queries[0]
    }
}

/// Stateless reference cleaner. Cheap to construct — the only expensive
/// piece is the format-keyword regex, built once at startup from the
/// built-in list plus operator extras.
///
/// Hold one of these for the lifetime of the process and pass into the
/// resolver via [`crate::Resolver::with_query_builder`].
pub struct QueryBuilder {
    format_re: Regex,
}

impl QueryBuilder {
    /// Build a cleaner whose format-keyword rule covers the built-in
    /// list **plus** `extras`. Each extra is `regex::escape`'d before
    /// joining, so they're matched as literal whole-word tokens, not
    /// patterns.
    ///
    /// Returns an error if any extra contains regex metacharacters or is
    /// empty after trimming. Validation is intentionally strict so
    /// operators can't accidentally smuggle a wildcard into the
    /// alternation.
    pub fn new(extras: &[String]) -> Result<Self> {
        for s in extras {
            validate_keyword(s)?;
        }
        let escaped: Vec<String> = BUILT_IN_FORMAT_WORDS
            .iter()
            .map(|s| (*s).to_string())
            .chain(extras.iter().cloned())
            .map(|w| regex::escape(&w))
            .collect();
        let pattern = format!(r"(?i)\b(?:{})\b", escaped.join("|"));
        let format_re =
            Regex::new(&pattern).map_err(|e| anyhow!("building format keyword regex: {e}"))?;
        Ok(Self { format_re })
    }

    /// Convenience: build the cleaner with only the built-in keyword set.
    /// Used by tests and by callers that haven't loaded config yet.
    pub fn with_defaults() -> Self {
        Self::new(&[]).expect("built-in keyword set is always valid")
    }

    /// Run the cleaner. Always returns at least one non-empty query;
    /// the worst case (degenerate input that strips down to nothing)
    /// falls back to the raw title with rule `empty_fallback`.
    pub fn clean(&self, raw: &str) -> CleanedQuery {
        let mut rules: Vec<String> = Vec::new();
        let mut s = raw.to_string();

        // 1. Strip `[scanlator]` brackets.
        s = apply_rule(&mut rules, "strip_brackets", &s, brackets_re());

        // 2. Strip `(...)` groups (years, edition labels, uploader tags).
        s = apply_rule(&mut rules, "strip_parens", &s, parens_re());

        // 3. Strip `{curly}` groups (rare but seen).
        s = apply_rule(&mut rules, "strip_braces", &s, braces_re());

        // 4. Strip compact volume / chapter markers (v01, v01-06, c100,
        // ch.10, vol 2, with optional .5 fractional and v2 revision).
        s = apply_rule(&mut rules, "strip_vol_compact", &s, vol_compact_re());

        // 5. Strip written-out Volume / Chapter forms.
        s = apply_rule(&mut rules, "strip_vol_word", &s, vol_word_re());

        // 6. Strip Part / Parts.
        s = apply_rule(&mut rules, "strip_parts", &s, parts_re());

        // 7. Strip bare year tokens.
        s = apply_rule(&mut rules, "strip_year", &s, year_re());

        // 8. Strip torrent / archive file extensions.
        s = apply_rule(&mut rules, "strip_ext", &s, ext_re());

        // 9. Strip configured format keywords (built-in + extras).
        s = apply_rule(&mut rules, "strip_format", &s, &self.format_re);

        // 10. Multi-title separators: `|` or ` / ` (with surrounding
        // spaces — bare `/` is a path separator, not a title divider).
        // Each half becomes its own query so the resolver can search
        // both romaji and English forms.
        let queries: Vec<String> = if s.contains('|') || s.contains(" / ") {
            let halves: Vec<String> = split_alternates_re()
                .split(&s)
                .map(normalize_whitespace_and_trim)
                .filter(|h| !h.is_empty())
                .collect();
            if halves.len() > 1 {
                rules.push("split_alternates".into());
                let mut sorted = halves;
                sorted.sort_by_key(|h| std::cmp::Reverse(h.len()));
                sorted
            } else {
                halves
            }
        } else {
            let single = normalize_whitespace_and_trim(&s);
            if single.is_empty() {
                Vec::new()
            } else {
                vec![single]
            }
        };

        // 11. Empty fallback.
        let queries = if queries.is_empty() {
            rules.push("empty_fallback".into());
            vec![raw.to_string()]
        } else {
            queries
        };

        CleanedQuery {
            queries,
            rules_applied: rules,
        }
    }
}

/// Apply one substitution rule, appending `name` to `rules` only when
/// the regex actually changed the string.
fn apply_rule(rules: &mut Vec<String>, name: &str, s: &str, re: &Regex) -> String {
    let next = re.replace_all(s, " ");
    if next != s {
        rules.push(name.into());
    }
    next.into_owned()
}

/// Collapse runs of whitespace and trim leading/trailing punctuation
/// that becomes noise after the regexes chew through.
fn normalize_whitespace_and_trim(s: &str) -> String {
    let collapsed = collapse_ws_re().replace_all(s, " ");
    collapsed
        .trim_matches(|c: char| c.is_whitespace() || matches!(c, '-' | ',' | '_' | '/' | '|' | '+'))
        .to_string()
}

/// Reject keywords containing regex metacharacters or that are blank
/// after trimming. Errors are operator-facing and name the offending
/// entry so the config diff is obvious.
fn validate_keyword(s: &str) -> Result<()> {
    if s.trim().is_empty() {
        return Err(anyhow!(
            "extra_format_keywords entry must not be empty or whitespace-only"
        ));
    }
    const BAD: &[char] = &[
        '\\', '.', '*', '+', '?', '[', ']', '(', ')', '{', '}', '|', '^', '$',
    ];
    if let Some(bad) = s.chars().find(|c| BAD.contains(c)) {
        return Err(anyhow!(
            "extra_format_keywords entry {s:?} contains regex metacharacter {bad:?}; \
             use plain words only"
        ));
    }
    Ok(())
}

// ----- compiled-once helper regexes -----

fn brackets_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\[[^\]]*\]").unwrap())
}

fn parens_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\([^)]*\)").unwrap())
}

fn braces_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\{[^}]*\}").unwrap())
}

fn vol_compact_re() -> &'static Regex {
    // v01 / v01-06 / c100 / ch.10 / vol 2 / 14.5 / 98v2, plus chained
    // continuations like `v01-05 + 041-065` (volume range plus follow-on
    // chapter range, common in long-running series). Anchored on a
    // leading marker so we never touch a bare number that's part of a
    // real title.
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:v|vol\.?|c|ch\.?)\s*\d{1,4}(?:\.\d{1,2})?(?:v\d+)?(?:\s*[-+,&]\s*\d{1,4}(?:\.\d{1,2})?)*\b",
        )
        .unwrap()
    })
}

fn vol_word_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:volumes?|chapters?)\s+\d{1,4}(?:\.\d{1,2})?(?:v\d+)?(?:\s*[-–]\s*\d{1,4}(?:\.\d{1,2})?)?\b",
        )
        .unwrap()
    })
}

fn parts_re() -> &'static Regex {
    // Only the plural form with an explicit range ("Parts 1-198", "Parts
    // 7 - 10") looks like release-metadata. Singular "Part 7" is often
    // canonical-title (JoJo Part 7 = Steel Ball Run), so we leave it
    // alone.
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)\bparts\s+\d{1,4}(?:\s*[-–]\s*\d{1,4})?\b").unwrap())
}

fn year_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\b(?:19|20)\d{2}\b").unwrap())
}

fn ext_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)\.(?:zip|cbz|cbr|epub|pdf|rar|7z)\b").unwrap())
}

fn split_alternates_re() -> &'static Regex {
    // Surrounding spaces required around `/` so we don't break tokens
    // like "TV/Movie" that aren't title separators.
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\s*\|\s*|\s+/\s+").unwrap())
}

fn collapse_ws_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\s+").unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(raw: &str) -> CleanedQuery {
        QueryBuilder::with_defaults().clean(raw)
    }

    // ---------- screenshot cases ----------

    #[test]
    fn solo_leveling_collapses_to_clean_title() {
        let out = clean("Solo Leveling (2021-2026) (Digital) (1r0n)");
        assert_eq!(out.queries, vec!["Solo Leveling".to_string()]);
        // The parens-strip already eats `(Digital)`, so `strip_format`
        // has nothing to do here — only `strip_parens` should fire.
        assert!(out.rules_applied.contains(&"strip_parens".into()));
        assert!(!out.rules_applied.contains(&"strip_format".into()));
    }

    #[test]
    fn singular_part_is_preserved_as_canonical_title() {
        // JoJo's Bizarre Adventure Part 7 (= Steel Ball Run) is a real
        // series title with a singular "Part N". Make sure the parts
        // rule doesn't eat it.
        let out = clean("JoJos Bizarre Adventure Part 7");
        assert_eq!(
            out.queries,
            vec!["JoJos Bizarre Adventure Part 7".to_string()]
        );
        assert!(!out.rules_applied.contains(&"strip_parts".into()));
    }

    #[test]
    fn plural_parts_with_range_is_meta_and_gets_stripped() {
        let out = clean("Some Series Parts 1-198");
        assert_eq!(out.queries, vec!["Some Series".to_string()]);
        assert!(out.rules_applied.contains(&"strip_parts".into()));
    }

    #[test]
    fn jojo_steel_ball_run_keeps_hyphenated_subtitle() {
        let out = clean(
            "JoJos Bizarre Adventure Part 7 - Steel Ball Run v01-06 (2025-2026) (Omnibus Edition) (Digital) (1r0n)",
        );
        assert_eq!(
            out.queries,
            vec!["JoJos Bizarre Adventure Part 7 - Steel Ball Run".to_string()]
        );
        for required in &["strip_parens", "strip_vol_compact"] {
            assert!(
                out.rules_applied.contains(&(*required).to_string()),
                "expected rule {required} to fire; got {:?}",
                out.rules_applied
            );
        }
    }

    #[test]
    fn after_god_strips_volume_and_parens() {
        let out = clean("After God v01-09 (2024-2026) (Digital) (1r0n)");
        assert_eq!(out.queries, vec!["After God".to_string()]);
    }

    #[test]
    fn dogsred_strips_compound_volume_range() {
        let out = clean("Dogsred v01-05 + 041-065 (2024-2026) (Digital) (1r0n)");
        assert_eq!(out.queries, vec!["Dogsred".to_string()]);
    }

    #[test]
    fn yubisaki_picks_longest_half_and_keeps_alternates() {
        let out = clean(
            "Yubisaki Kara Honki no Netsujou | Fire in His Fingertips - A Flirty Fireman Ravishes Me with My Smoldering Gaze v01-11 (2020-2026) (Digital) (1r0n)",
        );
        // Should produce two queries: the long English subtitle and the
        // shorter Japanese title; longest-first.
        assert!(out.queries.len() >= 2, "got {:?}", out.queries);
        assert!(out.rules_applied.contains(&"split_alternates".into()));
        assert!(out.queries[0].contains("Fire in His Fingertips"));
        assert!(out.queries.iter().any(|q| q.contains("Yubisaki")));
    }

    // ---------- live-feed cases ----------

    #[test]
    fn rezero_emits_all_three_alternates_longest_first() {
        let out = clean(
            "ReZero - Starting Life in Another World - Volume 01 [MTBBooks] | Re:Zero Kara Hajimeru Isekai Seikatsu | Re Zero",
        );
        assert_eq!(out.queries.len(), 3);
        assert!(out.rules_applied.contains(&"split_alternates".into()));
        // Longest-first ordering.
        for w in out.queries.windows(2) {
            assert!(
                w[0].len() >= w[1].len(),
                "not sorted longest-first: {:?}",
                out.queries
            );
        }
        assert!(out.queries.iter().any(|q| q.contains("Re:Zero Kara")));
        assert!(out.queries.iter().any(|q| q == "Re Zero"));
    }

    #[test]
    fn fractional_chapter_handled() {
        let out = clean("[Doki] Isyuzoku Joshi ni OO Suru Hanashi - Chapter 14.5 [850F2B60].zip");
        assert_eq!(
            out.queries,
            vec!["Isyuzoku Joshi ni OO Suru Hanashi".to_string()]
        );
    }

    #[test]
    fn revision_suffix_handled() {
        let out = clean("[Doki] New Game! - Chapter 98v2 [9C8582AA].zip");
        assert_eq!(out.queries, vec!["New Game!".to_string()]);
    }

    #[test]
    fn doki_chapter_collapses_to_series_name() {
        let out = clean("[Doki] Hitoribocchi no OO Seikatsu - Chapter 90 [C0045813].zip");
        assert_eq!(out.queries, vec!["Hitoribocchi no OO Seikatsu".to_string()]);
        assert!(out.rules_applied.contains(&"strip_brackets".into()));
        assert!(out.rules_applied.contains(&"strip_vol_word".into()));
        assert!(out.rules_applied.contains(&"strip_ext".into()));
    }

    #[test]
    fn already_clean_title_passes_through_untouched() {
        let out = clean("Golden Warrior Iczer-One");
        assert_eq!(out.queries, vec!["Golden Warrior Iczer-One".to_string()]);
        assert!(out.rules_applied.is_empty());
    }

    #[test]
    fn story_about_classmate_splits_on_slash_separator() {
        let out = clean(
            "Story About Buying My Classmate Once a Week / Shuu ni Ichido Kurasumeito wo Kau Hanashi (WN), Volumes 1 - 7, Parts 1-198",
        );
        assert!(out.rules_applied.contains(&"split_alternates".into()));
        assert!(
            out.queries
                .iter()
                .any(|q| q.contains("Classmate Once a Week")),
            "got {:?}",
            out.queries
        );
        assert!(
            out.queries.iter().any(|q| q.contains("Shuu ni Ichido")),
            "got {:?}",
            out.queries
        );
    }

    #[test]
    fn primary_returns_first_query() {
        let out = clean("Solo Leveling (2021-2026)");
        assert_eq!(out.primary(), "Solo Leveling");
    }

    // ---------- edge cases ----------

    #[test]
    fn degenerate_input_falls_back_to_raw() {
        let out = clean("(Digital) (Raw)");
        assert!(out.rules_applied.contains(&"empty_fallback".into()));
        assert_eq!(out.queries, vec!["(Digital) (Raw)".to_string()]);
    }

    #[test]
    fn bare_slash_inside_token_is_not_a_separator() {
        // No spaces around `/`, so it isn't a title separator.
        let out = clean("AC/DC");
        assert_eq!(out.queries, vec!["AC/DC".to_string()]);
        assert!(!out.rules_applied.contains(&"split_alternates".into()));
    }

    // ---------- config-extension cases ----------

    #[test]
    fn extra_format_keywords_extend_the_strip_list() {
        let qb = QueryBuilder::new(&["Remastered".to_string()]).unwrap();
        let out = qb.clean("Some Series Remastered (2024)");
        assert_eq!(out.queries, vec!["Some Series".to_string()]);
        assert!(out.rules_applied.contains(&"strip_format".into()));
    }

    #[test]
    fn extra_format_keyword_matches_case_insensitively() {
        let qb = QueryBuilder::new(&["DigitalUncen".to_string()]).unwrap();
        let out = qb.clean("Some Series DIGITALUNCEN v01");
        assert_eq!(out.queries, vec!["Some Series".to_string()]);
    }

    #[test]
    fn extra_format_keyword_respects_word_boundaries() {
        // "Color" is in the built-in list; "Colorful" must not be eaten.
        let out = clean("Colorful Series v01");
        assert_eq!(out.queries, vec!["Colorful Series".to_string()]);
    }

    #[test]
    fn validate_rejects_regex_metacharacters() {
        for bad in [".*", "\\d+", "[a-z]", "foo|bar", "foo?", "^x", "x$"] {
            let err = QueryBuilder::new(&[bad.to_string()]).err();
            assert!(err.is_some(), "expected {bad:?} to be rejected");
            let msg = format!("{}", err.unwrap());
            assert!(msg.contains("regex metacharacter"), "msg was {msg:?}");
        }
    }

    #[test]
    fn validate_rejects_empty_or_whitespace_entries() {
        assert!(QueryBuilder::new(&["".to_string()]).is_err());
        assert!(QueryBuilder::new(&["   ".to_string()]).is_err());
    }

    #[test]
    fn validate_accepts_multi_word_plain_phrases() {
        // Multi-word plain phrases are allowed — regex::escape handles
        // the space, and `\b` boundaries still match the phrase's edges.
        let qb = QueryBuilder::new(&["Bonus Chapter".to_string()]).unwrap();
        let out = qb.clean("Some Series Bonus Chapter v01");
        assert_eq!(out.queries, vec!["Some Series".to_string()]);
    }
}
