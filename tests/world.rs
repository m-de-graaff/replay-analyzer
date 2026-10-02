//! The shared pass over the movement and effects streams of the Y11S3
//! test replays, checked on what a round's `decodeStatus.world` says of
//! it. Data is read from `R6_TEST_DATA`, else `test_recordings/`; the
//! tests are skipped when neither has replays. With `R6_MATCH_REPLAY` set,
//! the same is checked on the rounds of that folder; it is only read.

use std::path::{Path, PathBuf};

use replay_analyzer::{ReadMode, Round};
use serde_json::Value;

fn data_dir() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = match std::env::var_os("R6_TEST_DATA") {
        Some(dir) => PathBuf::from(dir),
        None => root.join("test_recordings"),
    };
    if !dir.join("valid").join("Y11S3").is_dir() {
        eprintln!("skipping: no Y11S3 test replays in {}", dir.display());
        return None;
    }
    Some(dir)
}

/// Every `.rec` under `dir`, sorted.
fn replays(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "rec") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// The `decodeStatus` entry of `field`.
fn status<'a>(round: &'a Value, field: &str) -> Option<&'a Value> {
    let fields = round["decodeStatus"]["fields"].as_array()?;
    fields.iter().find(|f| f["field"] == field)
}

/// The world is `decoded` only when every effects record was read and
/// fewer than one in a thousand updates of the entities it follows were
/// not; a round that falls short says how many in its warnings.
#[test]
fn every_test_round_reads_the_world() {
    let Some(dir) = data_dir() else {
        return;
    };
    let rounds: Vec<PathBuf> = replays(&dir.join("valid").join("Y11S3"))
        .into_iter()
        .filter(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            name.starts_with("custom_")
        })
        .collect();
    assert!(!rounds.is_empty(), "no custom_*.rec in {}", dir.display());
    for path in rounds {
        let label = path.file_name().unwrap_or_default().to_string_lossy();
        let round = Round::open(&path, ReadMode::Full).unwrap();
        let round = serde_json::to_value(&round).unwrap();
        let world = status(&round, "world").unwrap_or_else(|| panic!("{label}: no world status"));
        assert_eq!(world["status"], "decoded", "{label}: {world}");
        // Thousands of updates in any round that was played.
        assert!(world["count"].as_u64() > Some(1000), "{label}: {world}");
        // The test rounds read without a single message left over.
        assert!(world["warnings"].is_null(), "{label}: {world}");
        // Nor does one of them skip game time.
        assert!(round["timing"]["skips"].is_null(), "{label}");
    }
}

/// A partial read does not walk the streams.
#[test]
fn a_partial_read_has_no_world() {
    let Some(dir) = data_dir() else {
        return;
    };
    let path = dir.join("valid").join("Y11S3").join("custom_1.rec");
    let round = Round::open(&path, ReadMode::Partial).unwrap();
    let round = serde_json::to_value(&round).unwrap();
    assert!(status(&round, "world").is_none());
}

#[test]
fn real_rounds_read_the_world() {
    let Some(dir) = std::env::var_os("R6_MATCH_REPLAY").map(PathBuf::from) else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let (mut rounds, mut updates, mut skips, mut skipping) = (0, 0, 0, 0);
    let mut failures = Vec::new();
    for path in replays(&dir) {
        // A round the game is still writing, or of another season, may
        // not read; that is not what is tested here.
        let Ok(round) = Round::open(&path, ReadMode::Full) else {
            continue;
        };
        let round = serde_json::to_value(&round).unwrap();
        let Some(world) = status(&round, "world") else {
            continue;
        };
        let label = path.file_name().unwrap_or_default().to_string_lossy();
        if world["status"] != "decoded" {
            failures.push(format!("{label}: {world}"));
        }
        rounds += 1;
        updates += world["count"].as_u64().unwrap_or(0);
        // A skip is two or more players jumping over one moment, by more
        // game time than passed.
        let found = round["timing"]["skips"].as_array();
        for s in found.into_iter().flatten() {
            let whole = s["bodies"].as_u64() >= Some(2)
                && s["until"].as_f64() > s["at"].as_f64()
                && s["seconds"].as_f64() > Some(0.0);
            if !whole {
                failures.push(format!("{label}: {s}"));
            }
            skips += 1;
        }
        skipping += usize::from(found.is_some());
    }
    println!("{updates} updates in {rounds} rounds of {}", dir.display());
    println!("{skips} skips of game time in {skipping} rounds");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
