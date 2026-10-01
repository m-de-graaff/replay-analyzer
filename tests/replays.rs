//! Tests against real replays. Data is read from `R6_TEST_DATA`, else
//! `test_recordings/`. Each holds `valid/**/*.rec` (optionally with `.rec.json`
//! expectations) and `invalid/*.rec`. Tests are skipped when none exists.

use std::path::{Path, PathBuf};

use replay_analyzer::{Error, ReadMode, Round, TeamRole};
use serde_json::Value;

fn data_dir() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidates = match std::env::var_os("R6_TEST_DATA") {
        Some(dir) => vec![PathBuf::from(dir)],
        None => vec![root.join("test_recordings")],
    };
    let found = candidates
        .iter()
        .find(|d| d.join("valid").is_dir())
        .cloned();
    if found.is_none() {
        eprintln!("skipping: no test replays in {candidates:?}");
    }
    found
}

fn replays(dir: &Path, kind: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.join(kind)];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
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

/// Missing keys and zero values are equivalent, so drop zero values on both
/// sides. `{name, id}` values compare by id only: some expectation files carry
/// names from before an id was given one.
fn normalize(v: Value) -> Value {
    match v {
        Value::Object(map)
            if map.len() == 2 && map.contains_key("name") && map.contains_key("id") =>
        {
            map["id"].clone()
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, normalize(v)))
                .filter(|(_, v)| !is_zero(v))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(normalize).collect()),
        v => v,
    }
}

fn is_zero(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Bool(b) => !b,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
    }
}

fn diff(path: &str, got: &Value, want: &Value, out: &mut Vec<String>) {
    match (got, want) {
        (Value::Object(g), Value::Object(w)) => {
            for k in g.keys().chain(w.keys().filter(|k| !g.contains_key(*k))) {
                let (g, w) = (
                    g.get(k).unwrap_or(&Value::Null),
                    w.get(k).unwrap_or(&Value::Null),
                );
                diff(&format!("{path}.{k}"), g, w, out);
            }
        }
        (Value::Array(g), Value::Array(w)) if g.len() == w.len() => {
            for (i, (g, w)) in g.iter().zip(w).enumerate() {
                diff(&format!("{path}[{i}]"), g, w, out);
            }
        }
        _ if got != want => out.push(format!("{path}: got {got}, want {want}")),
        _ => {}
    }
}

/// Valid replays that have a `.rec.json` expectation next to them.
fn with_expectations(dir: &Path) -> Vec<PathBuf> {
    replays(dir, "valid")
        .into_iter()
        .filter(|p| p.with_extension("rec.json").is_file())
        .collect()
}

fn expected(path: &Path) -> Value {
    let text = std::fs::read_to_string(path.with_extension("rec.json")).unwrap();
    normalize(serde_json::from_str(&text).unwrap())
}

/// Fields the expectation files do not carry.
const EXTENSIONS: &[&str] = &[
    "weapon",
    "key",
    "relation",
    "party",
    "entities",
    "spawnPosition",
    "targetProfileID",
    "creditedTo",
];

fn strip_extensions(v: Value, extra: &[&str]) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(k, _)| !EXTENSIONS.contains(&k.as_str()) && !extra.contains(&k.as_str()))
                .map(|(k, v)| (k, strip_extensions(v, extra)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|v| strip_extensions(v, extra))
                .collect(),
        ),
        v => v,
    }
}

fn check(path: &Path, key: &str, got: Value, want: &Value, failures: &mut Vec<String>) {
    let mut diffs = Vec::new();
    diff(
        key,
        // Feed entries now name their players' profile ids too.
        &normalize(strip_extensions(
            got,
            if key == "matchFeedback" {
                &["profileID"]
            } else {
                &[]
            },
        )),
        &want[key],
        &mut diffs,
    );
    if !diffs.is_empty() {
        failures.push(format!("{}:\n  {}", path.display(), diffs.join("\n  ")));
    }
}

#[test]
fn full_read_matches_expected_json() {
    let Some(dir) = data_dir() else { return };
    let mut failures = Vec::new();
    for path in with_expectations(&dir) {
        let round = Round::open(&path, ReadMode::Full).unwrap();
        let want = expected(&path);
        check(
            &path,
            "header",
            serde_json::to_value(&round.header).unwrap(),
            &want,
            &mut failures,
        );
        check(
            &path,
            "matchFeedback",
            serde_json::to_value(&round.match_feedback).unwrap(),
            &want,
            &mut failures,
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn partial_read_finds_defenders() {
    let Some(dir) = data_dir() else { return };
    for path in with_expectations(&dir) {
        let round = Round::open(&path, ReadMode::Partial).unwrap();
        let want = expected(&path);
        let want_players = want["header"]["players"].as_array().unwrap();
        assert_eq!(
            round.header.players.len(),
            want_players.len(),
            "{}",
            path.display()
        );
        let mut got: Vec<_> = round
            .header
            .players
            .iter()
            .filter(|p| p.operator.role() == Some(TeamRole::Defense))
            .map(|p| (p.id, p.username.clone(), p.operator.0))
            .collect();
        let mut expect: Vec<_> = want_players
            .iter()
            .filter(|p| got.iter().any(|g| g.2 == p["operator"].as_u64().unwrap()))
            .map(|p| {
                (
                    p["id"].as_u64().unwrap_or(0),
                    p["username"].as_str().unwrap().to_owned(),
                    p["operator"].as_u64().unwrap(),
                )
            })
            .collect();
        got.sort();
        expect.sort();
        assert_eq!(got, expect, "{}", path.display());
    }
}

/// Every valid replay, with or without an expectation file, must parse into a
/// full round: 10 players with operators, both team roles, and one winner.
#[test]
fn every_valid_replay_parses() {
    let Some(dir) = data_dir() else { return };
    let mut failures = Vec::new();
    for path in replays(&dir, "valid") {
        let problem = match Round::open(&path, ReadMode::Full) {
            Err(e) => Some(e.to_string()),
            Ok(r) => {
                let h = &r.header;
                if h.players.len() != 10 || h.players.iter().any(|p| p.operator.name().is_none()) {
                    Some(format!(
                        "{} players, unknown operators: {:?}",
                        h.players.len(),
                        h.players
                            .iter()
                            .filter(|p| p.operator.name().is_none())
                            .map(|p| p.operator.0)
                            .collect::<Vec<_>>()
                    ))
                } else if h.teams.iter().any(|t| t.role.is_none()) {
                    Some("team roles not derived".into())
                } else if h.teams.iter().filter(|t| t.won).count() != 1 {
                    Some("no single winner".into())
                } else {
                    None
                }
            }
        };
        if let Some(p) = problem {
            failures.push(format!("{}: {p}", path.display()));
        }
    }
    assert!(
        failures.is_empty(),
        "{}",
        failures.join(
            "
"
        )
    );
}

#[test]
fn invalid_replays_are_rejected() {
    let Some(dir) = data_dir() else { return };
    for path in replays(&dir, "invalid") {
        let result = Round::open(&path, ReadMode::Full);
        assert!(result.is_err(), "{} should fail", path.display());
    }
    assert!(matches!(
        Round::from_bytes(b"hello", ReadMode::Full),
        Err(Error::InvalidFile)
    ));
}

/// Y8S4+ replays keep the header uncompressed and split the body into many
/// zstd frames. No such fixture exists, so re-pack a legacy replay into that
/// layout and check the result is identical.
#[test]
fn chunked_layout_matches_single_stream() {
    let Some(dir) = data_dir() else { return };
    // Needs a pre-Y8S4 replay: those are one zstd stream with the header inside.
    let Some(path) = replays(&dir, "valid").into_iter().find(|p| {
        std::fs::read(p)
            .unwrap()
            .starts_with(&[0x28, 0xB5, 0x2F, 0xFD])
    }) else {
        return;
    };
    let path = &path;
    let raw = std::fs::read(path).unwrap();
    let data = replay_analyzer::decompressed_bytes(&raw).unwrap();
    let (_, _, header_len) = replay_analyzer::header::parse(&data).unwrap();

    let mut chunked = data[..header_len].to_vec();
    for (i, part) in data[header_len..].chunks(1 << 20).enumerate() {
        if i % 3 == 0 {
            chunked.extend_from_slice(b"\x01\x02padding"); // non-zstd bytes between frames
        }
        chunked.extend(zstd::bulk::compress(part, 1).unwrap());
    }

    let legacy = Round::from_bytes(&raw, ReadMode::Full).unwrap();
    let repacked = Round::from_bytes(&chunked, ReadMode::Full).unwrap();
    // The container facts differ by construction: layout, frame count, and
    // the frame index (now inside the first compressed frame).
    let content = |r: &Round| {
        let mut v = serde_json::to_value(r).unwrap();
        for key in ["replay", "timing", "decodeStatus"] {
            v.as_object_mut().unwrap().remove(key);
        }
        v
    };
    assert_eq!(content(&legacy), content(&repacked));
}

/// Y11S3 round from a pro match: fixed facts checked against the game.
fn y11s3(dir: &Path, name: &str) -> Option<Round> {
    let path = dir.join("valid/Y11S3").join(name);
    path.is_file()
        .then(|| Round::open(&path, ReadMode::Full).unwrap())
}

#[test]
fn bans_resolve_to_operators() {
    let Some(dir) = data_dir() else { return };
    let Some(round) = y11s3(&dir, "custom_1.rec") else {
        return;
    };
    let bans: Vec<_> = round
        .bans
        .iter()
        .map(|b| (b.operator.and_then(|o| o.name()), b.role))
        .collect();
    assert_eq!(
        bans,
        [
            (Some("Ace"), TeamRole::Attack),
            (Some("Montagne"), TeamRole::Attack),
            (Some("Kaid"), TeamRole::Defense),
            (Some("Castle"), TeamRole::Defense),
        ]
    );
}

/// Y11S3 caster UI ids are shared by a whole team, so swaps must be linked
/// through the player's state object instead.
#[test]
fn y11s3_attacker_swaps_reach_the_right_player() {
    let Some(dir) = data_dir() else { return };
    let Some(round) = y11s3(&dir, "custom_2.rec") else {
        return;
    };
    let operator = |name: &str| {
        let i = round.player_index_by_username(name).unwrap();
        round.header.players[i].operator.name()
    };
    assert_eq!(operator("soulz1.FaZe"), Some("Flores"));
    assert_eq!(operator("vitaking.FaZe"), Some("Thermite"));
    assert_eq!(operator("cyber.FaZe"), Some("Nomad"));
    assert_eq!(operator("kds.FaZe"), Some("Sens"));
}

/// Kill weapons, loadouts and health come from separate packets; check they
/// agree with each other on every replay.
#[test]
fn extended_data_is_consistent() {
    let Some(dir) = data_dir() else { return };
    for path in replays(&dir, "valid") {
        let round = Round::open(&path, ReadMode::Full).unwrap();
        let kills: Vec<_> = round
            .match_feedback
            .iter()
            .filter(|u| u.kind == replay_analyzer::MatchUpdateType::Kill)
            .collect();
        // A kill with an item from outside the loadout (picked up, or the
        // environment) is possible but rare.
        let foreign = kills
            .iter()
            .filter(|k| {
                !round.loadouts.iter().any(|l| {
                    l.username == k.username
                        && (l.weapons.contains(&k.weapon) || l.gadgets.contains(&k.weapon))
                })
            })
            .count();
        assert!(
            foreign <= 1,
            "{}: {foreign} kills outside loadouts",
            path.display()
        );

        // Nobody loses health they did not have, and anyone at zero was hit.
        for h in &round.health {
            assert!(
                h.change != 0 && h.health <= 145,
                "{}: {h:?}",
                path.display()
            );
            assert!(
                round.player_index_by_username(&h.username).is_some(),
                "{}: unknown player {}",
                path.display(),
                h.username
            );
        }
        assert!(
            round.observation.iter().all(|o| o.seconds >= 0.0),
            "{}",
            path.display()
        );
    }
}

#[test]
fn y11s3_container_and_version_are_decoded() {
    let Some(dir) = data_dir() else { return };
    let Some(round) = y11s3(&dir, "custom_1.rec") else {
        return;
    };
    let f = &round.format;
    assert!(f.prelude_decoded);
    assert_eq!((f.magic.as_str(), f.format_version), ("dissect", 8));
    assert_eq!(f.property_count, 175);
    assert_eq!(round.header.keys.iter().map(|k| k.1).sum::<u32>(), 175);
    let v = &round.version;
    assert_eq!(v.season.as_deref(), Some("Y11S3"));
    assert_eq!((v.branch.as_str(), v.build), ("Alpha04", 9883691));
    assert_eq!(round.parser.decoder, "Y9S4");
    assert_eq!(round.header.is_spectator, Some(true));
    // The index holds exactly the frames the prelude announces.
    let t = round.timing.as_ref().unwrap();
    assert_eq!(t.frames as u32, f.declared_frames);
    assert!((t.sample_rate - 29.4).abs() < 0.1, "{}", t.sample_rate);
    assert!(t.gaps.is_empty());
    // starttime and endtime (UTC) agree with the frame index, and put the
    // local header timestamp three hours behind UTC.
    assert_eq!(t.started_at.as_deref(), Some("2026-09-12T20:05:51.067Z"));
    assert_eq!(t.header_utc_offset_minutes, Some(-180));
    assert!(
        round.decode.warnings.is_empty(),
        "{:?}",
        round.decode.warnings
    );
    let file = round.file.as_ref().unwrap();
    assert_eq!(file.size, 9141431);
    assert_eq!(file.sha256.len(), 64);
}

#[test]
fn every_valid_replay_has_a_frame_index() {
    let Some(dir) = data_dir() else { return };
    for path in replays(&dir, "valid") {
        let round = Round::open(&path, ReadMode::Full).unwrap();
        let t = round.timing.as_ref().expect("frame index");
        assert_eq!(
            t.frames as u32,
            round.format.declared_frames,
            "{}",
            path.display()
        );
        assert!(
            t.sample_rate > 20.0,
            "{}: {}",
            path.display(),
            t.sample_rate
        );
        // Header-only reads find the same index without the packet data.
        let fast = Round::open(&path, ReadMode::Header).unwrap();
        assert_eq!(fast.timing.map(|t| t.frames), Some(t.frames));
        assert_eq!(fast.header.match_id, round.header.match_id);
    }
}

#[test]
fn decode_status_reports_what_can_be_trusted() {
    use replay_analyzer::Status;
    let Some(dir) = data_dir() else { return };
    for path in replays(&dir, "valid") {
        let round = Round::open(&path, ReadMode::Full).unwrap();
        let status = |f: &str| round.decode.get(f).unwrap().status;
        assert_eq!(status("header"), Status::Decoded, "{}", path.display());
        assert!(status("kills").trusted(), "{}", path.display());
        let result = if round.header.code_version >= replay_analyzer::types::version::Y9S4 {
            Status::Decoded
        } else {
            Status::Inferred
        };
        assert_eq!(status("result"), result, "{}", path.display());
        let partial = Round::open(&path, ReadMode::Partial).unwrap();
        assert_eq!(
            partial.decode.get("result").unwrap().status,
            Status::Skipped
        );
    }
    // Y11S3 scoreboard packets are linked through each player's scoreboard
    // object.
    if let Some(round) = y11s3(&dir, "custom_1.rec") {
        let f = round.decode.get("scoreboard").unwrap();
        assert_eq!((f.status, f.count), (Status::Decoded, 10));
    }
}

#[test]
fn census_counts_known_and_unknown_fields() {
    let Some(dir) = data_dir() else { return };
    for path in replays(&dir, "valid") {
        let options = replay_analyzer::ReadOptions {
            mode: ReadMode::Full,
            census: true,
        };
        let round = Round::open(&path, options).unwrap();
        let c = round.census.as_ref().unwrap();
        // Quick matches have no ban phase, so no operator icons.
        let core = ["player", "time", "legacyTime", "feedback", "health", "item"];
        let missing: Vec<_> = c
            .packets_not_seen
            .iter()
            .filter(|p| core.contains(p))
            .collect();
        assert!(missing.is_empty(), "{}: {missing:?}", path.display());
        let missing: Vec<_> = c
            .fields_not_seen
            .iter()
            .filter(|p| core.contains(p))
            .collect();
        assert!(missing.is_empty(), "{}: {missing:?}", path.display());
        assert!(
            c.unknown_header_keys.is_empty(),
            "{}: {:?}",
            path.display(),
            c.unknown_header_keys
        );
        assert!(c.fields.iter().any(|f| f.known == Some("health")));
        assert!(c.unknown_fields > 50, "{}", path.display());
        // Every stream is there; two have known roles, the rest are listed
        // as unknown so a new one after a patch stands out.
        assert!(c.streams_not_seen.is_empty(), "{:?}", c.streams_not_seen);
        let listed = round.container.as_ref().map_or(0, |c| c.streams.len());
        assert_eq!(c.unknown_streams.len(), listed - 2, "{}", path.display());
        // Fields say where they live: health updates in the state stream,
        // picks in the opening snapshots.
        let stream_of = |name: &str| {
            let f = c.fields.iter().find(|f| f.known == Some(name)).unwrap();
            f.stream.clone()
        };
        assert_eq!(stream_of("health").as_deref(), Some("state"));
        assert_eq!(stream_of("player").as_deref(), Some("snapshot"));
        let time = c.packets.iter().find(|p| p.name.ends_with("ime")).unwrap();
        assert!(time.seen > 100);
    }
}

#[test]
fn y11s3_container_maps_every_byte() {
    use replay_analyzer::DirectoryState;
    let Some(dir) = data_dir() else { return };
    let mut ids = Vec::new();
    for path in replays(&dir, "valid") {
        let raw = std::fs::read(&path).unwrap();
        if !raw.starts_with(b"dissect") {
            continue;
        }
        let round = Round::from_bytes(&raw, ReadMode::Full).unwrap();
        let c = round
            .container
            .as_ref()
            .expect("Y8S4+ files have a container");
        assert!(c.complete, "{}: {:?}", path.display(), c.warnings);
        assert!(
            c.warnings.is_empty(),
            "{}: {:?}",
            path.display(),
            c.warnings
        );
        assert_eq!(c.directory, DirectoryState::Valid);
        assert_eq!(c.streams.len(), 10, "{}", path.display());
        let named: Vec<_> = c.streams.iter().filter_map(|s| s.name).collect();
        assert!(
            named.contains(&"state") && named.contains(&"movement"),
            "{named:?}"
        );
        // Every block was decompressed, and nothing else.
        assert_eq!(c.frames.len(), round.zstd_frames);
        let data = replay_analyzer::decompressed_bytes(&raw).unwrap();
        assert_eq!(data.len() as u64, c.raw_bytes);
        // Listing a folder finds the same map without decompressing; only
        // the record counts need the decompressed data.
        let fast = Round::from_bytes(&raw, ReadMode::Header).unwrap();
        let mut without_records = c.clone();
        for s in &mut without_records.streams {
            assert!(s.records.is_some(), "{}: {}", path.display(), s.hash);
            (s.records, s.record_bytes) = (None, None);
        }
        assert_eq!(fast.container, Some(without_records));
        assert_eq!(
            round.decode.get("container").unwrap().status,
            replay_analyzer::Status::Decoded
        );
        ids.push((c.recording_id.unwrap(), c.streams.len() as u32));
    }
    // The test rounds were recorded one after another by one game session:
    // each takes its main id and ten stream ids from the same counter.
    ids.sort_unstable();
    for w in ids.windows(2) {
        assert_eq!(w[1].0, w[0].0 + 1 + w[0].1, "{ids:?}");
    }
}

#[test]
fn y11s3_events_are_placed_on_the_recording_clock() {
    use replay_analyzer::{MatchUpdateType, Phase};
    let Some(dir) = data_dir() else { return };
    for path in replays(&dir, "valid") {
        let round = Round::open(&path, ReadMode::Full).unwrap();
        if round.container.is_none() {
            continue;
        }
        let name = path.display();
        let t = round.timing.as_ref().unwrap();
        // The game sends updates about 28 times a second, whatever the frame
        // rate of the recording.
        let rate = t.data_rate.expect("state stream records");
        assert!((25.0..32.0).contains(&rate), "{name}: {rate}");
        assert!(t.holes.is_empty(), "{name}: {:?}", t.holes);
        let c = round.container.as_ref().unwrap();
        for stream in ["state", "movement"] {
            let s = c.streams.iter().find(|s| s.name == Some(stream)).unwrap();
            assert!(
                s.records.unwrap() > 1000,
                "{name}: {stream} {:?}",
                s.records
            );
        }

        let feed = &round.match_feedback;
        let times: Vec<f64> = feed.iter().map(|u| u.recording_time.unwrap()).collect();
        assert!(
            times.iter().all(|&s| (0.0..=t.duration).contains(&s)),
            "{name}"
        );
        assert!(times.windows(2).all(|w| w[0] <= w[1]), "{name}: {times:?}");
        assert!(
            round.health.iter().all(|h| h.recording_time.is_some()),
            "{name}"
        );

        // The round clock and the frame index measure the same seconds, so a
        // kill's recording time minus its whole seconds since prep started
        // stays within a second across the round.
        let offsets: Vec<f64> = feed
            .iter()
            .filter(|u| u.kind == MatchUpdateType::Kill && u.phase == Phase::Action)
            .map(|u| u.recording_time.unwrap() - u.elapsed)
            .collect();
        let (lo, hi) = offsets
            .iter()
            .fold((f64::MAX, f64::MIN), |(lo, hi), &o| (lo.min(o), hi.max(o)));
        assert!(offsets.is_empty() || hi - lo < 1.0, "{name}: {offsets:?}");

        let spans = round.timeline.spans();
        let action = spans.iter().find(|s| s.phase == Phase::Action).unwrap();
        let prep = spans.iter().find(|s| s.phase == Phase::Prep).unwrap();
        assert_eq!(prep.recording_end, action.recording_start, "{name}");
        let action_start = action.recording_start.unwrap();
        assert!(
            (40.0..50.0).contains(&action_start),
            "{name}: {action_start}"
        );
        // A plant completes when its countdown reaches zero; the clock
        // switches to the defuser timer an update or two later (34-69 ms in
        // the test rounds), not in the same frame.
        let plant = feed
            .iter()
            .find(|u| u.kind == MatchUpdateType::DefuserPlantComplete);
        let planted = spans.iter().find(|s| s.phase == Phase::Planted);
        if let (Some(plant), Some(planted)) = (plant, planted) {
            let lag = planted.recording_start.unwrap() - plant.recording_time.unwrap();
            assert!(lag > 0.0 && lag < 0.5, "{name}: {lag}");
        }
    }
}

/// Real files the game failed to finish end on a block header whose packed
/// size is 0xFFFFFFFF, right after the snapshots. Build one from a test round.
fn unfinished_copy(raw: &[u8]) -> Vec<u8> {
    let round = Round::from_bytes(raw, ReadMode::Header).unwrap();
    let c = round.container.unwrap();
    // The main stream's descriptor is 56 bytes before its data.
    let cut = c.main.unwrap().offset as usize - 56;
    let mut out = raw[..cut].to_vec();
    out.extend(b"200VRPMC");
    out.extend(10_822_076u32.to_le_bytes());
    out.extend(u32::MAX.to_le_bytes());
    out
}

#[test]
fn a_file_cut_inside_a_block_reads_up_to_the_cut() {
    use replay_analyzer::Status;
    let Some(dir) = data_dir() else { return };
    let path = dir.join("valid/Y11S3/custom_1.rec");
    if !path.is_file() {
        return;
    }
    let raw = std::fs::read(&path).unwrap();
    // Three megabytes in: inside the main stream, in the middle of a block.
    let round = Round::from_bytes(&raw[..3_000_000], ReadMode::Full).unwrap();
    let c = round.container.as_ref().unwrap();
    assert!(!c.complete);
    assert!(c.main.is_some());
    assert!(
        round
            .decode
            .warnings
            .iter()
            .any(|w| w.contains("inside the main stream")),
        "{:?}",
        round.decode.warnings
    );
    assert_eq!(
        round.decode.get("container").unwrap().status,
        Status::Partial
    );
    assert_eq!(round.header.players.len(), 10);
}

#[test]
fn a_file_the_game_did_not_finish_is_reported_incomplete() {
    use replay_analyzer::Status;
    let Some(dir) = data_dir() else { return };
    let path = dir.join("valid/Y11S3/custom_1.rec");
    if !path.is_file() {
        return;
    }
    let raw = unfinished_copy(&std::fs::read(&path).unwrap());
    for mode in [ReadMode::Header, ReadMode::Full] {
        let round = Round::from_bytes(&raw, mode).unwrap();
        let c = round.container.as_ref().unwrap();
        assert!(!c.complete);
        assert!(c.main.is_none());
        assert!(c.streams.iter().all(|s| s.snapshot.is_some()));
        assert_eq!(
            round.decode.get("container").unwrap().status,
            Status::Partial
        );
        assert!(
            round
                .decode
                .warnings
                .iter()
                .any(|w| w.contains("before the main stream")),
            "{:?}",
            round.decode.warnings
        );
        assert!(!round.decode.trusted);
    }
    // A full read still gets the header and players from the snapshots, and
    // says why the kill feed is empty.
    let round = Round::from_bytes(&raw, ReadMode::Full).unwrap();
    assert_eq!(round.header.players.len(), 10);
    let kills = round.decode.get("kills").unwrap();
    assert_eq!(kills.status, Status::Missing);
    assert!(
        kills.warnings.iter().any(|w| w.contains("incomplete")),
        "{:?}",
        kills.warnings
    );
}

/// A `MatchReplay` folder as the game writes it, from `R6_MATCH_REPLAY`.
/// Its contents change as matches are played, so tests on it check what
/// must hold for any such folder.
fn match_replay_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("R6_MATCH_REPLAY")?);
    dir.is_dir().then_some(dir)
}

#[test]
fn game_named_folders_and_files_agree_with_their_headers() {
    let Some(root) = match_replay_dir() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let folders = replay_analyzer::matches::find_match_folders(&root).unwrap();
    assert!(!folders.is_empty());
    for dir in folders {
        let m = replay_analyzer::Match::open_with(&dir, ReadMode::Header).unwrap();
        let f = m.folder.as_ref().unwrap();
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(f.name.is_some(), "{name} is not a game folder name");
        // Every warning is about the match, never about names or ids.
        for w in &f.warnings {
            assert!(
                !w.contains("named for another") && !w.contains("according to its header"),
                "{name}: {w}"
            );
        }
        assert!(f.skipped.is_empty(), "{name}: {:?}", f.skipped);
        for r in &m.rounds {
            let file = Path::new(&r.file.as_ref().unwrap().file_name);
            let round = replay_analyzer::file::round_from_file_name(file);
            assert_eq!(round, Some(r.header.round_number + 1), "{}", file.display());
            let owner = replay_analyzer::file::match_of_file_name(file);
            assert_eq!(owner, Some(name.as_str()), "{}", file.display());
            let c = r.container.as_ref().unwrap();
            let listed = f.incomplete.iter().any(|i| Path::new(i) == file);
            assert_eq!(!c.complete, listed, "{}", file.display());
            assert!(c.recording_id.is_some(), "{}", file.display());
        }
    }
}

/// A game install as the game lays it out: `MatchReplay/Match-.../` round
/// files, a `DissectTmp` folder next to it, and leftovers in both.
#[test]
fn a_library_scan_finds_sessions_copies_and_leftovers() {
    use replay_analyzer::library::{self, DuplicateKind};
    let Some(dir) = data_dir() else { return };
    let src = dir.join("valid/Y11S3");
    if !src.join("custom_2.rec").is_file() {
        return;
    }
    let game = std::env::temp_dir().join(format!("ra-library-{}", std::process::id()));
    let root = game.join("MatchReplay");
    let a = "Match-2026-09-12_22-00-00-4242";
    let b = "Match-2026-09-12_23-00-00-4242";
    for f in [a, b] {
        std::fs::create_dir_all(root.join(f)).unwrap();
    }
    std::fs::create_dir_all(game.join("DissectTmp")).unwrap();
    std::fs::copy(
        src.join("custom_1.rec"),
        root.join(a).join(format!("{a}-R01.rec")),
    )
    .unwrap();
    std::fs::copy(
        src.join("custom_2.rec"),
        root.join(a).join(format!("{a}-R02.rec")),
    )
    .unwrap();
    // The same round again in another folder, once byte for byte and once
    // with its last byte changed (like another player's recording of it).
    std::fs::copy(
        src.join("custom_1.rec"),
        root.join(b).join(format!("{b}-R01.rec")),
    )
    .unwrap();
    let mut other = std::fs::read(src.join("custom_2.rec")).unwrap();
    *other.last_mut().unwrap() ^= 0xFF;
    std::fs::write(root.join(b).join(format!("{b}-R02.rec")), other).unwrap();
    let leftover = "P4242_600_Y2026_M9_D12_H23_M30_FrameDataStream.tmprec";
    std::fs::write(game.join("DissectTmp").join(leftover), b"x").unwrap();
    std::fs::write(
        game.join("P4242_601_Y2026_M9_D12_H23_M30_StaticData.tmprec"),
        b"",
    )
    .unwrap();

    let lib = library::scan(&root, ReadMode::Header);
    std::fs::remove_dir_all(&game).unwrap();
    let lib = lib.unwrap();

    assert_eq!(lib.folders.len(), 2);
    assert!(lib.folders.iter().all(|f| f.error.is_none()));
    let kinds: Vec<_> = lib.duplicates.iter().map(|d| d.kind).collect();
    assert_eq!(kinds, [DuplicateKind::SameFile, DuplicateKind::SameRound]);
    assert!(lib.duplicates.iter().all(|d| d.files.len() == 2));
    let temps: Vec<_> = lib
        .temporary
        .iter()
        .map(|t| (t.process_id, t.stream_id))
        .collect();
    // Sorted by path: DissectTmp's leftover, then the game folder's.
    assert_eq!(temps, [(Some(4242), Some(600)), (Some(4242), Some(601))]);
    assert_eq!(lib.temporary[0].size, Some(1));
    // Both folders come from process 4242. The copies in the second folder
    // reuse the first folder's ids, so the ids start over: two sessions.
    assert_eq!(lib.sessions.len(), 2);
    assert!(lib.sessions.iter().all(|s| s.process_id == 4242));
    assert_eq!(lib.sessions[0].folders, [a]);
    assert!(
        lib.warnings.iter().any(|w| w.contains("temporary")),
        "{:?}",
        lib.warnings
    );
}

#[test]
fn a_real_library_accounts_for_every_round() {
    let Some(root) = match_replay_dir() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let lib = replay_analyzer::library::scan(&root, ReadMode::Header).unwrap();
    assert!(lib.folders.iter().all(|f| f.error.is_none()));
    let named: Vec<_> = lib
        .folders
        .iter()
        .filter(|f| f.folder.name.is_some())
        .collect();
    let in_sessions: usize = lib.sessions.iter().map(|s| s.folders.len()).sum();
    assert_eq!(in_sessions, named.len());
    let rounds: usize = named.iter().map(|f| f.round_list.len()).sum();
    assert_eq!(lib.sessions.iter().map(|s| s.rounds).sum::<usize>(), rounds);
    // The game never writes a round twice.
    assert!(lib.duplicates.is_empty(), "{:?}", lib.duplicates);
    // Leftover temporary recordings, if any, are named as players reported.
    for t in &lib.temporary {
        assert!(t.process_id.is_some() && t.kind.is_some(), "{t:?}");
    }
}

/// Every real round read in full, match by match.
fn real_rounds() -> Option<Vec<Round>> {
    let root = match_replay_dir()?;
    let mut rounds = Vec::new();
    for dir in replay_analyzer::matches::find_match_folders(&root).unwrap() {
        let m = replay_analyzer::Match::open_with(&dir, ReadMode::Full).unwrap();
        rounds.extend(m.rounds);
    }
    Some(rounds)
}

/// Each team bans operators of the side it plays against, so a ban credited
/// to the team playing the banned operator's side is credited to the wrong
/// team.
#[test]
fn real_bans_are_made_by_the_team_on_the_other_side() {
    let Some(rounds) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let mut wrong = Vec::new();
    for r in &rounds {
        for b in &r.bans {
            let Some(team) = b.team else { continue };
            if r.header.teams[team].role == Some(b.role) {
                wrong.push(format!(
                    "{} R{}: {:?}",
                    r.header.match_id,
                    r.header.round_number + 1,
                    b
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} bans credited to the wrong team, e.g. {:?}",
        wrong.len(),
        &wrong[..wrong.len().min(3)]
    );
}

/// In a player's own recording, the game numbers the player's team 1.
#[test]
fn real_recorders_team_has_team_color_one() {
    let Some(rounds) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let mut checked = 0;
    for r in &rounds {
        let Some(you) = r.header.recording_player() else {
            continue;
        };
        let colors = r.header.teams.each_ref().map(|t| t.color);
        if colors == [None, None] {
            continue; // a file cut before its team objects
        }
        assert_eq!(
            colors[you.team_index],
            Some(1),
            "{} R{}",
            r.header.match_id,
            r.header.round_number + 1
        );
        checked += 1;
    }
    assert!(checked > 0);
}

/// Fails when the game adds a map or gives one a new world id: name it in
/// `MAPS`, from its sites and spawns.
#[test]
fn real_maps_have_names() {
    let Some(root) = match_replay_dir() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let lib = replay_analyzer::library::scan(&root, ReadMode::Header).unwrap();
    let unnamed: Vec<_> = lib
        .folders
        .iter()
        .filter_map(|f| f.summary.as_ref())
        .filter(|s| s.map.base.is_none())
        .map(|s| s.map.id)
        .collect();
    assert!(unnamed.is_empty(), "unnamed map ids: {unnamed:?}");
}

#[test]
fn temporary_recordings_are_refused() {
    let dir = std::env::temp_dir().join(format!("ra-tmprec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("P1_50_Y2026_M9_D12_H17_M05_FrameDataStream.tmprec");
    std::fs::write(&path, b"dissect").unwrap();
    assert!(matches!(
        Round::open(&path, ReadMode::Full),
        Err(Error::TemporaryFile(_))
    ));
    // A folder with only temporary files has no rounds.
    assert!(matches!(
        replay_analyzer::Match::open(&dir),
        Err(Error::InvalidFolder)
    ));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn match_folder_groups_rounds_and_spots_gaps() {
    let Some(dir) = data_dir() else { return };
    let src = dir.join("valid/Y11S3");
    if !src.is_dir() {
        return;
    }
    let tmp = std::env::temp_dir().join(format!("ra-match-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    // Rounds 1, 2, 4 and 10 as R-files, a stray copy, and a temporary file.
    for (from, to) in [(1, "R01"), (2, "R02"), (4, "R04"), (10, "R10"), (2, "copy")] {
        std::fs::copy(
            src.join(format!("custom_{from}.rec")),
            tmp.join(format!("{to}.rec")),
        )
        .unwrap();
    }
    std::fs::write(tmp.join("x_StaticData.tmprec"), b"").unwrap();

    let m = replay_analyzer::Match::open_with(&tmp, ReadMode::Header).unwrap();
    let f = m.folder.as_ref().unwrap();
    std::fs::remove_dir_all(&tmp).unwrap();
    assert_eq!(f.round_files, 5);
    assert_eq!(f.rounds, vec![1, 2, 4, 10]);
    assert_eq!(f.missing_rounds, vec![3, 5, 6, 7, 8, 9]);
    assert_eq!(f.match_ids.len(), 1);
    assert_eq!(f.complete, Some(true));
    assert_eq!(f.final_score, Some([3, 7]));
    let skipped: Vec<_> = f.skipped.iter().map(|s| s.file.as_str()).collect();
    assert_eq!(skipped, ["x_StaticData.tmprec", "copy.rec"]);
    let order: Vec<_> = m.rounds.iter().map(|r| r.header.round_number).collect();
    assert_eq!(order, [0, 1, 3, 9]);
}

#[test]
fn match_summary_describes_the_whole_match() {
    use replay_analyzer::summary::Outcome;
    let Some(dir) = data_dir() else { return };
    let src = dir.join("valid/Y11S3");
    if !src.is_dir() {
        return;
    }
    // Header-only reads must be enough for match history.
    let m = replay_analyzer::Match::open_with(&src, ReadMode::Header).unwrap();
    let s = m.summary().unwrap();
    assert_eq!(s.match_id, "8f0fe7b9-461c-4294-95a1-be4d25709562");
    assert!(!s.start_time_is_local);
    assert_eq!(s.start_time.to_rfc3339(), "2026-09-12T20:05:51.067+00:00");
    assert!(s.end_time.unwrap() > s.start_time);
    assert_eq!(s.queue, "custom");
    assert_eq!(s.game_mode.name(), Some("Bomb"));
    assert_eq!(s.map.name, "BankY10");
    assert_eq!(s.map.base.as_deref(), Some("Bank"));
    assert_eq!(s.map.version.as_deref(), Some("Y10"));
    assert_eq!(s.rules.rounds_to_win, 7);
    assert_eq!(s.rules.overtime_rounds_to_win, Some(8));
    assert_eq!(s.rules.max_players_per_team, Some(5));
    assert_eq!(s.rules.game_mode_settings.len(), 61);
    assert_eq!(s.teams[0].name, "LUCKY FIVE");
    assert_eq!(s.teams[1].name, "FAZE CLAN");
    assert_eq!(s.teams[0].players.len(), 5);
    assert_eq!(s.teams[0].starting_side, Some(TeamRole::Defense));
    assert!(s.recording.spectator);
    assert_eq!(s.your_team, None);
    assert_eq!(s.result.final_score, [3, 7]);
    assert_eq!(s.result.winner, Some(1));
    assert_eq!(s.result.outcome, Outcome::Decided);
    assert_eq!(s.result.ended_early, Some(false));
    assert!(s.result.complete && !s.result.overtime);

    assert_eq!(s.rounds.len(), 10);
    // Sides swap after six rounds of twelve.
    assert_eq!(s.rounds[5].sides[0], Some(TeamRole::Defense));
    assert_eq!(s.rounds[6].sides[0], Some(TeamRole::Attack));
    // FaZe went into the last round on 6.
    assert_eq!(s.rounds[9].score_before, [3, 6]);
    assert_eq!(s.rounds[9].match_point, [false, true]);
    assert!(
        s.rounds[..9]
            .iter()
            .all(|r| r.match_point == [false, false])
    );
    let wins: Vec<_> = s.rounds.iter().map(|r| r.winner.unwrap()).collect();
    assert_eq!(wins, [0, 0, 1, 1, 1, 1, 1, 0, 1, 1]);
}

#[test]
fn y11s3_bans_levels_and_picks() {
    let Some(dir) = data_dir() else { return };
    let Some(r10) = y11s3(&dir, "custom_10.rec") else {
        return;
    };
    // Each team bans operators of the side it plays against.
    let bans: Vec<_> = r10
        .bans
        .iter()
        .map(|b| (b.team, b.operator.and_then(|o| o.name())))
        .collect();
    assert_eq!(
        bans,
        [
            (Some(0), Some("Castle")),
            (Some(0), Some("Wamai")),
            (Some(0), Some("Goyo")),
            (Some(1), Some("Ace")),
            (Some(1), Some("Hibana")),
            (Some(1), Some("Ying")),
        ]
    );
    let level = |name: &str| {
        r10.header
            .players
            .iter()
            .find(|p| p.username == name)
            .and_then(|p| p.level)
    };
    assert_eq!(level("WIZARD.L5"), Some(1039));
    assert_eq!(level("cyber.FaZe"), Some(364));
    assert!(r10.header.players.iter().all(|p| p.level.is_some()));

    let m = replay_analyzer::Match::open(dir.join("valid/Y11S3")).unwrap();
    let s = m.summary().unwrap();
    assert_eq!(s.teams[1].players[2].level, Some(364));
    // Bassetto swapped from Montagne to Ying in round 7.
    let pick = s.rounds[6]
        .picks
        .iter()
        .find(|p| p.username == "Bassetto.L5")
        .unwrap();
    let ops: Vec<_> = pick.operators.iter().map(|o| o.name().unwrap()).collect();
    assert_eq!(ops, ["Montagne", "Ying"]);
    assert!(
        s.rounds
            .iter()
            .all(|r| r.picks.len() == 10 && r.bans.len() >= 4)
    );
}

/// Every Y11S3 round: the end reason from events agrees with the header's
/// winner, the clock resolves into prep then action, and ten players start.
#[test]
fn y11s3_rounds_end_and_phase_consistently() {
    use replay_analyzer::{Phase, ReasonSource, WinCondition};
    let Some(dir) = data_dir() else { return };
    let mut reasons = Vec::new();
    for n in 1..=10 {
        let Some(round) = y11s3(&dir, &format!("custom_{n}.rec")) else {
            return;
        };
        let info = round.info();
        assert_eq!(info.number, n, "custom_{n}");
        assert_eq!(
            info.end_reason_source,
            ReasonSource::Confirmed,
            "custom_{n}: {:?}",
            info.warnings
        );
        assert_eq!(info.players_at_start, Some([5, 5]), "custom_{n}");
        let phases: Vec<Phase> = info.phases.iter().map(|p| p.phase).collect();
        assert_eq!(&phases[..2], [Phase::Prep, Phase::Action], "custom_{n}");
        assert_eq!(phases.last(), Some(&Phase::End), "custom_{n}");
        assert_eq!(info.planted, phases.contains(&Phase::Planted), "custom_{n}");
        // One timeline: events never go back in time.
        let elapsed: Vec<f64> = round.match_feedback.iter().map(|u| u.elapsed).collect();
        assert!(
            elapsed.windows(2).all(|w| w[0] <= w[1]),
            "custom_{n}: {elapsed:?}"
        );
        assert!(
            round.timing.as_ref().unwrap().clock_gaps.is_empty(),
            "custom_{n}"
        );
        reasons.push(info.end_reason.unwrap());
    }
    use WinCondition::*;
    assert_eq!(
        reasons,
        [
            KilledOpponents,
            DisabledDefuser,
            KilledOpponents,
            KilledOpponents,
            KilledOpponents,
            KilledOpponents,
            DisabledDefuser,
            KilledOpponents,
            KilledOpponents,
            KilledOpponents,
        ]
    );
}

/// Y11S3 defuser objects: plants and disables with their clock, and the
/// player named when only one of that side was alive.
#[test]
fn y11s3_defuser_plants_and_disables() {
    use replay_analyzer::MatchUpdateType::*;
    let Some(dir) = data_dir() else { return };
    let events = |name: &str| -> Option<Vec<(replay_analyzer::MatchUpdateType, String, String)>> {
        let round = y11s3(&dir, name)?;
        Some(
            round
                .match_feedback
                .iter()
                .filter(|u| matches!(u.kind, DefuserPlantComplete | DefuserDisableComplete))
                .map(|u| (u.kind, u.username.clone(), u.time.clone()))
                .collect(),
        )
    };
    let Some(r2) = events("custom_2.rec") else {
        return;
    };
    assert_eq!(
        r2,
        [
            (DefuserPlantComplete, "soulz1.FaZe".into(), "0:29".into()),
            (DefuserDisableComplete, String::new(), "0:33".into()),
        ]
    );
    // Planted as the action clock hit 0:00; disabled by the last defender.
    let r7 = events("custom_7.rec").unwrap();
    assert_eq!(
        r7,
        [
            (DefuserPlantComplete, String::new(), "0:00".into()),
            (DefuserDisableComplete, "Handyy.FaZe".into(), "0:04".into()),
        ]
    );
    // Started but abandoned plants do not complete.
    assert!(events("custom_10.rec").unwrap().is_empty());
}

/// The kill that ends a round is logged after the clock resets to 0:00; it
/// keeps the last live second. Prep swaps name both operators.
#[test]
fn y11s3_end_clock_and_swaps() {
    use replay_analyzer::Phase;
    let Some(dir) = data_dir() else { return };
    let Some(round) = y11s3(&dir, "custom_1.rec") else {
        return;
    };
    let last = round.match_feedback.last().unwrap();
    assert_eq!((last.time.as_str(), last.phase), ("0:12", Phase::End));

    let round = y11s3(&dir, "custom_3.rec").unwrap();
    let swaps: Vec<_> = round
        .info()
        .swaps
        .iter()
        .map(|s| (s.username.clone(), s.from.name(), s.to.name(), s.late))
        .collect();
    assert_eq!(
        swaps,
        [
            ("Handyy.FaZe".into(), Some("SolidSnake"), Some("IQ"), false),
            ("kds.FaZe".into(), Some("Lion"), Some("Ram"), false),
            (
                "soulz1.FaZe".into(),
                Some("Capitao"),
                Some("Striker"),
                false
            ),
            ("Handyy.FaZe".into(), Some("IQ"), Some("Twitch"), true),
            ("cyber.FaZe".into(), Some("Ash"), Some("Deimos"), true),
        ]
    );
    assert!(round.info().swaps.iter().all(|s| s.phase == Phase::Prep));
}

#[test]
fn match_analytics_add_up() {
    let Some(dir) = data_dir() else { return };
    let src = dir.join("valid/Y11S3");
    if !src.is_dir() {
        return;
    }
    let m = replay_analyzer::Match::open(&src).unwrap();
    let a = m.analytics();
    let won: Vec<u32> = a
        .teams
        .iter()
        .map(|t| t.attack.won + t.defense.won)
        .collect();
    assert_eq!(won, [3, 7]);
    for t in &a.teams {
        assert_eq!(t.attack.played + t.defense.played, 10);
    }
    let site_rounds: u32 = a.sites.iter().map(|s| s.defense.played).sum();
    assert_eq!(site_rounds, 10);
    let reasons: u32 = a.end_reasons.iter().map(|e| e.rounds).sum();
    assert_eq!(reasons, 10);
    // Every attacker spawns once per attacking round.
    let picks: u32 = a.spawns.iter().map(|s| s.picks.played).sum();
    assert_eq!(picks, 5 * 10);
    let operator_rounds: u32 = a.operators.iter().map(|o| o.rounds.played).sum();
    assert_eq!(operator_rounds, 10 * 10);
    let s = m.summary().unwrap();
    let wp = s.rounds[0].win_probability.unwrap();
    assert_eq!(wp, [0.5, 0.5]);
    assert!(s.rounds[9].win_probability.unwrap()[1] > 0.9);
}

/// Every Y11S3 player is linked to their own controller, scoreboard, health
/// and body objects, and has a stable key.
#[test]
fn y11s3_players_link_to_their_objects() {
    let Some(dir) = data_dir() else { return };
    for name in ["custom_1.rec", "custom_5.rec"] {
        let Some(round) = y11s3(&dir, name) else {
            return;
        };
        let players = &round.header.players;
        assert_eq!(players.len(), 10);
        let mut seen = std::collections::HashSet::new();
        for p in players {
            let e = p
                .entities
                .as_ref()
                .unwrap_or_else(|| panic!("{} has no entities", p.username));
            assert_eq!(p.key, p.profile_id, "{}", p.username);
            for id in [Some(e.controller), e.scoreboard, e.health, e.movement] {
                let id = id.unwrap_or_else(|| panic!("{}: missing object", p.username));
                assert!(seen.insert(id), "{}: object {id:08x} shared", p.username);
            }
            let pos = p.spawn_position.unwrap();
            assert!(pos.iter().all(|v| v.abs() < 1000.0), "{pos:?}");
            // Pick packets write to the controller.
            assert_eq!(p.state_id, Some(e.controller), "{}", p.username);
        }
        // A spectator recorded this match: nobody is `you`.
        assert!(
            players
                .iter()
                .all(|p| p.relation.is_none() && p.party.is_none())
        );
        for f in ["entities", "movement", "profileIds", "recorder"] {
            let f = round.decode.get(f).unwrap();
            assert_eq!(f.status, replay_analyzer::Status::Decoded, "{f:?}");
        }
    }
}

/// The Y11S3 scoreboard's match totals agree with the kill feed, once kills
/// the scoreboard credits to a teammate (who downed the victim) are counted
/// for that teammate.
#[test]
fn y11s3_scoreboard_totals_match_the_kill_feed() {
    use replay_analyzer::{LifeEventType, Match, MatchUpdateType};
    let Some(dir) = data_dir() else { return };
    let folder = dir.join("valid/Y11S3");
    if !folder.is_dir() {
        return;
    }
    let m = Match::open(&folder).unwrap();
    let mut kills = std::collections::HashMap::<String, u32>::new();
    let mut deaths = std::collections::HashMap::<String, u32>::new();
    let mut credited = 0;
    for r in &m.rounds {
        for u in &r.match_feedback {
            if u.kind != MatchUpdateType::Kill {
                continue;
            }
            assert!(!u.profile_id.is_empty() && !u.target_profile_id.is_empty());
            let who = if u.credited_to.is_empty() {
                &u.username
            } else {
                credited += 1;
                // Credit goes to a teammate of the finisher, and the victim
                // was downed first.
                let team = |n: &str| {
                    r.header
                        .players
                        .iter()
                        .find(|p| p.username == n)
                        .map(|p| p.team_index)
                };
                assert_eq!(team(&u.credited_to), team(&u.username));
                assert!(
                    r.life_events
                        .iter()
                        .any(|e| e.kind == LifeEventType::Down && e.username == u.target)
                );
                &u.credited_to
            };
            *kills.entry(who.clone()).or_default() += 1;
            *deaths.entry(u.target.clone()).or_default() += 1;
        }
        for p in &r.header.players {
            let sb = r.scoreboard_for(p);
            let want = |m: &std::collections::HashMap<String, u32>| {
                Some(m.get(&p.username).copied().unwrap_or(0))
            };
            assert_eq!(
                sb.kills,
                want(&kills),
                "round {} {}",
                r.header.round_number + 1,
                p.username
            );
            assert_eq!(
                sb.deaths,
                want(&deaths),
                "round {} {}",
                r.header.round_number + 1,
                p.username
            );
        }
    }
    assert!(credited > 0);
}

/// Relations and parties hold together on every replay that has them.
#[test]
fn relations_and_parties_are_consistent() {
    use replay_analyzer::Relation;
    let Some(dir) = data_dir() else { return };
    for path in replays(&dir, "valid") {
        let round = Round::open(&path, ReadMode::Partial).unwrap();
        let players = &round.header.players;
        let you: Vec<_> = players
            .iter()
            .filter(|p| p.relation == Some(Relation::You))
            .collect();
        if round.header.is_spectator == Some(true) {
            assert!(you.is_empty(), "{}", path.display());
            continue;
        }
        let Some(you) = you.first() else { continue };
        assert_eq!(
            players
                .iter()
                .filter(|p| p.relation == Some(Relation::You))
                .count(),
            1
        );
        for p in players {
            let same_team = p.team_index == you.team_index;
            match p.relation {
                Some(Relation::Teammate) => assert!(same_team, "{}", path.display()),
                Some(Relation::Opponent) => assert!(!same_team, "{}", path.display()),
                _ => {}
            }
            if p.party.is_some() {
                assert!(
                    same_team,
                    "{}: party member on the other team",
                    path.display()
                );
            }
        }
    }
}
