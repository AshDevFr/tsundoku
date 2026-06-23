//! Codex presence overlay types and status logic.
//!
//! `CodexInfo` is the admin-only object embedded in series list/detail
//! responses. Its `status` is derived purely from the parsed maxima
//! (`highest_volume`/`highest_chapter` on the tsundoku side vs Codex's
//! `local_max_*`); `volumes_owned` is carried through as an approximate,
//! display-only count and never feeds the status.
//!
//! "Not on Codex" is represented structurally by the absence of `CodexInfo`
//! (the `codex` field is `Option` with `skip_serializing_if`), not by a status
//! variant — so there is no `Missing` here.
//!
//! A series whose operator set `ignore_completion` short-circuits to `Ignored`
//! regardless of the maxima: the comparison is meaningless for series read in
//! omnibus (source single-volume numbering is permanently ahead of owned
//! omnibus numbering), so the "Behind" signal is muted on purpose.

use serde::Serialize;
use td_db::repos::codex_link_repo;
use utoipa::ToSchema;

/// Presence status for a series the operator owns on Codex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CodexStatus {
    /// Caught up: every tracked dimension's Codex max is >= the highest
    /// discovered (or nothing has been discovered to compare against).
    Complete,
    /// Newer volumes/chapters have surfaced on a source than Codex owns.
    Behind,
    /// Owned, but every discovered axis is on a different volume/chapter
    /// numbering than Codex owns (e.g. only chapter releases surfaced for a
    /// volume-only library entry), so there is no shared axis to compare and
    /// currency can't be judged. If *any* axis is comparable this never fires.
    Present,
    /// Owned, but the operator opted out of completion tracking
    /// (`series.ignore_completion`). The volume/chapter comparison is
    /// suppressed; the series reads as owned with tracking off.
    Ignored,
}

/// How a link was established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CodexLinkKind {
    Auto,
    Manual,
}

/// Admin-only presence object embedded on a series. Absent (the `codex` field
/// is `None`) when the series is not on Codex or the caller is not an admin.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexInfo {
    pub status: CodexStatus,
    pub series_uuid: String,
    /// Deep link to the series page in Codex's web UI.
    pub deep_link: String,
    /// Highest owned volume on Codex (comparison basis).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_max_volume: Option<f64>,
    /// Highest owned chapter on Codex (comparison basis).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_max_chapter: Option<f64>,
    /// Approximate count of owned volumes (display-only; never compared).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volumes_owned: Option<i64>,
    pub link_kind: CodexLinkKind,
    /// When this link was last written by a sweep.
    pub synced_at: i64,
}

/// Derive the presence status from the parsed maxima. An axis is comparable
/// only when we track it *and* Codex reports a max for it. `Behind` (a
/// comparable axis is below us) wins over `Complete` (any comparable axis is
/// caught up) over `Present` (we tracked an axis but it's on different
/// numbering than Codex owns, so nothing is comparable). A series with nothing
/// discovered to compare against is `Complete` (we own it, nothing newer is
/// known) — as is one where a single axis confirms currency even though a
/// sibling axis is uncomparable.
///
/// `ignore_completion` short-circuits to [`CodexStatus::Ignored`] before any
/// comparison, so the per-row badge and the server-side `codexStatus` filter
/// (both routed through here) can never disagree about an ignored series.
pub fn compute_status(
    ignore_completion: bool,
    highest_volume: Option<f64>,
    highest_chapter: Option<f64>,
    codex_max_volume: Option<f64>,
    codex_max_chapter: Option<f64>,
) -> CodexStatus {
    if ignore_completion {
        return CodexStatus::Ignored;
    }
    let vol_tracked = highest_volume.is_some();
    let chap_tracked = highest_chapter.is_some();

    // An axis is "comparable" only when we track it AND Codex reports a max for
    // it. Codex's max is per owned-file: a series owned as volume files reports
    // a volume max but no chapter max, so a discovered chapter release leaves
    // the chapter axis uncomparable even though Codex owns the series.
    let vol_comparable = vol_tracked && codex_max_volume.is_some();
    let chap_comparable = chap_tracked && codex_max_chapter.is_some();

    let vol_behind = vol_comparable && codex_max_volume.unwrap() < highest_volume.unwrap();
    let chap_behind = chap_comparable && codex_max_chapter.unwrap() < highest_chapter.unwrap();

    // Tracked on our side but Codex has no max for that axis (it owns the
    // series on the *other* axis).
    let vol_uncomparable = vol_tracked && codex_max_volume.is_none();
    let chap_uncomparable = chap_tracked && codex_max_chapter.is_none();

    if vol_behind || chap_behind {
        // A comparable axis that's behind is the actionable signal.
        CodexStatus::Behind
    } else if vol_comparable || chap_comparable {
        // At least one axis compares cleanly and isn't behind, which positively
        // confirms currency. An uncomparable sibling axis (e.g. a chapter
        // release against volume-only ownership) can never produce a `Behind`
        // signal anyway, so it must not drag a confirmed series to "unverified".
        CodexStatus::Complete
    } else if vol_uncomparable || chap_uncomparable {
        // We discovered something, but only on an axis Codex doesn't report for
        // this series, so there is no shared axis to compare against at all.
        CodexStatus::Present
    } else {
        CodexStatus::Complete
    }
}

/// Build a `CodexInfo` from a link row plus the series' discovered maxima.
/// `base_url` is the Codex web base (already trimmed); the deep link is
/// `<base_url>/series/{uuid}` (or a relative `/series/{uuid}` if unset).
pub fn build_codex_info(
    link: &codex_link_repo::Model,
    ignore_completion: bool,
    highest_volume: Option<f64>,
    highest_chapter: Option<f64>,
    base_url: Option<&str>,
) -> CodexInfo {
    let status = compute_status(
        ignore_completion,
        highest_volume,
        highest_chapter,
        link.local_max_volume,
        link.local_max_chapter,
    );
    let link_kind = if link.link_kind == codex_link_repo::KIND_MANUAL {
        CodexLinkKind::Manual
    } else {
        CodexLinkKind::Auto
    };
    let deep_link = match base_url {
        Some(base) if !base.is_empty() => format!("{base}/series/{}", link.codex_series_uuid),
        _ => format!("/series/{}", link.codex_series_uuid),
    };
    CodexInfo {
        status,
        series_uuid: link.codex_series_uuid.clone(),
        deep_link,
        local_max_volume: link.local_max_volume,
        local_max_chapter: link.local_max_chapter,
        volumes_owned: link.volumes_owned,
        link_kind,
        synced_at: link.synced_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_when_nothing_discovered_to_compare() {
        // No tracked dimension -> owned, nothing newer known.
        assert_eq!(
            compute_status(false, None, None, Some(5.0), Some(10.0)),
            CodexStatus::Complete
        );
    }

    #[test]
    fn complete_when_codex_caught_up_on_tracked_dims() {
        assert_eq!(
            compute_status(false, Some(10.0), None, Some(10.0), None),
            CodexStatus::Complete
        );
        assert_eq!(
            compute_status(false, Some(10.0), Some(100.0), Some(12.0), Some(100.0)),
            CodexStatus::Complete
        );
    }

    #[test]
    fn behind_when_any_tracked_dim_is_below() {
        // Volume behind.
        assert_eq!(
            compute_status(false, Some(12.0), None, Some(8.0), None),
            CodexStatus::Behind
        );
        // Chapter behind even though volume is caught up.
        assert_eq!(
            compute_status(false, Some(10.0), Some(200.0), Some(10.0), Some(150.0)),
            CodexStatus::Behind
        );
    }

    #[test]
    fn present_only_when_no_axis_is_comparable() {
        // We discovered volume 10 but Codex reports a chapter max, not a
        // volume one (owned on a different axis than we surfaced). No shared
        // axis -> we genuinely can't judge currency.
        assert_eq!(
            compute_status(false, Some(10.0), None, None, Some(300.0)),
            CodexStatus::Present
        );
        // Surfaced as volumes, but Codex reports neither max: still nothing to
        // compare against.
        assert_eq!(
            compute_status(false, Some(10.0), None, None, None),
            CodexStatus::Present
        );
    }

    #[test]
    fn complete_when_one_axis_confirms_despite_uncomparable_sibling() {
        // The Carefree-Journey case: owned as volume files (Codex reports a
        // volume max, no chapter max), but a chapter-numbered release was
        // discovered. The volume axis compares cleanly and is caught up, so
        // the uncomparable chapter axis must NOT force "unverified".
        assert_eq!(
            compute_status(false, Some(4.0), Some(22.0), Some(4.0), None),
            CodexStatus::Complete
        );
        // Symmetric: owned as chapters, a volume release surfaced; the chapter
        // axis confirms currency.
        assert_eq!(
            compute_status(false, Some(2.0), Some(150.0), None, Some(150.0)),
            CodexStatus::Complete
        );
    }

    #[test]
    fn behind_wins_over_present() {
        // Volume is uncomparable (Present-ish) but chapter is behind ->
        // Behind takes precedence as the actionable signal.
        assert_eq!(
            compute_status(false, Some(10.0), Some(200.0), None, Some(150.0)),
            CodexStatus::Behind
        );
    }

    #[test]
    fn behind_when_comparable_axis_is_behind_despite_uncomparable_sibling() {
        // Volume axis compares and is behind (own 4, source surfaced 5); the
        // uncomparable chapter axis doesn't suppress the actionable signal.
        assert_eq!(
            compute_status(false, Some(5.0), Some(22.0), Some(4.0), None),
            CodexStatus::Behind
        );
    }

    #[test]
    fn ignore_completion_short_circuits_to_ignored() {
        // The flag forces Ignored regardless of the maxima, including the
        // cases that would otherwise read Behind, Present, or Complete.
        assert_eq!(
            compute_status(true, Some(12.0), None, Some(8.0), None),
            CodexStatus::Ignored,
            "would be Behind without the flag"
        );
        assert_eq!(
            compute_status(true, Some(10.0), None, None, Some(300.0)),
            CodexStatus::Ignored,
            "would be Present without the flag"
        );
        assert_eq!(
            compute_status(true, None, None, Some(5.0), Some(10.0)),
            CodexStatus::Ignored,
            "would be Complete without the flag"
        );
    }
}
