//! The objective against real replays: the ten rounds of the Y11S3 test
//! match in `test_recordings/valid/Y11S3`, and (ignored unless asked for)
//! every round in the folder `R6_MATCH_REPLAY` names.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use replay_analyzer::activity::{InteractionKind, InteractionOutcome};
use replay_analyzer::combat::TimelineKind;
use replay_analyzer::objective::{
    Bomb, Carry, CarryEnd, CarryStart, Drop, Interaction, SiteSource,
};
use replay_analyzer::{MatchUpdateType, Phase, ReadMode, ReadOptions, Round, TeamRole};
use replay_analyzer::{Status, WinCondition};

fn test_file(number: usize) -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings/valid/Y11S3");
    let path = dir.join(format!("custom_{number}.rec"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The ten test rounds, read once.
fn rounds() -> &'static [Round] {
    static ROUNDS: OnceLock<Vec<Round>> = OnceLock::new();
    ROUNDS.get_or_init(|| {
        (1..=10)
            .map(|n| Round::from_bytes(&test_file(n), ReadMode::Full).unwrap())
            .collect()
    })
}

fn bomb_of(round: &Round) -> &Bomb {
    let objective = round
        .objective_state
        .as_ref()
        .expect("the round has an objective");
    objective.bomb.as_ref().expect("a Bomb round")
}

/// Test round `number` (1 to 10) with its defuser.
fn test_round(number: usize) -> (&'static Round, &'static Bomb) {
    let round = &rounds()[number - 1];
    (round, bomb_of(round))
}

fn near(a: f64, b: f64, by: f64) -> bool {
    (a - b).abs() <= by
}

/// Metres between two positions, over the ground.
fn apart(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn close(found: Option<[f32; 3]>, wanted: [f32; 3]) -> bool {
    found.is_some_and(|p| (0..3).all(|i| (p[i] - wanted[i]).abs() <= 0.0015))
}

fn lost(how: CarryEnd) -> bool {
    matches!(how, CarryEnd::Downed | CarryEnd::Died | CarryEnd::Dropped)
}

/// The number of the bomb a site's distances say is nearer.
fn nearer(distances: &[f32]) -> u8 {
    if distances[0] <= distances[1] { 1 } else { 2 }
}

/// What holds in every Bomb round, as a list of what does not.
fn broken_invariants(round: &Round, b: &Bomb) -> Vec<String> {
    let mut out = Vec::new();
    let role = |team: usize| round.header.teams.get(team).and_then(|t| t.role);
    let side = |username: &str| {
        let player = round.header.players.iter().find(|p| p.username == username);
        player.and_then(|p| role(p.team_index))
    };
    let events = round.combat.as_ref().map_or(&[][..], |c| &c.events);
    let done = |i: &&Interaction| i.outcome == InteractionOutcome::Completed;

    // Every completed plant ends a carry of the planter's, and nothing
    // else ends one as planted.
    for p in b.plants.iter().filter(done) {
        let carried = b.carrier.iter().any(|c| {
            c.username == p.username
                && c.ended == CarryEnd::Planted
                && c.end.zip(p.end).is_some_and(|(c, p)| near(c, p, 0.5))
        });
        if !carried {
            out.push(format!("no carry ends in the plant by {}", p.username));
        }
    }
    let ended = |how: CarryEnd| b.carrier.iter().filter(|c| c.ended == how).count();
    if ended(CarryEnd::Planted) != b.plants.iter().filter(done).count() {
        out.push("carries ended planted and completed plants differ".to_owned());
    }
    if b.planted_at.is_some() != round.info().planted {
        out.push(format!(
            "plantedAt is {:?} in a round with planted = {}",
            b.planted_at,
            round.info().planted
        ));
    }

    for (i, c) in b.carrier.iter().enumerate() {
        let before = i.checked_sub(1).map(|i| b.carrier[i].ended);
        // The first carrier was given the defuser; later ones took it off
        // the ground, where a carrier left it.
        let fits = match c.started {
            CarryStart::Spawn => matches!(before, None | Some(CarryEnd::Reassigned)),
            CarryStart::Pickup => before.is_some_and(lost),
        };
        if !fits {
            out.push(format!("a {:?} after {before:?}: {c:?}", c.started));
        }
        if c.end.is_none() && i + 1 != b.carrier.len() {
            out.push(format!("a carry without an end is not the last: {c:?}"));
        }
        if role(c.team) != Some(TeamRole::Attack) {
            out.push(format!("a carrier who does not attack: {c:?}"));
        }
        // A body is in the round from the end of prep on.
        if c.started == CarryStart::Pickup && c.position.is_none() {
            out.push(format!("a pickup without a position: {c:?}"));
        }
        if c.ended != CarryEnd::Reassigned && c.end_position.is_none() {
            out.push(format!("a carry that ended nowhere: {c:?}"));
        }
        let by_someone = matches!(c.ended, CarryEnd::Downed | CarryEnd::Died);
        if c.ended_by.is_some() && !by_someone {
            out.push(format!("somebody ended a carry nobody ended: {c:?}"));
        }
    }

    // A drop names the down or death behind it as the timeline has it, in
    // the half second before, and is where the defuser is.
    for d in &b.drops {
        let wanted: &[TimelineKind] = match d.reason {
            CarryEnd::Downed => &[TimelineKind::Down],
            CarryEnd::Died => &[
                TimelineKind::Kill,
                TimelineKind::TeamKill,
                TimelineKind::Death,
            ],
            _ => &[],
        };
        let fell = events.iter().any(|e| {
            wanted.contains(&e.kind)
                && e.username == d.username
                && e.by == d.by
                && (e.recording_time)
                    .is_some_and(|at| at <= d.recording_time && d.recording_time - at <= 0.5)
        });
        // A round without a timeline falls back on the kill feed.
        if !wanted.is_empty() && !fell && !events.is_empty() {
            out.push(format!("nothing in the timeline for {d:?}"));
        }
        if !lost(d.reason) || role(d.team) != Some(TeamRole::Attack) {
            out.push(format!("a drop that is none: {d:?}"));
        }
        // It falls, and can slide or roll a little.
        match (d.position, d.rest_position) {
            (Some(from), Some(to)) => {
                if apart(from, to) > 5.0 || to[2] > from[2] + 0.5 {
                    out.push(format!("a defuser that did not fall: {d:?}"));
                }
            }
            // A drop the defuser said nothing of, at its carrier.
            (Some(_), None) => {}
            _ => out.push(format!("a drop without its positions: {d:?}")),
        }
        if let Some(p) = &d.pickup {
            let lay = p.recording_time - d.recording_time;
            let same = p.by_other == (p.username != d.username)
                && lay >= 0.0
                && near(p.seconds_on_ground, lay, 0.001)
                && side(&p.username) == Some(TeamRole::Attack);
            if !same {
                out.push(format!("a pickup that does not fit its drop: {d:?}"));
            }
        }
    }
    // Only the round's last drop can stay on the ground.
    let left = b.drops.iter().rev().skip(1).filter(|d| d.pickup.is_none());
    if left.count() > 0 {
        out.push("a drop nobody picked up is not the last".to_owned());
    }
    let rising = b.drops.windows(2).all(|w| {
        let taken = w[0].pickup.as_ref().map_or(f64::MAX, |p| p.recording_time);
        taken <= w[1].recording_time
    });
    if !rising {
        out.push("a drop before the last one was picked up".to_owned());
    }

    // The round's two bombs, numbered 1 and 2 and named by the header.
    let numbers: Vec<u8> = b.sites.iter().map(|s| s.index).collect();
    if numbers != [1, 2] || b.sites.iter().any(|s| s.name.is_none()) {
        out.push(format!("the bombs in play are {:?}", b.sites));
    }

    for (i, wanted) in (b.plants.iter().map(|p| (p, TeamRole::Attack)))
        .chain(b.disables.iter().map(|d| (d, TeamRole::Defense)))
    {
        if i.side != Some(wanted) || role(i.team) != Some(wanted) {
            out.push(format!("an interaction by the wrong side: {i:?}"));
        }
        if i.position.is_none() || i.end.is_some() != i.end_position.is_some() {
            out.push(format!("an interaction without its positions: {i:?}"));
        }
        // Moving gives a plant or a disable up.
        if let (Some(from), Some(to)) = (i.position, i.end_position)
            && apart(from, to) > 1.0
        {
            out.push(format!("an interaction that moved: {i:?}"));
        }
        // The defuser is planted at the planter's feet, and disabled from
        // beside it.
        let reach = if wanted == TeamRole::Attack { 1.0 } else { 2.5 };
        match (i.defuser_position, i.position) {
            (Some(defuser), Some(player)) if apart(defuser, player) <= reach => {}
            _ => out.push(format!("no defuser where the player is: {i:?}")),
        }
        if i.site.is_none() && b.sites.len() == 2 {
            out.push(format!("an interaction at no bomb: {i:?}"));
        }
        if let Some(s) = &i.site {
            let decoded = s.source == SiteSource::Decoded;
            let may = wanted == TeamRole::Defense || i.outcome == InteractionOutcome::Completed;
            if s.distances.len() != b.sites.len() || (decoded && !may) {
                out.push(format!("a bomb told wrongly: {i:?}"));
            }
        }
        // The seconds still to go of the seven it takes.
        let given_up = i.outcome != InteractionOutcome::Completed;
        if i.remaining.is_some() != given_up
            || i.remaining.is_some_and(|r| !(0.0..=7.0).contains(&r))
        {
            out.push(format!("seconds to go that do not fit: {i:?}"));
        }
        if let (Some(left), Some(end)) = (i.remaining, i.end)
            && !near(7.0 - left, end - i.start, 0.25)
        {
            out.push(format!("{left} s to go after {} s: {i:?}", end - i.start));
        }
        let after = wanted == TeamRole::Defense;
        if i.defuser_time_left.is_some() != after
            || i.defuser_time_left_at_end.is_some() != (after && i.end.is_some())
        {
            out.push(format!("timer left on the wrong interaction: {i:?}"));
        }
        if after && i.phase != Phase::Planted {
            out.push(format!("a disable outside the planted phase: {i:?}"));
        }
        // The clock after the plant is what is left, in whole seconds
        // rounded down; an event and the clock's turn can share a frame.
        if let Some(left) = i.defuser_time_left {
            let shown = i
                .time
                .split_once(':')
                .and_then(|(m, s)| Some(m.parse::<f64>().ok()? * 60.0 + s.parse::<f64>().ok()?));
            if !shown.is_some_and(|s| left - 1.1 <= s && s <= left + 0.1) {
                out.push(format!("{left} s left at {}: {i:?}", i.time));
            }
        }
    }
    // The feed's own plant and disable starts, at the same clock.
    for u in &round.match_feedback {
        let list = match u.kind {
            MatchUpdateType::DefuserPlantStart => &b.plants,
            MatchUpdateType::DefuserDisableStart => &b.disables,
            _ => continue,
        };
        let found = list.iter().any(|i| {
            i.username == u.username
                && u.recording_time.is_some_and(|at| near(at, i.start, 0.05))
                && near(i.elapsed, u.elapsed, 1.0)
                && i.phase == u.phase
        });
        if !found {
            out.push(format!(
                "nothing for the feed's {} by {} at {} ({:?})",
                u.kind.name(),
                u.username,
                u.time,
                u.recording_time
            ));
        }
    }

    // The timer: 45 seconds whenever the clock showed it, read from the
    // file, and never more left than that.
    let timer = b.defuser_timer.map(f64::from);
    if b.planted_at.is_some() && (timer != Some(45.0) || !b.timer_decoded) {
        out.push(format!(
            "a defuser timer of {timer:?}, decoded: {}",
            b.timer_decoded
        ));
    }
    if b.planted_at.is_none() && (timer.is_some() || b.defuser_time_left.is_some()) {
        out.push("a defuser timer without a plant".to_owned());
    }
    if let (Some(left), Some(timer)) = (b.defuser_time_left, timer) {
        let info = round.info();
        let disabled = (b.disables.iter().filter(done)).find_map(|d| d.defuser_time_left_at_end);
        let fits = match info.end_reason {
            // The timer stops with the disable.
            Some(WinCondition::DisabledDefuser) => disabled == Some(left),
            Some(WinCondition::DefusedBomb) => left <= 0.2,
            _ => left > 0.0,
        };
        if !fits || left > timer {
            out.push(format!(
                "{left} s left as the round ended {:?}",
                info.end_reason
            ));
        }
    }
    if round.info().planted && b.defuser_time_left.is_none() {
        out.push("a planted round with no time left at its end".to_owned());
    }
    out
}

/// The carry a drop ended: the one of the player's that had started and
/// ends nearest it, within half a second, or goes on to the recording's
/// end.
fn carry_of<'a>(b: &'a Bomb, d: &Drop) -> Option<&'a Carry> {
    let off = |c: &Carry| c.end.map_or(0.5, |end| (end - d.recording_time).abs());
    let mine =
        |c: &&Carry| c.username == d.username && c.start <= d.recording_time + 0.5 && off(c) <= 0.5;
    let carries = b.carrier.iter().filter(mine);
    carries.min_by(|a, b| off(a).total_cmp(&off(b)))
}

/// How the defuser and the carriers' `HasDefuser` agree, as a list of
/// where they do not: every drop within a carry of the player's, every
/// pickup as a carry of the player's starts, and every carry that lost
/// the defuser with a drop of its own. `after` gets the seconds from the
/// defuser's word to the carry's. The two streams keep time a few
/// milliseconds apart in a player's own recording, so either can be first.
fn disagreements(b: &Bomb, after: &mut Vec<f64>) -> Vec<String> {
    let mut out = Vec::new();
    for d in &b.drops {
        match carry_of(b, d) {
            Some(c) => after.extend(c.end.map(|end| end - d.recording_time)),
            None if d.rest_position.is_none() => {}
            None => out.push(format!("no carry around the drop {d:?}")),
        }
        let Some(p) = &d.pickup else { continue };
        let start = (b.carrier.iter())
            .filter(|c| c.username == p.username)
            .map(|c| c.start - p.recording_time)
            .find(|d| d.abs() <= 0.5);
        match start {
            Some(d) => after.push(d),
            None => out.push(format!("no carry starts at the pickup of {d:?}")),
        }
    }
    for (i, c) in b.carrier.iter().enumerate().filter(|(_, c)| lost(c.ended)) {
        let mine = |d: &&Drop| carry_of(b, d).is_some_and(|of| std::ptr::eq(of, c));
        if b.drops.iter().filter(mine).count() != 1 {
            out.push(format!("not one drop for carry {i}: {c:?}"));
        }
    }
    // A pickup starts a carry, and nothing else does after the first.
    let taken = b.drops.iter().filter(|d| d.pickup.is_some()).count();
    let pickups = b.carrier.iter().filter(|c| c.started == CarryStart::Pickup);
    if taken != pickups.count() {
        out.push(format!("{taken} pickups of the defuser: {:?}", b.carrier));
    }
    out
}

#[test]
fn the_test_rounds_keep_every_invariant() {
    for (n, round) in rounds().iter().enumerate() {
        let b = bomb_of(round);
        let broken = broken_invariants(round, b);
        assert!(broken.is_empty(), "round {}: {broken:#?}", n + 1);
        assert!(b.drops_decoded, "round {}", n + 1);
        let apart = disagreements(b, &mut Vec::new());
        assert!(apart.is_empty(), "round {}: {apart:#?}", n + 1);
    }
}

#[test]
fn every_test_round_is_a_bomb_round_with_its_objective_reported() {
    for (n, round) in rounds().iter().enumerate() {
        let n = n + 1;
        let objective = round.objective_state.as_ref().unwrap();
        assert_eq!(objective.mode.name(), Some("Bomb"), "round {n}");
        let b = objective.bomb.as_ref().unwrap();
        let field = |name: &str| {
            let f = round.decode.get(name);
            f.unwrap_or_else(|| panic!("round {n}: no {name} in the report"))
        };
        let f = field("objectiveState");
        assert_eq!((f.status, f.count), (Status::Inferred, b.carrier.len()));
        assert!(f.warnings.is_empty(), "round {n}: {:?}", f.warnings);
        let f = field("defuserDrops");
        assert_eq!((f.status, f.count), (Status::Decoded, b.drops.len()));
        let f = field("bombSites");
        assert_eq!((f.status, f.count), (Status::Decoded, 2), "round {n}");
        let f = field("objectivePositions");
        let interactions = b.plants.len() + b.disables.len();
        assert_eq!((f.status, f.count), (Status::Decoded, interactions));
        // A spectator's recording names the bomb of every plant.
        for name in ["defuserTimer", "plantSite"] {
            let f = round.decode.get(name).map(|f| (f.status, f.count));
            let wanted = b.planted_at.map(|_| (Status::Decoded, 1));
            assert_eq!(f, wanted, "round {n}: {name}");
        }
    }
}

#[test]
fn how_each_carry_of_the_test_rounds_started_and_ended() {
    use CarryEnd::*;
    use CarryStart::*;
    let wanted: [&[(CarryStart, CarryEnd)]; 10] = [
        &[(Spawn, Dropped), (Pickup, Died)],
        &[(Spawn, Dropped), (Pickup, Planted)],
        &[(Spawn, Planted)],
        &[(Spawn, Dropped), (Pickup, Planted)],
        &[(Spawn, RoundEnd)],
        &[
            (Spawn, Dropped),
            (Pickup, Dropped),
            (Pickup, Died),
            (Pickup, RoundEnd),
        ],
        &[(Spawn, Downed), (Pickup, Died), (Pickup, Planted)],
        &[(Spawn, RoundEnd)],
        &[(Spawn, Died), (Pickup, Died)],
        &[(Spawn, Died)],
    ];
    for (n, wanted) in wanted.iter().enumerate() {
        let (_, b) = test_round(n + 1);
        let found: Vec<_> = b.carrier.iter().map(|c| (c.started, c.ended)).collect();
        assert_eq!(found, *wanted, "round {}", n + 1);
    }
}

#[test]
fn the_defuser_of_round_2_is_put_down_upstairs_and_planted_a_floor_below() {
    let (_, b) = test_round(2);
    let [drop] = &b.drops[..] else {
        panic!("{:?}", b.drops)
    };
    // Let go at the hands, 0.9 m above where it comes to lie.
    assert_eq!(
        (drop.username.as_str(), drop.reason, drop.by.as_deref()),
        ("vitaking.FaZe", CarryEnd::Dropped, None)
    );
    assert!(near(drop.recording_time, 138.184, 0.0005), "{drop:?}");
    assert!(close(drop.position, [-45.826, 9.609, 9.298]));
    assert!(close(drop.rest_position, [-45.826, 9.609, 8.394]));
    let pickup = drop.pickup.as_ref().unwrap();
    assert_eq!(
        (pickup.username.as_str(), pickup.by_other),
        ("soulz1.FaZe", true)
    );
    assert!(near(pickup.recording_time, 143.428, 0.0005), "{pickup:?}");
    assert!(near(pickup.seconds_on_ground, 5.244, 0.0005), "{pickup:?}");

    let [plant] = &b.plants[..] else {
        panic!("{:?}", b.plants)
    };
    assert!(near(plant.start, 188.309, 0.0005));
    assert!(close(plant.defuser_position, [-53.53, 10.254, 4.0]));
    let site = plant.site.as_ref().unwrap();
    assert_eq!(
        (site.index, site.name.as_deref(), site.source),
        (1, Some("2F Executive Lounge"), SiteSource::Decoded)
    );
    assert_eq!(site.distances, [3.54, 14.59]);
    // The disable is of the same defuser, with the timer as the game
    // wrote it at its first frame and its last.
    let [disable] = &b.disables[..] else {
        panic!("{:?}", b.disables)
    };
    assert_eq!(disable.defuser_position, plant.defuser_position);
    assert_eq!(disable.site, plant.site);
    assert_eq!(disable.defuser_time_left, Some(40.486));
    assert_eq!(disable.defuser_time_left_at_end, Some(33.484));
    assert_eq!(b.defuser_time_left, Some(33.484));
}

#[test]
fn the_defuser_of_round_7_changes_hands_twice_before_the_plant() {
    let (_, b) = test_round(7);
    // The first carrier is downed three seconds into a plant, a teammate
    // who picks the defuser up dies within four frames, and a third
    // plants it where the first had tried.
    let [downed, died] = &b.drops[..] else {
        panic!("{:?}", b.drops)
    };
    assert_eq!(
        (
            downed.username.as_str(),
            downed.reason,
            downed.by.as_deref()
        ),
        ("Bassetto.L5", CarryEnd::Downed, Some("vitaking.FaZe"))
    );
    assert_eq!(
        (downed.time.as_str(), downed.phase),
        ("0:06", Phase::Action)
    );
    // In the frame of the down; `HasDefuser` says so two frames later.
    assert!(near(downed.recording_time, 218.442, 0.0005), "{downed:?}");
    assert_eq!(b.carrier[0].end, Some(218.509));
    assert!(close(downed.position, [-49.762, 12.512, -3.374]));
    assert!(close(downed.rest_position, [-49.762, 12.512, -3.804]));
    let pickup = downed.pickup.as_ref().unwrap();
    assert_eq!(
        (pickup.username.as_str(), pickup.by_other),
        ("PSYCHO.L5", true)
    );
    assert!(near(pickup.seconds_on_ground, 1.699, 0.0005), "{pickup:?}");

    assert_eq!(
        (died.username.as_str(), died.reason, died.by.as_deref()),
        ("PSYCHO.L5", CarryEnd::Died, Some("vitaking.FaZe"))
    );
    assert!(near(died.recording_time, 220.243, 0.0005), "{died:?}");
    let pickup = died.pickup.as_ref().unwrap();
    assert_eq!(pickup.username, "WIZARD.L5");
    assert!(near(pickup.seconds_on_ground, 3.097, 0.0005), "{pickup:?}");

    let outcomes: Vec<_> = (b.plants.iter())
        .map(|p| (p.username.as_str(), p.outcome, p.time.as_str(), p.remaining))
        .collect();
    assert_eq!(
        outcomes,
        [
            (
                "Bassetto.L5",
                InteractionOutcome::Aborted,
                "0:09",
                Some(3.908)
            ),
            ("WIZARD.L5", InteractionOutcome::Completed, "0:01", None),
        ]
    );
    // Both plants put the defuser within half a metre of each other, at
    // the same bomb: the nearer one for the plant given up, the one the
    // game names for the other.
    let (first, second) = (&b.plants[0], &b.plants[1]);
    assert!(close(first.defuser_position, [-49.752, 12.513, -3.801]));
    assert!(close(second.defuser_position, [-49.321, 12.286, -3.801]));
    let sites: Vec<_> = [first, second]
        .iter()
        .map(|p| p.site.as_ref().unwrap())
        .map(|s| (s.index, s.name.as_deref(), s.source))
        .collect();
    assert_eq!(
        sites,
        [
            (1, Some("B Lockers"), SiteSource::Nearest),
            (1, Some("B Lockers"), SiteSource::Decoded)
        ]
    );
    assert_eq!(second.site.as_ref().unwrap().distances, [6.44, 19.26]);
}

#[test]
fn the_disable_of_round_7_ends_with_under_five_seconds_left() {
    let (round, b) = test_round(7);
    assert!(near(b.planted_at.unwrap(), 230.873, 0.0005));
    assert_eq!(b.defuser_timer, Some(45));
    let [disable] = &b.disables[..] else {
        panic!("{:?}", b.disables)
    };
    assert_eq!(disable.username, "Handyy.FaZe");
    assert_eq!((disable.team, disable.side), (1, Some(TeamRole::Defense)));
    assert_eq!(
        (disable.time.as_str(), disable.phase, disable.elapsed),
        ("0:11", Phase::Planted, 264.0)
    );
    assert_eq!(disable.defuser_time_left, Some(11.877));
    assert_eq!(disable.defuser_time_left_at_end, Some(4.856));
    assert_eq!(b.defuser_time_left, Some(4.856));
    // The defender stands still, a fifth of a metre from the defuser.
    assert!(close(disable.position, [-49.521, 12.199, -3.801]));
    assert_eq!(disable.position, disable.end_position);
    let defuser = disable.defuser_position.unwrap();
    assert!(apart(disable.position.unwrap(), defuser) < 0.25);
    assert_eq!(round.info().end_reason, Some(WinCondition::DisabledDefuser));
}

#[test]
fn the_planted_test_rounds_have_a_timer_and_the_others_none() {
    let left: Vec<_> = (1..=10)
        .map(|n| test_round(n).1)
        .map(|b| (b.defuser_timer, b.defuser_time_left))
        .collect();
    let planted = |left| (Some(45), Some(left));
    let wanted = [
        (None, None),
        planted(33.484),
        planted(43.168),
        planted(22.48),
        (None, None),
        (None, None),
        planted(4.856),
        (None, None),
        (None, None),
        (None, None),
    ];
    assert_eq!(left, wanted);
}

#[test]
fn the_bombs_in_play_are_those_of_the_site_the_header_names() {
    // Three sites of Bank over the ten rounds, each with its two bombs.
    let basement = [[-54.453, 8.401, -3.8], [-68.022, 7.681, -3.8]];
    let upstairs = [[-55.402, 13.258, 4.0], [-67.698, 13.739, 4.002]];
    let ground = [[-59.9, -0.4, 0.0], [-43.599, -0.091, 0.001]];
    for n in 1..=10 {
        let (round, b) = test_round(n);
        let wanted = match round.header.site.as_str() {
            "B Lockers, B CCTV Room" => basement,
            "2F Executive Lounge, 2F CEO Office" => upstairs,
            "1F Staff Room, 1F Open Area" => ground,
            other => panic!("round {n}: {other}"),
        };
        for (site, wanted) in b.sites.iter().zip(wanted) {
            assert!(close(Some(site.position), wanted), "round {n}: {site:?}");
        }
        let names: Vec<_> = b.sites.iter().filter_map(|s| s.name.as_deref()).collect();
        assert_eq!(names.join(", "), round.header.site, "round {n}");
        // Every plant of the match, completed or not, is at bomb 1, and
        // the game's word agrees with the nearer bomb where it has one.
        for p in b.plants.iter().chain(&b.disables) {
            let site = p.site.as_ref().unwrap();
            assert_eq!((site.index, nearer(&site.distances)), (1, 1), "{n}: {p:?}");
        }
    }
}

#[test]
fn a_defuser_handed_over_lies_one_frame() {
    let (_, b) = test_round(6);
    let handed = &b.drops[1];
    let pickup = handed.pickup.as_ref().unwrap();
    assert_eq!(
        (handed.reason, pickup.seconds_on_ground, pickup.by_other),
        (CarryEnd::Dropped, 0.034, true)
    );
    assert_eq!(handed.position, handed.rest_position);
    // The one a kill left on the stairs falls to the floor below and is
    // taken back by the round's first carrier, who is still planting when
    // the round ends.
    let fallen = &b.drops[2];
    assert!(close(fallen.position, [-55.918, 6.18, 0.004]));
    assert!(close(fallen.rest_position, [-55.918, 6.18, -3.804]));
    let last = fallen.pickup.as_ref().unwrap();
    assert_eq!(last.username, b.carrier[0].username);
    assert!(near(last.seconds_on_ground, 2.483, 0.0005), "{last:?}");
    let plant = b.plants.last().unwrap();
    assert_eq!(
        (plant.outcome, plant.end, plant.end_position),
        (InteractionOutcome::Unfinished, None, None)
    );
    assert_eq!(plant.remaining, Some(3.492));
    assert!(close(plant.defuser_position, [-53.359, 10.008, -3.801]));
}

#[test]
fn a_defuser_lies_where_its_carrier_died_until_it_is_picked_up() {
    // Round 9: 44 seconds on the ground.
    let (_, b) = test_round(9);
    let drop = &b.drops[0];
    let pickup = drop.pickup.as_ref().unwrap();
    assert!(near(pickup.seconds_on_ground, 44.349, 0.0005), "{pickup:?}");
    assert!(close(drop.position, [-75.132, 12.936, 4.477]));
    assert!(close(drop.rest_position, [-75.201, 12.991, 3.997]));
    // The second carrier's is never picked up.
    assert_eq!(b.drops[1].pickup, None);
    // A kill that decides the round drops it too, a frame before
    // `HasDefuser` ends with the round; the game taking it back in the
    // next record is no pickup.
    for n in [1, 10] {
        let (_, b) = test_round(n);
        let last = b.drops.last().unwrap();
        assert_eq!((last.reason, &last.pickup), (CarryEnd::Died, &None));
        let carry = b.carrier.last().unwrap();
        assert_eq!(carry.ended, CarryEnd::Died, "round {n}");
        assert!(carry.end.unwrap() - last.recording_time < 0.04);
    }
    // Every defuser of the test match is let go within half a metre of
    // where its carrier is two frames later.
    for n in 1..=10 {
        let (_, b) = test_round(n);
        for d in &b.drops {
            let c = carry_of(b, d).unwrap();
            let off = apart(d.position.unwrap(), c.end_position.unwrap());
            assert!(off < 0.5, "round {n}: {off} m from {d:?}");
        }
    }
}

#[test]
fn the_objective_is_the_same_with_the_movement_read() {
    for n in [6, 7] {
        let options = ReadOptions {
            movement: true,
            ..ReadMode::Full.into()
        };
        let with = Round::from_bytes(&test_file(n), options).unwrap();
        let movement = with.movement.as_ref().unwrap();
        assert_eq!(with.objective_state, rounds()[n - 1].objective_state, "round {n}");
        // The defuser is not something its carrier placed.
        let defuser = |p: &&replay_analyzer::movement::Placement| p.position == [0.0; 3];
        assert_eq!(movement.placements.iter().filter(defuser).count(), 0);
    }
}

#[test]
fn the_objective_is_in_the_json_of_a_full_read_only() {
    let (round, _) = test_round(7);
    let json = serde_json::to_value(round).unwrap();
    let bomb = &json["objectiveState"]["bomb"];
    assert_eq!(json["objectiveState"]["mode"]["name"], "Bomb");
    assert_eq!(bomb["sites"][1]["name"], "B CCTV Room");
    assert_eq!(bomb["carrier"][0]["ended"], "downed");
    assert_eq!(bomb["carrier"][0]["started"], "spawn");
    // An attacker has no body in prep: null, not left out.
    assert!(bomb["carrier"][0]["position"].is_null());
    assert_eq!(bomb["drops"][0]["pickup"]["secondsOnGround"], 1.699);
    assert_eq!(bomb["plants"][0]["remaining"], 3.908);
    assert_eq!(bomb["plants"][1]["site"]["source"], "decoded");
    assert_eq!(bomb["disables"][0]["defuserTimeLeftAtEnd"], 4.856);
    assert_eq!(bomb["defuserTimer"], 45);
    assert_eq!(json["activity"]["interactions"][0]["remaining"], 3.908);
    let partial = Round::from_bytes(&test_file(7), ReadMode::Partial).unwrap();
    assert!(partial.objective_state.is_none());
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
/// `cargo test --release --test objective -- --ignored --nocapture`, which
/// also prints what was counted. The folder is only read.
#[test]
#[ignore = "needs R6_MATCH_REPLAY"]
fn players_own_recordings_keep_every_invariant() {
    let dir = std::env::var_os("R6_MATCH_REPLAY").expect("R6_MATCH_REPLAY names a folder");
    let mut files = Vec::new();
    recordings(Path::new(&dir), &mut files);
    assert!(!files.is_empty(), "no .rec files in {dir:?}");
    let (mut failures, mut apart_list) = (Vec::new(), Vec::new());
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut count = |what: String| *counts.entry(what).or_default() += 1;
    // Seconds from the defuser's word to `HasDefuser`'s, metres from a
    // drop to where it lies and to its carrier, seconds from a down or
    // death to its drop, and seconds between the timer and the frames.
    let mut after = Vec::new();
    let (mut slid, mut off): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
    let (mut fell, mut drift): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
    for path in files {
        let raw = std::fs::read(&path).unwrap();
        let Ok(round) = Round::from_bytes(&raw, ReadMode::Full) else {
            continue;
        };
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let Some(objective) = &round.objective_state else {
            count("rounds without an objective".to_owned());
            continue;
        };
        count(format!("rounds of {}", objective.mode));
        let Some(b) = &objective.bomb else { continue };
        // A recording that stops in prep has no bombs in play yet.
        if b.sites.is_empty() && round.info().winner.is_none() {
            count("rounds that stop before the bombs are in play".to_owned());
            continue;
        }
        let broken = broken_invariants(&round, b);
        if !broken.is_empty() {
            failures.push(format!("{name}: {broken:#?}"));
        }
        count(format!(
            "rounds with the defuser decoded = {}",
            b.drops_decoded
        ));
        let apart_here = disagreements(b, &mut after);
        count(format!(
            "rounds where the defuser and HasDefuser agree = {}",
            apart_here.is_empty()
        ));
        apart_list.extend(apart_here.into_iter().map(|a| format!("{name}: {a}")));
        for c in &b.carrier {
            count(format!("carries {:?} -> {:?}", c.started, c.ended));
            if lost(c.ended) && c.end.is_none() {
                count("carries whose drop HasDefuser never said".to_owned());
            }
        }
        let events = round.combat.as_ref().map_or(&[][..], |c| &c.events);
        for d in &b.drops {
            let c = carry_of(b, d);
            count(format!("drops {:?}", d.reason));
            if d.rest_position.is_none() {
                count("drops the defuser said nothing of".to_owned());
            }
            match &d.pickup {
                Some(p) => count(format!("pickups by other = {}", p.by_other)),
                None => count("drops never picked up".to_owned()),
            }
            if let (Some(from), Some(to)) = (d.position, d.rest_position) {
                slid.push(apart(from, to));
            }
            if let (Some(from), Some(body)) = (d.position, c.and_then(|c| c.end_position)) {
                off.push(apart(from, body));
            }
            let down = d.reason == CarryEnd::Downed;
            let at = events
                .iter()
                .filter(|e| e.username == d.username && d.reason != CarryEnd::Dropped)
                .filter(|e| (e.kind == TimelineKind::Down) == down)
                .filter_map(|e| Some(d.recording_time - e.recording_time?))
                .find(|d| (0.0..=0.5).contains(d));
            fell.extend(at);
        }
        for i in b.plants.iter().chain(&b.disables) {
            let kind = match i.side {
                Some(TeamRole::Defense) => InteractionKind::Disable,
                _ => InteractionKind::Plant,
            };
            count(format!("{kind:?} {:?}", i.outcome));
            if let (Some(left), Some(at)) = (i.defuser_time_left, b.planted_at) {
                drift.push(left - (45.0 - (i.start - at)));
            }
        }
        // The game's word for the plant's bomb against the nearer one.
        let done = |p: &&Interaction| p.outcome == InteractionOutcome::Completed;
        if let Some(site) = b.plants.iter().find(done).and_then(|p| p.site.as_ref()) {
            count(match site.source {
                SiteSource::Decoded => format!(
                    "plants named by the game, nearer bomb agrees = {}",
                    site.index == nearer(&site.distances)
                ),
                SiteSource::Nearest => "plants at the nearer bomb, unnamed".to_owned(),
            });
            if site.index != nearer(&site.distances) {
                eprintln!("{name}: the game names bomb {}: {site:?}", site.index);
            }
            if (site.distances[0] - site.distances[1]).abs() < 3.0 {
                count("plants within 3 m of being as near the other bomb".to_owned());
            }
        }
    }
    let span = |list: &mut Vec<f64>| {
        list.sort_by(f64::total_cmp);
        format!(
            "{} from {:?} to {:?}",
            list.len(),
            list.first(),
            list.last()
        )
    };
    let span32 = |list: &mut Vec<f32>| {
        list.sort_by(f32::total_cmp);
        let median = list.get(list.len() / 2);
        format!("{} median {median:?} most {:?}", list.len(), list.last())
    };
    eprintln!("{counts:#?}");
    eprintln!("defuser to HasDefuser, seconds: {}", span(&mut after));
    eprintln!("down or death to drop, seconds: {}", span(&mut fell));
    eprintln!(
        "timer against frames at a disable's start, seconds: {}",
        span(&mut drift)
    );
    eprintln!("drop to where it lies, metres: {}", span32(&mut slid));
    eprintln!("drop to the carrier's body, metres: {}", span32(&mut off));
    eprintln!("defuser and HasDefuser apart:\n{}", apart_list.join("\n"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
