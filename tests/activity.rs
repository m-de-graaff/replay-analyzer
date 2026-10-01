//! Player activity against real replays: the ten rounds of the Y11S3 test
//! match in `test_recordings/valid/Y11S3`, and (ignored unless asked for)
//! every round in the folder `R6_MATCH_REPLAY` names.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use replay_analyzer::activity::{
    self, Activity, EquippedItem, InteractionKind, InteractionOutcome, Signal,
};
use replay_analyzer::{MatchUpdateType, ReadMode, Round, TeamRole};

/// The ten test rounds with their activity, read once.
fn rounds() -> &'static [(Round, Activity)] {
    static ROUNDS: OnceLock<Vec<(Round, Activity)>> = OnceLock::new();
    ROUNDS.get_or_init(|| {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings/valid/Y11S3");
        (1..=10)
            .map(|n| {
                let path = dir.join(format!("custom_{n}.rec"));
                let raw =
                    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                let round = Round::from_bytes(&raw, ReadMode::Full).unwrap();
                let activity = round.activity.clone().unwrap_or_default();
                (round, activity)
            })
            .collect()
    })
}

/// Test round `number` (1 to 10) with its activity.
fn test_round(number: usize) -> (&'static Round, &'static Activity) {
    let (round, activity) = &rounds()[number - 1];
    (round, activity)
}

fn test_rounds() -> impl Iterator<Item = (usize, &'static Round, &'static Activity)> {
    (1..=10).map(|n| {
        let (round, activity) = test_round(n);
        (n, round, activity)
    })
}

fn team_of(round: &Round, username: &str) -> Option<usize> {
    let player = round.header.players.iter().find(|p| p.username == username);
    player.map(|p| p.team_index)
}

fn role_of(round: &Round, username: &str) -> Option<TeamRole> {
    round.header.teams.get(team_of(round, username)?)?.role
}

fn near(a: f64, b: f64, by: f64) -> bool {
    (a - b).abs() <= by
}

/// What holds in every round, as a list of what does not.
fn broken_invariants(round: &Round, a: &Activity) -> Vec<String> {
    let mut out = Vec::new();
    for (i, carry) in a.defuser.iter().enumerate() {
        if role_of(round, &carry.username) != Some(TeamRole::Attack) {
            out.push(format!(
                "the defuser is carried by a non-attacker: {carry:?}"
            ));
        }
        if carry.end.is_some_and(|end| end < carry.start) {
            out.push(format!("a carry ends before it starts: {carry:?}"));
        }
        if let Some(next) = a.defuser.get(i + 1)
            && carry.end.is_none_or(|end| end > next.start)
        {
            out.push(format!("two carriers at once: {carry:?} and {next:?}"));
        }
    }
    for i in &a.interactions {
        let wanted = match i.kind {
            InteractionKind::Plant => TeamRole::Attack,
            InteractionKind::Disable => TeamRole::Defense,
        };
        if role_of(round, &i.username) != Some(wanted) {
            out.push(format!("an interaction by the wrong side: {i:?}"));
        }
        if (i.outcome == InteractionOutcome::Unfinished) != i.end.is_none() {
            out.push(format!("only an unfinished interaction has no end: {i:?}"));
        }
        let carried = a.defuser.iter().any(|c| {
            c.username == i.username && c.start <= i.start && c.end.is_none_or(|e| e >= i.start)
        });
        if i.kind == InteractionKind::Plant && !carried {
            out.push(format!("a plant started without the defuser: {i:?}"));
        }
    }
    // The round's own plant and disable events, which come from
    // `IsDefuserStarted` alone, each have their completed interaction.
    for u in &round.match_feedback {
        let kind = match u.kind {
            MatchUpdateType::DefuserPlantComplete => InteractionKind::Plant,
            MatchUpdateType::DefuserDisableComplete => InteractionKind::Disable,
            _ => continue,
        };
        let found = a.interactions.iter().any(|i| {
            i.kind == kind
                && i.username == u.username
                && i.outcome == InteractionOutcome::Completed
                && i.end
                    .zip(u.recording_time)
                    .is_some_and(|(end, at)| near(end, at, 0.05))
        });
        if !found {
            out.push(format!(
                "no completed {kind:?} by {} at {:?} among {:?}",
                u.username, u.recording_time, a.interactions
            ));
        }
    }
    let completed = |kind| {
        let done = |i: &&activity::DefuserInteraction| {
            i.kind == kind && i.outcome == InteractionOutcome::Completed
        };
        a.interactions.iter().filter(done).count()
    };
    let info = round.info();
    if completed(InteractionKind::Plant) != usize::from(info.planted) {
        out.push(format!(
            "{} completed plants in a round with planted = {}",
            completed(InteractionKind::Plant),
            info.planted
        ));
    }
    for e in &a.equipped {
        if let EquippedItem::Other(raw) = e.item {
            out.push(format!("{} holds unknown item {raw}", e.username));
        }
    }
    for s in &a.ability {
        let flag = !matches!(
            s.signal,
            Signal::GaugeState | Signal::Cooldown | Signal::DeviceState | Signal::CallState
        );
        if flag && s.value > 1 {
            out.push(format!("a flag that is not 0 or 1: {s:?}"));
        }
    }
    for p in &a.reinforcement_pool {
        if p.left > 10 {
            out.push(format!("a pool outside 0 to 10: {p:?}"));
        }
        if round.header.teams.get(p.team).and_then(|t| t.role) != Some(TeamRole::Defense) {
            out.push(format!("a pool of a team that does not defend: {p:?}"));
        }
    }
    let known = |name: &String| team_of(round, name).is_some();
    let names = (a.defuser.iter().map(|c| &c.username))
        .chain(a.interactions.iter().map(|i| &i.username))
        .chain(a.equipped.iter().map(|e| &e.username))
        .chain(a.ability.iter().map(|s| &s.username))
        .chain(a.reloads.iter().map(|r| &r.username));
    for name in names {
        if !known(name) {
            out.push(format!("{name} is not a player of the round"));
        }
    }
    let rising = |times: Vec<f64>| times.windows(2).all(|w| w[0] <= w[1]);
    let lists = [
        ("defuser", a.defuser.iter().map(|c| c.start).collect()),
        (
            "interactions",
            a.interactions.iter().map(|i| i.start).collect(),
        ),
        ("equipped", a.equipped.iter().map(|e| e.time).collect()),
        ("ability", a.ability.iter().map(|s| s.time).collect()),
        ("reloads", a.reloads.iter().map(|r| r.time).collect()),
        (
            "reinforcementPool",
            a.reinforcement_pool.iter().map(|p| p.time).collect(),
        ),
    ];
    for (name, times) in lists {
        if !rising(times) {
            out.push(format!("{name} is not in the order it happened"));
        }
    }
    out
}

#[test]
fn the_test_rounds_keep_every_invariant() {
    for (n, round, a) in test_rounds() {
        let broken = broken_invariants(round, a);
        assert!(broken.is_empty(), "round {n}: {broken:#?}");
    }
}

#[test]
fn every_player_of_a_test_round_has_something_in_their_hands() {
    for (n, round, a) in test_rounds() {
        for p in &round.header.players {
            let held = a.equipped.iter().filter(|e| e.username == p.username);
            assert!(held.count() > 0, "round {n}: nothing for {}", p.username);
        }
        // Nothing is listed twice in a row for one player.
        let mut last = std::collections::HashMap::new();
        for e in &a.equipped {
            let before = last.insert(&e.username, e.item);
            assert_ne!(before, Some(e.item), "round {n}: {e:?} repeats");
        }
    }
}

#[test]
fn the_defuser_dropped_in_round_1_is_picked_up_by_a_teammate() {
    let (round, a) = test_round(1);
    assert_eq!(a.defuser.len(), 2, "{:?}", a.defuser);
    let (dropped, picked_up) = (&a.defuser[0], &a.defuser[1]);
    let end = dropped.end.expect("the first carrier lost the defuser");
    assert!(near(end, 159.49, 0.01), "dropped at {end}");
    assert!(
        near(picked_up.start, 162.04, 0.01),
        "picked up at {}",
        picked_up.start
    );
    assert_ne!(dropped.username, picked_up.username);
    assert_eq!(
        team_of(round, &dropped.username),
        team_of(round, &picked_up.username)
    );
    assert!(a.interactions.is_empty(), "{:?}", a.interactions);
}

#[test]
fn the_defuser_handed_over_within_a_frame_in_round_6_gives_two_carries() {
    let (_, a) = test_round(6);
    let handed = a
        .defuser
        .windows(2)
        .find(|w| w[0].end == Some(w[1].start))
        .expect("a carry that ends as the next starts");
    assert_ne!(handed[0].username, handed[1].username);
    assert!(near(handed[1].start, 163.376, 0.001), "{handed:?}");
}

#[test]
fn the_plant_given_up_in_round_7_is_aborted() {
    let (_, a) = test_round(7);
    let first = a.interactions.first().expect("round 7 has interactions");
    assert_eq!(
        (first.kind, first.outcome),
        (InteractionKind::Plant, InteractionOutcome::Aborted)
    );
    assert!(near(first.start, 215.315, 0.0005), "{first:?}");
    assert!(
        near(first.end.unwrap(), 218.442, 0.0005),
        "ended at {:?}",
        first.end
    );
    // The round still saw a plant, by another player, and its disable.
    let outcomes: Vec<_> = a.interactions.iter().map(|i| (i.kind, i.outcome)).collect();
    assert_eq!(
        outcomes,
        [
            (InteractionKind::Plant, InteractionOutcome::Aborted),
            (InteractionKind::Plant, InteractionOutcome::Completed),
            (InteractionKind::Disable, InteractionOutcome::Completed),
        ]
    );
}

#[test]
fn completed_plants_are_the_planted_test_rounds() {
    let planted: Vec<usize> = test_rounds()
        .filter(|(_, _, a)| {
            a.interactions.iter().any(|i| {
                i.kind == InteractionKind::Plant && i.outcome == InteractionOutcome::Completed
            })
        })
        .map(|(n, ..)| n)
        .collect();
    assert_eq!(planted, [2, 3, 4, 7]);
}

#[test]
fn a_plant_the_round_end_cut_short_is_unfinished() {
    for n in [6, 8] {
        let (_, a) = test_round(n);
        let last = a.interactions.last().expect("the round has a plant");
        assert_eq!(
            (last.kind, last.outcome, last.end),
            (InteractionKind::Plant, InteractionOutcome::Unfinished, None),
            "round {n}"
        );
        // Whoever plants still carries the defuser when the file ends.
        let carry = a.defuser.last().unwrap();
        assert_eq!((&carry.username, carry.end), (&last.username, None));
    }
    for (n, _, a) in test_rounds().filter(|(n, ..)| ![6, 8].contains(n)) {
        let unfinished = |i: &activity::DefuserInteraction| i.end.is_none();
        assert!(!a.interactions.iter().any(unfinished), "round {n}");
    }
}

#[test]
fn the_defenders_of_a_test_round_start_with_ten_reinforcements() {
    for (n, round, a) in test_rounds() {
        let pool = &a.reinforcement_pool;
        let first = pool.first().unwrap_or_else(|| panic!("round {n}: no pool"));
        assert_eq!((first.time, first.left), (0.0, 10), "round {n}");
        let defenders = round
            .header
            .teams
            .iter()
            .position(|t| t.role == Some(TeamRole::Defense));
        assert!(
            pool.iter().all(|p| Some(p.team) == defenders),
            "round {n}: {pool:?}"
        );
        assert!(pool.len() > 5, "round {n}: {pool:?}");
        let repeats = pool.windows(2).any(|w| w[0].left == w[1].left);
        assert!(!repeats, "round {n}: {pool:?}");
    }
    // Round 1: a reinforcement started and cancelled gives one back.
    let (_, a) = test_round(1);
    let left: Vec<u32> = a.reinforcement_pool.iter().map(|p| p.left).collect();
    assert_eq!(left, [10, 9, 8, 9, 8, 7, 6, 5, 4, 3, 2]);
}

#[test]
fn abilities_and_reloads_are_changes_only() {
    for (n, _, a) in test_rounds() {
        assert!(!a.ability.is_empty() && !a.reloads.is_empty(), "round {n}");
        let mut signals = std::collections::HashMap::new();
        for s in &a.ability {
            let before = signals.insert((&s.username, s.slot, s.signal), s.value);
            assert_ne!(before.unwrap_or(0), s.value, "round {n}: {s:?} repeats");
        }
        let mut reloads = std::collections::HashMap::new();
        for r in &a.reloads {
            let before = reloads.insert((&r.username, r.weapon), r.reloading);
            assert_ne!(
                before.unwrap_or(false),
                r.reloading,
                "round {n}: {r:?} repeats"
            );
        }
    }
}

fn recordings(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            recordings(&path, out);
        } else if path.extension().is_some_and(|x| x == "rec") {
            out.push(path);
        }
    }
}

/// Run with `R6_MATCH_REPLAY` set to a folder of the game's own recordings:
/// `cargo test --release --test activity -- --ignored`. The folder is only
/// read.
#[test]
#[ignore = "needs R6_MATCH_REPLAY"]
fn players_own_recordings_keep_every_invariant() {
    let dir = std::env::var_os("R6_MATCH_REPLAY").expect("R6_MATCH_REPLAY names a folder");
    let mut files = Vec::new();
    recordings(Path::new(&dir), &mut files);
    assert!(!files.is_empty(), "no .rec files in {dir:?}");
    let mut failures = Vec::new();
    for path in files {
        let raw = std::fs::read(&path).unwrap();
        let Ok(round) = Round::from_bytes(&raw, ReadMode::Full) else {
            continue;
        };
        let a = round.activity.clone().unwrap_or_default();
        let broken = broken_invariants(&round, &a);
        if !broken.is_empty() {
            failures.push(format!("{}: {broken:#?}", path.display()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
