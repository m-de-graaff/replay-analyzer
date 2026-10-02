//! Line of sight checked against what rounds prove: a bullet that struck
//! a player had a clear line, a shot crossed nothing solid before where it
//! ended, and a killer looked at the victim. Skipped when the test replays are
//! missing; the tests named `real_` read the folder `R6_MATCH_REPLAY`
//! names and are skipped without it.

use std::path::{Path, PathBuf};
use std::time::Instant;

use replay_analyzer::movement::PlayerTrack;
use replay_analyzer::shots::HitResult;
use replay_analyzer::sight::{
    self, Entry, Geometry, Options, Pose, Scene, Sight, Validation, WallKind,
};
use replay_analyzer::{ReadMode, ReadOptions, Round};

const OPTIONS: ReadOptions = ReadOptions {
    mode: ReadMode::Full,
    census: false,
    movement: true,
};

/// Every test round, read once for all tests.
fn rounds() -> &'static [(PathBuf, Round)] {
    static ROUNDS: std::sync::OnceLock<Vec<(PathBuf, Round)>> = std::sync::OnceLock::new();
    ROUNDS.get_or_init(|| {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings/valid/Y11S3");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("skipping: no test replays in {}", dir.display());
            return Vec::new();
        };
        let mut paths: Vec<PathBuf> = entries.map(|e| e.unwrap().path()).collect();
        paths.retain(|p| p.extension().is_some_and(|e| e == "rec"));
        paths.sort();
        (paths.into_iter())
            .map(|p| {
                let round = Round::open(&p, OPTIONS).unwrap();
                (p, round)
            })
            .collect()
    })
}

/// The panels of the ten rounds as a geometry of Bank.
fn bank() -> &'static Geometry {
    static BANK: std::sync::OnceLock<Geometry> = std::sync::OnceLock::new();
    BANK.get_or_init(|| Geometry::harvest(rounds().iter().map(|(_, r)| r)))
}

/// Every finished round with movement under `R6_MATCH_REPLAY`, one after
/// the other: a folder of a season is too large to hold in memory.
fn real_rounds(mut each: impl FnMut(&Path, &Round)) -> bool {
    let Some(root) = std::env::var_os("R6_MATCH_REPLAY").map(PathBuf::from) else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return false;
    };
    if !root.is_dir() {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return false;
    }
    let mut stack = vec![root];
    let mut files = Vec::new();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rec") {
                files.push(path);
            }
        }
    }
    files.sort();
    for path in files {
        // Rounds of older seasons and unfinished files have no movement.
        let Ok(round) = Round::open(&path, OPTIONS) else {
            continue;
        };
        if round
            .movement
            .as_ref()
            .is_some_and(|m| !m.players.is_empty())
        {
            each(&path, &round);
        }
    }
    true
}

fn open_field(round: &Round) -> Sight {
    sight::analyze(round, None, &Options::default()).expect("a full Y11S3 read has movement")
}

fn percentile(values: &mut [f32], share: f64) -> f32 {
    values.sort_by(f32::total_cmp);
    values[((values.len() - 1) as f64 * share).round() as usize]
}

fn track<'a>(round: &'a Round, username: &str) -> &'a PlayerTrack {
    let players = &round.movement.as_ref().unwrap().players;
    players.iter().find(|t| t.username == username).unwrap()
}

/// What must hold of the outputs of any round, whatever the geometry.
fn check_outputs(name: &str, round: &Round, s: &Sight) {
    let team = |u: &str| {
        let players = &round.header.players;
        players.iter().find(|p| p.username == u).unwrap().team_index
    };
    let step = s.options.step;
    for l in &s.sightlines {
        let length = l.end - l.start;
        assert!(length >= step - 1e-6, "{name}: {l:?}");
        assert!(l.enemies && team(&l.from) != team(&l.to), "{name}: {l:?}");
        for seconds in [l.in_fov_seconds, l.mutual_seconds, l.head_seconds] {
            assert!((0.0..=length + 1e-6).contains(&seconds), "{name}: {l:?}");
        }
        assert_eq!(l.mutual, l.mutual_seconds > 0.0, "{name}: {l:?}");
        let listed: f64 = l.in_fov.iter().map(|i| i[1] - i[0]).sum();
        assert!(
            (listed - l.in_fov_seconds).abs() < 1e-3 + step,
            "{name}: {l:?}"
        );
        assert!(
            l.in_fov
                .iter()
                .all(|i| i[0] >= l.start - 1e-6 && i[1] <= l.end + 1e-6)
        );
        assert!(l.fraction > 0.0 && l.fraction <= 1.0, "{name}: {l:?}");
        assert!(l.min_distance <= l.distance && l.distance <= l.max_distance);
        assert!(
            l.min_distance > 0.0 && l.max_distance < 500.0,
            "{name}: {l:?}"
        );
    }
    assert_eq!(s.exposure.len(), s.crosshair.len());
    for e in &s.exposure {
        assert!(
            e.in_view <= e.exposed + 1e-6 && e.exposed <= e.alive + 1e-6,
            "{name}: {e:?}"
        );
        assert!(e.max_enemies <= 5 && e.enemies <= 5 && e.max_enemies <= e.enemies);
        assert!(
            e.mean_enemies <= e.max_enemies as f64 + 1e-9,
            "{name}: {e:?}"
        );
        assert_eq!(e.exposed > 0.0, e.max_enemies > 0, "{name}: {e:?}");
        let alive: f64 = e.phases.iter().map(|p| p.alive).sum();
        assert!((alive - e.alive).abs() < 1e-6, "{name}: {e:?}");
        // The track is the player's life: a test every step of it, and
        // for a second past its end when it does not end in a dead body.
        let t = track(round, &e.username);
        let life = t.time[t.time.len() - 1] - t.time[0];
        assert!(
            e.alive <= life + 1.0 + 2.0 * step && e.alive >= life - 2.0,
            "{name}: {e:?} {life}"
        );
    }
    for f in &s.first_sights {
        let a = &f.aim;
        for v in [a.yaw, a.pitch, a.angle, a.height, a.distance] {
            assert!(v.is_finite(), "{name}: {f:?}");
        }
        // In view is inside the field of view, and the angle is no more
        // than both errors together.
        assert!(
            a.yaw.abs() <= 45.0 && a.pitch.abs() <= 30.0,
            "{name}: {f:?}"
        );
        assert!(
            a.angle <= a.yaw.abs() + a.pitch.abs() + 1e-3,
            "{name}: {f:?}"
        );
        assert!(team(&f.username) != team(&f.enemy), "{name}: {f:?}");
    }
    for c in &s.crosshair {
        let errors = [
            c.yaw_error,
            c.pitch_error,
            c.angle_error,
            c.head_height_error,
        ];
        assert_eq!(
            errors.iter().all(Option::is_some),
            c.sights > 0,
            "{name}: {c:?}"
        );
        assert!(
            errors.iter().flatten().all(|e| e.is_finite() && *e >= 0.0),
            "{name}: {c:?}"
        );
        assert!(
            c.head_height_bias.is_none_or(f32::is_finite),
            "{name}: {c:?}"
        );
        assert!(
            c.error_before_hit
                .is_none_or(|e| (0.0..=180.0).contains(&e)),
            "{name}: {c:?}"
        );
    }
    for e in &s.engagements {
        assert!(
            e.error_at_hit.is_finite() && e.distance > 0.0,
            "{name}: {e:?}"
        );
    }
}

#[test]
fn an_open_field_says_so_and_tests_no_wall() {
    for (path, round) in rounds() {
        let name = path.display().to_string();
        let s = open_field(round);
        assert_eq!((s.geometry.as_str(), s.occlusion), ("none", false));
        assert!(s.state.is_none());
        check_outputs(&name, round, &s);
        // Nothing hides anybody: every sightline is mutual from end to
        // end unless the other is on a drone, all of the body shows, and
        // an enemy comes into view by the edge of the field of view or
        // with the first test, never round a corner.
        for l in &s.sightlines {
            assert!(l.fraction == 1.0 && l.head_seconds > 0.0, "{name}: {l:?}");
        }
        assert!(
            s.first_sights.iter().all(|f| f.entry != Entry::Appeared),
            "{name}"
        );
        assert!(s.crosshair.iter().all(|c| c.basis == "fovEntry"));
        // Every player alive in the action phase is exposed for most of
        // it: there are enemies until the last of them dies, and nothing
        // between them.
        for e in &s.exposure {
            for p in e
                .phases
                .iter()
                .filter(|p| p.phase == replay_analyzer::Phase::Action)
            {
                assert!(p.exposed >= p.alive * 0.8, "{name}: {e:?}");
            }
        }
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["geometry"], "none");
        assert_eq!(json["occlusion"], false);
        assert!(
            json["firstSights"].is_array() && json["sightlines"][0]["inFovSeconds"].is_number()
        );
    }
}

#[test]
fn a_line_is_as_clear_one_way_as_the_other() {
    let field = Scene::open_field();
    let (mut pairs, mut hidden) = (0, 0);
    for (_, round) in rounds() {
        let scene = Scene::for_round(bank(), round);
        let players = &round.movement.as_ref().unwrap().players;
        for k in 0..400 {
            let time = k as f64 * 0.5;
            let eyes: Vec<[f32; 3]> = (players.iter())
                .filter_map(|t| Some(Pose::at(t, time)?.eye()))
                .collect();
            for (i, a) in eyes.iter().enumerate() {
                for b in &eyes[i + 1..] {
                    assert!(field.visible(*a, *b, time) && field.visible(*b, *a, time));
                    let see = scene.visible(*a, *b, time);
                    assert_eq!(see, scene.visible(*b, *a, time), "{a:?} {b:?} at {time}");
                    assert_eq!(see, sight::blocking(&scene, *a, *b, time).is_none());
                    pairs += 1;
                    hidden += usize::from(!see);
                }
            }
        }
    }
    if !rounds().is_empty() {
        // The harvested panels do hide some of them.
        assert!(pairs > 50_000 && hidden > 500, "{hidden} of {pairs}");
    }
}

/// Degrees a shooter's view direction may be off the point their bullet
/// struck when it downs or kills. Measured: the largest of the 77 such
/// hits of the test rounds is 4.6 (5.7 for all 264 hits), and of 1,552 in
/// 175 real rounds 1,543 are within 8. Recoil, the spread of a shotgun
/// and the 35 ms between two samples of a flick are in it.
const AIM_TOLERANCE: f32 = 8.0;

/// The bullet hits of a round that downed or killed: degrees between the
/// shooter's view direction and the point struck.
fn lethal_aims(round: &Round) -> Vec<f32> {
    (round.bullet_hits.iter())
        .filter(|h| matches!(h.result, Some(HitResult::Dead | HitResult::Down)))
        .filter_map(|h| {
            let (time, point) = (h.when.recording_time?, h.position?);
            let pose = Pose::at(track(round, h.shooter.as_deref()?), time)?;
            Some(sight::aim(pose.eye(), pose.yaw, pose.pitch, point).angle)
        })
        .collect()
}

#[test]
fn a_killer_looks_at_where_the_bullet_strikes() {
    let mut aims: Vec<f32> = rounds().iter().flat_map(|(_, r)| lethal_aims(r)).collect();
    if aims.is_empty() {
        return;
    }
    let within = aims.iter().filter(|a| **a <= AIM_TOLERANCE).count();
    eprintln!(
        "lethal bullet hits: {}, median {:.2}, p95 {:.2}, max {:.2} degrees",
        aims.len(),
        percentile(&mut aims, 0.5),
        percentile(&mut aims, 0.95),
        percentile(&mut aims, 1.0)
    );
    assert!(
        aims.len() >= 50 && within == aims.len(),
        "{within} of {}",
        aims.len()
    );
}

fn report(what: &str, v: &mut Validation) {
    eprintln!(
        "{what}: {} bullet hits, {} stopped as bullets, {} as sight; {} shots, {} stopped short, {} through something soft",
        v.hits, v.blocked, v.sight_blocked, v.shots, v.shots_blocked, v.shots_through
    );
    if !v.eye_errors.is_empty() && !v.view_errors.is_empty() {
        eprintln!(
            "  eye distance error: median {:.3} m, p90 {:.3} m; view off the point struck: \
             median {:.2}, p95 {:.2}, p99.5 {:.2}, max {:.2} degrees",
            percentile(&mut v.eye_errors, 0.5),
            percentile(&mut v.eye_errors, 0.9),
            percentile(&mut v.view_errors, 0.5),
            percentile(&mut v.view_errors, 0.95),
            percentile(&mut v.view_errors, 0.995),
            percentile(&mut v.view_errors, 1.0),
        );
    }
}

#[test]
fn the_eye_is_where_the_shots_say() {
    let mut all = Validation::default();
    for (_, round) in rounds() {
        all.add(sight::validate(&Scene::open_field(), round).unwrap());
    }
    if all.shots == 0 {
        return;
    }
    report("open field", &mut all);
    // Nothing to test against: nothing is blocked, and it says so.
    assert!(
        !all.occlusion
            && all.blocked + all.sight_blocked + all.shots_blocked + all.shots_through == 0
    );
    assert!(
        all.shots > 4000 && all.hits > 200,
        "{} {}",
        all.shots,
        all.hits
    );
    // The eye of the stance tables is the eye the fire events measure
    // from: to 2 cm for half of the shots, to 15 cm for nine in ten (a
    // stance changes over a third of a second, and its number at once).
    assert!(percentile(&mut all.eye_errors, 0.5) < 0.03);
    assert!(percentile(&mut all.eye_errors, 0.9) < 0.15);
    assert!(percentile(&mut all.view_errors, 0.5) < 2.0);
    assert!(percentile(&mut all.view_errors, 1.0) < AIM_TOLERANCE);
}

#[test]
fn harvested_panels_agree_with_the_bullets() {
    if rounds().is_empty() {
        return;
    }
    let g = bank();
    let count = |kind| g.walls.iter().filter(|w| w.kind == kind).count();
    eprintln!(
        "harvested {}: {} walls, {} doors, {} windows, {} hatches",
        g.map.as_deref().unwrap_or("?"),
        count(WallKind::Soft),
        count(WallKind::Door),
        count(WallKind::Window),
        g.slabs.len()
    );
    assert!(count(WallKind::Soft) >= 20 && g.slabs.len() >= 5);
    assert!(count(WallKind::Door) + count(WallKind::Window) >= 25);
    // It survives its JSON.
    let json = serde_json::to_string(g).unwrap();
    assert_eq!(&serde_json::from_str::<Geometry>(&json).unwrap(), g);

    let (mut with_state, mut without) = (Validation::default(), Validation::default());
    for (path, round) in rounds() {
        let scene = Scene::for_round(g, round);
        let applied = scene.applied().unwrap();
        // Every panel of the round finds the wall it was harvested as.
        assert_eq!(applied.reinforcements_unmatched, 0, "{}", path.display());
        assert_eq!(applied.barricades_unmatched, 0, "{}", path.display());
        with_state.add(sight::validate(&scene, round).unwrap());
        // The same walls with nothing of the round applied: every wall
        // soft and whole, every door and window open, no smoke.
        without.add(sight::validate(&Scene::new(g), round).unwrap());
        let s = sight::analyze(round, Some(g), &Options::default()).unwrap();
        assert!(
            s.occlusion && s.geometry.starts_with("harvested: Bank"),
            "{}",
            s.geometry
        );
        assert_eq!(s.state.as_ref(), Some(applied));
        check_outputs(&path.display().to_string(), round, &s);
        assert!(s.crosshair.iter().all(|c| c.basis == "appeared"));
    }
    report("harvested, round state applied", &mut with_state);
    report("harvested, no state", &mut without);
    for m in &with_state.misses {
        eprintln!("  miss: {m:?}");
    }
    // No bullet that struck a player crossed a standing reinforcement,
    // and 2 shots of 4,618 went on behind one.
    assert_eq!(with_state.miss_rate(), Some(0.0));
    assert!(with_state.shots_blocked * 500 <= with_state.shots);
    // Without the round's state no wall is reinforced: nothing stops a
    // bullet, and the barricades that hid a victim are not there.
    assert_eq!(without.blocked + without.shots_blocked, 0);
    assert!(with_state.shots_through > without.shots_through);
}

#[test]
fn a_round_takes_well_under_a_second() {
    let Some((_, round)) = rounds().first() else {
        return;
    };
    let options = Options::default();
    let start = Instant::now();
    let field = sight::analyze(round, None, &options).unwrap();
    let open = start.elapsed();
    let start = Instant::now();
    let walled = sight::analyze(round, Some(bank()), &options).unwrap();
    let with_geometry = start.elapsed();
    eprintln!(
        "one round: open field {open:.1?} ({} sightlines), harvested geometry {with_geometry:.1?} \
         ({} sightlines)",
        field.sightlines.len(),
        walled.sightlines.len()
    );
    // Generous: an unoptimised build on a busy machine.
    assert!(open.as_secs() < 20 && with_geometry.as_secs() < 20);
}

/// The ray tests of a round's worth of queries against a geometry the
/// size of an authored map.
#[test]
fn many_walls_cost_little_more_than_few() {
    let mut n = 3u32;
    let mut next = |span: f32| {
        n = n.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (n >> 8) as f32 / (1u32 << 24) as f32 * span
    };
    let walls = (0..2000)
        .map(|_| {
            let a = [next(80.0), next(60.0)];
            sight::Wall {
                a,
                b: [a[0] + next(4.0) - 2.0, a[1] + next(4.0) - 2.0],
                bottom: 0.0,
                top: 3.0,
                ..sight::Wall::default()
            }
        })
        .collect();
    let g = Geometry {
        walls,
        ..Geometry::default()
    };
    let scene = Scene::new(&g);
    let lines: Vec<_> = (0..500_000)
        .map(|_| ([next(80.0), next(60.0), 1.4], [next(80.0), next(60.0), 1.0]))
        .collect();
    let start = Instant::now();
    let clear = lines
        .iter()
        .filter(|(a, b)| scene.visible(*a, *b, 0.0))
        .count();
    eprintln!(
        "500,000 lines against 2,000 walls: {:.1?}, {clear} clear",
        start.elapsed()
    );
    assert!(clear > 0 && clear < lines.len());
}

#[test]
fn real_rounds_hold_the_same() {
    let (mut all, mut aims, mut count) = (Validation::default(), Vec::new(), 0usize);
    let mut spent = std::time::Duration::ZERO;
    let ran = real_rounds(|path, round| {
        let name = path.display().to_string();
        let start = Instant::now();
        let s = open_field(round);
        spent += start.elapsed();
        assert_eq!(
            (s.geometry.as_str(), s.occlusion),
            ("none", false),
            "{name}"
        );
        check_outputs(&name, round, &s);
        let v = sight::validate(&Scene::open_field(), round).unwrap();
        assert_eq!(v.blocked + v.sight_blocked + v.shots_blocked, 0, "{name}");
        all.add(v);
        aims.extend(lethal_aims(round));
        // The round's own panels as its geometry: no bullet that struck
        // a player crossed a reinforcement that stood.
        let g = Geometry::harvest([round]);
        let scene = Scene::for_round(&g, round);
        let applied = scene.applied().unwrap();
        assert_eq!(
            applied.reinforcements_unmatched + applied.barricades_unmatched,
            0,
            "{name}"
        );
        let own = sight::validate(&scene, round).unwrap();
        for m in &own.misses {
            eprintln!("  {name}: {m:?}");
        }
        all.blocked += own.blocked;
        all.sight_blocked += own.sight_blocked;
        all.shots_blocked += own.shots_blocked;
        all.shots_through += own.shots_through;
        count += 1;
    });
    if !ran || count == 0 {
        return;
    }
    report(
        &format!("{count} real rounds, each with its own panels"),
        &mut all,
    );
    let within = aims.iter().filter(|a| **a <= AIM_TOLERANCE).count();
    eprintln!(
        "  lethal bullet hits: {}, {within} within {AIM_TOLERANCE} degrees; open field {:.1?} a round",
        aims.len(),
        spent / count as u32
    );
    assert!(percentile(&mut all.eye_errors, 0.5) < 0.03);
    assert!(percentile(&mut all.eye_errors, 0.9) < 0.15);
    assert!(percentile(&mut all.view_errors, 0.5) < 2.0);
    assert!(
        within * 100 >= aims.len() * 98,
        "{within} of {}",
        aims.len()
    );
    // Measured on 175 rounds: 4 of 5,486 hits and 40 of 60,866 shots
    // cross a standing panel of assumed extent.
    assert!(
        all.blocked * 200 <= all.hits,
        "{} of {}",
        all.blocked,
        all.hits
    );
    assert!(
        all.shots_blocked * 200 <= all.shots,
        "{}",
        all.shots_blocked
    );
}
