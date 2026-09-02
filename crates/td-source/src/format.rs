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
    /// Audiobook containers. Nyaa carries light-novel audiobooks alongside
    /// the e-book rips, and the format-to-kind rules need to see them as
    /// novel formats rather than falling through to `Other`.
    M4b,
    Mp3,
    M4a,
    Flac,
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
            Format::M4b => "m4b",
            Format::Mp3 => "mp3",
            Format::M4a => "m4a",
            Format::Flac => "flac",
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
            "m4b" => Some(Format::M4b),
            "mp3" => Some(Format::Mp3),
            "m4a" => Some(Format::M4a),
            "flac" => Some(Format::Flac),
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

/// Kind hints carried by the release *title* rather than its file list.
/// Returned in the same string vocabulary as [`Format::as_str`] so the
/// format-to-kind rules can list them alongside real extensions (the
/// default rule pairs `"audiobook"` with the audio containers). Hints are
/// never persisted as `release_formats` rows: they describe the upload's
/// self-declared nature, not a file that exists in the torrent.
///
/// Today the only hint is `audiobook`: most audiobook uploads carry the
/// word in their title (`(Audiobook)`, `[Audiobook]`, bare), and the file
/// list can be missing when the detail fetch is off or failed, so the
/// title is the more reliable signal of the two.
pub fn detect_title_hints(title: &str) -> Vec<String> {
    let has_audiobook = title
        .split(|c: char| !c.is_alphanumeric())
        .any(|tok| tok.eq_ignore_ascii_case("audiobook") || tok.eq_ignore_ascii_case("audiobooks"));
    if has_audiobook {
        vec!["audiobook".to_string()]
    } else {
        Vec::new()
    }
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
    fn detects_audiobook_formats_as_typed_variants() {
        // Audiobook uploads are audio containers, never book archives. They
        // need typed variants so the default format-to-kind rule can steer
        // them to `novel` series instead of the same-titled manga.
        let files = s(&[
            "Series v01.m4b",
            "Series v02/01.mp3",
            "Series v03.m4a",
            "Series v04.flac",
        ]);
        assert_eq!(
            detect_formats(&files),
            vec![Format::M4b, Format::Mp3, Format::M4a, Format::Flac]
        );
    }

    #[test]
    fn real_world_audiobook_upload_yields_only_m4b() {
        // Nyaa #2119814: one m4b plus cover art. The cover must not leak in.
        let files = s(&[
            "The Eminence in Shadow, Vol. 05 [Troglodyte].m4b",
            "cover-ln.jpg",
        ]);
        assert_eq!(detect_formats(&files), vec![Format::M4b]);
    }

    #[test]
    fn title_hints_flag_audiobook_keyword_in_any_wrapper() {
        for title in [
            "The Eminence in Shadow, Vol. 05 (Audiobook) [Troglodyte]",
            "Overlord Vol 1 [Audiobook]",
            "Mushoku Tensei audiobook v03",
            "Spice and Wolf AUDIOBOOKS 1-5",
        ] {
            assert_eq!(
                detect_title_hints(title),
                vec!["audiobook".to_string()],
                "{title}"
            );
        }
    }

    #[test]
    fn title_hints_ignore_unrelated_titles_and_substrings() {
        assert!(detect_title_hints("One Piece v01 (Digital)").is_empty());
        // Must be a whole word: a series called "Audiobookkeeper" is not a hint.
        assert!(detect_title_hints("Audiobookkeeper v01").is_empty());
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
            Format::M4b,
            Format::Mp3,
            Format::M4a,
            Format::Flac,
        ] {
            assert_eq!(Format::from_ext(known.as_str()).unwrap(), known);
        }
    }
}
