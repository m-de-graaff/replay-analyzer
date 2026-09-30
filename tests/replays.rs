//! Compares parser output against the expected JSON shipped with r6-dissect's
//! test replays. Data is read from `R6_TEST_DATA`, else `test_recordings/`,
//! else `.opensrc/r6-dissect`. Each holds `valid/**/*.rec` (with `.rec.json`
//! expectations) and `invalid/*.rec`. Tests are skipped when none exists.

use std::path::{Path, PathBuf};

use replay_analyzer::{Error, ReadMode, Round, TeamRole};
use serde_json::Value;

fn data_dir() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidates = match std::env::var_os("R6_TEST_DATA") {
        Some(dir) => vec![PathBuf::from(dir)],
        None => vec![
            root.join("test_recordings"),
            root.join(".opensrc/r6-dissect/dissect/test/data/replays"),
        ],
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

/// Go decodes missing keys as zero values, so drop zero values on both sides.
/// `{name, id}` values compare by id only, as r6-dissect's own tests do: some
/// fixtures carry names from before an id was given one.
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

/// Fields this parser adds that r6-dissect does not report.
const EXTENSIONS: &[&str] = &["weapon"];

fn strip_extensions(v: Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(k, _)| !EXTENSIONS.contains(&k.as_str()))
                .map(|(k, v)| (k, strip_extensions(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(strip_extensions).collect()),
        v => v,
    }
}

fn check(path: &Path, key: &str, got: Value, want: &Value, failures: &mut Vec<String>) {
    let mut diffs = Vec::new();
    diff(
        key,
        &normalize(strip_extensions(got)),
        &want[key],
        &mut diffs,
    );
    if !diffs.is_empty() {
        failures.push(format!("{}:\n  {}", path.display(), diffs.join("\n  ")));
    }
}

#[test]
fn full_read_matches_r6_dissect() {
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
    let (_, header_len) = replay_analyzer::header::parse(&data).unwrap();

    let mut chunked = data[..header_len].to_vec();
    for (i, part) in data[header_len..].chunks(1 << 20).enumerate() {
        if i % 3 == 0 {
            chunked.extend_from_slice(b"\x01\x02padding"); // non-zstd bytes between frames
        }
        chunked.extend(zstd::bulk::compress(part, 1).unwrap());
    }

    let legacy = Round::from_bytes(&raw, ReadMode::Full).unwrap();
    let repacked = Round::from_bytes(&chunked, ReadMode::Full).unwrap();
    assert_eq!(
        serde_json::to_value(&legacy).unwrap(),
        serde_json::to_value(&repacked).unwrap()
    );
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
