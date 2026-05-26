//! Provider error type.
//!
//! The trait surface unifies "couldn't reach the provider", "the provider
//! told us to back off", and "the provider returned something we couldn't
//! parse" so callers (resolver, scheduler, CLI) can route each case without
//! caring which provider produced it.

use std::time::Duration;

#[derive(thiserror::Error, Debug)]
pub enum MetadataError {
    /// Network error, DNS failure, TLS error, etc. Caller may retry with
    /// backoff; the resolver typically gives up on the foreign-ID step and
    /// falls through to fuzzy title search.
    #[error("provider {provider} unavailable: {source}")]
    Unavailable {
        provider: String,
        #[source]
        source: anyhow::Error,
    },

    /// Provider returned 429 (or equivalent). `retry_after` is the value
    /// the provider asked for, if any. Scheduler / CLI should honor it.
    #[error("provider {provider} rate limited; retry_after={retry_after:?}")]
    RateLimited {
        provider: String,
        retry_after: Option<Duration>,
    },

    /// Provider returned 401/403 or refused our credentials. Operator
    /// needs to fix the config; retrying is pointless.
    #[error("provider {provider} authentication failed")]
    AuthFailed { provider: String },

    /// Provider returned a response we could parse the envelope of, but
    /// the payload didn't deserialize into our canonical shape. Indicates
    /// a provider-side schema drift; surface loudly so we notice.
    #[error("provider {provider} returned malformed response: {message}")]
    Malformed { provider: String, message: String },

    /// Provider id is registered but the corresponding config block is
    /// missing required fields (e.g. no API key). Distinct from
    /// `AuthFailed`: this never reached the network.
    #[error("provider {provider} is not configured: {message}")]
    NotConfigured { provider: String, message: String },

    /// Catch-all for unanticipated internal errors. Prefer the typed
    /// variants above when the failure shape is known.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type MetadataResult<T> = Result<T, MetadataError>;
