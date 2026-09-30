//! Parsing a match folder: one `.rec` file per round (`R01.rec`, `R02.rec`, ...).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;

use crate::analytics::MatchAnalytics;
use crate::error::{Error, Result};
use crate::file::{self, FileInfo, MatchFolderName, TempRecording};
use crate::round::{ReadOptions, Round};
use crate::stats::{PlayerMatchStats, match_stats};
use crate::summary::MatchSummary;
use crate::types::version;

#[derive(Clone, Debug, Default)]
pub struct Match {
    /// Rounds in play order (by the header's round number).
    pub rounds: Vec<Round>,
    /// What the folder held and what is missing from it.
    pub folder: Option<FolderReport>,
}

/// A file in the folder that was not read as a round.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkippedFile {
    pub file: String,
    pub reason: String,
}

/// Stream ids no saved round used, between two rounds of a folder.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnsavedGap {
    /// The round file before the skipped ids, and the one after.
    pub after: String,
    pub before: String,
    /// How many ids were skipped. A recording takes one id when it starts
    /// and one per stream as they are created, so each unsaved recording
    /// accounts for at least one.
    pub ids: u32,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FolderReport {
    pub path: String,
    /// What the folder's name says, when the game named it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<MatchFolderName>,
    /// `.rec` files in the folder.
    pub round_files: usize,
    /// Rounds read, after dropping failures and duplicates.
    pub rounds_read: usize,
    /// Temporary `.tmprec` recordings, byte-identical copies and unreadable
    /// files.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<SkippedFile>,
    /// Temporary recordings found in the folder, with what their names say.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub temporary: Vec<TempRecording>,
    /// Round files the game did not finish writing (`replay.container`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub incomplete: Vec<String>,
    /// Recordings started between two rounds and never saved, found from the
    /// stream ids the rounds use (Y8S4+).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unsaved_recordings: Vec<UnsavedGap>,
    /// Match ids across the rounds; more than one means the folder mixes
    /// matches.
    pub match_ids: Vec<String>,
    /// Round numbers present, counting from 1 as the file names do.
    pub rounds: Vec<u32>,
    /// Round numbers between the first and last present round that have no
    /// file, counting from 1.
    pub missing_rounds: Vec<u32>,
    /// Round numbers that more than one file claims.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub duplicate_rounds: Vec<u32>,
    /// Scores after the last round read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_score: Option<[u32; 2]>,
    /// Whether the last round read ends the match. `None` when that cannot be
    /// told (overtime, unknown round count).
    pub complete: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl Match {
    /// Reads every round in `dir` in parallel.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(dir, crate::ReadMode::Full)
    }

    /// Reads every finished round in `dir`. `.tmprec` files are skipped, as
    /// are exact duplicates and files that fail to parse (listed in
    /// `folder.skipped`). Fails only when no round could be read.
    pub fn open_with(dir: impl AsRef<Path>, options: impl Into<ReadOptions>) -> Result<Self> {
        let dir = dir.as_ref();
        let options = options.into();
        let listing = list_folder(dir)?;
        let results: Vec<_> = listing
            .replays
            .par_iter()
            .map(|path| read_round(path, options))
            .collect();

        let mut skipped = listing.skipped;
        let mut rounds = Vec::new();
        let mut first_error = None;
        let mut by_hash: HashMap<String, String> = HashMap::new();
        for (path, result) in listing.replays.iter().zip(results) {
            let name = display_name(path);
            match result {
                Ok(round) => {
                    let hash = round
                        .file
                        .as_ref()
                        .map(|f| f.sha256.clone())
                        .unwrap_or_default();
                    // Files are in name order, so the first copy by name is kept.
                    if let Some(first) = by_hash.get(&hash) {
                        skipped.push(SkippedFile {
                            file: name,
                            reason: format!("identical to {first}"),
                        });
                        continue;
                    }
                    by_hash.insert(hash, name);
                    rounds.push(round);
                }
                Err(e) => {
                    skipped.push(SkippedFile {
                        file: name,
                        reason: format!("failed to parse: {e}"),
                    });
                    first_error.get_or_insert(e);
                }
            }
        }
        if rounds.is_empty() {
            return Err(first_error.unwrap_or(Error::InvalidFolder));
        }
        rounds.sort_by(|a, b| {
            let key = |r: &Round| (r.header.round_number, file_name(r));
            key(a).cmp(&key(b))
        });
        let mut report = analyze(&rounds);
        report.path = dir.display().to_string();
        if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
            check_names(&mut report, name, &rounds);
        }
        report.round_files = listing.replays.len();
        report.skipped = skipped;
        report.temporary = listing.temporary;
        Ok(Self {
            rounds,
            folder: Some(report),
        })
    }

    pub fn player_stats(&self) -> Vec<PlayerMatchStats> {
        match_stats(&self.rounds)
    }

    /// Match-level facts for match history. `None` only when there are no
    /// rounds.
    pub fn summary(&self) -> Option<MatchSummary> {
        MatchSummary::new(&self.rounds)
    }

    /// Side, site, spawn, operator and end-reason breakdowns.
    pub fn analytics(&self) -> MatchAnalytics {
        MatchAnalytics::new(&self.rounds)
    }
}

fn read_round(path: &Path, options: ReadOptions) -> Result<Round> {
    let raw = std::fs::read(path)?;
    let mut round = Round::from_bytes(&raw, options)?;
    round.file = Some(FileInfo::new(path, &raw));
    Ok(round)
}

fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn file_name(r: &Round) -> String {
    r.file
        .as_ref()
        .map(|f| f.file_name.clone())
        .unwrap_or_default()
}

/// Checks a set of rounds (sorted by round number) for gaps, duplicates and
/// whether the match finished.
pub fn analyze(rounds: &[Round]) -> FolderReport {
    let mut report = FolderReport {
        rounds_read: rounds.len(),
        ..FolderReport::default()
    };
    for r in rounds {
        if !report.match_ids.contains(&r.header.match_id) {
            report.match_ids.push(r.header.match_id.clone());
        }
    }
    if report.match_ids.len() > 1 {
        report.warnings.push(format!(
            "folder mixes {} matches; round checks assume one",
            report.match_ids.len()
        ));
    }

    let mut numbers: Vec<u32> = rounds.iter().map(|r| r.header.round_number + 1).collect();
    for r in rounds {
        let Some(from_name) = r
            .file
            .as_ref()
            .and_then(|f| file::round_from_file_name(Path::new(&f.file_name)))
        else {
            continue;
        };
        if from_name != r.header.round_number + 1 {
            report.warnings.push(format!(
                "{} holds round {} according to its header",
                file_name(r),
                r.header.round_number + 1
            ));
        }
    }
    numbers.sort_unstable();
    for w in numbers.windows(2) {
        if w[0] == w[1] && !report.duplicate_rounds.contains(&w[0]) {
            report.duplicate_rounds.push(w[0]);
        }
    }
    numbers.dedup();
    // Rounds before the first file are missing too: a match starts at 1.
    if let (Some(&last), true) = (numbers.last(), report.match_ids.len() == 1) {
        report.missing_rounds = (1..last).filter(|n| !numbers.contains(n)).collect();
    }
    report.rounds = numbers;
    if !report.missing_rounds.is_empty() {
        report
            .warnings
            .push(format!("rounds {:?} have no file", report.missing_rounds));
    }

    recording_gaps(rounds, &mut report);

    if let Some(last) = rounds.last() {
        let (score, mut complete) = match_result(last);
        // Y11S3+ marks the deciding round in its header.
        if let Some(i) = rounds.iter().position(|r| r.header.match_result.is_some()) {
            complete = Some(true);
            if i + 1 < rounds.len() {
                report.warnings.push(format!(
                    "round {} decided the match, but later rounds exist",
                    rounds[i].header.round_number + 1
                ));
            }
        }
        report.final_score = score;
        report.complete = complete;
        if complete == Some(false) {
            report.warnings.push(format!(
                "match not finished after round {}; later rounds may be missing",
                last.header.round_number + 1
            ));
        }
    }
    report
}

/// Rounds the game did not finish writing, and stream ids no saved round
/// used. Each round takes its recording id and one id per stream; the next
/// recording the game starts takes the id after.
fn recording_gaps(rounds: &[Round], report: &mut FolderReport) {
    let recorded: Vec<(&Round, &crate::Container, u32)> = rounds
        .iter()
        .filter_map(|r| {
            let c = r.container.as_ref()?;
            Some((r, c, c.recording_id?))
        })
        .collect();
    for (r, c, _) in &recorded {
        if !c.complete {
            report.incomplete.push(file_name(r));
        }
    }
    match report.incomplete.as_slice() {
        [] => {}
        [one] => report.warnings.push(format!(
            "{one} was not finished by the game: its events are missing"
        )),
        many => report.warnings.push(format!(
            "{} were not finished by the game: their events are missing",
            many.join(", ")
        )),
    }
    for w in recorded.windows(2) {
        let ((a, ca, first), (b, _, next)) = (w[0], w[1]);
        let expected = first + 1 + ca.streams.len() as u32;
        if next > expected {
            let gap = UnsavedGap {
                after: file_name(a),
                before: file_name(b),
                ids: next - expected,
            };
            report.warnings.push(format!(
                "{} stream ids unused between {} and {}: a recording was started there and never saved",
                gap.ids, gap.after, gap.before
            ));
            report.unsaved_recordings.push(gap);
        } else if next < first {
            report.warnings.push(format!(
                "recording ids go back from {} to {}: the game was restarted between them",
                file_name(a),
                file_name(b)
            ));
        }
    }
}

/// Reads the folder's name, and checks that each game-named round file
/// belongs to this folder.
pub fn check_names(report: &mut FolderReport, folder: &str, rounds: &[Round]) {
    report.name = file::parse_folder_name(folder);
    for r in rounds {
        let name = file_name(r);
        if let Some(owner) = file::match_of_file_name(Path::new(&name)).filter(|&o| o != folder) {
            report.warnings.push(format!(
                "{name} is named for another match folder ({owner})"
            ));
        }
    }
}

/// Scores after `round`, and whether they end the match.
fn match_result(round: &Round) -> (Option<[u32; 2]>, Option<bool>) {
    let h = &round.header;
    let t = &h.teams;
    // From Y9S4 the header holds the score after the round; before, the score
    // going into it.
    let score = if h.code_version >= version::Y9S4 {
        [t[0].score, t[1].score]
    } else if t.iter().any(|t| t.won) {
        [
            t[0].score + u32::from(t[0].won),
            t[1].score + u32::from(t[1].won),
        ]
    } else {
        return (None, None);
    };
    let per_match = h.rounds_per_match;
    if per_match == 0 {
        return (Some(score), None);
    }
    let half = per_match / 2;
    let (hi, lo) = (score[0].max(score[1]), score[0].min(score[1]));
    let complete = if hi > half && lo < half {
        Some(true)
    } else if hi >= half && lo >= half {
        // Overtime rules vary by playlist.
        None
    } else {
        Some(false)
    };
    (Some(score), complete)
}

impl Serialize for Match {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Output<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            summary: Option<MatchSummary>,
            #[serde(skip_serializing_if = "Option::is_none")]
            folder: Option<&'a FolderReport>,
            analytics: MatchAnalytics,
            rounds: &'a [Round],
            stats: Vec<PlayerMatchStats>,
        }
        Output {
            summary: self.summary(),
            folder: self.folder.as_ref(),
            analytics: self.analytics(),
            rounds: &self.rounds,
            stats: self.player_stats(),
        }
        .serialize(s)
    }
}

/// The files directly inside a folder, split by kind.
#[derive(Clone, Debug, Default)]
pub struct Listing {
    /// Finished round files, sorted by name.
    pub replays: Vec<PathBuf>,
    /// Temporary recordings and other files that are not rounds.
    pub skipped: Vec<SkippedFile>,
    /// The temporary recordings, with what their names say.
    pub temporary: Vec<TempRecording>,
}

/// Lists `dir`, separating finished `.rec` files from in-progress `.tmprec`
/// recordings.
pub fn list_folder(dir: &Path) -> Result<Listing> {
    let mut listing = Listing::default();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() {
            continue;
        }
        if file::is_replay(&path) {
            listing.replays.push(path);
        } else if file::is_temporary(&path) {
            let mut t = file::parse_temp_name(&display_name(&path));
            t.size = entry.metadata().ok().map(|m| m.len());
            listing.skipped.push(SkippedFile {
                file: t.file.clone(),
                reason: "temporary recording (.tmprec), not a finished replay".into(),
            });
            listing.temporary.push(t);
        }
    }
    if listing.replays.is_empty() {
        return Err(Error::InvalidFolder);
    }
    listing.replays.sort();
    listing.skipped.sort_by(|a, b| a.file.cmp(&b.file));
    listing.temporary.sort_by(|a, b| a.file.cmp(&b.file));
    Ok(listing)
}

/// Sorted paths of the `.rec` files directly inside `dir`.
pub fn list_replay_files(dir: &Path) -> Result<Vec<PathBuf>> {
    Ok(list_folder(dir)?.replays)
}

/// Every folder under `root` (itself included, two levels down) that holds
/// `.rec` files, e.g. each `Match-...` folder under `MatchReplay`.
pub fn find_match_folders(root: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let mut has_replay = false;
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                if depth < 2 {
                    stack.push((path, depth + 1));
                }
            } else if file::is_replay(&path) {
                has_replay = true;
            }
        }
        if has_replay {
            found.push(dir);
        }
    }
    found.sort();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::Header;

    fn recorded(number: u32, file: &str, recording_id: u32, streams: usize) -> Round {
        let mut r = round(number, [number, 0], file);
        r.container = Some(crate::Container {
            complete: true,
            recording_id: Some(recording_id),
            streams: vec![crate::StreamInfo::default(); streams],
            ..crate::Container::default()
        });
        r
    }

    fn round(number: u32, scores: [u32; 2], file: &str) -> Round {
        let mut header = Header {
            match_id: "m".into(),
            round_number: number,
            rounds_per_match: 12,
            code_version: version::Y9S4,
            ..Header::default()
        };
        header.teams[0].score = scores[0];
        header.teams[1].score = scores[1];
        Round {
            header,
            file: Some(FileInfo {
                file_name: file.into(),
                ..FileInfo::default()
            }),
            ..Round::default()
        }
    }

    #[test]
    fn spots_missing_rounds_and_unfinished_matches() {
        let rounds = [
            round(0, [1, 0], "R01.rec"),
            round(2, [2, 1], "R03.rec"),
            round(3, [2, 2], "R04.rec"),
        ];
        let r = analyze(&rounds);
        assert_eq!(r.rounds, vec![1, 3, 4]);
        assert_eq!(r.missing_rounds, vec![2]);
        assert_eq!(r.complete, Some(false));
        assert_eq!(r.final_score, Some([2, 2]));
    }

    #[test]
    fn a_first_round_file_is_expected() {
        let r = analyze(&[round(1, [7, 3], "R02.rec")]);
        assert_eq!(r.missing_rounds, vec![1]);
        assert_eq!(r.complete, Some(true));
    }

    #[test]
    fn flags_file_names_that_disagree_with_the_header() {
        let r = analyze(&[round(0, [1, 0], "R02.rec")]);
        assert_eq!(r.warnings.len(), 2, "{:?}", r.warnings); // name + unfinished
    }

    #[test]
    fn spots_a_recording_that_was_never_saved() {
        // R01 takes ids 0..=10; R02 should start at 11 but starts at 12.
        let rounds = [
            recorded(0, "R01.rec", 0, 10),
            recorded(1, "R02.rec", 12, 10),
            recorded(2, "R03.rec", 23, 10),
        ];
        let r = analyze(&rounds);
        assert_eq!(
            r.unsaved_recordings,
            [UnsavedGap {
                after: "R01.rec".into(),
                before: "R02.rec".into(),
                ids: 1
            }]
        );
        assert!(
            r.warnings.iter().any(|w| w.contains("never saved")),
            "{:?}",
            r.warnings
        );
    }

    #[test]
    fn lists_files_the_game_did_not_finish() {
        let mut broken = recorded(1, "R02.rec", 11, 10);
        broken.container.as_mut().unwrap().complete = false;
        let r = analyze(&[recorded(0, "R01.rec", 0, 10), broken]);
        assert_eq!(r.incomplete, ["R02.rec"]);
        assert!(r.unsaved_recordings.is_empty());
    }

    #[test]
    fn flags_a_round_file_from_another_match_folder() {
        let rounds = [round(0, [1, 0], "Match-2026-09-20_00-59-59-13160-R01.rec")];
        let mut report = analyze(&rounds);
        check_names(&mut report, "Match-2026-09-20_00-32-29-13160", &rounds);
        assert_eq!(report.name.as_ref().unwrap().process_id, 13160);
        assert!(
            report.warnings.iter().any(|w| w.contains("another match")),
            "{:?}",
            report.warnings
        );
    }

    #[test]
    fn overtime_is_not_judged() {
        let r = analyze(&[round(11, [6, 6], "R12.rec")]);
        assert_eq!(r.complete, None);
    }
}
