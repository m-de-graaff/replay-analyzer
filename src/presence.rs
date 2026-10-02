//! Players leaving a match and coming back, and the seats they leave
//! behind (Y11S3).
//!
//! Every seat of a match is a controller object (class `2759e897`) in the
//! `state` stream, and a controller says who sits in it:
//!
//! ```text
//! 22 b6b71dd2 04 <u32>      PlayerSlotType
//! 22 ca35436c 01 <u8>       HasLeft
//! 22 eed445c8 08 <u64>      OnlinePlayerID: the header's playerid
//! 22 8a509bd0 24 <ascii>    the profile id
//! 22 7dd4fc18 04 <u32>      PlayerPlatform
//! 22 07949bdc <n> <utf-8>   Name
//! 22 951c1650 08 <u64>      TeamVM: the team's object
//! ```
//!
//! When a player leaves, their controller is sent again in full, in a run
//! that starts `1b <controller> 00000000 4154dcc4` (`PlayerLifeVM`). The
//! controllers of teammates may be sent again with nothing changed, so
//! only a value that differs from the one before is an event.
//!
//! `PlayerSlotType` takes four values. What each means is not in the file;
//! it is inferred from what else the controller holds and from what
//! follows, over 185 real rounds:
//!
//! - 1: a player is in the seat.
//! - 4: the seat is held for the player who left (`reserved`). The profile
//!   id and the `playerid` stay. Ranked.
//! - 2: the seat was emptied for someone else to take (`opened`). The
//!   `playerid` turns ff*8, the profile id a UUID of zeros and the platform
//!   12; the name stays. Quick Match.
//! - 3: a player has connected and waits for the next round (`joining`).
//!   The name is written empty and then, 0.1 to 0.4 s later, in full.
//!
//! The changes seen in a recording are 1 to 4, 1 to 2, 3 to 2, 4 to 3 (a
//! reconnect) and 2 to 3 (a join). Nothing turns 1 while a round is
//! recorded: a seat at 3 reads 1 in the snapshot that opens the next
//! round. `HasLeft` turns 1 at a leave and never back, so a seat at 1 with
//! `HasLeft` 1 was vacated at some point and filled again (`refilled`).
//! A seat at 2 with `HasLeft` 0 and no name is one nobody ever sat in
//! (`empty`): a custom match of two players has eight.
//!
//! The feed says the same (see [`crate::messages`]): a `playerLeft` line
//! follows a change to 2 or 4 by 0 to 0.4 s, a `playerReconnected` line
//! comes with a change from 4 to 3 and a `playerJoined` line with one from
//! 2 to 3. A line and a change are paired when they name the same player
//! within [`LINE_WINDOW`] seconds. One `playerLeft` line was seen with no
//! change at all, 0.03 s before its recording ended: such a leaver says
//! `source: feedOnly`.
//!
//! Whether the leaver was alive is told by the kill feed: alive is a
//! player who spawned and has no death before the leave. The movement
//! stream agrees: the body of a player who leaves alive is deleted
//! (`637385fe`) in the frame of the change or just before, and a corpse is
//! not.
//!
//! # What is inferred
//!
//! The file never says why a player left. Three things are kept that bear
//! on it, and each says `inferred`:
//!
//! - `connectionLost`: the feed showed the line of id `66f4020000000065`
//!   just before the leave (0 to 0.04 s before the change of the seat). Its
//!   wording is not in the file. The three players who left alive with it
//!   had sent no input for 12.75, 15.5 and 91 s; the one who left alive
//!   without it had moved 3.6 s before.
//! - `silentSeconds`: how long before the leave the player's body last
//!   moved or turned, or the player last switched to a drone or camera.
//! - Across the rounds of a match, the `playerid` a player comes back
//!   with. It is fixed for one launch of the game, so a new one means the
//!   game was started again and the same one that it kept running.
//!
//! A player who quits and a game that crashes and closes its connection
//! cleanly look the same. A leave or a return between two rounds is in
//! neither recording: the next round's snapshot shows the seat and the
//! header lists the player or not, and that is all.
//!
//! # Across a match
//!
//! [`rollup`] follows each player who left through the rounds of a match,
//! by profile id: the leaves and returns a recording holds, the first
//! round that started without them, the round whose header lists them
//! again, and the rounds they missed while a seat was held for them. What
//! it says of why (`likely`) repeats the three observations above and is
//! no verdict on the player.

use std::collections::HashMap;

use serde::Serialize;

use crate::entities::{Hash, Record, ViewChange, for_each_record};
use crate::header::Player;
use crate::loadout::{Clock, Input, STATE_STREAM, When};
use crate::messages::{Kind, SystemMessage};
use crate::round::Round;

/// `PlayerSlotType`, `HasLeft` and the properties that say who a
/// controller's player is.
pub(crate) const SLOT_TYPE: Hash = [0xB6, 0xB7, 0x1D, 0xD2];
const HAS_LEFT: Hash = [0xCA, 0x35, 0x43, 0x6C];
const PLAYER_ID: Hash = [0xEE, 0xD4, 0x45, 0xC8];
const NAME: Hash = [0x07, 0x94, 0x9B, 0xDC];
const PROFILE_ID: Hash = [0x8A, 0x50, 0x9B, 0xD0];
const PLATFORM: Hash = [0x7D, 0xD4, 0xFC, 0x18];
const TEAM: Hash = [0x95, 0x1C, 0x16, 0x50];
/// The slot types: a player, an opened seat, a player waiting for the
/// next round, a seat held for its leaver.
const OCCUPIED: u32 = 1;
const OPENED: u32 = 2;
const JOINING: u32 = 3;
const RESERVED: u32 = 4;
/// The profile id of a seat that was emptied.
const NO_PROFILE: &str = "00000000-0000-0000-0000-000000000000";
/// A line of the feed and the change of the seat it tells of are this
/// many seconds apart at most (0.4 in 185 real rounds).
pub const LINE_WINDOW: f64 = 0.5;
/// A joining player's name and ids are written within this many seconds
/// of the seat's change.
const IDENTITY_WINDOW: f64 = 2.0;
/// The body of a player who leaves alive is deleted within this many
/// seconds of the change (0.03 before it at most).
const BODY_WINDOW: f64 = 0.5;
/// A `playerLeft` line this close to the end of the recording may come
/// without a change of the seat.
const TAIL: f64 = 0.5;
/// Two samples of a body differ when a coordinate or an angle does by more
/// than this, rounded as `movement` gives them: to the millimetre and to
/// the hundredth of a degree.
const MOVED: f32 = 1e-3;

/// What a seat is, by its `PlayerSlotType`. The names are this parser's:
/// what each value means is inferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SeatState {
    /// 4: held for the player who left, whose ids it keeps.
    Reserved,
    /// 2 with `HasLeft`: emptied, for another player to take.
    Opened,
    /// 2 without `HasLeft`: nobody has sat in it, as in a custom match of
    /// fewer players than seats.
    Empty,
    /// 3: a player has connected and waits for the next round.
    Joining,
    /// 1 with `HasLeft`: vacated at some point of the match and filled
    /// again, by the same player or another.
    Refilled,
    /// A slot type other than 1 to 4.
    Unknown,
}

impl SeatState {
    /// What a seat of `slot` is. A seat a player leaves has `HasLeft`.
    fn of(slot: u32, has_left: bool) -> SeatState {
        match slot {
            RESERVED => SeatState::Reserved,
            OPENED if !has_left => SeatState::Empty,
            OPENED => SeatState::Opened,
            JOINING => SeatState::Joining,
            OCCUPIED => SeatState::Refilled,
            _ => SeatState::Unknown,
        }
    }
}

/// Where a leaver comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// The seat changed.
    Slot,
    /// Only the feed's line says so.
    FeedOnly,
}

/// How a value that is no reading of the file is known.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Basis {
    #[default]
    Inferred,
}

/// When a leaver was back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Returned {
    /// In the round they left in.
    SameRound,
    /// In the round after it, or at its start.
    NextRound,
    Later,
    #[default]
    Never,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReconnectKind {
    /// The seat was held for a leaver (4 to 3).
    Reconnect,
    /// The seat was open (2 to 3).
    Join,
}

/// A player leaving during a round.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Leaver {
    pub username: String,
    #[serde(rename = "profileID", skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    /// The header's `playerid` of the player, as their controller had it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playerid: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<usize>,
    #[serde(flatten)]
    pub when: When,
    /// What the seat became: `reserved` or `opened` (inferred names).
    /// Absent for `feedOnly`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seat: Option<SeatState>,
    /// The `PlayerSlotType` the seat got.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot_type: Option<u32>,
    /// The player had spawned and the kill feed has no death of theirs
    /// before the leave.
    pub alive_at_leave: bool,
    /// Seconds from their death to the leave.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub died_seconds_before: Option<f64>,
    /// The player had no body this round: they were waiting for the next
    /// one, or never had health.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub never_spawned: bool,
    /// The feed showed the line taken to say the connection was lost.
    pub connection_lost: bool,
    pub connection_lost_source: Basis,
    /// Seconds since the body last moved or turned, or the player last
    /// switched to a device, for a player who left alive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silent_seconds: Option<f64>,
    /// `sameRound` when the player reconnected before the recording
    /// ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returned: Option<Returned>,
    pub source: Source,
    /// The frame the seat changed in, or the line was written in.
    #[serde(skip)]
    pub frame: Option<u32>,
    #[serde(skip)]
    pub controller: Option<u32>,
}

/// A player taking a seat during a round. They play from the next round.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reconnect {
    pub username: String,
    #[serde(rename = "profileID", skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playerid: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<usize>,
    #[serde(flatten)]
    pub when: When,
    pub kind: ReconnectKind,
    /// Seconds since the same player left the seat, when that is in this
    /// recording.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub away_seconds: Option<f64>,
    /// The seat was already reserved or opened when the recording
    /// started.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub left_before_recording: bool,
    /// A reconnect: whether the `playerid` differs from the one the seat
    /// held.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_playerid: Option<bool>,
    #[serde(skip)]
    pub frame: Option<u32>,
    #[serde(skip)]
    pub controller: u32,
}

/// A seat that is not plainly a player's when the recording starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Seat {
    /// The name the controller holds: the leaver's for a reserved or
    /// opened seat, none for an empty one.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub username: String,
    /// Absent for an opened seat, whose ids were wiped.
    #[serde(rename = "profileID", skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<usize>,
    pub slot_type: u32,
    pub seat: SeatState,
    pub has_left: bool,
    #[serde(skip)]
    pub playerid: Option<u64>,
    #[serde(skip)]
    pub controller: u32,
}

/// Who left, who came and which seats were not plain, in one round.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Presence {
    pub leavers: Vec<Leaver>,
    pub reconnects: Vec<Reconnect>,
    pub seats: Vec<Seat>,
}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    pub presence: Presence,
    /// What did not fit, for `decodeStatus`.
    pub warnings: Vec<String>,
}

/// What the rest of the round says of its players, by their index in the
/// header.
pub(crate) struct Context<'a> {
    /// The feed's lines that are no kills.
    pub lines: &'a [SystemMessage],
    /// `(victim, seconds since the recording started)` of every death.
    pub deaths: &'a [(&'a str, Option<f64>)],
    /// Players without health when action started.
    pub down_at_start: &'a [String],
    /// The frame a player's body was deleted in. `None` when the body is
    /// not known.
    pub body_deleted: &'a dyn Fn(usize) -> Option<Option<u32>>,
    /// The last frame up to the one given in which a player gave input.
    pub last_input: &'a dyn Fn(usize, u32) -> Option<u32>,
}

/// Who a controller's player is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Identity {
    name: String,
    /// `None` for the zeros of an emptied seat.
    profile: Option<String>,
    /// `None` for ff*8.
    playerid: Option<u64>,
    /// The team's object.
    team: Option<u32>,
}

/// An object some property of a controller was written to.
#[derive(Clone, Debug, Default)]
struct Controller {
    /// Whether an `OnlinePlayerID` was written: other classes share some
    /// of the hashes.
    known: bool,
    slot: Option<u32>,
    has_left: Option<bool>,
    identity: Identity,
    /// The identity before the block being read wrote to it.
    before: Identity,
    touched: usize,
    /// Slot, `HasLeft` and identity when the recording started.
    start: Option<(Option<u32>, Option<bool>, Identity)>,
}

/// A slot type written that differs from the one before.
#[derive(Clone, Debug, PartialEq)]
struct Change {
    controller: u32,
    frame: u32,
    time: Option<f64>,
    /// Offset of the record in the data.
    at: usize,
    from: Option<u32>,
    to: u32,
    before: Identity,
    /// The identity once [`IDENTITY_WINDOW`] has passed.
    after: Identity,
}

/// What the blocks read wrote to controllers.
#[derive(Debug, Default)]
struct Seats {
    controllers: HashMap<u32, Controller>,
    /// The objects in the order first written.
    order: Vec<u32>,
    changes: Vec<Change>,
    /// Blocks read so far.
    blocks: usize,
    /// Values of a size the property never has.
    malformed: usize,
    /// Times `HasLeft` went from 1 to 0 during the recording.
    cleared: usize,
}

impl Seats {
    /// Reads the records of one snapshot or frame record. `base` is where
    /// `block` starts in the data, `time` the frame's seconds.
    fn read(&mut self, block: &[u8], base: usize, frame: Option<u32>, time: Option<f64>) {
        self.blocks += 1;
        // Each run of records names its object before writing to it.
        let mut current: Option<u32> = None;
        let mut writes: Vec<(u32, Hash, usize, usize, usize)> = Vec::new();
        for_each_record(block, |at, r| match r {
            Record::Set(obj, hash, from, to) => {
                current = Some(obj);
                writes.push((obj, hash, at, from, to));
            }
            // A `26` is an array element, which none of these are.
            Record::Prop(hash, from, to) if block.get(at) == Some(&0x22) => {
                if let Some(obj) = current {
                    writes.push((obj, hash, at, from, to));
                }
            }
            Record::ParentChild(parent, ..) => current = Some(parent),
            _ => {}
        });
        for (obj, hash, at, from, to) in writes {
            if let Some(value) = block.get(from..to) {
                self.write(obj, hash, value, base + at, frame, time);
            }
        }
        if frame.is_none() {
            for c in self.controllers.values_mut() {
                c.start = Some((c.slot, c.has_left, c.identity.clone()));
            }
        }
    }

    fn write(
        &mut self,
        obj: u32,
        hash: Hash,
        value: &[u8],
        at: usize,
        frame: Option<u32>,
        time: Option<f64>,
    ) {
        if ![
            SLOT_TYPE, HAS_LEFT, PLAYER_ID, NAME, PROFILE_ID, PLATFORM, TEAM,
        ]
        .contains(&hash)
        {
            return;
        }
        let c = self.controllers.entry(obj).or_insert_with(|| {
            self.order.push(obj);
            Controller::default()
        });
        if c.touched != self.blocks {
            c.touched = self.blocks;
            c.before = c.identity.clone();
        }
        let text = || String::from_utf8_lossy(value).into_owned();
        match hash {
            SLOT_TYPE => {
                let Some(slot) = value.try_into().ok().map(u32::from_le_bytes) else {
                    self.malformed += 1;
                    return;
                };
                if let (Some(frame), true) = (frame, c.slot != Some(slot)) {
                    self.changes.push(Change {
                        controller: obj,
                        frame,
                        time,
                        at,
                        from: c.slot,
                        to: slot,
                        before: c.before.clone(),
                        after: c.identity.clone(),
                    });
                }
                c.slot = Some(slot);
            }
            HAS_LEFT => {
                let left = match value {
                    [0] => false,
                    [1] => true,
                    _ => {
                        self.malformed += 1;
                        return;
                    }
                };
                if frame.is_some() && c.has_left == Some(true) && !left {
                    self.cleared += 1;
                }
                c.has_left = Some(left);
            }
            PLAYER_ID => {
                let Some(id) = value.try_into().ok().map(u64::from_le_bytes) else {
                    self.malformed += 1;
                    return;
                };
                c.known = true;
                c.identity.playerid = Some(id).filter(|&id| id != u64::MAX && id != 0);
            }
            NAME => c.identity.name = text(),
            PROFILE_ID => {
                c.identity.profile = Some(text()).filter(|p| !p.is_empty() && p != NO_PROFILE);
            }
            // The platform of an emptied seat is 12; nothing here needs it.
            PLATFORM => self.malformed += usize::from(value.len() != 4),
            // Other classes hold a `TeamVM` of another size.
            TEAM => {
                if let Ok(team) = <[u8; 8]>::try_from(value) {
                    c.identity.team = Some(u64::from_le_bytes(team) as u32).filter(|&t| t != 0);
                }
            }
            _ => {}
        }
        // The name and ids of a player who takes a seat follow the change.
        let Some(frame) = frame else { return };
        for change in self.changes.iter_mut().filter(|ch| ch.controller == obj) {
            let near = match (change.time, time) {
                (Some(then), Some(now)) => now - then < IDENTITY_WINDOW,
                _ => frame == change.frame,
            };
            if near {
                change.after = c.identity.clone();
            }
        }
    }

    /// The controllers, in the order first written.
    fn seats(&self) -> impl Iterator<Item = (u32, &Controller)> {
        let known = |obj: &u32| Some((*obj, self.controllers.get(obj).filter(|c| c.known)?));
        self.order.iter().filter_map(known)
    }
}

fn millis(t: f64) -> f64 {
    (t * 1000.0).round() / 1000.0
}

fn hundredths(t: f64) -> f64 {
    (t * 100.0).round() / 100.0
}

/// Reads the seats from the blocks of the state stream that write one.
/// `writes` are the offsets at which the marker scan found a
/// `PlayerSlotType` or a `HasLeft`, rising: only the opening snapshot, the
/// frames that hold one and the frames just after are read.
pub(crate) fn decode(input: &Input, writes: &[usize], context: &Context) -> Decoded {
    let mut seats = Seats::default();
    let mut until: Option<f64> = None;
    for (start, end, frame) in input.blocks(STATE_STREAM) {
        let time = input.clock.seconds(frame);
        let written =
            (writes.get(writes.partition_point(|&w| w < start))).is_some_and(|&w| w < end);
        let follows = time.zip(until).is_some_and(|(t, until)| t < until);
        if frame.is_some() && !written && !follows {
            continue;
        }
        if written && frame.is_some() {
            until = time.map(|t| t + IDENTITY_WINDOW);
        }
        if let Some(block) = input.data.get(start..end) {
            seats.read(block, start, frame, time);
        }
    }
    let end = input.clock.frame_times.last().copied();
    resolve(&seats, input.players, input.clock, context, end)
}

/// A line of the feed with whether a change of a seat took it.
struct Line<'a> {
    line: &'a SystemMessage,
    used: bool,
}

/// The line of one of `kinds` that names `username`, not yet taken, whose
/// time less `time` is within `window`: the nearest one.
fn take<'a>(
    lines: &mut [Line<'a>],
    kinds: &[Kind],
    username: &str,
    time: Option<f64>,
    window: (f64, f64),
) -> Option<&'a SystemMessage> {
    let time = time?;
    let apart = |l: &Line| Some(l.line.when.recording_time? - millis(time));
    let fits = |l: &Line| {
        let named = username.is_empty() || l.line.username.as_deref() == Some(username);
        let near = apart(l).is_some_and(|d| d >= window.0 - 1e-9 && d <= window.1 + 1e-9);
        !l.used && kinds.contains(&l.line.kind) && named && near
    };
    let nearest = (lines.iter_mut().filter(|l| fits(l))).min_by(|a, b| {
        (apart(a).map(f64::abs))
            .partial_cmp(&apart(b).map(f64::abs))
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    nearest.used = true;
    Some(nearest.line)
}

fn resolve(
    seats: &Seats,
    players: &[Player],
    clock: &Clock,
    context: &Context,
    end: Option<f64>,
) -> Decoded {
    let mut out = Decoded::default();
    // The header's player a controller held: by profile id, else by name.
    let player_of = |id: &Identity| {
        let profile = |p: &Player| id.profile.as_deref().is_some_and(|x| x == p.profile_id);
        let name = |p: &Player| !id.name.is_empty() && p.username == id.name;
        (players.iter().position(profile)).or_else(|| players.iter().position(name))
    };
    // A seat nobody of the header sits in is on the team of the seats that
    // share its team object.
    let mut teams: HashMap<u32, usize> = HashMap::new();
    for (_, c) in seats.seats() {
        let id = c.start.as_ref().map_or(&c.identity, |s| &s.2);
        let team = player_of(id)
            .and_then(|i| players.get(i))
            .map(|p| p.team_index);
        if let (Some(object), Some(team)) = (id.team, team.filter(|&t| t < 2)) {
            teams.entry(object).or_insert(team);
        }
    }
    let team_of = |id: &Identity| {
        let listed = player_of(id)
            .and_then(|i| players.get(i))
            .map(|p| p.team_index);
        (listed.filter(|&t| t < 2)).or_else(|| teams.get(&id.team?).copied())
    };
    let seconds = |frame: u32| clock.seconds(Some(frame));
    // Whether a player was alive at `time`, when they died before it, and
    // whether they never had a body.
    let life = |name: &str, spawned: bool, time: Option<f64>| {
        let death = (context.deaths.iter()).find(|d| d.0 == name && !name.is_empty());
        let died = death.is_some_and(|d| d.1.zip(time).is_none_or(|(d, t)| d <= t));
        let down = context.down_at_start.iter().any(|d| d == name);
        let before = death
            .filter(|_| died)
            .and_then(|d| Some(hundredths(time? - d.1?)));
        let never = !spawned || (down && death.is_none());
        (spawned && !died && !down, before, never)
    };

    for (controller, c) in seats.seats() {
        let Some((slot, has_left, id)) = &c.start else {
            continue;
        };
        let has_left = *has_left == Some(true);
        let Some(slot) = (*slot).filter(|&s| s != OCCUPIED || has_left) else {
            continue;
        };
        if SeatState::of(slot, has_left) == SeatState::Unknown {
            out.warnings
                .push(format!("a seat starts with slot type {slot}"));
        }
        out.presence.seats.push(Seat {
            username: id.name.clone(),
            profile_id: id.profile.clone(),
            team: team_of(id),
            slot_type: slot,
            seat: SeatState::of(slot, has_left),
            has_left,
            playerid: id.playerid,
            controller,
        });
    }

    let mut lines: Vec<Line> = (context.lines.iter())
        .map(|line| Line { line, used: false })
        .collect();
    let (mut unannounced, mut mismatched) = (0, 0);
    let mut unlisted: Vec<String> = Vec::new();
    // The last leave of each controller: who left, and when.
    let mut last: HashMap<u32, (Identity, Option<f64>)> = HashMap::new();
    // Other classes share the hash: only a controller's slot is a seat.
    let known = |c: &&Change| (seats.controllers.get(&c.controller)).is_some_and(|c| c.known);
    for change in seats.changes.iter().filter(known) {
        let time = change.time.or_else(|| seconds(change.frame));
        let when = clock.when(change.at, Some(change.frame));
        let listed = matches!(
            (change.from, change.to),
            (Some(OCCUPIED), RESERVED | OPENED)
                | (Some(JOINING), OPENED)
                | (Some(RESERVED | OPENED), JOINING)
        );
        if !listed {
            let from = change.from.map_or("none".to_owned(), |f| f.to_string());
            let said = format!("{from} to {}", change.to);
            if !unlisted.contains(&said) {
                unlisted.push(said);
            }
        }
        match (change.from, change.to) {
            // A player leaves the seat they were in or waited in.
            (Some(from @ (OCCUPIED | JOINING)), to @ (RESERVED | OPENED)) => {
                let id = &change.before;
                let player = player_of(id);
                let (alive, died, never) =
                    life(&id.name, from == OCCUPIED && player.is_some(), time);
                let lost = take(
                    &mut lines,
                    &[Kind::ConnectionLost],
                    &id.name,
                    time,
                    (-LINE_WINDOW, 0.0),
                );
                let line = take(
                    &mut lines,
                    &[Kind::PlayerLeft],
                    &id.name,
                    time,
                    (0.0, LINE_WINDOW),
                );
                unannounced += usize::from(line.is_none());
                // The body goes when its player leaves alive, and stays
                // when they were dead.
                let body = player.and_then(context.body_deleted);
                let gone = body.map(|deleted| {
                    let at = deleted.and_then(seconds);
                    at.zip(time)
                        .is_some_and(|(at, t)| (at - t).abs() <= BODY_WINDOW)
                });
                mismatched += usize::from(gone.is_some_and(|gone| gone != alive));
                let input = player
                    .filter(|_| alive)
                    .and_then(|p| (context.last_input)(p, change.frame));
                let silent = input
                    .and_then(seconds)
                    .zip(time)
                    .map(|(at, t)| hundredths(t - at));
                last.insert(change.controller, (id.clone(), time));
                out.presence.leavers.push(Leaver {
                    username: id.name.clone(),
                    profile_id: id.profile.clone(),
                    playerid: id.playerid,
                    team: team_of(id),
                    when,
                    seat: Some(SeatState::of(to, true)),
                    slot_type: Some(to),
                    alive_at_leave: alive,
                    died_seconds_before: died,
                    never_spawned: never,
                    connection_lost: lost.is_some(),
                    connection_lost_source: Basis::Inferred,
                    silent_seconds: silent,
                    returned: None,
                    source: Source::Slot,
                    frame: Some(change.frame),
                    controller: Some(change.controller),
                });
            }
            // A player takes a seat that was held or open.
            (Some(from @ (RESERVED | OPENED)), JOINING) => {
                let id = &change.after;
                let kind = if from == RESERVED {
                    ReconnectKind::Reconnect
                } else {
                    ReconnectKind::Join
                };
                let kinds = [Kind::PlayerReconnected, Kind::PlayerJoined];
                let line = take(
                    &mut lines,
                    &kinds,
                    &id.name,
                    time,
                    (-LINE_WINDOW, LINE_WINDOW),
                );
                unannounced += usize::from(line.is_none());
                // Whose seat it was: the leaver of this recording, else
                // the one a reserved seat started with.
                let start = c_start(seats, change.controller);
                let left = last.get(&change.controller);
                let held = left.map(|l| &l.0).or(start.filter(|_| from == RESERVED));
                let same = held.is_some_and(|h| h.profile.is_some() && h.profile == id.profile);
                let away = left
                    .filter(|_| same)
                    .and_then(|l| Some(hundredths(time? - l.1?)));
                let ids = held
                    .filter(|_| same)
                    .and_then(|h| h.playerid.zip(id.playerid));
                let said = line.and_then(|l| l.username.clone()).unwrap_or_default();
                out.presence.reconnects.push(Reconnect {
                    username: if id.name.is_empty() {
                        said
                    } else {
                        id.name.clone()
                    },
                    profile_id: id.profile.clone(),
                    playerid: id.playerid,
                    team: team_of(id),
                    when,
                    kind,
                    away_seconds: away,
                    left_before_recording: left.is_none(),
                    new_playerid: ids.map(|(was, now)| was != now),
                    frame: Some(change.frame),
                    controller: change.controller,
                });
            }
            _ => {}
        }
    }

    // What the feed said and no seat did.
    let mut unseated = [0usize; 3];
    for l in lines.iter().filter(|l| !l.used) {
        let line = l.line;
        match line.kind {
            Kind::PlayerLeft => {
                let name = line.username.clone().unwrap_or_default();
                let time = line.when.recording_time;
                let id = Identity {
                    name: name.clone(),
                    profile: line.profile_id.clone(),
                    ..Identity::default()
                };
                let player = player_of(&id);
                let (alive, died, never) = life(&name, player.is_some(), time);
                let lost = (context.lines.iter()).any(|c| {
                    let before = time.zip(c.when.recording_time).map(|(t, at)| t - at);
                    c.kind == Kind::ConnectionLost
                        && c.username.as_deref() == Some(name.as_str())
                        && before.is_some_and(|d| (0.0..=2.0 * LINE_WINDOW).contains(&d))
                });
                // The recording can end before the seat is written.
                let tail = time.zip(end).is_some_and(|(t, end)| end - t <= TAIL);
                unseated[0] += usize::from(!tail);
                let input = (player.filter(|_| alive).zip(line.frame))
                    .and_then(|(p, frame)| (context.last_input)(p, frame));
                let silent = input
                    .and_then(seconds)
                    .zip(time)
                    .map(|(at, t)| hundredths(t - at));
                out.presence.leavers.push(Leaver {
                    username: name,
                    profile_id: id.profile.clone(),
                    playerid: player
                        .and_then(|p| players.get(p))
                        .map(|p| p.id)
                        .filter(|&id| id != 0),
                    team: team_of(&id),
                    when: line.when.clone(),
                    seat: None,
                    slot_type: None,
                    alive_at_leave: alive,
                    died_seconds_before: died,
                    never_spawned: never,
                    connection_lost: lost,
                    connection_lost_source: Basis::Inferred,
                    silent_seconds: silent,
                    returned: None,
                    source: Source::FeedOnly,
                    frame: line.frame,
                    controller: None,
                });
            }
            Kind::PlayerJoined | Kind::PlayerReconnected => unseated[1] += 1,
            // The line of a leaver the feed alone tells of is theirs.
            Kind::ConnectionLost => unseated[2] += 1,
            _ => {}
        }
    }
    let lost_with_line = (out.presence.leavers.iter())
        .filter(|l| l.source == Source::FeedOnly && l.connection_lost)
        .count();
    unseated[2] = unseated[2].saturating_sub(lost_with_line);
    out.presence.leavers.sort_by_key(|l| l.frame);

    // A leaver is back when a later change gives a seat to their profile.
    let Presence {
        leavers,
        reconnects,
        ..
    } = &mut out.presence;
    for l in leavers.iter_mut().filter(|l| l.profile_id.is_some()) {
        let back = |r: &Reconnect| r.profile_id == l.profile_id && r.frame > l.frame;
        if reconnects.iter().any(back) {
            l.returned = Some(Returned::SameRound);
        }
    }

    let notes = [
        (unannounced, "changes of a seat have no line of the feed"),
        (
            unseated[0],
            "playerLeft lines come with no change of a seat, before the end of the recording",
        ),
        (
            unseated[1],
            "playerJoined or playerReconnected lines come with no change of a seat",
        ),
        (
            unseated[2],
            "connectionLost lines are not followed by a leave of the player they name",
        ),
        (
            mismatched,
            "leavers are alive by the kill feed and their body stays, or dead and it goes",
        ),
        (seats.cleared, "times HasLeft went back to 0"),
        (
            seats.malformed,
            "slot types, flags or ids of controllers of another size than theirs",
        ),
    ];
    for (count, what) in notes {
        if count > 0 {
            out.warnings.push(format!("{count} {what}"));
        }
    }
    if !unlisted.is_empty() {
        out.warnings.push(format!(
            "slot type changes not seen before: {}",
            unlisted.join(", ")
        ));
    }
    out
}

/// The identity a controller started the recording with.
fn c_start(seats: &Seats, controller: u32) -> Option<&Identity> {
    let c = seats.controllers.get(&controller)?;
    c.start.as_ref().map(|s| &s.2)
}

/// The last frame up to `frame` in which `player` gave input: their body
/// moved or turned, or they switched to a drone or camera. A body that
/// never did counts from its first sample.
pub(crate) fn last_input(
    stream: &crate::movement::Stream,
    views: &[ViewChange],
    player: &Player,
    frame: u32,
) -> Option<u32> {
    let body = player.entities.as_ref().and_then(|e| e.movement);
    let body = body.map(u64::from);
    let mine = |t: &&crate::movement::Track| {
        (player.id != 0 && t.player_id == Some(player.id)) || Some(t.object) == body
    };
    let mut last: Option<u32> = None;
    let mut first: Option<u32> = None;
    for track in stream.tracks.iter().filter(mine) {
        let mut before: Option<[f32; 5]> = None;
        for s in track.samples.iter().filter(|s| s.shown) {
            let [x, y, z] = s.position.map(|v| (v * 1000.0).round() / 1000.0);
            let angle = |a: f32| (a * 100.0).round() / 100.0;
            let now = [x, y, z, angle(s.yaw), angle(s.pitch)];
            let was = before.replace(now);
            // The snapshot's sample is where the body starts.
            let Some(at) = s.frame else { continue };
            if at > frame {
                break;
            }
            first = Some(first.map_or(at, |f| f.min(at)));
            let moved = |was: [f32; 5]| (was.iter().zip(&now)).any(|(a, b)| (a - b).abs() > MOVED);
            if was.is_some_and(moved) {
                last = Some(last.map_or(at, |l| l.max(at)));
            }
        }
    }
    let switched = (views.iter())
        .filter(|v| v.player_id == player.id && player.id != 0 && v.view != 0)
        .filter_map(|v| v.frame.filter(|&f| f > 0 && f <= frame))
        .max();
    last.max(switched).or(first)
}

/// What a player's leaving and returning looked like, one step at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EventKind {
    /// Left during the round.
    Left,
    /// Took their reserved seat again during the round.
    Reconnected,
    /// Took an open seat during the round, after leaving earlier.
    Joined,
    /// Not in the round's header.
    AbsentAtStart,
    /// In the round's header again.
    BackAtStart,
}

/// One step of a player's [`PlayerPresence`].
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// The round's number, from 1.
    pub round: u32,
    #[serde(rename = "type")]
    pub kind: EventKind,
    /// When, for what happened during a round.
    #[serde(flatten)]
    pub when: Option<When>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_lost: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alive_at_leave: Option<bool>,
    /// A return: whether the `playerid` differs from the one before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_playerid: Option<bool>,
}

/// What a leave is likely to have been, read from what was observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Likely {
    /// The feed showed the line taken to say the connection was lost.
    ConnectionLost,
    /// The player came back with another `playerid`: the game was started
    /// again.
    GameRestarted,
    /// The player came back with the same `playerid`: the game kept
    /// running.
    GameKeptRunning,
}

/// A player who left at some point of a match.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerPresence {
    pub username: String,
    #[serde(rename = "profileID", skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<usize>,
    pub events: Vec<Event>,
    /// Rounds whose header lacks the player while a seat was reserved for
    /// them.
    pub rounds_missed: u32,
    /// When the player was back after they last left.
    pub returned: Returned,
    /// Whether they came back with another `playerid`, the last time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_playerid: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub likely: Vec<Likely>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub likely_source: Option<Basis>,
}

/// A player being followed through a match.
#[derive(Default)]
struct Followed {
    entry: PlayerPresence,
    /// The round they have been away since: the one they left in, or the
    /// one before the first that started without them.
    away: Option<u32>,
    /// The `playerid` they had before.
    playerid: Option<u64>,
    /// Whether the absence was said already.
    absent: bool,
}

impl Followed {
    fn push(&mut self, round: u32, kind: EventKind) -> &mut Event {
        self.entry.events.push(Event {
            round,
            kind,
            when: None,
            connection_lost: None,
            alive_at_leave: None,
            new_playerid: None,
        });
        self.entry.events.last_mut().expect("just pushed")
    }

    /// The player is back in `round` with `playerid`.
    fn back(&mut self, round: u32, kind: EventKind, playerid: Option<u64>) -> &mut Event {
        let new = self.playerid.zip(playerid).map(|(was, now)| was != now);
        self.entry.returned = match self.away {
            Some(since) if round <= since => Returned::SameRound,
            Some(since) if round == since + 1 => Returned::NextRound,
            _ => Returned::Later,
        };
        self.entry.new_playerid = new;
        self.playerid = playerid.or(self.playerid);
        self.away = None;
        self.absent = false;
        let event = self.push(round, kind);
        event.new_playerid = new;
        event
    }
}

/// The key a player is followed by: the profile id, else the name.
fn key(profile: Option<&str>, name: &str) -> String {
    match profile.filter(|p| !p.is_empty()) {
        Some(profile) => profile.to_owned(),
        None => format!("name:{name}"),
    }
}

/// Every player who left at some point of the match, with what they did
/// round by round. A player who took a seat someone else left is none.
pub fn rollup<'a>(rounds: impl IntoIterator<Item = &'a Round>) -> Vec<PlayerPresence> {
    let mut followed: Vec<(String, Followed)> = Vec::new();
    fn find<'f>(list: &'f mut [(String, Followed)], key: &str) -> Option<&'f mut Followed> {
        list.iter_mut().find(|f| f.0 == key).map(|f| &mut f.1)
    }
    fn follow<'f>(
        list: &'f mut Vec<(String, Followed)>,
        key: String,
        username: &str,
        profile: Option<&str>,
        team: Option<usize>,
    ) -> &'f mut Followed {
        let at = match list.iter().position(|f| f.0 == key) {
            Some(at) => at,
            None => {
                let entry = PlayerPresence {
                    username: username.to_owned(),
                    profile_id: profile.map(str::to_owned),
                    team,
                    ..PlayerPresence::default()
                };
                list.push((
                    key,
                    Followed {
                        entry,
                        ..Followed::default()
                    },
                ));
                list.len() - 1
            }
        };
        &mut list[at].1
    }

    for round in rounds {
        let Some(presence) = &round.presence else {
            continue;
        };
        let number = round.header.round_number + 1;
        let players = &round.header.players;
        let listed = |profile: Option<&str>, name: &str| {
            let by_profile = |p: &&Player| profile.is_some_and(|x| x == p.profile_id);
            let by_name = |p: &&Player| profile.is_none() && p.username == name;
            (players.iter().find(by_profile)).or_else(|| players.iter().find(by_name))
        };

        // The start: who a seat is held for, who is back, who is not.
        for seat in (presence.seats.iter()).filter(|s| s.seat == SeatState::Reserved) {
            let profile = seat.profile_id.as_deref();
            if listed(profile, &seat.username).is_some() {
                continue;
            }
            let key = key(profile, &seat.username);
            let f = follow(&mut followed, key, &seat.username, profile, seat.team);
            f.away.get_or_insert(number.saturating_sub(1));
            f.playerid = f.playerid.or(seat.playerid);
            f.entry.rounds_missed += 1;
        }
        for (_, f) in followed.iter_mut().filter(|f| f.1.away.is_some()) {
            let profile = f.entry.profile_id.clone();
            match listed(profile.as_deref(), &f.entry.username) {
                Some(p) => {
                    let id = Some(p.id).filter(|&id| id != 0);
                    f.back(number, EventKind::BackAtStart, id);
                }
                None if !f.absent => {
                    f.absent = true;
                    f.push(number, EventKind::AbsentAtStart);
                }
                None => {}
            }
        }

        // The round, in the order things happened.
        enum Step<'s> {
            Left(&'s Leaver),
            Took(&'s Reconnect),
        }
        let mut steps: Vec<(Option<u32>, Step)> = (presence.leavers.iter())
            .map(|l| (l.frame, Step::Left(l)))
            .chain(presence.reconnects.iter().map(|r| (r.frame, Step::Took(r))))
            .collect();
        steps.sort_by_key(|s| s.0);
        for (_, step) in steps {
            match step {
                Step::Left(l) => {
                    let profile = l.profile_id.as_deref();
                    let key = key(profile, &l.username);
                    // Someone who took a seat and left it again before
                    // their first round was never a player of the match.
                    let played = listed(profile, &l.username).is_some();
                    if l.never_spawned && !played && find(&mut followed, &key).is_none() {
                        continue;
                    }
                    let f = follow(&mut followed, key, &l.username, profile, l.team);
                    let id = listed(profile, &l.username).map(|p| p.id);
                    f.playerid = l.playerid.or(id.filter(|&id| id != 0)).or(f.playerid);
                    f.away = Some(number);
                    f.absent = false;
                    f.entry.returned = Returned::Never;
                    f.entry.new_playerid = None;
                    let event = f.push(number, EventKind::Left);
                    event.when = Some(l.when.clone());
                    event.connection_lost = Some(l.connection_lost);
                    event.alive_at_leave = Some(l.alive_at_leave);
                }
                Step::Took(r) => {
                    let profile = r.profile_id.as_deref();
                    let key = key(profile, &r.username);
                    // Someone taking a seat they never left is no leaver.
                    let Some(f) = find(&mut followed, &key).filter(|f| f.away.is_some()) else {
                        continue;
                    };
                    let kind = match r.kind {
                        ReconnectKind::Reconnect => EventKind::Reconnected,
                        ReconnectKind::Join => EventKind::Joined,
                    };
                    f.back(number, kind, r.playerid).when = Some(r.when.clone());
                }
            }
        }
    }

    let mut out: Vec<PlayerPresence> = Vec::new();
    for (_, f) in followed {
        let mut entry = f.entry;
        if entry.events.is_empty() {
            continue;
        }
        let events = &entry.events;
        let returns = |new: bool| events.iter().any(|e| e.new_playerid == Some(new));
        let signs = [
            (
                events.iter().any(|e| e.connection_lost == Some(true)),
                Likely::ConnectionLost,
            ),
            (returns(true), Likely::GameRestarted),
            (returns(false), Likely::GameKeptRunning),
        ];
        entry.likely = signs.iter().filter(|s| s.0).map(|s| s.1).collect();
        entry.likely_source = (!entry.likely.is_empty()).then_some(Basis::Inferred);
        out.push(entry);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::details::Phase;
    use crate::timeline::Timeline;

    const SEAT: u32 = 0xF000_0100;
    const TEAMS: [u64; 2] = [0xF000_0010, 0xF000_0020];
    /// The hash of the link that opens a controller's run when it is sent
    /// again (`PlayerLifeVM`).
    const LIFE: Hash = [0x41, 0x54, 0xDC, 0xC4];

    /// `23 <obj> 00000000 <hash> <size> <value>`.
    fn set(d: &mut Vec<u8>, obj: u32, hash: Hash, value: &[u8]) {
        d.push(0x23);
        d.extend(obj.to_le_bytes());
        d.extend([0; 4]);
        d.extend(hash);
        d.push(value.len() as u8);
        d.extend(value);
    }

    /// `22 <hash> <size> <value>`.
    fn prop(d: &mut Vec<u8>, hash: Hash, value: &[u8]) {
        d.push(0x22);
        d.extend(hash);
        d.push(value.len() as u8);
        d.extend(value);
    }

    /// `1b <parent> 00000000 <field> <child> 00000000 <class>`: how a
    /// controller sent again starts.
    fn resend(d: &mut Vec<u8>, controller: u32) {
        d.push(0x1B);
        d.extend(controller.to_le_bytes());
        d.extend([0; 4]);
        d.extend(LIFE);
        d.extend((controller + 0x1000).to_le_bytes());
        d.extend([0; 4]);
        d.extend([9, 9, 9, 9]);
    }

    fn profile(n: usize) -> String {
        format!("{n:08}-aaaa-bbbb-cccc-dddddddddddd")
    }

    /// Controller `n` as the snapshot writes it: `p<n>` of team `n / 2`.
    fn seat(n: usize, slot: u32, has_left: u8) -> Vec<u8> {
        let mut d = vec![];
        set(&mut d, SEAT + n as u32, NAME, format!("p{n}").as_bytes());
        prop(&mut d, PROFILE_ID, profile(n).as_bytes());
        prop(&mut d, PLAYER_ID, &(100 + n as u64).to_le_bytes());
        prop(&mut d, TEAM, &TEAMS[n / 2].to_le_bytes());
        prop(&mut d, SLOT_TYPE, &slot.to_le_bytes());
        prop(&mut d, HAS_LEFT, &[has_left]);
        d
    }

    /// Controller `n` sent again with a new slot type.
    fn leave(n: usize, slot: u32) -> Vec<u8> {
        let mut d = vec![];
        resend(&mut d, SEAT + n as u32);
        prop(&mut d, SLOT_TYPE, &slot.to_le_bytes());
        if slot == OPENED {
            prop(&mut d, PLAYER_ID, &[0xFF; 8]);
            prop(&mut d, PROFILE_ID, NO_PROFILE.as_bytes());
            prop(&mut d, PLATFORM, &12u32.to_le_bytes());
        }
        prop(&mut d, HAS_LEFT, &[1]);
        d
    }

    /// Someone taking seat `n`: the name comes empty at first.
    fn arrive(n: usize, profile: &str, playerid: u64) -> Vec<u8> {
        let mut d = vec![];
        resend(&mut d, SEAT + n as u32);
        prop(&mut d, SLOT_TYPE, &JOINING.to_le_bytes());
        prop(&mut d, NAME, &[]);
        prop(&mut d, PROFILE_ID, profile.as_bytes());
        prop(&mut d, PLAYER_ID, &playerid.to_le_bytes());
        d
    }

    fn name(n: usize, name: &str) -> Vec<u8> {
        let mut d = vec![];
        set(&mut d, SEAT + n as u32, NAME, name.as_bytes());
        d
    }

    /// A round of four players, `p0` and `p1` against `p2` and `p3`.
    /// Frame `n` is `n / 10` seconds into the recording.
    struct Fixture {
        players: Vec<Player>,
        seats: Seats,
        lines: Vec<SystemMessage>,
        deaths: Vec<(&'static str, Option<f64>)>,
        down: Vec<String>,
        /// The frame each player's body was deleted in.
        bodies: Vec<Option<u32>>,
        /// The frame each player last gave input in.
        inputs: Vec<Option<u32>>,
    }

    impl Fixture {
        /// `slots` are the slot type and `HasLeft` each seat starts with;
        /// a player whose seat is not 1 is not in the header.
        fn new(slots: [(u32, u8); 4]) -> Self {
            let players = (0..4usize)
                .filter(|&i| slots[i].0 == OCCUPIED)
                .map(|i| Player {
                    id: 100 + i as u64,
                    username: format!("p{i}"),
                    profile_id: profile(i),
                    team_index: i / 2,
                    ..Player::default()
                })
                .collect();
            let mut seats = Seats::default();
            let mut d = vec![];
            for (n, (slot, has_left)) in slots.into_iter().enumerate() {
                d.extend(seat(n, slot, has_left));
            }
            seats.read(&d, 0, None, None);
            Fixture {
                players,
                seats,
                lines: Vec::new(),
                deaths: Vec::new(),
                down: Vec::new(),
                bodies: vec![None; 4],
                inputs: vec![None; 4],
            }
        }

        fn plain() -> Self {
            Fixture::new([(OCCUPIED, 0); 4])
        }

        fn frame(&mut self, frame: u32, block: &[u8]) {
            let time = Some(f64::from(frame) / 10.0);
            (self.seats).read(block, 1000 * frame as usize, Some(frame), time);
        }

        /// A line of the feed that names `username`, written in `frame`.
        fn line(&mut self, frame: u32, kind: Kind, username: &str) {
            let player = self.players.iter().find(|p| p.username == username);
            self.lines.push(SystemMessage {
                kind,
                kind_source: None,
                message_id: None,
                text: None,
                args: Vec::new(),
                username: Some(username.to_owned()),
                profile_id: player.map(|p| p.profile_id.clone()),
                background_color: 0,
                when: When {
                    recording_time: Some(f64::from(frame) / 10.0),
                    ..When::default()
                },
                frame: Some(frame),
                offset: 1000 * frame as usize + 500,
            });
        }

        fn resolve(&self) -> Decoded {
            let times: Vec<f64> = (0..3000).map(|f| f64::from(f) / 10.0).collect();
            // A clock that reads 0:45, 0:00, 3:00 and 2:59.
            let recording = vec![Some(0.0), Some(45.0), Some(46.0), Some(47.0)];
            let mut timeline =
                Timeline::resolve(&[45.0, 0.0, 180.0, 179.0], None, None, &recording);
            timeline.recording = recording;
            let clock = Clock {
                timeline: &timeline,
                reading_offsets: &[0, 450_000, 460_000, 470_000],
                frame_times: &times,
            };
            let body = |player: usize| Some(self.bodies.get(player).copied().flatten());
            let input = |player: usize, frame: u32| {
                let at = self.inputs.get(player).copied().flatten();
                at.filter(|&at| at <= frame)
            };
            let context = Context {
                lines: &self.lines,
                deaths: &self.deaths,
                down_at_start: &self.down,
                body_deleted: &body,
                last_input: &input,
            };
            resolve(&self.seats, &self.players, &clock, &context, Some(299.9))
        }
    }

    #[test]
    fn a_plain_round_has_nothing() {
        let mut f = Fixture::plain();
        // A controller sent again with nothing changed is no event.
        let mut d = vec![];
        resend(&mut d, SEAT + 1);
        prop(&mut d, SLOT_TYPE, &OCCUPIED.to_le_bytes());
        prop(&mut d, HAS_LEFT, &[0]);
        f.frame(500, &d);
        let out = f.resolve();
        assert_eq!(out.presence, Presence::default());
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    #[test]
    fn a_leave_takes_its_line() {
        let mut f = Fixture::plain();
        f.frame(600, &leave(2, RESERVED));
        f.line(598, Kind::ConnectionLost, "p2");
        f.line(603, Kind::PlayerLeft, "p2");
        f.bodies[2] = Some(600);
        f.inputs[2] = Some(450);
        let out = f.resolve();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let [l] = out.presence.leavers.as_slice() else {
            panic!("{:?}", out.presence.leavers);
        };
        assert_eq!(l.username, "p2");
        assert_eq!(l.profile_id, Some(profile(2)));
        assert_eq!((l.playerid, l.team), (Some(102), Some(1)));
        assert_eq!((l.seat, l.slot_type), (Some(SeatState::Reserved), Some(4)));
        assert!(l.alive_at_leave && l.connection_lost && !l.never_spawned);
        assert_eq!(l.silent_seconds, Some(15.0));
        assert_eq!((l.source, l.returned), (Source::Slot, None));
        assert_eq!(l.when.phase, Phase::Action);
        let json = serde_json::to_value(l).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "username": "p2",
                "profileID": profile(2),
                "playerid": 102,
                "team": 1,
                "time": "2:59",
                "phase": "Action",
                "elapsed": 47,
                "recordingTime": 60.0,
                "seat": "reserved",
                "slotType": 4,
                "aliveAtLeave": true,
                "connectionLost": true,
                "connectionLostSource": "inferred",
                "silentSeconds": 15.0,
                "source": "slot",
            })
        );
    }

    #[test]
    fn a_dead_leaver_says_how_long_they_were_dead() {
        let mut f = Fixture::plain();
        f.deaths = vec![("p0", Some(50.0)), ("p1", Some(70.0))];
        // An opened seat loses its ids; the leaver's are those before.
        f.frame(600, &leave(0, OPENED));
        f.line(600, Kind::PlayerLeft, "p0");
        // A death after the leave is none of a leaver's.
        f.frame(650, &leave(1, OPENED));
        f.line(654, Kind::PlayerLeft, "p1");
        f.bodies[1] = Some(650);
        let out = f.resolve();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let l = &out.presence.leavers;
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].profile_id, Some(profile(0)));
        assert_eq!(l[0].playerid, Some(100));
        assert_eq!(l[0].seat, Some(SeatState::Opened));
        assert!(!l[0].alive_at_leave && !l[0].connection_lost);
        assert_eq!(l[0].died_seconds_before, Some(10.0));
        // Nothing is said of the input of the dead.
        assert_eq!(l[0].silent_seconds, None);
        assert!(l[1].alive_at_leave);
        assert_eq!(l[1].died_seconds_before, None);
    }

    #[test]
    fn a_line_without_a_change_is_a_leaver_of_the_feed() {
        let mut f = Fixture::plain();
        f.line(2996, Kind::PlayerLeft, "p3");
        let out = f.resolve();
        // At the very end of the recording that is as it was seen.
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let [l] = out.presence.leavers.as_slice() else {
            panic!("{:?}", out.presence.leavers);
        };
        assert_eq!(
            (l.source, l.seat, l.slot_type),
            (Source::FeedOnly, None, None)
        );
        assert_eq!((l.username.as_str(), l.team), ("p3", Some(1)));
        assert_eq!(l.profile_id, Some(profile(3)));
        assert_eq!(l.playerid, Some(103));
        assert!(l.alive_at_leave);
        // Anywhere else it is a fault, as is a change with no line.
        let mut f = Fixture::plain();
        f.bodies[1] = Some(900);
        f.line(700, Kind::PlayerLeft, "p3");
        f.frame(900, &leave(1, RESERVED));
        let out = f.resolve();
        assert_eq!(out.presence.leavers.len(), 2);
        assert_eq!(out.warnings.len(), 2, "{:?}", out.warnings);
        // A line too late for the change is not its line.
        let mut f = Fixture::plain();
        f.bodies[1] = Some(900);
        f.frame(900, &leave(1, RESERVED));
        f.line(906, Kind::PlayerLeft, "p1");
        assert_eq!(f.resolve().presence.leavers.len(), 2);
        // Nor is the line of another player.
        let mut f = Fixture::plain();
        f.bodies[1] = Some(900);
        f.frame(900, &leave(1, RESERVED));
        f.line(902, Kind::PlayerLeft, "p0");
        assert_eq!(f.resolve().warnings.len(), 2);
    }

    #[test]
    fn a_reconnect_in_the_same_round_says_how_long_they_were_away() {
        let mut f = Fixture::plain();
        f.frame(600, &leave(2, RESERVED));
        f.line(602, Kind::PlayerLeft, "p2");
        f.bodies[2] = Some(600);
        // Back with another playerid; the name follows.
        f.frame(1000, &arrive(2, &profile(2), 902));
        f.frame(1003, &name(2, "p2"));
        f.line(1003, Kind::PlayerReconnected, "p2");
        let out = f.resolve();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let [r] = out.presence.reconnects.as_slice() else {
            panic!("{:?}", out.presence.reconnects);
        };
        assert_eq!(r.kind, ReconnectKind::Reconnect);
        assert_eq!((r.username.as_str(), r.playerid), ("p2", Some(902)));
        assert_eq!(r.profile_id, Some(profile(2)));
        assert_eq!(r.away_seconds, Some(40.0));
        assert_eq!(r.new_playerid, Some(true));
        assert!(!r.left_before_recording);
        assert_eq!(r.when.recording_time, Some(100.0));
        assert_eq!(out.presence.leavers[0].returned, Some(Returned::SameRound));
        assert_eq!(out.presence.leavers[0].playerid, Some(102));
        let json = serde_json::to_value(r).unwrap();
        assert_eq!(json["kind"], "reconnect");
        assert_eq!(json["awaySeconds"], 40.0);
        assert_eq!(json["newPlayerid"], true);
        assert!(json.get("leftBeforeRecording").is_none());
    }

    #[test]
    fn a_seat_reserved_at_the_start_is_taken_again() {
        let mut f = Fixture::new([(OCCUPIED, 0), (RESERVED, 1), (OCCUPIED, 1), (OCCUPIED, 0)]);
        f.frame(400, &arrive(1, &profile(1), 101));
        f.frame(402, &name(1, "p1"));
        f.line(402, Kind::PlayerReconnected, "p1");
        let out = f.resolve();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        // The seat held and the seat filled again are the ones not plain.
        let seats: Vec<_> = (out.presence.seats.iter())
            .map(|s| (s.username.as_str(), s.slot_type, s.seat, s.has_left, s.team))
            .collect();
        assert_eq!(
            seats,
            [
                ("p1", 4, SeatState::Reserved, true, Some(0)),
                ("p2", 1, SeatState::Refilled, true, Some(1)),
            ]
        );
        assert_eq!(out.presence.seats[0].profile_id, Some(profile(1)));
        let [r] = out.presence.reconnects.as_slice() else {
            panic!("{:?}", out.presence.reconnects);
        };
        assert_eq!(r.kind, ReconnectKind::Reconnect);
        assert!(r.left_before_recording);
        assert_eq!((r.away_seconds, r.new_playerid), (None, Some(false)));
        assert_eq!(r.team, Some(0));
        assert!(out.presence.leavers.is_empty());
    }

    #[test]
    fn a_join_takes_an_opened_seat() {
        let mut f = Fixture::new([(OCCUPIED, 0), (OCCUPIED, 0), (OCCUPIED, 0), (OPENED, 1)]);
        // An opened seat holds no ids.
        f.seats = Seats::default();
        let mut d = vec![];
        for n in 0..3 {
            d.extend(seat(n, OCCUPIED, 0));
        }
        set(&mut d, SEAT + 3, NAME, b"p3");
        prop(&mut d, PROFILE_ID, NO_PROFILE.as_bytes());
        prop(&mut d, PLAYER_ID, &[0xFF; 8]);
        prop(&mut d, TEAM, &TEAMS[1].to_le_bytes());
        prop(&mut d, SLOT_TYPE, &OPENED.to_le_bytes());
        prop(&mut d, HAS_LEFT, &[1]);
        f.seats.read(&d, 0, None, None);
        f.frame(400, &arrive(3, &profile(7), 107));
        f.frame(403, &name(3, "new"));
        f.line(403, Kind::PlayerJoined, "new");
        let out = f.resolve();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let [s] = out.presence.seats.as_slice() else {
            panic!("{:?}", out.presence.seats);
        };
        assert_eq!((s.seat, s.profile_id.as_deref()), (SeatState::Opened, None));
        assert_eq!(s.team, Some(1));
        let json = serde_json::to_value(s).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "username": "p3", "team": 1, "slotType": 2, "seat": "opened", "hasLeft": true,
            })
        );
        let [r] = out.presence.reconnects.as_slice() else {
            panic!("{:?}", out.presence.reconnects);
        };
        assert_eq!(r.kind, ReconnectKind::Join);
        assert_eq!((r.username.as_str(), r.playerid), ("new", Some(107)));
        assert_eq!(r.profile_id, Some(profile(7)));
        assert_eq!((r.away_seconds, r.new_playerid), (None, None));
        assert!(r.left_before_recording);
        // The joiner who leaves before the next round never spawned.
        f.frame(700, &leave(3, OPENED));
        f.line(701, Kind::PlayerLeft, "new");
        let out = f.resolve();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let [l] = out.presence.leavers.as_slice() else {
            panic!("{:?}", out.presence.leavers);
        };
        assert_eq!((l.username.as_str(), l.playerid), ("new", Some(107)));
        assert!(l.never_spawned && !l.alive_at_leave);
        assert_eq!(l.returned, None);
    }

    #[test]
    fn what_was_not_seen_before_is_named() {
        let mut f = Fixture::plain();
        // A slot type of 7, and a seat going back to 1.
        let mut d = vec![];
        resend(&mut d, SEAT);
        prop(&mut d, SLOT_TYPE, &7u32.to_le_bytes());
        f.frame(300, &d);
        f.frame(500, &leave(1, RESERVED));
        f.line(500, Kind::PlayerLeft, "p1");
        f.bodies[1] = Some(500);
        let mut d = vec![];
        resend(&mut d, SEAT + 1);
        prop(&mut d, SLOT_TYPE, &OCCUPIED.to_le_bytes());
        prop(&mut d, HAS_LEFT, &[0]);
        f.frame(800, &d);
        let out = f.resolve();
        assert_eq!(out.presence.leavers.len(), 1);
        assert!(out.presence.reconnects.is_empty());
        assert_eq!(
            out.warnings,
            [
                "1 times HasLeft went back to 0",
                "slot type changes not seen before: 1 to 7, 4 to 1",
            ]
        );
        // A seat that starts with an unknown slot type is listed as such.
        let f = Fixture::new([(9, 0), (OCCUPIED, 0), (OCCUPIED, 0), (OCCUPIED, 0)]);
        let out = f.resolve();
        assert_eq!(out.presence.seats[0].seat, SeatState::Unknown);
        assert_eq!(out.warnings, ["a seat starts with slot type 9"]);
        // A seat nobody ever sat in is empty, not opened.
        let f = Fixture::new([(OPENED, 0), (OCCUPIED, 0), (OCCUPIED, 0), (OCCUPIED, 0)]);
        let out = f.resolve();
        assert_eq!(out.presence.seats[0].seat, SeatState::Empty);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    #[test]
    fn a_body_that_disagrees_with_the_feed_is_said() {
        let mut f = Fixture::plain();
        f.frame(600, &leave(2, RESERVED));
        f.line(601, Kind::PlayerLeft, "p2");
        // Alive by the feed, and the body stays.
        let out = f.resolve();
        assert!(out.presence.leavers[0].alive_at_leave);
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
        // A player with no health when action started never spawned.
        f.down = vec!["p2".to_owned()];
        let out = f.resolve();
        let l = &out.presence.leavers[0];
        assert!(!l.alive_at_leave && l.never_spawned);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    #[test]
    fn malformed_values_are_skipped_and_counted() {
        let mut f = Fixture::plain();
        let mut d = vec![];
        resend(&mut d, SEAT);
        prop(&mut d, SLOT_TYPE, &[4, 0]);
        prop(&mut d, HAS_LEFT, &[1, 1]);
        prop(&mut d, PLAYER_ID, &[1, 2, 3]);
        f.frame(100, &d);
        let out = f.resolve();
        assert_eq!(out.presence, Presence::default());
        assert_eq!(
            out.warnings,
            ["3 slot types, flags or ids of controllers of another size than theirs"]
        );
        // Bytes cut anywhere do not panic.
        let whole = leave(1, OPENED);
        for cut in 0..whole.len() {
            let mut f = Fixture::plain();
            f.frame(100, &whole[..cut]);
            f.resolve();
        }
        // An object that never had a playerid is no controller.
        let mut f = Fixture::plain();
        let mut d = vec![];
        set(&mut d, 0xF000_0900, SLOT_TYPE, &OCCUPIED.to_le_bytes());
        f.seats.read(&d, 0, None, None);
        let mut d = vec![];
        set(&mut d, 0xF000_0900, SLOT_TYPE, &RESERVED.to_le_bytes());
        f.frame(100, &d);
        let out = f.resolve();
        assert_eq!(out.presence, Presence::default());
    }

    /// A round of a match: `players` are in its header, `presence` what
    /// it showed.
    fn round(number: u32, players: &[(usize, u64)], presence: Presence) -> Round {
        let mut round = Round::default();
        round.header.round_number = number - 1;
        round.header.players = (players.iter())
            .map(|&(i, id)| Player {
                id,
                username: format!("p{i}"),
                profile_id: profile(i),
                team_index: i / 2,
                ..Player::default()
            })
            .collect();
        round.presence = Some(presence);
        round
    }

    fn leaver(n: usize, frame: u32, lost: bool) -> Leaver {
        Leaver {
            username: format!("p{n}"),
            profile_id: Some(profile(n)),
            playerid: Some(100 + n as u64),
            team: Some(n / 2),
            when: When::default(),
            seat: Some(SeatState::Reserved),
            slot_type: Some(RESERVED),
            alive_at_leave: true,
            died_seconds_before: None,
            never_spawned: false,
            connection_lost: lost,
            connection_lost_source: Basis::Inferred,
            silent_seconds: None,
            returned: None,
            source: Source::Slot,
            frame: Some(frame),
            controller: Some(SEAT + n as u32),
        }
    }

    fn reserved(n: usize) -> Seat {
        Seat {
            username: format!("p{n}"),
            profile_id: Some(profile(n)),
            team: Some(n / 2),
            slot_type: RESERVED,
            seat: SeatState::Reserved,
            has_left: true,
            playerid: Some(100 + n as u64),
            controller: SEAT + n as u32,
        }
    }

    fn taking(n: usize, frame: u32, kind: ReconnectKind, playerid: u64) -> Reconnect {
        Reconnect {
            username: format!("p{n}"),
            profile_id: Some(profile(n)),
            playerid: Some(playerid),
            team: Some(n / 2),
            when: When::default(),
            kind,
            away_seconds: None,
            left_before_recording: false,
            new_playerid: None,
            frame: Some(frame),
            controller: SEAT + n as u32,
        }
    }

    #[test]
    fn a_match_follows_its_leavers() {
        let all = [(0, 100), (1, 101), (2, 102), (3, 103)];
        let without_2 = [(0, 100), (1, 101), (3, 103)];
        let rounds = [
            round(1, &all, Presence::default()),
            // p2 leaves; p3 leaves and is back in the round.
            round(
                2,
                &all,
                Presence {
                    leavers: vec![leaver(2, 500, true), leaver(3, 600, false)],
                    reconnects: vec![taking(3, 900, ReconnectKind::Reconnect, 103)],
                    seats: vec![],
                },
            ),
            // Two rounds start without p2, whose seat is held.
            round(
                3,
                &without_2,
                Presence {
                    seats: vec![reserved(2)],
                    ..Presence::default()
                },
            ),
            round(
                4,
                &without_2,
                Presence {
                    seats: vec![reserved(2)],
                    ..Presence::default()
                },
            ),
            // And p2 is back, the game started again.
            round(
                5,
                &[(0, 100), (1, 101), (2, 777), (3, 103)],
                Presence::default(),
            ),
        ];
        let out = rollup(&rounds);
        assert_eq!(out.len(), 2);
        let p2 = &out[0];
        assert_eq!((p2.username.as_str(), p2.team), ("p2", Some(1)));
        let events: Vec<_> = p2.events.iter().map(|e| (e.round, e.kind)).collect();
        assert_eq!(
            events,
            [
                (2, EventKind::Left),
                (3, EventKind::AbsentAtStart),
                (5, EventKind::BackAtStart),
            ]
        );
        assert_eq!((p2.rounds_missed, p2.returned), (2, Returned::Later));
        assert_eq!(p2.new_playerid, Some(true));
        assert_eq!(p2.likely, [Likely::ConnectionLost, Likely::GameRestarted]);
        let p3 = &out[1];
        assert_eq!((p3.rounds_missed, p3.returned), (0, Returned::SameRound));
        assert_eq!(p3.likely, [Likely::GameKeptRunning]);
        let json = serde_json::to_value(p3).unwrap();
        assert_eq!(json["likelySource"], "inferred");
        assert_eq!(json["events"][0]["type"], "left");
        assert_eq!(json["events"][0]["connectionLost"], false);
        assert_eq!(json["events"][1]["type"], "reconnected");
        assert_eq!(json["events"][1]["newPlayerid"], false);
        // What was observed is said, and no verdict on the player.
        let said = serde_json::to_string(&out).unwrap().to_lowercase();
        for word in ["crash", "rage", "abandon"] {
            assert!(!said.contains(word), "{said}");
        }
    }

    #[test]
    fn a_player_who_takes_another_seat_is_no_leaver() {
        let all = [(0, 100), (1, 101), (2, 102), (3, 103)];
        let mut opened = leaver(1, 500, false);
        opened.seat = Some(SeatState::Opened);
        let mut join = taking(3, 800, ReconnectKind::Join, 555);
        join.username = "new".to_owned();
        join.profile_id = Some(profile(9));
        let rounds = [
            round(
                1,
                &all,
                Presence {
                    leavers: vec![opened],
                    reconnects: vec![join],
                    seats: vec![],
                },
            ),
            round(2, &[(0, 100), (2, 102), (3, 103)], Presence::default()),
            // The leaver takes an open seat again, the game still running.
            round(
                3,
                &[(0, 100), (2, 102), (3, 103)],
                Presence {
                    reconnects: vec![taking(1, 300, ReconnectKind::Join, 101)],
                    ..Presence::default()
                },
            ),
        ];
        let out = rollup(&rounds);
        let [p1] = out.as_slice() else {
            panic!("{out:?}");
        };
        let events: Vec<_> = p1.events.iter().map(|e| (e.round, e.kind)).collect();
        assert_eq!(
            events,
            [
                (1, EventKind::Left),
                (2, EventKind::AbsentAtStart),
                (3, EventKind::Joined),
            ]
        );
        // No seat was held for them, so no round counts as missed.
        assert_eq!((p1.rounds_missed, p1.returned), (0, Returned::Later));
        assert_eq!(p1.likely, [Likely::GameKeptRunning]);
        // A match nobody left has no entries; rounds not read for it none.
        assert!(rollup(&[round(1, &all, Presence::default())]).is_empty());
        assert!(rollup(&[Round::default()]).is_empty());
    }
}
