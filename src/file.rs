//! Facts about a replay file that are not inside it: identity for
//! deduplication, and whether it is a finished recording at all.

use std::path::Path;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
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

/// The round number in a round file name: the game's own
/// `Match-2026-09-20_00-32-29-13160-R03.rec`, or a bare `R03.rec`.
pub fn round_from_file_name(path: &Path) -> Option<u32> {
    let stem = path.file_stem()?.to_str()?;
    round_suffix(stem.rsplit_once('-').map_or(stem, |(_, round)| round))
}

fn round_suffix(s: &str) -> Option<u32> {
    let digits = s.strip_prefix(['R', 'r'])?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The match folder a game-named round file belongs to: `Match-...-13160`
/// for `Match-...-13160-R03.rec`.
pub fn match_of_file_name(path: &Path) -> Option<&str> {
    let (folder, round) = path.file_stem()?.to_str()?.rsplit_once('-')?;
    (round_suffix(round).is_some() && parse_folder_name(folder).is_some()).then_some(folder)
}

/// What the game's name for a match folder says:
/// `Match-2026-09-20_00-32-29-13160`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MatchFolderName {
    /// When the game created the folder, in the recording PC's local time.
    pub local_time: String,
    /// Windows process id of the game that recorded the match. Matches
    /// played without restarting the game share it, and their recordings
    /// share one stream-id counter (`recordingId`).
    pub process_id: u32,
}

/// Parses a match folder name the game wrote: `Match-`, local date and time,
/// then the process id.
pub fn parse_folder_name(name: &str) -> Option<MatchFolderName> {
    let (time, pid) = name.strip_prefix("Match-")?.rsplit_once('-')?;
    let time = NaiveDateTime::parse_from_str(time, "%Y-%m-%d_%H-%M-%S").ok()?;
    Some(MatchFolderName {
        local_time: time.format("%Y-%m-%dT%H:%M:%S").to_string(),
        process_id: pid.parse().ok()?,
    })
}

/// A temporary recording (`.tmprec`) and what its name says. Players have
/// reported names such as
/// `P15440_50_Y2022_M1_D16_H23_M58_FrameDataStream.tmprec`: the game's
/// process id, a stream id from the counter `recordingId` comes from, the
/// local time, and which part of the recording the file holds
/// (`FrameDataStream`, `StaticData`, `StreamInfo`). The game deletes them
/// once a round is saved; ones left behind are recordings that never became
/// a round file.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TempRecording {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<u32>,
    /// Local time from the name, to the minute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// Reads what a temporary recording's name says. Names in another form keep
/// only `file`.
pub fn parse_temp_name(file_name: &str) -> TempRecording {
    let mut t = TempRecording {
        file: file_name.to_owned(),
        ..TempRecording::default()
    };
    let Some(stem) = Path::new(file_name).file_stem().and_then(|s| s.to_str()) else {
        return t;
    };
    let parts: Vec<&str> = stem.split('_').collect();
    let &[pid, id, year, month, day, hour, minute, kind] = parts.as_slice() else {
        return t;
    };
    let num = |s: &str, prefix: char| s.strip_prefix(prefix)?.parse::<u32>().ok();
    let (Some(pid), Ok(id), Some(year), Some(month), Some(day), Some(hour), Some(minute)) = (
        num(pid, 'P'),
        id.parse(),
        num(year, 'Y'),
        num(month, 'M'),
        num(day, 'D'),
        num(hour, 'H'),
        num(minute, 'M'),
    ) else {
        return t;
    };
    t.process_id = Some(pid);
    t.stream_id = Some(id);
    t.local_time = i32::try_from(year)
        .ok()
        .and_then(|y| NaiveDate::from_ymd_opt(y, month, day))
        .and_then(|d| d.and_hms_opt(hour, minute, 0))
        .map(|t| t.format("%Y-%m-%dT%H:%M").to_string());
    t.kind = Some(kind.to_owned());
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_round_from_the_game_file_name() {
        let name = Path::new("Match-2026-09-20_00-32-29-13160-R06.rec");
        assert_eq!(round_from_file_name(name), Some(6));
        assert_eq!(
            match_of_file_name(name),
            Some("Match-2026-09-20_00-32-29-13160")
        );
        assert_eq!(match_of_file_name(Path::new("R06.rec")), None);
    }

    #[test]
    fn reads_the_game_folder_name() {
        let f = parse_folder_name("Match-2026-09-20_00-32-29-13160").unwrap();
        assert_eq!(f.local_time, "2026-09-20T00:32:29");
        assert_eq!(f.process_id, 13160);
        assert_eq!(parse_folder_name("Match-2026-09-20_00-32-29"), None);
        assert_eq!(parse_folder_name("Y11S3"), None);
    }

    #[test]
    fn reads_a_temporary_recording_name() {
        let t = parse_temp_name("P15440_50_Y2022_M1_D16_H23_M58_FrameDataStream.tmprec");
        assert_eq!(t.process_id, Some(15440));
        assert_eq!(t.stream_id, Some(50));
        assert_eq!(t.local_time.as_deref(), Some("2022-01-16T23:58"));
        assert_eq!(t.kind.as_deref(), Some("FrameDataStream"));
        let other = parse_temp_name("leftover.tmprec");
        assert_eq!(
            (
                other.process_id,
                other.stream_id,
                other.local_time,
                other.kind
            ),
            (None, None, None, None)
        );
        assert_eq!(other.file, "leftover.tmprec");
    }

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
