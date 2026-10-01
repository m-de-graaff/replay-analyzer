//! Throws and launches of the Y11S3 test replays, checked on the JSON a
//! round is written as. Data is read from `R6_TEST_DATA`, else
//! `test_recordings/`; the tests are skipped when neither has replays.
//! With `R6_MATCH_REPLAY` set, the invariants are checked on the rounds
//! of that folder too; it is only read.

use std::path::{Path, PathBuf};

use replay_analyzer::{ReadMode, Round};
use serde_json::Value;

/// Throws of the ten `custom_*.rec` rounds, by round number.
const RELEASES: [usize; 10] = [44, 30, 25, 54, 26, 47, 43, 32, 22, 25];
/// A count drops this long after the release at most (seconds).
const USE_LAG: f64 = 1.1;
/// A launcher's count drops at most this long before the release.
const FIRE_LEAD: f64 = 0.3;

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

fn numbers(v: &Value) -> Vec<f64> {
    let items = v.as_array().into_iter().flatten();
    items.map(|n| n.as_f64().unwrap()).collect()
}

/// What holds for every throw of a round; the failures, each named by
/// `label` and the throw's index.
fn violations(label: &str, round: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let players: Vec<&str> = list(round, "players")
        .iter()
        .filter_map(|p| p["username"].as_str())
        .collect();
    let mut last = 0.0;
    for (i, t) in list(round, "throws").iter().enumerate() {
        let mut fail = |what: &str| out.push(format!("{label} throw {i}: {what}"));
        if !players.contains(&t["username"].as_str().unwrap_or_default()) {
            fail("thrower is no player of the round");
        }
        if t["asset"].as_u64().is_none_or(|a| a == 0) {
            fail("no asset");
        }
        if !matches!(
            t["slot"].as_str(),
            None | Some("ability" | "gadget" | "drone")
        ) {
            fail("unknown slot");
        }
        if t["subMunition"] == true && !t["slot"].is_null() {
            fail("a sub-munition from a slot");
        }
        if t["inferred"] == true && t["name"].is_null() {
            fail("inferred without a name");
        }
        if !t["id"].is_null() && t["slot"].is_null() {
            fail("an item id without a slot");
        }
        let when = t["recordingTime"].as_f64().unwrap_or(-1.0);
        if when < last {
            fail("released before the throw listed before it");
        }
        last = when;
        if t["time"].as_str().is_none() || t["phase"].as_str().is_none() {
            fail("not placed on the round clock");
        }

        let path: Vec<Vec<f64>> = list(t, "path").iter().map(numbers).collect();
        if path.len() > 60 || path.iter().any(|p| p.len() != 4) {
            fail("path too long or not [t, x, y, z]");
            continue;
        }
        if path.windows(2).any(|w| w[1][0] < w[0][0]) || path.first().is_some_and(|p| p[0] != 0.0) {
            fail("path times do not start at 0 and go up");
        }
        if path
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1e4)
        {
            fail("path leaves the map");
        }
        let (origin, end) = (numbers(&t["origin"]), numbers(&t["end"]));
        match (path.first(), path.last()) {
            (Some(first), Some(last)) => {
                if origin != first[1..] || end != last[1..] {
                    fail("origin and end are not the ends of the path");
                }
                if t["flightTime"].as_f64() != Some(last[0]) {
                    fail("flight time is not the last path time");
                }
            }
            _ if !origin.is_empty() || !end.is_empty() => fail("origin without a path"),
            _ => {}
        }
        let direction = numbers(&t["direction"]);
        if !direction.is_empty() {
            let length = direction.iter().map(|v| v * v).sum::<f64>().sqrt();
            if direction.len() != 3 || (length - 1.0).abs() > 1e-3 {
                fail("direction is no unit vector");
            }
            if t["speed"].as_f64().is_none_or(|s| s <= 0.0 || s > 200.0) {
                fail("direction without a plausible speed");
            }
        }
        match (t["ended"].as_str(), t["endedAfter"].as_f64()) {
            (None, None) => {}
            (Some("deleted" | "returned"), Some(after)) if after >= 0.0 => {}
            _ => fail("ended and endedAfter disagree"),
        }
    }
    out
}

#[test]
fn throws_hold_their_invariants() {
    let rounds = rounds();
    let mut failures = Vec::new();
    for (number, round) in &rounds {
        failures.extend(violations(&format!("custom_{number}"), round));
        let throws = list(round, "throws");
        let named = throws.iter().filter(|t| !t["name"].is_null()).count();
        println!("custom_{number}: {} throws, {named} named", throws.len());
        assert_eq!(named, throws.len(), "custom_{number}: every item is known");
        let status = list(&round["decodeStatus"], "fields")
            .iter()
            .find(|d| d["field"] == "throws");
        if let Some(status) = status {
            assert!(status["warnings"].is_null(), "custom_{number}: {status}");
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // The counts the research scripts found, when the ten rounds are there.
    let counts: Vec<usize> = (rounds.iter())
        .map(|(_, r)| list(r, "throws").len())
        .collect();
    if rounds.iter().map(|r| r.0).eq(1..=10) {
        assert_eq!(counts, RELEASES);
    }
}

/// The throws of `username` that `wanted` accepts.
fn releases<'a>(
    round: &'a Value,
    username: &Value,
    wanted: impl Fn(&Value) -> bool,
) -> Vec<&'a Value> {
    list(round, "throws")
        .iter()
        .filter(|t| t["username"] == *username && wanted(t))
        .collect()
}

#[test]
fn every_counted_use_follows_a_release() {
    let rounds = rounds();
    if rounds.is_empty() {
        return;
    }
    // (uses, uses with a release before them) of what is thrown by hand,
    // and of launchers.
    let (mut hand, mut launcher) = ((0, 0), (0, 0));
    let mut lags = Vec::new();
    let mut misplaced = Vec::new();
    for (number, round) in &rounds {
        for l in list(round, "loadouts") {
            for slot in ["ability", "gadget"] {
                let thrown = releases(round, &l["username"], |t| t["slot"] == slot);
                // A launcher's ammunition is in no slot.
                let launched = releases(round, &l["username"], |t| {
                    slot == "ability" && t["slot"].is_null() && t["subMunition"].is_null()
                });
                // A launcher's count drops in the frame it fires, which
                // can be the one before the projectile is let go.
                let (throws, tally, lead) = match (thrown.is_empty(), launched.is_empty()) {
                    (false, _) => (thrown, &mut hand, 0.0),
                    (true, false) => (launched, &mut launcher, FIRE_LEAD),
                    // The item is placed, not thrown.
                    (true, true) => continue,
                };
                let seconds = |v: &Value| v["recordingTime"].as_f64().unwrap();
                for u in list(&l[slot], "uses") {
                    let found = (throws.iter().rev())
                        .find(|t| (-lead..=USE_LAG).contains(&(seconds(u) - seconds(t))));
                    tally.0 += 1;
                    tally.1 += usize::from(found.is_some());
                    let Some(t) = found else {
                        println!(
                            "custom_{number}: {} {} use at {} has no release before it",
                            l["username"],
                            l[slot]["name"],
                            seconds(u)
                        );
                        continue;
                    };
                    lags.push(seconds(u) - seconds(t));
                    // The two are placed on the round clock from different
                    // streams, and must agree on it.
                    let elapsed = |v: &Value| v["elapsed"].as_f64().unwrap();
                    if (elapsed(u) - elapsed(t)).abs() > 2.0 {
                        misplaced.push(format!(
                            "custom_{number}: release at {} {} ({}), its use at {} {} ({})",
                            t["time"],
                            t["phase"],
                            t["elapsed"],
                            u["time"],
                            u["phase"],
                            u["elapsed"]
                        ));
                    }
                }
            }
        }
    }
    lags.sort_by(f64::total_cmp);
    println!(
        "uses with a release up to {USE_LAG} s before: thrown by hand {} of {}, launched {} of \
         {}; lag median {:.2} s, max {:.2} s",
        hand.1,
        hand.0,
        launcher.1,
        launcher.0,
        lags[lags.len() / 2],
        lags[lags.len() - 1]
    );
    assert!(hand.0 >= 100, "only {} uses of thrown items", hand.0);
    assert!(hand.1 * 100 >= hand.0 * 98, "{} of {}", hand.1, hand.0);
    assert!(launcher.1 * 100 >= launcher.0 * 95, "{launcher:?}");
    assert!(misplaced.is_empty(), "{}", misplaced.join("\n"));
}

#[test]
fn releases_are_placed_on_the_round_clock() {
    for (number, round) in rounds() {
        let throws = list(&round, "throws");
        let of = |t: &Value| {
            let seconds = t["recordingTime"].as_f64().unwrap();
            (seconds, t["elapsed"].as_f64().unwrap())
        };
        // The clock runs with the recording: two releases are as far apart
        // on one as on the other, to the second the clock shows. It stands
        // still only between the phases.
        for w in throws.windows(2) {
            let ((t0, e0), (t1, e1)) = (of(&w[0]), of(&w[1]));
            assert!(e1 >= e0, "custom_{number}: elapsed goes back at {t1}");
            if w[0]["phase"] == w[1]["phase"] {
                let drift = (e1 - e0) - (t1 - t0);
                assert!(drift.abs() <= 2.0, "custom_{number}: {drift} s off at {t1}");
            }
        }
        // Throws of the action phase are not left at the start of prep.
        for t in throws.iter().filter(|t| t["phase"] != "Prep") {
            assert!(of(t).1 > 0.0, "custom_{number}: {t}");
        }
        let phases: Vec<&str> = throws.iter().filter_map(|t| t["phase"].as_str()).collect();
        assert!(phases.contains(&"Action"), "custom_{number}: {phases:?}");
    }
}

/// The downward acceleration of the parabola through `(t, z)`, by least
/// squares.
fn gravity(points: &[(f64, f64)]) -> f64 {
    // Normal equations of z = a t^2 + b t + c, solved by elimination.
    let mut m = [[0.0f64; 4]; 3];
    for &(t, z) in points {
        let powers = [t * t, t, 1.0];
        for (row, p) in m.iter_mut().zip(powers) {
            for (cell, q) in row.iter_mut().zip(powers) {
                *cell += p * q;
            }
            row[3] += p * z;
        }
    }
    for i in 0..3 {
        for j in 0..3 {
            if j != i {
                let f = m[j][i] / m[i][i];
                let pivot = m[i];
                for (cell, p) in m[j].iter_mut().zip(pivot) {
                    *cell -= f * p;
                }
            }
        }
    }
    -2.0 * m[0][3] / m[0][0]
}

#[test]
fn grenades_fall_as_gravity_makes_them() {
    let mut found = Vec::new();
    for (_, round) in rounds() {
        for t in list(&round, "throws") {
            let name = t["name"].as_str().unwrap_or_default();
            if !["Frag Grenade", "Stun Grenade", "Smoke Grenade"].contains(&name) {
                continue;
            }
            // The first step is often cut short; free flight lasts until
            // the vertical speed jumps up at a bounce.
            let path: Vec<(f64, f64)> = (list(t, "path").iter().skip(1))
                .map(|p| (p[0].as_f64().unwrap(), p[3].as_f64().unwrap()))
                .collect();
            let speeds: Vec<f64> = (path.windows(2))
                .map(|w| (w[1].1 - w[0].1) / (w[1].0 - w[0].0))
                .collect();
            let bounce = speeds.windows(2).position(|w| w[1] - w[0] > 1.0);
            let free = &path[..bounce.map_or(path.len(), |b| b + 2)];
            if free.len() >= 9 {
                found.push(gravity(free));
            }
        }
    }
    if found.is_empty() {
        return;
    }
    found.sort_by(f64::total_cmp);
    let median = found[found.len() / 2];
    println!(
        "gravity over {} grenade flights: median {median:.2} m/s^2, from {:.2} to {:.2}",
        found.len(),
        found[0],
        found[found.len() - 1]
    );
    assert!(found.len() >= 10, "only {} flights", found.len());
    assert!((8.5..10.5).contains(&median), "median {median}");
    // Most flights, not all: one that grazes a wall fits a parabola badly.
    let near = found.iter().filter(|g| (5.0..13.0).contains(*g)).count();
    assert!(near * 10 >= found.len() * 9, "{near} of {}", found.len());
}

#[test]
fn real_rounds_hold_the_invariants() {
    let Some(dir) = std::env::var_os("R6_MATCH_REPLAY").map(PathBuf::from) else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let (mut rounds, mut throws) = (0, 0);
    let mut failures = Vec::new();
    for path in replays(&dir) {
        // A round the game is still writing, or of another season, may
        // not read; that is not what is tested here.
        let Ok(round) = Round::open(&path, ReadMode::Full) else {
            continue;
        };
        let round = serde_json::to_value(&round).unwrap();
        let label = path.file_name().unwrap_or_default().to_string_lossy();
        failures.extend(violations(&label, &round));
        rounds += 1;
        throws += list(&round, "throws").len();
    }
    println!("{throws} throws in {rounds} rounds of {}", dir.display());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
