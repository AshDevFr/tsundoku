//! Source-agnostic format detection from a release's file list.
//!
//! Detection is purely lexical: extension-only, lowercased, deduplicated.
//! That keeps the function pure (no I/O, no archive peeking) and matches
//! how `release-nyaa` already classifies file types. Archive contents are
//! intentionally not unpacked: if a `.zip` actually holds an `.epub`, the
//! release lands on `zip` here and `epub` only after the user (or a future
//! enhancement) classifies it explicitly.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// A recognized file format. Stored as a lowercase string in
/// `release_formats.format`, so the canonical spelling matters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Cbz,
    Cbr,
    Cb7,
    Cbt,
    Zip,
    Rar,
    SevenZ,
    Tar,
    Epub,
    Pdf,
    Mobi,
    Azw3,
    Other(String),
}

impl Format {
    /// Canonical string form. Round-trips with [`Format::from_ext`].
    pub fn as_str(&self) -> &str {
        match self {
            Format::Cbz => "cbz",
            Format::Cbr => "cbr",
            Format::Cb7 => "cb7",
            Format::Cbt => "cbt",
            Format::Zip => "zip",
            Format::Rar => "rar",
            Format::SevenZ => "7z",
            Format::Tar => "tar",
            Format::Epub => "epub",
            Format::Pdf => "pdf",
            Format::Mobi => "mobi",
            Format::Azw3 => "azw3",
            Format::Other(s) => s,
        }
    }

    /// Map a lowercased extension to a [`Format`]. Returns `None` for
    /// extensions we don't recognize at all (silent dotfiles, missing
    /// extension); returns `Format::Other` for extensions we accept but
    /// don't have a typed variant for.
    fn from_ext(ext: &str) -> Option<Self> {
        match ext {
            "cbz" => Some(Format::Cbz),
            "cbr" => Some(Format::Cbr),
            "cb7" => Some(Format::Cb7),
            "cbt" => Some(Format::Cbt),
            "zip" => Some(Format::Zip),
            "rar" => Some(Format::Rar),
            "7z" => Some(Format::SevenZ),
            "tar" => Some(Format::Tar),
            "epub" => Some(Format::Epub),
            "pdf" => Some(Format::Pdf),
            "mobi" => Some(Format::Mobi),
            "azw3" => Some(Format::Azw3),
            // Skip noise that shows up in torrent file lists but is not a
            // book/comic format: cover art, sample images, info text files,
            // checksums, dotfiles. The pre-pass in [`detect_formats`] also
            // skips entries without an extension at all.
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "txt" | "md" | "nfo" | "sfv"
            | "url" | "log" | "json" | "xml" | "yml" | "yaml" | "db" | "ds_store" => None,
            other if !other.is_empty() => Some(Format::Other(other.to_string())),
            _ => None,
        }
    }
}

/// Detect every distinct format present in a file list. Returns the set in
/// canonical sorted order so the persistence layer can write deterministic
/// `release_formats` rows.
pub fn detect_formats(files: &[String]) -> Vec<Format> {
    let mut out: BTreeSet<Format> = BTreeSet::new();
    for path in files {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(ext) = Path::new(trimmed).extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let ext = ext.to_ascii_lowercase();
        if let Some(fmt) = Format::from_ext(&ext) {
            out.insert(fmt);
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn detects_single_cbz() {
        let files = s(&["Some Series v01.cbz"]);
        assert_eq!(detect_formats(&files), vec![Format::Cbz]);
    }

    #[test]
    fn deduplicates_repeated_format() {
        let files = s(&["v01.cbz", "v02.cbz", "v03.cbz"]);
        assert_eq!(detect_formats(&files), vec![Format::Cbz]);
    }

    #[test]
    fn returns_canonical_sorted_order_for_multiple_formats() {
        // Mixed bag: cbr + epub + pdf in arbitrary input order.
        let files = s(&["read me first.pdf", "Series v01.epub", "Series v01.cbr"]);
        let got = detect_formats(&files);
        assert_eq!(got, vec![Format::Cbr, Format::Epub, Format::Pdf]);
    }

    #[test]
    fn ignores_cover_art_and_info_files() {
        // A typical comic-pack file list has art and metadata mixed with the
        // archive itself. Only the archive should land.
        let files = s(&[
            "cover.jpg",
            "Series v01.cbz",
            "info.txt",
            "checksums.sfv",
            ".DS_Store",
        ]);
        assert_eq!(detect_formats(&files), vec![Format::Cbz]);
    }

    #[test]
    fn ignores_extensionless_and_empty_entries() {
        let files = s(&["LICENSE", "", "  ", "Series v01.cbz"]);
        assert_eq!(detect_formats(&files), vec![Format::Cbz]);
    }

    #[test]
    fn unknown_but_plausible_extensions_land_as_other() {
        let files = s(&["weird.foo"]);
        assert_eq!(detect_formats(&files), vec![Format::Other("foo".into())]);
    }

    #[test]
    fn case_insensitive_extension_matching() {
        let files = s(&["Series v01.CBZ", "Series v02.Cbz"]);
        assert_eq!(detect_formats(&files), vec![Format::Cbz]);
    }

    #[test]
    fn format_as_str_round_trips_with_from_ext_on_known_variants() {
        for known in [
            Format::Cbz,
            Format::Cbr,
            Format::Cb7,
            Format::Cbt,
            Format::Zip,
            Format::Rar,
            Format::SevenZ,
            Format::Tar,
            Format::Epub,
            Format::Pdf,
            Format::Mobi,
            Format::Azw3,
        ] {
            assert_eq!(Format::from_ext(known.as_str()).unwrap(), known);
        }
    }
}
