//! Series catalog export.
//!
//! `GET /series/export` dumps the **whole** filtered catalog (no pagination)
//! as a downloadable JSON, CSV, or Markdown file, for feeding to an external
//! LLM agent ("here is what exists / is available / I don't own"). It reuses
//! the exact filter machinery of `GET /series`
//! ([`super::series::apply_series_filters`] + the admin-only codex-status
//! filter) so the export and the browse list can never disagree about what a
//! filter selects.
//!
//! The endpoint is admin-only (mounted in the `require_admin` writes group,
//! like `GET /codex/status`) because it surfaces Codex ownership, which must
//! never reach the public read tier. The caller is therefore always an admin,
//! so the ownership fields are unconditionally available.
//!
//! Field selection mirrors the Codex export modal: `fields=` is a
//! comma-separated allow-list of [`ExportField`] keys (absent ⇒ all);
//! `canonicalTitle` is always present. `includeReleases=true` nests each
//! series' linked releases — in JSON and Markdown only; CSV is a flat table
//! and carries `releaseCount` instead.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sea_orm::{EntityTrait, QueryOrder};
use serde::{Deserialize, Serialize};
use td_db::entities::{releases, series, series_external_ids};
use td_db::repos::{codex_link_repo, releases_repo, series_external_ids_repo, tagging_repo};
use utoipa::IntoParams;

use crate::codex_presence::compute_status;
use crate::errors::{ApiError, ApiResult};
use crate::handlers::series::{
    SeriesListQuery, apply_codex_id_filter, apply_series_filters, codex_status_filter,
};
use crate::state::AppState;

/// Output format for the export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportFormat {
    Json,
    Csv,
    Markdown,
}

impl ExportFormat {
    /// Lenient parse (mirrors the list endpoint's lenient query handling):
    /// anything unrecognized falls back to JSON.
    fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("csv") => Self::Csv,
            Some("markdown") | Some("md") => Self::Markdown,
            _ => Self::Json,
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            Self::Json => "application/json; charset=utf-8",
            Self::Csv => "text/csv; charset=utf-8",
            Self::Markdown => "text/markdown; charset=utf-8",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Markdown => "md",
        }
    }
}

/// A single exportable column. The backend is the authority on which keys are
/// valid; the frontend owns the human labels and grouping. `CanonicalTitle` is
/// always included regardless of selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExportField {
    Id,
    CanonicalTitle,
    AlternateTitles,
    CoverUrl,
    MetadataSource,
    FirstSeenAt,
    LastReleaseAt,
    MetadataFetchedAt,
    Kind,
    Status,
    Year,
    Description,
    Genres,
    Tags,
    ExternalIds,
    TotalVolumes,
    TotalChapters,
    HighestVolume,
    HighestChapter,
    ReleaseCount,
    Rating,
    Owned,
    CodexStatus,
}

impl ExportField {
    /// Canonical column order. Drives both the default "all fields" export and
    /// the column order in CSV/Markdown output.
    const ALL: [ExportField; 23] = [
        ExportField::Id,
        ExportField::CanonicalTitle,
        ExportField::AlternateTitles,
        ExportField::CoverUrl,
        ExportField::MetadataSource,
        ExportField::FirstSeenAt,
        ExportField::LastReleaseAt,
        ExportField::MetadataFetchedAt,
        ExportField::Kind,
        ExportField::Status,
        ExportField::Year,
        ExportField::Description,
        ExportField::Genres,
        ExportField::Tags,
        ExportField::ExternalIds,
        ExportField::TotalVolumes,
        ExportField::TotalChapters,
        ExportField::HighestVolume,
        ExportField::HighestChapter,
        ExportField::ReleaseCount,
        ExportField::Rating,
        ExportField::Owned,
        ExportField::CodexStatus,
    ];

    /// The camelCase key used as the JSON object key, CSV header, and Markdown
    /// column title. Also the value accepted in the `fields=` query param.
    fn key(self) -> &'static str {
        match self {
            ExportField::Id => "id",
            ExportField::CanonicalTitle => "canonicalTitle",
            ExportField::AlternateTitles => "alternateTitles",
            ExportField::CoverUrl => "coverUrl",
            ExportField::MetadataSource => "metadataSource",
            ExportField::FirstSeenAt => "firstSeenAt",
            ExportField::LastReleaseAt => "lastReleaseAt",
            ExportField::MetadataFetchedAt => "metadataFetchedAt",
            ExportField::Kind => "kind",
            ExportField::Status => "status",
            ExportField::Year => "year",
            ExportField::Description => "description",
            ExportField::Genres => "genres",
            ExportField::Tags => "tags",
            ExportField::ExternalIds => "externalIds",
            ExportField::TotalVolumes => "totalVolumes",
            ExportField::TotalChapters => "totalChapters",
            ExportField::HighestVolume => "highestVolume",
            ExportField::HighestChapter => "highestChapter",
            ExportField::ReleaseCount => "releaseCount",
            ExportField::Rating => "rating",
            ExportField::Owned => "owned",
            ExportField::CodexStatus => "codexStatus",
        }
    }

    fn from_key(key: &str) -> Option<ExportField> {
        ExportField::ALL.into_iter().find(|f| f.key() == key)
    }

    /// Resolve the `fields=` query param to an ordered, deduplicated list of
    /// fields. `None`/empty (or only unrecognized keys) ⇒ every field. Unknown
    /// keys are dropped leniently. `canonicalTitle` is always forced in so an
    /// export can never lose the one column that identifies a row.
    ///
    /// Order is canonical ([`ExportField::ALL`]), not the user's input order,
    /// so output columns are deterministic regardless of how the param was
    /// spelled.
    pub(crate) fn parse_selection(raw: Option<&str>) -> Vec<ExportField> {
        let requested: Vec<ExportField> = raw
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .filter_map(ExportField::from_key)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if requested.is_empty() {
            return ExportField::ALL.to_vec();
        }
        ExportField::ALL
            .into_iter()
            .filter(|f| *f == ExportField::CanonicalTitle || requested.contains(f))
            .collect()
    }
}

/// A scalar cell value, format-agnostic. Rendered per output format by the
/// serializers. Releases are carried separately on [`ExportRecord`] (they're
/// nested, not a scalar column), so there is no list-of-objects variant here.
#[derive(Debug, Clone)]
enum ExportValue {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    List(Vec<String>),
    Null,
}

impl ExportValue {
    fn to_json(&self) -> serde_json::Value {
        match self {
            ExportValue::Str(s) => serde_json::Value::String(s.clone()),
            ExportValue::Int(i) => serde_json::Value::from(*i),
            ExportValue::Float(f) => serde_json::Value::from(*f),
            ExportValue::Bool(b) => serde_json::Value::Bool(*b),
            ExportValue::List(v) => serde_json::Value::Array(
                v.iter()
                    .map(|s| serde_json::Value::String(s.clone()))
                    .collect(),
            ),
            ExportValue::Null => serde_json::Value::Null,
        }
    }

    /// Plain-text rendering for a CSV/Markdown cell (before per-format
    /// escaping). `Null` is the empty string; lists join with `"; "`.
    fn to_cell(&self) -> String {
        match self {
            ExportValue::Str(s) => s.clone(),
            ExportValue::Int(i) => i.to_string(),
            ExportValue::Float(f) => fmt_f64(*f),
            ExportValue::Bool(b) => b.to_string(),
            ExportValue::List(v) => v.join("; "),
            ExportValue::Null => String::new(),
        }
    }
}

/// `f64` without a trailing `.0` for whole numbers (volume/chapter spans and
/// ratings read better as `12` and `8.5` than `12.0`).
fn fmt_f64(f: f64) -> String {
    if f.fract() == 0.0 && f.is_finite() {
        format!("{}", f as i64)
    } else {
        format!("{f}")
    }
}

/// A volume/chapter span for the export's nested release output. Backed by the
/// `*_span_json` columns, which now store a gap-preserving list of
/// `td_source::Span`. This single-span DTO carries the coarse `(min, max)` of
/// that list; surfacing the full list (with gaps) is a Phase 4 follow-up.
#[derive(Debug, Clone, Serialize)]
struct SpanDto {
    start: f64,
    end: f64,
}

impl SpanDto {
    fn human(&self) -> String {
        if (self.start - self.end).abs() < f64::EPSILON {
            fmt_f64(self.start)
        } else {
            format!("{}-{}", fmt_f64(self.start), fmt_f64(self.end))
        }
    }
}

fn parse_span(raw: Option<&str>) -> Option<SpanDto> {
    // The column stores a gap-preserving list; collapse it to a coarse
    // (min, max) for the current single-span export field. Tolerant of the
    // legacy single-object shape via `spans_from_json`.
    let spans = td_source::spans_from_json(raw);
    let start = spans.iter().map(|s| s.start).reduce(f64::min)?;
    let end = spans.iter().map(|s| s.end).reduce(f64::max)?;
    Some(SpanDto { start, end })
}

/// One linked release in the nested `includeReleases` output. Field naming
/// mirrors `ReleaseDto` so a consumer sees the same shape it would from
/// `GET /releases`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseExport {
    id: String,
    title: String,
    /// The feed the release came from (`releases.source_name`).
    source: String,
    source_kind: String,
    link: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_bytes: Option<i64>,
    posted_at: i64,
    resolution_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    volume_span: Option<SpanDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chapter_span: Option<SpanDto>,
}

impl ReleaseExport {
    fn from_model(m: releases::Model) -> Self {
        ReleaseExport {
            volume_span: parse_span(m.volume_span_json.as_deref()),
            chapter_span: parse_span(m.chapter_span_json.as_deref()),
            id: m.id,
            title: m.title,
            source: m.source_name,
            source_kind: m.source_kind,
            link: m.link,
            size_bytes: m.size_bytes,
            posted_at: m.posted_at,
            resolution_status: m.resolution_status,
        }
    }

    /// Markdown bullet: `Title (vol 1-3, ch 1-30) — <link>`. Newlines in a
    /// title are flattened so the bullet stays on one line.
    fn md_bullet(&self) -> String {
        let title = self.title.replace(['\n', '\r'], " ");
        let mut spans = Vec::new();
        if let Some(v) = &self.volume_span {
            spans.push(format!("vol {}", v.human()));
        }
        if let Some(c) = &self.chapter_span {
            spans.push(format!("ch {}", c.human()));
        }
        let span_suffix = if spans.is_empty() {
            String::new()
        } else {
            format!(" ({})", spans.join(", "))
        };
        format!("{title}{span_suffix} — {}", self.link)
    }
}

/// One series' export row: the selected scalar columns in canonical order,
/// plus its nested releases when `includeReleases` is on.
struct ExportRecord {
    fields: Vec<(&'static str, ExportValue)>,
    releases: Option<Vec<ReleaseExport>>,
}

impl ExportRecord {
    /// The `canonicalTitle` cell, used to head a series' Markdown release
    /// section. Always present because `parse_selection` forces the column in.
    fn title(&self) -> String {
        self.fields
            .iter()
            .find(|(k, _)| *k == ExportField::CanonicalTitle.key())
            .map(|(_, v)| v.to_cell())
            .unwrap_or_default()
    }
}

/// Query string for `GET /series/export`. The filter fields mirror
/// [`SeriesListQuery`] (minus pagination and the relevance `q` search, which
/// is a ranking path, not a catalog filter); the export-specific params are
/// `format`, `fields`, and `includeReleases`.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(default, rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct SeriesExportQuery {
    /// `json` (default), `csv`, or `markdown`.
    pub format: Option<String>,
    /// Comma-separated [`ExportField`] keys. Absent ⇒ all fields.
    pub fields: Option<String>,
    /// When `true`, nest each series' linked releases (JSON/Markdown only;
    /// CSV stays a flat series-level table and carries `releaseCount`).
    pub include_releases: bool,
    // ---- filters, mirrored from SeriesListQuery ----
    pub kind: Option<String>,
    pub status: Option<String>,
    pub owned: Option<bool>,
    pub has_releases: Option<bool>,
    pub metadata_source: Option<String>,
    pub genres: Option<String>,
    pub genres_mode: Option<String>,
    pub tags: Option<String>,
    pub tags_mode: Option<String>,
    pub codex_status: Option<String>,
}

impl SeriesExportQuery {
    /// Project the filter fields onto a [`SeriesListQuery`] so the export can
    /// reuse `apply_series_filters` / `codex_status_filter` verbatim.
    /// Pagination and `sort`/`order`/`q` are irrelevant to the export and left
    /// at their defaults.
    fn to_list_query(&self) -> SeriesListQuery {
        SeriesListQuery {
            kind: self.kind.clone(),
            status: self.status.clone(),
            owned: self.owned,
            has_releases: self.has_releases,
            metadata_source: self.metadata_source.clone(),
            genres: self.genres.clone(),
            genres_mode: self.genres_mode.clone(),
            tags: self.tags.clone(),
            tags_mode: self.tags_mode.clone(),
            codex_status: self.codex_status.clone(),
            ..SeriesListQuery::default()
        }
    }
}

/// Export the (filtered) series catalog as a downloadable file.
#[utoipa::path(
    get,
    path = "/api/v1/series/export",
    tag = "series",
    operation_id = "export_series",
    params(SeriesExportQuery),
    responses(
        (status = 200, description = "Catalog export as a downloadable JSON/CSV/Markdown file")
    ),
    security(("admin" = []))
)]
pub async fn export(
    State(state): State<AppState>,
    Query(query): Query<SeriesExportQuery>,
) -> ApiResult<Response> {
    let format = ExportFormat::parse(query.format.as_deref());
    let selected = ExportField::parse_selection(query.fields.as_deref());
    let list_query = query.to_list_query();

    // Same filter pipeline as `GET /series`. The caller is always an admin
    // (admin-gated route), so the codex-status filter is enabled.
    let mut select = apply_series_filters(series::Entity::find(), &list_query);
    if let Some(filter) = codex_status_filter(&state, &list_query, true).await? {
        select = apply_codex_id_filter(select, &filter);
    }
    // Alphabetical by title: a stable, human/agent-friendly order for a dump
    // (the browse list's recency sort is a UI concern, not an export one).
    let rows = select
        .order_by_asc(series::Column::CanonicalTitle)
        .all(&state.db)
        .await
        .map_err(anyhow_err)?;

    let records = build_records(&state, rows, &selected, query.include_releases).await?;

    let body = match format {
        ExportFormat::Json => render_json(&records),
        ExportFormat::Csv => render_csv(&selected, &records),
        ExportFormat::Markdown => render_markdown(&selected, &records, query.include_releases),
    };

    let date = Utc::now().format("%Y-%m-%d");
    let filename = format!("tsundoku-series-export-{date}.{}", format.extension());
    let disposition = format!("attachment; filename=\"{filename}\"");
    let response = (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static(format.content_type()),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&disposition).map_err(|e| anyhow_err(anyhow::anyhow!(e)))?,
            ),
        ],
        body,
    )
        .into_response();
    Ok(response)
}

/// Hydrate the filtered rows into [`ExportRecord`]s. Genres / tags / counts are
/// always loaded (cheap, batched); external IDs, Codex links, and releases are
/// loaded only when a selected field (or `includeReleases`) needs them.
async fn build_records(
    state: &AppState,
    rows: Vec<series::Model>,
    selected: &[ExportField],
    include_releases: bool,
) -> ApiResult<Vec<ExportRecord>> {
    let ids: Vec<i32> = rows.iter().map(|m| m.id).collect();

    let genres_map = tagging_repo::genres_by_series_ids(&state.db, &ids)
        .await
        .map_err(anyhow_err)?;
    let tags_map = tagging_repo::tags_by_series_ids(&state.db, &ids)
        .await
        .map_err(anyhow_err)?;
    let counts_map = releases_repo::count_by_series_ids(&state.db, &ids)
        .await
        .map_err(anyhow_err)?;

    let want_external = selected.contains(&ExportField::ExternalIds);
    let external_map: HashMap<i32, Vec<series_external_ids::Model>> = if want_external {
        series_external_ids_repo::by_series_ids(&state.db, &ids)
            .await
            .map_err(anyhow_err)?
    } else {
        HashMap::new()
    };

    let want_codex =
        selected.contains(&ExportField::Owned) || selected.contains(&ExportField::CodexStatus);
    let codex_map: HashMap<i32, codex_link_repo::Model> = if want_codex {
        codex_link_repo::get_for_series_ids(&state.db, &ids)
            .await
            .map_err(anyhow_err)?
            .into_iter()
            .map(|l| (l.series_id, l))
            .collect()
    } else {
        HashMap::new()
    };

    let mut releases_map: HashMap<i32, Vec<releases::Model>> = if include_releases {
        releases_repo::list_by_series_ids(&state.db, &ids)
            .await
            .map_err(anyhow_err)?
    } else {
        HashMap::new()
    };

    Ok(rows
        .into_iter()
        .map(|m| {
            let genres = genres_map.get(&m.id).cloned().unwrap_or_default();
            let tags = tags_map.get(&m.id).cloned().unwrap_or_default();
            let release_count = counts_map.get(&m.id).copied().unwrap_or(0);
            let external = external_map.get(&m.id).cloned().unwrap_or_default();
            let codex = codex_map.get(&m.id);

            let fields = selected
                .iter()
                .map(|f| {
                    (
                        f.key(),
                        extract_value(*f, &m, &genres, &tags, release_count, &external, codex),
                    )
                })
                .collect();

            let releases = include_releases.then(|| {
                releases_map
                    .remove(&m.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(ReleaseExport::from_model)
                    .collect()
            });

            ExportRecord { fields, releases }
        })
        .collect())
}

/// Compute one field's value for a series.
#[allow(clippy::too_many_arguments)]
fn extract_value(
    field: ExportField,
    m: &series::Model,
    genres: &[String],
    tags: &[String],
    release_count: i64,
    external: &[series_external_ids::Model],
    codex: Option<&codex_link_repo::Model>,
) -> ExportValue {
    let opt_str = |v: &Option<String>| match v {
        Some(s) => ExportValue::Str(s.clone()),
        None => ExportValue::Null,
    };
    let opt_int = |v: Option<i32>| match v {
        Some(i) => ExportValue::Int(i as i64),
        None => ExportValue::Null,
    };
    let opt_float = |v: Option<f64>| match v {
        Some(f) => ExportValue::Float(f),
        None => ExportValue::Null,
    };
    match field {
        ExportField::Id => ExportValue::Int(m.id as i64),
        ExportField::CanonicalTitle => ExportValue::Str(m.canonical_title.clone()),
        ExportField::AlternateTitles => ExportValue::List(
            m.alternate_titles_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
                .unwrap_or_default(),
        ),
        ExportField::CoverUrl => opt_str(&m.cover_url),
        ExportField::MetadataSource => ExportValue::Str(m.metadata_source.clone()),
        ExportField::FirstSeenAt => ExportValue::Int(m.first_seen_at),
        ExportField::LastReleaseAt => ExportValue::Int(m.last_release_at),
        ExportField::MetadataFetchedAt => ExportValue::Int(m.metadata_fetched_at),
        ExportField::Kind => opt_str(&m.kind),
        ExportField::Status => opt_str(&m.status),
        ExportField::Year => opt_int(m.year),
        ExportField::Description => opt_str(&m.description),
        ExportField::Genres => ExportValue::List(genres.to_vec()),
        ExportField::Tags => ExportValue::List(tags.to_vec()),
        ExportField::ExternalIds => ExportValue::List(
            external
                .iter()
                .map(|e| format!("{}:{}", e.provider, e.external_id))
                .collect(),
        ),
        ExportField::TotalVolumes => opt_int(m.total_volumes),
        ExportField::TotalChapters => opt_int(m.total_chapters),
        ExportField::HighestVolume => opt_float(m.highest_volume),
        ExportField::HighestChapter => opt_float(m.highest_chapter),
        ExportField::ReleaseCount => ExportValue::Int(release_count),
        ExportField::Rating => opt_float(m.rating),
        ExportField::Owned => ExportValue::Bool(codex.is_some()),
        ExportField::CodexStatus => match codex {
            Some(link) => {
                let status = compute_status(
                    m.ignore_completion,
                    m.highest_volume,
                    m.highest_chapter,
                    link.local_max_volume,
                    link.local_max_chapter,
                );
                // `serde_json` renders the `#[serde(rename_all = "lowercase")]`
                // variant; strip the surrounding quotes to get the bare token.
                ExportValue::Str(
                    serde_json::to_value(status)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default(),
                )
            }
            None => ExportValue::Null,
        },
    }
}

fn render_json(records: &[ExportRecord]) -> String {
    let arr: Vec<serde_json::Value> = records
        .iter()
        .map(|rec| {
            let mut map = serde_json::Map::new();
            for (k, v) in &rec.fields {
                map.insert((*k).to_string(), v.to_json());
            }
            if let Some(releases) = &rec.releases {
                map.insert(
                    "releases".to_string(),
                    serde_json::to_value(releases).unwrap_or(serde_json::Value::Null),
                );
            }
            serde_json::Value::Object(map)
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::Value::Array(arr))
        .unwrap_or_else(|_| "[]".to_string())
}

/// RFC 4180 field: quote when it contains a comma, quote, CR, or LF; escape
/// embedded quotes by doubling.
fn csv_field(raw: &str) -> String {
    if raw.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", raw.replace('"', "\"\""))
    } else {
        raw.to_string()
    }
}

fn render_csv(selected: &[ExportField], records: &[ExportRecord]) -> String {
    let mut out = String::new();
    // Header.
    let header: Vec<String> = selected.iter().map(|f| csv_field(f.key())).collect();
    out.push_str(&header.join(","));
    out.push_str("\r\n");
    // Rows. Releases are intentionally omitted (flat table); `releaseCount`
    // carries the availability signal instead.
    for rec in records {
        let row: Vec<String> = rec
            .fields
            .iter()
            .map(|(_, v)| csv_field(&v.to_cell()))
            .collect();
        out.push_str(&row.join(","));
        out.push_str("\r\n");
    }
    out
}

/// Escape a Markdown table cell: pipes would split the cell, and newlines
/// would break the row, so both are neutralized.
fn md_cell(raw: &str) -> String {
    let cell = raw.replace('|', "\\|").replace(['\n', '\r'], " ");
    if cell.is_empty() {
        // A blank cell renders as an empty column; a single space keeps the
        // table grid intact in stricter renderers.
        " ".to_string()
    } else {
        cell
    }
}

fn render_markdown(
    selected: &[ExportField],
    records: &[ExportRecord],
    include_releases: bool,
) -> String {
    let mut out = String::new();
    out.push_str("# tsundoku series catalog\n\n");

    // Scalar table.
    let header: Vec<String> = selected.iter().map(|f| md_cell(f.key())).collect();
    out.push_str(&format!("| {} |\n", header.join(" | ")));
    let sep: Vec<&str> = selected.iter().map(|_| "---").collect();
    out.push_str(&format!("| {} |\n", sep.join(" | ")));
    for rec in records {
        let row: Vec<String> = rec
            .fields
            .iter()
            .map(|(_, v)| md_cell(&v.to_cell()))
            .collect();
        out.push_str(&format!("| {} |\n", row.join(" | ")));
    }

    // Per-series release breakdown. A table can't nest a variable-length list,
    // so releases get their own section below the table when requested.
    if include_releases {
        out.push_str("\n## Releases\n");
        for rec in records {
            let releases = rec.releases.as_deref().unwrap_or(&[]);
            if releases.is_empty() {
                continue;
            }
            out.push_str(&format!(
                "\n### {}\n\n",
                rec.title().replace(['\n', '\r'], " ")
            ));
            for r in releases {
                out.push_str(&format!("- {}\n", r.md_bullet()));
            }
        }
    }

    out
}

fn anyhow_err<E: Into<anyhow::Error>>(e: E) -> ApiError {
    ApiError::Internal(e.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_selection_defaults_to_all_when_absent_or_empty() {
        assert_eq!(
            ExportField::parse_selection(None),
            ExportField::ALL.to_vec()
        );
        assert_eq!(
            ExportField::parse_selection(Some("   ")),
            ExportField::ALL.to_vec()
        );
        // Only unrecognized keys ⇒ treated as "no selection" ⇒ all.
        assert_eq!(
            ExportField::parse_selection(Some("bogus,nope")),
            ExportField::ALL.to_vec()
        );
    }

    #[test]
    fn parse_selection_keeps_requested_in_canonical_order_and_drops_unknown() {
        // Requested out of order + an unknown key. `canonicalTitle` is always
        // forced in, and the result is in canonical column order.
        let got = ExportField::parse_selection(Some("rating,kind,bogus"));
        assert_eq!(
            got,
            vec![
                ExportField::CanonicalTitle,
                ExportField::Kind,
                ExportField::Rating
            ],
            "canonical order, unknown dropped, title forced in"
        );
    }

    #[test]
    fn parse_selection_always_includes_canonical_title() {
        let got = ExportField::parse_selection(Some("rating"));
        assert!(got.contains(&ExportField::CanonicalTitle));
        assert!(got.contains(&ExportField::Rating));
        // Canonical order means title precedes rating.
        assert_eq!(got, vec![ExportField::CanonicalTitle, ExportField::Rating]);
    }

    fn rec(fields: Vec<(&'static str, ExportValue)>) -> ExportRecord {
        ExportRecord {
            fields,
            releases: None,
        }
    }

    #[test]
    fn csv_quotes_commas_quotes_and_newlines() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("line1\nline2"), "\"line1\nline2\"");
    }

    #[test]
    fn csv_renders_header_and_rows_with_selected_columns() {
        let selected = vec![ExportField::CanonicalTitle, ExportField::Genres];
        let records = vec![rec(vec![
            ("canonicalTitle", ExportValue::Str("One, Piece".to_string())),
            (
                "genres",
                ExportValue::List(vec!["Action".to_string(), "Adventure".to_string()]),
            ),
        ])];
        let csv = render_csv(&selected, &records);
        let mut lines = csv.lines();
        assert_eq!(lines.next().unwrap(), "canonicalTitle,genres");
        // Title has a comma ⇒ quoted; list joins with "; ".
        assert_eq!(lines.next().unwrap(), "\"One, Piece\",Action; Adventure");
    }

    #[test]
    fn json_includes_releases_only_when_present() {
        let with = ExportRecord {
            fields: vec![("canonicalTitle", ExportValue::Str("X".into()))],
            releases: Some(vec![]),
        };
        let without = rec(vec![("canonicalTitle", ExportValue::Str("Y".into()))]);
        let json_with = render_json(std::slice::from_ref(&with));
        assert!(json_with.contains("\"releases\""));
        let json_without = render_json(&[without]);
        assert!(!json_without.contains("\"releases\""));
    }

    #[test]
    fn markdown_table_has_header_separator_and_rows() {
        let selected = vec![ExportField::CanonicalTitle, ExportField::Year];
        let records = vec![rec(vec![
            ("canonicalTitle", ExportValue::Str("Berserk".into())),
            ("year", ExportValue::Int(1989)),
        ])];
        let md = render_markdown(&selected, &records, false);
        assert!(md.contains("| canonicalTitle | year |"));
        assert!(md.contains("| --- | --- |"));
        assert!(md.contains("| Berserk | 1989 |"));
        // No releases section when not requested.
        assert!(!md.contains("## Releases"));
    }

    #[test]
    fn markdown_escapes_pipes_in_cells() {
        let selected = vec![ExportField::CanonicalTitle];
        let records = vec![rec(vec![(
            "canonicalTitle",
            ExportValue::Str("A | B".into()),
        )])];
        let md = render_markdown(&selected, &records, false);
        assert!(md.contains("A \\| B"));
    }

    #[test]
    fn fmt_f64_trims_whole_numbers() {
        assert_eq!(fmt_f64(12.0), "12");
        assert_eq!(fmt_f64(8.5), "8.5");
    }

    #[test]
    fn span_human_collapses_single_point() {
        assert_eq!(
            SpanDto {
                start: 3.0,
                end: 3.0
            }
            .human(),
            "3"
        );
        assert_eq!(
            SpanDto {
                start: 1.0,
                end: 5.0
            }
            .human(),
            "1-5"
        );
    }

    #[test]
    fn export_format_parse_is_lenient() {
        assert_eq!(ExportFormat::parse(Some("csv")), ExportFormat::Csv);
        assert_eq!(
            ExportFormat::parse(Some("MARKDOWN")),
            ExportFormat::Markdown
        );
        assert_eq!(ExportFormat::parse(Some("md")), ExportFormat::Markdown);
        assert_eq!(ExportFormat::parse(Some("weird")), ExportFormat::Json);
        assert_eq!(ExportFormat::parse(None), ExportFormat::Json);
    }
}
