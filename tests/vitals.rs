//! Health, heals, plates, status effects and reverse friendly fire (Y11S3):
//! the ten test rounds, and invariants over a real `MatchReplay` folder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use replay_analyzer::types::version;
use replay_analyzer::vitals::{HealKind, LifeKind, Vitals};
use replay_analyzer::{LifeEventType, MatchUpdateType, Phase, ReadMode, Round};

fn data_dir() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = match std::env::var_os("R6_TEST_DATA") {
        Some(dir) => PathBuf::from(dir),
        None => root.join("test_recordings"),
    };
    let found = dir.join("valid/Y11S3").is_dir();
    if !found {
        eprintln!("skipping: no Y11S3 test replays in {}", dir.display());
    }
    found.then_some(dir)
}

/// Every round of `valid/Y11S3`, by file name, read in full.
fn test_rounds() -> Option<Vec<(String, Round)>> {
    let dir = data_dir()?.join("valid/Y11S3");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rec"))
        .collect();
    files.sort();
    let rounds = files
        .iter()
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            (name, Round::open(p, ReadMode::Full).unwrap())
        })
        .collect();
    Some(rounds)
}

fn vitals<'a>(name: &str, r: &'a Round) -> &'a Vitals {
    r.vitals
        .as_ref()
        .unwrap_or_else(|| panic!("{name}: no vitals"))
}

fn team_of(r: &Round, username: &str) -> Option<usize> {
    let p = r.header.players.iter().find(|p| p.username == username)?;
    Some(p.team_index)
}

/// Whether the team of `username` plays `operator`.
fn team_has(r: &Round, username: &str, operator: &str) -> bool {
    let team = team_of(r, username);
    let on_team = |p: &&replay_analyzer::Player| Some(p.team_index) == team;
    let mut mates = r.header.players.iter().filter(on_team);
    mates.any(|p| p.operator.name() == Some(operator))
}

/// What must hold for the vitals of any round.
fn check(name: &str, r: &Round) {
    let v = vitals(name, r);
    let is_player = |n: &str| r.header.players.iter().any(|p| p.username == n);

    for l in &v.life {
        assert!(is_player(&l.username), "{name}: {l:?}");
        if l.kind == LifeKind::Revive {
            assert!(l.health.is_some_and(|h| h > 0), "{name}: {l:?}");
        }
    }
    // The round's own life events follow the same rule.
    for e in &r.life_events {
        if e.kind == LifeEventType::Revive {
            assert!(e.health.is_some_and(|h| h > 0), "{name}: {e:?}");
        }
    }
    let revives = |kind| v.life.iter().filter(|l| l.kind == kind).count();
    let events = |kind| r.life_events.iter().filter(|e| e.kind == kind).count();
    assert_eq!(
        revives(LifeKind::Revive),
        events(LifeEventType::Revive),
        "{name}: revives"
    );
    // The HUD passes through "down" as a kill ends the round; the round's
    // life events leave those out, so every HUD down they lack has a kill
    // of that player within half a second.
    let listed = |l: &&replay_analyzer::vitals::LifeChange| {
        r.life_events.iter().any(|e| {
            e.kind == LifeEventType::Down
                && e.username == l.username
                && e.recording_time == l.recording_time
        })
    };
    let downs = || v.life.iter().filter(|l| l.kind == LifeKind::Down);
    assert_eq!(
        downs().filter(listed).count(),
        events(LifeEventType::Down),
        "{name}: downs"
    );
    for l in downs().filter(|l| !listed(l)) {
        let killed = r.match_feedback.iter().any(|u| {
            u.kind == MatchUpdateType::Kill
                && u.target == l.username
                && (u.recording_time.zip(l.recording_time))
                    .is_some_and(|(a, b)| (a - b).abs() <= 0.5)
        });
        assert!(killed, "{name}: {l:?} is no life event and no kill");
    }

    for e in &v.effects {
        assert!(is_player(&e.username), "{name}: {e:?}");
        assert!(e.seconds >= 0.0, "{name}: {e:?}");
        assert_eq!(e.buff, e.name.is_some_and(is_buff), "{name}: {e:?}");
    }
    // One effect of a type at a time on a player.
    let mut spans: BTreeMap<(&str, u32), Vec<(f64, f64)>> = BTreeMap::new();
    for e in &v.effects {
        let start = e.start.recording_time.unwrap_or(0.0);
        let span = (start, start + e.seconds);
        spans.entry((&e.username, e.kind)).or_default().push(span);
    }
    for (key, mut list) in spans {
        list.sort_by(|a, b| a.0.total_cmp(&b.0));
        for w in list.windows(2) {
            assert!(w[0].1 <= w[1].0 + 0.0015, "{name}: {key:?} {w:?}");
        }
    }

    for h in &v.heals {
        assert!(is_player(&h.username), "{name}: {h:?}");
        assert!(h.amount > 0 && h.health >= h.amount, "{name}: {h:?}");
        assert!(h.overheal <= 20, "{name}: {h:?}");
        if let Some(by) = &h.by {
            assert!(is_player(by), "{name}: {h:?}");
        }
    }
    // Overheal tops out 20 above the maximum, for as long as the round is
    // undecided: plates come off the maximum as it ends (seen 0.07 s before
    // the clock says so), while the health stays.
    let spans = r.timeline.spans();
    let decided = spans.iter().find(|s| s.phase == Phase::End);
    let decided = decided.and_then(|s| s.recording_start);
    for p in &v.players {
        for s in &p.samples {
            let times = s.recording_time.zip(decided);
            let ending = times.is_some_and(|(t, end)| t > end - 1.0);
            assert!(
                ending || s.health <= s.max_health + 20,
                "{name} {}: {s:?}, decided at {decided:?}",
                p.username
            );
        }
    }
    for f in &v.friendly_fire {
        assert!(is_player(&f.username), "{name}: {f:?}");
        assert_eq!(f.active_at_start, f.on.is_none(), "{name}: {f:?}");
    }
    for f in &v.flashes {
        assert!(f.seconds >= 0.0, "{name}: {f:?}");
        let you = Some(replay_analyzer::Relation::You);
        let recorder = r.header.players.iter().find(|p| p.relation == you);
        assert_eq!(
            recorder.map(|p| &p.username),
            Some(&f.username),
            "{name}: {f:?}"
        );
    }
}

/// The effects known to help the player they are on.
fn is_buff(name: &str) -> bool {
    [
        "FinkaSurge",
        "RookArmor",
        "JackalTracking",
        "FriendlyJammer",
        "ThunderbirdHeal",
    ]
    .contains(&name)
}

#[test]
fn test_rounds_have_vitals_that_hold_together() {
    let Some(rounds) = test_rounds() else { return };
    assert_eq!(rounds.len(), 10);
    for (name, r) in &rounds {
        check(name, r);
        let v = vitals(name, r);
        // Every player spawned in these rounds, as an operator of one of
        // the three health classes.
        assert_eq!(v.players.len(), r.header.players.len(), "{name}");
        for p in &v.players {
            assert!(
                matches!(p.max_health, Some(100 | 110 | 125)),
                "{name} {}: {:?}",
                p.username,
                p.max_health
            );
        }
        assert!(v.warnings.is_empty(), "{name}: {:?}", v.warnings);
        let status = r.decode.get("vitals").unwrap();
        assert_eq!(status.status, replay_analyzer::Status::Decoded, "{name}");
        assert_eq!(status.count, r.header.players.len(), "{name}");
    }
}

/// The effects each test round holds, counted on the bytes by the research
/// script (`status/effects.py`).
#[test]
fn test_rounds_list_their_effects() {
    let Some(rounds) = test_rounds() else { return };
    let count = |n: &str| {
        let (name, r) = rounds.iter().find(|(name, _)| name == n).unwrap();
        vitals(name, r).effects.len()
    };
    // custom_5 has one more than the script found: Thorn stands in a Grim
    // swarm from 136.9 s, an item the script's walker read past.
    let expected = [12, 2, 23, 24, 7, 5, 7, 6, 3, 8];
    for (i, want) in expected.into_iter().enumerate() {
        assert_eq!(
            count(&format!("custom_{}", i + 1)),
            want,
            "custom_{}",
            i + 1
        );
    }
    // custom_1: Deimos marks Fenrir, which shows on both for as long.
    let (name, r) = rounds.iter().find(|(n, _)| n == "custom_1").unwrap();
    let v = vitals(name, r);
    let named = |n: &str| v.effects.iter().find(|e| e.name == Some(n)).unwrap();
    let (marked, tracking) = (named("DeimosMarked"), named("DeimosTracking"));
    let operator = |user: &str| {
        let p = r.header.players.iter().find(|p| p.username == user);
        p.and_then(|p| p.operator.name())
    };
    assert_eq!(operator(&tracking.username), Some("Deimos"));
    assert_eq!(operator(&marked.username), Some("Fenrir"));
    assert_eq!(marked.start.recording_time, tracking.start.recording_time);
    assert!((marked.seconds - 13.1).abs() < 0.05, "{marked:?}");
    assert!(!marked.buff && !marked.open);
}

/// The one revive of the test rounds, and none of the deaths the old rule
/// took for one.
#[test]
fn test_rounds_revive_only_players_who_get_health() {
    let Some(rounds) = test_rounds() else { return };
    let mut revives = Vec::new();
    for (name, r) in &rounds {
        for e in &r.life_events {
            if e.kind == LifeEventType::Revive {
                revives.push((name.as_str(), e.username.as_str(), e.health));
            }
        }
    }
    assert_eq!(revives, [("custom_7", "soulz1.FaZe", Some(20))]);
    // A down that ends in a revive says how far the bleed-out got.
    let (name, r) = rounds.iter().find(|(n, _)| n == "custom_7").unwrap();
    let v = vitals(name, r);
    let down = v
        .life
        .iter()
        .rfind(|l| l.kind == LifeKind::Down && l.username == "soulz1.FaZe")
        .unwrap();
    assert!(
        down.bleed_out.is_some_and(|b| b > 0.0 && b < 1.0),
        "{down:?}"
    );
}

#[test]
fn test_rounds_serialize_vitals() {
    let Some(rounds) = test_rounds() else { return };
    let (_, r) = rounds.iter().find(|(n, _)| n == "custom_1").unwrap();
    let json = serde_json::to_value(r).unwrap();
    let effect = &json["effects"][0];
    for key in [
        "username",
        "type",
        "name",
        "buff",
        "time",
        "phase",
        "elapsed",
        "recordingTime",
        "seconds",
    ] {
        assert!(!effect[key].is_null(), "effects[0].{key}: {effect}");
    }
    for p in json["players"].as_array().unwrap() {
        assert!(p["maxHealth"].is_u64(), "{p}");
    }
    // Health changes say what the maximum was, and nothing about a cause
    // when it is damage.
    let health = json["health"].as_array().unwrap();
    assert!(!health.is_empty());
    for h in health {
        assert!(h["maxHealth"].is_u64(), "{h}");
        if h["change"].as_i64().unwrap() < -1 {
            assert!(h["cause"].is_null(), "{h}");
        }
    }
    let status = json["decodeStatus"]["fields"].as_array().unwrap();
    assert!(status.iter().any(|f| f["field"] == "vitals"));
    // Nothing of these in the test rounds: the keys are left out.
    for key in ["heals", "plates"] {
        assert!(json.get(key).is_none(), "{key}");
    }
}

/// A `MatchReplay` folder as the game writes it, from `R6_MATCH_REPLAY`.
fn real_rounds() -> Option<Vec<Round>> {
    let root = PathBuf::from(std::env::var_os("R6_MATCH_REPLAY")?);
    if !root.is_dir() {
        return None;
    }
    let mut rounds = Vec::new();
    for dir in replay_analyzer::matches::find_match_folders(&root).unwrap() {
        let m = replay_analyzer::Match::open_with(&dir, ReadMode::Full).unwrap();
        rounds.extend(m.rounds);
    }
    Some(rounds)
}

fn real_name(r: &Round) -> String {
    let file = r.file.as_ref().map(|f| f.file_name.clone());
    file.unwrap_or_default()
}

/// Real rounds: heals and plates need their operator on the team, health
/// stays within the overheal limit, and reverse friendly fire turns on for
/// the team that killed one of its own.
#[test]
fn real_vitals_hold_together() {
    let Some(rounds) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let mut total: BTreeMap<String, usize> = BTreeMap::new();
    let mut effects: BTreeMap<String, usize> = BTreeMap::new();
    let mut add = |key: &str, n: usize| *total.entry(key.to_owned()).or_default() += n;
    for r in &rounds {
        if r.header.code_version < version::Y11S3 {
            continue;
        }
        let name = real_name(r);
        let Some(v) = &r.vitals else {
            // A file cut off before its records has nothing to read.
            let complete = r.container.as_ref().is_some_and(|c| c.complete);
            assert!(!complete, "{name}: no vitals");
            add("rounds without vitals", 1);
            continue;
        };
        check(&name, r);
        add("rounds", 1);
        add("warnings", v.warnings.len());
        for w in &v.warnings {
            eprintln!("{name}: {w}");
        }
        for h in &v.heals {
            let (kind, operator) = match h.kind {
                HealKind::FinkaSurge => ("heals: FinkaSurge", "Finka"),
                HealKind::DocStim => ("heals: DocStim", "Doc"),
                HealKind::KonaBurst => ("heals: KonaBurst", "Thunderbird"),
                HealKind::KonaTick => ("heals: KonaTick", "Thunderbird"),
            };
            add(kind, 1);
            add("heals on a revive", usize::from(h.revive));
            // Doc heals anyone; the others only their own team.
            if h.kind == HealKind::DocStim {
                let doc = |p: &replay_analyzer::Player| p.operator.name() == Some("Doc");
                assert!(r.header.players.iter().any(doc), "{name}: {h:?}");
            } else {
                assert!(team_has(r, &h.username, operator), "{name}: {h:?}");
            }
            assert!(h.by.is_some(), "{name}: {h:?}");
        }
        for p in &v.plates {
            assert!(team_has(r, &p.username, "Rook"), "{name}: {p:?}");
            assert!(p.by.is_some(), "{name}: {p:?}");
        }
        add("plates", v.plates.len());
        for p in &v.players {
            match p.max_health {
                Some(max) => {
                    assert!(
                        matches!(max, 100 | 110 | 125),
                        "{name} {}: {max}",
                        p.username
                    );
                    add(&format!("players with a maximum health of {max}"), 1);
                }
                None => add("players without a maximum health", 1),
            }
        }
        add("overheal decay steps", v.decay.len());
        add("unexplained rises", v.unexplained.len());
        for u in &v.unexplained {
            eprintln!("{name}: unexplained rise {u:?}");
        }
        let life = |kind| v.life.iter().filter(|l| l.kind == kind).count();
        add("downs", life(LifeKind::Down));
        add("revives", life(LifeKind::Revive));
        // What the rule before this one made of the same states: every
        // step from down to 0 or 2, deaths written 3, 2, 4 included.
        for p in &v.players {
            let steps = p.samples.windows(2);
            let up = steps.filter(|w| w[0].state == 3 && matches!(w[1].state, 0 | 2));
            add("revives by the old rule", up.count());
        }
        for e in &v.effects {
            let key = match e.name {
                Some(n) => format!("{:>2} {n}", e.kind),
                None => format!("{:>2}", e.kind),
            };
            *effects.entry(key).or_default() += 1;
        }
        add("effects", v.effects.len());
        add("flashes", v.flashes.len());

        // Reverse friendly fire.
        let kills = || {
            let all = r.match_feedback.iter();
            all.filter(|u| u.kind == MatchUpdateType::Kill)
        };
        let killers: Vec<usize> = kills()
            .filter(|u| team_of(r, &u.username).is_some())
            .filter(|u| team_of(r, &u.username) == team_of(r, &u.target))
            .filter_map(|u| team_of(r, &u.username))
            .collect();
        add("team kills", killers.len());
        for f in &v.friendly_fire {
            add("reverse friendly fire: activations", 1);
            if f.active_at_start {
                add("reverse friendly fire: active at start", 1);
                continue;
            }
            add("reverse friendly fire: turned on", 1);
            if killers.is_empty() {
                add("reverse friendly fire: on without a team kill", 1);
                continue;
            }
            let team = team_of(r, &f.username).unwrap();
            assert!(killers.contains(&team), "{name}: {f:?}");
        }
    }
    eprintln!("vitals over the real folder:");
    for (key, n) in &total {
        eprintln!("  {n:>6}  {key}");
    }
    eprintln!("effects by type:");
    for (key, n) in &effects {
        eprintln!("  {n:>6}  {key}");
    }
    assert!(total.get("rounds").is_some_and(|&n| n > 0));
}

/// The old rule took every down that went 3, 2, 4 (a death) for a revive.
#[test]
fn real_revives_are_given_health() {
    let Some(rounds) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let (mut revives, mut killed) = (0, 0);
    for r in &rounds {
        for e in &r.life_events {
            if e.kind != LifeEventType::Revive {
                continue;
            }
            revives += 1;
            if r.header.code_version < version::Y11S3 {
                continue;
            }
            assert!(e.health.is_some_and(|h| h > 0), "{}: {e:?}", real_name(r));
            // A revived player is alive afterwards: no kill of them at the
            // same moment.
            let died = r.match_feedback.iter().any(|u| {
                u.kind == MatchUpdateType::Kill
                    && u.target == e.username
                    && u.recording_time
                        .zip(e.recording_time)
                        .is_some_and(|(a, b)| (a - b).abs() < 0.3)
            });
            killed += usize::from(died);
        }
    }
    eprintln!("revives in the real folder: {revives}");
    assert_eq!(killed, 0, "revives of players killed at that moment");
}
