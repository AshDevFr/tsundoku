//! `enrich` behaviour for Nyaa: a detail fetch that does not yield a post page
//! is reported as an error (so the poll counts it), while a deleted post (404)
//! stays a quiet, non-error keep-feed-data outcome.

use std::io::{Read, Write};
use std::net::TcpListener as StdListener;
use std::thread;
use std::time::Duration;

use chrono::Utc;
use td_http::HttpLimiter;
use td_source::{DiscoveredRelease, DiscoverySource, ExternalLinks, SearchSource};
use td_source_nyaa::{NyaaSearch, NyaaSearchConfig, NyaaSource, NyaaSourceConfig};

const DETAIL_FIXTURE: &str = include_str!("fixtures/nyaa_detail_information_mangabaka.html");
const INTERSTITIAL: &str = "<html><head><title>Just a moment...</title></head>\
     <body><p>Checking your browser before accessing nyaa.si</p></body></html>";

/// Same canned server as `url_ingest.rs`: answers every connection with one
/// fixed response.
fn spawn_canned_server(body: String) -> String {
    let listener = StdListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(body.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}")
}

fn http_response(status_line: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn source(base: &str) -> NyaaSource {
    NyaaSource::from_config(
        NyaaSourceConfig {
            name: "nyaa-feed".into(),
            feed_url: format!("{base}/?page=rss"),
            timeout: Duration::from_secs(5),
            fetch_details: true,
            site_base_url: base.to_string(),
        },
        HttpLimiter::no_limit(),
    )
    .unwrap()
}

fn search(base: &str) -> NyaaSearch {
    NyaaSearch::from_config(
        NyaaSearchConfig {
            name: "nyaa-search".into(),
            search_url: format!("{base}/?c=3_1"),
            timeout: Duration::from_secs(5),
            fetch_details: true,
            site_base_url: base.to_string(),
        },
        HttpLimiter::no_limit(),
    )
    .unwrap()
}

/// A release as the RSS pass leaves it: summary-line description, no files.
fn feed_only(base: &str) -> DiscoveredRelease {
    DiscoveredRelease {
        source_kind: "nyaa".into(),
        source_name: "nyaa-feed".into(),
        external_id: "2167316".into(),
        title: "A Banished Odd-jobber Starts a New Life 001-058 as v01-10".into(),
        link: format!("{base}/view/2167316"),
        magnet: None,
        torrent_url: None,
        ddl_url: None,
        info_hash: None,
        size_bytes: None,
        files: Vec::new(),
        description_html: Some("#2167316 | rss summary".into()),
        external_links: ExternalLinks::default(),
        comment_suggested_links: ExternalLinks::default(),
        information_url: None,
        posted_at: Utc::now(),
    }
}

#[tokio::test]
async fn source_enrich_fills_details_from_a_post_page() {
    let base = spawn_canned_server(http_response("200 OK", DETAIL_FIXTURE));
    let mut release = feed_only(&base);
    source(&base).enrich(&mut release).await.unwrap();
    assert_eq!(release.files.len(), 10);
    assert!(release.information_url.is_some());
    assert!(
        release
            .description_html
            .as_deref()
            .unwrap()
            .contains("| Volumes |")
    );
}

#[tokio::test]
async fn source_enrich_errors_on_a_page_that_is_not_a_post() {
    let base = spawn_canned_server(http_response("200 OK", INTERSTITIAL));
    let mut release = feed_only(&base);
    let res = source(&base).enrich(&mut release).await;
    assert!(
        res.is_err(),
        "non-post page must surface as an enrich error"
    );
    assert!(release.files.is_empty());
    assert_eq!(
        release.description_html.as_deref(),
        Some("#2167316 | rss summary")
    );
}

#[tokio::test]
async fn source_enrich_errors_on_a_non_success_status() {
    let base = spawn_canned_server(http_response("403 Forbidden", "nope"));
    let mut release = feed_only(&base);
    assert!(source(&base).enrich(&mut release).await.is_err());
}

#[tokio::test]
async fn source_enrich_keeps_feed_data_quietly_when_the_post_is_gone() {
    let base = spawn_canned_server(http_response("404 Not Found", "gone"));
    let mut release = feed_only(&base);
    source(&base).enrich(&mut release).await.unwrap();
    assert!(release.files.is_empty());
}

#[tokio::test]
async fn search_enrich_errors_on_a_page_that_is_not_a_post() {
    let base = spawn_canned_server(http_response("200 OK", INTERSTITIAL));
    let mut release = feed_only(&base);
    assert!(search(&base).enrich(&mut release).await.is_err());
}
