//! Every match folder under a root such as the game's `MatchReplay`, read
//! together: which game session recorded what, recordings that were started
//! and never saved, copies of the same round, and temporary recordings left
//! behind.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::decoder::ParserInfo;
use crate::error::{Error, Result};
use crate::file::{self, FileInfo, MatchFolderName, TempRecording};
use crate::format::GameVersion;
use crate::matches::{
    FolderReport, IdGap, MIN_ROUND_IDS, Match, file_name, find_match_folders, unused_ids,
};
use crate::round::{ReadMode, Round};
use crate::summary::MatchSummary;

/// What the folders under a root hold.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub root: String,
    pub folders: Vec<LibraryFolder>,
    /// One per run of the game, from the folder names' process ids.
    pub sessions: Vec<Session>,
    /// Rounds found more than once.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub duplicates: Vec<Duplicate>,
    /// `.tmprec` files under the root, in `DissectTmp`, and next to
    /// `MatchReplay` in the game folder.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub temporary: Vec<TempRecording>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// One match folder in the library.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFolder {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<MatchSummary>,
    #[serde(flatten)]
    pub folder: FolderReport,
    pub round_list: Vec<LibraryRound>,
    /// Why the folder could not be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One round file in the library.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRound {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<FileInfo>,
    /// Round number from 1, from the header.
    pub round: u32,
    #[serde(rename = "matchID")]
    pub match_id: String,
    /// When recording started, UTC (Y11S3+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    /// The header's `datetime`: the recording PC's local time.
    pub local_time: String,
    pub version: GameVersion,
    pub parser: ParserInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_id: Option<u32>,
    /// Whether the game finished writing the file (Y8S4+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complete: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gaps: Option<usize>,
}

/// The rounds one run of the game recorded. The game numbers every stream it
/// records from one counter that starts at 0, so the rounds of a session
/// line up by `recordingId`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// Windows process id of the game, from the folder names.
    pub process_id: u32,
    /// Match folders, oldest first.
    pub folders: Vec<String>,
    pub rounds: usize,
    /// Ids start at 0 when the game starts, so a higher first id means the
    /// run recorded earlier rounds that are not here, at 9 to 12 ids each.
    pub first_recording_id: u32,
    pub last_recording_id: u32,
    /// Ids no round here used, between two rounds of the session. Fewer than
    /// 9 cannot be a saved round: a recording was started and never saved.
    /// More can also be rounds or matches no longer here.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub id_gaps: Vec<IdGap>,
}

/// The same round found more than once.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Duplicate {
    /// `sameFile`: byte-identical copies. `sameRound`: different files of
    /// the same round of the same match, such as another player's recording.
    pub kind: DuplicateKind,
    /// The SHA-256, or `<matchID> R<round>`.
    pub key: String,
    pub files: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DuplicateKind {
    SameFile,
    SameRound,
}

/// A round as the session analysis sees it.
#[derive(Clone, Debug)]
struct Recording {
    file: String,
    id: u32,
    streams: usize,
}

/// A game-named folder and its rounds' recordings, for [`sessions`].
#[derive(Clone, Debug)]
struct SessionFolder {
    folder: String,
    name: MatchFolderName,
    recordings: Vec<Recording>,
}

/// Groups folders by process id and splits a group where the ids start
/// over: Windows reuses process ids, so one id can name two runs.
fn sessions(mut folders: Vec<SessionFolder>) -> Vec<Session> {
    folders.sort_by(|a, b| {
        (a.name.process_id, &a.name.local_time).cmp(&(b.name.process_id, &b.name.local_time))
    });
    let mut out: Vec<Session> = Vec::new();
    // The last recording seen, with its process id.
    let mut last: Option<(u32, Recording)> = None;
    for f in folders {
        let pid = f.name.process_id;
        let mut recordings = f.recordings;
        recordings.sort_by_key(|r| r.id);
        let Some(first) = recordings.first() else {
            continue;
        };
        let continues = last
            .as_ref()
            .is_some_and(|(p, prev)| *p == pid && first.id > prev.id);
        if !continues {
            out.push(Session {
                process_id: pid,
                folders: Vec::new(),
                rounds: 0,
                first_recording_id: first.id,
                last_recording_id: first.id,
                id_gaps: Vec::new(),
            });
            last = None;
        }
        let s = out.last_mut().expect("pushed above");
        s.folders.push(f.folder);
        for r in recordings {
            if let Some(ids) = last
                .as_ref()
                .and_then(|(_, prev)| unused_ids(prev.id, prev.streams, r.id))
            {
                let (_, prev) = last.as_ref().expect("checked above");
                s.id_gaps.push(IdGap {
                    after: prev.file.clone(),
                    before: r.file.clone(),
                    ids,
                });
            }
            s.rounds += 1;
            s.last_recording_id = r.id;
            last = Some((pid, r));
        }
    }
    out
}

/// Reads every match folder under `root`. Fails only when there is none.
pub fn scan(root: &Path, mode: ReadMode) -> Result<Library> {
    let dirs = find_match_folders(root)?;
    if dirs.is_empty() {
        return Err(Error::InvalidFolder);
    }
    let mut lib = Library {
        root: root.display().to_string(),
        ..Library::default()
    };
    let mut session_folders = Vec::new();
    let mut by_hash: HashMap<String, Vec<String>> = HashMap::new();
    let mut by_round: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for dir in dirs {
        let m = match Match::open_with(&dir, mode) {
            Ok(m) => m,
            Err(e) => {
                lib.folders.push(LibraryFolder {
                    folder: FolderReport {
                        path: dir.display().to_string(),
                        ..FolderReport::default()
                    },
                    error: Some(e.to_string()),
                    ..LibraryFolder::default()
                });
                continue;
            }
        };
        let folder = m.folder.clone().expect("open_with sets folder");
        let dir_name = dir
            .file_name()
            .map_or_else(|| folder.path.clone(), |n| n.to_string_lossy().into_owned());
        if let Some(name) = folder.name.clone() {
            session_folders.push(SessionFolder {
                folder: dir_name.clone(),
                name,
                recordings: m
                    .rounds
                    .iter()
                    .filter_map(|r| {
                        let c = r.container.as_ref()?;
                        Some(Recording {
                            file: file_name(r),
                            id: c.recording_id?,
                            streams: c.streams.len(),
                        })
                    })
                    .collect(),
            });
        }
        for r in &m.rounds {
            let Some(f) = &r.file else { continue };
            by_hash
                .entry(f.sha256.clone())
                .or_default()
                .push(f.path.clone());
            let key = format!("{} R{:02}", r.header.match_id, r.header.round_number + 1);
            by_round
                .entry(key)
                .or_default()
                .push((f.path.clone(), f.sha256.clone()));
        }
        lib.temporary
            .extend(folder.temporary.iter().map(|t| TempRecording {
                file: dir.join(&t.file).display().to_string(),
                ..t.clone()
            }));
        lib.folders.push(LibraryFolder {
            summary: m.summary(),
            round_list: m.rounds.iter().map(library_round).collect(),
            folder,
            error: None,
        });
    }
    lib.sessions = sessions(session_folders);
    lib.duplicates = duplicates(by_hash, by_round);
    for dir in temporary_dirs(root) {
        lib.temporary.extend(temporary_in(&dir));
    }
    lib.temporary.sort_by(|a, b| a.file.cmp(&b.file));
    lib.temporary.dedup_by(|a, b| a.file == b.file);
    lib.warnings = library_warnings(&lib);
    Ok(lib)
}

fn library_round(r: &Round) -> LibraryRound {
    let h = &r.header;
    LibraryRound {
        file: r.file.clone(),
        round: h.round_number + 1,
        match_id: h.match_id.clone(),
        start_time: h
            .start_time
            .map(|t| t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()),
        local_time: h.timestamp.format("%Y-%m-%dT%H:%M:%S").to_string(),
        version: r.version.clone(),
        parser: r.parser.clone(),
        recording_id: r.container.as_ref().and_then(|c| c.recording_id),
        complete: r.container.as_ref().map(|c| c.complete),
        frames: r.timing.as_ref().map(|t| t.frames),
        sample_rate: r.timing.as_ref().map(|t| t.sample_rate),
        gaps: r.timing.as_ref().map(|t| t.gaps.len()),
    }
}

/// Byte-identical files, and different files of one round of one match.
fn duplicates(
    by_hash: HashMap<String, Vec<String>>,
    by_round: HashMap<String, Vec<(String, String)>>,
) -> Vec<Duplicate> {
    let mut out: Vec<Duplicate> = by_hash
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(key, mut files)| {
            files.sort();
            Duplicate {
                kind: DuplicateKind::SameFile,
                key,
                files,
            }
        })
        .collect();
    for (key, copies) in by_round {
        let mut hashes: Vec<&str> = copies.iter().map(|c| c.1.as_str()).collect();
        hashes.sort_unstable();
        hashes.dedup();
        if hashes.len() > 1 {
            let mut files: Vec<String> = copies.into_iter().map(|c| c.0).collect();
            files.sort();
            out.push(Duplicate {
                kind: DuplicateKind::SameRound,
                key,
                files,
            });
        }
    }
    out.sort_by(|a, b| a.files.cmp(&b.files));
    out
}

/// Where temporary recordings may sit: the root, its `DissectTmp`, and for a
/// `MatchReplay` root, the game folder around it and that folder's
/// `DissectTmp`. Players found them in the game folder (2022); current
/// installs have an empty `DissectTmp` next to `MatchReplay`.
fn temporary_dirs(root: &Path) -> Vec<std::path::PathBuf> {
    let mut dirs = vec![root.to_path_buf(), root.join("DissectTmp")];
    let is_match_replay = root
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("MatchReplay"));
    if let Some(game) = root.parent().filter(|_| is_match_replay) {
        dirs.push(game.to_path_buf());
        dirs.push(game.join("DissectTmp"));
    }
    dirs
}

fn temporary_in(dir: &Path) -> Vec<TempRecording> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()) && file::is_temporary(&e.path()))
        .map(|e| {
            let mut t = file::parse_temp_name(&e.file_name().to_string_lossy());
            t.file = e.path().display().to_string();
            t.size = e.metadata().ok().map(|m| m.len());
            t
        })
        .collect()
}

fn library_warnings(lib: &Library) -> Vec<String> {
    let mut w = Vec::new();
    let failed = lib.folders.iter().filter(|f| f.error.is_some()).count();
    if failed > 0 {
        w.push(format!("{failed} folders could not be read"));
    }
    let incomplete: usize = lib.folders.iter().map(|f| f.folder.incomplete.len()).sum();
    if incomplete > 0 {
        w.push(format!(
            "{incomplete} round files were not finished by the game; see each folder's `incomplete`"
        ));
    }
    for s in &lib.sessions {
        for g in &s.id_gaps {
            w.push(if g.ids < MIN_ROUND_IDS {
                format!(
                    "game process {}: a recording was started between {} and {} and never saved",
                    s.process_id, g.after, g.before
                )
            } else {
                format!(
                    "game process {}: {} ids unused between {} and {}: rounds no longer here, or recordings never saved",
                    s.process_id, g.ids, g.after, g.before
                )
            });
        }
    }
    if !lib.duplicates.is_empty() {
        w.push(format!(
            "{} rounds found more than once",
            lib.duplicates.len()
        ));
    }
    if !lib.temporary.is_empty() {
        w.push(format!(
            "{} temporary recordings left behind, from recordings that never became a round file",
            lib.temporary.len()
        ));
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str, pid: u32, time: &str, recordings: &[(u32, u32)]) -> SessionFolder {
        SessionFolder {
            folder: name.into(),
            name: MatchFolderName {
                local_time: time.into(),
                process_id: pid,
            },
            recordings: recordings
                .iter()
                .enumerate()
                .map(|(i, &(id, streams))| Recording {
                    file: format!("{name}-R{:02}.rec", i + 1),
                    id,
                    streams: streams as usize,
                })
                .collect(),
        }
    }

    #[test]
    fn a_session_spots_a_recording_never_saved_between_folders() {
        let s = sessions(vec![
            folder("B", 7, "2026-09-23T00:18:22", &[(12, 10), (23, 10)]),
            folder("A", 7, "2026-09-23T00:12:30", &[(0, 10)]),
        ]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].folders, ["A", "B"]);
        assert_eq!((s[0].first_recording_id, s[0].last_recording_id), (0, 23));
        assert_eq!(
            s[0].id_gaps,
            [IdGap {
                after: "A-R01.rec".into(),
                before: "B-R01.rec".into(),
                ids: 1
            }]
        );
    }

    #[test]
    fn a_session_that_starts_late_counts_the_ids_before_it() {
        let s = sessions(vec![folder("C", 9, "2026-09-20T00:32:29", &[(156, 10)])]);
        assert_eq!(s[0].first_recording_id, 156);
        assert!(s[0].id_gaps.is_empty());
    }

    #[test]
    fn a_reused_process_id_starts_a_new_session() {
        let s = sessions(vec![
            folder("Old", 5, "2026-09-01T20:00:00", &[(0, 10), (11, 10)]),
            folder("New", 5, "2026-09-28T21:00:00", &[(0, 10)]),
        ]);
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].folders, ["New"]);
        assert!(s.iter().all(|s| s.id_gaps.is_empty()));
    }

    #[test]
    fn garbage_recording_ids_do_not_overflow_a_session() {
        let s = sessions(vec![folder(
            "D",
            3,
            "2026-09-20T00:00:00",
            &[(u32::MAX - 1, 10), (u32::MAX, 10)],
        )]);
        assert!(s[0].id_gaps.is_empty());
    }
}
