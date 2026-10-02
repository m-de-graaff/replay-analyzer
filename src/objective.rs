//! What the round's objective did (Y11S3): for a Bomb round, who carried
//! the defuser and how each carry started and ended, where it was dropped
//! and came to rest, who picked it up, and every plant and disable with
//! where it happened, on which bomb, and what the defuser timer had left.
//!
//! It joins what other modules decode:
//!
//! ```text
//! activity   carries (`HasDefuser`), plants and disables with their outcome,
//!            the defuser timer and the bomb the defuser is on
//! movement   the defuser itself and the map's bombs (see
//!            `movement::DefuserUpdate`), and where each player's body was
//! combat     the round's timeline: kills, downs
//! timeline   the round clock
//! ```
//!
//! The defuser is an object of the movement stream. It sits at (0, 0, 0)
//! while a player carries it and says so itself when that changes:
//!
//! ```text
//! given      an update naming a player and no position: the carrier when
//!            action starts
//! dropped    the state bit 08 turns on, with the position it was let go
//!            at; position-only updates follow as it falls, and the last
//!            one is where it lies
//! picked up  back to (0, 0, 0) and not shown; the update names the player
//!            unless it is the one who dropped it
//! plant      the position and rotation it is planted at come as the plant
//!            starts; it goes to (0, 0, -100) when the plant is given up,
//!            and the state bit 02 turns on when it completes
//! ```
//!
//! When the round is decided the game takes a lying defuser back in the
//! record that says so on the bombs; that is no pickup.
//!
//! Why the defuser was dropped is not stated. `downed` and `died` are the
//! round's timeline having the carrier go down or die in the half second
//! up to the drop (0 to 0.07 s before, in the 166 such drops checked); a
//! drop with neither is the carrier putting it down. A carry is a stretch of the
//! carrier's `HasDefuser`, which follows the object a few frames behind,
//! and ends for the first reason that fits:
//!
//! ```text
//! planted     a completed plant by the carrier ended within 0.5 s of it
//! downed, died, dropped
//!             the defuser was dropped by the carrier during it, up to
//!             0.5 s either side; `HasDefuser` can outlast the drop when
//!             the kill decides the round
//! roundEnd    the carrier still had it when the round was decided, or when
//!             the recording ended
//! downed, died
//!             the carrier fell in the 0.5 s before it and the defuser never
//!             lay anywhere: a teammate at hand had it in the same frame
//! reassigned  none of these: the game gave the defuser to someone else,
//!             as it does before action starts, when nobody has a body to
//!             drop it
//! ```
//!
//! A carry lost with no drop of the defuser's own still gets one, at the
//! carrier's body and with the next carrier as its pickup. That is every
//! drop of a recording whose movement stream has no defuser; there a
//! carry that ends for no other reason is `dropped`.
//!
//! What the defuser timer has left is `TimerInMilliseconds` as the game
//! wrote it, taken between the two samples around a moment, or the last
//! one when the timer has stopped. Its length is not a value of its own:
//! after a plant the round clock shows what is left in whole seconds,
//! rounded down, and every reading says 45.
//!
//! Each site has two bombs, numbered 1 and 2. A completed plant's bomb is
//! the number the game-mode object gives when the recording has it, and
//! otherwise the nearer of the two. Which number is A and which B, and
//! which of the header's two site names goes with which, is assumed: 1
//! first. Other game modes give `mode` and nothing else: no recording of
//! one has been seen, so none is decoded.

use serde::Serialize;

use crate::activity::{Activity, DefuserInteraction, InteractionKind, InteractionOutcome};
use crate::combat::{TimelineEvent, TimelineKind};
use crate::details::{LifeEvent, LifeEventType, Phase};
use crate::feedback::{MatchUpdate, MatchUpdateType, display_clock};
use crate::header::Header;
use crate::movement::{Positions, Stream};
use crate::timeline::Timeline;
use crate::types::{GameMode, TeamRole};

/// How far apart, in seconds, a carry's end and the plant or drop that
/// ended it can be written, and a drop and the down or death behind it.
/// A carry ends up to 0.14 s after its drop, and in a player's own
/// recording up to 0.005 s before it; a carrier who put the defuser down
/// and was shot after is 1.5 s apart at the closest.
const JOIN_WINDOW: f64 = 0.5;
/// A plant and the defuser's move to where it is planted are written in
/// the same frame.
const SAME_FRAME: f64 = 0.1;

/// State bit of the defuser: lying in the world.
const LYING: u8 = 0x08;
/// Where the defuser is while carried, and where a plant given up puts it.
const CARRIED: [f32; 3] = [0.0, 0.0, 0.0];
const PUT_AWAY: [f32; 3] = [0.0, 0.0, -100.0];

/// How a carry started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CarryStart {
    /// The game gave it to the player: before action started, or from a
    /// carrier it took it from.
    Spawn,
    /// The player picked it up off the ground.
    Pickup,
}

/// How a carry ended (see the module's notes for how each is told).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CarryEnd {
    Planted,
    Downed,
    Died,
    /// Still carrying when the round was decided or the recording ended.
    RoundEnd,
    /// Given to another player by the game, without lying anywhere.
    Reassigned,
    /// Put down by the carrier.
    Dropped,
}

/// One stretch a player carried the defuser.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Carry {
    pub username: String,
    /// Index into the header's teams.
    pub team: usize,
    /// Seconds since the recording started.
    pub start: f64,
    /// Absent while the player still carried it when the recording ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
    pub started: CarryStart,
    pub ended: CarryEnd,
    /// Who downed or killed the carrier, for `downed` and `died`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_by: Option<String>,
    /// Where the carrier was when the carry started: null before their
    /// body is in the round, and when the movement stream has no body for
    /// them.
    pub position: Option<[f32; 3]>,
    /// Where the carrier was when it ended, or when the recording did.
    pub end_position: Option<[f32; 3]>,
}

/// The defuser picked up again after a [`Drop`].
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pickup {
    pub username: String,
    /// Round clock, phase and seconds since prep started.
    pub time: String,
    pub phase: Phase,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
    /// Seconds since the recording started.
    pub recording_time: f64,
    /// Seconds the defuser was out of a player's hands.
    pub seconds_on_ground: f64,
    /// Whether another player than the one who lost it picked it up.
    pub by_other: bool,
}

/// The defuser leaving its carrier without being planted.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drop {
    /// The carrier who lost it.
    pub username: String,
    pub team: usize,
    /// `downed`, `died` or `dropped`.
    pub reason: CarryEnd,
    /// Who downed or killed the carrier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    pub time: String,
    pub phase: Phase,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
    /// Seconds since the recording started.
    pub recording_time: f64,
    /// Where the defuser was let go: at the carrier's hands. The carrier's
    /// feet when the defuser itself said nothing of the drop.
    pub position: Option<[f32; 3]>,
    /// Where it came to lie; null when the defuser said nothing of the
    /// drop.
    pub rest_position: Option<[f32; 3]>,
    /// Null when nobody picked it up again.
    pub pickup: Option<Pickup>,
}

/// One of the round's two bombs.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Site {
    /// The game's number for the bomb, 1 or 2.
    pub index: u8,
    /// The bomb's object, in hex: the same in every round on the map.
    pub object: String,
    /// The header's name for it, taking its two names in order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub position: [f32; 3],
}

/// How a plant's bomb was told.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SiteSource {
    /// The game-mode object names the bomb the defuser is on.
    Decoded,
    /// The bomb nearer the defuser.
    Nearest,
}

/// The bomb a plant or disable is at.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlantSite {
    pub index: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub source: SiteSource,
    /// Metres over the ground from the defuser to each bomb of `sites`, in
    /// their order.
    pub distances: Vec<f32>,
}

/// A plant or a disable, from start to finish.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Interaction {
    pub username: String,
    pub team: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<TeamRole>,
    /// Seconds since the recording started.
    pub start: f64,
    /// Absent for an unfinished one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
    pub outcome: InteractionOutcome,
    /// Seconds of the 7 it takes still to go when it was given up or cut
    /// short.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
    /// Round clock, phase and seconds since prep started, at the start.
    pub time: String,
    pub phase: Phase,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
    /// Where the player was at the start.
    pub position: Option<[f32; 3]>,
    /// Where the player was at the end; null for an unfinished one.
    pub end_position: Option<[f32; 3]>,
    /// Where the defuser is planted: for a plant where this one put it,
    /// completed or not, for a disable where the round's plant did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defuser_position: Option<[f32; 3]>,
    /// The bomb it is at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<PlantSite>,
    /// Seconds left on the defuser timer at the start and at the end, for
    /// what happened after the plant: every disable, and a plant never.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defuser_time_left: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defuser_time_left_at_end: Option<f64>,
}

/// The defuser over a Bomb round.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bomb {
    /// The round's two bombs, in the order of their numbers.
    pub sites: Vec<Site>,
    pub carrier: Vec<Carry>,
    pub drops: Vec<Drop>,
    pub plants: Vec<Interaction>,
    pub disables: Vec<Interaction>,
    /// Seconds since the recording started when the plant completed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planted_at: Option<f64>,
    /// Length of the defuser timer in whole seconds, from the round clock
    /// after the plant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defuser_timer: Option<u32>,
    /// Seconds left on the defuser timer when the round was decided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defuser_time_left: Option<f64>,
    /// Whether the drops come from the defuser itself, and the time left
    /// from the timer: what the decode report says of them.
    #[serde(skip)]
    pub drops_decoded: bool,
    #[serde(skip)]
    pub timer_decoded: bool,
    /// Joins that did not work out.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

/// What the round's objective did. Each game mode has a section of its
/// own; only Bomb's exists.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Objective {
    pub mode: GameMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bomb: Option<Bomb>,
}

/// Whether `mode` is played with a defuser.
pub fn is_bomb(mode: GameMode) -> bool {
    matches!(mode.name(), Some("Bomb" | "QuickMatchBomb"))
}

/// What a round's other sections say, for [`derive`].
pub(crate) struct Input<'a> {
    pub header: &'a Header,
    pub activity: &'a Activity,
    /// The round's timeline of kills and downs; empty in a round without.
    pub events: &'a [TimelineEvent],
    /// The kill feed and the HUD's downs, for a round without a timeline.
    pub feed: &'a [MatchUpdate],
    pub life_events: &'a [LifeEvent],
    pub timeline: &'a Timeline,
    /// The movement stream: the defuser, the bombs and the bodies.
    pub stream: &'a Stream,
    pub positions: &'a Positions,
    /// Seconds since the recording started, per frame.
    pub frame_times: &'a [f64],
}

/// A carrier going down or dying.
struct Fall<'a> {
    username: &'a str,
    time: f64,
    down: bool,
    by: Option<&'a str>,
}

/// What the defuser says happened to it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Event {
    /// Given to a player without having lain anywhere.
    Given(u64),
    /// Let go at the first position, lying at the second.
    Dropped([f32; 3], [f32; 3]),
    /// Picked up, by the player named or by the one who dropped it.
    PickedUp(Option<u64>),
    /// Put where a plant that starts will leave it.
    Placed([f32; 3]),
}

fn lost(how: CarryEnd) -> bool {
    matches!(how, CarryEnd::Downed | CarryEnd::Died | CarryEnd::Dropped)
}

fn millis(t: f64) -> f64 {
    (t * 1000.0).round() / 1000.0
}

fn rounded(p: [f32; 3]) -> [f32; 3] {
    p.map(|v| (v * 1000.0).round() / 1000.0)
}

/// Metres between two positions, over the ground.
fn apart(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The defuser's updates as events, each with its seconds since the
/// recording started.
fn object_events(stream: &Stream, frame_times: &[f64]) -> Vec<(f64, Event)> {
    let mut out: Vec<(f64, Event)> = Vec::new();
    let (mut state, mut lying) = (0u8, None);
    for u in &stream.defuser {
        let seconds = frame_times.get(u.frame.unwrap_or(0) as usize);
        let Some(time) = seconds.or(frame_times.last()).copied().map(millis) else {
            continue;
        };
        let now = u.state.unwrap_or(state);
        let fell = now & LYING != 0 && state & LYING == 0;
        state = now;
        if let (true, Some(at)) = (fell, u.position) {
            lying = Some(out.len());
            out.push((time, Event::Dropped(at, at)));
        } else if let Some(drop) = lying {
            if u.position == Some(CARRIED) && u.shown == Some(0) {
                lying = None;
                // The game takes a lying defuser back as the round is
                // decided, in the record that says so on the bombs.
                let cleared = stream.decided.is_some_and(|d| u.frame >= Some(d));
                if u.player_id.is_some() || !cleared {
                    out.push((time, Event::PickedUp(u.player_id)));
                }
            } else if let (Some(at), Event::Dropped(from, _)) = (u.position, out[drop].1) {
                out[drop].1 = Event::Dropped(from, at);
            }
        } else if let (Some(at), true, None) = (u.position, u.turned, u.shown) {
            if at != CARRIED && at != PUT_AWAY {
                out.push((time, Event::Placed(at)));
            }
        } else if let (Some(player), None) = (u.player_id, u.position) {
            out.push((time, Event::Given(player)));
        }
    }
    out
}

impl Input<'_> {
    fn player(&self, username: &str) -> Option<usize> {
        let players = &self.header.players;
        players.iter().position(|p| p.username == username)
    }

    fn username(&self, id: u64) -> Option<&str> {
        let players = &self.header.players;
        let player = players.iter().find(|p| p.id == id && id != 0);
        player.map(|p| p.username.as_str())
    }

    fn team(&self, username: &str) -> usize {
        self.player(username)
            .map_or(0, |p| self.header.players[p].team_index)
    }

    fn position(&self, username: &str, time: f64) -> Option<[f32; 3]> {
        self.positions.at(self.player(username)?, time)
    }

    /// Seconds since the recording started at `tick`.
    fn phase_start(&self, tick: Option<usize>) -> Option<f64> {
        self.timeline.recording.get(tick?).copied().flatten()
    }

    /// The clock reading in force `time` seconds into the recording.
    fn clock(&self, time: f64) -> (String, Phase, f64) {
        let shown = |r: &Option<f64>| r.is_some_and(|r| r <= time);
        let tick = self.timeline.recording.iter().rposition(shown);
        let at = self.timeline.at_time(tick, Some(time));
        (display_clock(at.seconds), at.phase, at.elapsed)
    }

    /// Downs and deaths with their time, from the timeline, or from the
    /// feed and the HUD in a round whose timeline is empty.
    fn falls(&self) -> Vec<Fall<'_>> {
        let mut out: Vec<Fall> = Vec::new();
        for e in self.events {
            let (Some(time), false) = (e.recording_time, e.kind == TimelineKind::Revive) else {
                continue;
            };
            out.push(Fall {
                username: &e.username,
                time,
                down: e.kind == TimelineKind::Down,
                by: e.by.as_deref(),
            });
        }
        if !self.events.is_empty() {
            return out;
        }
        for u in self.feed {
            if let (Some(victim), Some(time)) = (u.victim(), u.recording_time) {
                out.push(Fall {
                    username: victim,
                    time,
                    down: false,
                    by: (u.kind == MatchUpdateType::Kill).then_some(u.username.as_str()),
                });
            }
        }
        for l in self.life_events {
            if let (LifeEventType::Down, Some(time)) = (l.kind, l.recording_time) {
                out.push(Fall {
                    username: &l.username,
                    time,
                    down: true,
                    by: l.by.as_deref(),
                });
            }
        }
        out.sort_by(|a, b| a.time.total_cmp(&b.time));
        out
    }

    /// The round's two bombs, with the header's two site names in order.
    fn sites(&self) -> Vec<Site> {
        let names: Vec<&str> = self.header.site.split(", ").collect();
        let mut bombs: Vec<_> = self.stream.bombs.iter().filter(|b| b.active).collect();
        bombs.sort_by_key(|b| b.index);
        let two = bombs.len() == 2 && names.len() == 2;
        let name = |index: u8| names.get(usize::from(index).checked_sub(1)?).copied();
        bombs
            .iter()
            .map(|b| Site {
                index: b.index,
                object: format!("{:x}", b.object),
                name: name(b.index).filter(|_| two).map(str::to_owned),
                position: rounded(b.position),
            })
            .collect()
    }
}

/// The length of the defuser timer: the clock shows what is left in whole
/// seconds rounded down, so a reading `r` first shown `t` seconds after the
/// plant says the timer is `r + 1 + t` long. The median of all readings,
/// to the second.
fn defuser_timer(timeline: &Timeline, planted_at: f64) -> Option<u32> {
    let mut lengths: Vec<f64> = (timeline.ticks.iter().zip(&timeline.recording))
        .filter(|(t, _)| t.phase == Phase::Planted)
        .filter_map(|(t, shown)| Some(t.seconds + 1.0 + (*shown)? - planted_at))
        .collect();
    lengths.sort_by(f64::total_cmp);
    lengths.get(lengths.len() / 2).map(|l| l.round() as u32)
}

/// Seconds left on the defuser timer at `time`, from the samples the game
/// wrote: between the two around it, or the last one when none follows,
/// since the timer stops with the disable that completes.
fn sampled(samples: &[(f64, u32)], time: f64) -> Option<f64> {
    let after = samples.partition_point(|s| s.0 <= time);
    let (at, left) = *samples.get(after.checked_sub(1)?)?;
    let left = f64::from(left);
    let Some(&(next, then)) = samples.get(after) else {
        return Some(left / 1000.0);
    };
    let share = (time - at) / (next - at);
    Some((left + (f64::from(then) - left) * share) / 1000.0)
}

/// The bomb the defuser at `at` is planted on: `named` by the game-mode
/// object, or the nearer one.
fn plant_site(sites: &[Site], at: [f32; 3], named: Option<u32>) -> Option<PlantSite> {
    let distance = |s: &Site| (apart(at, s.position) * 100.0).round() / 100.0;
    let named = sites.iter().find(|s| Some(u32::from(s.index)) == named);
    let nearest = (sites.iter()).min_by(|a, b| distance(a).total_cmp(&distance(b)));
    let site = named.or(nearest)?;
    Some(PlantSite {
        index: site.index,
        name: site.name.clone(),
        source: match named {
            Some(_) => SiteSource::Decoded,
            None => SiteSource::Nearest,
        },
        distances: sites.iter().map(distance).collect(),
    })
}

/// Joins the round's sections into what the defuser did.
fn bomb(input: &Input) -> Bomb {
    let a = input.activity;
    let mut out = Bomb {
        sites: input.sites(),
        ..Bomb::default()
    };
    let completed = |i: &&DefuserInteraction| {
        i.kind == InteractionKind::Plant && i.outcome == InteractionOutcome::Completed
    };
    let plant = a.interactions.iter().find(completed);
    // The plant the feed has, when no player's interaction completed.
    let fed = (input.feed.iter())
        .find(|u| u.kind == MatchUpdateType::DefuserPlantComplete)
        .and_then(|u| u.recording_time);
    out.planted_at = plant.and_then(|p| p.end).or(fed);
    out.defuser_timer = (out.planted_at).and_then(|at| defuser_timer(input.timeline, at));
    out.timer_decoded = !a.defuser_timer.is_empty();
    let left = |time: f64| {
        let planted_at = out.planted_at?;
        // The plant itself ends as the timer starts: it has none left.
        if time <= planted_at {
            return None;
        }
        // Without the timer, its length less the time since the plant.
        let reckoned = || Some(f64::from(out.defuser_timer?) - (time - planted_at));
        let left = sampled(&a.defuser_timer, time).or_else(reckoned)?;
        Some(millis(left.max(0.0)))
    };
    let action = input.phase_start(input.timeline.action_start);
    let decided = input.phase_start(input.timeline.end_start);
    out.defuser_time_left = decided.and_then(left);

    let events = object_events(input.stream, input.frame_times);
    out.drops_decoded = !input.stream.defuser.is_empty();
    let placed = |start: f64| {
        events.iter().find_map(|&(time, e)| match e {
            Event::Placed(at) if (time - start).abs() <= SAME_FRAME => Some(rounded(at)),
            _ => None,
        })
    };
    // Where the round's plant left the defuser, for its disables.
    let planted = plant.and_then(|p| placed(p.start));
    for i in &a.interactions {
        let (time, phase, elapsed) = input.clock(i.start);
        let team = input.team(&i.username);
        let position = input.position(&i.username, i.start);
        let (defuser, named) = match i.kind {
            InteractionKind::Plant => (placed(i.start), a.planted_bomb.filter(|_| completed(&i))),
            InteractionKind::Disable => (planted, a.planted_bomb),
        };
        let interaction = Interaction {
            username: i.username.clone(),
            team,
            side: input.header.teams.get(team).and_then(|t| t.role),
            start: i.start,
            end: i.end,
            outcome: i.outcome,
            remaining: i.remaining,
            time,
            phase,
            elapsed,
            position,
            end_position: i.end.and_then(|end| input.position(&i.username, end)),
            defuser_position: defuser,
            site: (defuser.or(position)).and_then(|at| plant_site(&out.sites, at, named)),
            defuser_time_left: left(i.start),
            defuser_time_left_at_end: i.end.and_then(left),
        };
        match i.kind {
            InteractionKind::Plant => out.plants.push(interaction),
            InteractionKind::Disable => out.disables.push(interaction),
        }
    }

    let falls = input.falls();
    // The down comes before the kill that finishes it, and it is the down
    // that takes the defuser.
    let fell = |username: &str, at: f64| {
        let lost = |f: &&Fall| f.username == username && f.time <= at && at - f.time <= JOIN_WINDOW;
        falls.iter().find(lost)
    };
    let reason = |username: &str, at: f64| match fell(username, at) {
        Some(f) if f.down => (CarryEnd::Downed, f.by.map(str::to_owned)),
        Some(f) => (CarryEnd::Died, f.by.map(str::to_owned)),
        None => (CarryEnd::Dropped, None),
    };
    // The carrier by `HasDefuser`, for a drop of a defuser nobody was named
    // for.
    let carrying = |at: f64| {
        let then = |c: &&crate::activity::DefuserCarry| {
            c.start <= at && c.end.is_none_or(|end| end + JOIN_WINDOW >= at)
        };
        a.defuser.iter().rfind(then).map(|c| c.username.as_str())
    };
    let mut holder: Option<&str> = None;
    for &(time, event) in &events {
        match event {
            Event::Given(player) => holder = input.username(player),
            Event::Placed(_) => {}
            Event::Dropped(from, rest) => {
                let Some(username) = holder.or_else(|| carrying(time)) else {
                    continue;
                };
                holder = Some(username);
                let (clock, phase, elapsed) = input.clock(time);
                let (reason, by) = reason(username, time);
                out.drops.push(Drop {
                    username: username.to_owned(),
                    team: input.team(username),
                    reason,
                    by,
                    time: clock,
                    phase,
                    elapsed,
                    recording_time: time,
                    position: Some(rounded(from)),
                    rest_position: Some(rounded(rest)),
                    pickup: None,
                });
            }
            Event::PickedUp(player) => {
                holder = player.and_then(|id| input.username(id)).or(holder);
                let (Some(username), Some(drop)) = (holder, out.drops.last_mut()) else {
                    continue;
                };
                let (clock, phase, elapsed) = input.clock(time);
                drop.pickup.get_or_insert(Pickup {
                    username: username.to_owned(),
                    time: clock,
                    phase,
                    elapsed,
                    recording_time: time,
                    seconds_on_ground: millis((time - drop.recording_time).max(0.0)),
                    by_other: username != drop.username,
                });
            }
        }
    }

    let last = input.timeline.recording.iter().rev().find_map(|r| *r);
    // The carries whose end the defuser did not see, to make drops of.
    let mut unseen = Vec::new();
    for c in &a.defuser {
        let planted = |end: f64| {
            let near = |p: &DefuserInteraction| {
                p.username == c.username && p.end.is_some_and(|e| (e - end).abs() <= JOIN_WINDOW)
            };
            plant.is_some_and(near)
        };
        // The defuser dropped by the carrier while they carried it.
        let dropped = out.drops.iter().rfind(|d| {
            d.username == c.username
                && d.recording_time >= c.start - JOIN_WINDOW
                && c.end
                    .is_none_or(|end| d.recording_time <= end + JOIN_WINDOW)
        });
        let (ended, by) = match (c.end, dropped) {
            (Some(end), _) if planted(end) => (CarryEnd::Planted, None),
            (_, Some(d)) => (d.reason, d.by.clone()),
            (None, None) => (CarryEnd::RoundEnd, None),
            // The defuser never lay anywhere. A carrier who fell with a
            // teammate at hand lost it to them in that frame; otherwise
            // the game moved it, unless there is no defuser to say so.
            (Some(end), None) => match reason(&c.username, end) {
                (CarryEnd::Dropped, _) if decided.is_some_and(|d| end >= d) => {
                    (CarryEnd::RoundEnd, None)
                }
                (CarryEnd::Dropped, _) if action.is_some_and(|a| end < a) || out.drops_decoded => {
                    (CarryEnd::Reassigned, None)
                }
                fell => fell,
            },
        };
        if dropped.is_none() && lost(ended) {
            unseen.push(out.carrier.len());
        }
        let given = matches!(
            out.carrier.last().map(|before| before.ended),
            None | Some(CarryEnd::Reassigned)
        );
        out.carrier.push(Carry {
            username: c.username.clone(),
            team: input.team(&c.username),
            start: c.start,
            end: c.end,
            started: if given || action.is_none_or(|a| c.start < a) {
                CarryStart::Spawn
            } else {
                CarryStart::Pickup
            },
            ended,
            ended_by: by,
            position: input.position(&c.username, c.start),
            end_position: (c.end.or(last)).and_then(|end| input.position(&c.username, end)),
        });
    }

    // A carry lost with no drop of the defuser's own is a drop at the
    // carrier, picked up by the next one.
    for i in unseen {
        let c = &out.carrier[i];
        let Some(end) = c.end else { continue };
        let (time, phase, elapsed) = input.clock(end);
        let pickup = out.carrier.get(i + 1).map(|next| {
            let (time, phase, elapsed) = input.clock(next.start);
            Pickup {
                username: next.username.clone(),
                time,
                phase,
                elapsed,
                recording_time: next.start,
                seconds_on_ground: millis((next.start - end).max(0.0)),
                by_other: next.username != c.username,
            }
        });
        out.drops.push(Drop {
            username: c.username.clone(),
            team: c.team,
            reason: c.ended,
            by: c.ended_by.clone(),
            time,
            phase,
            elapsed,
            recording_time: end,
            position: c.end_position,
            rest_position: None,
            pickup,
        });
    }
    out.drops
        .sort_by(|a, b| a.recording_time.total_cmp(&b.recording_time));

    // Every completed plant ends a carry of the planter's.
    if let Some(p) = plant {
        let carried = |c: &Carry| c.username == p.username && c.ended == CarryEnd::Planted;
        if !out.carrier.iter().any(carried) {
            out.warnings.push(format!(
                "the plant by {} ends no carry of theirs",
                p.username
            ));
        }
    }
    if input.stream.unread > 0 {
        out.warnings.push(format!(
            "{} updates of the defuser and the bombs could not be read",
            input.stream.unread
        ));
    }
    out
}

/// What the round's objective did: the defuser in a Bomb round, and the
/// game mode alone in any other.
pub(crate) fn derive(input: &Input) -> Objective {
    let mode = input.header.game_mode;
    Objective {
        mode,
        bomb: is_bomb(mode).then(|| bomb(input)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::DefuserCarry;
    use crate::header::{Player, Team};
    use crate::movement::{BombSite, DefuserUpdate};
    use crate::timeline::TickInfo;

    fn header() -> Header {
        let player = |name: &str, id, team_index| Player {
            username: name.to_owned(),
            id,
            team_index,
            ..Player::default()
        };
        let team = |role| Team {
            role: Some(role),
            ..Team::default()
        };
        Header {
            game_mode: GameMode::BOMB,
            site: "1F Kitchen, 1F Bar".to_owned(),
            teams: [team(TeamRole::Attack), team(TeamRole::Defense)],
            players: vec![player("a", 11, 0), player("b", 12, 0), player("d", 13, 1)],
            ..Header::default()
        }
    }

    /// Prep to 45 s, action to a plant at 100 s, the defuser timer from
    /// 44 down, and the round decided at 110 s.
    fn timeline() -> Timeline {
        let tick = |phase, elapsed, seconds| TickInfo {
            phase,
            elapsed,
            seconds,
        };
        let mut ticks = vec![
            tick(Phase::Prep, 0.0, 45.0),
            tick(Phase::Action, 45.0, 180.0),
        ];
        let mut recording = vec![Some(0.0), Some(45.0)];
        for s in 0..10 {
            ticks.push(tick(
                Phase::Planted,
                100.0 + f64::from(s),
                44.0 - f64::from(s),
            ));
            recording.push(Some(100.0 + f64::from(s)));
        }
        ticks.push(tick(Phase::End, 110.0, 35.0));
        recording.push(Some(110.0));
        Timeline {
            action_start: Some(1),
            plant_start: Some(2),
            end_start: Some(ticks.len() - 1),
            ticks,
            recording,
        }
    }

    fn carry(username: &str, start: f64, end: Option<f64>) -> DefuserCarry {
        DefuserCarry {
            username: username.to_owned(),
            start,
            end,
        }
    }

    fn interaction(
        username: &str,
        kind: InteractionKind,
        start: f64,
        end: Option<f64>,
        outcome: InteractionOutcome,
    ) -> DefuserInteraction {
        DefuserInteraction {
            username: username.to_owned(),
            kind,
            start,
            end,
            outcome,
            remaining: None,
        }
    }

    fn event(kind: TimelineKind, username: &str, time: f64) -> TimelineEvent {
        TimelineEvent {
            kind,
            username: username.to_owned(),
            by: Some("d".to_owned()),
            weapon: None,
            headshot: None,
            time: String::new(),
            phase: Phase::Action,
            elapsed: 0.0,
            recording_time: Some(time),
        }
    }

    /// One frame a second.
    fn derived(
        activity: &Activity,
        events: &[TimelineEvent],
        header: &Header,
        stream: &Stream,
    ) -> Objective {
        let frame_times: Vec<f64> = (0..200).map(f64::from).collect();
        derive(&Input {
            header,
            activity,
            events,
            feed: &[],
            life_events: &[],
            timeline: &timeline(),
            stream,
            positions: &Positions::default(),
            frame_times: &frame_times,
        })
    }

    #[test]
    fn without_the_defuser_a_carry_ends_for_the_first_reason_that_fits() {
        let activity = Activity {
            defuser: vec![
                // Taken away in prep, then given to `a`, who puts it down.
                carry("b", 0.0, Some(30.0)),
                carry("a", 42.0, Some(50.0)),
                // Shot well after putting it down: still dropped.
                carry("a", 52.0, Some(60.0)),
                // Downed, and finished after the defuser left them.
                carry("b", 61.0, Some(70.1)),
                carry("a", 72.0, Some(80.1)),
                carry("b", 80.1, Some(100.1)),
            ],
            interactions: vec![interaction(
                "b",
                InteractionKind::Plant,
                93.0,
                Some(100.0),
                InteractionOutcome::Completed,
            )],
            ..Activity::default()
        };
        let events = [
            event(TimelineKind::Down, "a", 61.5),
            event(TimelineKind::Revive, "a", 65.0),
            event(TimelineKind::Down, "b", 70.0),
            event(TimelineKind::Kill, "b", 70.3),
            event(TimelineKind::Kill, "a", 80.0),
        ];
        let o = derived(&activity, &events, &header(), &Stream::default());
        let bomb = o.bomb.expect("a Bomb round");
        let ends: Vec<_> = bomb.carrier.iter().map(|c| (c.started, c.ended)).collect();
        use CarryEnd::*;
        use CarryStart::*;
        assert_eq!(
            ends,
            [
                (Spawn, Reassigned),
                (Spawn, Dropped),
                (Pickup, Dropped),
                (Pickup, Downed),
                (Pickup, Died),
                (Pickup, Planted),
            ]
        );
        assert_eq!(bomb.carrier[3].ended_by.as_deref(), Some("d"));
        assert!(bomb.warnings.is_empty(), "{:?}", bomb.warnings);
        assert!(!bomb.drops_decoded);
        // A reassignment is no drop; a hand-over within a frame lay 0 s.
        let drops: Vec<_> = (bomb.drops.iter())
            .map(|d| {
                let p = d.pickup.as_ref().unwrap();
                (d.reason, p.seconds_on_ground, p.by_other)
            })
            .collect();
        assert_eq!(
            drops,
            [
                (Dropped, 2.0, false),
                (Dropped, 1.0, true),
                (Downed, 1.9, true),
                (Died, 0.0, true),
            ]
        );
    }

    fn update(frame: u32) -> DefuserUpdate {
        DefuserUpdate {
            frame: Some(frame),
            ..DefuserUpdate::default()
        }
    }

    /// The defuser of a round: given to `a`, dropped as `a` dies, picked
    /// up by `b`, put down and taken back, planted by `b`, and taken away
    /// by the game after a last drop as the round is decided.
    fn stream() -> Stream {
        let at = |frame, position: [f32; 3]| DefuserUpdate {
            position: Some(position),
            ..update(frame)
        };
        let taken = |frame, player_id| DefuserUpdate {
            shown: Some(0),
            player_id,
            state: Some(1),
            ..at(frame, CARRIED)
        };
        let dropped = |frame, position| DefuserUpdate {
            shown: Some(1),
            state: Some(9),
            ..at(frame, position)
        };
        let placed = |frame, position| DefuserUpdate {
            turned: true,
            ..at(frame, position)
        };
        let bomb = |object, index, x| BombSite {
            object,
            position: [x, 0.0, 0.0],
            active: true,
            index,
        };
        Stream {
            defuser: vec![
                // The snapshot: carried by nobody yet.
                DefuserUpdate {
                    frame: None,
                    shown: Some(0),
                    turned: true,
                    ..at(0, CARRIED)
                },
                DefuserUpdate {
                    player_id: Some(11),
                    state: Some(0),
                    ..update(45)
                },
                dropped(60, [1.0, 2.0, 0.9]),
                at(61, [1.0, 2.0, 0.4]),
                at(62, [1.0, 2.0, 0.0]),
                taken(70, Some(12)),
                dropped(75, [5.0, 5.0, 0.9]),
                taken(78, None),
                // A plant given up, and the one that completes.
                placed(85, [9.0, 1.0, 0.0]),
                placed(86, PUT_AWAY),
                placed(93, [9.5, 1.0, 0.0]),
                DefuserUpdate {
                    shown: Some(1),
                    state: Some(3),
                    ..update(100)
                },
            ],
            bombs: vec![bomb(2, 2, 20.0), bomb(1, 1, 10.0)],
            decided: Some(110),
            ..Stream::default()
        }
    }

    #[test]
    fn the_defuser_says_where_it_was_dropped_and_who_picked_it_up() {
        let plant =
            |start, end, outcome| interaction("b", InteractionKind::Plant, start, end, outcome);
        let activity = Activity {
            // `HasDefuser` runs a tenth of a second behind the defuser.
            defuser: vec![
                carry("a", 42.0, Some(60.1)),
                carry("b", 70.1, Some(75.1)),
                carry("b", 78.1, Some(100.0)),
            ],
            interactions: vec![
                plant(85.0, Some(86.0), InteractionOutcome::Aborted),
                plant(93.0, Some(100.0), InteractionOutcome::Completed),
            ],
            planted_bomb: Some(2),
            ..Activity::default()
        };
        let events = [event(TimelineKind::Kill, "a", 60.0)];
        let o = derived(&activity, &events, &header(), &stream());
        let bomb = o.bomb.unwrap();
        assert!(bomb.drops_decoded && bomb.warnings.is_empty());
        let ends: Vec<_> = bomb.carrier.iter().map(|c| c.ended).collect();
        assert_eq!(ends, [CarryEnd::Died, CarryEnd::Dropped, CarryEnd::Planted]);
        let [died, put_down] = &bomb.drops[..] else {
            panic!("{:?}", bomb.drops)
        };
        assert_eq!((died.username.as_str(), died.reason), ("a", CarryEnd::Died));
        assert_eq!(died.recording_time, 60.0);
        assert_eq!(died.position, Some([1.0, 2.0, 0.9]));
        assert_eq!(died.rest_position, Some([1.0, 2.0, 0.0]));
        let pickup = died.pickup.as_ref().unwrap();
        assert_eq!(
            (
                pickup.username.as_str(),
                pickup.seconds_on_ground,
                pickup.by_other
            ),
            ("b", 10.0, true)
        );
        // A pickup that names nobody is by the player who dropped it.
        let pickup = put_down.pickup.as_ref().unwrap();
        assert_eq!(
            (
                pickup.username.as_str(),
                pickup.seconds_on_ground,
                pickup.by_other
            ),
            ("b", 3.0, false)
        );
        assert_eq!(put_down.rest_position, put_down.position);

        let sites: Vec<_> = (bomb.sites.iter())
            .map(|s| (s.index, s.name.as_deref(), s.position[0]))
            .collect();
        assert_eq!(
            sites,
            [(1, Some("1F Kitchen"), 10.0), (2, Some("1F Bar"), 20.0)]
        );
        // The plant given up is at the nearer bomb; the game names the
        // completed one's, whatever is nearer.
        let found: Vec<_> = (bomb.plants.iter())
            .map(|p| {
                let s = p.site.as_ref().unwrap();
                (p.defuser_position.unwrap()[0], s.index, s.source)
            })
            .collect();
        assert_eq!(
            found,
            [(9.0, 1, SiteSource::Nearest), (9.5, 2, SiteSource::Decoded)]
        );
        assert_eq!(
            bomb.plants[1].site.as_ref().unwrap().distances,
            [1.12, 10.55]
        );
    }

    #[test]
    fn a_defuser_the_game_takes_back_at_the_round_end_is_not_picked_up() {
        let mut stream = stream();
        stream.defuser.truncate(2);
        stream.defuser.push(DefuserUpdate {
            position: Some([3.0, 3.0, 0.9]),
            shown: Some(1),
            state: Some(9),
            ..update(109)
        });
        stream.defuser.push(DefuserUpdate {
            position: Some(CARRIED),
            shown: Some(0),
            state: Some(1),
            ..update(110)
        });
        // `HasDefuser` goes on to the end of the recording.
        let activity = Activity {
            defuser: vec![carry("a", 42.0, None)],
            ..Activity::default()
        };
        let events = [event(TimelineKind::Kill, "a", 109.0)];
        let o = derived(&activity, &events, &header(), &stream);
        let bomb = o.bomb.unwrap();
        assert_eq!(bomb.carrier[0].ended, CarryEnd::Died);
        assert_eq!(bomb.drops.len(), 1);
        assert_eq!(bomb.drops[0].pickup, None);
    }

    #[test]
    fn the_timer_gives_what_is_left_and_the_clock_how_long_it_is() {
        let disable = |start, end, outcome| DefuserInteraction {
            remaining: (outcome != InteractionOutcome::Completed).then_some(5.5),
            ..interaction("d", InteractionKind::Disable, start, end, outcome)
        };
        let mut activity = Activity {
            defuser: vec![carry("a", 0.0, Some(100.0))],
            interactions: vec![
                interaction(
                    "a",
                    InteractionKind::Plant,
                    93.0,
                    Some(100.0),
                    InteractionOutcome::Completed,
                ),
                disable(102.5, Some(104.0), InteractionOutcome::Aborted),
                disable(106.0, None, InteractionOutcome::Unfinished),
            ],
            ..Activity::default()
        };
        // Without the timer: its length less the time since the plant.
        let o = derived(&activity, &[], &header(), &Stream::default());
        let bomb = o.bomb.unwrap();
        assert!(!bomb.timer_decoded);
        assert_eq!(bomb.planted_at, Some(100.0));
        assert_eq!(bomb.defuser_timer, Some(45));
        assert_eq!(bomb.defuser_time_left, Some(35.0));
        let plant = &bomb.plants[0];
        assert_eq!((plant.team, plant.side), (0, Some(TeamRole::Attack)));
        assert_eq!((plant.time.as_str(), plant.phase), ("3:00", Phase::Action));
        assert_eq!(plant.defuser_time_left, None);
        assert_eq!(plant.defuser_time_left_at_end, None);
        let left: Vec<_> = (bomb.disables.iter())
            .map(|d| (d.defuser_time_left, d.defuser_time_left_at_end))
            .collect();
        assert_eq!(left, [(Some(42.5), Some(41.0)), (Some(39.0), None)]);
        assert_eq!(bomb.disables[0].time, "0:42");
        assert_eq!(bomb.disables[0].side, Some(TeamRole::Defense));
        assert_eq!(bomb.disables[0].remaining, Some(5.5));

        // With it: between two samples, and the last one after them.
        activity.defuser_timer = vec![(100.0, 44_940), (102.0, 42_940), (104.0, 40_940)];
        let o = derived(&activity, &[], &header(), &Stream::default());
        let bomb = o.bomb.unwrap();
        assert!(bomb.timer_decoded);
        let left: Vec<_> = (bomb.disables.iter())
            .map(|d| (d.defuser_time_left, d.defuser_time_left_at_end))
            .collect();
        assert_eq!(left, [(Some(42.44), Some(40.94)), (Some(40.94), None)]);
        assert_eq!(bomb.defuser_time_left, Some(40.94));
    }

    #[test]
    fn another_game_mode_gives_its_name_alone() {
        let mut header = header();
        header.game_mode = GameMode(1983085217);
        let activity = Activity {
            defuser: vec![carry("a", 0.0, None)],
            ..Activity::default()
        };
        let o = derived(&activity, &[], &header, &Stream::default());
        assert_eq!(o.bomb, None);
        assert_eq!(
            serde_json::to_string(&o).unwrap(),
            r#"{"mode":{"name":"SecureArea","id":1983085217}}"#
        );
    }
}
