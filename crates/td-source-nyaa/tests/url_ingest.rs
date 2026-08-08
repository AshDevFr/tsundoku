//! `UrlIngestSource` behaviour for Nyaa: recognising a pasted post URL and
//! building a full `DiscoveredRelease` from the detail page alone.

use std::io::{Read, Write};
use std::net::TcpListener as StdListener;
use std::thread;
use std::time::Duration;

use td_http::HttpLimiter;
use td_source::{SearchSource, UrlIngestSource};
use td_source_nyaa::{NyaaSearch, NyaaSearchConfig};

const DETAIL_FIXTURE: &str = include_str!("fixtures/nyaa_detail_single_file.html");

fn search_with_base(site_base_url: &str) -> NyaaSearch {
    NyaaSearch::from_config(
        NyaaSearchConfig {
            name: "nyaa-search".into(),
            search_url: format!("{site_base_url}/?c=3_1"),
            timeout: Duration::from_secs(5),
            fetch_details: false,
            site_base_url: site_base_url.to_string(),
        },
        HttpLimiter::no_limit(),
    )
    .expect("building search source")
}

fn ingestable(s: &NyaaSearch) -> &dyn UrlIngestSource {
    s.as_url_ingestable()
        .expect("nyaa search should support url ingest")
}

/// Bind a listener that answers every connection with the same canned
/// response until the test ends. The thread is deliberately not joined:
/// a test that makes no request should fail on its assertions, not block
/// forever waiting on `accept`.
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

#[test]
fn handles_view_and_download_urls_on_the_configured_host() {
    let s = search_with_base("https://nyaa.si");
    let u = ingestable(&s);
    assert!(u.handles_url("https://nyaa.si/view/2111533"));
    assert!(u.handles_url("https://nyaa.si/view/2111533/"));
    assert!(u.handles_url("https://nyaa.si/view/2111533?foo=bar"));
    assert!(u.handles_url("https://nyaa.si/download/2111533.torrent"));
}

#[test]
fn rejects_other_hosts_and_non_post_paths() {
    let s = search_with_base("https://nyaa.si");
    let u = ingestable(&s);
    // Right shape, wrong site.
    assert!(!u.handles_url("https://sukebei.nyaa.si/view/2111533"));
    assert!(!u.handles_url("https://example.com/view/2111533"));
    // Right site, not a post.
    assert!(!u.handles_url("https://nyaa.si/?c=3_1"));
    assert!(!u.handles_url("https://nyaa.si/user/motbob"));
    // Not a URL at all.
    assert!(!u.handles_url("2111533"));
    assert!(!u.handles_url("magnet:?xt=urn:btih:abc"));
}

#[tokio::test]
async fn fetch_by_url_builds_a_release_from_the_detail_page_alone() {
    let base = spawn_canned_server(http_response("200 OK", DETAIL_FIXTURE));
    let s = search_with_base(&base);

    let release = ingestable(&s)
        .fetch_by_url(&format!("{base}/view/2111533?highlight=1"))
        .await
        .expect("fetch should succeed")
        .expect("post exists");

    assert_eq!(release.source_kind, "nyaa");
    assert_eq!(release.source_name, "nyaa-search");
    assert_eq!(release.external_id, "2111533");
    // The pasted URL's query string must be dropped: `releases.link` is
    // unique and the poll path always stores the bare `/view/N` form.
    assert_eq!(release.link, format!("{base}/view/2111533"));
    assert!(release.title.starts_with("ReZero - Starting Life"));
    assert_eq!(release.posted_at.timestamp(), 1_779_147_296);
    assert_eq!(
        release.size_bytes,
        Some((11.6_f64 * 1024.0 * 1024.0) as u64)
    );
    assert_eq!(
        release.info_hash.as_deref(),
        Some("3cce2a1b1dd491be89a5a2461250b1f7ee6700c7")
    );
    assert_eq!(
        release.torrent_url.as_deref(),
        Some(format!("{base}/download/2111533.torrent").as_str())
    );
    assert!(release.magnet.is_some());
    assert_eq!(release.files.len(), 1);
    assert!(release.description_html.is_some());
    assert_eq!(
        release.information_url.as_deref(),
        Some("https://discord.gg/r9gyPwJeqW")
    );
}

#[tokio::test]
async fn fetch_by_url_returns_none_when_the_post_does_not_exist() {
    let base = spawn_canned_server(http_response("404 Not Found", "<h1>404</h1>"));
    let s = search_with_base(&base);

    let got = ingestable(&s)
        .fetch_by_url(&format!("{base}/view/999999999"))
        .await
        .expect("a 404 is a normal outcome, not a transport error");

    assert!(
        got.is_none(),
        "expected None for a removed post; got {got:?}"
    );
}

#[tokio::test]
async fn fetch_by_url_errors_when_the_page_has_no_title() {
    // A page we fetched but can't read as a post means Nyaa changed shape;
    // that must surface loudly rather than persist a titleless release.
    let base = spawn_canned_server(http_response(
        "200 OK",
        "<html><body>nothing useful</body></html>",
    ));
    let s = search_with_base(&base);

    let err = ingestable(&s)
        .fetch_by_url(&format!("{base}/view/2111533"))
        .await
        .expect_err("a shapeless page should be an error");

    assert!(
        matches!(err, td_source::SourceError::Malformed { .. }),
        "expected Malformed; got {err:?}"
    );
}
