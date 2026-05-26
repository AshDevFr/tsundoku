//! Fuzzy-title scoring built on the Sørensen–Dice coefficient over character
//! bigrams. Borrowed from the `release-nyaa` plugin's matcher: cheap, easy to
//! reason about, and tolerant of the common variations between Nyaa post
//! titles and canonical series titles (punctuation, romanization spelling,
//! "vol. 3" suffixes, etc.).
//!
//! The released title is normalized once per candidate set; each candidate
//! is scored independently and the best score wins. We score against both
//! the canonical title and every alternate, so a release that uses the
//! romaji while the canonical row is in English still resolves correctly.

use std::collections::HashSet;

/// Dice coefficient between two strings over their character-bigram sets.
/// Both inputs are lowercased and stripped of non-alphanumeric characters
/// before bigram extraction, so "Chainsaw Man" and "chainsawman!" score
/// identically.
pub fn dice(a: &str, b: &str) -> f32 {
    let a_grams = bigrams(a);
    let b_grams = bigrams(b);
    if a_grams.is_empty() || b_grams.is_empty() {
        // Two empty (or single-character-after-normalization) strings are
        // either both empty (treat as match) or one is empty (no match).
        return if a_grams.is_empty() && b_grams.is_empty() {
            1.0
        } else {
            0.0
        };
    }
    let intersection = a_grams.intersection(&b_grams).count();
    (2.0 * intersection as f32) / (a_grams.len() + b_grams.len()) as f32
}

/// Best Dice score for `query` against every title in `candidates`. Empty
/// candidate set returns `0.0`. Used to re-rank provider search hits where
/// the canonical title and alternates should both be considered.
pub fn best_dice<I, S>(query: &str, candidates: I) -> f32
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    candidates
        .into_iter()
        .map(|c| dice(query, c.as_ref()))
        .fold(0.0_f32, f32::max)
}

fn bigrams(s: &str) -> HashSet<[char; 2]> {
    let normalized: Vec<char> = s
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else {
                None
            }
        })
        .collect();
    let mut out = HashSet::with_capacity(normalized.len().saturating_sub(1));
    for window in normalized.windows(2) {
        out.insert([window[0], window[1]]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dice_of_identical_strings_is_one() {
        assert!((dice("Chainsaw Man", "Chainsaw Man") - 1.0).abs() < 1e-6);
    }

    #[test]
    fn dice_is_case_and_punctuation_insensitive() {
        let a = dice("Chainsaw Man", "chainsawman!");
        assert!((a - 1.0).abs() < 1e-6, "got {a}");
    }

    #[test]
    fn dice_handles_partial_overlap() {
        // "Chainsaw Man v3" vs "Chainsaw Man" — high but not 1.0.
        let s = dice("Chainsaw Man v3", "Chainsaw Man");
        assert!(s > 0.7 && s < 1.0, "expected high-but-not-perfect, got {s}");
    }

    #[test]
    fn dice_handles_no_overlap() {
        assert!(dice("Naruto", "Bleach") < 0.2);
    }

    #[test]
    fn dice_handles_empty_strings() {
        assert!((dice("", "") - 1.0).abs() < 1e-6);
        assert!((dice("anything", "") - 0.0).abs() < 1e-6);
        assert!((dice("", "anything") - 0.0).abs() < 1e-6);
    }

    #[test]
    fn best_dice_picks_max_across_alternates() {
        let alternates = ["Naruto", "ナルト", "Naruto: Shippuden"];
        // Query that's closer to Shippuden should win even though "Naruto"
        // also matches.
        let score = best_dice("Naruto Shippuden", alternates);
        assert!(score > 0.8, "expected high best score, got {score}");
    }

    #[test]
    fn best_dice_returns_zero_for_empty_candidate_set() {
        let candidates: [&str; 0] = [];
        assert!((best_dice("Naruto", candidates) - 0.0).abs() < 1e-6);
    }
}
