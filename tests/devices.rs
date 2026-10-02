//! Drones and cameras of the Y11S3 test replays, against the reference
//! decode of the same rounds, and, with `R6_MATCH_REPLAY` set, the
//! invariants on a real `MatchReplay` folder; it is only read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use replay_analyzer::devices::{
    BySource, Camera, CaptureKind, DeviceEventType, DeviceKind, Drone, End, EndKind,
};
use replay_analyzer::throws::Slot;
use replay_analyzer::{ReadMode, Round};

/// Per test round: drones, cameras, devices destroyed, pickups and jams.
const COUNTS: [[usize; 5]; 10] = [
    [11, 8, 17, 0, 0],
    [14, 10, 14, 1, 0],
    [10, 8, 11, 0, 1],
    [10, 8, 13, 2, 6],
    [14, 9, 11, 0, 0],
    [10, 8, 10, 1, 0],
    [12, 9, 15, 0, 0],
    [10, 10, 10, 0, 0],
    [13, 7, 12, 0, 0],
    [11, 9, 14, 0, 0],
];

/// Per test round: each destroyed device and who destroyed it, by entity;
/// an empty name where nobody can be named.
const DESTROYED: [&[(&str, &str)]; 10] = [
    &[
        ("60572f1958", "Handyy.FaZe"),
        ("60572f2331", "cyber.FaZe"),
        ("60572f29b7", "soulz1.FaZe"),
        ("60572f46dd", "soulz1.FaZe"),
        ("60572f611d", "Handyy.FaZe"),
        ("60572f73cd", "cyber.FaZe"),
        ("60572f8075", "cyber.FaZe"),
        ("f02b84ef", "Handyy.FaZe"),
        ("f02b8bcc", "Neskin.L5"),
        ("f02b8bf3", "Neskin.L5"),
        ("f02b8c41", "WIZARD.L5"),
        ("f02b8c68", "Bassetto.L5"),
        ("f02ba434", "Bassetto.L5"),
        ("f02ba5f0", "WIZARD.L5"),
        ("f02ba66c", "pino.L5"),
        ("f02ba6b6", "WIZARD.L5"),
        ("f02ba87d", "Bassetto.L5"),
    ],
    &[
        ("60572f1958", "Handyy.FaZe"),
        ("60572f2331", "vitaking.FaZe"),
        ("60572f46dd", "soulz1.FaZe"),
        ("60572f73cd", "cyber.FaZe"),
        ("60572f8075", "cyber.FaZe"),
        ("f028afba", "cyber.FaZe"),
        ("f028b03b", "soulz1.FaZe"),
        ("f028b728", "Neskin.L5"),
        ("f028b776", "pino.L5"),
        ("f028b7be", "WIZARD.L5"),
        ("f028c072", "WIZARD.L5"),
        ("f028c15c", "WIZARD.L5"),
        ("f028c320", "Neskin.L5"),
        ("f028c451", "WIZARD.L5"),
    ],
    &[
        ("60572f1958", "vitaking.FaZe"),
        ("60572f2331", "cyber.FaZe"),
        ("60572f46dd", "soulz1.FaZe"),
        ("60572f611d", "Handyy.FaZe"),
        ("60572f73cd", "kds.FaZe"),
        ("60572f8075", "cyber.FaZe"),
        ("f0290c28", "PSYCHO.L5"),
        ("f029cbab", "WIZARD.L5"),
        ("f029cc20", "Bassetto.L5"),
        ("f029cc79", "PSYCHO.L5"),
        ("f029ccc7", "Neskin.L5"),
    ],
    &[
        ("60572f1958", "Handyy.FaZe"),
        ("60572f2331", "cyber.FaZe"),
        ("60572f46dd", "soulz1.FaZe"),
        ("60572f611d", "cyber.FaZe"),
        ("60572f73cd", "cyber.FaZe"),
        ("60572f8075", "cyber.FaZe"),
        ("f0361916", "PSYCHO.L5"),
        ("f0361948", "Neskin.L5"),
        ("f036196f", "PSYCHO.L5"),
        ("f036327c", "WIZARD.L5"),
        ("f03632b3", "Bassetto.L5"),
        ("f03634b1", "WIZARD.L5"),
        ("f03635a4", "pino.L5"),
    ],
    &[
        ("60572f1958", "kds.FaZe"),
        ("60572f2331", "cyber.FaZe"),
        ("60572f46dd", "soulz1.FaZe"),
        ("60572f73cd", "cyber.FaZe"),
        ("60572f8075", "cyber.FaZe"),
        ("f0370872", "WIZARD.L5"),
        ("f0370899", "WIZARD.L5"),
        ("f03708e7", "PSYCHO.L5"),
        ("f0374a61", "Bassetto.L5"),
        ("f0374b84", "PSYCHO.L5"),
        ("f0374c11", "Neskin.L5"),
    ],
    &[
        ("60572f1958", "Handyy.FaZe"),
        ("60572f2331", "cyber.FaZe"),
        ("60572f46dd", "soulz1.FaZe"),
        ("60572f611d", "Handyy.FaZe"),
        ("60572f73cd", "cyber.FaZe"),
        ("60572f8075", "cyber.FaZe"),
        ("f03475ac", "PSYCHO.L5"),
        ("f03477a4", "WIZARD.L5"),
        ("f0347964", "Neskin.L5"),
        ("f03479d0", "Neskin.L5"),
    ],
    &[
        ("60572f1958", "PSYCHO.L5"),
        ("60572f2331", "WIZARD.L5"),
        ("60572f46dd", "Neskin.L5"),
        ("60572f611d", "WIZARD.L5"),
        ("60572f73cd", "WIZARD.L5"),
        ("60572f8075", "WIZARD.L5"),
        ("f0355009", "cyber.FaZe"),
        ("f03550d7", "Handyy.FaZe"),
        ("f03550fe", "cyber.FaZe"),
        ("f03561b7", "Handyy.FaZe"),
        ("f035633f", ""),
        ("f03564a1", "Handyy.FaZe"),
        ("f0356521", "Handyy.FaZe"),
        ("f0356551", "vitaking.FaZe"),
        ("f035662c", "cyber.FaZe"),
    ],
    &[
        ("60572f1958", "Neskin.L5"),
        ("60572f2331", "Bassetto.L5"),
        ("60572f46dd", "WIZARD.L5"),
        ("60572f73cd", "WIZARD.L5"),
        ("60572f8075", "WIZARD.L5"),
        ("f0326e30", "WIZARD.L5"),
        ("f03274b0", "cyber.FaZe"),
        ("f03275d1", "soulz1.FaZe"),
        ("f033bd9b", "cyber.FaZe"),
        ("f033bebc", "Handyy.FaZe"),
    ],
    &[
        ("60572f1958", "Neskin.L5"),
        ("60572f2331", "WIZARD.L5"),
        ("60572f29b7", "PSYCHO.L5"),
        ("60572f46dd", "pino.L5"),
        ("60572f73cd", "WIZARD.L5"),
        ("60572f8075", "WIZARD.L5"),
        ("f0308d90", "cyber.FaZe"),
        ("f0308db7", "vitaking.FaZe"),
        ("f030d896", "cyber.FaZe"),
        ("f030da17", "kds.FaZe"),
        ("f030db92", "vitaking.FaZe"),
        ("f030dd48", "Handyy.FaZe"),
    ],
    &[
        ("60572f1958", "pino.L5"),
        ("60572f2331", "WIZARD.L5"),
        ("60572f46dd", "Neskin.L5"),
        ("60572f611d", "Bassetto.L5"),
        ("60572f73cd", "WIZARD.L5"),
        ("60572f8075", "WIZARD.L5"),
        ("f031abf1", "PSYCHO.L5"),
        ("f031b496", "kds.FaZe"),
        ("f031b4bd", "vitaking.FaZe"),
        ("f031b50b", "cyber.FaZe"),
        ("f031c689", ""),
        ("f031c8e2", "vitaking.FaZe"),
        ("f031c974", "Handyy.FaZe"),
        ("f031ca66", "vitaking.FaZe"),
    ],
];

/// A deployment is written within this long of the throw it is (seconds).
const THROW_LAG: f64 = 0.15;

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

/// The ten rounds of one custom match, in round order: read once, shared
/// by the tests. Empty without the test replays.
fn test_rounds() -> &'static [Round] {
    static ROUNDS: std::sync::OnceLock<Vec<Round>> = std::sync::OnceLock::new();
    ROUNDS.get_or_init(|| {
        let Some(dir) = data_dir() else {
            return Vec::new();
        };
        (1..=10)
            .map(|n| dir.join(format!("valid/Y11S3/custom_{n}.rec")))
            .map(|path| Round::open(path, ReadMode::Full).unwrap())
            .collect()
    })
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

/// The end of every drone and camera of a round, with its entity.
fn ends(round: &Round) -> impl Iterator<Item = (&str, &End)> {
    let drones = round.drones.iter().map(|d| (d.entity.as_str(), &d.end));
    let cameras = round.cameras.iter().map(|c| (c.entity.as_str(), &c.end));
    (drones.chain(cameras)).filter_map(|(entity, end)| Some((entity, end.as_ref()?)))
}

fn is_drone_kind(kind: DeviceKind) -> bool {
    matches!(
        kind,
        DeviceKind::Drone
            | DeviceKind::ShockDrone
            | DeviceKind::KludgeDrone
            | DeviceKind::Yokai
            | DeviceKind::RceRatero
    )
}

/// Index into the sources counted: `score+shot`, `score`, `shot`, nobody.
fn source_index(end: &End) -> usize {
    match end.by_source {
        Some(BySource::ScoreAndShot) => 0,
        Some(BySource::Score) => 1,
        Some(BySource::Shot) => 2,
        None => 3,
    }
}

/// What holds for the end of the device `entity`.
fn end_violations(label: &str, round: &Round, entity: &str, end: &End) -> Vec<String> {
    let mut out = Vec::new();
    let mut fail = |what: &str| out.push(format!("{label} {entity}: {what}"));
    let players = &round.header.players;
    if (end.by.as_ref()).is_some_and(|by| !players.iter().any(|p| &p.username == by)) {
        fail("destroyed by someone who is no player of the round");
    }
    if end.by.is_some() != end.by_source.is_some() {
        fail("a destroyer without a source, or a source without one");
    }
    if end.kind != EndKind::Destroyed && (end.by.is_some() || end.no_flag) {
        fail("a destroyer of a device that was not destroyed");
    }
    if end.team_kill && end.by.is_none() {
        fail("a team kill by nobody");
    }
    out
}

/// What holds for the devices of every round; the failures, each named by
/// `label` and the device.
fn violations(label: &str, round: &Round) -> Vec<String> {
    let mut out = Vec::new();
    let team = |name: &str| {
        let player = round.header.players.iter().find(|p| p.username == name);
        player.map(|p| p.team_index)
    };
    let known = |name: &Option<String>| name.as_deref().is_none_or(|n| team(n).is_some());
    let entities: Vec<&str> = (round.drones.iter().map(|d| d.entity.as_str()))
        .chain(round.cameras.iter().map(|c| c.entity.as_str()))
        .collect();
    let captured = |entity: &str| {
        let mut events = round.device_events.iter();
        events.any(|e| e.kind == DeviceEventType::Capture && e.device == entity)
    };
    for (entity, end) in ends(round) {
        out.extend(end_violations(label, round, entity, end));
    }

    for d in &round.drones {
        let mut fail = |what: &str| out.push(format!("{label} {}: {what}", d.entity));
        if !is_drone_kind(d.kind) && d.kind != DeviceKind::Unknown {
            fail("a drone of a camera's kind");
        }
        if !known(&d.owner) || d.owner.as_deref().and_then(team) != d.team {
            fail("the owner is no player of the round, or of another team");
        }
        if d.deployments.is_empty() {
            fail("never deployed");
        }
        if d.path.len() > 60 {
            fail("more than 60 path points");
        }
        if d.path.windows(2).any(|w| w[1][0] < w[0][0]) {
            fail("path times go back");
        }
        if (d.path.iter().flatten()).any(|v| !v.is_finite() || v.abs() > 1e4) {
            fail("path leaves the map");
        }
        let ended = d.end.as_ref().and_then(|e| e.when.recording_time);
        let last = d.path.last().map(|p| p[0]);
        if let (Some(ended), Some(last)) = (ended, last)
            && last > ended + 0.001
        {
            fail("a path point after its end");
        }
        for s in &d.sessions {
            if !known(&s.username) {
                fail("driven by someone who is no player of the round");
            }
            let side = s.username.as_deref().and_then(team);
            if side.is_some() && side != d.team && !captured(&d.entity) {
                fail("driven by the other team without a capture");
            }
            if s.seconds.is_some_and(|s| s < 0.0) {
                fail("a session that ends before it starts");
            }
        }
    }
    for c in &round.cameras {
        let mut fail = |what: &str| out.push(format!("{label} {}: {what}", c.entity));
        if is_drone_kind(c.kind) {
            fail("a camera of a drone's kind");
        }
        let map = c.kind == DeviceKind::Default;
        if map && (c.owner.is_some() || c.placed.is_some()) {
            fail("a camera of the map with an owner or a placement");
        }
        if !map && c.placed.is_none() {
            fail("a gadget camera that was never placed");
        }
        if !known(&c.owner) || (c.owner.is_some() && c.owner.as_deref().and_then(team) != c.team) {
            fail("the owner is no player of the round, or of another team");
        }
        if c.position.is_none_or(|p| p.iter().any(|v| !v.is_finite())) {
            fail("no position");
        }
    }

    for (i, e) in round.device_events.iter().enumerate() {
        let mut fail = |what: &str| out.push(format!("{label} device event {i}: {what}"));
        if !entities.contains(&e.device.as_str()) {
            fail("names no drone or camera");
        }
        if !known(&e.by) {
            fail("by someone who is no player of the round");
        }
        if (e.kind == DeviceEventType::Capture) != e.capture.is_some() {
            fail("a capture without a kind, or a kind without one");
        }
        if e.inferred && e.kind != DeviceEventType::Capture {
            fail("only who captured is inferred");
        }
        if e.jammer.is_some() && e.kind != DeviceEventType::Jam {
            fail("a jammer without a jam");
        }
        if e.seconds.is_some_and(|s| s < 0.0) {
            fail("a span that ends before it starts");
        }
    }
    let times = round.device_events.windows(2);
    if times
        .clone()
        .any(|w| w[1].when.recording_time < w[0].when.recording_time)
    {
        out.push(format!("{label}: device events out of order"));
    }

    // The cameras of the map only go, and the last count is what is left.
    for w in round.camera_counts.windows(2) {
        if (0..2).any(|t| w[1].teams[t].default > w[0].teams[t].default) {
            out.push(format!("{label}: a camera of the map came back"));
        }
    }
    if let Some(last) = round.camera_counts.last() {
        for (team, count) in last.teams.iter().enumerate() {
            let alive = |c: &&Camera| c.team == Some(team) && c.end.is_none();
            let map = |c: &&Camera| c.kind == DeviceKind::Default;
            let default = round.cameras.iter().filter(alive).filter(map).count();
            let gadget = round.cameras.iter().filter(alive).count() - default;
            if (count.default as usize, count.gadget as usize) != (default, gadget) {
                out.push(format!(
                    "{label}: the last camera count of team {team} is off"
                ));
            }
        }
    }

    for (i, s) in round.observation.iter().enumerate() {
        if let Some(device) = &s.device
            && !entities.contains(&device.as_str())
        {
            out.push(format!("{label} observation {i}: names no drone or camera"));
        }
    }
    // A drone thrown is a drone deployed, by its owner.
    for (i, t) in round.throws.iter().enumerate() {
        let Some(at) = t
            .when
            .recording_time
            .filter(|_| t.slot == Some(Slot::Drone))
        else {
            continue;
        };
        let deployed = |d: &Drone| {
            let mut times = d.deployments.iter().filter_map(|w| w.recording_time);
            d.owner.as_deref() == Some(&t.username) && times.any(|w| (w - at).abs() <= THROW_LAG)
        };
        if !round.drones.iter().any(deployed) {
            out.push(format!(
                "{label} throw {i}: a drone thrown with no deployment"
            ));
        }
    }
    out
}

#[test]
fn the_test_rounds_agree_with_the_reference() {
    let (mut destroyed, mut sources) = (0, [0; 4]);
    for (i, round) in test_rounds().iter().enumerate() {
        let name = format!("custom_{}", i + 1);
        let gone = || ends(round).filter(|e| e.1.kind == EndKind::Destroyed);
        let mut by: Vec<(&str, &str)> = gone()
            .map(|(entity, end)| (entity, end.by.as_deref().unwrap_or("")))
            .collect();
        by.sort_unstable();
        assert_eq!(by, DESTROYED[i], "{name}: who destroyed what");
        let pickups: usize = round.drones.iter().map(|d| d.pickups.len()).sum();
        let jams = (round.device_events.iter())
            .filter(|e| e.kind == DeviceEventType::Jam)
            .count();
        let counts = [
            round.drones.len(),
            round.cameras.len(),
            by.len(),
            pickups,
            jams,
        ];
        assert_eq!(counts, COUNTS[i], "{name}: drones, cameras, ends");
        assert_eq!(round.device_events.len(), jams, "{name}: only jams");
        for (_, end) in gone() {
            destroyed += 1;
            sources[source_index(end)] += 1;
            assert!(!end.team_kill, "{name}: no team kill in the test rounds");
        }
        let status = round.decode.get("devices").expect("a devices status");
        assert_eq!(status.count, round.drones.len() + round.cameras.len());
        assert!(status.warnings.is_empty(), "{name}: {:?}", status.warnings);
    }
    if !test_rounds().is_empty() {
        assert_eq!((destroyed, sources), (127, [124, 1, 0, 2]));
    }
}

#[test]
fn the_test_rounds_hold_the_invariants() {
    let mut failures = Vec::new();
    for (i, round) in test_rounds().iter().enumerate() {
        failures.extend(violations(&format!("custom_{}", i + 1), round));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Kinds, ends, deployments and sessions the reference found in the test
/// rounds.
#[test]
fn the_test_rounds_have_the_devices_of_the_reference() {
    let rounds = test_rounds();
    if rounds.is_empty() {
        return;
    }
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut ends_of: BTreeMap<String, usize> = BTreeMap::new();
    let (mut deployments, mut sessions, mut thrown) = (0, 0, 0);
    for round in rounds {
        let drones = round.drones.iter().map(|d| d.kind);
        for kind in drones.chain(round.cameras.iter().map(|c| c.kind)) {
            *kinds.entry(format!("{kind:?}")).or_default() += 1;
        }
        for (_, end) in ends(round) {
            *ends_of.entry(format!("{:?}", end.kind)).or_default() += 1;
        }
        for d in &round.drones {
            deployments += d.deployments.len();
            sessions += d.sessions.len();
        }
        let drone = |t: &&replay_analyzer::throws::Throw| t.slot == Some(Slot::Drone);
        thrown += round.throws.iter().filter(drone).count();
        // Each round has the seven cameras of the map, all the defenders'.
        let first = round.camera_counts.first().expect("camera counts");
        let defense = (round.header.teams.iter())
            .position(|t| t.role == Some(replay_analyzer::TeamRole::Defense))
            .expect("a defending team");
        assert_eq!(first.teams[defense].default, 7);
        assert_eq!(first.teams[1 - defense].default, 0);
        assert_eq!(first.when.recording_time, None, "counted in the snapshot");
    }
    let kinds: Vec<(&str, usize)> = kinds.iter().map(|(k, &n)| (k.as_str(), n)).collect();
    assert_eq!(
        kinds,
        [
            ("BlackEye", 8),
            ("Bulletproof", 8),
            ("Default", 70),
            ("Drone", 96),
            ("RceRatero", 12),
            ("ShockDrone", 7),
        ]
    );
    let ends_of: Vec<(&str, usize)> = ends_of.iter().map(|(k, &n)| (k.as_str(), n)).collect();
    assert_eq!(ends_of, [("Destroyed", 127), ("Expired", 12)]);
    // Three of the deployments are throws after a pickup, which `throws`
    // does not have; the others are the drones out as the recording starts
    // and the RCE-Rateros.
    assert_eq!((deployments, sessions, thrown), (119, 330, 47));
}

/// The jams of the test rounds: each names its jammer and who carries it,
/// a player of the other team.
#[test]
fn every_jam_names_its_jammer() {
    for round in test_rounds() {
        for e in &round.device_events {
            assert_eq!(e.kind, DeviceEventType::Jam);
            assert!(e.jammer.is_some() && e.seconds.is_some());
            let by = e.by.as_deref().expect("a jam by a player");
            let jammer = (round.header.players.iter()).find(|p| p.username == by);
            let drone = round.drones.iter().find(|d| d.entity == e.device);
            let (jammer, drone) = (jammer.unwrap(), drone.unwrap());
            assert_ne!(Some(jammer.team_index), drone.team);
        }
    }
}

/// Nearly every observation session is linked to the device it is of.
#[test]
fn observation_sessions_name_their_device() {
    let rounds = test_rounds();
    if rounds.is_empty() {
        return;
    }
    let sessions: usize = rounds.iter().map(|r| r.observation.len()).sum();
    let linked = (rounds.iter().flat_map(|r| &r.observation))
        .filter(|s| s.device.is_some())
        .count();
    assert_eq!((linked, sessions), (650, 652));
    // A session on a drone is of a drone.
    for round in rounds {
        for s in round.observation.iter().filter(|s| s.tool.is_drone()) {
            let Some(device) = &s.device else { continue };
            assert!(round.drones.iter().any(|d| &d.entity == device));
        }
    }
}

/// `right` of `of`, with the share.
fn share(right: usize, of: usize) -> String {
    let percent = 100.0 * right as f64 / of.max(1) as f64;
    format!("{right} of {of} ({percent:.1}%)")
}

/// Real rounds: the invariants hold. How often each signal named the
/// destroyer, and the jams and captures, are printed, not asserted.
#[test]
fn real_rounds_hold_the_invariants() {
    let Some(dir) = std::env::var_os("R6_MATCH_REPLAY").map(PathBuf::from) else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let mut failures = Vec::new();
    let (mut rounds, mut drones, mut cameras, mut warned) = (0, 0, 0, 0);
    let (mut destroyed, mut sources, mut team_kills, mut no_flag) = (0, [0; 4], 0, 0);
    let (mut sessions, mut linked) = (0, 0);
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut events: BTreeMap<String, usize> = BTreeMap::new();
    let mut warnings: BTreeMap<String, usize> = BTreeMap::new();
    for path in replays(&dir) {
        // A round the game is still writing, or of another season, may
        // not read; that is not what is tested here.
        let Ok(round) = Round::open(&path, ReadMode::Full) else {
            continue;
        };
        let Some(status) = round.decode.get("devices") else {
            continue;
        };
        // Names the round without naming its players.
        let label = path.file_name().unwrap_or_default().to_string_lossy();
        failures.extend(violations(&label, &round));
        rounds += 1;
        drones += round.drones.len();
        cameras += round.cameras.len();
        warned += usize::from(!status.warnings.is_empty());
        for w in &status.warnings {
            // Without the count, which differs per round.
            let what = w.split_once(' ').map_or(w.as_str(), |w| w.1);
            *warnings.entry(what.to_owned()).or_default() += 1;
        }
        let all = round.drones.iter().map(|d| d.kind);
        for kind in all.chain(round.cameras.iter().map(|c| c.kind)) {
            *kinds.entry(format!("{kind:?}")).or_default() += 1;
        }
        for (_, end) in ends(&round).filter(|e| e.1.kind == EndKind::Destroyed) {
            destroyed += 1;
            sources[source_index(end)] += 1;
            team_kills += usize::from(end.team_kill);
            no_flag += usize::from(end.no_flag);
        }
        for e in &round.device_events {
            let kind = match e.capture {
                Some(CaptureKind::Pest) => "Capture by a Pest".to_owned(),
                Some(CaptureKind::Kludge) => "Capture by a Kludge Drone".to_owned(),
                None => format!("{:?}", e.kind),
            };
            let named = e.by.is_some() || e.jammer.is_some();
            let spans = matches!(e.kind, DeviceEventType::Jam | DeviceEventType::Capture);
            let kind = match spans && !named {
                true => kind + ", by nobody",
                false => kind,
            };
            *events.entry(kind).or_default() += 1;
        }
        sessions += round.observation.len();
        linked += (round.observation.iter())
            .filter(|s| s.device.is_some())
            .count();
    }
    println!(
        "{drones} drones and {cameras} cameras in {rounds} rounds of {}",
        dir.display()
    );
    println!("kinds: {kinds:?}");
    println!("destroyed: {destroyed}, {team_kills} by a teammate, {no_flag} with no flag");
    let names = ["score+shot", "score", "shot", "nobody named"];
    for (name, count) in names.iter().zip(sources) {
        println!("  {name}: {}", share(count, destroyed));
    }
    println!("device events: {events:?}");
    println!(
        "observation sessions with a device: {}",
        share(linked, sessions)
    );
    println!("rounds with warnings: {warned} {warnings:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
