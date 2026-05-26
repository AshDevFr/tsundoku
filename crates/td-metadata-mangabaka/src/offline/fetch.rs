//! Stream a MangaBaka dump tarball to disk, with optional SHA-1
//! verification against the published `.sha1` sidecar.
//!
//! The dump is large (~500 MB compressed). We use reqwest's byte-stream
//! interface so memory stays bounded regardless of file size. The
//! downloaded bytes are SHA-1'd in-flight so a corrupt file fails fast
//! without a second pass over disk.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use futures::StreamExt;
use sha1::{Digest, Sha1};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

/// Outcome of a download.
#[derive(Debug)]
pub struct DownloadOutcome {
    pub path: PathBuf,
    pub bytes_downloaded: u64,
    pub sha1_hex: String,
}

/// Stream `url` into `target_path`, returning the byte count and the
/// computed SHA-1 hex. Creates the parent directory if needed. Any
/// existing file at `target_path` is overwritten.
pub async fn download(
    http: &reqwest::Client,
    url: &str,
    target_path: impl AsRef<Path>,
) -> anyhow::Result<DownloadOutcome> {
    let target_path = target_path.as_ref().to_path_buf();
    if let Some(parent) = target_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let resp = http
        .get(url)
        .send()
        .await
        .with_context(|| format!("requesting {url}"))?;
    let resp = resp
        .error_for_status()
        .with_context(|| format!("downloading {url}"))?;

    let mut file = File::create(&target_path)
        .await
        .with_context(|| format!("creating {}", target_path.display()))?;
    let mut hasher = Sha1::new();
    let mut bytes_downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("streaming chunk from {url}"))?;
        hasher.update(&chunk);
        bytes_downloaded += chunk.len() as u64;
        file.write_all(&chunk)
            .await
            .with_context(|| format!("writing to {}", target_path.display()))?;
    }
    file.flush().await.context("flushing dump file")?;
    file.sync_all().await.context("fsync dump file")?;

    Ok(DownloadOutcome {
        path: target_path,
        bytes_downloaded,
        sha1_hex: hex::encode(hasher.finalize()),
    })
}

/// Fetch the `.sha1` sidecar published next to the dump and return the
/// expected hex digest. The sidecar is whitespace-delimited:
/// `<hex>  <filename>`; we accept either format.
pub async fn fetch_expected_sha1(http: &reqwest::Client, url: &str) -> anyhow::Result<String> {
    let body = http
        .get(url)
        .send()
        .await
        .with_context(|| format!("requesting sidecar {url}"))?
        .error_for_status()
        .with_context(|| format!("downloading sidecar {url}"))?
        .text()
        .await
        .with_context(|| format!("reading sidecar body {url}"))?;
    let hex = body
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("sidecar {url} returned no hex digest"))?
        .to_string();
    if hex.len() != 40 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!("sidecar {url} returned non-hex content: {body:?}"));
    }
    Ok(hex.to_lowercase())
}

/// Convenience: derive the `.sha1` sidecar URL from a tarball URL.
pub fn sha1_sidecar_url(tarball_url: &str) -> String {
    format!("{tarball_url}.sha1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_url_appends_sha1() {
        assert_eq!(
            sha1_sidecar_url("https://api.mangabaka.dev/v1/database/series.sqlite.tar.gz"),
            "https://api.mangabaka.dev/v1/database/series.sqlite.tar.gz.sha1"
        );
    }
}
