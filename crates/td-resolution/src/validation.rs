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
/// `series_kind` is `None` when the provider didn't classify the series.
/// Conservative behavior: if a rule requires a specific kind and we don't
/// know, the rule fails. Operators can opt out by leaving `format_type_rules`
/// empty.
pub fn validate(
    rules: &[FormatTypeRule],
    formats: &[String],
    series_kind: Option<&SeriesKind>,
) -> ValidationOutcome {
    if rules.is_empty() {
        return ValidationOutcome::Ok;
    }
    let kind_label = series_kind.map(kind_to_label);
    let kind_label_ref = kind_label.as_deref();

    for rule in rules {
        let triggers: Vec<&String> = rule
            .formats
            .iter()
            .filter(|wanted| formats.iter().any(|have| have.eq_ignore_ascii_case(wanted)))
            .collect();
        if triggers.is_empty() {
            continue;
        }
        let kind_ok = match kind_label_ref {
            Some(k) => rule
                .required_kinds
                .iter()
                .any(|r| r.eq_ignore_ascii_case(k)),
            None => false,
        };
        if !kind_ok {
            return ValidationOutcome::Mismatch {
                offending_formats: triggers.into_iter().cloned().collect(),
                required_kinds: rule.required_kinds.clone(),
                series_kind: kind_label.clone(),
            };
        }
    }
    ValidationOutcome::Ok
}

fn kind_to_label(k: &SeriesKind) -> String {
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
    fn unknown_series_kind_fails_when_rules_apply() {
        let r = validate(&rules_default(), &["cbz".into()], None);
        match r {
            ValidationOutcome::Mismatch { series_kind, .. } => assert!(series_kind.is_none()),
            ValidationOutcome::Ok => panic!("expected mismatch with unknown kind"),
        }
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
}
