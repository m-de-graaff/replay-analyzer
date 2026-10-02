//! Gadget objects and map cameras of the Y11S3 test replays, checked on
//! the JSON a round is written as. Data is read from `R6_TEST_DATA`, else
//! `test_recordings/`; the tests are skipped when neither has replays.
//! With `R6_MATCH_REPLAY` set, the invariants are checked on the rounds
//! of that folder too; it is only read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use replay_analyzer::{ReadMode, Round};
use serde_json::Value;

/// Count drops of slots that hold a placed gadget, over the ten
/// `custom_*.rec` rounds.
const DROPS: usize = 118;
/// A count drops this long before the object is deployed at most, and
/// this long after it (seconds).
const USE_LEAD: f64 = 0.3;
const USE_LAG: f64 = 0.8;
/// An armor pack's count drops when the bag is on the floor.
const ARMOR_PACK_LAG: f64 = 2.1;
/// A gadget is placed within arm's reach (metres), with some room for the
/// tenth of a second a body's track is thinned to.
const REACH: f64 = 4.0;
/// What is not placed by hand: an Aqua Breacher is thrown onto its
/// surface, and Skopos is in one shell while the other stands elsewhere.
/// The shell a round starts with names nobody.
const REMOTE: [&str; 2] = ["Aqua Breacher", "V10 Pantheon Shells"];
/// The recording goes on this long after the round was decided at most
/// (seconds).
const AFTERMATH: f64 = 15.0;
/// Default cameras of the test rounds' map.
const CAMERAS: usize = 8;

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
    items.filter_map(Value::as_f64).collect()
}

fn seconds(when: &Value) -> Option<f64> {
    when["recordingTime"].as_f64()
}

/// A gadget the round's owner put down from a loadout slot: not what
/// another object left behind.
fn is_placed(g: &Value) -> bool {
    g["kind"] == "placed" && g["parent"].is_null()
}

/// What holds for every gadget and camera of a round; the failures, each
/// named by `label` and the gadget's index.
fn violations(label: &str, round: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let players: Vec<&str> = list(round, "players")
        .iter()
        .filter_map(|p| p["username"].as_str())
        .collect();
    let end = list(&round["round"], "phases")
        .iter()
        .filter_map(|p| p["recordingEnd"].as_f64())
        .fold(0.0, f64::max);
    let mut lives: HashMap<&str, Vec<(f64, bool)>> = HashMap::new();
    for (i, g) in list(round, "gadgets").iter().enumerate() {
        let mut fail = |what: &str| out.push(format!("{label} gadget {i}: {what}"));
        let entity = g["entity"].as_str().unwrap_or_default();
        if entity.is_empty() || u64::from_str_radix(entity, 16).is_err() {
            fail("its entity is not a hex id");
        }
        let name = g["name"].as_str().unwrap_or_default();
        let remote = REMOTE.iter().any(|r| name.contains(r));
        match g["username"].as_str() {
            Some(name) if !players.contains(&name) => fail("its owner is not a player"),
            None if is_placed(g) && !remote => fail("a placed gadget without an owner"),
            _ => {}
        }
        if g["username"].is_string() && !g["parent"].is_null() && g["usernameSource"].is_null() {
            fail("an owner taken from a parent does not say so");
        }
        if g["name"].is_string() == g["nameSource"].is_null() {
            fail("name and nameSource do not come together");
        }
        let started = seconds(&g["placing"]).or(seconds(&g["released"]));
        let deployed = seconds(&g["deployed"]);
        let position = numbers(&g["position"]);
        if deployed.is_some() && (position.len() != 3 || position == [0.0, 0.0, -100.0]) {
            fail("deployed without a place in the world");
        }
        if let (Some(a), Some(b)) = (started, deployed)
            && b < a
        {
            fail("deployed before it was started");
        }
        let when = seconds(&g["end"]);
        let present = g["end"]["how"] == "presentAtEnd";
        if g["end"]["source"].is_null() || present != when.is_none() {
            fail("its end has no source, or a time without an end");
        }
        if let (Some(a), Some(b)) = (started.or(deployed), when)
            && (b < a || b > end + AFTERMATH)
        {
            fail("it ended before it started or after the recording");
        }
        if let Some(d) = g["ownerDistance"].as_f64()
            && is_placed(g)
            && d > REACH
            && !remote
        {
            fail("placed out of its owner's reach");
        }
        if g["states"].as_array().is_some_and(|s| s.len() > 64) {
            fail("more states than are kept");
        }
        if let Some(start) = started.or(deployed) {
            lives.entry(entity).or_default().push((start, present));
        }
    }
    // An entity the game uses again is another gadget, and only the last
    // can be there at the end.
    for (entity, mut uses) in lives {
        uses.sort_by(|a, b| a.0.total_cmp(&b.0));
        let earlier = uses.split_last().map_or(&[][..], |u| u.1);
        if earlier.iter().any(|u| u.1) || uses.windows(2).any(|w| w[0].0 == w[1].0) {
            out.push(format!("{label}: entity {entity} is two gadgets at once"));
        }
    }
    for (i, c) in list(round, "mapCameras").iter().enumerate() {
        let alone = numbers(&c["position"]).len() == 3 && numbers(&c["rotation"]).len() == 4;
        if !alone || c["object"].as_str().is_none_or(str::is_empty) {
            out.push(format!("{label} camera {i}: no object, place or rotation"));
        }
    }
    out
}

/// The count drops of a round's slots that hold a placed gadget, as
/// `(drops, drops with a gadget deployed at that time, deployed gadgets
/// without a drop)`.
fn drops(label: &str, round: &Value) -> (usize, usize, Vec<String>) {
    let (mut all, mut matched, mut spare) = (0, 0, Vec::new());
    for l in list(round, "loadouts") {
        for slot in ["ability", "gadget"] {
            let Some(name) = l[slot]["name"].as_str() else {
                continue;
            };
            let mut mine: Vec<f64> = list(round, "gadgets")
                .iter()
                .filter(|g| is_placed(g) && g["username"] == l["username"])
                .filter(|g| g["slot"] == slot && g["name"] == name)
                .filter_map(|g| seconds(&g["deployed"]))
                .collect();
            if mine.is_empty() {
                continue;
            }
            let lag = if name == "Armor Pack" {
                ARMOR_PACK_LAG
            } else {
                USE_LAG
            };
            for u in list(&l[slot], "uses") {
                all += 1;
                let Some(at) = seconds(u) else {
                    continue;
                };
                let nearest = (0..mine.len())
                    .min_by(|&a, &b| (mine[a] - at).abs().total_cmp(&(mine[b] - at).abs()));
                if let Some(i) = nearest
                    && (-USE_LEAD..=lag).contains(&(at - mine[i]))
                {
                    mine.swap_remove(i);
                    matched += 1;
                }
            }
            for t in mine {
                spare.push(format!("{label} {} {name}: deployed at {t}", l["username"]));
            }
        }
    }
    (all, matched, spare)
}

#[test]
fn every_test_round_has_gadgets_and_the_maps_cameras() {
    for (n, round) in rounds() {
        let status = list(&round["decodeStatus"], "fields")
            .iter()
            .find(|f| f["field"] == "gadgets")
            .unwrap_or_else(|| panic!("round {n}: no gadgets in decodeStatus"));
        assert_eq!(status["status"], "decoded", "round {n}");
        let gadgets = list(&round, "gadgets");
        assert_eq!(status["count"].as_u64(), Some(gadgets.len() as u64));
        assert!(gadgets.iter().any(|g| g["kind"] == "placed"), "round {n}");
        assert!(gadgets.iter().any(|g| g["kind"] == "thrown"), "round {n}");
        assert!(
            gadgets.iter().all(|g| g["name"].is_string()),
            "round {n}: a gadget without a name"
        );
        assert_eq!(list(&round, "mapCameras").len(), CAMERAS, "round {n}");
    }
}

#[test]
fn gadgets_of_the_test_rounds_hold_the_invariants() {
    let failures: Vec<String> = rounds()
        .iter()
        .flat_map(|(n, round)| violations(&format!("round {n}"), round))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_count_drop_of_a_placed_gadget_has_its_object() {
    let rounds = rounds();
    if rounds.is_empty() {
        return;
    }
    let (mut all, mut matched, mut spare) = (0, 0, Vec::new());
    for (n, round) in &rounds {
        let (a, m, s) = drops(&format!("round {n}"), round);
        all += a;
        matched += m;
        spare.extend(s);
    }
    assert_eq!((all, matched), (DROPS, DROPS));
    assert!(spare.is_empty(), "{}", spare.join("\n"));
}

#[test]
fn a_destroyed_camera_of_the_map_says_when() {
    for (n, round) in rounds() {
        let cameras = list(&round, "mapCameras");
        let destroyed: Vec<f64> = cameras
            .iter()
            .filter_map(|c| seconds(&c["destroyed"]))
            .collect();
        assert!(!destroyed.is_empty(), "round {n}: no camera was destroyed");
        assert!(destroyed.len() < cameras.len(), "round {n}");
    }
}

#[test]
fn gadgets_of_the_real_folder_hold_the_invariants() {
    let Some(dir) = std::env::var_os("R6_MATCH_REPLAY") else {
        return;
    };
    let mut failures = Vec::new();
    let mut seen = 0;
    for path in replays(Path::new(&dir)) {
        let Ok(round) = Round::open(&path, ReadMode::Full) else {
            continue;
        };
        let round = serde_json::to_value(&round).unwrap();
        if list(&round, "gadgets").is_empty() {
            continue;
        }
        seen += 1;
        let label = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
        failures.extend(violations(label, &round));
    }
    eprintln!("{seen} real rounds with gadgets");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
