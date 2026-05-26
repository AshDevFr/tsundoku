//! Coarse classification of scheduler-tick failures.
//!
//! Operators want to know whether a failing source is currently fighting a
//! network outage vs the upstream returning HTML when JSON is expected vs
//! the DB being unhappy. Naming five buckets covers the cases the admin UI
//! actually surfaces:
//!
//! - `network`: TCP/TLS/timeout/connection-reset before the upstream
//!   responded (reqwest connect or timeout error).
//! - `http_status`: the upstream answered, but the HTTP status was an error
//!   (4xx/5xx) — distinct from network so the donut visualises rate-limit
//!   tickets separately from transient outages.
//! - `parse`: the response decoded fine at the wire level but failed
//!   downstream parsing (RSS/JSON/etc.).
//! - `db`: a `sea_orm::DbErr` bubbled out of a repo call.
//! - `internal`: catch-all fallback so unknown errors still get a bucket.
//!
//! Unknown errors fall through to `internal` rather than panicking; review
//! the bucket regularly and promote frequent causes into named kinds.

/// String constants used as the persisted `error_kind` column value. The
/// admin UI switches on these.
pub mod kind {
    pub const NETWORK: &str = "network";
    pub const HTTP_STATUS: &str = "http_status";
    pub const PARSE: &str = "parse";
    pub const DB: &str = "db";
    pub const INTERNAL: &str = "internal";
}

/// Classify an `anyhow::Error` chain into one of the buckets above. Walks
/// the source chain so a wrapped `reqwest::Error` still classifies as
/// `network` when surfaced via `?` through a top-level handler.
pub fn classify_anyhow(err: &anyhow::Error) -> &'static str {
    for cause in err.chain() {
        if let Some(re) = cause.downcast_ref::<reqwest::Error>() {
            return classify_reqwest(re);
        }
        if cause.downcast_ref::<sea_orm::DbErr>().is_some() {
            return kind::DB;
        }
        if cause.downcast_ref::<serde_json::Error>().is_some() {
            return kind::PARSE;
        }
    }
    // Fall back to substring sniffing on the rendered message: the source
    // layer wraps everything in `td_source::SourceError`, which loses the
    // concrete type but keeps a useful Display.
    let rendered = format!("{err:#}");
    if rendered.contains("timed out")
        || rendered.contains("dns error")
        || rendered.contains("connection refused")
        || rendered.contains("connect error")
        || rendered.contains("tcp connect")
        || rendered.contains("tls handshake")
    {
        return kind::NETWORK;
    }
    if rendered.contains("status: 4") || rendered.contains("status: 5") {
        return kind::HTTP_STATUS;
    }
    if rendered.contains("parse") || rendered.contains("invalid") || rendered.contains("decode") {
        return kind::PARSE;
    }
    kind::INTERNAL
}

fn classify_reqwest(err: &reqwest::Error) -> &'static str {
    if err.is_timeout() || err.is_connect() {
        kind::NETWORK
    } else if err.is_status() {
        kind::HTTP_STATUS
    } else if err.is_decode() {
        kind::PARSE
    } else {
        kind::INTERNAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_error_classifies_as_db() {
        let err = sea_orm::DbErr::Custom("boom".into());
        let wrapped: anyhow::Error = anyhow::Error::new(err);
        assert_eq!(classify_anyhow(&wrapped), kind::DB);
    }

    #[test]
    fn serde_error_classifies_as_parse() {
        let serde_err = serde_json::from_str::<i64>("not a number").unwrap_err();
        let wrapped: anyhow::Error = anyhow::Error::new(serde_err);
        assert_eq!(classify_anyhow(&wrapped), kind::PARSE);
    }

    #[test]
    fn substring_fallback_catches_network_text() {
        let err = anyhow::anyhow!("nyaa fetch failed: connection refused");
        assert_eq!(classify_anyhow(&err), kind::NETWORK);
    }

    #[test]
    fn substring_fallback_catches_http_status_text() {
        let err = anyhow::anyhow!("upstream returned status: 503");
        assert_eq!(classify_anyhow(&err), kind::HTTP_STATUS);
    }

    #[test]
    fn unknown_error_falls_through_to_internal() {
        let err = anyhow::anyhow!("something went totally sideways with the universe");
        assert_eq!(classify_anyhow(&err), kind::INTERNAL);
    }
}
