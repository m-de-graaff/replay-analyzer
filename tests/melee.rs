//! Melee hits and shield actions on real replays. Test rounds are read
//! from `R6_TEST_DATA`, else `test_recordings/`; the same checks run on a
//! `MatchReplay` folder when `R6_MATCH_REPLAY` names one. Tests are skipped
//! when their replays are missing.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use replay_analyzer::loadout::When;
use replay_analyzer::melee::{MeleeHit, ShieldAction, ShieldActionType, Target};
use replay_analyzer::{Match, Phase, ReadMode, Round};

fn data_dir() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = match std::env::var_os("R6_TEST_DATA") {
        Some(dir) => PathBuf::from(dir),
        None => root.join("test_recordings"),
    };
    let found = dir.join("valid/Y11S3").is_dir();
    if !found {
        eprintln!("skipping: no Y11S3 test replays in {dir:?}");
    }
    found.then_some(dir)
}

/// The `custom_*.rec` rounds, by file name.
fn custom_rounds(dir: &Path) -> Vec<(String, Round)> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir.join("valid/Y11S3"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            name.starts_with("custom_") && name.ends_with(".rec")
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            (name, Round::open(&p, ReadMode::Full).unwrap())
        })
        .collect()
}

fn seconds(hit: &MeleeHit) -> f64 {
    hit.when.recording_time.expect("a hit has a recording time")
}

fn action_seconds(action: &ShieldAction) -> f64 {
    (action.when.recording_time).expect("a shield action has a recording time")
}

/// A clock reading holds for a second, and the kill feed is placed by the
/// packet that reported the kill.
const CLOCK_SLACK: f64 = 1.5;

/// What one round held, added up over the rounds checked.
#[derive(Debug, Default)]
struct Counts {
    rounds: usize,
    hits: usize,
    barricade_hits: usize,
    map_object_hits: usize,
    broken: usize,
    bashes: usize,
    knife_seen: usize,
    raises: usize,
    stows: usize,
    drops: usize,
    extensions: usize,
    shield_players: usize,
    montagnes: usize,
}

/// Checks what must hold for the melee hits and shield actions of any
/// round. `name` names the round in a failure.
fn check(round: &Round, name: &str, counts: &mut Counts) {
    let players: HashSet<&str> = (round.header.players.iter())
        .map(|p| p.username.as_str())
        .collect();
    // Players whose primary is a shield, on the operator they spawned with.
    let shielded: HashSet<&str> = (round.loadouts.iter())
        .filter(|l| l.primary.as_ref().is_some_and(|w| w.shield))
        .map(|l| l.username.as_str())
        .collect();
    counts.rounds += 1;
    let skips = round.timing.as_ref().map_or(&[][..], |t| &t.skips);

    let hits = &round.melee_hits;
    for w in hits.windows(2) {
        assert!(
            seconds(&w[0]) <= seconds(&w[1]),
            "{name}: hits out of order"
        );
    }
    // The clock is the state stream's: an event carries the reading of its
    // own frame, so it agrees with the kill feed around it and does not
    // stay at the start of prep.
    let whens: Vec<&When> = (hits.iter().map(|h| &h.when))
        .chain(round.shield_actions.iter().map(|a| &a.when))
        .collect();
    for w in &whens {
        let t = w.recording_time.expect("an event has a recording time");
        for u in &round.match_feedback {
            let Some(feed) = u.recording_time else {
                continue;
            };
            if t < feed - CLOCK_SLACK {
                assert!(w.elapsed <= u.elapsed, "{name}: {w:?} is before {u:?}");
            }
            if t > feed + CLOCK_SLACK {
                assert!(w.elapsed >= u.elapsed, "{name}: {w:?} is after {u:?}");
                let prep = w.phase == Phase::Prep && u.phase != Phase::Prep;
                assert!(!prep, "{name}: {w:?} is after {u:?}");
            }
        }
        for other in &whens {
            if other.recording_time < w.recording_time {
                assert!(other.elapsed <= w.elapsed, "{name}: {other:?} {w:?}");
            }
        }
    }
    let mut by_object: HashMap<&str, Vec<&MeleeHit>> = HashMap::new();
    for h in hits {
        assert!(players.contains(h.username.as_str()), "{name}: {h:?}");
        assert!(u64::from_str_radix(&h.object, 16).is_ok(), "{name}: {h:?}");
        assert!(
            h.position.is_some() != h.point.is_some(),
            "{name}: one of position and point: {h:?}"
        );
        for v in h.position.iter().chain(&h.point).flatten() {
            assert!(v.is_finite(), "{name}: {h:?}");
        }
        assert!(!h.broke || h.target == Target::Barricade, "{name}: {h:?}");
        // A map object has a 40-bit id, an entity a 32-bit one.
        assert_eq!(
            h.target == Target::MapObject,
            h.object.len() > 8,
            "{name}: {h:?}"
        );
        assert!(
            !h.with_shield || shielded.contains(h.username.as_str()),
            "{name}: a bash by a player with no shield: {h:?}"
        );
        by_object.entry(&h.object).or_default().push(h);
        counts.hits += 1;
        counts.barricade_hits += usize::from(h.target == Target::Barricade);
        counts.map_object_hits += usize::from(h.target == Target::MapObject);
        counts.broken += usize::from(h.broke);
        counts.bashes += usize::from(h.with_shield);
        counts.knife_seen += usize::from(h.knife_seen);
    }
    for (object, hits) in by_object {
        assert_eq!(hits[0].hit, 1, "{name}: {object} starts at {:?}", hits[0]);
        for w in hits.windows(2) {
            let (a, b) = (w[0], w[1]);
            // Numbers run upward, and start over once the object broke.
            assert!(
                b.hit == a.hit + 1 && !a.broke || b.hit == 1,
                "{name}: {object} goes from {a:?} to {b:?}"
            );
            assert_eq!(a.target, b.target, "{name}: {object}");
            // One player cannot swing again within 0.9 s of game time
            // (1.02 s is the least in 524 pairs of 188 rounds). Where the
            // recording skips game time between the two, the hits are
            // that much closer on its clock than they were.
            if a.username == b.username {
                let skipped: f64 = skips
                    .iter()
                    .filter(|s| s.at < seconds(b) && s.until > seconds(a))
                    .map(|s| s.seconds)
                    .sum();
                let gap = seconds(b) - seconds(a) + skipped;
                assert!(
                    gap >= 0.9,
                    "{name}: {object} hit twice in {gap:.3} s by {}",
                    a.username
                );
            }
        }
    }

    let actions = &round.shield_actions;
    for w in actions.windows(2) {
        assert!(
            action_seconds(&w[0]) <= action_seconds(&w[1]),
            "{name}: shield actions out of order"
        );
    }
    let mut by_player: HashMap<&str, Vec<&ShieldAction>> = HashMap::new();
    for a in actions {
        assert!(
            shielded.contains(a.username.as_str()),
            "{name}: a shield action by a player with no shield: {a:?}"
        );
        if a.action != ShieldActionType::Extend {
            let none = [a.extend_time, a.extended_for, a.retract_time, a.duration];
            assert_eq!(none, [None; 4], "{name}: {a:?}");
        }
        by_player.entry(&a.username).or_default().push(a);
    }
    counts.shield_players += by_player.len();
    for (player, actions) in by_player {
        // The shield goes in hand, then away, in turn.
        let mut in_hand = false;
        for a in actions
            .iter()
            .filter(|a| a.action != ShieldActionType::Extend)
        {
            let raise = a.action == ShieldActionType::Raise;
            assert_ne!(raise, in_hand, "{name}: {player} repeats {a:?}");
            in_hand = raise;
            counts.raises += usize::from(raise);
            counts.stows += usize::from(a.action == ShieldActionType::Stow);
            counts.drops += usize::from(a.action == ShieldActionType::Drop);
        }
        // Extensions follow each other without overlap.
        let extensions: Vec<&&ShieldAction> = (actions.iter())
            .filter(|a| a.action == ShieldActionType::Extend)
            .collect();
        counts.extensions += extensions.len();
        counts.montagnes += usize::from(!extensions.is_empty());
        for e in &extensions {
            let parts = [e.extend_time, e.extended_for, e.retract_time, e.duration];
            for part in parts.into_iter().flatten() {
                assert!(part >= 0.0, "{name}: {e:?}");
            }
            let sum: f64 = parts[..3].iter().flatten().sum();
            if let Some(duration) = e.duration {
                assert!(sum <= duration + 0.002, "{name}: parts exceed {e:?}");
            }
        }
        for w in extensions.windows(2) {
            let duration = (w[0].duration)
                .unwrap_or_else(|| panic!("{name}: {player} extends again after {:?}", w[0]));
            assert!(
                action_seconds(w[0]) + duration <= action_seconds(w[1]) + 0.001,
                "{name}: {player}'s extensions overlap: {:?} {:?}",
                w[0],
                w[1]
            );
        }
    }
}

#[test]
fn test_rounds_hold_consistent_melee_hits_and_shield_actions() {
    let Some(dir) = data_dir() else { return };
    let mut counts = Counts::default();
    for (name, round) in custom_rounds(&dir) {
        check(&round, &name, &mut counts);
        assert!(!round.melee_hits.is_empty(), "{name}: no melee hit");
    }
    eprintln!("test rounds: {counts:?}");
    if counts.rounds == 10 {
        assert_eq!(counts.hits, 79);
        assert_eq!(counts.broken, 10);
        assert_eq!(counts.extensions, 8);
    }
}

/// Facts read off the bytes of two test rounds.
#[test]
fn test_rounds_hold_the_hits_and_extensions_seen_in_the_bytes() {
    let Some(dir) = data_dir() else { return };
    let rounds = custom_rounds(&dir);
    let round = |name: &str| rounds.iter().find(|r| r.0 == name).map(|r| &r.1);

    // custom_1: Blackbeard bashes one barricade three times, a second
    // apart, and the third breaks it.
    if let Some(r) = round("custom_1") {
        let hits: Vec<&MeleeHit> = (r.melee_hits.iter())
            .filter(|h| h.object == "f02b8b7b")
            .collect();
        assert_eq!(hits.len(), 3);
        for (i, h) in hits.iter().enumerate() {
            assert_eq!(h.username, "kds.FaZe");
            assert_eq!(h.target, Target::Barricade);
            assert_eq!(h.hit as usize, i + 1);
            assert_eq!(h.broke, i == 2);
            assert!(h.with_shield);
        }
        assert_eq!(seconds(hits[0]), 60.672);
        assert_eq!(hits[0].position, Some([-83.954, 33.781, 1.389]));
        // The same swing lands on two parts of the map.
        let first: Vec<&MeleeHit> = (r.melee_hits.iter())
            .filter(|h| seconds(h) == 58.326)
            .collect();
        assert_eq!(first.len(), 2);
        assert!(
            first
                .iter()
                .all(|h| h.target == Target::MapObject && h.knife_seen)
        );
        assert_ne!(first[0].object, first[1].object);
        // 58 s in, action has started and the clock shows 2:46.
        for h in first {
            assert_eq!(h.when.phase, Phase::Action);
            assert_eq!((h.when.time.as_str(), h.when.elapsed), ("2:46", 58.0));
        }
        let live = |w: &When| w.phase != Phase::Prep && w.elapsed > 0.0;
        assert!(r.melee_hits.iter().all(|h| live(&h.when)));
        assert!(r.shield_actions.iter().all(|a| live(&a.when)));
    }

    // custom_10: Montagne extends seven times, and bashes a barricade that
    // bullets had hit before.
    if let Some(r) = round("custom_10") {
        let extensions: Vec<&ShieldAction> = (r.shield_actions.iter())
            .filter(|a| a.action == ShieldActionType::Extend)
            .collect();
        assert_eq!(extensions.len(), 7);
        assert!(extensions.iter().all(|e| e.username == "Bassetto.L5"));
        let first = extensions[0];
        assert_eq!(action_seconds(first), 93.843);
        assert_eq!(first.extend_time, Some(1.022));
        assert_eq!(first.extended_for, Some(5.654));
        assert_eq!(first.retract_time, Some(0.989));
        assert_eq!(first.duration, Some(7.665));
        let numbers: Vec<(u32, bool, bool)> = (r.melee_hits.iter())
            .map(|h| (h.hit, h.broke, h.with_shield))
            .collect();
        assert_eq!(
            numbers,
            [(1, false, true), (2, false, true), (3, true, true)]
        );
    }

    // custom_7: the player who picked Montagne spawned as another operator
    // and has no shield.
    if let Some(r) = round("custom_7") {
        assert!(r.shield_actions.is_empty());
        assert!(r.melee_hits.iter().all(|h| !h.with_shield));
    }
}

/// `<match id> R<round>`, to name a real round in a failure without naming
/// its players.
fn real_round_name(r: &Round) -> String {
    format!("{} R{}", r.header.match_id, r.header.round_number + 1)
}

/// The same checks on the user's own recordings. These hold what the test
/// rounds lack: Blitz, Fuze and many Montagnes.
#[test]
fn real_rounds_hold_consistent_melee_hits_and_shield_actions() {
    let root = std::env::var_os("R6_MATCH_REPLAY").map(PathBuf::from);
    let Some(root) = root.filter(|r| r.is_dir()) else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let mut counts = Counts::default();
    for dir in replay_analyzer::matches::find_match_folders(&root).unwrap() {
        let m = Match::open_with(&dir, ReadMode::Full).unwrap();
        for round in &m.rounds {
            check(round, &real_round_name(round), &mut counts);
        }
    }
    eprintln!("real rounds: {counts:?}");
}
