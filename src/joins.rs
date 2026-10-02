//! What the marker, movement and state streams say apart, put together
//! (Y11S3): the marks of one scan as one spot, who spotted, the points a
//! spotter gets for a teammate's kill, what identified a player, and what
//! the killer's team knew of the victim of a kill.
//!
//! Everything a join adds is inferred, and each value says so in a field
//! beside it or in its documentation. None of it is read from the file as
//! such.
//!
//! **Spots.** The marker stream writes a mark on a spotted player (see
//! [`crate::markers`]), and another every 1.5 to 1.9 s while the scan goes
//! on. Marks on one player at most 2.5 s apart are one spot: `seconds` is
//! from the first mark to the last, and 0 for a single mark. How long the
//! red marker then stays is not in the file.
//!
//! **Who spotted** is not in the file either. `by` is the first of these
//! to name a player, and `bySource` says which did:
//!
//! - `spotAssistScore`: the player who got the points of a spot assist
//!   for this spot (below).
//! - `onlyObserver`: one opponent of the spotted player looked through a
//!   drone or camera at the first mark, by the player tables (see
//!   [`crate::devices`]).
//! - `nearestFacingTool`: several looked through several devices; the
//!   device nearest to the spotted player is also the one turned most
//!   towards them (the local +Y of its rotation, over the ground), and one
//!   player was on it.
//!
//! Otherwise `byCandidates` lists the opponents who were on a device: all
//! of them, or those on the nearest facing device when several were on
//! it. Against the first rule, on 41 real spots that have its points:
//! `onlyObserver` named that player in 19 of 20, `nearestFacingTool` in 6
//! of 8, and the player was among the candidates in 13 of 13. `with` and
//! `device` are the device of `by`, or the one device all candidates were
//! on.
//!
//! **Spot assists.** The score gives no reasons, only amounts (see
//! [`crate::intel`]). A spotter gets 50 points when a teammate kills the
//! spotted player: in real rounds 36 of 39 kills within 6 s of the newest
//! mark came with them, 10 of 21 kills 6 to 14 s after it, and none of 24
//! later ones. So a spot assist is a +50 of one player alone, with no
//! assist counted (`MatchAssists`), from 0.3 s before to 0.8 s after a
//! teammate's kill of a player whose spot has a mark at most 14 s old.
//! Other awards are 50 too, a revive for one.
//!
//! **Causes of a reveal.** An `identified` reveal (see [`crate::intel`])
//! gets as `cause` what the marker stream wrote within 0.25 s of it: a
//! spot mark on the player, a tracking marker on them, a spot mark on a
//! teammate, or an opponent's ping on an object, in that order. That it
//! was the cause is inferred from the moment alone.
//!
//! **Kills.** A kill gains `victimSpotted` when the newest spot mark on
//! the victim that the killer's team saw is at most 15 s old, and
//! `victimPinged` when a player of the killer's team put a ping within 3 m
//! of where the victim was hit, at most 15 s before. Both say what was on
//! the killer's screen, not that it led to the kill.

use std::collections::HashMap;

use serde::Serialize;

use crate::details::ObservationSession;
use crate::devices::{Camera, Drone, Pose, View};
use crate::feedback::{MatchUpdate, MatchUpdateType, VictimKnown};
use crate::header::Player;
use crate::intel::{Cause, Gain, Reveal, Trigger};
use crate::loadout::When;
use crate::markers::{Ping, SpotMark, Track};

/// Marks of one scan come 1.5 to 1.9 s apart: marks on one player at most
/// this far apart are one spot (seconds).
const SPOT_GAP: f64 = 2.5;
/// A kill this long after a mark still pays the spotter (13.2 s was the
/// longest seen).
const ASSIST_WINDOW: f64 = 14.0;
/// What a spot assist is worth.
const SPOT_ASSIST: i64 = 50;
/// The points come from this long before the feed shows the kill to this
/// long after it.
const PAID_BEFORE: f64 = 0.3;
const PAID_AFTER: f64 = 0.8;
/// A +50 this close to a write of the assist count is the assist's.
const ASSIST: f64 = 0.4;
/// This many equal rises of one team in a frame are a reward of the team.
const TEAM_REWARD: usize = 4;
/// A session is its player's from this long before it starts to this long
/// after it ends.
const SESSION_SLACK: f64 = 0.3;
/// The marker stream and a reveal are written within this of each other.
const SAME_MOMENT: f64 = 0.25;
/// What a kill's victim is still known by: a mark or ping this old
/// (seconds), a ping this near (metres).
const KNOWN_SECONDS: f64 = 15.0;
const PING_REACH: f64 = 3.0;

/// How the spotter was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SpotBy {
    /// They got the points of a spot assist for this spot.
    SpotAssistScore,
    /// They alone looked through a device.
    OnlyObserver,
    /// They alone were on the device nearest to the spotted player, which
    /// was also the one turned most towards them.
    NearestFacingTool,
}

/// What a player spotted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Tool {
    Drone,
    Camera,
}

/// An operator spotted for a team: the marks of one scan. `when` is the
/// first mark.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spot {
    /// The player who was spotted.
    pub username: String,
    /// The team that sees the marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seen_by: Option<usize>,
    /// Where the spotted player's body was at the first mark.
    pub position: [f64; 3],
    /// From the first mark to the last.
    pub seconds: f64,
    pub marks: usize,
    /// Who spotted. Inferred, as `by_source` says how; absent when nobody
    /// can be named.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_source: Option<SpotBy>,
    /// Without `by`: the opponents who looked through a device.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub by_candidates: Vec<String>,
    /// The device of `by`, or the one all candidates were on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with: Option<Tool>,
    /// Its entity id, as `drones` and `cameras` give it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(flatten)]
    pub when: When,
    /// When each mark was written, in seconds since the recording started.
    #[serde(skip)]
    pub mark_times: Vec<f64>,
}

/// The points a spotter got as a teammate killed the player they spotted.
/// Inferred from the amount and the moment; `when` is when the score rose.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotAssist {
    /// Who got the points.
    pub username: String,
    pub victim: String,
    pub killer: String,
    /// Seconds from the newest mark on the victim to the kill.
    pub mark_age: f64,
    #[serde(flatten)]
    pub when: When,
}

/// Who looked through what, and where those devices were.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Sight<'a> {
    pub views: &'a [View],
    pub poses: &'a [Pose],
    pub sessions: &'a [ObservationSession],
    /// The devices of the round, which say what an entity is.
    pub drones: &'a [Drone],
    pub cameras: &'a [Camera],
}

fn round_ms(t: f64) -> f64 {
    (t * 1000.0).round() / 1000.0
}

impl Sight<'_> {
    /// The device `username` looked through `time` seconds into the
    /// recording.
    fn view(&self, username: &str, time: f64) -> Option<u64> {
        let open = |v: &&View| {
            v.username == username
                && round_ms(v.from) <= time
                && v.to.is_none_or(|end| time < round_ms(end))
        };
        self.views.iter().rfind(open).map(|v| v.entity)
    }

    /// Where the device was and how it was turned by then.
    fn pose(&self, entity: u64, time: f64) -> (Option<[f32; 3]>, Option<[f32; 4]>) {
        fn last<T: Copy>(series: &[(f64, T)], time: f64) -> Option<(f64, T)> {
            let i = series.partition_point(|s| s.0 <= time).checked_sub(1)?;
            series.get(i).copied()
        }
        // An id can be given to a second entity: the newer state counts.
        fn newest<T>(a: Option<(f64, T)>, b: Option<(f64, T)>) -> Option<(f64, T)> {
            match (a, b) {
                (Some(a), Some(b)) if a.0 > b.0 => Some(a),
                (a, b) => b.or(a),
            }
        }
        let (mut place, mut turn) = (None, None);
        for p in self.poses.iter().filter(|p| p.entity == entity) {
            place = newest(place, last(&p.places, time));
            turn = newest(turn, last(&p.turns, time));
        }
        (place.map(|p| p.1), turn.map(|t| t.1))
    }

    /// What kind of device the entity with this id (hex) is: as the round
    /// lists it, else by the class of an entity that was never out.
    fn tool(&self, entity: &str) -> Option<Tool> {
        if self.drones.iter().any(|d| d.entity == entity) {
            return Some(Tool::Drone);
        }
        if self.cameras.iter().any(|c| c.entity == entity) {
            return Some(Tool::Camera);
        }
        let pose = (self.poses.iter()).find(|p| format!("{:08x}", p.entity) == entity)?;
        Some(if pose.drone {
            Tool::Drone
        } else {
            Tool::Camera
        })
    }

    /// The session `username` was in at `time`.
    fn session(&self, username: &str, time: f64) -> Option<&ObservationSession> {
        self.sessions.iter().find(|s| {
            let Some(start) = s.recording_time else {
                return false;
            };
            s.username == username
                && start - SESSION_SLACK <= time
                && time <= start + s.seconds + SESSION_SLACK
        })
    }
}

/// Where the local +Y of a rotation points over the ground: the way a
/// drone or camera faces.
fn forward(q: [f32; 4]) -> [f64; 2] {
    let [x, y, z, w] = q.map(f64::from);
    [2.0 * (x * y - w * z), 1.0 - 2.0 * (x * x + z * z)]
}

/// Degrees between `facing` and the way to a point `to` away, over the
/// ground.
fn off_bearing(facing: [f64; 2], to: [f64; 3]) -> f64 {
    let turn = (facing[1].atan2(facing[0]) - to[1].atan2(to[0])).to_degrees();
    ((turn + 180.0).rem_euclid(360.0) - 180.0).abs()
}

/// The marks of each scan as one spot, in the order the scans started.
fn join(marks: &[SpotMark]) -> Vec<Spot> {
    let mut out: Vec<Spot> = Vec::new();
    // Player -> the spot their last mark is in.
    let mut open: HashMap<&str, usize> = HashMap::new();
    for m in marks {
        let time = m.when.recording_time.unwrap_or(0.0);
        let current = open.get(m.username.as_str()).and_then(|&i| out.get_mut(i));
        let last = current.as_ref().and_then(|s| s.mark_times.last().copied());
        match (current, last) {
            (Some(spot), Some(last)) if time - last <= SPOT_GAP => {
                // A frame can say a mark twice.
                if time != last {
                    spot.mark_times.push(time);
                    spot.marks += 1;
                    let start = spot.mark_times.first().copied().unwrap_or(time);
                    spot.seconds = round_ms(time - start);
                }
            }
            _ => {
                open.insert(&m.username, out.len());
                out.push(Spot {
                    username: m.username.clone(),
                    seen_by: m.seen_by,
                    position: m.position,
                    marks: 1,
                    when: m.when.clone(),
                    mark_times: vec![time],
                    ..Spot::default()
                });
            }
        }
    }
    out
}

/// A +50 that is a spot assist: `(player, index of the spot, index of the
/// kill in the feed, the rise)`.
type Paid = (usize, usize, usize, Gain);

/// The spot assists among the `gains`: see the module. `assists` are the
/// times each player's assist count was written.
fn paid(
    spots: &[Spot],
    players: &[Player],
    feed: &[MatchUpdate],
    gains: &[Vec<Gain>],
    assists: &[Vec<f64>],
) -> Vec<Paid> {
    let team = |i: usize| players.get(i).map(|p| p.team_index);
    let index = |name: &str| players.iter().position(|p| p.username == name);
    // Every +50, in the order written.
    let mut fifties: Vec<(usize, Gain)> = (gains.iter().enumerate())
        .flat_map(|(i, g)| g.iter().map(move |g| (i, *g)))
        .filter(|(_, g)| g.points == SPOT_ASSIST)
        .collect();
    fifties.sort_by(|a, b| (a.1.time.total_cmp(&b.1.time)).then(a.1.at.cmp(&b.1.at)));
    // Not one of a team's equal rises in a frame.
    let lone = |player: usize, gain: &Gain| {
        let equal = (gains.iter().enumerate())
            .filter(|(i, _)| team(*i) == team(player))
            .flat_map(|(_, g)| g)
            .filter(|g| g.frame == gain.frame && g.points == gain.points);
        equal.count() < TEAM_REWARD
    };
    let assisted = |player: usize, gain: &Gain| {
        let written = assists.get(player).map_or(&[][..], Vec::as_slice);
        (written.iter()).any(|a| (round_ms(*a) - round_ms(gain.time)).abs() < ASSIST)
    };

    // The rise at each index of `fifties`, with the spot and kill it is
    // for; a later spot takes a rise an earlier one would have had.
    let mut found: Vec<Option<(usize, usize)>> = vec![None; fifties.len()];
    for (s, spot) in spots.iter().enumerate() {
        let (Some(victim), Some(last)) = (index(&spot.username), spot.mark_times.last()) else {
            continue;
        };
        for (k, kill) in feed.iter().enumerate() {
            let (Some(at), Some(killer)) = (kill.recording_time, index(&kill.username)) else {
                continue;
            };
            if kill.kind != MatchUpdateType::Kill
                || kill.target != spot.username
                || team(killer) == team(victim)
            {
                continue;
            }
            let fresh = |mark: &f64| (0.0..=ASSIST_WINDOW).contains(&(at - mark));
            if !spot.mark_times.iter().any(fresh) {
                continue;
            }
            // A newer spot of the same player before the kill is the one
            // that counts.
            let newer = spots.iter().any(|other| {
                let first = other.mark_times.first();
                other.username == spot.username && first.is_some_and(|f| last < f && *f <= at)
            });
            if newer {
                continue;
            }
            for (slot, (player, gain)) in found.iter_mut().zip(&fifties) {
                let after = round_ms(gain.time) - at;
                if *player != killer
                    && team(*player) == team(killer)
                    && -PAID_BEFORE < after
                    && after < PAID_AFTER
                    && lone(*player, gain)
                    && !assisted(*player, gain)
                {
                    *slot = Some((s, k));
                }
            }
        }
    }
    (found.iter().zip(&fifties))
        .filter_map(|(found, &(player, gain))| found.map(|(s, k)| (player, s, k, gain)))
        .collect()
}

/// Names who spotted, for one spot. `paid` is who got the points of a
/// spot assist for it.
fn spotter(spot: &mut Spot, players: &[Player], sight: &Sight, paid: Option<usize>) {
    let name = |i: usize| players.get(i).map(|p| p.username.clone());
    let Some(victim) = players.iter().find(|p| p.username == spot.username) else {
        return;
    };
    let start = spot.mark_times.first().copied().unwrap_or(0.0);
    // The opponents on a device at the first mark: `(player, device)`.
    let mut on: Vec<(usize, u64)> = (players.iter().enumerate())
        .filter(|(_, p)| p.team_index != victim.team_index)
        .filter_map(|(i, p)| Some((i, sight.view(&p.username, start)?)))
        .collect();
    let mut devices: Vec<u64> = on.iter().map(|c| c.1).collect();
    devices.sort_unstable();
    devices.dedup();

    let mut by = None;
    if let Some(player) = paid {
        by = Some((player, SpotBy::SpotAssistScore));
    } else if let &[(player, _)] = on.as_slice() {
        by = Some((player, SpotBy::OnlyObserver));
    } else if devices.len() > 1 {
        // `(device, metres to the spotted player, degrees off them)`.
        let mut placed: Vec<(u64, f64, Option<f64>)> = Vec::new();
        for &(_, device) in &on {
            let (Some(at), turn) = sight.pose(device, start) else {
                continue;
            };
            let to = [0, 1, 2].map(|i| spot.position[i] - f64::from(at[i]));
            let metres = to.iter().map(|v| v * v).sum::<f64>().sqrt();
            placed.push((device, metres, turn.map(|q| off_bearing(forward(q), to))));
        }
        let nearest = placed.iter().min_by(|a, b| a.1.total_cmp(&b.1));
        let facing =
            (placed.iter()).min_by(|a, b| a.2.unwrap_or(999.0).total_cmp(&b.2.unwrap_or(999.0)));
        if let (Some(near), Some(face)) = (nearest, facing)
            && near.0 == face.0
        {
            on.retain(|c| c.1 == near.0);
            if let &[(player, _)] = on.as_slice() {
                by = Some((player, SpotBy::NearestFacingTool));
            }
        }
    }
    spot.by = by.and_then(|b| name(b.0));
    spot.by_source = by.map(|b| b.1).filter(|_| spot.by.is_some());
    if spot.by.is_none() {
        spot.by_candidates = on.iter().filter_map(|c| name(c.0)).collect();
        spot.by_candidates.sort();
    }

    // The device: that of `by`, or the one every candidate was on.
    let shared = match on.as_slice() {
        [first, rest @ ..] if rest.iter().all(|c| c.1 == first.1) => Some(first.0),
        _ => None,
    };
    let Some(who) = by.map(|b| b.0).or(shared) else {
        return;
    };
    // The tables name the device; a session does when they do not.
    let viewed = (on.iter().find(|c| c.0 == who).map(|c| c.1))
        .or_else(|| sight.view(players.get(who)?.username.as_str(), start));
    let session = (players.get(who)).and_then(|p| sight.session(&p.username, start));
    let of_session = session.and_then(|s| {
        if s.tool.is_drone() {
            Some(Tool::Drone)
        } else if s.tool.is_camera() {
            Some(Tool::Camera)
        } else {
            None
        }
    });
    spot.device = match viewed {
        Some(device) => Some(format!("{device:08x}")),
        None => session.and_then(|s| s.device.clone()),
    };
    let listed = spot.device.as_deref().and_then(|d| sight.tool(d));
    spot.with = listed.or(of_session);
}

/// Makes the spots of the round from its `marks`, names who spotted, and
/// lists the spot assists. `gains` and `assists` are per player, as
/// [`crate::intel`] read them; `when` places a rise of the score.
pub(crate) fn spots(
    marks: &[SpotMark],
    players: &[Player],
    sight: &Sight,
    feed: &[MatchUpdate],
    gains: &[Vec<Gain>],
    assists: &[Vec<f64>],
    when: impl Fn(&Gain) -> When,
) -> (Vec<Spot>, Vec<SpotAssist>) {
    let mut spots = join(marks);
    let paid = paid(&spots, players, feed, gains, assists);
    for (s, spot) in spots.iter_mut().enumerate() {
        // The first to be paid for this spot.
        let first = paid.iter().find(|p| p.1 == s).map(|p| p.0);
        spotter(spot, players, sight, first);
    }
    let mut assists = Vec::new();
    for &(player, s, k, gain) in &paid {
        let (Some(player), Some(spot), Some(kill)) =
            (players.get(player), spots.get(s), feed.get(k))
        else {
            continue;
        };
        let at = kill.recording_time.unwrap_or(0.0);
        let newest = (spot.mark_times.iter().rev()).find(|m| **m <= at);
        assists.push(SpotAssist {
            username: player.username.clone(),
            victim: spot.username.clone(),
            killer: kill.username.clone(),
            mark_age: newest.map_or(0.0, |m| round_ms(at - m)),
            when: when(&gain),
        });
    }
    (spots, assists)
}

/// Gives each `identified` reveal its cause: what the marker stream wrote
/// within [`SAME_MOMENT`] of it.
pub(crate) fn causes(
    reveals: &mut [Reveal],
    players: &[Player],
    spots: &[Spot],
    tracks: &[Track],
    pings: &[Ping],
) {
    let team = |name: &str| {
        let player = players.iter().find(|p| p.username == name);
        player.map(|p| p.team_index)
    };
    for r in reveals
        .iter_mut()
        .filter(|r| r.trigger == Trigger::Identified)
    {
        let Some(at) = r.when.recording_time else {
            continue;
        };
        let near = |time: f64| (time - at).abs() <= SAME_MOMENT;
        let marked = |s: &&Spot| s.mark_times.iter().any(|m| near(*m));
        let tracked = |t: &&Track| {
            let Some(start) = t.when.recording_time else {
                return false;
            };
            near(start) || t.path.iter().any(|p| near(start + p[0]))
        };
        let own = team(&r.username);
        let mut spotted = spots.iter().filter(marked);
        let pinged = |p: &&Ping| {
            p.label.is_some() && team(&p.username) != own && p.when.recording_time.is_some_and(near)
        };
        r.cause = if spotted.clone().any(|s| s.username == r.username) {
            Some(Cause::Spot)
        } else if (tracks.iter().filter(tracked)).any(|t| t.username == r.username) {
            Some(Cause::AbilityMarker)
        } else if spotted.any(|s| team(&s.username) == own) {
            Some(Cause::TeammateSpot)
        } else if pings.iter().any(|p| pinged(&p)) {
            Some(Cause::Ping)
        } else {
            None
        };
    }
}

/// Says of each kill of the feed what the killer's team knew of the
/// victim: see the module. `place` is where a victim was when killed, for
/// the kills that have it.
pub(crate) fn kills(
    feed: &mut [MatchUpdate],
    players: &[Player],
    spots: &[Spot],
    pings: &[Ping],
    place: impl Fn(&MatchUpdate) -> Option<[f64; 3]>,
) {
    let team = |name: &str| {
        let player = players.iter().find(|p| p.username == name);
        player.map(|p| p.team_index)
    };
    for kill in feed.iter_mut().filter(|u| u.kind == MatchUpdateType::Kill) {
        let (Some(at), Some(side)) = (kill.recording_time, team(&kill.username)) else {
            continue;
        };
        if team(&kill.target) == Some(side) {
            continue;
        }
        let recent = |time: f64| (0.0..=KNOWN_SECONDS).contains(&(at - time));
        // The newest of the marks and of the pings counts.
        let marks = (spots.iter())
            .filter(|s| s.username == kill.target && s.seen_by == Some(side))
            .flat_map(|s| s.mark_times.iter().map(move |m| (*m, s)))
            .filter(|m| recent(m.0));
        kill.victim_spotted = marks
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(time, spot)| VictimKnown {
                seconds_ago: round_ms(at - time),
                by: spot.by.clone(),
            });
        let Some(here) = place(kill) else {
            continue;
        };
        let near = |p: &Ping| {
            let metres = (0..3)
                .map(|i| (p.position[i] - here[i]).powi(2))
                .sum::<f64>();
            metres.sqrt() <= PING_REACH
        };
        let pinged = (pings.iter())
            .filter(|p| team(&p.username) == Some(side) && near(p))
            .filter_map(|p| Some((p.when.recording_time?, p)))
            .filter(|p| recent(p.0));
        kill.victim_pinged = pinged
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(time, ping)| VictimKnown {
                seconds_ago: round_ms(at - time),
                by: Some(ping.username.clone()),
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feedback::Clock;
    use crate::markers::PingKind;

    /// Two attackers (`a0`, `a1`, team 0) and two defenders (`d0`, `d1`).
    fn players() -> Vec<Player> {
        (["a0", "a1", "d0", "d1"].iter().enumerate())
            .map(|(i, name)| Player {
                username: (*name).into(),
                team_index: i / 2,
                ..Player::default()
            })
            .collect()
    }

    fn at(time: f64) -> When {
        When {
            recording_time: Some(time),
            ..When::default()
        }
    }

    /// A mark on `username`, seen by the other team.
    fn mark(username: &str, time: f64) -> SpotMark {
        SpotMark {
            username: username.into(),
            seen_by: Some(usize::from(username.starts_with('a'))),
            position: [10.0, 0.0, 0.0],
            when: at(time),
        }
    }

    fn view(username: &str, entity: u64, from: f64, to: Option<f64>) -> View {
        View {
            username: username.into(),
            entity,
            known: true,
            from,
            to,
        }
    }

    /// A device at `place` that faces along +Y turned by `degrees` about
    /// the vertical: 0 faces +Y, -90 faces +X.
    fn pose(entity: u64, drone: bool, place: [f32; 3], degrees: f32) -> Pose {
        let half = degrees.to_radians() / 2.0;
        Pose {
            entity,
            drone,
            places: vec![(0.0, place)],
            turns: vec![(0.0, [0.0, 0.0, half.sin(), half.cos()])],
        }
    }

    fn kill(by: &str, target: &str, time: f64) -> MatchUpdate {
        MatchUpdate {
            username: by.into(),
            target: target.into(),
            recording_time: Some(time),
            ..MatchUpdate::new(MatchUpdateType::Kill, &Clock::default())
        }
    }

    fn gain(time: f64, points: i64) -> Gain {
        let frame = (time * 10.0).round() as u32;
        Gain {
            time,
            points,
            frame: Some(frame),
            at: frame as usize,
        }
    }

    fn no_gains() -> Vec<Vec<Gain>> {
        vec![Vec::new(); 4]
    }

    fn run(
        marks: &[SpotMark],
        sight: &Sight,
        feed: &[MatchUpdate],
        gains: &[Vec<Gain>],
        assists: &[Vec<f64>],
    ) -> (Vec<Spot>, Vec<SpotAssist>) {
        let when = |g: &Gain| at(round_ms(g.time));
        spots(marks, &players(), sight, feed, gains, assists, when)
    }

    #[test]
    fn marks_of_one_scan_are_one_spot() {
        let marks = [
            mark("d0", 10.0),
            mark("d1", 10.5),
            mark("d0", 11.7),
            // Said twice in a frame.
            mark("d0", 11.7),
            mark("d0", 13.5),
            // Too long after the last one: another spot.
            mark("d0", 16.1),
            mark("d1", 13.0),
        ];
        let (spots, assists) = run(&marks, &Sight::default(), &[], &no_gains(), &[]);
        let told: Vec<_> = (spots.iter())
            .map(|s| {
                (
                    s.username.as_str(),
                    s.when.recording_time,
                    s.seconds,
                    s.marks,
                )
            })
            .collect();
        assert_eq!(
            told,
            [
                ("d0", Some(10.0), 3.5, 3),
                ("d1", Some(10.5), 2.5, 2),
                ("d0", Some(16.1), 0.0, 1),
            ]
        );
        assert_eq!(spots[0].mark_times, [10.0, 11.7, 13.5]);
        assert_eq!(spots[0].seen_by, Some(0));
        assert!(assists.is_empty());
        // Nobody was on a device: nobody is named, and nothing is listed.
        let json = serde_json::to_value(&spots[2]).unwrap();
        for key in [
            "by",
            "bySource",
            "byCandidates",
            "with",
            "device",
            "markTimes",
        ] {
            assert!(json.get(key).is_none(), "{key}");
        }
        assert_eq!((&json["seconds"], &json["marks"]), (&0.0.into(), &1.into()));
    }

    #[test]
    fn the_only_opponent_on_a_device_is_the_spotter() {
        let views = [
            view("a0", 0xF000_0001, 2.0, Some(9.0)),
            view("a1", 0xF000_0002, 5.0, None),
            // The spotted player's own team is on a camera too.
            view("d1", 0x60_0000_0003, 1.0, None),
        ];
        let poses = [pose(0xF000_0002, true, [0.0; 3], 0.0)];
        let sight = Sight {
            views: &views,
            poses: &poses,
            ..Sight::default()
        };
        let (spots, _) = run(&[mark("d0", 10.0)], &sight, &[], &no_gains(), &[]);
        let spot = &spots[0];
        assert_eq!(spot.by.as_deref(), Some("a1"));
        assert_eq!(spot.by_source, Some(SpotBy::OnlyObserver));
        assert_eq!(spot.with, Some(Tool::Drone));
        assert_eq!(spot.device.as_deref(), Some("f0000002"));
        assert!(spot.by_candidates.is_empty());
        let json = serde_json::to_value(spot).unwrap();
        assert_eq!(json["bySource"], "onlyObserver");
        assert_eq!(json["with"], "drone");
    }

    #[test]
    fn of_several_devices_the_nearest_that_faces_the_player_names_the_spotter() {
        // The spotted player is at (10, 0, 0). One drone is 2 m away and
        // faces them (+X); a camera is 6 m away and faces away.
        let views = [
            view("a0", 0xF000_0001, 2.0, None),
            view("a1", 0x60_0000_0002, 2.0, None),
        ];
        let near = pose(0xF000_0001, true, [8.0, 0.0, 0.0], -90.0);
        let far = pose(0x60_0000_0002, false, [16.0, 0.0, 0.0], -90.0);
        let poses = [near.clone(), far.clone()];
        let sight = |poses| Sight {
            views: &views,
            poses,
            ..Sight::default()
        };
        let marks = [mark("d0", 10.0)];
        let (spots, _) = run(&marks, &sight(&poses), &[], &no_gains(), &[]);
        assert_eq!(spots[0].by.as_deref(), Some("a0"));
        assert_eq!(spots[0].by_source, Some(SpotBy::NearestFacingTool));
        assert_eq!(spots[0].with, Some(Tool::Drone));

        // The nearest faces away and the far one faces the player: the
        // two do not agree, so both players are candidates.
        let poses = [
            pose(0xF000_0001, true, [8.0, 0.0, 0.0], 90.0),
            pose(0x60_0000_0002, false, [16.0, 0.0, 0.0], 90.0),
        ];
        let (spots, _) = run(&marks, &sight(&poses), &[], &no_gains(), &[]);
        assert_eq!((spots[0].by.as_deref(), spots[0].by_source), (None, None));
        assert_eq!(spots[0].by_candidates, ["a0", "a1"]);
        assert_eq!((spots[0].with, spots[0].device.as_deref()), (None, None));

        // A device without a place is not compared.
        let poses = [far];
        let (spots, _) = run(&marks, &sight(&poses), &[], &no_gains(), &[]);
        assert_eq!(spots[0].by.as_deref(), Some("a1"));
        assert_eq!(spots[0].with, Some(Tool::Camera));
    }

    #[test]
    fn players_on_one_device_are_candidates_with_that_device() {
        let views = [
            view("a0", 0x60_0000_0002, 2.0, None),
            view("a1", 0x60_0000_0002, 3.0, None),
        ];
        let poses = [pose(0x60_0000_0002, false, [0.0; 3], 0.0)];
        let sight = Sight {
            views: &views,
            poses: &poses,
            ..Sight::default()
        };
        let (spots, _) = run(&[mark("d0", 10.0)], &sight, &[], &no_gains(), &[]);
        assert_eq!(spots[0].by, None);
        assert_eq!(spots[0].by_candidates, ["a0", "a1"]);
        assert_eq!(spots[0].with, Some(Tool::Camera));
        assert_eq!(spots[0].device.as_deref(), Some("6000000002"));
    }

    #[test]
    fn a_lone_fifty_at_a_teammates_kill_of_a_spotted_player_is_a_spot_assist() {
        let marks = [mark("d0", 10.0), mark("d0", 11.6)];
        let feed = [kill("a0", "d0", 20.0)];
        let mut gains = no_gains();
        gains[1] = vec![gain(20.2, 50)];
        // The killer's own points, and a defender's, are not it.
        gains[0] = vec![gain(20.0, 110)];
        gains[2] = vec![gain(20.2, 50)];
        let assists = vec![Vec::new(); 4];
        // Both attackers were on a device: the points name the spotter.
        let views = [
            view("a0", 0xF000_0001, 2.0, None),
            view("a1", 0xF000_0002, 2.0, Some(15.0)),
        ];
        let sight = Sight {
            views: &views,
            poses: &[],
            ..Sight::default()
        };
        let (spots, paid) = run(&marks, &sight, &feed, &gains, &assists);
        assert_eq!(spots[0].by.as_deref(), Some("a1"));
        assert_eq!(spots[0].by_source, Some(SpotBy::SpotAssistScore));
        assert_eq!(spots[0].device.as_deref(), Some("f0000002"));
        assert_eq!(paid.len(), 1);
        let assist = &paid[0];
        assert_eq!(
            (
                assist.username.as_str(),
                assist.victim.as_str(),
                assist.killer.as_str()
            ),
            ("a1", "d0", "a0")
        );
        assert_eq!(assist.mark_age, 8.4);
        assert_eq!(assist.when.recording_time, Some(20.2));
        let json = serde_json::to_value(assist).unwrap();
        assert_eq!(json["markAge"], 8.4);

        // With an assist counted, the 50 is the assist's.
        let mut counted = assists.clone();
        counted[1] = vec![20.3];
        assert!(run(&marks, &sight, &feed, &gains, &counted).1.is_empty());
        // Too long after the kill, or after a mark too old.
        gains[1] = vec![gain(20.9, 50)];
        assert!(run(&marks, &sight, &feed, &gains, &assists).1.is_empty());
        gains[1] = vec![gain(26.0, 50)];
        let late = [kill("a0", "d0", 25.8)];
        assert!(run(&marks, &sight, &late, &gains, &assists).1.is_empty());
        // A kill by the spotted player's own team pays nobody.
        gains[3] = vec![gain(20.2, 50)];
        let own = [kill("d1", "d0", 20.0)];
        assert!(run(&marks, &sight, &own, &gains, &assists).1.is_empty());
    }

    #[test]
    fn equal_rises_of_a_whole_team_are_no_spot_assist() {
        let players: Vec<Player> = (0..6)
            .map(|i| Player {
                username: format!("p{i}"),
                team_index: usize::from(i == 5),
                ..Player::default()
            })
            .collect();
        let mut spot = mark("p5", 10.0);
        spot.seen_by = Some(0);
        let feed = [kill("p0", "p5", 12.0)];
        let mut gains = vec![Vec::new(); 6];
        for g in gains.iter_mut().take(5).skip(1) {
            *g = vec![gain(12.1, 50)];
        }
        let when = |g: &Gain| at(g.time);
        let assists = vec![Vec::new(); 6];
        let sight = Sight::default();
        let (_, paid) = spots(
            &[spot.clone()],
            &players,
            &sight,
            &feed,
            &gains,
            &assists,
            when,
        );
        assert!(paid.is_empty(), "four rises are the team's");
        gains[4].clear();
        let (spots, paid) = spots(&[spot], &players, &sight, &feed, &gains, &assists, when);
        assert_eq!(paid.len(), 3);
        // The first to be paid is named.
        assert_eq!(spots[0].by.as_deref(), Some("p1"));
    }

    #[test]
    fn the_newer_spot_of_a_player_takes_the_points() {
        let marks = [mark("d0", 10.0), mark("d0", 15.0)];
        let feed = [kill("a0", "d0", 18.0)];
        let mut gains = no_gains();
        gains[1] = vec![gain(18.1, 50)];
        let assists = vec![Vec::new(); 4];
        let (spots, paid) = run(&marks, &Sight::default(), &feed, &gains, &assists);
        assert_eq!(spots.len(), 2);
        assert_eq!(spots[0].by, None);
        assert_eq!(spots[1].by.as_deref(), Some("a1"));
        assert_eq!(paid.len(), 1);
        assert_eq!(paid[0].mark_age, 3.0);
        // Not on a device by the tables, but in a session: its device.
        let sessions = [ObservationSession {
            username: "a1".into(),
            owner: String::new(),
            tool: crate::types::ObservationTool(2),
            phase: crate::details::Phase::Action,
            time: String::new(),
            time_in_seconds: 0.0,
            elapsed: 0.0,
            recording_time: Some(14.0),
            seconds: 3.0,
            device: Some("6000000009".into()),
        }];
        let sight = Sight {
            sessions: &sessions,
            ..Sight::default()
        };
        let (spots, _) = run(&marks, &sight, &feed, &gains, &assists);
        assert_eq!(spots[1].with, Some(Tool::Camera));
        assert_eq!(spots[1].device.as_deref(), Some("6000000009"));
    }

    fn ping(username: &str, time: f64, position: [f64; 3], label: bool) -> Ping {
        Ping {
            username: username.into(),
            team: None,
            kind: PingKind::Location,
            target: 3,
            label: label.then_some(crate::markers::Label { id: 1, name: None }),
            position,
            when: at(time),
        }
    }

    fn track(username: &str, time: f64, path: Vec<[f64; 4]>) -> Track {
        Track {
            username: username.into(),
            source: crate::markers::Source { id: 5, name: None },
            by: None,
            by_source: None,
            seen_by: None,
            position: [0.0; 3],
            path,
            pulse: false,
            ended: None,
            seconds: None,
            open: false,
            when: at(time),
        }
    }

    #[test]
    fn an_identified_reveal_gets_what_the_markers_wrote_in_that_moment() {
        let reveal = |username: &str, time: f64, trigger: Trigger| Reveal {
            username: username.into(),
            trigger,
            when: at(time),
            ..Reveal::default()
        };
        let mut reveals = vec![
            reveal("d0", 10.1, Trigger::Identified),
            reveal("d1", 10.2, Trigger::Identified),
            reveal("a0", 10.2, Trigger::Identified),
            reveal("a1", 30.0, Trigger::Identified),
            reveal("d0", 40.1, Trigger::Identified),
            reveal("d1", 50.0, Trigger::Identified),
            // A kill explains itself.
            reveal("d0", 10.0, Trigger::Kill),
            reveal("a1", 60.0, Trigger::Identified),
        ];
        let (spots, _) = run(
            &[mark("d0", 10.0)],
            &Sight::default(),
            &[],
            &no_gains(),
            &[],
        );
        // A marker that moved 2 s after it started, at 30 s.
        let tracks = [track(
            "a1",
            28.0,
            vec![[0.0, 0.0, 0.0, 0.0], [2.0, 1.0, 0.0, 0.0]],
        )];
        let pings = [
            ping("a0", 40.0, [0.0; 3], true),
            // A ping on no object, and one by the player's own team.
            ping("a0", 50.0, [0.0; 3], false),
            ping("d0", 50.0, [0.0; 3], true),
        ];
        causes(&mut reveals, &players(), &spots, &tracks, &pings);
        let told: Vec<Option<Cause>> = reveals.iter().map(|r| r.cause).collect();
        assert_eq!(
            told,
            [
                Some(Cause::Spot),
                Some(Cause::TeammateSpot),
                None,
                Some(Cause::AbilityMarker),
                Some(Cause::Ping),
                None,
                None,
                None,
            ]
        );
        let json = serde_json::to_value(&reveals[1]).unwrap();
        assert_eq!(json["cause"], "teammateSpot");
        assert!(
            serde_json::to_value(&reveals[2])
                .unwrap()
                .get("cause")
                .is_none()
        );
    }

    #[test]
    fn a_kill_says_what_the_killers_team_knew_of_the_victim() {
        let views = [view("a1", 0xF000_0002, 2.0, None)];
        let sight = Sight {
            views: &views,
            ..Sight::default()
        };
        let marks = [mark("d0", 10.0), mark("d0", 11.5), mark("a0", 12.0)];
        let (spots, _) = run(&marks, &sight, &[], &no_gains(), &[]);
        let pings = [
            ping("a1", 9.0, [1.0, 1.0, 0.0], false),
            ping("a1", 12.0, [2.0, 0.0, 0.0], false),
            // Too far, by the other team, and after the kill.
            ping("a1", 13.0, [9.0, 0.0, 0.0], false),
            ping("d1", 13.5, [0.0; 3], false),
            ping("a0", 15.0, [0.0; 3], false),
        ];
        let mut feed = vec![
            kill("a0", "d0", 14.0),
            // Marked 20 s before: no longer known. No place: no ping.
            kill("a0", "d0", 31.5),
            // A defender kills a spotted attacker nobody can be named for.
            kill("d1", "a0", 14.0),
            // A kill of a teammate says nothing.
            kill("d1", "d0", 14.0),
        ];
        let place = |u: &MatchUpdate| (u.recording_time == Some(14.0)).then_some([0.0; 3]);
        kills(&mut feed, &players(), &spots, &pings, place);
        let spotted = feed[0].victim_spotted.as_ref().unwrap();
        assert_eq!(
            (spotted.seconds_ago, spotted.by.as_deref()),
            (2.5, Some("a1"))
        );
        let pinged = feed[0].victim_pinged.as_ref().unwrap();
        assert_eq!(
            (pinged.seconds_ago, pinged.by.as_deref()),
            (2.0, Some("a1"))
        );
        assert!(feed[1].victim_spotted.is_none() && feed[1].victim_pinged.is_none());
        let spotted = feed[2].victim_spotted.as_ref().unwrap();
        assert_eq!((spotted.seconds_ago, spotted.by.as_deref()), (2.0, None));
        let pinged = feed[2].victim_pinged.as_ref().unwrap();
        assert_eq!(
            (pinged.seconds_ago, pinged.by.as_deref()),
            (0.5, Some("d1"))
        );
        assert!(feed[3].victim_spotted.is_none() && feed[3].victim_pinged.is_none());
        let json = serde_json::to_value(&feed[2]).unwrap();
        assert_eq!(
            json["victimSpotted"],
            serde_json::json!({ "secondsAgo": 2.0 })
        );
        assert!(
            serde_json::to_value(&feed[1])
                .unwrap()
                .get("victimPinged")
                .is_none()
        );
    }

    #[test]
    fn a_device_faces_along_its_own_y() {
        let level = |degrees: f32| {
            let half = degrees.to_radians() / 2.0;
            forward([0.0, 0.0, half.sin(), half.cos()])
        };
        let [x, y] = level(0.0);
        assert!(x.abs() < 1e-6 && (y - 1.0).abs() < 1e-6);
        let [x, y] = level(-90.0);
        assert!((x - 1.0).abs() < 1e-6 && y.abs() < 1e-6);
        // Degrees off a point, whichever way round.
        assert!(off_bearing([0.0, 1.0], [0.0, 5.0, 3.0]) < 1e-9);
        assert!((off_bearing([0.0, 1.0], [5.0, 0.0, 0.0]) - 90.0).abs() < 1e-9);
        assert!((off_bearing([0.0, 1.0], [-5.0, 0.0, 0.0]) - 90.0).abs() < 1e-9);
        assert!((off_bearing([1.0, 0.0], [-5.0, 0.0, 0.0]) - 180.0).abs() < 1e-9);
    }
}
