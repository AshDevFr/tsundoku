use axum::http::Uri;
use axum::response::IntoResponse;

#[cfg(feature = "embed-frontend")]
mod embedded {
    use axum::body::Body;
    use axum::http::{Uri, header};
    use axum::response::{IntoResponse, Response};
    use rust_embed::RustEmbed;

    // Path is relative to this crate's manifest dir (crates/tsundoku).
    // `../../web/dist` resolves to `web/dist` at the workspace root.
    #[derive(RustEmbed)]
    #[folder = "../../web/dist"]
    struct WebAssets;

    pub fn serve(uri: Uri) -> Response {
        let path = uri.path().trim_start_matches('/');
        let path = if path.is_empty() { "index.html" } else { path };

        // Fall back to index.html so the SPA router owns unknown paths.
        let (file, data) = match WebAssets::get(path) {
            Some(f) => (path.to_owned(), f.data.into_owned()),
            None => match WebAssets::get("index.html") {
                Some(f) => ("index.html".to_owned(), f.data.into_owned()),
                None => {
                    return (axum::http::StatusCode::NOT_FOUND, "frontend not found")
                        .into_response();
                }
            },
        };

        let mime = mime_guess::from_path(&file).first_or_octet_stream();
        // Hashed assets are immutable; HTML must revalidate so deploys are seen.
        let cache = if file.ends_with(".html") {
            "no-cache"
        } else {
            "public, max-age=31536000, immutable"
        };

        Response::builder()
            .header(header::CONTENT_TYPE, mime.as_ref())
            .header(header::CACHE_CONTROL, cache)
            .body(Body::from(data))
            .unwrap()
    }
}

#[cfg(feature = "embed-frontend")]
pub async fn serve_static(uri: Uri) -> impl IntoResponse {
    embedded::serve(uri)
}

#[cfg(not(feature = "embed-frontend"))]
pub async fn serve_static(_uri: Uri) -> impl IntoResponse {
    (
        axum::http::StatusCode::NOT_FOUND,
        "Frontend not embedded. Run the Vite dev server, or build with --features embed-frontend.",
    )
}
