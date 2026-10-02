//! Spots with their spotters, spot assists, causes of reveals, phone hacks
//! and what a kill's victim was known by (Y11S3), against the reference
//! decode of the test rounds and, with `R6_MATCH_REPLAY` set, a real
//! `MatchReplay` folder, which is only read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use replay_analyzer::devices::{DeviceEventType, EndKind};
use replay_analyzer::intel::{Cause, Trigger};
use replay_analyzer::joins::{Spot, SpotBy, Tool};
use replay_analyzer::{MatchUpdateType, ReadMode, Round, TeamRole};

/// A spot: the spotted player, seconds into the recording, seconds from
/// the first mark to the last, marks, and who spotted (empty where nobody
/// is named).
type Expected = (&'static str, f64, f64, usize, &'static str);

/// The spots of the ten `custom_*.rec` rounds as the reference decode
/// gives them. The reference has one more, in round 5 at 0.0 s: the mark
/// the stream's snapshot holds from before the recording, which is left
/// out here.
const SPOTS: [&[Expected]; 10] = [
    &[
        ("Bassetto.L5", 16.431, 0.0, 1, "soulz1.FaZe"),
        ("WIZARD.L5", 38.679, 0.0, 1, ""),
        ("Bassetto.L5", 43.837, 0.0, 1, ""),
        ("Bassetto.L5", 128.212, 0.0, 1, "soulz1.FaZe"),
        ("Bassetto.L5", 177.622, 0.0, 1, "vitaking.FaZe"),
    ],
    &[],
    &[
        ("Bassetto.L5", 32.512, 0.0, 1, "kds.FaZe"),
        ("PSYCHO.L5", 46.199, 0.0, 1, "Handyy.FaZe"),
    ],
    &[
        ("pino.L5", 35.525, 0.0, 1, ""),
        ("WIZARD.L5", 92.351, 0.0, 1, ""),
        ("WIZARD.L5", 106.107, 0.0, 1, "soulz1.FaZe"),
        ("Neskin.L5", 110.185, 0.0, 1, "vitaking.FaZe"),
        ("Neskin.L5", 120.782, 0.0, 1, ""),
    ],
    &[
        ("WIZARD.L5", 14.356, 0.0, 1, ""),
        ("pino.L5", 14.356, 0.0, 1, "soulz1.FaZe"),
        ("WIZARD.L5", 18.615, 0.0, 1, ""),
        ("WIZARD.L5", 109.211, 0.0, 1, ""),
        ("WIZARD.L5", 115.646, 0.0, 1, ""),
        ("soulz1.FaZe", 141.062, 0.0, 1, "pino.L5"),
    ],
    &[("PSYCHO.L5", 19.313, 0.0, 1, "")],
    &[
        ("cyber.FaZe", 184.929, 0.0, 1, ""),
        ("vitaking.FaZe", 201.728, 0.0, 1, "Neskin.L5"),
        ("cyber.FaZe", 201.728, 1.631, 2, "Neskin.L5"),
        ("WIZARD.L5", 246.79, 1.991, 2, ""),
        ("WIZARD.L5", 251.749, 0.0, 1, ""),
        ("Handyy.FaZe", 257.523, 1.811, 2, "Neskin.L5"),
    ],
    &[
        ("kds.FaZe", 39.349, 0.0, 1, ""),
        ("cyber.FaZe", 111.415, 0.0, 1, "PSYCHO.L5"),
    ],
    &[
        ("kds.FaZe", 31.689, 0.0, 1, "Bassetto.L5"),
        ("cyber.FaZe", 31.689, 0.0, 1, ""),
        ("soulz1.FaZe", 35.429, 0.0, 1, "pino.L5"),
        ("Handyy.FaZe", 41.012, 0.0, 1, ""),
    ],
    &[
        ("kds.FaZe", 36.607, 0.0, 1, ""),
        ("Bassetto.L5", 163.429, 3.576, 3, "vitaking.FaZe"),
    ],
];

/// Marks of one scan are at most this far apart, and a spot assist's mark
/// at most this old (seconds).
const SPOT_GAP: f64 = 2.5;
const ASSIST_WINDOW: f64 = 14.0;
/// What a kill's victim is known by is at most this old.
const KNOWN_SECONDS: f64 = 15.0;

/// The ten rounds of one custom match, in round order: read once, shared
/// by the tests.
fn test_rounds() -> &'static [(String, Round)] {
    static ROUNDS: std::sync::OnceLock<Vec<(String, Round)>> = std::sync::OnceLock::new();
    ROUNDS.get_or_init(|| {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings/valid/Y11S3");
        (1..=10)
            .map(|n| {
                let name = format!("custom_{n}");
                let round = Round::open(dir.join(format!("{name}.rec")), ReadMode::Full).unwrap();
                (name, round)
            })
            .collect()
    })
}

fn team(round: &Round, username: &str) -> Option<usize> {
    let player = round.header.players.iter().find(|p| p.username == username);
    player.map(|p| p.team_index)
}

fn operator(round: &Round, username: &str) -> String {
    let player = round.header.players.iter().find(|p| p.username == username);
    player.map_or(String::new(), |p| p.operator.to_string())
}

/// What a device of `drones` or `cameras` is, by its entity id.
fn tool(round: &Round, entity: &str) -> Option<Tool> {
    if round.drones.iter().any(|d| d.entity == entity) {
        Some(Tool::Drone)
    } else if round.cameras.iter().any(|c| c.entity == entity) {
        Some(Tool::Camera)
    } else {
        None
    }
}

/// What holds for the joins of every round; the failures, each named by
/// `label`.
fn violations(label: &str, round: &Round) -> Vec<String> {
    let mut out = Vec::new();
    let mut fail = |what: String| out.push(format!("{label}: {what}"));
    for s in &round.spots {
        let (Some(of), Some(seen_by)) = (team(round, &s.username), s.seen_by) else {
            fail("a spot of nobody, or seen by no team".into());
            continue;
        };
        if of == seen_by {
            fail("a spot seen by the spotted player's own team".into());
        }
        // The marks of one scan, in order.
        let gaps = s.mark_times.windows(2).map(|w| w[1] - w[0]);
        let span = s.mark_times.last().zip(s.mark_times.first());
        let apart = span.map_or(-1.0, |(last, first)| last - first);
        if s.marks != s.mark_times.len()
            || s.marks == 0
            || gaps.clone().any(|g| g <= 0.0 || g > SPOT_GAP)
            || (apart - s.seconds).abs() > 0.0015
            || s.when.recording_time != s.mark_times.first().copied()
        {
            fail(format!("a spot of {} marks over {} s", s.marks, s.seconds));
        }
        // A spotter, or candidates, or nobody on a device: all of the
        // team that sees the spot.
        if s.by.is_some() != s.by_source.is_some()
            || (s.by.is_some() && !s.by_candidates.is_empty())
        {
            fail(format!(
                "a spot by {:?} and {:?}",
                s.by_source, s.by_candidates
            ));
        }
        for name in s.by.iter().chain(&s.by_candidates) {
            if team(round, name) != Some(seen_by) {
                fail(format!("a spotter not of team {seen_by}"));
            }
        }
        if !s.by_candidates.is_sorted() || s.by_candidates.len() == 1 {
            fail(format!("{} candidates", s.by_candidates.len()));
        }
        // A device named is one of the round, of the kind `with` says.
        if let Some(device) = &s.device
            && (tool(round, device) != s.with || s.with.is_none())
        {
            fail(format!("a spot with {:?}, device {device}", s.with));
        }
        // The points name the spotter of the spot they were paid for.
        if s.by_source == Some(SpotBy::SpotAssistScore) {
            let paid = (round.spot_assists.iter())
                .any(|a| a.victim == s.username && Some(&a.username) == s.by.as_ref());
            if !paid {
                fail("a spotter by points nobody got".into());
            }
        }
    }
    for a in &round.spot_assists {
        let names = [&a.username, &a.victim, &a.killer].map(|n| team(round, n));
        let [Some(to), Some(victim), Some(killer)] = names else {
            fail("a spot assist that names nobody".into());
            continue;
        };
        if to != killer || a.username == a.killer || victim == killer {
            fail("a spot assist not for a teammate of the killer".into());
        }
        let at = a.when.recording_time.unwrap_or(f64::MAX);
        let kill = round.match_feedback.iter().find(|u| {
            u.kind == MatchUpdateType::Kill
                && u.username == a.killer
                && u.target == a.victim
                && u.recording_time.is_some_and(|t| (at - t).abs() < 1.0)
        });
        let spotted =
            (round.spots.iter()).any(|s| s.username == a.victim && s.seen_by == Some(killer));
        if kill.is_none() || !spotted || !(0.0..=ASSIST_WINDOW).contains(&a.mark_age) {
            fail(format!("a spot assist {} s after the mark", a.mark_age));
        }
    }
    for t in &round.ability_markers {
        if t.by.is_some() != t.by_source.is_some() {
            fail("a tracking marker by someone without a source".into());
        }
        // The one opponent who plays the operator of the ability.
        let Some(by) = &t.by else { continue };
        let plays = operator(round, by);
        let source = t.source.name.unwrap_or_default();
        let users = (round.header.players.iter()).filter(|p| p.operator.to_string() == plays);
        if team(round, by) == team(round, &t.username)
            || team(round, by).is_none()
            || !source.starts_with(&plays)
            || users
                .filter(|p| team(round, &p.username) == team(round, by))
                .count()
                != 1
        {
            fail(format!("a {source} marker by a {plays}"));
        }
    }
    for r in &round.operator_reveals {
        if r.trigger == Trigger::Kill && r.cause.is_some() {
            fail("a reveal by a kill with another cause".into());
        }
    }
    for h in &round.phone_hacks {
        let whole = h.seconds.is_some_and(|s| s >= 2.4);
        if team(round, &h.username).is_none() || h.completed != whole {
            fail(format!("a phone hack of {:?} s", h.seconds));
        }
    }
    for u in &round.match_feedback {
        let known = [&u.victim_spotted, &u.victim_pinged];
        if known.iter().all(|k| k.is_none()) {
            continue;
        }
        let side = team(round, &u.username);
        if u.kind != MatchUpdateType::Kill || side == team(round, &u.target) || side.is_none() {
            fail("a spotted or pinged victim of no kill of an opponent".into());
        }
        for k in known.into_iter().flatten() {
            let by = k.by.as_deref().map(|by| team(round, by));
            if !(0.0..=KNOWN_SECONDS).contains(&k.seconds_ago) || by.is_some_and(|t| t != side) {
                fail(format!("a victim known {} s before", k.seconds_ago));
            }
        }
        if u.victim_pinged.as_ref().is_some_and(|k| k.by.is_none()) {
            fail("a ping by nobody".into());
        }
    }

    // The stats count what the lists hold.
    let stats = round.player_stats();
    let sum = |f: &dyn Fn(&replay_analyzer::PlayerRoundStats) -> u32| {
        stats.iter().map(f).sum::<u32>() as usize
    };
    let named = round.spots.iter().filter(|s| s.by.is_some()).count();
    let destroyed = |e: &Option<replay_analyzer::devices::End>| {
        e.as_ref().is_some_and(|e| e.kind == EndKind::Destroyed)
    };
    let lost = (round.drones.iter())
        .filter(|d| d.owner.is_some() && destroyed(&d.end))
        .count();
    let ends = (round.drones.iter().map(|d| &d.end)).chain(round.cameras.iter().map(|c| &c.end));
    let credited = ends
        .filter(|e| destroyed(e))
        .filter(|e| e.as_ref().is_some_and(|e| e.by.is_some() && !e.team_kill))
        .count();
    let jams = (round.device_events.iter())
        .filter(|e| e.kind == DeviceEventType::Jam)
        .filter(|e| {
            let drone = round.drones.iter().find(|d| d.entity == e.device);
            drone.is_some_and(|d| d.owner.is_some())
        })
        .count();
    let found = round.objective.as_ref().is_some_and(|o| o.by.is_some());
    let counted = [
        ("pings", sum(&|s| s.pings), round.pings.len()),
        ("timesSpotted", sum(&|s| s.times_spotted), round.spots.len()),
        ("spotsMade", sum(&|s| s.spots_made), named),
        (
            "spotAssists",
            sum(&|s| s.spot_assists),
            round.spot_assists.len(),
        ),
        ("devicesDestroyed", sum(&|s| s.devices_destroyed), credited),
        ("dronesLost", sum(&|s| s.drones_lost), lost),
        ("timesJammed", sum(&|s| s.times_jammed), jams),
        (
            "objectiveFound",
            sum(&|s| s.objective_found),
            usize::from(found),
        ),
    ];
    for (field, stat, listed) in counted {
        if stat != listed {
            fail(format!(
                "stats count {stat} {field}, the round has {listed}"
            ));
        }
    }
    out
}

#[test]
fn the_test_rounds_hold_the_spots_of_the_reference() {
    let mut failures = Vec::new();
    let mut sources: BTreeMap<String, usize> = BTreeMap::new();
    let (mut marks, mut drones, mut cameras) = (0, 0, 0);
    for ((name, round), expected) in test_rounds().iter().zip(SPOTS) {
        failures.extend(violations(name, round));
        let spots: Vec<_> = (round.spots.iter())
            .map(|s| {
                let at = s.when.recording_time.unwrap();
                let by = s.by.as_deref().unwrap_or_default();
                (s.username.as_str(), at, s.seconds, s.marks, by)
            })
            .collect();
        assert_eq!(spots, expected, "{name}");
        for s in &round.spots {
            let source = serde_json::to_value(s.by_source).unwrap();
            *sources.entry(source.to_string()).or_default() += 1;
            marks += s.marks;
            drones += usize::from(s.with == Some(Tool::Drone));
            cameras += usize::from(s.with == Some(Tool::Camera));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // 41 marks, 3 of them said twice in a frame, make 33 spots.
    assert_eq!(marks, 38);
    let split: Vec<(&str, usize)> = sources.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    assert_eq!(
        split,
        [
            ("\"nearestFacingTool\"", 9),
            ("\"onlyObserver\"", 6),
            ("\"spotAssistScore\"", 1),
            ("null", 17),
        ]
    );
    assert_eq!((drones, cameras), (16, 4));
}

#[test]
fn a_spot_names_its_device_and_candidates() {
    let rounds = test_rounds();
    // Round 5: the player who got the points was on their own Black Eye.
    let paid = &rounds[4].1.spots[5];
    assert_eq!(paid.by_source, Some(SpotBy::SpotAssistScore));
    assert_eq!(
        (paid.with, paid.device.as_deref()),
        (Some(Tool::Camera), Some("f0370123"))
    );
    let camera = rounds[4].1.cameras.iter().find(|c| c.entity == "f0370123");
    assert_eq!(camera.unwrap().owner.as_deref(), Some("pino.L5"));
    // Round 1: a spot in prep with all five attackers on drones.
    let open = &rounds[0].1.spots[1];
    assert_eq!(
        (open.by.as_deref(), open.with, &open.device),
        (None, None, &None)
    );
    assert_eq!(
        open.by_candidates,
        [
            "Handyy.FaZe",
            "cyber.FaZe",
            "kds.FaZe",
            "soulz1.FaZe",
            "vitaking.FaZe"
        ]
    );
    let json = serde_json::to_value(&rounds[4].1).unwrap();
    let spot = &json["spots"][5];
    assert_eq!(spot["bySource"], "spotAssistScore");
    assert_eq!(
        (&spot["with"], &spot["device"]),
        (&"camera".into(), &"f0370123".into())
    );
    assert_eq!((&spot["seconds"], &spot["marks"]), (&0.0.into(), &1.into()));
    assert!(spot.get("byCandidates").is_none() && spot.get("markTimes").is_none());
    assert_eq!(
        json["spots"][0]["byCandidates"].as_array().unwrap().len(),
        5
    );
}

#[test]
fn the_one_spot_assist_of_the_test_rounds_is_for_the_killers_teammate() {
    let mut assists = Vec::new();
    for (name, round) in test_rounds() {
        for a in &round.spot_assists {
            assists.push((name.as_str(), a));
        }
    }
    assert_eq!(assists.len(), 1);
    let (name, a) = assists[0];
    assert_eq!(name, "custom_5");
    assert_eq!(
        (a.username.as_str(), a.victim.as_str(), a.killer.as_str()),
        ("pino.L5", "soulz1.FaZe", "PSYCHO.L5")
    );
    assert_eq!((a.mark_age, a.when.recording_time), (3.535, Some(144.665)));
    let stats = test_rounds()[4].1.player_stats();
    let pino = stats.iter().find(|s| s.username == "pino.L5").unwrap();
    assert_eq!((pino.spot_assists, pino.spots_made), (1, 1));
    // The kill says the same of its victim.
    let kill = (test_rounds()[4].1.match_feedback.iter())
        .find(|u| u.kind == MatchUpdateType::Kill && u.target == "soulz1.FaZe");
    let spotted = kill.unwrap().victim_spotted.as_ref().unwrap();
    assert_eq!(
        (spotted.seconds_ago, spotted.by.as_deref()),
        (3.535, Some("pino.L5"))
    );
}

#[test]
fn reveals_markers_and_kills_of_the_test_rounds_match_the_reference() {
    let mut causes = [0; 5];
    let (mut kills, mut spotted, mut pinged) = (0, 0, 0);
    let (mut tracks, mut by) = (0, 0);
    let mut hacks = 0;
    for (_, round) in test_rounds() {
        for r in &round.operator_reveals {
            if r.trigger != Trigger::Identified {
                continue;
            }
            causes[match r.cause {
                Some(Cause::Spot) => 0,
                Some(Cause::TeammateSpot) => 1,
                Some(Cause::Ping) => 2,
                Some(Cause::AbilityMarker) => 3,
                None => 4,
            }] += 1;
        }
        for u in &round.match_feedback {
            kills += usize::from(u.kind == MatchUpdateType::Kill);
            spotted += usize::from(u.victim_spotted.is_some());
            pinged += usize::from(u.victim_pinged.is_some());
        }
        tracks += round.ability_markers.len();
        by += round
            .ability_markers
            .iter()
            .filter(|t| t.by.is_some())
            .count();
        hacks += round.phone_hacks.len();
    }
    // The reference's table of causes, its kills aside: 19 spots, 2 spots
    // of a teammate, 3 pings, 3 ability markers, 11 with nothing.
    assert_eq!(causes, [19, 2, 3, 3, 11]);
    assert_eq!((kills, spotted, pinged), (66, 2, 5));
    // Every marker of Jackal, Alibi, Lion, Grim and Deimos on his target
    // has its player; Deimos's own and the jammed ones name nobody.
    assert_eq!((tracks, by), (48, 37));
    // No round of the test match has Dokkaebi.
    assert_eq!(hacks, 0);
}

#[test]
fn the_stats_of_the_test_match_add_up() {
    let rounds = test_rounds();
    let totals = replay_analyzer::stats::match_stats(rounds.iter().map(|r| &r.1));
    let sum =
        |f: &dyn Fn(&replay_analyzer::PlayerMatchStats) -> u32| totals.iter().map(f).sum::<u32>();
    assert_eq!(sum(&|s| s.pings), 138);
    assert_eq!(sum(&|s| s.times_spotted), 33);
    assert_eq!(sum(&|s| s.spots_made), 16);
    assert_eq!(sum(&|s| s.spot_assists), 1);
    // Every round of the match had its objective found.
    assert_eq!(sum(&|s| s.objectives_found), 10);
    let lost = sum(&|s| s.drones_lost);
    let destroyed = sum(&|s| s.devices_destroyed);
    let jammed = sum(&|s| s.times_jammed);
    println!("{destroyed} devices destroyed, {lost} drones lost, {jammed} jams of a drone");
    assert!(lost > 0 && destroyed >= lost && jammed > 0);
    let json = serde_json::to_value(&rounds[4].1).unwrap();
    let pino = (json["stats"].as_array().unwrap().iter())
        .find(|s| s["username"] == "pino.L5")
        .unwrap();
    assert_eq!(
        (&pino["spotAssists"], &pino["spotsMade"]),
        (&1.into(), &1.into())
    );
}

fn match_replay_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("R6_MATCH_REPLAY")?);
    dir.is_dir().then_some(dir)
}

fn share(count: usize, of: usize) -> String {
    let percent = 100.0 * count as f64 / of.max(1) as f64;
    format!("{count} of {of} ({percent:.1}%)")
}

/// Real rounds: the joins hold their invariants, and how often each rule
/// names someone.
#[test]
fn real_rounds_hold_the_invariants_of_the_joins() {
    let Some(root) = match_replay_dir() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let mut rounds = 0;
    let mut failures = Vec::new();
    let mut spots: Vec<Spot> = Vec::new();
    let (mut assists, mut on_tool, mut agree) = (0, 0, [0; 4]);
    let (mut identified, mut causes) = (0, [0; 4]);
    let (mut kills, mut spotted, mut named, mut pinged, mut placed) = (0, 0, 0, 0, 0);
    let (mut tracks, mut by) = (0, 0);
    let (mut hacks, mut completed, mut after_death, mut dokkaebi) = (0, 0, 0, 0);
    let mut hack_rounds = 0;
    for dir in replay_analyzer::matches::find_match_folders(&root).unwrap() {
        let folder = replay_analyzer::Match::open_with(&dir, ReadMode::Full).unwrap();
        for round in &folder.rounds {
            if round.decode.get("intel").is_none() {
                continue;
            }
            // Names the round without naming its players.
            let name = format!(
                "{} R{}",
                round.header.match_id,
                round.header.round_number + 1
            );
            rounds += 1;
            failures.extend(violations(&name, round));
            spots.extend(round.spots.iter().cloned());
            for a in &round.spot_assists {
                assists += 1;
                // Was the recipient on a device at the spot they were
                // paid for, and whom would the other rules have named?
                let spot = round.spots.iter().rfind(|s| {
                    s.username == a.victim && s.by.as_deref() == Some(a.username.as_str())
                });
                on_tool += usize::from(spot.is_some_and(|s| s.device.is_some()));
            }
            for s in &round.spots {
                // A spot the points name: what the tables alone would say
                // is not kept, so only the other spots are split by rule.
                agree[match s.by_source {
                    Some(SpotBy::SpotAssistScore) => 0,
                    Some(SpotBy::OnlyObserver) => 1,
                    Some(SpotBy::NearestFacingTool) => 2,
                    None => 3,
                }] += 1;
            }
            for r in &round.operator_reveals {
                if r.trigger != Trigger::Identified {
                    continue;
                }
                identified += 1;
                match r.cause {
                    Some(Cause::Spot) => causes[0] += 1,
                    Some(Cause::TeammateSpot) => causes[1] += 1,
                    Some(Cause::Ping) => causes[2] += 1,
                    Some(Cause::AbilityMarker) => causes[3] += 1,
                    None => {}
                }
            }
            for u in &round.match_feedback {
                if u.kind != MatchUpdateType::Kill {
                    continue;
                }
                kills += 1;
                spotted += usize::from(u.victim_spotted.is_some());
                named += usize::from(u.victim_spotted.as_ref().is_some_and(|k| k.by.is_some()));
                pinged += usize::from(u.victim_pinged.is_some());
                let hit = round.combat.iter().flat_map(|c| &c.hits).any(|h| {
                    let apart = h.recording_time.zip(u.recording_time);
                    h.username == u.target && apart.is_some_and(|(a, b)| (a - b).abs() <= 0.5)
                });
                placed += usize::from(hit);
            }
            tracks += round.ability_markers.len();
            by += round
                .ability_markers
                .iter()
                .filter(|t| t.by.is_some())
                .count();

            // A phone is hacked off a dead defender.
            let defense =
                (round.header.teams.iter()).position(|t| t.role == Some(TeamRole::Defense));
            hack_rounds += usize::from(!round.phone_hacks.is_empty());
            for h in &round.phone_hacks {
                hacks += 1;
                completed += usize::from(h.completed);
                dokkaebi += usize::from(operator(round, &h.username) == "Dokkaebi");
                let died = round.match_feedback.iter().any(|u| {
                    let dead = u.victim().and_then(|v| team(round, v));
                    dead.is_some()
                        && dead == defense
                        && u.recording_time
                            .zip(h.when.recording_time)
                            .is_some_and(|(d, s)| d < s)
                });
                after_death += usize::from(died);
            }
        }
    }
    let total = spots.len();
    let marks: usize = spots.iter().map(|s| s.marks).sum();
    let with = |tool: Tool| spots.iter().filter(|s| s.with == Some(tool)).count();
    let candidates = spots.iter().filter(|s| !s.by_candidates.is_empty()).count();
    println!("real rounds: {rounds} with intel, {total} spots of {marks} marks");
    println!(
        "spotter: spotAssistScore {}, onlyObserver {}, nearestFacingTool {}, nobody {} (with candidates {})",
        share(agree[0], total),
        share(agree[1], total),
        share(agree[2], total),
        share(agree[3], total),
        share(candidates, agree[3]),
    );
    println!(
        "spotted with: drone {}, camera {}",
        share(with(Tool::Drone), total),
        share(with(Tool::Camera), total)
    );
    println!(
        "spot assists: {assists}; recipient on a device at the spot: {}",
        share(on_tool, assists)
    );
    println!(
        "identified reveals: spot {}, teammateSpot {}, ping {}, abilityMarker {}",
        share(causes[0], identified),
        share(causes[1], identified),
        share(causes[2], identified),
        share(causes[3], identified),
    );
    println!(
        "kills: victim spotted {} (spotter named in {named}), victim pinged {}, a hit places the victim in {}",
        share(spotted, kills),
        share(pinged, kills),
        share(placed, kills),
    );
    println!("ability markers with their player: {}", share(by, tracks));
    println!(
        "phone hacks: {hacks} in {hack_rounds} rounds; completed {}, after a defender died {}, by Dokkaebi {}",
        share(completed, hacks),
        share(after_death, hacks),
        share(dokkaebi, hacks),
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
