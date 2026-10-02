//! Smoke, fire, gas and swarm areas, environment objects and light screens
//! of the Y11S3 test replays, checked on the JSON a round is written as.
//! Data is read from `R6_TEST_DATA`, else `test_recordings/`; the tests
//! are skipped when neither has replays. With `R6_MATCH_REPLAY` set, the
//! invariants are checked on the rounds of that folder too; it is only
//! read.

use std::path::{Path, PathBuf};

use replay_analyzer::{ReadMode, Round};
use serde_json::Value;

/// Areas of the ten `custom_*.rec` rounds, by round number: smoke, fire,
/// gas, swarm, extinguisher.
const AREAS: [[usize; 5]; 10] = [
    [0, 0, 0, 1, 2],
    [2, 0, 0, 0, 2],
    [0, 2, 0, 0, 3],
    [1, 12, 2, 5, 2],
    [0, 2, 0, 3, 2],
    [0, 5, 0, 4, 2],
    [0, 8, 3, 0, 1],
    [1, 2, 3, 0, 5],
    [0, 0, 0, 0, 3],
    [1, 2, 2, 0, 1],
];
const KINDS: [&str; 5] = ["smoke", "fire", "gas", "swarm", "extinguisher"];
/// Gas pipes blown up, fire extinguishers burst and light screen posts of
/// those rounds.
const PIPES: [usize; 10] = [0, 0, 0, 1, 0, 0, 2, 0, 0, 2];
const EXTINGUISHERS: [usize; 10] = [2, 2, 3, 2, 2, 2, 1, 5, 3, 1];
const POSTS: [usize; 10] = [0, 21, 0, 0, 0, 0, 14, 28, 0, 14];
/// The rounds with a gas cloud of the round's Smoke.
const GAS_ROUNDS: [usize; 4] = [4, 7, 8, 10];

fn data_dir() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = match std::env::var_os("R6_TEST_DATA") {
        Some(dir) => PathBuf::from(dir),
        None => root.join("test_recordings"),
    };
    if !dir.join("valid").is_dir() {
        eprintln!("skipping: no test replays in {}", dir.display());
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

/// `(round number, round as JSON)` of the `custom_*.rec` test rounds.
fn rounds() -> Vec<(usize, Value)> {
    let Some(dir) = data_dir() else {
        return Vec::new();
    };
    let mut out: Vec<(usize, Value)> = replays(&dir.join("valid/Y11S3"))
        .into_iter()
        .filter_map(|path| {
            let name = path.file_stem()?.to_str()?;
            let number = name.strip_prefix("custom_")?.parse().ok()?;
            let round = Round::open(&path, ReadMode::Full).unwrap();
            Some((number, serde_json::to_value(&round).unwrap()))
        })
        .collect();
    out.sort_by_key(|r| r.0);
    out
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v[key].as_array().map_or(&[], Vec::as_slice)
}

fn seconds(when: &Value) -> Option<f64> {
    when["recordingTime"].as_f64()
}

/// How long something with `started` and `ended` lasted.
fn lasted(v: &Value) -> Option<f64> {
    Some(seconds(&v["ended"])? - seconds(&v["started"])?)
}

/// What holds for the areas, environment events and light screens of any
/// round; the failures, each named by `label` and the entry's index.
fn violations(label: &str, round: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let players: Vec<&str> = list(round, "players")
        .iter()
        .filter_map(|p| p["username"].as_str())
        .collect();
    let player = |v: &Value| v.is_null() || players.contains(&v.as_str().unwrap_or_default());
    let placed = |v: &Value| v["time"].is_string() && v["phase"].is_string();
    let areas = list(round, "areas");

    let mut last = 0.0;
    for (i, a) in areas.iter().enumerate() {
        let mut fail = |what: &str| out.push(format!("{label} area {i}: {what}"));
        let kind = a["kind"].as_str().unwrap_or_default();
        if !KINDS.contains(&kind) {
            fail("unknown kind");
        }
        if matches!(kind, "fire" | "gas" | "swarm") && a["source"].is_null() {
            fail("a fire, gas or swarm area without a source");
        }
        if !player(&a["username"]) || !player(&a["triggeredBy"]) {
            fail("names someone who is no player of the round");
        }
        // A pipe's fire and an extinguisher's cloud come from the map.
        let of_the_map = kind == "extinguisher" || a["source"] == "Gas Pipe";
        if of_the_map && !(a["username"].is_null() && a["sourceEntity"].is_null()) {
            fail("an area of the map with an owner");
        }
        if a["username"].is_null() != a["usernameSource"].is_null() {
            fail("an owner and how it is known do not come together");
        }
        if a["triggeredBy"].is_null() != a["triggerSource"].is_null() {
            fail("a trigger and how it is known do not come together");
        }
        if !placed(&a["started"]) {
            fail("not placed on the round clock");
        }
        let start = seconds(&a["started"]).unwrap_or(-1.0);
        if start < last {
            fail("started before the area listed before it");
        }
        last = start;
        if let Some(lasted) = lasted(a) {
            if lasted <= 0.0 {
                fail("ended before it started");
            }
            if kind == "gas" && !(9.7..=9.9).contains(&lasted) {
                fail("a gas cloud that did not last 9.7 to 9.9 s");
            }
            if a["source"] == "Smoke Grenade" && !(13.9..=14.2).contains(&lasted) {
                fail("a smoke grenade's cloud that did not last 13.9 to 14.2 s");
            }
        } else if !a["ended"].is_null() {
            fail("an end without a time");
        }
        // No radius is in the file: an area has its cells, or a radius
        // that says it is assumed.
        let points = list(a, "points");
        if a["radius"].is_null() != a["radiusSource"].is_null()
            || (!a["radius"].is_null() && a["radiusSource"] != "assumed")
        {
            fail("a radius that does not say it is assumed");
        }
        if !points.is_empty() && !a["radius"].is_null() {
            fail("both cells and a radius");
        }
        if matches!(kind, "smoke" | "extinguisher") && a["radius"].is_null() {
            fail("a cloud without its assumed radius");
        }
        if a["position"].as_array().is_none_or(|p| p.len() != 3) {
            fail("no position");
        }
        if points
            .iter()
            .any(|p| p.as_array().is_none_or(|p| p.len() != 3))
        {
            fail("a cell that is not [x, y, z]");
        }
        if a["capacity"]
            .as_u64()
            .is_some_and(|c| (c as usize) < points.len())
        {
            fail("more cells than the list holds");
        }
    }

    for (i, e) in list(round, "environment").iter().enumerate() {
        let mut fail = |what: &str| out.push(format!("{label} environment {i}: {what}"));
        let kind = e["kind"].as_str().unwrap_or_default();
        if !matches!(kind, "gasPipe" | "fireExtinguisher" | "metalDetector") {
            fail("unknown kind");
        }
        if !player(&e["by"]) {
            fail("set off by someone who is no player of the round");
        }
        if e["by"].is_null() != e["bySource"].is_null() {
            fail("who set it off and how that is known do not come together");
        }
        if e["object"].is_null() != e["objectSource"].is_null() {
            fail("an object and how it is known do not come together");
        }
        if !placed(e) {
            fail("not placed on the round clock");
        }
        if let (Some(start), Some(end)) = (seconds(e), seconds(&e["ended"]))
            && end < start
        {
            fail("ended before it started");
        }
        if let Some(area) = e["area"].as_u64() {
            let wanted = match kind {
                "gasPipe" => ("fire", "Gas Pipe"),
                _ => ("extinguisher", "Fire Extinguisher"),
            };
            let found = areas.get(area as usize);
            if found.is_none_or(|a| a["kind"] != wanted.0 || a["source"] != wanted.1) {
                fail("its area is not the fire or the cloud it made");
            }
        } else if kind == "fireExtinguisher" {
            fail("a burst without its cloud");
        }
        if (kind == "metalDetector") != e["event"].is_string() {
            fail("only a metal detector says what it did");
        }
    }

    for (i, s) in list(round, "lightScreens").iter().enumerate() {
        let mut fail = |what: &str| out.push(format!("{label} light screen {i}: {what}"));
        if !player(&s["username"]) {
            fail("owned by someone who is no player of the round");
        }
        if s["username"].is_null() != s["usernameSource"].is_null() {
            fail("an owner and how it is known do not come together");
        }
        let posts = list(s, "posts");
        if posts.is_empty() {
            fail("no posts");
        }
        if !placed(&s["started"]) {
            fail("not placed on the round clock");
        }
        for p in posts {
            if p["entity"].as_str().is_none() || p["position"].as_array().is_none() {
                fail("a post without its entity or its place");
            }
            if lasted(p).is_some_and(|l| l <= 0.0) {
                fail("a post that ended before it started");
            }
            if seconds(&p["started"]) < seconds(&s["started"]) {
                fail("a post up before its screen");
            }
        }
    }
    out
}

/// The `decodeStatus` entry of `field`.
fn status<'a>(round: &'a Value, field: &str) -> Option<&'a Value> {
    let fields = round["decodeStatus"]["fields"].as_array()?;
    fields.iter().find(|f| f["field"] == field)
}

#[test]
fn every_test_round_has_its_areas() {
    let rounds = rounds();
    let mut failures = Vec::new();
    for (number, round) in &rounds {
        let label = format!("custom_{number}");
        failures.extend(violations(&label, round));
        let Some(status) = status(round, "areas") else {
            failures.push(format!("{label}: no decodeStatus entry for areas"));
            continue;
        };
        if status["status"] != "decoded" || !list(status, "warnings").is_empty() {
            failures.push(format!("{label}: areas are {status}"));
        }
        let Some(expected) = AREAS.get(number - 1) else {
            continue;
        };
        let areas = list(round, "areas");
        for (kind, &count) in KINDS.iter().zip(expected) {
            let found = areas.iter().filter(|a| a["kind"] == *kind).count();
            if found != count {
                failures.push(format!("{label}: {found} {kind} areas, not {count}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A gas cloud is the round's Smoke's, and a smoke grenade's cloud the
/// thrower's: the owner of each is on the team its alliance names.
#[test]
fn clouds_name_their_owner() {
    let mut failures = Vec::new();
    for (number, round) in rounds() {
        let players = list(&round, "players");
        let operator = |name: &str| {
            let player = players.iter().find(|p| p["username"] == name);
            player.and_then(|p| p["operator"]["name"].as_str())
        };
        let areas = list(&round, "areas");
        let gas: Vec<&Value> = areas.iter().filter(|a| a["kind"] == "gas").collect();
        if GAS_ROUNDS.contains(&number) == gas.is_empty() {
            failures.push(format!("custom_{number}: {} gas clouds", gas.len()));
        }
        for a in gas {
            let owner = a["username"].as_str().unwrap_or_default();
            if operator(owner) != Some("Smoke") {
                failures.push(format!("custom_{number}: a gas cloud of {owner:?}"));
            }
        }
        for a in areas {
            let owner = a["username"].as_str();
            if matches!(a["kind"].as_str(), Some("smoke" | "gas" | "swarm")) && owner.is_none() {
                failures.push(format!("custom_{number}: a cloud without an owner: {a}"));
            }
            // The effect carries the alliance of whoever set it off, which
            // for a thrown gadget is its owner.
            let alliance = owner
                .and_then(|o| players.iter().find(|p| p["username"] == o))
                .map(|p| &p["alliance"]);
            let thrown = !matches!(a["source"].as_str(), Some("Volcan Canister"));
            if thrown && alliance.is_some_and(|al| *al != a["alliance"]) {
                failures.push(format!("custom_{number}: owner on the other team: {a}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn environment_objects_and_light_screens_of_the_test_rounds() {
    let mut failures = Vec::new();
    for (number, round) in rounds() {
        let label = format!("custom_{number}");
        let events = list(&round, "environment");
        let count = |kind: &str| events.iter().filter(|e| e["kind"] == kind).count();
        let expected = |table: &[usize; 10]| table.get(number - 1).copied();
        if Some(count("gasPipe")) != expected(&PIPES) {
            failures.push(format!("{label}: {} gas pipes", count("gasPipe")));
        }
        if Some(count("fireExtinguisher")) != expected(&EXTINGUISHERS) {
            let found = count("fireExtinguisher");
            failures.push(format!("{label}: {found} fire extinguishers"));
        }
        for e in events {
            // Every pipe of the test rounds was shot, and is followed by
            // its fire; the test map's pipes and extinguishers are known.
            if e["kind"] == "gasPipe" {
                if e["bySource"] != "read" || e["area"].is_null() {
                    failures.push(format!("{label}: a pipe without shooter or fire: {e}"));
                }
                if e["objectSource"] != "table" {
                    failures.push(format!("{label}: a pipe not from the table: {e}"));
                }
            }
            if e["kind"] == "metalDetector" && e["event"] == "alarm" && e["by"].is_null() {
                failures.push(format!("{label}: an alarm with nobody near: {e}"));
            }
        }
        let screens = list(&round, "lightScreens");
        let posts: usize = screens.iter().map(|s| list(s, "posts").len()).sum();
        if Some(posts) != expected(&POSTS) {
            failures.push(format!("{label}: {posts} light screen posts"));
        }
        for s in screens {
            let owner = s["username"].as_str().unwrap_or_default();
            let player = list(&round, "players")
                .iter()
                .find(|p| p["username"] == owner);
            if player.map(|p| &p["operator"]["name"]) != Some(&Value::from("Sens")) {
                failures.push(format!("{label}: a light screen of {owner:?}"));
            }
            // Each roll of the test rounds drops seven posts, and its
            // projector and throw are found.
            if list(s, "posts").len() != 7 || s["entity"].is_null() || s["throw"].is_null() {
                failures.push(format!("{label}: a light screen without its roll: {s}"));
            }
            let throw = s["throw"]
                .as_u64()
                .and_then(|i| list(&round, "throws").get(i as usize));
            if throw.is_none_or(|t| t["username"] != s["username"]) {
                failures.push(format!("{label}: a light screen of another's throw"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// With `R6_MATCH_REPLAY` set: every area with an owner names a player,
/// and nothing ends before it starts, in each round of that folder.
#[test]
fn real_rounds_hold_the_invariants() {
    let Some(dir) = std::env::var_os("R6_MATCH_REPLAY") else {
        eprintln!("skipping: R6_MATCH_REPLAY is not set");
        return;
    };
    let mut failures = Vec::new();
    let (mut rounds, mut areas) = (0, 0);
    for path in replays(Path::new(&dir)) {
        let Ok(round) = Round::open(&path, ReadMode::Full) else {
            continue;
        };
        let round = serde_json::to_value(&round).unwrap();
        if status(&round, "areas").is_none() {
            continue;
        }
        rounds += 1;
        areas += list(&round, "areas").len();
        let label = path.file_name().unwrap_or_default().to_string_lossy();
        failures.extend(violations(&label, &round));
    }
    eprintln!("{rounds} rounds, {areas} areas");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Fire and gas damage is taken in a fire or a gas area that is there at
/// that moment: every such hit of the test rounds names one (16 fire hits
/// and 8 gas hits), and says the area is derived from where the body was.
#[test]
fn fire_and_gas_hits_lie_inside_an_area() {
    let rounds = rounds();
    if rounds.is_empty() {
        return;
    }
    let (mut fire, mut gas) = ((0, 0), (0, 0));
    for (number, round) in &rounds {
        let areas = list(round, "areas");
        for h in list(round, "hits") {
            let (count, kind) = match h["type"]["id"].as_u64() {
                Some(36) => (&mut fire, "fire"),
                Some(9) => (&mut gas, "gas"),
                _ => {
                    assert!(h["inArea"].is_null(), "custom_{number}: {h}");
                    continue;
                }
            };
            count.0 += 1;
            let inside = &h["inArea"];
            if inside.is_null() {
                continue;
            }
            count.1 += 1;
            assert_eq!(h["inAreaSource"], "derived", "custom_{number}: {h}");
            assert_eq!(inside["kind"], kind, "custom_{number}: {h}");
            let area = &areas[inside["area"].as_u64().unwrap() as usize];
            assert_eq!(area["kind"], inside["kind"], "custom_{number}: {h}");
            assert_eq!(area["source"], inside["source"], "custom_{number}: {h}");
            assert_eq!(area["username"], inside["username"], "custom_{number}: {h}");
            let at = seconds(h).unwrap();
            let started = seconds(&area["started"]).unwrap();
            assert!(started <= at, "custom_{number}: {h}");
            let ended = seconds(&area["ended"]);
            assert!(ended.is_none_or(|end| at <= end), "custom_{number}: {h}");
        }
    }
    assert_eq!((fire, gas), ((16, 16), (8, 8)));
}

/// What the areas say of a kill and of a shot: round 4 has two kills of
/// players standing in the swarm of a Kawan hive, and shots pass through
/// smoke only while a smoke cloud is there.
#[test]
fn kills_name_their_area_and_shots_their_smoke() {
    let rounds = rounds();
    if rounds.is_empty() {
        return;
    }
    let (mut in_area, mut through) = (Vec::new(), 0);
    for (number, round) in &rounds {
        for u in list(round, "matchFeedback") {
            if u["inArea"].is_null() {
                assert!(u["inAreaSource"].is_null(), "custom_{number}: {u}");
                continue;
            }
            assert_eq!(u["inAreaSource"], "derived", "custom_{number}: {u}");
            let name = u["type"]["name"].as_str().unwrap_or_default();
            assert!(matches!(name, "Kill" | "Death"), "custom_{number}: {u}");
            let kind = u["inArea"]["kind"].as_str().unwrap_or_default();
            let known = matches!(kind, "fire" | "gas" | "swarm");
            assert!(known, "custom_{number}: {u}");
            in_area.push((*number, kind.to_owned(), u["inArea"]["username"].clone()));
        }
        let clouds: Vec<(f64, Option<f64>)> = list(round, "areas")
            .iter()
            .filter(|a| a["kind"] == "smoke")
            .filter_map(|a| Some((seconds(&a["started"])?, seconds(&a["ended"]))))
            .collect();
        for s in list(round, "shots") {
            if s["throughSmoke"].is_null() {
                continue;
            }
            assert_eq!(s["throughSmoke"], true, "custom_{number}: {s}");
            through += 1;
            let at = seconds(s).unwrap();
            let there = |c: &(f64, Option<f64>)| c.0 <= at && c.1.is_none_or(|end| at <= end);
            assert!(clouds.iter().any(there), "custom_{number}: {s}");
        }
    }
    let hive = |n: usize| (n, "swarm".to_owned(), Value::from("vitaking.FaZe"));
    assert_eq!(in_area, [hive(4), hive(4)]);
    // Rounds 2, 4 and 10 have shots fired through a smoke grenade's cloud.
    assert_eq!(through, 20);
}
