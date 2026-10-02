//! The scoreboard of the Y11S3 test replays (`scoreboard[]`): the match
//! totals in the opening snapshot and at the end, and what the round added,
//! checked against the kill feed, the score changes and the next round.
//! Data is read from `R6_TEST_DATA`, else `test_recordings/`; the tests are
//! skipped when neither has replays. With `R6_MATCH_REPLAY` set, the same
//! invariants are checked on the rounds of that folder; it is only read.

use std::collections::HashMap;
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

fn list<'a>(round: &'a Value, key: &str) -> &'a [Value] {
    round[key].as_array().map_or(&[], Vec::as_slice)
}

fn name(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

/// What the round's feed and score changes say each player's scoreboard
/// gained: (kills, deaths, score, plants). A kill counts for the player it
/// is credited to, and a team kill for nobody.
fn expected(round: &Value) -> HashMap<String, (i64, i64, i64, i64)> {
    let team: HashMap<String, &Value> = list(round, "players")
        .iter()
        .map(|p| (name(&p["username"]), &p["teamIndex"]))
        .collect();
    let mut want: HashMap<String, (i64, i64, i64, i64)> = HashMap::new();
    for u in list(round, "matchFeedback") {
        let (killer, target) = (name(&u["username"]), name(&u["target"]));
        match u["type"]["name"].as_str() {
            Some("Kill") => {
                want.entry(target.clone()).or_default().1 += 1;
                if team.get(&killer) != team.get(&target) {
                    let credited = u["creditedTo"].as_str().map_or(killer, str::to_owned);
                    want.entry(credited).or_default().0 += 1;
                }
            }
            Some("Death") => want.entry(killer).or_default().1 += 1,
            Some("DefuserPlantComplete") => want.entry(killer).or_default().3 += 1,
            _ => {}
        }
    }
    for c in list(round, "scoreChanges") {
        want.entry(name(&c["username"])).or_default().2 += c["delta"].as_i64().unwrap();
    }
    want
}

/// The invariants of one round's `scoreboard[]`. Plants are compared only
/// when `plants` is set: the game sometimes counts one the feed has not.
fn check(label: &str, round: &Value, plants: bool) {
    let want = expected(round);
    let stats: HashMap<String, &Value> = list(round, "stats")
        .iter()
        .map(|s| (name(&s["username"]), s))
        .collect();
    for b in list(round, "scoreboard") {
        let user = name(&b["username"]);
        let label = format!("{label} {user}");
        let (start, end, added) = (&b["start"], &b["end"], &b["round"]);
        for key in ["score", "kills", "deaths", "assists", "gameModeActions"] {
            let of = |v: &Value| {
                v[key]
                    .as_i64()
                    .unwrap_or_else(|| panic!("{label}: no {key}"))
            };
            assert_eq!(of(end) - of(start), of(added), "{label}: {key}");
        }
        let (kills, deaths, score, planted) = want.get(&user).copied().unwrap_or_default();
        assert_eq!(added["kills"], kills, "{label}: kills");
        assert_eq!(added["deaths"], deaths, "{label}: deaths");
        assert_eq!(added["score"], score, "{label}: score");
        if plants {
            assert_eq!(added["gameModeActions"], planted, "{label}: plants");
        }
        let s = stats[&user];
        assert_eq!(end["score"], s["score"], "{label}: stats score");
        assert_eq!(added["assists"], s["assists"], "{label}: stats assists");
        assert_eq!(added["deaths"] == 1, s["died"] == true, "{label}: died");
    }
}

/// Every player of the test rounds has a scoreboard; it agrees with the
/// feed and the score changes; and each round starts on the totals the
/// round before ended on, the first on zero.
#[test]
fn test_rounds_carry_the_match_totals() {
    let Some(dir) = data_dir() else { return };
    let mut before: Option<Value> = None;
    let mut assists = 0;
    for n in 1..=10 {
        let path = dir.join("valid/Y11S3").join(format!("custom_{n}.rec"));
        let round = Round::open(&path, ReadMode::Full).unwrap();
        let round = serde_json::to_value(&round).unwrap();
        let boards = list(&round, "scoreboard");
        assert_eq!(boards.len(), 10, "custom_{n}");
        check(&format!("custom_{n}"), &round, true);
        for b in boards {
            assert_eq!(b["placement"], 0);
            assists += b["round"]["assists"].as_i64().unwrap();
            let start = match &before {
                Some(before) => list(before, "scoreboard")
                    .iter()
                    .find(|p| p["username"] == b["username"])
                    .map(|p| p["end"].clone())
                    .unwrap(),
                None => serde_json::json!({
                    "score": 0, "kills": 0, "deaths": 0, "assists": 0, "gameModeActions": 0
                }),
            };
            assert_eq!(b["start"], start, "custom_{n} {}", b["username"]);
        }
        before = Some(round);
    }
    assert_eq!(assists, 32);
}

/// With `R6_MATCH_REPLAY` set: the same on every Y11S3 round of the folder.
#[test]
fn real_rounds_hold_the_invariants() {
    let Some(dir) = std::env::var_os("R6_MATCH_REPLAY") else {
        eprintln!("skipping: R6_MATCH_REPLAY is not set");
        return;
    };
    let mut read = 0;
    for path in replays(Path::new(&dir)) {
        let Ok(round) = Round::open(&path, ReadMode::Full) else {
            continue;
        };
        let label = path.file_name().unwrap_or_default().to_string_lossy();
        let round = serde_json::to_value(&round).unwrap();
        if list(&round, "scoreboard").is_empty() {
            continue;
        }
        read += 1;
        assert_eq!(
            list(&round, "scoreboard").len(),
            list(&round, "players").len(),
            "{label}"
        );
        check(&label, &round, false);
    }
    eprintln!("{read} rounds");
}
