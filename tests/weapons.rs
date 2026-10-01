//! What players held, fired and reloaded, checked against what the game
//! must agree on: the kill feed's weapon, the loadouts' ammunition and the
//! size of a magazine. Data as in `tests/replays.rs`.

use std::path::{Path, PathBuf};

use replay_analyzer::weapons::{Activity, Item, ReloadOutcome};
use replay_analyzer::{MatchUpdateType, ReadMode, Round};

fn test_rounds() -> Vec<Round> {
    let root = match std::env::var_os("R6_TEST_DATA") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings"),
    };
    let dir = root.join("valid/Y11S3");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("skipping: no test replays in {dir:?}");
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rec"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|p| Round::open(p, ReadMode::Full).unwrap())
        .collect()
}

/// Real rounds from `R6_MATCH_REPLAY`, every `step`th: a player's own
/// recordings, where the test rounds are a spectator's.
fn real_rounds(step: usize) -> Vec<Round> {
    let Some(root) = std::env::var_os("R6_MATCH_REPLAY").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut rounds = Vec::new();
    for dir in replay_analyzer::matches::find_match_folders(&root).unwrap() {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "rec"))
            .collect();
        files.sort();
        for file in files.iter().step_by(step) {
            if let Ok(round) = Round::open(file, ReadMode::Full)
                && round.container.as_ref().is_some_and(|c| c.complete)
            {
                rounds.push(round);
            }
        }
    }
    rounds
}

fn activity<'a>(round: &'a Round, name: &str) -> Option<&'a Activity> {
    round.weapon_activity.iter().find(|a| a.username == name)
}

/// `part` of `whole` as a share; 1 when there is nothing to count.
fn share(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        1.0
    } else {
        part as f64 / whole as f64
    }
}

/// Counts over a set of rounds of what the HUD says against what else the
/// replay says.
#[derive(Debug, Default)]
struct Agreement {
    players: usize,
    with_activity: usize,
    /// Guns with ammunition in `loadouts`, and those whose `fired` there is
    /// the rounds `weaponActivity` saw leave them.
    guns: usize,
    guns_agree: usize,
    /// Drops of a gun's ammunition, and those with that gun in hand.
    fired: usize,
    fired_in_hand: usize,
    /// Kills with a gun the killer carries, and those with it in hand.
    gun_kills: usize,
    gun_kills_in_hand: usize,
    /// Drops of a gadget's count (`uses`), and those with it in hand
    /// shortly before.
    gadget_uses: usize,
    gadget_uses_in_hand: usize,
    reloads: usize,
    completed: usize,
    kills: usize,
    victims_placed: usize,
}

fn agreement(rounds: &[Round]) -> Agreement {
    let mut a = Agreement::default();
    for round in rounds {
        a.players += round.header.players.len();
        a.with_activity += round.weapon_activity.len();
        for act in &round.weapon_activity {
            let loadout = (round.loadouts.iter())
                .rfind(|l| l.username == act.username && l.primary.is_some());
            let guns = loadout.into_iter().flat_map(|l| {
                [(Item::Primary, &l.primary), (Item::Secondary, &l.secondary)]
                    .into_iter()
                    .filter_map(|(slot, w)| Some((slot, w.as_ref()?.ammo?)))
            });
            for (slot, ammo) in guns {
                let seen: u32 = (act.fired.iter())
                    .filter(|f| f.slot == slot)
                    .map(|f| f.rounds)
                    .sum();
                a.guns += 1;
                a.guns_agree += usize::from(seen == ammo.fired);
            }
            for f in &act.fired {
                let Some(t) = f.when.recording_time else {
                    continue;
                };
                a.fired += 1;
                a.fired_in_hand += usize::from(act.at(t).held == f.slot);
            }
            if let Some(l) = loadout {
                let uses = l.gadget.iter().flat_map(|g| &g.counts).flat_map(|c| &c.uses);
                for t in uses.filter_map(|u| u.recording_time) {
                    a.gadget_uses += 1;
                    let held = |back: f64| act.at(t - back).held == Item::Gadget;
                    a.gadget_uses_in_hand += usize::from(held(0.0) || held(0.5) || held(1.0));
                }
            }
            a.reloads += act.reloads.len();
            a.completed += (act.reloads.iter())
                .filter(|r| r.outcome == ReloadOutcome::Completed)
                .count();
        }
        let kills = (round.match_feedback.iter()).filter(|u| u.kind == MatchUpdateType::Kill);
        for kill in kills {
            a.kills += 1;
            a.victims_placed += usize::from(
                activity(round, &kill.target).is_some_and(|v| v.at_death.is_some()),
            );
            let (Some(killer), Some(t)) = (activity(round, &kill.username), kill.recording_time)
            else {
                continue;
            };
            let carried = (round.loadouts.iter())
                .filter(|l| l.username == kill.username)
                .any(|l| l.weapons.contains(&kill.weapon));
            if carried {
                a.gun_kills += 1;
                // A kill can be named a moment after the killer's hands
                // changed.
                let held = |back: f64| killer.at(t - back).id == Some(kill.weapon);
                a.gun_kills_in_hand += usize::from(held(0.0) || held(0.5));
            }
        }
    }
    a
}

fn check_agreement(a: &Agreement, what: &str) {
    println!("{what}: {a:?}");
    assert!(
        share(a.with_activity, a.players) >= 0.95,
        "{what}: players without weapon activity: {a:?}"
    );
    assert!(
        share(a.guns_agree, a.guns) >= 0.99,
        "{what}: guns whose shots disagree with the loadout: {a:?}"
    );
    assert!(
        share(a.fired_in_hand, a.fired) >= 0.99,
        "{what}: shots from a gun that was not in hand: {a:?}"
    );
    assert!(
        share(a.gun_kills_in_hand, a.gun_kills) >= 0.9,
        "{what}: kills with a gun that was not in hand: {a:?}"
    );
    assert!(
        share(a.gadget_uses_in_hand, a.gadget_uses) >= 0.95,
        "{what}: gadgets used without being in hand: {a:?}"
    );
    assert!(
        share(a.completed, a.reloads) >= 0.8,
        "{what}: few reloads complete: {a:?}"
    );
    assert_eq!(
        a.victims_placed, a.kills,
        "{what}: victims without a state at death"
    );
}

#[test]
fn hud_agrees_with_the_kill_feed_and_the_loadouts() {
    let rounds = test_rounds();
    if rounds.is_empty() {
        return;
    }
    let a = agreement(&rounds);
    check_agreement(&a, "test rounds");
    // Counted from the bytes of the ten test rounds.
    if rounds.len() == 10 && std::env::var_os("R6_TEST_DATA").is_none() {
        assert_eq!((a.players, a.with_activity), (100, 100));
        assert_eq!((a.guns, a.guns_agree), (193, 193));
    }
}

#[test]
fn real_hud_agrees_with_the_kill_feed_and_the_loadouts() {
    let rounds = real_rounds(4);
    if rounds.is_empty() {
        return;
    }
    check_agreement(&agreement(&rounds), "real rounds");
}

/// A gun never holds more than a magazine and the chambered round, a shot
/// leaves what the HUD showed before minus the rounds fired, and a reload
/// takes from the reserve what it puts in the gun.
#[test]
fn ammunition_adds_up() {
    let mut rounds = test_rounds();
    rounds.extend(real_rounds(8));
    let (mut shots, mut in_sequence) = (0, 0);
    for round in &rounds {
        for act in &round.weapon_activity {
            let loadout = (round.loadouts.iter())
                .rfind(|l| l.username == act.username && l.primary.is_some());
            for slot in [Item::Primary, Item::Secondary] {
                let size = loadout
                    .and_then(|l| match slot {
                        Item::Primary => l.primary.as_ref(),
                        _ => l.secondary.as_ref(),
                    })
                    .and_then(|w| w.ammo?.magazine_size);
                let fired: Vec<_> = act.fired.iter().filter(|f| f.slot == slot).collect();
                for f in &fired {
                    if let (Some(magazine), Some(size)) = (f.magazine, size) {
                        assert!(
                            magazine <= size,
                            "{}: {magazine} rounds left after a shot from a magazine of {size}",
                            act.username
                        );
                    }
                }
                for w in fired.windows(2) {
                    let (Some(before), Some(after)) = (w[0].magazine, w[1].magazine) else {
                        continue;
                    };
                    let reloaded = act.reloads.iter().any(|r| {
                        r.slot == slot
                            && r.when.recording_time >= w[0].when.recording_time
                            && r.when.recording_time <= w[1].when.recording_time
                    });
                    if !reloaded {
                        shots += 1;
                        in_sequence += usize::from(before == after + w[1].rounds);
                    }
                }
            }
            for r in &act.reloads {
                if let (Some(a), Some(b)) = (r.magazine_before, r.magazine_after) {
                    match r.outcome {
                        ReloadOutcome::Completed => assert!(b >= a.min(1), "{r:?}"),
                        ReloadOutcome::Cancelled => assert!(b <= a, "{r:?}"),
                        ReloadOutcome::Unfinished => {}
                    }
                }
                assert!(r.duration.is_none_or(|d| d >= 0.0), "{r:?}");
            }
            for s in &act.swaps {
                assert_ne!(s.from, s.to, "{}: {s:?}", act.username);
                assert!((0.0..=3.0).contains(&s.duration), "{s:?}");
            }
            for w in act.held.windows(2) {
                assert_ne!(w[0].item, w[1].item, "{}: held twice", act.username);
            }
        }
    }
    println!("{in_sequence} of {shots} shots leave the magazine one step down");
    assert!(share(in_sequence, shots) >= 0.99);
}
