//! Shared per-run resolution-outcome tally for poll + backfill.
//!
//! Both jobs run the same resolver pipeline against every persisted release
//! and want to surface the same five-way breakdown on the metrics card. The
//! tally lives here so the two job loops use the exact same bucketing rules
//! (otherwise polls and backfills would drift apart silently when the
//! resolver grows a new path).

use td_resolution::pipeline::{ResolutionOutcome, ResolutionPath, ResolutionStatus};

#[derive(Default, Debug, Clone, Copy)]
pub struct OutcomeBreakdown {
    pub known_id: i32,
    pub foreign_id: i32,
    pub fuzzy: i32,
    pub review: i32,
    pub failed: i32,
}

impl OutcomeBreakdown {
    pub fn record(&mut self, outcome: &ResolutionOutcome) {
        match (outcome.path, outcome.status) {
            (Some(ResolutionPath::KnownExternalId), ResolutionStatus::Resolved) => {
                self.known_id += 1
            }
            (Some(ResolutionPath::ForeignIdLookup), ResolutionStatus::Resolved) => {
                self.foreign_id += 1
            }
            (Some(ResolutionPath::FuzzyTitle), ResolutionStatus::Resolved) => self.fuzzy += 1,
            (_, ResolutionStatus::ReviewPending) | (_, ResolutionStatus::Ambiguous) => {
                self.review += 1
            }
            _ => self.failed += 1,
        }
    }
}
