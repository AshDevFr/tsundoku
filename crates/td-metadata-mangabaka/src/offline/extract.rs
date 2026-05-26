//! Extract `series.sqlite` from a downloaded `series.sqlite.tar.gz`.
//!
//! The published archive contains exactly one entry (`series` / `series.sqlite`,
//! depending on packaging). We unpack the first regular file we find and write
//! it to `target_path`. Decompression is streamed via `flate2`; the tarball
//! traversal is synchronous (the `tar` crate has no async API) but happens
//! on a `tokio::task::spawn_blocking` so the runtime stays responsive.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use anyhow::{Context, anyhow};
use flate2::read::GzDecoder;
use tar::Archive;

/// Extract `archive_path` (a `.tar.gz`) and write the first regular file
/// to `target_path`. Returns the number of uncompressed bytes written.
pub async fn extract_dump(
    archive_path: impl AsRef<Path>,
    target_path: impl AsRef<Path>,
) -> anyhow::Result<u64> {
    let archive = archive_path.as_ref().to_path_buf();
    let target = target_path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || extract_blocking(&archive, &target))
        .await
        .context("extraction task panicked")?
}

fn extract_blocking(archive_path: &Path, target_path: &Path) -> anyhow::Result<u64> {
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let f =
        File::open(archive_path).with_context(|| format!("opening {}", archive_path.display()))?;
    let gz = GzDecoder::new(BufReader::new(f));
    let mut archive = Archive::new(gz);

    for entry in archive.entries().context("reading tar entries")? {
        let mut entry = entry.context("iterating tar entries")?;
        let path = entry.path()?.to_path_buf();
        let header = entry.header().clone();
        // Only consider regular files; skip directories, symlinks, etc.
        if header.entry_type().is_file() {
            tracing::info!(
                target_entry = %path.display(),
                size = header.size().unwrap_or(0),
                "extracting dump entry"
            );
            let out = File::create(target_path)
                .with_context(|| format!("creating {}", target_path.display()))?;
            let mut writer = BufWriter::new(out);
            let mut buf = vec![0u8; 1 << 20]; // 1 MiB chunks
            let mut total: u64 = 0;
            loop {
                let n = entry.read(&mut buf).context("reading from tar entry")?;
                if n == 0 {
                    break;
                }
                writer
                    .write_all(&buf[..n])
                    .with_context(|| format!("writing to {}", target_path.display()))?;
                total += n as u64;
            }
            writer.flush().context("flushing extracted file")?;
            return Ok(total);
        }
    }
    Err(anyhow!(
        "archive {} contained no regular file",
        archive_path.display()
    ))
}

/// Helper: build a tarball alongside a fixture so tests can exercise the
/// extractor end-to-end. Only used in test/dev code.
#[cfg(test)]
pub fn pack_for_test(file_path: &Path, archive_path: &Path) -> anyhow::Result<PathBuf> {
    use flate2::Compression;
    use flate2::write::GzEncoder;

    let f = File::create(archive_path)?;
    let gz = GzEncoder::new(BufWriter::new(f), Compression::default());
    let mut tar = tar::Builder::new(gz);
    let mut src = File::open(file_path)?;
    tar.append_file(
        file_path.file_name().unwrap_or_else(|| "series".as_ref()),
        &mut src,
    )?;
    let gz = tar.into_inner()?;
    gz.finish()?;
    Ok(archive_path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[tokio::test]
    async fn extract_writes_payload_to_target() {
        let dir = TempDir::new().unwrap();
        let payload_path = dir.path().join("payload.bin");
        let mut f = File::create(&payload_path).unwrap();
        let payload = b"hello world hello world";
        f.write_all(payload).unwrap();
        drop(f);

        let archive_path = dir.path().join("payload.tar.gz");
        pack_for_test(&payload_path, &archive_path).unwrap();

        let extracted = dir.path().join("out.bin");
        let n = extract_dump(&archive_path, &extracted).await.unwrap();
        assert_eq!(n, payload.len() as u64);
        let read_back = std::fs::read(&extracted).unwrap();
        assert_eq!(read_back, payload);
    }
}
