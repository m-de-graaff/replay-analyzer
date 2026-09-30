//! Parsing a match folder: one `.rec` file per round.

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;

use crate::error::{Error, Result};
use crate::round::{ReadMode, Round};
use crate::stats::{PlayerMatchStats, match_stats};

#[derive(Clone, Debug, Default)]
pub struct Match {
    pub rounds: Vec<Round>,
}

impl Match {
    /// Reads every round in `dir` in parallel, ordered by file name.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let rounds = list_replay_files(dir.as_ref())?
            .par_iter()
            .map(|path| Round::open(path, ReadMode::Full))
            .collect::<Result<_>>()?;
        Ok(Self { rounds })
    }

    pub fn player_stats(&self) -> Vec<PlayerMatchStats> {
        match_stats(&self.rounds)
    }
}

impl Serialize for Match {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Output<'a> {
            rounds: &'a [Round],
            stats: Vec<PlayerMatchStats>,
        }
        Output {
            rounds: &self.rounds,
            stats: self.player_stats(),
        }
        .serialize(s)
    }
}

/// Sorted paths of the `.rec` files directly inside `dir`.
pub fn list_replay_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_file() && path.extension().is_some_and(|e| e == "rec") {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        return Err(Error::InvalidFolder);
    }
    paths.sort();
    Ok(paths)
}
