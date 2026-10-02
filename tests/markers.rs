//! Pings, spots, tracking markers and device markers (Y11S3), against the
//! test rounds and, with `R6_MATCH_REPLAY` set, a real `MatchReplay`
//! folder, which is only read.

use std::path::{Path, PathBuf};

use replay_analyzer::markers::{DeviceOp, Ended, PingKind};
use replay_analyzer::{ReadMode, Round};

/// What the research scripts found in the ten `custom_*.rec` rounds, by
/// round number.
const PINGS: [usize; 10] = [17, 17, 4, 11, 13, 10, 24, 21, 8, 13];
const SPOTS: [usize; 10] = [5, 0, 2, 5, 8, 1, 9, 2, 4, 5];
const TRACKS: [usize; 10] = [6, 6, 8, 16, 6, 2, 0, 4, 0, 0];
/// Device markers added; only the rounds with Solis have any.
const DEVICES: [usize; 10] = [0, 0, 0, 0, 40, 0, 0, 0, 12, 0];

/// The second press of a double ping follows the first within this long,
/// and this close to it (metres).
const DOUBLE_PRESS: f64 = 0.5;
const SAME_SPOT: f64 = 0.5;
/// A shot leaves its player's gun; a spot is on their body. A shot fired
/// within `SAME_MOMENT` seconds of a spot is this close to it over the
/// ground, and no higher above it than a standing operator's eyes.
const SAME_MOMENT: f64 = 0.1;
const ON_THE_BODY: f32 = 1.0;
const EYE_HEIGHT: f32 = 2.0;

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

fn team(round: &Round, username: &str) -> Option<usize> {
    let player = round.header.players.iter().find(|p| p.username == username);
    player.map(|p| p.team_index)
}

fn warnings(round: &Round) -> &[String] {
    let field = round.decode.get("markers");
    field.map_or(&[], |f| &f.warnings)
}

fn finite(position: &[f64]) -> bool {
    position.iter().all(|v| v.is_finite())
}

/// What holds for the markers of every round; the failures, each named by
/// `label`.
fn violations(label: &str, round: &Round) -> Vec<String> {
    let mut out = Vec::new();
    let mut fail = |what: String| out.push(format!("{label}: {what}"));
    for p in &round.pings {
        let of = team(round, &p.username);
        if of.is_none() {
            fail("a ping by nobody".into());
        }
        // The record states the pinger's alliance itself.
        if p.team != of {
            fail(format!("a ping of team {:?} by one of {of:?}", p.team));
        }
        if !finite(&p.position) || p.when.recording_time.is_none() {
            fail(format!("a ping at {:?}", p.position));
        }
        if p.kind == PingKind::LocationRepeat && p.label.is_none() {
            fail("a repeated ping without its label".into());
        }
    }
    for s in &round.spots {
        if team(round, &s.username).is_none() {
            fail("a spot of nobody".into());
        }
        if s.seen_by.is_none() || !finite(&s.position) {
            fail(format!("a spot at {:?} for {:?}", s.position, s.seen_by));
        }
    }
    for t in &round.ability_markers {
        if team(round, &t.username).is_none() {
            fail("a tracking marker on nobody".into());
        }
        if !finite(&t.position) || !t.path.iter().all(|p| finite(p)) {
            fail(format!("a tracking marker at {:?}", t.position));
        }
        // A pulse has no end; any other marker ended or was still there.
        let lasted = t.seconds.is_some_and(|s| s >= 0.0);
        let timed = t.ended.is_some() != t.open && lasted;
        if t.pulse == timed || (t.pulse && (t.open || t.seconds.is_some())) {
            fail(format!("source {} lasting {:?}", t.source.id, t.seconds));
        }
        if t.path.len() == 1 {
            fail("a path of one place".into());
        }
    }
    let mut added: Vec<u64> = Vec::new();
    for d in &round.device_markers {
        // A remove names its marker only.
        let named = d.target.is_some() && d.position.is_some_and(|p| finite(&p));
        if named == (d.op == DeviceOp::Remove) {
            fail(format!("a device marker {:?} at {:?}", d.op, d.position));
        }
        if d.op == DeviceOp::Add {
            if added.contains(&d.handle) {
                fail(format!("device marker {} added twice", d.handle));
            }
            added.push(d.handle);
        }
    }
    out
}

#[test]
fn the_test_rounds_hold_the_markers_the_research_found() {
    let mut failures = Vec::new();
    let mut counts = [[0; 10]; 4];
    let mut kinds = [0; 4];
    let mut ends = [0; 4];
    for (i, (name, round)) in test_rounds().iter().enumerate() {
        assert!(warnings(round).is_empty(), "{name}: {:?}", warnings(round));
        failures.extend(violations(name, round));
        let of = |op: DeviceOp| round.device_markers.iter().filter(move |d| d.op == op);
        counts[0][i] = round.pings.len();
        counts[1][i] = round.spots.len();
        counts[2][i] = round.ability_markers.len();
        counts[3][i] = of(DeviceOp::Add).count();
        for p in &round.pings {
            match p.kind {
                PingKind::Location => kinds[0] += 1,
                PingKind::LocationRepeat => kinds[1] += 1,
                PingKind::EnemyObject => kinds[2] += 1,
                PingKind::TeamObject => kinds[3] += 1,
                PingKind::Other(target) => panic!("{name}: a ping on target {target}"),
            }
            // A plain location has no label; an object always has one.
            let plain = p.kind == PingKind::Location;
            assert_eq!(p.label.is_none(), plain, "{name}: {p:?}");
        }
        for t in &round.ability_markers {
            match (t.pulse, t.ended) {
                (true, _) => ends[0] += 1,
                (false, Some(Ended::Removed)) => ends[1] += 1,
                (false, Some(Ended::Cleared)) => ends[2] += 1,
                (false, None) => ends[3] += 1,
            }
        }
        // Every device marker of the match was removed again, and an
        // update names the entity its marker was added for.
        assert_eq!(of(DeviceOp::Remove).count(), counts[3][i], "{name}");
        for d in of(DeviceOp::Update) {
            let add = of(DeviceOp::Add).find(|a| a.handle == d.handle);
            assert_eq!(add.map(|a| a.target), Some(d.target), "{name}: {d:?}");
        }
        for d in of(DeviceOp::Remove) {
            let add = of(DeviceOp::Add).find(|a| a.handle == d.handle);
            assert!(add.is_some(), "{name}: {d:?}");
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(counts, [PINGS, SPOTS, TRACKS, DEVICES]);
    assert_eq!(PINGS.iter().sum::<usize>(), 138);
    assert_eq!(kinds, [90, 11, 33, 4]);
    // 17 pulses of Lion and Grim, 28 markers removed, 3 cleared by a
    // death, none left when a round ended.
    assert_eq!(ends, [17, 28, 3, 0]);
}

#[test]
fn a_repeated_ping_follows_the_same_players_ping() {
    let mut repeats = 0;
    for (name, round) in test_rounds() {
        for (i, p) in round.pings.iter().enumerate() {
            if p.kind != PingKind::LocationRepeat {
                continue;
            }
            repeats += 1;
            let at = p.when.recording_time.unwrap();
            let before = round.pings[..i].iter().rfind(|b| b.username == p.username);
            let first = before.unwrap_or_else(|| panic!("{name}: {p:?} follows nothing"));
            // On a location the two are at one spot; on an object the
            // second lands next to it.
            assert_ne!(first.kind, PingKind::LocationRepeat, "{name}: {p:?}");
            let apart = (0..3)
                .map(|i| (first.position[i] - p.position[i]).powi(2))
                .sum::<f64>()
                .sqrt();
            assert!(apart <= SAME_SPOT, "{name}: {apart} m apart");
            let gap = at - first.when.recording_time.unwrap();
            assert!((0.0..=DOUBLE_PRESS).contains(&gap), "{name}: {gap} s");
        }
    }
    assert_eq!(repeats, 11);
}

/// A spot is on the spotted player's body: where that player fired a shot
/// in the same moment, the shot left from there.
#[test]
fn a_spot_is_where_the_spotted_player_is() {
    let (mut spots, mut compared) = (0, 0);
    for (name, round) in test_rounds() {
        for s in &round.spots {
            spots += 1;
            let at = s.when.recording_time.unwrap();
            let shots = round.shots.iter().filter(|shot| {
                shot.username.as_deref() == Some(s.username.as_str())
                    && (shot.when.recording_time).is_some_and(|t| (t - at).abs() <= SAME_MOMENT)
            });
            for shot in shots {
                compared += 1;
                let [dx, dy, dz] = [0, 1, 2].map(|i| shot.origin[i] - s.position[i] as f32);
                let ground = (dx * dx + dy * dy).sqrt();
                assert!(
                    ground <= ON_THE_BODY && (0.0..=EYE_HEIGHT).contains(&dz),
                    "{name}: {} spotted {ground} m from their shot, {dz} m under it",
                    s.username
                );
            }
        }
    }
    println!("{compared} shots compared with {spots} spots");
    assert_eq!(spots, 41);
}

fn match_replay_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("R6_MATCH_REPLAY")?);
    dir.is_dir().then_some(dir)
}

/// Real rounds: the marker stream parses to its last byte in every
/// finished round, and every marker kept names a player of its round.
#[test]
fn real_marker_streams_parse_and_name_players() {
    let Some(root) = match_replay_dir() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let (mut rounds, mut finished, mut unparsed, mut warned) = (0, 0, 0, 0);
    let mut counts = [0; 4];
    let mut failures = Vec::new();
    for dir in replay_analyzer::matches::find_match_folders(&root).unwrap() {
        let folder = replay_analyzer::Match::open_with(&dir, ReadMode::Full).unwrap();
        for round in &folder.rounds {
            if round.decode.get("markers").is_none() {
                continue;
            }
            // Names the round without naming its players.
            let name = format!(
                "{} R{}",
                round.header.match_id,
                round.header.round_number + 1
            );
            rounds += 1;
            let whole = round.container.as_ref().is_some_and(|c| c.complete);
            let cut: Vec<&String> = (warnings(round).iter())
                .filter(|w| w.contains("parse"))
                .collect();
            if whole {
                finished += 1;
                assert!(cut.is_empty(), "{name}: {cut:?}");
            } else if !cut.is_empty() {
                unparsed += 1;
            }
            if warnings(round).len() > cut.len() {
                warned += 1;
                println!("{name}: {:?}", warnings(round));
            }
            failures.extend(violations(&name, round));
            counts[0] += round.pings.len();
            counts[1] += round.spots.len();
            counts[2] += round.ability_markers.len();
            counts[3] += round.device_markers.len();
        }
    }
    println!(
        "real rounds: {rounds} with markers, {finished} finished (every record parses to its last byte), {unparsed} unfinished with one that does not, {warned} with markers left out"
    );
    println!(
        "real rounds: {} pings, {} spots, {} tracking markers, {} device marker changes",
        counts[0], counts[1], counts[2], counts[3]
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
