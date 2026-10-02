//! Movement tracks checked against what else each round says: spawns, the
//! kill feed, downs and the sides. Skipped when the test replays are
//! missing.

use std::path::{Path, PathBuf};

use replay_analyzer::movement::{Change, Doing, Movement, PlacedKind, PlayerTrack, ViewKind};
use replay_analyzer::{LifeEventType, MatchUpdateType, ReadMode, ReadOptions, Round, TeamRole};

/// Every test round, read once for all tests.
fn rounds() -> &'static [(PathBuf, Round)] {
    static ROUNDS: std::sync::OnceLock<Vec<(PathBuf, Round)>> = std::sync::OnceLock::new();
    ROUNDS.get_or_init(read_rounds)
}

fn read_rounds() -> Vec<(PathBuf, Round)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings/valid/Y11S3");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("skipping: no test replays in {}", dir.display());
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries.map(|e| e.unwrap().path()).collect();
    paths.retain(|p| p.extension().is_some_and(|e| e == "rec"));
    paths.sort();
    let options = ReadOptions {
        mode: ReadMode::Full,
        census: false,
        movement: true,
    };
    (paths.into_iter())
        .map(|p| {
            let round = Round::open(&p, options).unwrap();
            (p, round)
        })
        .collect()
}

fn movement(round: &Round) -> &Movement {
    round.movement.as_ref().expect("a full Y11S3 read has it")
}

fn track<'a>(round: &'a Round, username: &str) -> &'a PlayerTrack {
    let players = &movement(round).players;
    players.iter().find(|t| t.username == username).unwrap()
}

/// Index of the last sample at or before `time`.
fn sample_at(t: &PlayerTrack, time: f64) -> Option<usize> {
    t.time.iter().rposition(|&s| s <= time)
}

/// The value in force at `time`.
fn value_at<T: Copy>(list: &[Change<T>], time: f64) -> Option<T> {
    list.iter().rfind(|c| c.time <= time).map(|c| c.value)
}

#[test]
fn every_body_reads_and_starts_where_it_spawned() {
    for (path, round) in rounds() {
        for p in &round.header.players {
            let t = track(round, &p.username);
            assert_eq!(t.unread, 0, "{}: {}", path.display(), p.username);
            let Some(spawn) = p.spawn_position else {
                assert!(t.time.is_empty(), "{} never spawned", p.username);
                continue;
            };
            assert!(t.time.len() > 100, "{}: {}", path.display(), p.username);
            // An attacker's body settles on the ground before it is shown.
            for (got, want) in [t.x[0], t.y[0], t.z[0]].iter().zip(spawn) {
                assert!(
                    (got - want).abs() < 0.1,
                    "{}: {}",
                    path.display(),
                    p.username
                );
            }
            let columns = [&t.x, &t.y, &t.z, &t.yaw, &t.pitch, &t.speed];
            assert!(columns.iter().all(|c| c.len() == t.time.len()));
        }
    }
}

#[test]
fn tracks_are_continuous() {
    for (path, round) in rounds() {
        for t in &movement(round).players {
            for i in 1..t.time.len() {
                let dt = t.time[i] - t.time[i - 1];
                assert!(
                    dt > 0.0,
                    "{}: {} at {}",
                    path.display(),
                    t.username,
                    t.time[i]
                );
                let step = (t.x[i] - t.x[i - 1]).hypot(t.y[i] - t.y[i - 1]);
                assert!(
                    step < 2.0,
                    "{}: {} moves {step} m at {}",
                    path.display(),
                    t.username,
                    t.time[i]
                );
            }
            assert!(t.pitch.iter().all(|p| p.abs() <= 90.0));
            assert!(t.speed.iter().all(|&s| (0.0..12.0).contains(&s)));
        }
    }
}

/// The feed's kills by a player, with the time and both tracks.
fn kills(round: &Round) -> Vec<(f64, &PlayerTrack, &PlayerTrack)> {
    (round.match_feedback.iter())
        .filter(|u| u.kind == MatchUpdateType::Kill && u.username != u.target)
        .filter_map(|u| {
            let time = u.recording_time?;
            Some((time, track(round, &u.username), track(round, &u.target)))
        })
        .collect()
}

#[test]
fn killers_look_at_their_victims() {
    let (mut near, mut all) = (0, 0);
    for (_, round) in rounds() {
        for (time, killer, victim) in kills(round) {
            let (Some(k), Some(v)) = (sample_at(killer, time), sample_at(victim, time)) else {
                continue;
            };
            let (dx, dy) = (victim.x[v] - killer.x[k], victim.y[v] - killer.y[k]);
            // Yaw 0 looks along +y and grows counter-clockwise.
            let bearing = (-dx).atan2(dy).to_degrees();
            let off = (bearing - killer.yaw[k] + 540.0).rem_euclid(360.0) - 180.0;
            all += 1;
            near += usize::from(off.abs() < 10.0);
        }
    }
    if all > 0 {
        // The rest are gadget kills and flicks.
        assert!(all >= 60 && near * 10 >= all * 9, "{near} of {all}");
    }
}

#[test]
fn a_track_ends_when_its_player_dies() {
    for (path, round) in rounds() {
        for (time, _, victim) in kills(round) {
            let last = victim.doing.last().unwrap();
            assert_eq!(
                last.value,
                Doing::Dead,
                "{}: {}",
                path.display(),
                victim.username
            );
            assert!(
                (last.time - time).abs() < 1.0,
                "{}: {} dies at {time}, body at {}",
                path.display(),
                victim.username,
                last.time
            );
            assert_eq!(victim.time.last(), Some(&last.time));
        }
    }
}

#[test]
fn a_downed_player_is_shown_downed() {
    let (mut shown, mut all) = (0, 0);
    for (_, round) in rounds() {
        for e in &round.life_events {
            let (LifeEventType::Down, Some(time)) = (e.kind, e.recording_time) else {
                continue;
            };
            let t = track(round, &e.username);
            all += 1;
            // A player killed in the same moment goes straight to dead.
            let after = (t.doing.iter()).find(|c| c.time >= time - 0.2 && c.time <= time + 0.5);
            shown += usize::from(after.is_some_and(|c| c.value == Doing::Downed));
        }
    }
    if all > 0 {
        assert!(shown * 5 >= all * 4, "{shown} of {all}");
    }
}

#[test]
fn stances_slow_a_player_down() {
    use replay_analyzer::movement::Stance::{Crouched, Prone, Standing};
    let mut top = [0f32; 3];
    for (_, round) in rounds() {
        for t in &movement(round).players {
            for (i, &time) in t.time.iter().enumerate() {
                // On the ground, moving under the player's own power.
                if value_at(&t.doing, time) != Some(Doing::Nothing)
                    || value_at(&t.airborne, time) != Some(false)
                {
                    continue;
                }
                // Half a second into a stance, so the speed is all of it.
                let since = t.stance.iter().rfind(|c| c.time <= time).unwrap();
                let slot = match since.value {
                    Standing => 0,
                    Crouched => 1,
                    Prone => 2,
                    other => panic!("{} is {other:?} at {time}", t.username),
                };
                if time - since.time > 0.5 {
                    top[slot] = top[slot].max(t.speed[i]);
                }
            }
        }
    }
    if top[0] > 0.0 {
        assert!(top[0] > top[1] && top[1] > top[2], "{top:?}");
    }
}

#[test]
fn only_attackers_rappel_and_only_defenders_reinforce() {
    let (mut finished, mut started) = (0, 0);
    for (path, round) in rounds() {
        let role = |username: &str| {
            let p = (round.header.players.iter()).find(|p| p.username == username)?;
            round.header.teams[p.team_index].role
        };
        let m = movement(round);
        for t in &m.players {
            let ropes = t.doing.iter().any(|c| c.value == Doing::Rappelling);
            assert!(
                !ropes || role(&t.username) == Some(TeamRole::Attack),
                "{}: {}",
                path.display(),
                t.username
            );
        }
        let walls = (m.placements.iter()).filter(|p| p.kind == PlacedKind::Reinforcement);
        // More than the ten a team has can be started: one given up is
        // handed back.
        for w in walls {
            assert_eq!(
                role(&w.username),
                Some(TeamRole::Defense),
                "{}",
                path.display()
            );
            // The hands are busy for the 4.5 seconds a reinforcement takes,
            // or less when it is given up.
            if let Some(end) = w.end {
                let took = end - w.time;
                assert!(took < 4.7, "{}: {w:?}", path.display());
                finished += usize::from(took > 4.4);
                started += 1;
            }
        }
    }
    if started > 0 {
        assert!(finished * 10 >= started * 9, "{finished} of {started}");
    }
}

#[test]
fn every_view_names_its_device() {
    for (path, round) in rounds() {
        let m = movement(round);
        assert!(!m.views.is_empty(), "{}", path.display());
        for v in &m.views {
            assert!(v.end > v.start);
            // A camera of the map has no owner; everything else has one.
            assert!(v.fixed != v.owner.is_some(), "{}: {v:?}", path.display());
            assert!(
                v.kind != ViewKind::Drone || v.position.is_none(),
                "{}: {v:?}",
                path.display()
            );
        }
    }
}

#[test]
fn killers_mostly_aim_down_sights() {
    let (mut aiming, mut all) = (0, 0);
    for (_, round) in rounds() {
        for (time, killer, _) in kills(round) {
            // A quarter second before the kill; overall a player aims about
            // a third of the time.
            if let Some(aims) = value_at(&killer.aiming, time - 0.25) {
                all += 1;
                aiming += usize::from(aims);
            }
        }
    }
    if all > 0 {
        assert!(aiming * 5 >= all * 4, "{aiming} of {all}");
    }
}

#[test]
fn falls_lose_height_in_the_air() {
    let mut storeys = 0;
    for (path, round) in rounds() {
        for t in &movement(round).players {
            for f in &t.falls {
                assert!(
                    f.end > f.start && f.drop >= 1.0,
                    "{}: {f:?}",
                    path.display()
                );
                assert_eq!(value_at(&t.airborne, f.start), Some(true));
                storeys += usize::from(f.drop > 2.5);
            }
        }
    }
    // Hatches and windows: a storey at a time.
    assert!(rounds().is_empty() || storeys > 10, "{storeys}");
}
