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
    /// Owned, but the relevant Codex maximum didn't parse, so we can't judge
    /// whether it's caught up.
    Present,
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

/// Derive the presence status from the parsed maxima. Compares per tracked
/// dimension; `Behind` (actionable) wins over `Present` (can't fully judge)
/// over `Complete`. A series with nothing discovered to compare against is
/// `Complete` (we own it, nothing newer is known).
pub fn compute_status(
    highest_volume: Option<f64>,
    highest_chapter: Option<f64>,
    codex_max_volume: Option<f64>,
    codex_max_chapter: Option<f64>,
) -> CodexStatus {
    let vol_tracked = highest_volume.is_some();
    let chap_tracked = highest_chapter.is_some();

    let vol_behind = vol_tracked && codex_max_volume.is_some_and(|c| c < highest_volume.unwrap());
    let chap_behind =
        chap_tracked && codex_max_chapter.is_some_and(|c| c < highest_chapter.unwrap());

    // Tracked on our side but Codex has no parsed maximum for that dimension.
    let vol_uncomparable = vol_tracked && codex_max_volume.is_none();
    let chap_uncomparable = chap_tracked && codex_max_chapter.is_none();

    if vol_behind || chap_behind {
        CodexStatus::Behind
    } else if vol_uncomparable || chap_uncomparable {
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
    highest_volume: Option<f64>,
    highest_chapter: Option<f64>,
    base_url: Option<&str>,
) -> CodexInfo {
    let status = compute_status(
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
            compute_status(None, None, Some(5.0), Some(10.0)),
            CodexStatus::Complete
        );
    }

    #[test]
    fn complete_when_codex_caught_up_on_tracked_dims() {
        assert_eq!(
            compute_status(Some(10.0), None, Some(10.0), None),
            CodexStatus::Complete
        );
        assert_eq!(
            compute_status(Some(10.0), Some(100.0), Some(12.0), Some(100.0)),
            CodexStatus::Complete
        );
    }

    #[test]
    fn behind_when_any_tracked_dim_is_below() {
        // Volume behind.
        assert_eq!(
            compute_status(Some(12.0), None, Some(8.0), None),
            CodexStatus::Behind
        );
        // Chapter behind even though volume is caught up.
        assert_eq!(
            compute_status(Some(10.0), Some(200.0), Some(10.0), Some(150.0)),
            CodexStatus::Behind
        );
    }

    #[test]
    fn present_when_tracked_dim_is_uncomparable() {
        // We discovered volume 10 but Codex parsed no volume max.
        assert_eq!(
            compute_status(Some(10.0), None, None, Some(300.0)),
            CodexStatus::Present
        );
    }

    #[test]
    fn behind_wins_over_present() {
        // Volume is uncomparable (Present-ish) but chapter is behind ->
        // Behind takes precedence as the actionable signal.
        assert_eq!(
            compute_status(Some(10.0), Some(200.0), None, Some(150.0)),
            CodexStatus::Behind
        );
    }

    #[test]
    fn cross_dimension_mismatch_reads_as_behind() {
        // Surfaced as volumes, owned only as chapters on Codex: the volume
        // dimension is uncomparable, but there's no behind dim -> Present.
        assert_eq!(
            compute_status(Some(10.0), None, None, None),
            CodexStatus::Present
        );
    }
}
