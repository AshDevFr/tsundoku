//! Format-to-series-kind validation.
//!
//! A release that matches a `cbz` format but resolves to a `novel`-kind
//! series is almost certainly the wrong match (or an unusual edge case
//! that deserves a human eyeball). The rule list, read straight from
//! `[ingestion.format_type_rules]`, captures these constraints declaratively.
//!
//! A rule fires when any of its `formats` is present on the release; once
//! fired, the matched series's kind must be in `required_kinds` (case
//! insensitive). The empty rule list short-circuits to "always valid".

use td_config::FormatTypeRule;
use td_metadata::SeriesKind;

/// Outcome of running every rule against `(formats, series_kind)`.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationOutcome {
    /// All rules either didn't fire or passed.
    Ok,
    /// At least one rule fired and failed. Carries the offending format and
    /// the rule's required kinds for use in the review queue's `reason`
    /// column.
    Mismatch {
        offending_formats: Vec<String>,
        required_kinds: Vec<String>,
        series_kind: Option<String>,
    },
}

impl ValidationOutcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, ValidationOutcome::Ok)
    }
}

/// Apply every rule in `rules` against `formats` and `series_kind`. Returns
/// the first mismatch (the rule order in config is operator-meaningful) or
/// [`ValidationOutcome::Ok`] if everything passes.
///
/// `series_kind` is `None` when the provider didn't classify the series
/// (older catalog rows, an upstream miss, a provider that doesn't track
/// kinds). With no kind to compare against the rule can't actually
/// fire — return Ok so unrelated metadata-quality issues don't pollute
/// the review queue. The pipeline's candidate filter takes the same
/// permissive stance.
pub fn validate(
    rules: &[FormatTypeRule],
    formats: &[String],
    series_kind: Option<&SeriesKind>,
) -> ValidationOutcome {
    if rules.is_empty() {
        return ValidationOutcome::Ok;
    }
    let Some(kind) = series_kind else {
        return ValidationOutcome::Ok;
    };
    let kind_label = kind_to_label(kind);

    for rule in rules {
        let triggers: Vec<&String> = rule
            .formats
            .iter()
            .filter(|wanted| formats.iter().any(|have| have.eq_ignore_ascii_case(wanted)))
            .collect();
        if triggers.is_empty() {
            continue;
        }
        let kind_ok = rule
            .required_kinds
            .iter()
            .any(|r| r.eq_ignore_ascii_case(&kind_label));
        if !kind_ok {
            return ValidationOutcome::Mismatch {
                offending_formats: triggers.into_iter().cloned().collect(),
                required_kinds: rule.required_kinds.clone(),
                series_kind: Some(kind_label.clone()),
            };
        }
    }
    ValidationOutcome::Ok
}

pub(crate) fn kind_to_label(k: &SeriesKind) -> String {
    match k {
        SeriesKind::Manga => "manga".into(),
        SeriesKind::Manhwa => "manhwa".into(),
        SeriesKind::Manhua => "manhua".into(),
        SeriesKind::Novel => "novel".into(),
        SeriesKind::OneShot => "one_shot".into(),
        SeriesKind::Oel => "oel".into(),
        SeriesKind::Other(s) => s.clone(),
    }
}

/// Buckets of allowed kinds derived from the format set. Each bucket
/// corresponds to one rule that fires on at least one of the release's
/// formats. The pipeline uses this to (a) filter candidates whose kind is
/// not in the union of any bucket and (b) detect when high-score
/// candidates split across multiple buckets — the "mixed format release"
/// signal that routes the row to review.
///
/// Empty groups means "no rule fires for these formats" → no constraint.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FormatKindGroups {
    pub groups: Vec<Vec<String>>,
}

impl FormatKindGroups {
    /// `true` when no rule fires for these formats. Caller treats this as
    /// "all candidates pass".
    pub fn is_unconstrained(&self) -> bool {
        self.groups.is_empty()
    }

    /// Union of every required-kinds list across firing rules, lowercased.
    pub fn allowed_kinds(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for g in &self.groups {
            for k in g {
                let lower = k.to_ascii_lowercase();
                if !out.contains(&lower) {
                    out.push(lower);
                }
            }
        }
        out
    }

    /// `true` when `kind` (case-insensitive) appears in at least one
    /// bucket. An unknown kind is treated as compatible — the pipeline
    /// only filters candidates whose kind it knows.
    pub fn is_kind_compatible(&self, kind: Option<&SeriesKind>) -> bool {
        if self.is_unconstrained() {
            return true;
        }
        let Some(k) = kind else {
            return true;
        };
        let label = kind_to_label(k);
        self.allowed_kinds()
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&label))
    }

    /// Indexes of buckets that contain `kind` (case-insensitive). Used to
    /// detect the multi-bucket case: a CBZ+EPUB release fires both rules,
    /// and a manga candidate belongs only to the cbz bucket while a novel
    /// candidate belongs only to the epub bucket.
    pub fn bucket_indexes_for(&self, kind: Option<&SeriesKind>) -> Vec<usize> {
        let Some(k) = kind else {
            return Vec::new();
        };
        let label = kind_to_label(k);
        self.groups
            .iter()
            .enumerate()
            .filter_map(|(idx, group)| {
                group
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(&label))
                    .then_some(idx)
            })
            .collect()
    }
}

/// Walk every rule and collect one bucket of required-kinds per rule
/// whose `formats` list intersects `formats`. Order matches `rules`; an
/// empty result means no rule fires.
pub fn rule_groups(rules: &[FormatTypeRule], formats: &[String]) -> FormatKindGroups {
    let groups = rules
        .iter()
        .filter_map(|rule| {
            let fires = rule
                .formats
                .iter()
                .any(|wanted| formats.iter().any(|have| have.eq_ignore_ascii_case(wanted)));
            fires.then(|| rule.required_kinds.clone())
        })
        .collect();
    FormatKindGroups { groups }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules_default() -> Vec<FormatTypeRule> {
        vec![
            FormatTypeRule {
                formats: vec!["cbz".into(), "cbr".into(), "zip".into()],
                required_kinds: vec!["manga".into(), "manhwa".into(), "manhua".into()],
            },
            FormatTypeRule {
                formats: vec!["epub".into(), "azw3".into()],
                required_kinds: vec!["novel".into()],
            },
        ]
    }

    #[test]
    fn empty_rule_list_short_circuits_ok() {
        let r = validate(&[], &["epub".into()], Some(&SeriesKind::Manga));
        assert!(r.is_ok());
    }

    #[test]
    fn manga_with_cbz_passes() {
        let r = validate(&rules_default(), &["cbz".into()], Some(&SeriesKind::Manga));
        assert!(r.is_ok());
    }

    #[test]
    fn novel_with_cbz_is_a_mismatch() {
        let r = validate(&rules_default(), &["cbz".into()], Some(&SeriesKind::Novel));
        match r {
            ValidationOutcome::Mismatch {
                offending_formats,
                required_kinds,
                series_kind,
            } => {
                assert_eq!(offending_formats, vec!["cbz".to_string()]);
                assert_eq!(required_kinds, vec!["manga", "manhwa", "manhua"]);
                assert_eq!(series_kind.as_deref(), Some("novel"));
            }
            ValidationOutcome::Ok => panic!("expected mismatch"),
        }
    }

    #[test]
    fn manga_with_epub_is_a_mismatch_on_second_rule() {
        let r = validate(&rules_default(), &["epub".into()], Some(&SeriesKind::Manga));
        assert!(!r.is_ok());
    }

    #[test]
    fn unknown_series_kind_is_treated_as_ok() {
        // No kind to compare against = no signal; don't pollute the review
        // queue with metadata-quality complaints unrelated to format-vs-kind.
        let r = validate(&rules_default(), &["cbz".into()], None);
        assert!(r.is_ok());
    }

    #[test]
    fn format_match_is_case_insensitive() {
        let r = validate(&rules_default(), &["CBZ".into()], Some(&SeriesKind::Manga));
        assert!(r.is_ok());
    }

    #[test]
    fn other_kind_label_is_compared_as_is() {
        let rules = vec![FormatTypeRule {
            formats: vec!["cbz".into()],
            required_kinds: vec!["doujinshi".into()],
        }];
        let r = validate(
            &rules,
            &["cbz".into()],
            Some(&SeriesKind::Other("doujinshi".into())),
        );
        assert!(r.is_ok());
    }

    #[test]
    fn release_with_no_rule_relevant_format_passes() {
        // Release has mkv only; no rule fires.
        let r = validate(&rules_default(), &["mkv".into()], Some(&SeriesKind::Manga));
        assert!(r.is_ok());
    }

    #[test]
    fn rule_groups_empty_when_no_rule_fires() {
        let g = rule_groups(&rules_default(), &["mkv".into()]);
        assert!(g.is_unconstrained());
        assert!(g.allowed_kinds().is_empty());
        // Without rules firing, every candidate is compatible — including
        // the awkward "kind is unknown" case.
        assert!(g.is_kind_compatible(Some(&SeriesKind::Novel)));
        assert!(g.is_kind_compatible(None));
    }

    #[test]
    fn rule_groups_single_bucket_for_cbz_only() {
        let g = rule_groups(&rules_default(), &["cbz".into()]);
        assert_eq!(g.groups.len(), 1);
        assert!(g.is_kind_compatible(Some(&SeriesKind::Manga)));
        assert!(!g.is_kind_compatible(Some(&SeriesKind::Novel)));
        // Unknown kind is permissive: we only filter when we know.
        assert!(g.is_kind_compatible(None));
        assert_eq!(
            g.bucket_indexes_for(Some(&SeriesKind::Manga)),
            vec![0_usize]
        );
        assert!(g.bucket_indexes_for(Some(&SeriesKind::Novel)).is_empty());
    }

    #[test]
    fn rule_groups_two_buckets_for_cbz_plus_epub() {
        // Mixed-format release fires both rules. A manga candidate sits
        // only in bucket 0; a novel candidate only in bucket 1.
        let g = rule_groups(&rules_default(), &["cbz".into(), "epub".into()]);
        assert_eq!(g.groups.len(), 2);
        assert_eq!(
            g.bucket_indexes_for(Some(&SeriesKind::Manga)),
            vec![0_usize]
        );
        assert_eq!(
            g.bucket_indexes_for(Some(&SeriesKind::Novel)),
            vec![1_usize]
        );
        // Both manga and novel are in the allowed union for a mixed-format
        // release — the multi-bucket signal is the second-order check.
        assert!(g.is_kind_compatible(Some(&SeriesKind::Manga)));
        assert!(g.is_kind_compatible(Some(&SeriesKind::Novel)));
    }

    #[test]
    fn rule_groups_uppercase_format_input_still_matches() {
        let g = rule_groups(&rules_default(), &["CBZ".into()]);
        assert_eq!(g.groups.len(), 1);
        assert!(g.is_kind_compatible(Some(&SeriesKind::Manga)));
    }
}
