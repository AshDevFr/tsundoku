//! Source error type.

#[derive(thiserror::Error, Debug)]
pub enum SourceError {
    /// Network error, DNS failure, TLS error. Caller (scheduler) may retry
    /// with backoff; the failure is recorded on `source_state.last_error`.
    #[error("source {source_name}/{source_kind} unavailable: {source}")]
    Unavailable {
        source_kind: String,
        source_name: String,
        #[source]
        source: anyhow::Error,
    },

    /// Upstream returned a body we couldn't parse. Indicates a source-side
    /// change in shape; surface loudly so we notice.
    #[error("source {source_name}/{source_kind} returned malformed response: {message}")]
    Malformed {
        source_kind: String,
        source_name: String,
        message: String,
    },

    /// Source is registered but misconfigured (e.g. missing required URL).
    /// Distinct from `Unavailable`: never reached the network.
    #[error("source {source_name}/{source_kind} is not configured: {message}")]
    NotConfigured {
        source_kind: String,
        source_name: String,
        message: String,
    },

    /// Catch-all for unanticipated internal errors.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type SourceResult<T> = Result<T, SourceError>;
