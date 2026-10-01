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
        // The level's property is named ClearanceLevelText.
        if round.header.code_version >= replay_analyzer::types::version::Y11S3 {
            assert_eq!(status("levels"), Status::Decoded, "{}", path.display());
        }
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
        // A plant completes in the frame the clock switches to the defuser
        // timer in.
        let plant = feed
            .iter()
            .find(|u| u.kind == MatchUpdateType::DefuserPlantComplete);
        let planted = spans.iter().find(|s| s.phase == Phase::Planted);
        if let (Some(plant), Some(planted)) = (plant, planted) {
            let lag = planted.recording_start.unwrap() - plant.recording_time.unwrap();
            assert!(lag.abs() < 0.05, "{name}: {lag}");
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

/// Every real round read in full, match by match: read once, shared by the
/// tests that need it.
fn real_rounds() -> Option<&'static [Round]> {
    static ROUNDS: std::sync::OnceLock<Option<Vec<Round>>> = std::sync::OnceLock::new();
    ROUNDS
        .get_or_init(|| {
            let root = match_replay_dir()?;
            let mut rounds = Vec::new();
            for dir in replay_analyzer::matches::find_match_folders(&root).unwrap() {
                let m = replay_analyzer::Match::open_with(&dir, ReadMode::Full).unwrap();
                rounds.extend(m.rounds);
            }
            Some(rounds)
        })
        .as_deref()
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
    for r in rounds {
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

/// Fails when a ban icon is missing from `ROLE_IMAGES`, or names an operator
/// of the other side.
#[test]
fn real_bans_name_an_operator_of_the_banned_side() {
    let Some(rounds) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let wrong: Vec<_> = rounds
        .iter()
        .flat_map(|r| &r.bans)
        .filter(|b| !b.no_ban && b.operator.and_then(|o| o.role()) != Some(b.role))
        .collect();
    assert!(
        wrong.is_empty(),
        "{} bans, e.g. {:?}",
        wrong.len(),
        wrong.first()
    );
}

/// Each team fills its ban slots in order, so a round's bans of one team
/// are slots 0, 1, 2 with none missing.
#[test]
fn real_ban_slots_fill_in_order() {
    let Some(rounds) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    for r in rounds {
        for team in [Some(0), Some(1)] {
            let slots: Vec<_> = r
                .bans
                .iter()
                .filter(|b| b.team == team)
                .map(|b| b.slot)
                .collect();
            let expected: Vec<_> = (0..slots.len() as u32).map(Some).collect();
            assert_eq!(
                slots,
                expected,
                "{} R{} team {team:?}",
                r.header.match_id,
                r.header.round_number + 1
            );
        }
    }
}

/// The slots and the banned-operator icons near them are two reads of the
/// same bans.
#[test]
fn real_ban_slots_agree_with_their_icons() {
    let Some(rounds) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    for r in rounds {
        let bans = r.decode.get("bans").unwrap();
        assert!(
            bans.warnings.is_empty(),
            "{} R{}: {:?}",
            r.header.match_id,
            r.header.round_number + 1,
            bans.warnings
        );
    }
}

/// In a player's own recording, the game numbers the player's team 1.
#[test]
fn real_recorders_team_has_team_color_one() {
    let Some(rounds) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let mut checked = 0;
    for r in rounds {
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

/// Every real match folder, headers only, with its rounds.
fn real_matches() -> Option<Vec<replay_analyzer::Match>> {
    let root = match_replay_dir()?;
    let folders = replay_analyzer::matches::find_match_folders(&root).unwrap();
    Some(
        folders
            .iter()
            .map(|dir| replay_analyzer::Match::open_with(dir, ReadMode::Header).unwrap())
            .collect(),
    )
}

/// `matchresult` is the recorder's result in their own recording: 2 won, 1
/// lost, 7 ended with no winner.
#[test]
fn real_outcomes_agree_with_matchresult() {
    use replay_analyzer::summary::Outcome;
    let Some(matches) = real_matches() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    for m in &matches {
        let s = m.summary().unwrap();
        let Some(value) = s.result.raw_match_result else {
            continue;
        };
        if s.recording.spectator {
            continue;
        }
        let expected = match value {
            2 => Outcome::Win,
            1 => Outcome::Loss,
            7 => Outcome::Cancelled,
            other => panic!("{}: matchresult {other} not seen before", s.match_id),
        };
        assert_eq!(s.result.outcome, expected, "{}", s.match_id);
    }
}

/// The game writes `isspectator` only for spectators, so a recording
/// without it is a player's, even when its header lacks the player (a file
/// cut during prep has no attackers in its header).
#[test]
fn real_player_recordings_are_not_listed_as_spectators() {
    let Some(matches) = real_matches() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    for m in &matches {
        if m.rounds.iter().all(|r| r.header.is_spectator != Some(true)) {
            let s = m.summary().unwrap();
            assert!(!s.recording.spectator, "{}", s.match_id);
        }
    }
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
/// player whose interaction object did it.
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
            (DefuserDisableComplete, "WIZARD.L5".into(), "0:33".into()),
        ]
    );
    // Planted 6 s after the action clock hit 0:00; disabled by the last
    // defender.
    let r7 = events("custom_7.rec").unwrap();
    assert_eq!(
        r7,
        [
            (DefuserPlantComplete, "WIZARD.L5".into(), "0:00".into()),
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

/// Every player of the test match is on PC under their own name, and wears
/// and carries what their body's creation message lists.
#[test]
fn y11s3_players_have_a_platform_and_cosmetics() {
    use replay_analyzer::{Platform, Status};
    let Some(dir) = data_dir() else { return };
    let Some(round) = y11s3(&dir, "custom_1.rec") else {
        return;
    };
    let players = &round.header.players;
    for p in players {
        let name = &p.username;
        assert_eq!(p.platform, Some(Platform::Pc), "{name}");
        assert!(!p.uses_nickname && p.renamed_to.is_none(), "{name}");
        let c = p.cosmetics.as_ref().unwrap_or_else(|| panic!("{name}"));
        assert!(c.uniform.is_some() && c.headgear.is_some(), "{name}");
        // Shield operators carry no primary gun.
        assert!((1..=2).contains(&c.weapons.len()), "{name}");
        for w in &c.weapons {
            assert!(w.skin.is_some(), "{name}: {w:?}");
        }
    }
    let wizard = players.iter().find(|p| p.username == "WIZARD.L5").unwrap();
    let c = wizard.cosmetics.as_ref().unwrap();
    assert_eq!(c.uniform, Some(393844871224));
    assert_eq!(c.headgear, Some(393844871002));
    assert_eq!(c.operator_card.badges, [414187259927]);
    let primary = &c.weapons[0];
    assert_eq!(primary.item, 393596493099);
    assert_eq!(primary.skin, Some(246545425488));
    assert_eq!(primary.charm, Some(361075170164));
    assert_eq!(c.weapons[1].charm, None, "the placeholder is no charm");
    for (field, status) in [
        ("platform", Status::Inferred),
        ("names", Status::Decoded),
        ("cosmetics", Status::Decoded),
    ] {
        let f = round.decode.get(field).unwrap();
        assert_eq!((f.status, f.count), (status, 10), "{f:?}");
    }
    let json = serde_json::to_value(&round).unwrap();
    assert_eq!(json["players"][0]["platform"], "pc");
    assert_eq!(
        json["players"][0]["cosmetics"]["weapons"][0]["slot"],
        "primary"
    );
}

/// A player wears the same uniform and headgear on an operator in every
/// round of the test match, and players on the same operator do not all
/// wear the same: the ids are the player's own, not the operator's.
#[test]
fn y11s3_cosmetics_belong_to_the_player() {
    let Some(dir) = data_dir() else { return };
    let mut worn = std::collections::HashMap::new();
    let mut by_operator = std::collections::HashMap::new();
    for i in 1..=10 {
        let Some(round) = y11s3(&dir, &format!("custom_{i}.rec")) else {
            return;
        };
        for p in &round.header.players {
            let Some(c) = &p.cosmetics else { continue };
            let look = (c.uniform, c.headgear);
            let operator = p.operator.to_string();
            let before = worn.insert((p.key.clone(), operator.clone()), look);
            assert!(
                before.is_none_or(|b| b == look),
                "{} changed clothes on {operator}",
                p.username
            );
            by_operator
                .entry(operator)
                .or_insert_with(std::collections::HashSet::new)
                .insert(look);
        }
    }
    assert!(by_operator.values().any(|looks| looks.len() > 1));
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

/// Attackers change spawn during prep: the lineup holds the last name
/// written to their vote object, not the one in the pick packet. Each of
/// these bodies appeared at the spawn named here.
#[test]
fn y11s3_spawns_follow_changes_in_prep() {
    let Some(dir) = data_dir() else { return };
    let spawn = |file: &str, name: &str| -> Option<String> {
        let round = y11s3(&dir, file)?;
        let i = round.player_index_by_username(name).unwrap();
        Some(round.header.players[i].spawn.clone())
    };
    let Some(first) = spawn("custom_1.rec", "soulz1.FaZe") else {
        return;
    };
    assert_eq!(first, "Alley Access");
    assert_eq!(spawn("custom_9.rec", "PSYCHO.L5").unwrap(), "Jewelry Front");
    // Defenders keep the site, whatever their vote object still holds from
    // their last attack round.
    let round = y11s3(&dir, "custom_7.rec").unwrap();
    for p in round.header.players.iter().filter(|p| p.team_index == 1) {
        assert_eq!(p.spawn, round.header.site, "{}", p.username);
    }
}

/// Y11S3 states what older seasons leave to inference: each team's side, the
/// frame the round was decided in, and the frame the defuser went live in.
#[test]
fn y11s3_sides_end_and_plant_are_decoded() {
    use replay_analyzer::{MatchUpdateType, Phase, TeamRole};
    let Some(dir) = data_dir() else { return };
    for n in 1..=10 {
        let Some(round) = y11s3(&dir, &format!("custom_{n}.rec")) else {
            return;
        };
        assert!(round.decode.trusted, "custom_{n}: {:?}", round.decode);
        let roles = round.decode.get("teamRoles").unwrap();
        assert_eq!(roles.status, replay_analyzer::Status::Decoded, "custom_{n}");
        // The game's round history states the same kind of win as the events.
        let reason = round.decode.get("winCondition").unwrap();
        assert_eq!(
            reason.status,
            replay_analyzer::Status::Decoded,
            "custom_{n}"
        );
        let attack = if n <= 6 { 1 } else { 0 };
        assert_eq!(round.header.teams[attack].role, Some(TeamRole::Attack));
        // Every plant and disable names its player.
        assert!(
            round
                .match_feedback
                .iter()
                .filter(|u| matches!(
                    u.kind,
                    MatchUpdateType::DefuserPlantStart
                        | MatchUpdateType::DefuserPlantComplete
                        | MatchUpdateType::DefuserDisableStart
                        | MatchUpdateType::DefuserDisableComplete
                ))
                .all(|u| !u.username.is_empty()),
            "custom_{n}"
        );
        assert!(round.info().left.is_empty(), "custom_{n}");
    }
    // Round 7: the action clock ran out during a plant, which completed 6 s
    // later. Those seconds are on the timeline, so the round's end agrees
    // with the recording.
    let round = y11s3(&dir, "custom_7.rec").unwrap();
    let info = round.info();
    let plant = info.plant.unwrap();
    assert_eq!((plant.time.as_str(), plant.elapsed), ("0:00", 231.0));
    let end = info.phases.iter().find(|p| p.phase == Phase::End).unwrap();
    assert!((end.recording_start.unwrap() - end.start).abs() < 1.0);
}

/// `<match id> R<round>`, to name a real round in a failure without naming
/// its players.
fn real_round_name(r: &Round) -> String {
    format!("{} R{}", r.header.match_id, r.header.round_number + 1)
}

/// Whether the game finished writing the round's file.
fn written_whole(r: &Round) -> bool {
    r.container.as_ref().is_some_and(|c| c.complete)
}

/// Every player of a real round has a profile id, an id no other player
/// has, and a place relative to the one player who recorded.
#[test]
fn real_players_are_who_the_header_says() {
    use replay_analyzer::Relation;
    let Some(rounds) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    for r in rounds {
        let name = real_round_name(r);
        let h = &r.header;
        let mut ids = std::collections::HashSet::new();
        let mut profiles = std::collections::HashSet::new();
        for (i, p) in h.players.iter().enumerate() {
            assert!(
                !p.profile_id.is_empty(),
                "{name}: player {i} has no profile id"
            );
            assert_eq!(p.key, p.profile_id, "{name}: player {i}");
            assert!(
                profiles.insert(&p.profile_id),
                "{name}: player {i} listed twice"
            );
            assert!(p.relation.is_some(), "{name}: player {i} has no relation");
            // Read in full, a player's id is the one their controller holds.
            if written_whole(r) {
                assert_ne!(p.id, 0, "{name}: player {i} has no id");
                assert!(ids.insert(p.id), "{name}: player {i} shares an id");
            }
        }
        let you: Vec<_> = (h.players.iter())
            .filter(|p| p.relation == Some(Relation::You))
            .collect();
        assert_eq!(you.len(), 1, "{name}: players who are `you`");
        let you = you[0];
        assert_eq!(you.profile_id, h.recording_profile_id, "{name}");
        if written_whole(r) {
            assert_eq!(you.id, h.recording_player_id, "{name}");
        }
        for (i, p) in h.players.iter().enumerate() {
            let expected = if std::ptr::eq(p, you) {
                Relation::You
            } else if p.team_index == you.team_index {
                Relation::Teammate
            } else {
                Relation::Opponent
            };
            assert_eq!(p.relation, Some(expected), "{name}: player {i}");
            if p.party.is_some() {
                assert_eq!(p.team_index, you.team_index, "{name}: player {i}");
            }
        }
        let f = r.decode.get("recorder").unwrap();
        assert_eq!(
            (f.status, f.count),
            (replay_analyzer::Status::Decoded, 1),
            "{name}"
        );
        let f = r.decode.get("profileIds").unwrap();
        assert_eq!(f.status, replay_analyzer::Status::Decoded, "{name}");
    }
}

/// Every player of a real round is carried by objects of their own, and the
/// player table links a body to everyone who spawned, the recorder
/// included. A player without a body is one the table lists and never gave
/// one, which the report says without calling it a fault.
#[test]
fn real_players_link_to_their_objects_and_bodies() {
    use replay_analyzer::{Relation, Status};
    let Some(rounds) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    for r in rounds {
        let name = real_round_name(r);
        let players = &r.header.players;
        let movement = r.decode.get("movement").unwrap();
        if !written_whole(r) {
            // A file the game did not finish may stop before any body is
            // given, or hold no table at all: never a clean read.
            let bodies = players
                .iter()
                .filter(|p| p.entities.as_ref().is_some_and(|e| e.movement.is_some()))
                .count();
            if bodies < players.len() {
                assert_ne!(movement.status, Status::Decoded, "{name}");
                assert_ne!(movement.status, Status::NotInVersion, "{name}");
            }
            continue;
        }
        let mut seen = std::collections::HashSet::new();
        let mut bodiless = 0;
        for (i, p) in players.iter().enumerate() {
            let e = (p.entities.as_ref())
                .unwrap_or_else(|| panic!("{name}: player {i} has no objects"));
            for id in [Some(e.controller), e.scoreboard, e.health] {
                let id = id.unwrap_or_else(|| panic!("{name}: player {i} lacks an object"));
                assert!(seen.insert(id), "{name}: player {i} shares object {id:08x}");
            }
            match e.movement {
                Some(body) => {
                    assert!(
                        seen.insert(body),
                        "{name}: player {i} shares body {body:08x}"
                    );
                    let pos = (p.spawn_position)
                        .unwrap_or_else(|| panic!("{name}: player {i} has a body created nowhere"));
                    assert!(pos.iter().all(|v| v.abs() < 1000.0), "{name}: {pos:?}");
                }
                None => {
                    bodiless += 1;
                    assert!(p.spawn_position.is_none(), "{name}: player {i}");
                }
            }
        }
        // Every table was read and lists every player.
        assert_eq!(movement.status, Status::Decoded, "{name}: {movement:?}");
        assert_eq!(movement.count, players.len() - bodiless, "{name}");
        let explained = (movement.warnings.iter())
            .any(|w| w.starts_with(&format!("{bodiless} players were never given a body")));
        assert_eq!(bodiless > 0, explained, "{name}: {movement:?}");
        // The recorder cannot have left their own recording: when all their
        // teammates spawned, so did they.
        let you = (players.iter())
            .find(|p| p.relation == Some(Relation::You))
            .unwrap_or_else(|| panic!("{name}: nobody recorded"));
        let body = |p: &replay_analyzer::Player| p.entities.as_ref().and_then(|e| e.movement);
        let team_spawned = players
            .iter()
            .filter(|p| p.relation == Some(Relation::Teammate))
            .all(|p| body(p).is_some());
        if team_spawned && players.len() > 1 {
            assert!(
                body(you).is_some(),
                "{name}: the recorder's body is not linked"
            );
        }
        let f = r.decode.get("entities").unwrap();
        assert_eq!(
            (f.status, f.count),
            (Status::Decoded, players.len()),
            "{name}"
        );
    }
}

/// What a real round says about players names players of the round: both
/// ends of every kill, with their profile ids, and every weapon-ready
/// change.
#[test]
fn real_events_name_players_of_the_round() {
    use replay_analyzer::MatchUpdateType;
    let Some(rounds) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    for r in rounds {
        let name = real_round_name(r);
        let profile = |username: &str| {
            (r.header.players.iter())
                .find(|p| p.username == username)
                .map(|p| p.profile_id.as_str())
        };
        for (i, u) in r.match_feedback.iter().enumerate() {
            if u.kind != MatchUpdateType::Kill {
                continue;
            }
            assert_eq!(
                profile(&u.username),
                Some(u.profile_id.as_str()),
                "{name}: kill {i}, killer"
            );
            assert_eq!(
                profile(&u.target),
                Some(u.target_profile_id.as_str()),
                "{name}: kill {i}, target"
            );
        }
        for (i, w) in r.weapon_ready.iter().enumerate() {
            assert!(
                profile(&w.username).is_some(),
                "{name}: weapon-ready change {i}"
            );
        }
    }
}

/// Across the rounds of a real match a player keeps their key, team and
/// relation, whether or not they play every round or come back under a new
/// `playerid`, and the summary lists everyone seen exactly once.
#[test]
fn real_matches_keep_their_players_apart_across_rounds() {
    use replay_analyzer::{MatchSummary, Relation};
    let Some(rounds) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    for of_match in rounds.chunk_by(|a, b| a.header.match_id == b.header.match_id) {
        let id = &of_match[0].header.match_id;
        let mut seen = std::collections::HashMap::new();
        for r in of_match {
            for p in &r.header.players {
                let first = *seen.entry(&p.key).or_insert((p.team_index, p.relation));
                assert_eq!(first, (p.team_index, p.relation), "{}", real_round_name(r));
            }
        }
        let summary = MatchSummary::new(of_match).unwrap();
        let mut listed = std::collections::HashSet::new();
        for (t, team) in summary.teams.iter().enumerate() {
            for (i, p) in team.players.iter().enumerate() {
                assert!(
                    listed.insert(&p.key),
                    "{id}: team {t} player {i} listed twice"
                );
                let known = seen.get(&p.key);
                assert_eq!(known, Some(&(t, p.relation)), "{id}: team {t} player {i}");
            }
        }
        assert_eq!(
            listed.len(),
            seen.len(),
            "{id}: players seen and not listed"
        );
        let you: Vec<_> = (summary.teams.iter())
            .flat_map(|t| &t.players)
            .filter(|p| p.relation == Some(Relation::You))
            .collect();
        assert_eq!(you.len(), 1, "{id}");
        assert_eq!(you[0].profile_id, summary.recording.profile_id, "{id}");
        assert_eq!(
            summary.your_team,
            seen.get(&you[0].key).map(|s| s.0),
            "{id}"
        );
    }
}

/// Every player of a real round is on a platform the parser knows, the
/// recorder (who records on PC) on PC, and everyone with a body has what
/// they wear. A player is given another name only in the round a recording
/// ends on.
#[test]
fn real_players_have_a_platform_names_and_cosmetics() {
    use replay_analyzer::{Platform, Relation, Status};
    let Some(rounds) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    for (n, r) in rounds.iter().enumerate() {
        let name = real_round_name(r);
        let platform = r.decode.get("platform").unwrap();
        assert_eq!(platform.status, Status::Inferred, "{name}: {platform:?}");
        let last_of_match =
            (rounds.get(n + 1)).is_none_or(|next| next.header.match_id != r.header.match_id);
        for (i, p) in r.header.players.iter().enumerate() {
            assert!(p.platform.is_some(), "{name}: player {i}");
            if p.relation == Some(Relation::You) {
                assert_eq!(p.platform, Some(Platform::Pc), "{name}");
                assert!(p.renamed_to.is_none(), "{name}: the recorder was renamed");
            }
            if let Some(to) = &p.renamed_to {
                assert!(last_of_match, "{name}: player {i} renamed mid-match");
                assert!(!to.is_empty() && *to != p.username, "{name}: player {i}");
                assert!(
                    p.uses_nickname || p.platform != Some(Platform::Pc),
                    "{name}: player {i} is on PC under their own name"
                );
            }
            let body = p.entities.as_ref().and_then(|e| e.movement);
            assert_eq!(p.cosmetics.is_some(), body.is_some(), "{name}: player {i}");
            if let Some(c) = &p.cosmetics {
                assert!(c.uniform.is_some() && c.headgear.is_some(), "{name}: {i}");
                assert!(c.weapons.len() <= 2, "{name}: player {i}");
            }
        }
        let cosmetics = r.decode.get("cosmetics").unwrap();
        assert_eq!(cosmetics.status, Status::Decoded, "{name}: {cosmetics:?}");
    }
}

/// Across real matches a player wears one uniform and headgear per
/// operator within a match, and keeps one platform throughout.
#[test]
fn real_cosmetics_and_platforms_stay_with_the_player() {
    let Some(rounds) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let mut worn = std::collections::HashMap::new();
    let mut platforms = std::collections::HashMap::new();
    for r in rounds {
        let name = real_round_name(r);
        for (i, p) in r.header.players.iter().enumerate() {
            let before = platforms.insert(p.key.clone(), p.platform);
            assert!(
                before.is_none_or(|b| b == p.platform),
                "{name}: player {i} changed platform"
            );
            let Some(c) = &p.cosmetics else { continue };
            let look = (c.uniform, c.headgear);
            let key = (
                r.header.match_id.clone(),
                p.key.clone(),
                p.operator.to_string(),
            );
            let before = worn.insert(key, look);
            assert!(
                before.is_none_or(|b| b == look),
                "{name}: player {i} changed clothes"
            );
        }
    }
}

/// The loadout of `name` on the operator they spawned with.
fn loadout_of<'a>(round: &'a Round, name: &str) -> &'a replay_analyzer::Loadout {
    let i = round.player_index_by_username(name).unwrap();
    let operator = round.header.players[i].operator;
    let mut found = round
        .loadouts
        .iter()
        .filter(|l| l.username == name && l.operator == operator);
    let loadout = found.next().unwrap_or_else(|| panic!("{name}: no loadout"));
    assert!(found.next().is_none(), "{name}: two loadouts on {operator}");
    loadout
}

/// Loadouts of custom_1, checked against the bytes: the HUD objects of the
/// state stream and the entity descriptors of the movement stream.
#[test]
fn y11s3_loadouts_carry_attachments_and_counts() {
    use replay_analyzer::loadout::Ammo;
    use replay_analyzer::{Phase, Status};
    let Some(dir) = data_dir() else { return };
    let Some(round) = y11s3(&dir, "custom_1.rec") else {
        return;
    };
    let id = |a: &Option<replay_analyzer::loadout::Named>| a.map(|a| a.id);

    // Fenrir: an MP7 with five attachments, four mines, one camera.
    let l = loadout_of(&round, "WIZARD.L5");
    assert_eq!(l.weapons, [1366019616, 1366019412]);
    assert_eq!(l.gadgets, [396493597839, 243267468233]);
    let w = l.primary.as_ref().unwrap();
    assert_eq!((w.id, w.name), (Some(1366019616), Some("MP7")));
    assert_eq!(w.asset, Some(393596493099));
    assert_eq!(
        [&w.sight, &w.barrel, &w.grip, &w.underbarrel, &w.magazine].map(id),
        [
            Some(258614298894),
            Some(258614298875),
            Some(238373621281),
            Some(238373621282),
            Some(238373621288)
        ]
    );
    assert!(!w.shield);
    let ammo = Ammo {
        magazine_size: Some(30),
        start: 181,
        end: 151,
        fired: 30,
    };
    assert_eq!(w.ammo, Some(ammo));
    assert_eq!(l.secondary.as_ref().unwrap().asset, Some(398874672307));
    let ability = l.ability.as_ref().unwrap();
    assert_eq!(ability.id, Some(396493597839));
    let c = ability.counts.as_ref().unwrap();
    assert_eq!((c.start, c.end, c.max), (4, 1, Some(4)));
    assert_eq!((c.used, c.gained, c.regenerates), (3, 0, false));
    let left: Vec<u32> = c.uses.iter().map(|u| u.count).collect();
    assert_eq!(left, [3, 2, 1]);
    // Two mines went down in prep, one in action, on the round's timeline.
    let phases: Vec<Phase> = c.uses.iter().map(|u| u.phase).collect();
    assert_eq!(phases, [Phase::Prep, Phase::Prep, Phase::Action]);
    for u in &c.uses {
        let recorded = u.recording_time.unwrap();
        assert!((recorded - u.elapsed).abs() < 2.0, "{u:?}");
    }
    let gadget = l.gadget.as_ref().unwrap();
    assert_eq!(gadget.id, Some(243267468233));
    let c = gadget.counts.as_ref().unwrap();
    assert_eq!((c.start, c.end, c.used, c.uses.len()), (1, 0, 1, 1));

    // Wamai's disks refill: six come back, seven are thrown.
    let c = loadout_of(&round, "pino.L5").ability.as_ref().unwrap();
    let c = c.counts.as_ref().unwrap();
    assert!(c.regenerates && c.max.is_none());
    assert_eq!((c.start, c.end, c.used, c.gained), (1, 0, 7, 6));
    let left: Vec<u32> = c.uses.iter().map(|u| u.count).collect();
    assert_eq!(left, [6, 5, 4, 3, 2, 1, 0]);

    // Clash has no primary gun: the slot holds her shield.
    let l = loadout_of(&round, "PSYCHO.L5");
    let shield = l.primary.as_ref().unwrap();
    assert!(shield.shield);
    assert_eq!(shield.id, Some(419258819322));
    assert!(shield.asset.is_none() && shield.sight.is_none() && shield.ammo.is_none());
    assert_eq!(l.weapons, [139558932060]);
    assert_eq!(l.gadgets, [419258819322, 133651070258]);
    assert_eq!(l.secondary.as_ref().unwrap().asset, Some(238373640573));

    // Grim's launcher sits in the ability slot as a weapon: its count is
    // its ammunition.
    let c = loadout_of(&round, "vitaking.FaZe")
        .ability
        .as_ref()
        .unwrap();
    assert_eq!(c.id, Some(374667788026));
    let c = c.counts.as_ref().unwrap();
    assert_eq!((c.start, c.end, c.used, c.max), (5, 0, 5, None));

    // Blackbeard's shield is both his primary and his ability.
    let l = loadout_of(&round, "kds.FaZe");
    assert!(l.primary.as_ref().unwrap().shield);
    assert_eq!(l.gadgets, [395972905309, 133651070436]);

    let f = round.decode.get("loadouts").unwrap();
    assert_eq!((f.status, f.count), (Status::Decoded, 10));
    assert!(f.warnings.is_empty(), "{f:?}");

    let json = serde_json::to_value(&round).unwrap();
    let wizard = &json["loadouts"][0];
    assert_eq!(wizard["username"], "WIZARD.L5");
    assert_eq!(wizard["primary"]["sight"]["id"], 258614298894u64);
    assert_eq!(wizard["primary"]["ammo"]["magazineSize"], 30);
    assert_eq!(wizard["ability"]["uses"][0]["phase"], "Prep");
    assert_eq!(wizard["ability"]["max"], 4);
    // Unnamed attachments carry their id only; a shield has no asset.
    assert!(wizard["primary"]["sight"].get("name").is_none());
    assert_eq!(json["loadouts"][1]["primary"]["shield"], true);
    assert!(json["loadouts"][1]["primary"].get("asset").is_none());
}

/// What holds for a loadout wherever it comes from. Returns whether a gun
/// with its asset was found.
fn check_loadout(l: &replay_analyzer::Loadout, context: &str) -> bool {
    let mut gun = false;
    for w in [&l.primary, &l.secondary].into_iter().flatten() {
        if w.shield {
            assert!(w.asset.is_none() && w.ammo.is_none(), "{context}: {w:?}");
            continue;
        }
        gun |= w.asset.is_some_and(|a| a != 0);
        if let Some(a) = &w.ammo {
            assert!(a.start >= a.end, "{context}: {a:?}");
            assert!(a.fired >= a.start - a.end, "{context}: {a:?}");
        }
    }
    for c in [&l.ability, &l.gadget].into_iter().flatten() {
        let Some(n) = &c.counts else { continue };
        // Counts are unsigned, so none is negative; the drops and rises
        // account for the whole change.
        assert_eq!(
            i64::from(n.start) - i64::from(n.used) + i64::from(n.gained),
            i64::from(n.end),
            "{context}: {c:?}"
        );
        assert!(n.uses.len() <= n.used as usize, "{context}: {c:?}");
        assert!(n.uses.windows(2).all(|w| w[0].elapsed <= w[1].elapsed));
        assert_ne!(n.max, Some(99), "{context}: {c:?}");
    }
    gun
}

/// Every player of every Y11S3 test round has a decoded loadout.
#[test]
fn y11s3_loadouts_hold_for_every_round() {
    use replay_analyzer::Status;
    let Some(dir) = data_dir() else { return };
    for path in replays(&dir, "valid") {
        let round = Round::open(&path, ReadMode::Full).unwrap();
        if round.header.code_version < replay_analyzer::types::version::Y11S3 {
            assert!(round.loadouts.iter().all(|l| l.primary.is_none()));
            continue;
        }
        let f = round.decode.get("loadouts").unwrap();
        assert_eq!((f.status, f.count), (Status::Decoded, 10), "{f:?}");
        for p in &round.header.players {
            let context = format!("{} {}", path.display(), p.username);
            let l = loadout_of(&round, &p.username);
            assert!(
                check_loadout(l, &context),
                "{context}: no gun with an asset"
            );
            // Guns in the test rounds are never refilled.
            for w in [&l.primary, &l.secondary].into_iter().flatten() {
                if !w.shield {
                    let a = w
                        .ammo
                        .as_ref()
                        .unwrap_or_else(|| panic!("{context}: no ammo"));
                    assert_eq!(a.start - a.fired, a.end, "{context}");
                    assert!(w.id.is_some() && w.linked, "{context}: {w:?}");
                }
            }
            assert!(l.ability.is_some() && l.gadget.is_some(), "{context}");
            // The id lists follow the slots.
            let guns: Vec<u64> = [&l.primary, &l.secondary]
                .into_iter()
                .flatten()
                .filter(|w| !w.shield)
                .filter_map(|w| w.id)
                .collect();
            assert_eq!(l.weapons, guns, "{context}");
        }
        // A partial read stops before the HUD settles: no detail.
        let partial = Round::open(&path, ReadMode::Partial).unwrap();
        assert!(partial.loadouts.iter().all(|l| l.primary.is_none()));
    }
    // Striker has no ability: the slot holds the first of two gadgets, and
    // the gadget slot the second.
    if let Some(round) = y11s3(&dir, "custom_3.rec") {
        let i = round.player_index_by_username("soulz1.FaZe").unwrap();
        assert_eq!(round.header.players[i].operator.name(), Some("Striker"));
        let l = loadout_of(&round, "soulz1.FaZe");
        assert_eq!(l.ability.as_ref().unwrap().id, Some(133651070288));
        assert_eq!(l.gadget.as_ref().unwrap().id, Some(133651070436));
        assert_eq!(l.gadgets, [133651070288, 133651070436]);
    }
}

/// A match folder lists what each player changed since their previous round
/// on the same side.
#[test]
fn y11s3_loadout_changes_compare_rounds_on_the_same_side() {
    let Some(dir) = data_dir() else { return };
    let src = dir.join("valid/Y11S3");
    if !src.is_dir() {
        return;
    }
    let m = replay_analyzer::Match::open(&src).unwrap();
    let changes = m.loadout_changes();
    assert!(!changes.is_empty());
    let side_of = |number: u32, name: &str| {
        let r = &m.rounds[number as usize - 1];
        let i = r.player_index_by_username(name).unwrap();
        r.header.teams[r.header.players[i].team_index].role
    };
    for c in &changes {
        assert!(c.previous_round < c.round && !c.changes.is_empty(), "{c:?}");
        assert_eq!(side_of(c.round, &c.username), Some(c.side), "{c:?}");
        assert_eq!(side_of(c.previous_round, &c.username), Some(c.side));
    }
    // Grim kept his guns in round 5 and took another gadget.
    let c = changes
        .iter()
        .find(|c| c.username == "vitaking.FaZe" && c.round == 5)
        .unwrap();
    assert_eq!((c.previous_round, c.side), (4, TeamRole::Attack));
    let json = serde_json::to_value(&c.changes).unwrap();
    assert_eq!(
        json,
        serde_json::json!([{
            "field": "gadget",
            "from": {"id": 263047965420u64, "name": "Hard Breach Charge"},
            "to": {"id": 387197346354u64, "name": "Impact EMP Grenade"},
        }])
    );
    let json = serde_json::to_value(&m).unwrap();
    assert_eq!(
        json["loadoutChanges"].as_array().unwrap().len(),
        changes.len()
    );
}

/// Real rounds: loadouts add up, and nearly every player of a finished file
/// has a gun with its entity found.
#[test]
fn real_loadouts_add_up() {
    let Some(rounds) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let (mut players, mut with_gun) = (0, 0);
    for r in rounds {
        let complete = r.container.as_ref().is_some_and(|c| c.complete);
        if r.header.code_version < replay_analyzer::types::version::Y11S3 || !complete {
            continue;
        }
        let name = r
            .file
            .as_ref()
            .map(|f| f.file_name.clone())
            .unwrap_or_default();
        let f = r.decode.get("loadouts").unwrap();
        assert!(f.count <= r.header.players.len(), "{name}: {f:?}");
        for p in &r.header.players {
            players += 1;
            let context = format!("{name} {}", p.username);
            let found = r
                .loadouts
                .iter()
                .rfind(|l| l.username == p.username && l.operator == p.operator);
            let Some(l) = found else { continue };
            with_gun += usize::from(check_loadout(l, &context));
            if l.primary.is_some() || l.secondary.is_some() {
                assert!(!l.weapons.is_empty(), "{context}: {l:?}");
            }
        }
    }
    assert!(players > 0);
    // Players who never spawned (a round that ended in prep, a player who
    // left) have no body, so no gun entity.
    assert!(with_gun * 20 >= players * 19, "{with_gun} of {players}");
}
