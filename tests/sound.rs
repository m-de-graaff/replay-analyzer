//! Metal-detector alarms (Y11S3), against the test rounds and, with
//! `R6_MATCH_REPLAY` set, a real `MatchReplay` folder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use replay_analyzer::{ReadMode, Round, Status};

/// Per round of the ten test rounds: alarms, those recorded to their end,
/// and those with a player within reach.
const ALARMS: [(usize, usize, usize); 10] = [
    (10, 10, 10),
    (2, 2, 2),
    (10, 8, 9),
    (13, 11, 12),
    (1, 1, 1),
    (15, 15, 15),
    (9, 9, 9),
    (2, 2, 2),
    (6, 4, 5),
    (11, 11, 11),
];

/// The detectors of Bank, where the ten rounds were played: the entity,
/// where its alarm sounds, and its alarms over the ten rounds.
const DETECTORS: [(&str, [f64; 3], usize); 4] = [
    ("630f39fa15", [-69.3, 21.05, 0.84], 16),
    ("630f39fa59", [-42.03, 18.8, 0.84], 23),
    ("630f39faa3", [-69.3, 21.38, 4.84], 5),
    ("631500877c", [-66.36, 2.0, 0.85], 35),
];

/// The ten rounds of one custom match, in round order: read once, shared
/// by the tests.
fn test_rounds() -> &'static [(String, Round)] {
    static ROUNDS: std::sync::OnceLock<Vec<(String, Round)>> = std::sync::OnceLock::new();
    ROUNDS.get_or_init(|| {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings/valid/Y11S3");
        (1..=10)
            .map(|n| {
                let name = format!("custom_{n}");
                let round = Round::open(dir.join(format!("{name}.rec")), ReadMode::Full).unwrap();
                (name, round)
            })
            .collect()
    })
}

/// What every alarm has to hold, whatever the round.
fn check(name: &str, round: &Round) {
    let mut last = f64::NEG_INFINITY;
    for a in &round.metal_detectors {
        if let Some(by) = &a.username {
            let known = round.header.players.iter().any(|p| &p.username == by);
            assert!(known, "{name}: {a:?}");
        }
        assert!(
            u64::from_str_radix(&a.detector, 16).is_ok(),
            "{name}: {a:?}"
        );
        // An alarm lasts 3.0 s, and is whole from 2.8.
        assert!((0.0..=3.1).contains(&a.seconds), "{name}: {a:?}");
        assert_eq!(a.complete, a.seconds >= 2.8, "{name}: {a:?}");
        let at = a.when.recording_time.unwrap_or(0.0);
        assert!(at >= last, "{name}: alarms out of order");
        last = at;
    }
}

#[test]
fn the_alarms_are_the_reference_decoders() {
    let mut detectors: BTreeMap<&str, ([f64; 3], usize)> = BTreeMap::new();
    for ((name, round), wanted) in test_rounds().iter().zip(ALARMS) {
        check(name, round);
        let alarms = &round.metal_detectors;
        let complete = alarms.iter().filter(|a| a.complete).count();
        let named = alarms.iter().filter(|a| a.username.is_some()).count();
        assert_eq!((alarms.len(), complete, named), wanted, "{name}");
        for a in alarms {
            let position = a.position.unwrap_or_else(|| panic!("{name}: {a:?}"));
            let seen = detectors.entry(&a.detector).or_insert((position, 0));
            // A detector does not move.
            assert_eq!(seen.0, position, "{name}: {a:?}");
            seen.1 += 1;
        }
        let status = round.decode.get("sound").unwrap();
        assert_eq!(status.status, Status::Decoded, "{name}");
        assert_eq!(status.count, alarms.len(), "{name}");
        assert!(status.warnings.is_empty(), "{name}: {:?}", status.warnings);
    }
    let found: Vec<(&str, [f64; 3], usize)> = (detectors.into_iter())
        .map(|(id, (position, alarms))| (id, position, alarms))
        .collect();
    assert_eq!(found, DETECTORS);
}

#[test]
fn the_json_has_the_keys_of_an_alarm() {
    let (_, round) = &test_rounds()[1];
    let json = serde_json::to_value(round).unwrap();
    let alarms = json["metalDetectors"].as_array().unwrap();
    assert_eq!(alarms.len(), 2);
    let a = &alarms[0];
    assert_eq!(a["detector"], "631500877c");
    assert_eq!(a["position"], serde_json::json!([-66.36, 2.0, 0.85]));
    assert_eq!(a["username"], "pino.L5");
    assert_eq!(a["complete"], true);
    assert_eq!(a["seconds"], 2.953);
    assert_eq!(a["recordingTime"], 178.384);
    assert_eq!(a["phase"], "Action");
    assert!(a["time"].is_string() && a["elapsed"].is_number(), "{a}");
}

fn match_replay_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("R6_MATCH_REPLAY")?);
    dir.is_dir().then_some(dir)
}

/// Real rounds: the sound stream parses to its last byte in every finished
/// round, and a detector is where it was in every round on its map.
#[test]
fn real_sound_streams_parse_to_their_last_byte() {
    let Some(root) = match_replay_dir() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let (mut rounds, mut finished, mut unparsed) = (0, 0, 0);
    // Per map: rounds, alarms, complete ones and those with a player.
    let mut maps: BTreeMap<String, [usize; 4]> = BTreeMap::new();
    let mut places: BTreeMap<(String, String), [f64; 3]> = BTreeMap::new();
    for dir in replay_analyzer::matches::find_match_folders(&root).unwrap() {
        let folder = replay_analyzer::Match::open_with(&dir, ReadMode::Full).unwrap();
        for round in &folder.rounds {
            // Names the round without naming its players.
            let name = format!(
                "{} R{}",
                round.header.match_id,
                round.header.round_number + 1
            );
            let Some(status) = round.decode.get("sound") else {
                continue;
            };
            rounds += 1;
            check(&name, round);
            if round.container.as_ref().is_some_and(|c| c.complete) {
                finished += 1;
                assert!(status.warnings.is_empty(), "{name}: {:?}", status.warnings);
            } else if !status.warnings.is_empty() {
                unparsed += 1;
            }
            let map = round.header.map.name().unwrap_or("?").to_string();
            let counts = maps.entry(map.clone()).or_default();
            counts[0] += 1;
            for a in &round.metal_detectors {
                counts[1] += 1;
                counts[2] += usize::from(a.complete);
                counts[3] += usize::from(a.username.is_some());
                let Some(position) = a.position else { continue };
                let seen = places
                    .entry((map.clone(), a.detector.clone()))
                    .or_insert(position);
                assert_eq!(*seen, position, "{name}: {a:?}");
            }
        }
    }
    for (map, [rounds, alarms, complete, named]) in &maps {
        eprintln!("{map}: {rounds} rounds, {alarms} alarms, {complete} complete, {named} named");
    }
    eprintln!(
        "{rounds} rounds, {finished} finished; {unparsed} unfinished ones with sound records \
         that do not parse; {} detectors",
        places.len()
    );
    assert!(rounds > 0, "no Y11S3 rounds under {}", root.display());
}
