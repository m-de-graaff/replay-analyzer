//! Destruction, surfaces and breaches of the Y11S3 test replays, checked
//! on the JSON of a round. Data is read from `R6_TEST_DATA`, else
//! `test_recordings/`; the tests are skipped when neither has replays.
//! With `R6_MATCH_REPLAY` set, what must hold for any round is checked on
//! the rounds of that folder; it is only read.

use std::collections::HashSet;
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

/// The test round `custom_<n>.rec` as JSON.
fn custom(dir: &Path, n: usize) -> Value {
    let path = dir.join("valid/Y11S3").join(format!("custom_{n}.rec"));
    let round = Round::open(&path, ReadMode::Full).unwrap();
    serde_json::to_value(&round).unwrap()
}

fn list<'a>(round: &'a Value, key: &str) -> &'a [Value] {
    round[key].as_array().map_or(&[], Vec::as_slice)
}

/// The `decodeStatus` entry of `field`.
fn status<'a>(round: &'a Value, field: &str) -> Option<&'a Value> {
    let fields = round["decodeStatus"]["fields"].as_array()?;
    fields.iter().find(|f| f["field"] == field)
}

const OUTCOMES: [&str; 10] = [
    "detonated",
    "destroyed",
    "removed",
    "armedAtEnd",
    "deletedWithoutDamage",
    "noDestruction",
    "partly",
    "none",
    "burned",
    "cancelled",
];
const LABELS: [&str; 5] = [
    "verticalPlay",
    "rotationHole",
    "breach",
    "murderHole",
    "bulletHoles",
];
const KINDS: [&str; 8] = [
    "wall",
    "floor",
    "hatch",
    "object",
    "reinforcedWall",
    "reinforcedHatch",
    "barricade",
    "entity",
];

/// What one round held, added up over the rounds checked.
#[derive(Debug, Default)]
struct Counts {
    rounds: usize,
    events: usize,
    surfaces: usize,
    breaches: usize,
    opened: usize,
    stopped: usize,
}

/// Checks what must hold for the destruction of any round. `name` names
/// the round in a failure.
fn check(round: &Value, name: &str, counts: &mut Counts) {
    let players: HashSet<&str> = list(round, "players")
        .iter()
        .filter_map(|p| p["username"].as_str())
        .collect();
    let is_player = |v: &Value| v.as_str().is_some_and(|u| players.contains(u));
    counts.rounds += 1;

    for e in list(round, "destruction") {
        counts.events += 1;
        // Bullets are counted in the surfaces, not listed.
        assert_ne!(e["cause"]["category"], "bullet", "{name}: {e}");
        assert!(e["cause"]["id"].is_u64(), "{name}: {e}");
        assert!(e["cause"]["category"].is_string(), "{name}: {e}");
        if !e["username"].is_null() {
            assert!(is_player(&e["username"]), "{name}: {e}");
        }
        // Debris and props colliding are counted, not listed.
        if e["cause"]["category"] == "physics" {
            assert!(e["username"].is_string(), "{name}: {e}");
        }
        assert!(e["recordingTime"].is_f64(), "{name}: {e}");
        let objects = list(e, "objects");
        assert!(!objects.is_empty(), "{name}: {e}");
        let mut seen = HashSet::new();
        for o in objects {
            let id = o["object"].as_str().unwrap_or_default();
            assert!(seen.insert(id), "{name}: {id} twice in {e}");
            let kind = o["kind"].as_str().unwrap_or_default();
            assert!(KINDS.contains(&kind), "{name}: {o}");
            // What a map object is, is never read: it says where it is from.
            let read = matches!(
                kind,
                "reinforcedWall" | "reinforcedHatch" | "barricade" | "entity" | "object"
            );
            if !read {
                let source = o["kindSource"].as_str().unwrap_or_default();
                let known = ["catalog", "impacts", "derived"].contains(&source);
                assert!(known, "{name}: {o}");
            }
        }
    }

    for s in list(round, "surfaces") {
        counts.surfaces += 1;
        assert_eq!(s["derived"], true, "{name}: {s}");
        let label = s["label"].as_str().unwrap_or_default();
        assert!(LABELS.contains(&label), "{name}: {s}");
        assert!(["wall", "floor"].contains(&s["kind"].as_str().unwrap_or_default()));
        assert!(s["points"].as_u64() > Some(0), "{name}: {s}");
        assert!(s["width"].as_f64() >= Some(0.0), "{name}: {s}");
        assert!(s["height"].as_f64() >= Some(0.0), "{name}: {s}");
        let makers = s["makers"].as_object().unwrap();
        assert!(
            makers.keys().all(|u| players.contains(u.as_str())),
            "{name}: {s}"
        );
        let made: u64 = makers.values().filter_map(Value::as_u64).sum();
        let caused: u64 = (s["causes"].as_object().unwrap().values())
            .filter_map(Value::as_u64)
            .sum();
        assert_eq!(Some(caused), s["points"].as_u64(), "{name}: {s}");
        assert!(made <= caused, "{name}: {s}");
        assert!(
            s["until"].as_f64() >= s["recordingTime"].as_f64(),
            "{name}: {s}"
        );
    }

    for b in list(round, "breaches") {
        counts.breaches += 1;
        assert!(b["device"].is_string(), "{name}: {b}");
        let outcome = b["outcome"].as_str().unwrap_or_default();
        assert!(OUTCOMES.contains(&outcome), "{name}: {b}");
        if !b["username"].is_null() {
            assert!(is_player(&b["username"]), "{name}: {b}");
        }
        // A breach that opened a reinforcement has it among what it
        // affected.
        if b["openedReinforcement"] == true {
            counts.opened += 1;
            let reinforcement = |o: &&Value| {
                let kind = o["kind"].as_str().unwrap_or_default();
                kind.starts_with("reinforced") && o["opened"] == true
            };
            assert!(
                list(b, "affected").iter().any(|o| reinforcement(&o)),
                "{name}: {b}"
            );
        }
        // Who put a reinforcement up is a player of the round.
        let reinforced = list(b, "affected").iter().map(|o| &o["reinforcedBy"]);
        for by in reinforced.chain([&b["reinforcedBy"]]) {
            assert!(by.is_null() || is_player(by), "{name}: {b}");
        }
        // What stopped a device is only ever one of three things seen
        // near it, and says it is a guess by proximity.
        if !b["stoppedBy"].is_null() {
            counts.stopped += 1;
            let by = b["stoppedBy"].as_str().unwrap_or_default();
            assert!(
                ["electricity", "shot", "explosion"].contains(&by),
                "{name}: {b}"
            );
            assert_eq!(b["stoppedBySource"], "proximity", "{name}: {b}");
            assert!(!list(b, "near").is_empty(), "{name}: {b}");
            assert!(matches!(outcome, "destroyed" | "partly"), "{name}: {b}");
        }
        for n in list(b, "near") {
            assert!(n["distance"].as_f64() >= Some(0.0), "{name}: {b}");
        }
    }
}

#[test]
fn the_test_rounds_hold_their_destruction() {
    let Some(dir) = data_dir() else {
        return;
    };
    let mut counts = Counts::default();
    for n in 1..=10 {
        let round = custom(&dir, n);
        let name = format!("custom_{n}");
        let field = status(&round, "destruction")
            .unwrap_or_else(|| panic!("{name}: no destruction status"));
        assert_eq!(field["status"], "decoded", "{name}: {field}");
        // The only thing to say is how many debris events were left out.
        let warnings = list(field, "warnings");
        let debris = |w: &Value| w.as_str().is_some_and(|w| w.contains("debris"));
        assert!(warnings.iter().all(debris), "{name}: {field}");
        assert_eq!(warnings.len(), 1, "{name}: {field}");
        let surfaces = status(&round, "surfaces").unwrap();
        assert_eq!(surfaces["status"], "inferred", "{name}: {surfaces}");
        check(&round, &name, &mut counts);
        // Every breach of the test rounds is by a player.
        for b in list(&round, "breaches") {
            assert!(b["username"].is_string(), "{name}: {b}");
        }
        assert!(!list(&round, "destruction").is_empty(), "{name}");
        assert!(!list(&round, "surfaces").is_empty(), "{name}");
    }
    println!("{counts:?}");
    assert_eq!(counts.rounds, 10);
}

/// Every Exothermic Charge and X-KAIROS volley of the test rounds went
/// off on a reinforcement and opened it.
#[test]
fn thermite_and_hibana_open_reinforcements() {
    let Some(dir) = data_dir() else {
        return;
    };
    let (mut charges, mut volleys) = (0, 0);
    for n in 1..=10 {
        let round = custom(&dir, n);
        for b in list(&round, "breaches") {
            let device = b["device"].as_str().unwrap_or_default();
            if device != "Exothermic Charge" && device != "X-KAIROS" {
                continue;
            }
            assert_eq!(b["outcome"], "detonated", "custom_{n}: {b}");
            assert_eq!(b["openedReinforcement"], true, "custom_{n}: {b}");
            if device == "X-KAIROS" {
                volleys += 1;
                assert_eq!(b["pellets"], b["detonated"], "custom_{n}: {b}");
            } else {
                charges += 1;
            }
        }
    }
    // Thermite in rounds 2, 4 and 6; Hibana in rounds 1 and 3.
    assert_eq!((charges, volleys), (7, 4));
}

/// A reinforcement a breach opened says when, and the breach says who had
/// put it up: a defender. The moment is the breach's.
#[test]
fn an_opened_reinforcement_says_when_and_whose_it_was() {
    let Some(dir) = data_dir() else {
        return;
    };
    let (mut opened, mut named) = (0, 0);
    for n in 1..=10 {
        let round = custom(&dir, n);
        let name = format!("custom_{n}");
        let defenders: HashSet<&str> = list(&round, "players")
            .iter()
            .filter(|p| {
                round["teams"][p["teamIndex"].as_u64().unwrap() as usize]["role"] == "Defense"
            })
            .filter_map(|p| p["username"].as_str())
            .collect();
        let reinforcements = list(&round, "reinforcements");
        for r in reinforcements.iter().filter(|r| !r["opened"].is_null()) {
            opened += 1;
            assert!(r["completed"].is_object(), "{name}: {r}");
            let (up, open) = (
                &r["completed"]["recordingTime"],
                &r["opened"]["recordingTime"],
            );
            assert!(open.as_f64() > up.as_f64(), "{name}: {r}");
        }
        for b in list(&round, "breaches") {
            let mut whose: Vec<&Value> = vec![b];
            whose.extend(list(b, "affected"));
            for o in whose.iter().filter(|o| !o["reinforcedBy"].is_null()) {
                named += 1;
                let by = o["reinforcedBy"].as_str().unwrap_or_default();
                assert!(defenders.contains(by), "{name}: {b}");
            }
            if b["openedReinforcement"] != true {
                continue;
            }
            // Each reinforcement it opened is one of `reinforcements`,
            // opened within a second of the breach's end.
            let end = b["ended"]["recordingTime"].as_f64().unwrap();
            let struck = list(b, "affected").iter().filter(|o| {
                o["kind"]
                    .as_str()
                    .is_some_and(|k| k.starts_with("reinforced"))
                    && o["opened"] == true
            });
            for o in struck {
                assert!(o["reinforcedBy"].is_string(), "{name}: {b}");
                let panel = reinforcements
                    .iter()
                    .find(|r| r["entity"] == o["object"] && r["username"] == o["reinforcedBy"])
                    .unwrap_or_else(|| panic!("{name}: {o}"));
                let at = panel["opened"]["recordingTime"].as_f64();
                assert!(
                    at.is_some_and(|at| (at - end).abs() < 1.0),
                    "{name}: {b} {panel}"
                );
            }
        }
    }
    // 20 breaches opened a reinforcement; other things (a grenade under a
    // hatch) opened more.
    assert!(
        opened >= 20 && named >= 20,
        "{opened} opened, {named} named"
    );
}

/// Round 7: Neskin's first Hard Breach Charge on a reinforced hatch dies
/// 0.14 s after it is armed, with no Shock Wire or Electroclaw, no shot
/// and no explosion near it. The file does not say what stopped it, so
/// nothing is named.
#[test]
fn a_charge_that_died_of_nothing_known_names_no_cause() {
    let Some(dir) = data_dir() else {
        return;
    };
    let round = custom(&dir, 7);
    let destroyed: Vec<&Value> = list(&round, "breaches")
        .iter()
        .filter(|b| b["outcome"] == "destroyed")
        .collect();
    let [charge] = destroyed.as_slice() else {
        panic!("{destroyed:?}");
    };
    assert_eq!(charge["device"], "Hard Breach Charge");
    assert_eq!(charge["username"], "Neskin.L5");
    assert_eq!(charge["targetKind"], "reinforcedHatch");
    assert!(charge["stoppedBy"].is_null(), "{charge}");
    assert!(charge["stoppedBySource"].is_null(), "{charge}");
    assert!(charge["affected"].is_null(), "{charge}");
    // The same player's second charge on the same hatch went off.
    let second = (list(&round, "breaches").iter())
        .find(|b| b["username"] == "Neskin.L5" && b["outcome"] == "detonated")
        .unwrap();
    assert_eq!(second["target"], charge["target"]);
    assert_eq!(second["openedReinforcement"], true);
}

/// A partial read does not walk the streams.
#[test]
fn a_partial_read_has_no_destruction() {
    let Some(dir) = data_dir() else {
        return;
    };
    let path = dir.join("valid").join("Y11S3").join("custom_1.rec");
    let round = Round::open(&path, ReadMode::Partial).unwrap();
    let round = serde_json::to_value(&round).unwrap();
    assert!(status(&round, "destruction").is_none());
    assert!(round["destruction"].is_null());
    assert!(round["surfaces"].is_null());
    assert!(round["breaches"].is_null());
}

#[test]
fn real_rounds_hold_their_destruction() {
    let Some(dir) = std::env::var_os("R6_MATCH_REPLAY").map(PathBuf::from) else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let mut counts = Counts::default();
    for path in replays(&dir) {
        // A round the game is still writing, or of another season, may
        // not read; that is not what is tested here.
        let Ok(round) = Round::open(&path, ReadMode::Full) else {
            continue;
        };
        let round = serde_json::to_value(&round).unwrap();
        if status(&round, "destruction").is_none() {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        check(&round, &name, &mut counts);
    }
    println!("{counts:?} in {}", dir.display());
}
