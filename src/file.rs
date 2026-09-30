//! Facts about a replay file that are not inside it: identity for
//! deduplication, and whether it is a finished recording at all.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::census::hex;

/// Extension of a finished round recording.
pub const REPLAY_EXTENSION: &str = "rec";
/// Extension of the game's in-progress recording streams
/// (`*_FrameDataStream.tmprec`, `*_StaticData.tmprec`, `*_StreamInfo.tmprec`).
/// They are not replays and are never read.
pub const TEMP_EXTENSION: &str = "tmprec";

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub path: String,
    pub file_name: String,
    /// Bytes on disk.
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    /// SHA-256 of the file contents, hex. Same hash, same replay.
    pub sha256: String,
}

impl FileInfo {
    /// Describes `path`, whose contents are `raw`.
    pub fn new(path: &Path, raw: &[u8]) -> Self {
        let modified = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .map(|t| {
                DateTime::<Utc>::from(t)
                    .format("%Y-%m-%dT%H:%M:%SZ")
                    .to_string()
            });
        FileInfo {
            path: path.display().to_string(),
            file_name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            size: raw.len() as u64,
            modified,
            sha256: sha256(raw),
        }
    }
}

pub fn sha256(raw: &[u8]) -> String {
    hex(&Sha256::digest(raw))
}

fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension()
        .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(ext))
}

pub fn is_replay(path: &Path) -> bool {
    has_extension(path, REPLAY_EXTENSION)
}

pub fn is_temporary(path: &Path) -> bool {
    has_extension(path, TEMP_EXTENSION)
}

/// The round number in a round file name such as `R03.rec`, if it has one.
pub fn round_from_file_name(path: &Path) -> Option<u32> {
    let stem = path.file_stem()?.to_str()?;
    let digits = stem.strip_prefix(['R', 'r'])?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_files() {
        assert!(is_replay(Path::new("Match/R01.rec")));
        assert!(is_replay(Path::new("R01.REC")));
        assert!(!is_replay(Path::new("x_FrameDataStream.tmprec")));
        assert!(is_temporary(Path::new("x_FrameDataStream.tmprec")));
        assert_eq!(round_from_file_name(Path::new("R01.rec")), Some(1));
        assert_eq!(round_from_file_name(Path::new("R12.rec")), Some(12));
        assert_eq!(round_from_file_name(Path::new("custom_1.rec")), None);
        assert_eq!(round_from_file_name(Path::new("R.rec")), None);
    }

    #[test]
    fn hashes_contents() {
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
