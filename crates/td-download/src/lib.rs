//! Torrent-client download clients for tsundoku's send-to-client action.
//!
//! A discovered release can be pushed straight into the operator's torrent
//! client. v1 targets **ruTorrent** behind a small [`DownloadClient`] trait so
//! a second client (qBittorrent, Transmission, …) is a drop-in later.
//!
//! The ruTorrent wire format lives here in two halves so it can be tested
//! without a live server:
//!
//! - [`build_add_fields`] is a pure function that turns an [`AddRequest`] into
//!   the ordered list of multipart fields `addtorrent.php` expects
//!   (`torrent_file`/`url`, `label`, `torrents_start_stopped`, `dir_edit`,
//!   `json`). No I/O, fully unit-testable.
//! - [`RuTorrentClient::add`] converts those fields into a
//!   `reqwest::multipart::Form` and POSTs them.
//!
//! Like the other client crates, this one carries no `td-config` dependency:
//! callers pass the resolved base URL, credentials, and timeout so the crate
//! stays usable from both `td-api` and any future scheduler path.

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde::Deserialize;
use td_http::{HttpLimiter, LimitedClient};

/// Errors from a download-client call. `Rejected` carries the client's own
/// error string (e.g. ruTorrent's non-`Success` `result`) so the operator sees
/// why the add failed; `Unexpected` covers non-200 HTTP (auth failures, the
/// wrong URL) that never reach the JSON body.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("download client transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("torrent client returned unexpected HTTP {0}")]
    Unexpected(u16),
    #[error("torrent client rejected the add: {0}")]
    Rejected(String),
    #[error("decoding torrent client response: {0}")]
    Decode(#[source] serde_json::Error),
}

/// Where the torrent to add comes from. `.torrent` bytes are the default
/// source (fetched upstream through the rate limiter); a magnet URL is the
/// opt-in fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddSource {
    /// Raw `.torrent` file bytes plus the file name to present in the upload.
    Torrent { bytes: Vec<u8>, file_name: String },
    /// A magnet (or remote `.torrent`) URL handed to the client to fetch.
    Magnet(String),
}

/// One add request, already resolved to a concrete source. The handler decides
/// magnet-vs-torrent and fills the per-send overrides before calling
/// [`DownloadClient::add`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddRequest {
    pub source: AddSource,
    /// Custom label, or `None` to let the client apply its own default.
    pub label: Option<String>,
    /// `true` starts the torrent immediately; `false` adds it stopped
    /// (`torrents_start_stopped`).
    pub start: bool,
    /// Download directory override, or `None` for the client default.
    pub dir: Option<String>,
}

/// A torrent client that can accept a new download.
#[async_trait::async_trait]
pub trait DownloadClient: Send + Sync {
    /// Add a torrent (file bytes or magnet) to the client.
    async fn add(&self, req: AddRequest) -> Result<(), DownloadError>;

    /// Fetch the raw `.torrent` bytes from `url`. Routed through the client's
    /// rate limiter so a fetch from the discovery source (e.g. Nyaa) stays
    /// polite — this is the counterpart to the deliberately-unlimited upload in
    /// [`DownloadClient::add`]. The send handler calls this for the
    /// torrent-file path before handing the bytes back to `add`.
    async fn fetch_torrent(&self, url: &str) -> Result<Vec<u8>, DownloadError>;

    /// Probe the client for reachability. When credentials are configured this
    /// also validates them (a `401` is surfaced as a failure). `Ok(())` means
    /// the client answered a simple request with `200`; any other status or a
    /// transport error returns the reason so the admin sees *why* a probe
    /// failed. Drives the launch / cron / manual connection tests.
    async fn test_connection(&self) -> Result<(), DownloadError>;
}

/// Map a connection probe's HTTP status to a result: `200` is
/// reachable-and-authorized, anything else is a failure carrying the code.
/// Pure so the classification is unit-testable without a live server.
fn classify_test_status(status: u16) -> Result<(), DownloadError> {
    if status == StatusCode::OK.as_u16() {
        Ok(())
    } else {
        Err(DownloadError::Unexpected(status))
    }
}

/// One multipart field in the inspectable intermediate representation the pure
/// assembler produces. Splitting this out from `reqwest::multipart::Form`
/// (which exposes no way to read its parts back) lets us unit-test the wire
/// format without a live server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormField {
    /// A plain text field: `(name, value)`.
    Text(String, String),
    /// A file part: `(name, file_name, bytes)`.
    File(String, String, Vec<u8>),
}

/// Pure assembler: turn an [`AddRequest`] into the ordered list of multipart
/// fields ruTorrent's `addtorrent.php` expects. No I/O so the
/// magnet-vs-torrent / label / start / dir decisions are unit-testable.
///
/// Wire notes:
/// - `torrent_file` (file part) **or** `url` (text) — never both.
/// - `torrents_start_stopped` is keyed on *presence*: any value means
///   add-stopped, omission means start immediately. We only emit it when
///   `start == false`.
/// - empty `label` / `dir` are dropped rather than sent blank.
/// - `json=1` makes ruTorrent answer with `{"result":"Success"}` (or an error
///   string) instead of an HTML redirect.
pub fn build_add_fields(req: &AddRequest) -> Vec<FormField> {
    let mut fields = Vec::new();

    match &req.source {
        AddSource::Torrent { bytes, file_name } => {
            fields.push(FormField::File(
                "torrent_file".into(),
                file_name.clone(),
                bytes.clone(),
            ));
        }
        AddSource::Magnet(url) => {
            fields.push(FormField::Text("url".into(), url.clone()));
        }
    }

    if let Some(label) = &req.label
        && !label.is_empty()
    {
        fields.push(FormField::Text("label".into(), label.clone()));
    }

    if !req.start {
        fields.push(FormField::Text("torrents_start_stopped".into(), "1".into()));
    }

    if let Some(dir) = &req.dir
        && !dir.is_empty()
    {
        fields.push(FormField::Text("dir_edit".into(), dir.clone()));
    }

    fields.push(FormField::Text("json".into(), "1".into()));
    fields
}

/// Convert the assembled fields into a `reqwest::multipart::Form`.
fn fields_to_form(fields: Vec<FormField>) -> reqwest::multipart::Form {
    let mut form = reqwest::multipart::Form::new();
    for field in fields {
        form = match field {
            FormField::Text(name, value) => form.text(name, value),
            FormField::File(name, file_name, bytes) => {
                let part = reqwest::multipart::Part::bytes(bytes).file_name(file_name);
                form.part(name, part)
            }
        };
    }
    form
}

/// `addtorrent.php?json=1` response body. ruTorrent returns
/// `{"result":"Success"}` on success or `{"result":"<error>"}` otherwise.
#[derive(Debug, Deserialize)]
struct AddResponse {
    result: Option<String>,
}

/// Parse a ruTorrent add response, treating `result == "Success"` as ok and
/// anything else (an error string, a missing field) as a [`DownloadError`].
/// Pure so the success/error split is testable against a fixture.
fn parse_add_response(bytes: &[u8]) -> Result<(), DownloadError> {
    let parsed: AddResponse = serde_json::from_slice(bytes).map_err(DownloadError::Decode)?;
    match parsed.result.as_deref() {
        Some("Success") => Ok(()),
        Some(other) => Err(DownloadError::Rejected(other.to_string())),
        None => Err(DownloadError::Rejected("missing result field".to_string())),
    }
}

/// Shared HTTP layer for the ruTorrent-family clients: a rate-limited reqwest
/// client plus Basic/Digest auth negotiation against a base URL. Both the
/// web-UI and XML-RPC clients hold one.
struct AuthedHttp {
    http: LimitedClient,
    base_url: String,
    username: Option<String>,
    password: Option<String>,
}

impl AuthedHttp {
    fn new(
        base_url: impl Into<String>,
        username: Option<String>,
        password: Option<String>,
        timeout: Duration,
        limiter: Arc<HttpLimiter>,
    ) -> Result<Self, DownloadError> {
        let inner = Client::builder()
            .timeout(timeout)
            .user_agent(concat!("tsundoku/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            http: limiter.client(inner),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            username,
            password,
        })
    }

    /// Send a request to `{base_url}{suffix}` handling either HTTP Basic or
    /// Digest auth. `build` adds the method-specific bits (a multipart or XML
    /// body for a POST, nothing for a GET) and is called per attempt, because a
    /// 401 Digest challenge forces a resend and multipart bodies can't be
    /// cloned.
    ///
    /// The first attempt carries preemptive Basic auth — one round-trip for
    /// Basic (or unauthenticated) servers. On a `401` whose `WWW-Authenticate`
    /// is `Digest` (common for ruTorrent behind a seedbox proxy, which `reqwest`
    /// can't do natively), it computes the Digest response and retries once. A
    /// `401` with a Basic challenge means the credentials are simply wrong, so
    /// it's returned as-is. The single unthrottled request to the operator's own
    /// box (built off `inner()`) is deliberate — the rate limiter governs the
    /// upstream `.torrent` fetch, not the client we own.
    async fn send_authed<F>(
        &self,
        method: Method,
        suffix: &str,
        build: F,
    ) -> Result<Response, DownloadError>
    where
        F: Fn(RequestBuilder) -> RequestBuilder,
    {
        let client = self.http.inner();
        let url = format!("{}{}", self.base_url, suffix);

        let mut first = build(client.request(method.clone(), &url));
        if let Some(user) = &self.username {
            first = first.basic_auth(user, self.password.clone());
        }
        let resp = first.send().await?;
        if resp.status() != StatusCode::UNAUTHORIZED {
            return Ok(resp);
        }

        // 401: only a Digest challenge is recoverable. Need a username to answer.
        let Some(user) = self.username.as_deref() else {
            return Ok(resp);
        };
        let challenge = resp
            .headers()
            .get(WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        if !challenge
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("digest")
        {
            return Ok(resp);
        }

        // The digest `uri` is the request path (no scheme/host); derive it from
        // the resolved URL so a base_url with a path prefix is handled right.
        let uri_path = reqwest::Url::parse(&url)
            .ok()
            .map(|u| match u.query() {
                Some(q) => format!("{}?{}", u.path(), q),
                None => u.path().to_string(),
            })
            .unwrap_or_else(|| suffix.to_string());

        let mut prompt = digest_auth::parse(&challenge)
            .map_err(|e| DownloadError::Rejected(format!("parsing digest challenge: {e}")))?;
        let context = digest_auth::AuthContext::new_with_method(
            user,
            self.password.clone().unwrap_or_default(),
            &uri_path,
            Option::<&[u8]>::None,
            digest_method(&method),
        );
        let header = prompt
            .respond(&context)
            .map_err(|e| DownloadError::Rejected(format!("computing digest response: {e}")))?
            .to_header_string();

        let retry = build(client.request(method, &url)).header(AUTHORIZATION, header);
        Ok(retry.send().await?)
    }

    /// Fetch `url` through the rate limiter. Used for the `.torrent` fetch from
    /// the discovery source (e.g. Nyaa) — not authed, since that host is not the
    /// client we own.
    async fn fetch_via_limiter(&self, url: &str) -> Result<Vec<u8>, DownloadError> {
        let resp = self.http.get(url).send().await?;
        match resp.status() {
            StatusCode::OK => Ok(resp.bytes().await?.to_vec()),
            other => Err(DownloadError::Unexpected(other.as_u16())),
        }
    }
}

/// ruTorrent web-UI client: POSTs the `.torrent` (or a magnet URL) to
/// `addtorrent.php`. Construct once and reuse.
pub struct RuTorrentClient {
    http: AuthedHttp,
}

impl RuTorrentClient {
    /// `base_url` is the ruTorrent root (e.g. `https://box/rutorrent`); the
    /// trailing slash is trimmed. `username`/`password` are the HTTP auth
    /// credentials (Basic or Digest, auto-negotiated), both `None` for an
    /// unauthenticated instance.
    pub fn new(
        base_url: impl Into<String>,
        username: Option<String>,
        password: Option<String>,
        timeout: Duration,
        limiter: Arc<HttpLimiter>,
    ) -> Result<Self, DownloadError> {
        Ok(Self {
            http: AuthedHttp::new(base_url, username, password, timeout, limiter)?,
        })
    }
}

/// Map a `reqwest::Method` to the `digest_auth` equivalent. Only the verbs the
/// client actually uses are mapped; anything else falls back to `GET` (the
/// digest method only feeds HA2 and we never issue those verbs here).
fn digest_method(method: &Method) -> digest_auth::HttpMethod<'static> {
    match *method {
        Method::POST => digest_auth::HttpMethod::POST,
        Method::HEAD => digest_auth::HttpMethod::HEAD,
        _ => digest_auth::HttpMethod::GET,
    }
}

#[async_trait::async_trait]
impl DownloadClient for RuTorrentClient {
    async fn add(&self, req: AddRequest) -> Result<(), DownloadError> {
        // The assembled fields are cloned per attempt because a 401 Digest
        // challenge forces a resend and `multipart::Form` isn't cloneable.
        let fields = build_add_fields(&req);
        let resp = self
            .http
            .send_authed(Method::POST, "/php/addtorrent.php", |rb| {
                rb.multipart(fields_to_form(fields.clone()))
            })
            .await?;
        match resp.status() {
            StatusCode::OK => parse_add_response(resp.bytes().await?.as_ref()),
            other => Err(DownloadError::Unexpected(other.as_u16())),
        }
    }

    async fn fetch_torrent(&self, url: &str) -> Result<Vec<u8>, DownloadError> {
        self.http.fetch_via_limiter(url).await
    }

    async fn test_connection(&self) -> Result<(), DownloadError> {
        // GET the ruTorrent root. A 200 proves reachability *and* that the
        // credentials are accepted; a 401 means they're wrong (after a digest
        // retry, if the server asked for digest).
        let resp = self.http.send_authed(Method::GET, "/", |rb| rb).await?;
        classify_test_status(resp.status().as_u16())
    }
}

/// rTorrent XML-RPC client (ruTorrent's `httprpc` plugin or a bare `RPC2`
/// mount). What Prowlarr/Sonarr/Radarr use. Add-only for now — the same wire is
/// the door to download lifecycle (progress, removal) later.
pub struct RtorrentXmlRpcClient {
    http: AuthedHttp,
    /// XML-RPC endpoint path relative to `base_url`, leading-slash normalized
    /// (e.g. `/plugins/httprpc/action.php` or `/RPC2`).
    endpoint: String,
}

impl RtorrentXmlRpcClient {
    /// `url_path` is the XML-RPC endpoint relative to `base_url`; defaults to
    /// `RPC2` (the conventional bare-rTorrent mount) when omitted.
    pub fn new(
        base_url: impl Into<String>,
        url_path: Option<String>,
        username: Option<String>,
        password: Option<String>,
        timeout: Duration,
        limiter: Arc<HttpLimiter>,
    ) -> Result<Self, DownloadError> {
        let path = url_path.unwrap_or_else(|| "RPC2".to_string());
        Ok(Self {
            http: AuthedHttp::new(base_url, username, password, timeout, limiter)?,
            endpoint: format!("/{}", path.trim_matches('/')),
        })
    }

    /// Issue one XML-RPC `methodCall`. Returns the raw response body on success;
    /// a transport/HTTP failure or an XML-RPC `<fault>` is an error.
    async fn call(&self, method_name: &str, params: String) -> Result<String, DownloadError> {
        let body = format!(
            r#"<?xml version="1.0"?><methodCall><methodName>{method_name}</methodName><params>{params}</params></methodCall>"#
        );
        let resp = self
            .http
            .send_authed(Method::POST, &self.endpoint, |rb| {
                rb.header(reqwest::header::CONTENT_TYPE, "text/xml")
                    .body(body.clone())
            })
            .await?;
        let status = resp.status();
        if status != StatusCode::OK {
            return Err(DownloadError::Unexpected(status.as_u16()));
        }
        let text = resp.text().await?;
        match parse_xmlrpc_fault(&text) {
            Some(msg) => Err(DownloadError::Rejected(msg)),
            None => Ok(text),
        }
    }
}

#[async_trait::async_trait]
impl DownloadClient for RtorrentXmlRpcClient {
    async fn add(&self, req: AddRequest) -> Result<(), DownloadError> {
        // load.* takes an empty target, the source, then a variadic list of
        // commands run on the new download. Map start/source to the right verb.
        let mut params = xml_string_param("");
        let method_name = match (&req.source, req.start) {
            (AddSource::Torrent { bytes, .. }, true) => {
                params.push_str(&xml_base64_param(bytes));
                "load.raw_start"
            }
            (AddSource::Torrent { bytes, .. }, false) => {
                params.push_str(&xml_base64_param(bytes));
                "load.raw"
            }
            (AddSource::Magnet(url), true) => {
                params.push_str(&xml_string_param(url));
                "load.start"
            }
            (AddSource::Magnet(url), false) => {
                params.push_str(&xml_string_param(url));
                "load.normal"
            }
        };
        if let Some(label) = req.label.as_deref().filter(|l| !l.is_empty()) {
            params.push_str(&xml_string_param(&format!("d.custom1.set={label}")));
        }
        if let Some(dir) = req.dir.as_deref().filter(|d| !d.is_empty()) {
            params.push_str(&xml_string_param(&format!("d.directory.set={dir}")));
        }
        self.call(method_name, params).await.map(|_| ())
    }

    async fn fetch_torrent(&self, url: &str) -> Result<Vec<u8>, DownloadError> {
        self.http.fetch_via_limiter(url).await
    }

    async fn test_connection(&self) -> Result<(), DownloadError> {
        // A non-fault 200 to system.client_version proves reachability + auth.
        self.call("system.client_version", String::new())
            .await
            .map(|_| ())
    }
}

/// XML-escape a string for placement inside a `<string>` element.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// `<param><value><string>…</string></value></param>` for a string argument.
fn xml_string_param(s: &str) -> String {
    format!(
        "<param><value><string>{}</string></value></param>",
        xml_escape(s)
    )
}

/// `<param><value><base64>…</base64></value></param>` for raw `.torrent` bytes.
fn xml_base64_param(bytes: &[u8]) -> String {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("<param><value><base64>{b64}</base64></value></param>")
}

/// Extract an XML-RPC `<fault>`'s `faultString`, or `None` when the response is
/// a normal `methodResponse`. rTorrent answers with either a fault or a plain
/// response, so the presence of `<fault` is the reliable error signal; the
/// message is best-effort.
fn parse_xmlrpc_fault(xml: &str) -> Option<String> {
    if !xml.contains("<fault") {
        return None;
    }
    let msg = xml
        .split("faultString")
        .nth(1)
        .and_then(|rest| rest.split("<string>").nth(1))
        .and_then(|rest| rest.split("</string>").next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "rTorrent XML-RPC fault".to_string());
    Some(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUCCESS: &str = include_str!("../tests/fixtures/addtorrent_success.json");

    fn torrent_req() -> AddRequest {
        AddRequest {
            source: AddSource::Torrent {
                bytes: vec![1, 2, 3, 4],
                file_name: "release.torrent".to_string(),
            },
            label: Some("manga".to_string()),
            start: true,
            dir: None,
        }
    }

    #[test]
    fn assembles_torrent_file_path() {
        let fields = build_add_fields(&torrent_req());
        assert_eq!(
            fields,
            vec![
                FormField::File(
                    "torrent_file".into(),
                    "release.torrent".into(),
                    vec![1, 2, 3, 4]
                ),
                FormField::Text("label".into(), "manga".into()),
                FormField::Text("json".into(), "1".into()),
            ]
        );
    }

    #[test]
    fn assembles_magnet_path() {
        let req = AddRequest {
            source: AddSource::Magnet("magnet:?xt=urn:btih:abc".to_string()),
            label: None,
            start: true,
            dir: None,
        };
        let fields = build_add_fields(&req);
        assert_eq!(
            fields,
            vec![
                FormField::Text("url".into(), "magnet:?xt=urn:btih:abc".into()),
                FormField::Text("json".into(), "1".into()),
            ]
        );
    }

    #[test]
    fn emits_start_stopped_only_when_not_starting() {
        // start = true: no torrents_start_stopped field.
        let started = build_add_fields(&torrent_req());
        assert!(
            !started
                .iter()
                .any(|f| matches!(f, FormField::Text(k, _) if k == "torrents_start_stopped"))
        );

        // start = false: the field is present (presence == add-stopped).
        let mut req = torrent_req();
        req.start = false;
        let stopped = build_add_fields(&req);
        assert!(stopped.contains(&FormField::Text(
            "torrents_start_stopped".into(),
            "1".into()
        )));
    }

    #[test]
    fn includes_dir_edit_when_set_and_drops_empties() {
        let mut req = torrent_req();
        req.dir = Some("/downloads/manga".into());
        let fields = build_add_fields(&req);
        assert!(fields.contains(&FormField::Text(
            "dir_edit".into(),
            "/downloads/manga".into()
        )));

        // Empty label / dir are dropped rather than sent blank.
        req.label = Some(String::new());
        req.dir = Some(String::new());
        let fields = build_add_fields(&req);
        assert!(
            !fields
                .iter()
                .any(|f| matches!(f, FormField::Text(k, _) if k == "label" || k == "dir_edit"))
        );
    }

    #[test]
    fn parses_success_fixture() {
        assert!(parse_add_response(SUCCESS.as_bytes()).is_ok());
    }

    #[test]
    fn parses_error_result_as_rejected() {
        let body = br#"{"result":"Failed to add torrent"}"#;
        let err = parse_add_response(body).unwrap_err();
        assert!(matches!(err, DownloadError::Rejected(msg) if msg == "Failed to add torrent"));
    }

    #[test]
    fn missing_result_is_rejected() {
        let err = parse_add_response(b"{}").unwrap_err();
        assert!(matches!(err, DownloadError::Rejected(_)));
    }

    #[test]
    fn non_json_body_is_decode_error() {
        let err = parse_add_response(b"<html>nope</html>").unwrap_err();
        assert!(matches!(err, DownloadError::Decode(_)));
    }

    #[test]
    fn classify_test_status_treats_200_as_reachable() {
        assert!(classify_test_status(200).is_ok());
    }

    #[test]
    fn classify_test_status_surfaces_non_200() {
        // 401 (bad credentials) and 5xx (down) both report the code.
        assert!(matches!(
            classify_test_status(401),
            Err(DownloadError::Unexpected(401))
        ));
        assert!(matches!(
            classify_test_status(503),
            Err(DownloadError::Unexpected(503))
        ));
    }

    #[test]
    fn maps_reqwest_methods_to_digest_methods() {
        assert_eq!(digest_method(&Method::GET), digest_auth::HttpMethod::GET);
        assert_eq!(digest_method(&Method::POST), digest_auth::HttpMethod::POST);
    }

    #[test]
    fn xml_escape_handles_special_chars() {
        assert_eq!(
            xml_escape("a&b<c>d\"e'f"),
            "a&amp;b&lt;c&gt;d&quot;e&apos;f"
        );
    }

    #[test]
    fn xml_base64_param_encodes_bytes() {
        // "hi" -> base64 "aGk=".
        assert_eq!(
            xml_base64_param(b"hi"),
            "<param><value><base64>aGk=</base64></value></param>"
        );
    }

    #[test]
    fn parse_xmlrpc_fault_ignores_a_normal_response() {
        let ok = r#"<?xml version="1.0"?><methodResponse><params><param><value><string>0.9.8</string></value></param></params></methodResponse>"#;
        assert!(parse_xmlrpc_fault(ok).is_none());
    }

    #[test]
    fn parse_xmlrpc_fault_extracts_the_fault_string() {
        let fault = r#"<?xml version="1.0"?><methodResponse><fault><value><struct><member><name>faultCode</name><value><i4>-501</i4></value></member><member><name>faultString</name><value><string>Could not create download</string></value></member></struct></value></fault></methodResponse>"#;
        assert_eq!(
            parse_xmlrpc_fault(fault).as_deref(),
            Some("Could not create download")
        );
    }

    /// End-to-end probe against a real ruTorrent. No-op unless
    /// `TD_TEST_RUTORRENT_URL` is set, so it's inert in CI; run locally with
    /// the seedbox URL + credentials to confirm the Basic/Digest negotiation:
    ///   TD_TEST_RUTORRENT_URL=… TD_TEST_RUTORRENT_USER=… TD_TEST_RUTORRENT_PASS=… \
    ///     cargo test -p td-download live_connection -- --nocapture
    #[tokio::test]
    async fn live_connection() {
        let Ok(base) = std::env::var("TD_TEST_RUTORRENT_URL") else {
            return;
        };
        let client = RuTorrentClient::new(
            base,
            std::env::var("TD_TEST_RUTORRENT_USER").ok(),
            std::env::var("TD_TEST_RUTORRENT_PASS").ok(),
            Duration::from_secs(15),
            HttpLimiter::no_limit(),
        )
        .unwrap();
        client
            .test_connection()
            .await
            .expect("connection test should succeed against the configured ruTorrent");
    }

    /// End-to-end XML-RPC probe. No-op unless both `TD_TEST_RUTORRENT_URL` and
    /// `TD_TEST_RUTORRENT_XMLRPC_PATH` are set. Confirms `system.client_version`
    /// over the httprpc/RPC2 endpoint (with Basic/Digest negotiation).
    #[tokio::test]
    async fn live_xmlrpc_connection() {
        let (Ok(base), Ok(path)) = (
            std::env::var("TD_TEST_RUTORRENT_URL"),
            std::env::var("TD_TEST_RUTORRENT_XMLRPC_PATH"),
        ) else {
            return;
        };
        let client = RtorrentXmlRpcClient::new(
            base,
            Some(path),
            std::env::var("TD_TEST_RUTORRENT_USER").ok(),
            std::env::var("TD_TEST_RUTORRENT_PASS").ok(),
            Duration::from_secs(15),
            HttpLimiter::no_limit(),
        )
        .unwrap();
        client
            .test_connection()
            .await
            .expect("XML-RPC system.client_version should succeed");
    }
}
