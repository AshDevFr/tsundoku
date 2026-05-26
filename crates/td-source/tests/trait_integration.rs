//! Trait-level test that a stub second source kind lives entirely in the
//! consumer crate — the `td-source` core does not change to add it. Adding
//! a new source kind must require no edits to this crate; the stub source
//! below is the proof.

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use std::sync::Arc;
use td_source::{
    DiscoveredRelease, DiscoverySource, ExternalLinks, PollContext, PollOutcome, SourceRegistry,
    SourceResult,
};

/// A made-up "rsslike" source that satisfies the trait. The fact that this
/// compiles without touching `td-source`'s own crate is the contract.
struct RssLikeSource {
    name: &'static str,
}

#[async_trait]
impl DiscoverySource for RssLikeSource {
    fn name(&self) -> &str {
        self.name
    }
    fn kind(&self) -> &str {
        "rsslike"
    }
    async fn poll(&self, _ctx: &PollContext) -> SourceResult<PollOutcome> {
        Ok(PollOutcome::from_releases(vec![DiscoveredRelease {
            source_kind: "rsslike".into(),
            source_name: self.name.into(),
            external_id: "abc".into(),
            title: "Some Release".into(),
            link: "https://example.com/post/abc".into(),
            magnet: None,
            torrent_url: None,
            ddl_url: Some("https://example.com/dl/abc.zip".into()),
            info_hash: None,
            size_bytes: Some(1024),
            files: vec!["release.cbz".into()],
            description_html: None,
            external_links: ExternalLinks::default(),
            posted_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        }]))
    }
}

#[tokio::test]
async fn registry_holds_a_user_defined_source_without_core_changes() {
    let mut b = SourceRegistry::builder();
    b.register(Arc::new(RssLikeSource { name: "feed-1" }))
        .unwrap();
    let reg = b.build();
    let source = reg.get("feed-1").expect("source should be registered");
    assert_eq!(source.kind(), "rsslike");

    let outcome = source.poll(&PollContext::default()).await.unwrap();
    assert_eq!(outcome.releases.len(), 1);
    let r = &outcome.releases[0];
    assert_eq!(r.source_kind, "rsslike");
    assert_eq!(r.external_id, "abc");
}
