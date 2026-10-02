//! Reinforcements and barricades of the Y11S3 test replays, checked on the
//! round's JSON. Data is read from `R6_TEST_DATA`, else `test_recordings/`;
//! the tests are skipped when neither has replays. With `R6_MATCH_REPLAY`
//! set, invariants are checked on the rounds of that folder; it is only
//! read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use replay_analyzer::{Match, ReadMode, Round};
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

/// The JSON of `custom_1.rec` .. `custom_10.rec`, in that order.
fn custom_rounds(dir: &Path) -> Vec<(String, Value)> {
    (1..=10)
        .map(|n| {
            let name = format!("custom_{n}");
            let path = dir.join("valid").join("Y11S3").join(format!("{name}.rec"));
            let round = Round::open(&path, ReadMode::Full).unwrap();
            (name, serde_json::to_value(&round).unwrap())
        })
        .collect()
}

fn list<'a>(round: &'a Value, key: &str) -> &'a [Value] {
    round[key].as_array().map_or(&[], Vec::as_slice)
}

/// Username -> side (`Attack` or `Defense`).
fn sides(round: &Value) -> HashMap<String, String> {
    list(&round["round"], "lineup")
        .iter()
        .filter_map(|l| {
            Some((
                l["username"].as_str()?.to_owned(),
                l["side"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

fn recording_time(when: &Value) -> Option<f64> {
    when["recordingTime"].as_f64()
}

/// Seconds a panel took to place; `None` when it was not completed.
fn took(panel: &Value) -> Option<f64> {
    Some(recording_time(&panel["completed"])? - recording_time(&panel["started"])?)
}

/// Completed reinforcements per test round, as the scoreboard and the
/// reference decoder count them.
const COMPLETED: [usize; 10] = [8, 10, 9, 9, 9, 10, 9, 8, 10, 9];

#[test]
fn the_test_rounds_have_their_reinforcements() {
    let Some(dir) = data_dir() else {
        return;
    };
    for ((name, round), expected) in custom_rounds(&dir).iter().zip(COMPLETED) {
        let sides = sides(round);
        let all = list(round, "reinforcements");
        let done: Vec<&Value> = all.iter().filter(|r| !r["completed"].is_null()).collect();
        assert_eq!(done.len(), expected, "{name}: completed reinforcements");
        for r in &done {
            let who = r["username"].as_str().unwrap_or_default();
            assert_eq!(
                sides.get(who).map(String::as_str),
                Some("Defense"),
                "{name}: {r}"
            );
            assert!(r["host"].is_string(), "{name}: no host: {r}");
            assert_eq!(r["cancelled"], false, "{name}: {r}");
            assert_eq!(r["default"], false, "{name}: {r}");
            let took = took(r).unwrap_or_else(|| panic!("{name}: no times: {r}"));
            // 4.1 s for a wall and 4.4 s for a hatch. One started as the
            // prep phase ended took 3.985 s (round 2).
            let range = match r["kind"].as_str() {
                Some("wall") => 3.9..=4.2,
                Some("hatch") => 4.3..=4.5,
                kind => panic!("{name}: kind {kind:?}: {r}"),
            };
            assert!(range.contains(&took), "{name}: took {took}: {r}");
            // A wall stands upright and a hatch lies flat.
            let up = r["normal"][2].as_f64().unwrap_or(f64::NAN).abs();
            let flat = r["kind"] == "hatch";
            assert_eq!(up > 0.7, flat, "{name}: {r}");
        }
        // One that was not completed was called off, and says when.
        for r in all.iter().filter(|r| r["completed"].is_null()) {
            assert_eq!(r["cancelled"], true, "{name}: {r}");
            assert!(recording_time(&r["ended"]).is_some(), "{name}: {r}");
            assert!(r["host"].is_null(), "{name}: {r}");
        }
        let status = list(&round["decodeStatus"], "fields")
            .iter()
            .find(|f| f["field"] == "panels")
            .unwrap_or_else(|| panic!("{name}: no panels status"));
        assert_eq!(status["status"], "decoded", "{name}: {status}");
        assert!(status["warnings"].is_null(), "{name}: {status}");
    }
}

#[test]
fn the_test_rounds_have_their_barricades() {
    let Some(dir) = data_dir() else {
        return;
    };
    for (name, round) in custom_rounds(&dir) {
        let sides = sides(&round);
        let all = list(&round, "barricades");
        let defaults = all.iter().filter(|b| b["default"] == true).count();
        assert_eq!(defaults, 18, "{name}: the map's own barricades");
        for b in all {
            assert_eq!(b["kind"], "barricade", "{name}: {b}");
            assert!(
                matches!(b["opening"].as_str(), Some("door" | "window")),
                "{name}: {b}"
            );
            assert_eq!(b["openingInferred"], true, "{name}: {b}");
            if b["default"] == true {
                // Nobody placed it.
                assert!(b["username"].is_null(), "{name}: {b}");
                assert!(b["started"].is_null(), "{name}: {b}");
                continue;
            }
            let who = b["username"].as_str().unwrap_or_default();
            assert!(sides.contains_key(who), "{name}: {b}");
            if b["cancelled"] == true {
                assert!(b["completed"].is_null(), "{name}: {b}");
                continue;
            }
            let took = took(b).unwrap_or_else(|| panic!("{name}: no times: {b}"));
            // 2.5 or 2.6 s. One started as the prep phase ended took
            // 2.342 s (round 3).
            assert!((2.3..=2.7).contains(&took), "{name}: took {took}: {b}");
            assert!(b["host"].is_string(), "{name}: no host: {b}");
        }
        // A destroyed barricade says how, and who when a damage entry or a
        // body near it names one.
        for b in all.iter().filter(|b| !b["destroyed"].is_null()) {
            let d = &b["destroyed"];
            assert!(recording_time(d).is_some(), "{name}: {b}");
            match d["source"].as_str() {
                Some("read") => {
                    assert!(d["damageId"].is_u64(), "{name}: {b}");
                    assert!(
                        matches!(
                            d["how"].as_str(),
                            Some("bullet" | "melee" | "explosion" | "gadget" | "other")
                        ),
                        "{name}: {b}"
                    );
                }
                Some("proximity") => {
                    assert!(sides.contains_key(d["by"].as_str().unwrap_or_default()));
                    assert!(d["distance"].as_f64() < Some(2.5), "{name}: {b}");
                    assert!(
                        matches!(d["how"].as_str(), Some("removed" | "brokenThrough")),
                        "{name}: {b}"
                    );
                }
                None => assert_eq!(d["how"], "unknown", "{name}: {b}"),
                source => panic!("{name}: source {source:?}: {b}"),
            }
            if let Some(by) = d["by"].as_str() {
                assert!(sides.contains_key(by), "{name}: {b}");
            }
        }
    }
}

/// Round 9 has the one barricade of the test rounds taken down by hand:
/// no damage entry, and a player standing 0.4 m from it.
#[test]
fn a_barricade_taken_down_by_hand_names_who_stood_at_it() {
    let Some(dir) = data_dir() else {
        return;
    };
    let rounds = custom_rounds(&dir);
    let (_, round) = &rounds[8];
    let removed: Vec<&Value> = list(round, "barricades")
        .iter()
        .filter(|b| b["destroyed"]["how"] == "removed")
        .collect();
    assert_eq!(removed.len(), 1);
    let d = &removed[0]["destroyed"];
    assert_eq!(d["by"], "kds.FaZe");
    assert_eq!(d["source"], "proximity");
    assert!((0.3..=0.5).contains(&d["distance"].as_f64().unwrap()));
}

/// What holds for every round, whatever was played in it.
fn check(round: &Value, name: &str, counts: &mut HashMap<&'static str, usize>) {
    let sides = sides(round);
    let operators: HashMap<&str, &str> = list(round, "players")
        .iter()
        .filter_map(|p| Some((p["username"].as_str()?, p["operator"]["name"].as_str()?)))
        .collect();
    let reinforcements = list(round, "reinforcements");
    let placed = |r: &&Value| !r["completed"].is_null() && r["default"] == false;
    let done = reinforcements.iter().filter(placed).count();
    assert!(done <= 10, "{name}: {done} reinforcements");
    *counts.entry("rounds").or_default() += 1;
    *counts.entry("reinforcements").or_default() += done;
    for r in reinforcements {
        assert!(
            matches!(r["kind"].as_str(), Some("wall" | "hatch")),
            "{name}: {r}"
        );
        if r["default"] == true {
            assert!(r["username"].is_null(), "{name}: {r}");
            *counts.entry("default reinforcements").or_default() += 1;
            continue;
        }
        let who = r["username"].as_str().unwrap_or_default();
        assert_eq!(
            sides.get(who).map(String::as_str),
            Some("Defense"),
            "{name}: {r}"
        );
        if let (Some(a), Some(b)) = (
            recording_time(&r["started"]),
            recording_time(&r["completed"]),
        ) {
            assert!(b >= a, "{name}: {r}");
        }
        if !r["kindSource"].is_null() {
            *counts.entry("reinforcements of unknown asset").or_default() += 1;
        }
    }
    for b in list(round, "barricades") {
        *counts.entry("barricades").or_default() += 1;
        if b["kind"] == "castle" {
            *counts.entry("castle panels").or_default() += 1;
            let who = b["username"].as_str().unwrap_or_default();
            assert_eq!(operators.get(who), Some(&"Castle"), "{name}: {b}");
        }
        if b["default"] == false {
            let who = b["username"].as_str().unwrap_or_default();
            assert!(sides.contains_key(who), "{name}: {b}");
        }
        if !b["kindSource"].is_null() {
            *counts.entry("barricades of unknown asset").or_default() += 1;
        }
        let d = &b["destroyed"];
        if let Some(how) = d["how"].as_str() {
            *counts.entry("destroyed barricades").or_default() += 1;
            let key = match how {
                "unknown" => "destroyed, unknown how",
                "removed" => "destroyed, removed by hand",
                "brokenThrough" => "destroyed, broken through",
                _ => "destroyed, by a damage entry",
            };
            *counts.entry(key).or_default() += 1;
            if d["source"] == "proximity" {
                *counts.entry("destroyed, by proximity").or_default() += 1;
            }
            assert_eq!(d["source"].is_null(), how == "unknown", "{name}: {b}");
        }
    }
}

#[test]
fn real_rounds_keep_the_invariants() {
    let root = std::env::var_os("R6_MATCH_REPLAY").map(PathBuf::from);
    let Some(root) = root.filter(|r| r.is_dir()) else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let mut counts = HashMap::new();
    for dir in replay_analyzer::matches::find_match_folders(&root).unwrap() {
        let m = Match::open_with(&dir, ReadMode::Full).unwrap();
        for (i, round) in m.rounds.iter().enumerate() {
            let label = dir.file_name().unwrap_or_default().to_string_lossy();
            let name = format!("{label} round {}", i + 1);
            let round = serde_json::to_value(round).unwrap();
            if round["barricades"].is_null() && round["reinforcements"].is_null() {
                continue;
            }
            check(&round, &name, &mut counts);
        }
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort();
    eprintln!("real rounds: {counts:?}");
}
