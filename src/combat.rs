//! Who killed, downed and revived whom, and every hit a player took (Y11S3).
//!
//! The `TimelineChannel` stream holds the finished round's timeline. The
//! game writes it into the first frames of the file (within 41 frames in
//! the test rounds), in records that each hold the timeline from its start
//! up to a later entry, so the largest record is the whole round's:
//!
//! ```text
//! u32 n, n x { u8 type, u32 frame, body }
//!
//! 1 kill       u64 weapon, u64 attacker body, u64 attacker, u64 icon, u32 team,
//!              u64 victim, u64 icon, u32 team, u8 headshot
//! 2 team kill  as 1
//! 3 death      u64 weapon, u64 body, u64 victim, u64 icon, u32 team
//! 5 down       as 1 without weapon and headshot; the attacker is ff..ff
//!              when nobody downed the victim
//! 7 revive     as 5; the "attacker" is the reviver, the victim themselves on
//!              a self-revive
//! 9 phase      u32
//! 10 defuser   u64 player, u64 icon, u32 team, u8
//! ```
//!
//! Players are the header's `playerid`, icons the operator's role image,
//! and `frame` is a frame of the frame index. The layout was checked by
//! parsing the record to its last byte in every finished real round, and by
//! finding every kill of the kill feed in it with the same killer, victim
//! and headshot flag.
//!
//! A hit is written to the victim's body, in the `607385fe` updates of the
//! movement stream (see [`crate::loadout`] for its messages). An update is
//! `607385fe, u16 mask, 3 x f32 position, ...` and a body's ends in a tail:
//!
//! ```text
//! u16 F, u16 G
//! F & 0002   4 x f32   where the player aims, a quaternion (forward is +Y)
//! F & 0008, 0010, 0040, 0080   one f32 each
//! F & 0100   u8
//! F & 0200   the hit block, 24 bytes
//! G & 0008   30fe44a6, f32
//! then 1 to 3 bytes
//!
//! hit block:
//! +0  f32  health / max health after the hit, below 0 when it downs or kills
//! +4  f32  damage multiplier, above 0 and at most 1
//! +8  u32  direction the hit came from, an octant 0 to 7
//! +12 u32  life state after the hit: 1 or 2 alive, 3 down, 4 dead
//! +16 u32  not known, 1 to 8
//! +20 u32  damage type
//! ```
//!
//! What sits before the tail is not decoded, so a hit block is found from
//! the end of the update: a signature scan of its last 140 bytes for 24
//! bytes in the ranges above, kept when the flag word `F` sits where the
//! layout puts it for some combination of the fields before the block.
//! This was checked against the HUD: in the ten test rounds the scan finds
//! 243 blocks, the HUD writes a `DamageTakenEvent` for each of them, and the
//! health the block gives equals the HUD's `Health` wherever both are
//! written.
//!
//! The file does not say who dealt a hit. The attacker is taken from the
//! timeline when the hit downed or killed, and otherwise inferred for
//! bullets from who fired and who aimed at the victim; each hit says which
//! (`attacker_source`).

use std::collections::HashMap;

use rayon::prelude::*;
use serde::Serialize;

use crate::container::StreamInfo;
use crate::details::Phase;
use crate::entities::{Hash, Record, for_each_record, u32_at};
use crate::feedback::display_clock;
use crate::header::Player;
use crate::loadout::{Clock, blocks, messages};
use crate::records::RecordMap;

/// Name hash of the timeline stream (`TimelineChannel`).
const TIMELINE_STREAM: Hash = [0xEE, 0xE4, 0x2D, 0x83];
/// Name hashes of the HUD stream and of the stream that moves the entities.
const STATE_STREAM: Hash = [0xA9, 0x8F, 0xDD, 0x0B];
const MOVEMENT_STREAM: Hash = [0x20, 0xA5, 0xC4, 0xE3];

/// Movement payload type of an entity's updates.
const UPDATE: Hash = [0x60, 0x73, 0x85, 0xFE];
/// Where an update's position starts, after the type and a `u16` mask.
const POSITION_AT: usize = 6;
/// Only updates of at least this many bytes are searched for a hit block,
/// and only its last `TAIL` bytes.
const HIT_UPDATE: usize = 60;
const TAIL: usize = 140;
const HIT_BLOCK: usize = 24;
/// Only updates longer than this carry a body's position and aim: its full
/// updates, of 900 to 1300 bytes.
const FULL_UPDATE: usize = 700;
/// Tail flags `F`: the aim, the four single floats, the byte and the hit
/// block; `G`: the `30fe44a6` float.
const AIM: u16 = 0x0002;
const FLOATS: [u16; 4] = [0x0008, 0x0010, 0x0040, 0x0080];
const BYTE: u16 = 0x0100;
const HIT: u16 = 0x0200;
const G_FLOAT: u16 = 0x0008;
/// `F` bits never set in a tail that holds a hit block, and those never set
/// in one whose aim is read.
const NOT_WITH_HIT: u16 = 0x0024;
const NOT_WITH_AIM: u16 = 0x3C24;

/// HUD life object: `Health` (overheal included) and `MaxHealth`.
const HEALTH: Hash = [0x25, 0x26, 0x76, 0xC9];
const MAX_HEALTH: Hash = [0x11, 0x49, 0xA6, 0x72];
/// HUD weapon ammunition: rounds in the gun plus in reserve (`TotalAmmo`).
/// It drops when the gun fires.
const TOTAL_AMMO: Hash = [0x40, 0x0A, 0xC8, 0x29];
/// How far up the HUD tree a weapon's ammunition is looked up to its
/// player's controller; it hangs four links under it.
const OWNER_DEPTH: usize = 12;

/// Hit block life states.
const STATE_DOWN: u32 = 3;
const STATE_DEAD: u32 = 4;
/// Damage type of a bullet, the only one an attacker is inferred for.
const BULLET: u32 = 0;

/// A timeline kill or down belongs to a hit on its victim within this many
/// seconds.
const SAME_EVENT: f64 = 0.5;
/// A shot counts for a hit when the shooter's ammunition dropped from this
/// long before the hit to this long after it: 4 updates before to 2 after,
/// at the 34 ms the game sends them.
const SHOT_BEFORE: f64 = 0.140;
const SHOT_AFTER: f64 = 0.070;
/// Without a shot, the attacker is the opponent aiming within this many
/// degrees of the victim.
const AIM_DEGREES: f64 = 10.0;
/// The HUD writes the health a hit left from one update before the body's
/// update holds the hit to three after it (35 ms before to 100 ms after in
/// the test rounds). A `Health` written up to `HUD_EARLY` seconds before a
/// hit can be that hit's own, and one written up to `HUD_LATE` seconds
/// after a hit says nothing new.
const HUD_EARLY: f64 = 0.05;
const HUD_LATE: f64 = 0.15;
/// Seconds per frame, for a file without a usable frame index.
const FRAME: f64 = 0.034;

/// Timeline entry types.
const KILL: u8 = 1;
const TEAM_KILL: u8 = 2;
const DEATH: u8 = 3;
const DOWN: u8 = 5;
const REVIVE: u8 = 7;
const PHASE: u8 = 9;
const DEFUSER: u8 = 10;
/// The attacker of a down nobody dealt.
const NOBODY: u64 = u64::MAX;

/// What the timeline and the hit blocks of a round hold.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Combat {
    /// Kills, team kills, deaths, downs and revives, in timeline order.
    pub events: Vec<TimelineEvent>,
    /// Every hit a player took, in the order the file holds them.
    pub hits: Vec<Hit>,
    /// What could not be read.
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum TimelineKind {
    Kill,
    /// A kill of a teammate.
    TeamKill,
    /// A death the game credits to nobody.
    Death,
    Down,
    Revive,
}

/// One entry of the round's timeline.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEvent {
    #[serde(rename = "type")]
    pub kind: TimelineKind,
    /// The victim, or the player revived.
    pub username: String,
    /// The killer, the player who downed the victim, or the reviver (the
    /// player themselves on a self-revive). `None` when the file names
    /// nobody.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// The weapon or gadget id, as in the kill feed (kills and deaths).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weapon: Option<u64>,
    /// Kills only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headshot: Option<bool>,
    pub time: String,
    pub phase: Phase,
    /// Seconds since the prep phase started.
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
    /// Seconds since the recording started, to the frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_time: Option<f64>,
}

/// What a hit left of the victim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum HitResult {
    Alive,
    Down,
    Dead,
}

/// What dealt a hit. The names are inferred from what was being used when
/// hits of each type landed; the file holds the number only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct DamageType {
    pub id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
}

impl DamageType {
    fn new(id: u32) -> Self {
        let name = match id {
            0 => Some("bullet"),
            1 => Some("melee"),
            2 => Some("explosion"),
            9 => Some("gas"),
            36 => Some("fire"),
            _ => None,
        };
        DamageType { id, name }
    }
}

/// How the attacker of a hit was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum AttackerSource {
    /// The hit downed or killed, and the timeline names who did.
    Timeline,
    /// Inferred: the opponent who fired as the hit landed; of several, the
    /// one aiming closest to the victim.
    Shot,
    /// Inferred: nobody was seen firing, and this opponent aimed at the
    /// victim.
    Aim,
}

/// A hit a player took.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    /// The victim.
    pub username: String,
    /// Health lost. `None` for a hit that downed or killed: its block gives
    /// a health below zero, not what was left to lose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damage: Option<u32>,
    /// Health after the hit; 0 when down or dead.
    pub health: u32,
    pub result: HitResult,
    #[serde(rename = "type")]
    pub kind: DamageType,
    /// What the damage was multiplied by; below 1 seen on bullets and
    /// explosions only.
    pub multiplier: f32,
    /// Where the hit came from, as one of eight directions around the
    /// victim, 0 to 7.
    pub direction: u32,
    /// Who dealt the hit, as `attacker_source` found them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attacker_source: Option<AttackerSource>,
    /// Metres between `by` and the victim, when both bodies' positions are
    /// known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<f32>,
    pub time: String,
    pub phase: Phase,
    /// Seconds since the prep phase started.
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
    /// Seconds since the recording started, to the frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_time: Option<f64>,
    /// Who the shot and aim rules name for a bullet, also when the timeline
    /// names the attacker: kept to measure the rules against the timeline.
    #[serde(skip)]
    pub estimate: Option<(String, AttackerSource)>,
}

/// A kill, team kill, death, down or revive as the timeline record holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    kind: TimelineKind,
    frame: u32,
    /// `playerid` of the killer, downer or reviver.
    attacker: Option<u64>,
    /// `playerid` of the victim or the player revived.
    victim: u64,
    /// The teams as the entry gives them. `None` for the attacker of a
    /// death.
    attacker_team: Option<u32>,
    victim_team: u32,
    weapon: Option<u64>,
    headshot: Option<bool>,
}

/// A timeline record's entries, and why reading stopped early if it did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Entries {
    entries: Vec<Entry>,
    problem: Option<String>,
}

/// Little-endian reads that stop at the end of the bytes.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let end = self.at.checked_add(N)?;
        let v = self.bytes.get(self.at..end)?.try_into().ok()?;
        self.at = end;
        Some(v)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|b| b[0])
    }

    fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_le_bytes)
    }

    fn u64(&mut self) -> Option<u64> {
        self.take().map(u64::from_le_bytes)
    }
}

/// Parses a timeline record. An entry of an unknown type or one the record
/// cannot hold ends the read: what was read before it is kept.
fn entries(record: &[u8]) -> Entries {
    let mut r = Reader {
        bytes: record,
        at: 0,
    };
    let mut out = Entries::default();
    let Some(count) = r.u32() else {
        out.problem = Some("the timeline record is too short for its count".into());
        return out;
    };
    for i in 0..count {
        let start = r.at;
        match entry(&mut r) {
            Ok(Some(e)) => out.entries.push(e),
            Ok(None) => {}
            Err(Unread::Type(t)) => {
                out.problem = Some(format!(
                    "timeline entry {i} of {count} has unknown type {t}: the rest is not read"
                ));
                return out;
            }
            Err(Unread::Cut) => {
                out.problem = Some(format!(
                    "the timeline record ends inside entry {i} of {count} (at byte {start} of {})",
                    record.len()
                ));
                return out;
            }
        }
    }
    if r.at != record.len() {
        out.problem = Some(format!(
            "{} bytes follow the timeline's {count} entries",
            record.len() - r.at
        ));
    }
    out
}

/// Why a timeline entry could not be read.
enum Unread {
    /// A type this parser does not know the size of.
    Type(u8),
    /// The record ends inside the entry.
    Cut,
}

/// The entry at the reader. `Ok(None)` for the types that are passed over
/// (phase changes and defuser events).
fn entry(r: &mut Reader) -> Result<Option<Entry>, Unread> {
    let read = |r: &mut Reader| -> Option<Result<Option<Entry>, Unread>> {
        let kind = r.u8()?;
        let frame = r.u32()?;
        let kind = match kind {
            KILL => TimelineKind::Kill,
            TEAM_KILL => TimelineKind::TeamKill,
            DEATH => TimelineKind::Death,
            DOWN => TimelineKind::Down,
            REVIVE => TimelineKind::Revive,
            PHASE => {
                r.u32()?;
                return Some(Ok(None));
            }
            DEFUSER => {
                r.take::<21>()?;
                return Some(Ok(None));
            }
            other => return Some(Err(Unread::Type(other))),
        };
        let kill = matches!(kind, TimelineKind::Kill | TimelineKind::TeamKill);
        let weapon = match kind {
            TimelineKind::Down | TimelineKind::Revive => None,
            _ => Some(r.u64()?),
        };
        // The body of the attacker, or of the victim of a death.
        r.u64()?;
        let (attacker, attacker_team) = if kind == TimelineKind::Death {
            (None, None)
        } else {
            let id = r.u64()?;
            r.u64()?;
            (Some(id).filter(|&id| id != NOBODY), Some(r.u32()?))
        };
        let victim = r.u64()?;
        r.u64()?;
        let victim_team = r.u32()?;
        let headshot = if kill { Some(r.u8()? != 0) } else { None };
        Some(Ok(Some(Entry {
            kind,
            frame,
            attacker,
            victim,
            attacker_team,
            victim_team,
            weapon,
            headshot,
        })))
    };
    read(r).unwrap_or(Err(Unread::Cut))
}

/// The timeline stream's largest record, which holds every entry the
/// smaller ones do. Empty when the stream has only empty records, as in a
/// recording that stops before anything happened (real ones of 6 and 7
/// seconds do). `None` when the file has no such stream.
fn timeline_record<'a>(data: &'a [u8], map: &RecordMap) -> Option<&'a [u8]> {
    let stream = map.stream_index(TIMELINE_STREAM)?;
    let mut largest: &[u8] = &[];
    for (_, start, end) in map.records_of(stream) {
        let record = data.get(start..end).unwrap_or_default();
        if record.len() > largest.len() {
            largest = record;
        }
    }
    Some(largest)
}

/// When a frame was, on the round's clocks.
struct When {
    time: String,
    phase: Phase,
    elapsed: f64,
    recording_time: Option<f64>,
}

/// Places `frame` on the round clock: the reading in force is the last one
/// shown at or before the frame's time. Streams are stored one after the
/// other, so unlike in the state stream an offset does not say which
/// reading that is.
fn when(clock: &Clock, frame: u32) -> When {
    let seconds = clock.frame_times.get(frame as usize).copied();
    let tick = seconds.and_then(|t| {
        let shown = &clock.timeline.recording;
        shown.iter().rposition(|r| r.is_some_and(|r| r <= t))
    });
    let at = clock.timeline.at_time(tick, seconds);
    When {
        time: display_clock(at.seconds),
        phase: at.phase,
        elapsed: at.elapsed,
        recording_time: seconds.map(|t| (t * 1000.0).round() / 1000.0),
    }
}

/// Seconds since the recording started at `frame`, for comparing frames.
fn seconds(clock: &Clock, frame: u32) -> f64 {
    let known = clock.frame_times.get(frame as usize).copied();
    known.unwrap_or(f64::from(frame) * FRAME)
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn f32_at(d: &[u8], at: usize) -> Option<f32> {
    u32_at(d, at).map(f32::from_bits)
}

/// A hit block as an update holds it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Block {
    /// Health over max health after the hit.
    ratio: f32,
    multiplier: f32,
    direction: u32,
    state: u32,
    kind: u32,
}

/// The 24 bytes at `at` as a hit block, when every field is in the range
/// hit blocks keep to.
fn block_at(update: &[u8], at: usize) -> Option<Block> {
    let ratio = f32_at(update, at)?;
    let multiplier = f32_at(update, at + 4)?;
    let direction = u32_at(update, at + 8)?;
    let state = u32_at(update, at + 12)?;
    let unknown = u32_at(update, at + 16)?;
    let kind = u32_at(update, at + 20)?;
    let (r, m) = (f64::from(ratio), f64::from(multiplier));
    let fits = direction < 64
        && (1..=8).contains(&state)
        && (1..=8).contains(&unknown)
        && kind < 64
        && m > 0.0
        && m <= 1.0001
        && (-3.0..=1.0001).contains(&r)
        // A ratio of zero is written as zero, not as a tiny number.
        && (r.abs() > 1e-6 || ratio.to_bits() == 0);
    fits.then_some(Block {
        ratio,
        multiplier,
        direction,
        state,
        kind,
    })
}

/// Whether a tail's flag words sit before a hit block at `at`: for some
/// combination of the fields the layout puts between them, the word found
/// there has exactly those fields' flags and the hit block's.
fn flags_before(update: &[u8], at: usize) -> bool {
    let fields = [
        (AIM, 16),
        (FLOATS[0], 4),
        (FLOATS[1], 4),
        (FLOATS[2], 4),
        (FLOATS[3], 4),
        (BYTE, 1),
    ];
    let all = fields.iter().fold(0, |all, f| all | f.0);
    (0..1u32 << fields.len()).any(|chosen| {
        let (mut flags, mut size) = (0, 4);
        for (i, (flag, bytes)) in fields.iter().enumerate() {
            if chosen & (1 << i) != 0 {
                flags |= flag;
                size += bytes;
            }
        }
        let word = at
            .checked_sub(size)
            .filter(|&q| q >= POSITION_AT)
            .and_then(|q| u16_at(update, q));
        word.is_some_and(|f| f & HIT != 0 && f & all == flags && f & NOT_WITH_HIT == 0)
    })
}

/// The hit blocks in the tail of a body's update.
fn hit_blocks(update: &[u8]) -> Vec<Block> {
    let mut out = Vec::new();
    if update.len() < HIT_UPDATE {
        return out;
    }
    let first = update.len().saturating_sub(TAIL).max(POSITION_AT);
    for at in (first..=update.len() - HIT_BLOCK).rev() {
        if let Some(block) = block_at(update, at)
            && flags_before(update, at)
        {
            out.push(block);
        }
    }
    out
}

/// Where the player aims, from the tail of a body's update: the unit
/// quaternion after flag words whose fields end 1 to 3 bytes before the
/// update does.
fn tail_aim(update: &[u8]) -> Option<[f32; 4]> {
    for back in 21..100 {
        let q = update
            .len()
            .checked_sub(back)
            .filter(|&q| q >= POSITION_AT)?;
        let (Some(f), Some(g)) = (u16_at(update, q), u16_at(update, q + 2)) else {
            continue;
        };
        if f & AIM == 0 || f & NOT_WITH_AIM != 0 || g & !G_FLOAT != 0 {
            continue;
        }
        let mut size = 4 + 16;
        size += 4 * FLOATS.iter().filter(|&&flag| f & flag != 0).count();
        size += usize::from(f & BYTE != 0);
        size += if f & HIT != 0 { HIT_BLOCK } else { 0 };
        size += if g & G_FLOAT != 0 { 8 } else { 0 };
        if !(size + 1..=size + 3).contains(&back) {
            continue;
        }
        let mut aim = [0.0; 4];
        for (i, v) in aim.iter_mut().enumerate() {
            *v = f32_at(update, q + 4 + 4 * i)?;
        }
        let norm: f64 = aim.iter().map(|&v| f64::from(v) * f64::from(v)).sum();
        if (norm - 1.0).abs() < 1e-3 {
            return Some(aim);
        }
    }
    None
}

/// `v` turned by the quaternion `q` (`x, y, z, w`).
fn rotate(q: [f32; 4], v: [f64; 3]) -> [f64; 3] {
    let [x, y, z, w] = q.map(f64::from);
    let [vx, vy, vz] = v;
    let (tx, ty, tz) = (
        2.0 * (y * vz - z * vy),
        2.0 * (z * vx - x * vz),
        2.0 * (x * vy - y * vx),
    );
    [
        vx + w * tx + (y * tz - z * ty),
        vy + w * ty + (z * tx - x * tz),
        vz + w * tz + (x * ty - y * tx),
    ]
}

/// Degrees between two directions; `None` when either has no length.
fn degrees_between(u: [f64; 3], v: [f64; 3]) -> Option<f64> {
    let dot: f64 = u.iter().zip(&v).map(|(a, b)| a * b).sum();
    let length = |v: [f64; 3]| v.iter().map(|a| a * a).sum::<f64>().sqrt();
    let lengths = length(u) * length(v);
    (lengths > 0.0).then(|| (dot / lengths).clamp(-1.0, 1.0).acos().to_degrees())
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f64 {
    let squares = a.iter().zip(&b).map(|(a, b)| {
        let d = f64::from(*a) - f64::from(*b);
        d * d
    });
    squares.sum::<f64>().sqrt()
}

/// A hit block found in the movement stream.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RawHit {
    frame: u32,
    /// Index of the victim in the players.
    player: usize,
    block: Block,
}

/// A body's position, and where its player aims when the update says.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pose {
    frame: u32,
    player: usize,
    position: [f32; 3],
    aim: Option<[f32; 4]>,
}

/// What one record of the movement stream holds of the players' bodies.
#[derive(Debug, Default)]
struct Moved {
    hits: Vec<RawHit>,
    poses: Vec<Pose>,
    /// The record ends before the messages it counts do.
    cut: bool,
}

/// Reads one movement record. `bodies` maps a body's entity to its player.
fn moved(record: &[u8], frame: u32, bodies: &HashMap<u64, usize>) -> Moved {
    let mut out = Moved::default();
    let counted = u16_at(record, 0).map_or(0, usize::from);
    let mut read = 0;
    for (entity, _, update) in messages(record) {
        read += 1;
        let Some(&player) = bodies.get(&entity) else {
            continue;
        };
        if !update.starts_with(&UPDATE) {
            continue;
        }
        out.hits
            .extend(hit_blocks(update).into_iter().map(|block| RawHit {
                frame,
                player,
                block,
            }));
        if update.len() > FULL_UPDATE {
            let at = |i: usize| f32_at(update, POSITION_AT + 4 * i);
            if let (Some(x), Some(y), Some(z)) = (at(0), at(1), at(2)) {
                out.poses.push(Pose {
                    frame,
                    player,
                    position: [x, y, z],
                    aim: tail_aim(update),
                });
            }
        }
    }
    out.cut = read < counted;
    out
}

/// Where each body was and where its player aimed, update by update.
#[derive(Debug, Default)]
struct Tracks {
    /// Per player, in frame order. The aim is the last one read.
    players: Vec<Vec<Pose>>,
}

impl Tracks {
    /// Follows the bodies through `poses`, which are in stream order. A
    /// position off the map, at the origin or more than 5 m from the last
    /// one is not one: the body keeps its last.
    fn follow(players: usize, poses: impl Iterator<Item = Pose>) -> Tracks {
        let mut tracks = Tracks {
            players: vec![Vec::new(); players],
        };
        for mut pose in poses {
            let Some(track) = tracks.players.get_mut(pose.player) else {
                continue;
            };
            let last = track.last();
            let on_map = pose.position.iter().all(|c| c.abs() < 500.0)
                && distance(pose.position, [0.0; 3]) > 1e-3
                && last.is_none_or(|l| distance(pose.position, l.position) < 5.0);
            if !on_map {
                match last {
                    Some(l) => pose.position = l.position,
                    None => continue,
                }
            }
            pose.aim = pose.aim.or(last.and_then(|l| l.aim));
            track.push(pose);
        }
        tracks
    }

    /// The player's pose after the last update before `frame`.
    fn before(&self, player: usize, frame: u32) -> Option<&Pose> {
        let track = self.players.get(player)?;
        track[..track.partition_point(|p| p.frame < frame)].last()
    }
}

/// A value the HUD stream wrote that the hits need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seen {
    /// `child` hangs off `parent`.
    Link(u32, u32),
    /// Object, property, value.
    Value(u32, Hash, u32),
}

/// The links and the health and ammunition values of one HUD snapshot or
/// record. `life` holds the players' health objects.
fn seen(block: &[u8], life: &HashMap<u32, usize>) -> Vec<Seen> {
    let mut out = Vec::new();
    // Each record block names its object before writing to it.
    let mut current: Option<u32> = None;
    let value = |out: &mut Vec<Seen>, obj: u32, hash: Hash, from: usize, to: usize| {
        let wanted = hash == TOTAL_AMMO
            || ((hash == HEALTH || hash == MAX_HEALTH) && life.contains_key(&obj));
        if let (true, Some(v)) = (wanted && to - from == 4, u32_at(block, from)) {
            out.push(Seen::Value(obj, hash, v));
        }
    };
    for_each_record(block, |at, r| match r {
        Record::Set(obj, hash, from, to) => {
            current = Some(obj);
            value(&mut out, obj, hash, from, to);
        }
        // `26` array elements are not values of the object.
        Record::Prop(hash, from, to) if block.get(at) == Some(&0x22) => {
            if let Some(obj) = current {
                value(&mut out, obj, hash, from, to);
            }
        }
        Record::ParentChild(parent, _, child) => {
            current = Some(parent);
            if child != 0 {
                out.push(Seen::Link(parent, child));
            }
        }
        Record::Child(_, child) | Record::Element(_, _, child) => {
            if let (Some(parent), true) = (current, child != 0) {
                out.push(Seen::Link(parent, child));
            }
        }
        Record::Prop(..) => {}
    });
    out
}

/// What the HUD says of each player, by index into the players. Times are
/// seconds since the recording started, negative infinity in the opening
/// snapshot.
#[derive(Debug, Default)]
struct Hud {
    /// `(time, Health)` in stream order.
    health: Vec<Vec<(f64, u32)>>,
    /// `(time, MaxHealth)` in stream order, values above 0 only.
    max_health: Vec<Vec<(f64, u32)>>,
    /// When a gun of the player fired.
    shots: Vec<Vec<f64>>,
}

impl Hud {
    /// Reads the HUD stream. Its blocks do not depend on each other until
    /// their values are put in order, so they are read in parallel.
    fn read(
        data: &[u8],
        map: &RecordMap,
        streams: &[StreamInfo],
        players: &[Player],
        clock: &Clock,
    ) -> Hud {
        let of = |object: fn(&crate::header::PlayerEntities) -> Option<u32>| {
            let ids = players.iter().enumerate();
            ids.filter_map(|(i, p)| Some((object(p.entities.as_ref()?)?, i)))
                .collect::<HashMap<u32, usize>>()
        };
        let life = of(|e| e.health);
        let controllers = of(|e| Some(e.controller));
        let spans: Vec<_> = blocks(map, streams, STATE_STREAM).collect();
        let read: Vec<(Option<u32>, Vec<Seen>)> = spans
            .par_iter()
            .map(|&(start, end, frame)| {
                let block = data.get(start..end).unwrap_or_default();
                (frame, seen(block, &life))
            })
            .collect();

        let mut hud = Hud::new(players.len());
        let mut tree = Tree {
            life,
            controllers,
            ..Tree::default()
        };
        for (frame, values) in read {
            hud.add(&mut tree, frame.map(|f| seconds(clock, f)), &values);
        }
        hud
    }

    fn new(players: usize) -> Hud {
        Hud {
            health: vec![Vec::new(); players],
            max_health: vec![Vec::new(); players],
            shots: vec![Vec::new(); players],
        }
    }

    /// Takes in what one snapshot or record wrote, in stream order. `time`
    /// is `None` for the snapshot: it holds the state the recording started
    /// in, not something that happened in it.
    fn add(&mut self, tree: &mut Tree, time: Option<f64>, values: &[Seen]) {
        let at = time.unwrap_or(f64::NEG_INFINITY);
        for &v in values {
            match v {
                Seen::Link(parent, child) => {
                    tree.parents.insert(child, parent);
                }
                Seen::Value(obj, TOTAL_AMMO, now) => {
                    let before = tree.ammo.insert(obj, now);
                    let fired = before.is_some_and(|before| now < before);
                    if let (true, Some(_), Some(p)) = (fired, time, tree.owner(obj))
                        && let Some(shots) = self.shots.get_mut(p)
                    {
                        shots.push(at);
                    }
                }
                Seen::Value(obj, hash, now) => {
                    let Some(&p) = tree.life.get(&obj) else {
                        continue;
                    };
                    if hash == HEALTH {
                        self.health[p].push((at, now));
                    } else if now > 0 {
                        self.max_health[p].push((at, now));
                    }
                }
            }
        }
    }

    /// The player's max health at `time`: the last written by then, else
    /// the first written at all.
    fn max_at(&self, player: usize, time: f64) -> Option<u32> {
        let series = self.max_health.get(player)?;
        let by_then = series[..series.partition_point(|s| s.0 <= time)].last();
        by_then.or(series.first()).map(|s| s.1)
    }

    /// `(time, Health)` last written to the player before `time`.
    fn health_before(&self, player: usize, time: f64) -> Option<(f64, u32)> {
        let series = self.health.get(player)?;
        series[..series.partition_point(|s| s.0 < time)]
            .last()
            .copied()
    }

    /// Whether a gun of the player fired between `from` and `to`.
    fn fired(&self, player: usize, from: f64, to: f64) -> bool {
        let shots = self.shots.get(player).map_or(&[][..], Vec::as_slice);
        let first = shots.partition_point(|&t| t < from);
        shots.get(first).is_some_and(|&t| t <= to)
    }
}

/// The HUD objects as the stream has linked them so far.
#[derive(Debug, Default)]
struct Tree {
    /// Health object -> index of its player, and controller -> the same.
    life: HashMap<u32, usize>,
    controllers: HashMap<u32, usize>,
    /// Object -> the object it was last linked under.
    parents: HashMap<u32, u32>,
    /// Ammunition object -> its last `TotalAmmo`.
    ammo: HashMap<u32, u32>,
}

impl Tree {
    /// The player whose controller `object` hangs under.
    fn owner(&self, mut object: u32) -> Option<usize> {
        for _ in 0..OWNER_DEPTH {
            if let Some(&player) = self.controllers.get(&object) {
                return Some(player);
            }
            object = *self.parents.get(&object)?;
        }
        None
    }
}

/// Who the shot and aim rules name as the attacker of a bullet that hit
/// `victim` in `frame`: of the opponents whose aim is known, the one who
/// fired as the hit landed (of several, the one aiming closest to the
/// victim), else the one aiming within `AIM_DEGREES` of the victim.
fn estimate(
    victim: usize,
    frame: u32,
    time: f64,
    players: &[Player],
    tracks: &Tracks,
    hud: &Hud,
) -> Option<(usize, AttackerSource)> {
    let team = players.get(victim)?.team_index;
    let target = tracks.before(victim, frame).map(|p| p.position);
    // Degrees between where the opponent aims and where the victim is. An
    // angle that cannot be measured is larger than any that can.
    let off = |pose: &Pose, aim: [f32; 4]| -> f64 {
        let to_victim =
            target.map(|t| [0, 1, 2].map(|i| f64::from(t[i]) - f64::from(pose.position[i])));
        let angle = to_victim.and_then(|d| degrees_between(rotate(aim, [0.0, 1.0, 0.0]), d));
        angle.unwrap_or(999.0)
    };
    let opponents: Vec<(usize, f64)> = (players.iter().enumerate())
        .filter(|(_, p)| p.team_index != team)
        .filter_map(|(i, _)| {
            let pose = tracks.before(i, frame)?;
            Some((i, off(pose, pose.aim?)))
        })
        .collect();
    // The first of those equally close, as the players are listed.
    let closest = |of: &mut dyn Iterator<Item = (usize, f64)>| {
        of.fold(None, |best: Option<(usize, f64)>, o| match best {
            Some(b) if b.1 <= o.1 => Some(b),
            _ => Some(o),
        })
    };
    let (from, to) = (time - SHOT_BEFORE, time + SHOT_AFTER);
    let mut shooters = (opponents.iter().copied()).filter(|&(i, _)| hud.fired(i, from, to));
    if let Some((i, _)) = closest(&mut shooters) {
        return Some((i, AttackerSource::Shot));
    }
    let (i, off) = closest(&mut opponents.iter().copied())?;
    (off <= AIM_DEGREES).then_some((i, AttackerSource::Aim))
}

/// Reads the hits from the movement stream and names their attackers.
/// `timeline` holds the kills and downs: `(time, victim, attacker)` as
/// indices into the players.
fn hits(
    data: &[u8],
    map: &RecordMap,
    streams: &[StreamInfo],
    players: &[Player],
    clock: &Clock,
    timeline: &[(f64, usize, Option<usize>)],
    warnings: &mut Vec<String>,
) -> Vec<Hit> {
    let bodies: HashMap<u64, usize> = (players.iter().enumerate())
        .filter_map(|(i, p)| Some((u64::from(p.entities.as_ref()?.movement?), i)))
        .collect();
    // The snapshot holds the state the recording started in.
    let records: Vec<(usize, usize, u32)> = blocks(map, streams, MOVEMENT_STREAM)
        .filter_map(|(start, end, frame)| Some((start, end, frame?)))
        .collect();
    let read: Vec<Moved> = records
        .par_iter()
        .map(|&(start, end, frame)| moved(data.get(start..end).unwrap_or_default(), frame, &bodies))
        .collect();
    let cut = read.iter().filter(|m| m.cut).count();
    if cut > 0 {
        warnings.push(format!(
            "{cut} of {} movement records end before their messages do",
            read.len()
        ));
    }
    let found: Vec<RawHit> = read.iter().flat_map(|m| m.hits.iter().copied()).collect();
    if found.is_empty() {
        return Vec::new();
    }
    let tracks = Tracks::follow(
        players.len(),
        read.iter().flat_map(|m| m.poses.iter().copied()),
    );
    let hud = Hud::read(data, map, streams, players, clock);

    let mut out = Vec::with_capacity(found.len());
    // `(time, health)` after each player's last hit.
    let mut last: Vec<Option<(f64, u32)>> = vec![None; players.len()];
    let (mut malformed, mut no_max) = (0, 0);
    for RawHit {
        frame,
        player,
        block,
    } in found
    {
        let result = match block.state {
            1 | 2 => HitResult::Alive,
            STATE_DOWN => HitResult::Down,
            STATE_DEAD => HitResult::Dead,
            _ => {
                malformed += 1;
                continue;
            }
        };
        if block.direction > 7 {
            malformed += 1;
            continue;
        }
        let time = seconds(clock, frame);
        let Some(max) = hud.max_at(player, time) else {
            no_max += 1;
            continue;
        };
        let left = (f64::from(block.ratio) * f64::from(max)).round();
        let health = if result == HitResult::Alive && left > 0.0 {
            left as u32
        } else {
            0
        };
        // The health before: what the last hit left, unless the HUD wrote
        // another since (a heal, a revive). The HUD's value for this very
        // hit can come before the body's update, and its value for the
        // last hit after that one's.
        let written = hud.health_before(player, time - HUD_EARLY);
        let before = match (last[player], written) {
            (Some(hit), Some(hud)) if hud.0 <= hit.0 + HUD_LATE => hit.1,
            (_, Some(hud)) => hud.1,
            (Some(hit), None) => hit.1,
            (None, None) => max,
        };
        last[player] = Some((time, health));
        let damage = (block.ratio >= 0.0)
            .then(|| before.checked_sub(health))
            .flatten();

        let guess = (block.kind == BULLET)
            .then(|| estimate(player, frame, time, players, &tracks, &hud))
            .flatten();
        // The timeline entry closest to a hit that downed or killed is the
        // one it caused; an entry that names nobody leaves it unnamed.
        let caused = (result != HitResult::Alive)
            .then(|| {
                let near = timeline
                    .iter()
                    .filter(|e| e.1 == player && (e.0 - time).abs() <= SAME_EVENT);
                near.min_by(|a, b| (a.0 - time).abs().total_cmp(&(b.0 - time).abs()))
            })
            .flatten();
        let attacker = match caused {
            Some(&(_, _, by)) => by.map(|by| (by, AttackerSource::Timeline)),
            None => guess,
        };
        let name = |i: usize| players.get(i).map(|p| p.username.clone());
        let apart = attacker.and_then(|(by, _)| {
            let (a, b) = (tracks.before(by, frame)?, tracks.before(player, frame)?);
            Some((distance(a.position, b.position) * 100.0).round() as f32 / 100.0)
        });
        let at = when(clock, frame);
        out.push(Hit {
            username: name(player).unwrap_or_default(),
            damage,
            health,
            result,
            kind: DamageType::new(block.kind),
            multiplier: block.multiplier,
            direction: block.direction,
            by: attacker.and_then(|(by, _)| name(by)),
            attacker_source: attacker.map(|a| a.1),
            distance: apart,
            time: at.time,
            phase: at.phase,
            elapsed: at.elapsed,
            recording_time: at.recording_time,
            estimate: guess.and_then(|(by, rule)| Some((name(by)?, rule))),
        });
    }
    if malformed > 0 {
        warnings.push(format!(
            "{malformed} hit blocks with a life state or direction outside the known ones were left out"
        ));
    }
    if no_max > 0 {
        warnings.push(format!(
            "{no_max} hits were left out: the HUD holds no max health for their victim"
        ));
    }
    out
}

/// Reads the round's timeline and hits.
pub(crate) fn decode(
    data: &[u8],
    map: &RecordMap,
    streams: &[StreamInfo],
    players: &[Player],
    clock: &Clock,
) -> Combat {
    let mut out = Combat::default();
    let ids: HashMap<u64, usize> = (players.iter().enumerate())
        .filter(|(_, p)| p.id != 0)
        .map(|(i, p)| (p.id, i))
        .collect();
    let name = |i: usize| players[i].username.clone();
    // Kills and downs, for the hits that caused them.
    let mut caused: Vec<(f64, usize, Option<usize>)> = Vec::new();
    let read = match timeline_record(data, map) {
        Some([]) => Entries::default(),
        Some(record) => entries(record),
        None => Entries {
            entries: Vec::new(),
            problem: Some("the file has no timeline stream".into()),
        },
    };
    out.warnings.extend(read.problem);
    let mut unnamed = 0;
    for e in &read.entries {
        let Some(&victim) = ids.get(&e.victim) else {
            unnamed += 1;
            continue;
        };
        let by = e.attacker.and_then(|id| ids.get(&id).copied());
        if e.attacker.is_some() && by.is_none() {
            unnamed += 1;
        }
        if matches!(
            e.kind,
            TimelineKind::Kill | TimelineKind::TeamKill | TimelineKind::Down
        ) {
            caused.push((seconds(clock, e.frame), victim, by));
        }
        let at = when(clock, e.frame);
        out.events.push(TimelineEvent {
            kind: e.kind,
            username: name(victim),
            by: by.map(name),
            weapon: e.weapon,
            headshot: e.headshot,
            time: at.time,
            phase: at.phase,
            elapsed: at.elapsed,
            recording_time: at.recording_time,
        });
    }
    if unnamed > 0 {
        out.warnings.push(format!(
            "{unnamed} timeline entries name a player who is not in the round"
        ));
    }
    out.hits = hits(
        data,
        map,
        streams,
        players,
        clock,
        &caused,
        &mut out.warnings,
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ATTACKER: u64 = 0x1111_2222_3333_4444;
    const VICTIM: u64 = 0x5555_6666_7777_8888;

    fn header(d: &mut Vec<u8>, kind: u8, frame: u32) {
        d.push(kind);
        d.extend(frame.to_le_bytes());
    }

    /// `body, attacker, icon, team, victim, icon, team`: 48 bytes.
    fn pair(d: &mut Vec<u8>, attacker: u64, victim: u64, teams: (u32, u32)) {
        d.extend(0xF000_0001u64.to_le_bytes());
        d.extend(attacker.to_le_bytes());
        d.extend(77u64.to_le_bytes());
        d.extend(teams.0.to_le_bytes());
        d.extend(victim.to_le_bytes());
        d.extend(78u64.to_le_bytes());
        d.extend(teams.1.to_le_bytes());
    }

    fn kill(d: &mut Vec<u8>, kind: u8, frame: u32, teams: (u32, u32), headshot: bool) {
        header(d, kind, frame);
        d.extend(500u64.to_le_bytes());
        pair(d, ATTACKER, VICTIM, teams);
        d.push(u8::from(headshot));
    }

    fn death(d: &mut Vec<u8>, frame: u32) {
        header(d, DEATH, frame);
        d.extend(600u64.to_le_bytes());
        d.extend(0xF000_0002u64.to_le_bytes());
        d.extend(VICTIM.to_le_bytes());
        d.extend(78u64.to_le_bytes());
        d.extend(1u32.to_le_bytes());
    }

    fn paired(d: &mut Vec<u8>, kind: u8, frame: u32, attacker: u64) {
        header(d, kind, frame);
        pair(d, attacker, VICTIM, (0, 1));
    }

    fn phase(d: &mut Vec<u8>, frame: u32) {
        header(d, PHASE, frame);
        d.extend(3u32.to_le_bytes());
    }

    fn defuser(d: &mut Vec<u8>, frame: u32) {
        header(d, DEFUSER, frame);
        d.extend(ATTACKER.to_le_bytes());
        d.extend(77u64.to_le_bytes());
        d.extend(0u32.to_le_bytes());
        d.push(1);
    }

    /// A record of `count` entries holding `body`.
    fn record(count: u32, body: &[u8]) -> Vec<u8> {
        let mut d = count.to_le_bytes().to_vec();
        d.extend(body);
        d
    }

    #[test]
    fn every_entry_type_parses() {
        let mut d = vec![];
        phase(&mut d, 10);
        paired(&mut d, DOWN, 20, ATTACKER);
        kill(&mut d, KILL, 30, (0, 1), true);
        kill(&mut d, TEAM_KILL, 40, (1, 1), false);
        death(&mut d, 50);
        defuser(&mut d, 60);
        paired(&mut d, DOWN, 70, NOBODY);
        paired(&mut d, REVIVE, 80, VICTIM);
        let read = entries(&record(8, &d));
        assert_eq!(read.problem, None);
        let e = &read.entries;
        let kinds: Vec<_> = e.iter().map(|e| (e.kind, e.frame)).collect();
        use TimelineKind::*;
        assert_eq!(
            kinds,
            [
                (Down, 20),
                (Kill, 30),
                (TeamKill, 40),
                (Death, 50),
                (Down, 70),
                (Revive, 80)
            ],
            "phase changes and defuser events are passed over"
        );
        assert_eq!(
            e[1],
            Entry {
                kind: Kill,
                frame: 30,
                attacker: Some(ATTACKER),
                victim: VICTIM,
                attacker_team: Some(0),
                victim_team: 1,
                weapon: Some(500),
                headshot: Some(true),
            }
        );
        assert_eq!((e[0].weapon, e[0].headshot), (None, None));
        assert_eq!(e[0].attacker, Some(ATTACKER));
        assert_eq!(e[2].headshot, Some(false));
        assert_eq!((e[2].attacker_team, e[2].victim_team), (Some(1), 1));
        assert_eq!(
            (e[3].attacker, e[3].victim, e[3].weapon, e[3].headshot),
            (None, VICTIM, Some(600), None)
        );
        assert_eq!(e[4].attacker, None, "nobody downed the victim");
        assert_eq!(e[5].attacker, Some(VICTIM), "a self-revive");
    }

    #[test]
    fn an_unknown_type_ends_the_read_and_keeps_what_came_before() {
        let mut d = vec![];
        kill(&mut d, KILL, 30, (0, 1), false);
        header(&mut d, 4, 35);
        d.extend([0xAB; 40]);
        kill(&mut d, KILL, 40, (0, 1), false);
        let read = entries(&record(3, &d));
        assert_eq!(read.entries.len(), 1);
        assert!(read.problem.unwrap().contains("unknown type 4"));
    }

    #[test]
    fn a_record_cut_anywhere_keeps_its_whole_entries() {
        let mut d = vec![];
        paired(&mut d, DOWN, 20, ATTACKER);
        let first = 4 + d.len();
        kill(&mut d, KILL, 30, (0, 1), true);
        death(&mut d, 50);
        phase(&mut d, 60);
        defuser(&mut d, 70);
        let whole = record(5, &d);
        assert_eq!(entries(&whole).problem, None);
        for cut in 0..whole.len() {
            let read = entries(&whole[..cut]);
            assert!(read.problem.is_some(), "cut at {cut}");
            assert_eq!(!read.entries.is_empty(), cut >= first, "cut at {cut}");
        }
        // Bytes after the last entry are reported, the entries kept.
        let mut longer = whole.clone();
        longer.push(0);
        let read = entries(&longer);
        assert_eq!(read.entries.len(), 3);
        assert!(read.problem.unwrap().contains("1 bytes follow"));
        // A count the record cannot hold.
        let read = entries(&u32::MAX.to_le_bytes());
        assert!(read.entries.is_empty() && read.problem.is_some());
    }

    const BLOCK: Block = Block {
        ratio: 0.5,
        multiplier: 0.75,
        direction: 3,
        state: 1,
        kind: 0,
    };
    /// A unit quaternion: a quarter turn about the vertical.
    const AIMED: [f32; 4] = [0.0, 0.0, 0.6, 0.8];

    fn block_bytes(b: Block, unknown: u32) -> Vec<u8> {
        let mut d = b.ratio.to_le_bytes().to_vec();
        d.extend(b.multiplier.to_le_bytes());
        for v in [b.direction, b.state, unknown, b.kind] {
            d.extend(v.to_le_bytes());
        }
        d
    }

    /// A tail with these flag words: each field they call for, then
    /// `trailing` bytes.
    fn tail(f: u16, g: u16, trailing: usize) -> Vec<u8> {
        let mut d = f.to_le_bytes().to_vec();
        d.extend(g.to_le_bytes());
        if f & AIM != 0 {
            d.extend(AIMED.iter().flat_map(|v| v.to_le_bytes()));
        }
        for flag in FLOATS {
            if f & flag != 0 {
                d.extend(0.25f32.to_le_bytes());
            }
        }
        if f & BYTE != 0 {
            d.push(1);
        }
        if f & HIT != 0 {
            d.extend(block_bytes(BLOCK, 1));
        }
        if g & G_FLOAT != 0 {
            d.extend([0x30, 0xFE, 0x44, 0xA6]);
            d.extend(0.25f32.to_le_bytes());
        }
        d.extend(vec![0xAB; trailing]);
        d
    }

    /// A `607385fe` update of `size` bytes at `position`, ending in `tail`.
    fn update(size: usize, position: [f32; 3], tail: &[u8]) -> Vec<u8> {
        let mut d = UPDATE.to_vec();
        d.extend([0xB8, 0x03]);
        d.extend(position.iter().flat_map(|v| v.to_le_bytes()));
        d.resize(size - tail.len(), 0xAB);
        d.extend(tail);
        d
    }

    const HERE: [f32; 3] = [10.0, 20.0, 1.5];

    #[test]
    fn the_tail_is_read_for_every_combination_of_its_fields() {
        let fields = [AIM, FLOATS[0], FLOATS[1], FLOATS[2], FLOATS[3], BYTE, HIT];
        for chosen in 0..1u16 << fields.len() {
            let mut f = 0;
            for (i, flag) in fields.iter().enumerate() {
                if chosen & (1 << i) != 0 {
                    f |= flag;
                }
            }
            // The high bits are set in some real tails (`46c2`, `82c2`).
            for high in [0, 0x4000, 0x8000] {
                for g in [0, G_FLOAT] {
                    for trailing in 1..=3 {
                        let u = update(900, HERE, &tail(f | high, g, trailing));
                        let what = format!("F {:04x} G {g:04x}, {trailing} trailing", f | high);
                        let hit = if f & HIT != 0 { vec![BLOCK] } else { vec![] };
                        assert_eq!(hit_blocks(&u), hit, "{what}");
                        assert_eq!(tail_aim(&u), (f & AIM != 0).then_some(AIMED), "{what}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_hit_block_needs_its_flag_and_fields_in_range() {
        let with = |block: Block, unknown: u32, f: u16| {
            let mut t = f.to_le_bytes().to_vec();
            t.extend([0, 0]);
            t.extend(block_bytes(block, unknown));
            t.push(0);
            hit_blocks(&update(100, HERE, &t))
        };
        assert_eq!(with(BLOCK, 1, HIT), [BLOCK]);
        // The flag word does not announce a hit block, or has a bit no tail
        // with one has.
        assert!(with(BLOCK, 1, 0).is_empty());
        assert!(with(BLOCK, 1, HIT | 0x0004).is_empty());
        // A hit that kills leaves a ratio below zero; one that takes all
        // health leaves exactly zero.
        for ratio in [-0.1727, 0.0, 1.0] {
            let b = Block { ratio, ..BLOCK };
            assert_eq!(with(b, 1, HIT), [b], "ratio {ratio}");
        }
        let out_of_range = [
            Block {
                ratio: 1e-9,
                ..BLOCK
            },
            Block {
                ratio: 1.5,
                ..BLOCK
            },
            Block {
                ratio: -4.0,
                ..BLOCK
            },
            Block {
                ratio: f32::NAN,
                ..BLOCK
            },
            Block {
                multiplier: 0.0,
                ..BLOCK
            },
            Block {
                multiplier: 1.5,
                ..BLOCK
            },
            Block {
                direction: 64,
                ..BLOCK
            },
            Block { state: 0, ..BLOCK },
            Block { state: 9, ..BLOCK },
            Block { kind: 64, ..BLOCK },
        ];
        for b in out_of_range {
            assert!(with(b, 1, HIT).is_empty(), "{b:?}");
        }
        assert!(with(BLOCK, 0, HIT).is_empty());
        assert!(with(BLOCK, 9, HIT).is_empty());
    }

    #[test]
    fn an_update_cut_anywhere_is_read_without_a_hit_it_does_not_hold() {
        let whole = update(900, HERE, &tail(AIM | BYTE | HIT, G_FLOAT, 2));
        assert_eq!(hit_blocks(&whole), [BLOCK]);
        // The block ends before the `G` float (8 bytes) and 2 more bytes.
        let block_end = whole.len() - 10;
        for cut in 0..whole.len() {
            let u = &whole[..cut];
            assert_eq!(hit_blocks(u).len(), usize::from(cut >= block_end), "{cut}");
            // Cut in the bytes after the block, the tail can read as whole.
            assert!(tail_aim(u).is_none() || cut > block_end, "cut at {cut}");
        }
        // Too short to be searched, though it ends in a block.
        let mut small = (HIT.to_le_bytes()).to_vec();
        small.extend([0, 0]);
        small.extend(block_bytes(BLOCK, 1));
        assert_eq!(hit_blocks(&update(HIT_UPDATE, HERE, &small)), [BLOCK]);
        assert!(hit_blocks(&update(HIT_UPDATE - 1, HERE, &small)).is_empty());
    }

    /// A movement record holding these `(entity, payload)` messages.
    fn record_of(messages: &[(u64, Vec<u8>)]) -> Vec<u8> {
        let mut d = (messages.len() as u16).to_le_bytes().to_vec();
        for (entity, payload) in messages {
            d.extend(entity.to_le_bytes());
            d.extend((payload.len() as u32).to_le_bytes());
            d.extend(payload);
        }
        d
    }

    #[test]
    fn a_movement_record_gives_the_hits_and_poses_of_the_bodies() {
        const BODY: u64 = 0xF02B_8AEF;
        let bodies = HashMap::from([(BODY, 4)]);
        let hit = update(900, HERE, &tail(AIM | HIT, 0, 1));
        let mut other_type = hit.clone();
        other_type[0] = 0x61;
        let mut d = record_of(&[
            // Another entity's, and another payload type.
            (BODY + 1, hit.clone()),
            (BODY, other_type),
            (BODY, hit),
            // Too small to carry a position.
            (BODY, update(100, HERE, &tail(AIM, 0, 1))),
        ]);
        let m = moved(&d, 77, &bodies);
        assert_eq!(
            m.hits,
            [RawHit {
                frame: 77,
                player: 4,
                block: BLOCK
            }]
        );
        assert_eq!(
            m.poses,
            [Pose {
                frame: 77,
                player: 4,
                position: HERE,
                aim: Some(AIMED)
            }]
        );
        assert!(!m.cut);
        // A record that counts a message it does not hold.
        d[0] = 5;
        let m = moved(&d, 77, &bodies);
        assert!(m.cut && m.hits.len() == 1);
        assert!(!moved(&[], 77, &bodies).cut);
    }

    fn pose(frame: u32, player: usize, position: [f32; 3], aim: Option<[f32; 4]>) -> Pose {
        Pose {
            frame,
            player,
            position,
            aim,
        }
    }

    #[test]
    fn a_body_keeps_its_last_position_and_aim() {
        let far = [400.0, 0.0, 0.0];
        let t = Tracks::follow(
            2,
            [
                // No position yet: nothing to keep.
                pose(1, 0, [0.0; 3], Some(AIMED)),
                pose(2, 0, HERE, None),
                pose(3, 0, [10.5, 20.0, 1.5], Some(AIMED)),
                // At the origin, off the map and a jump are not positions.
                pose(4, 0, [0.0; 3], None),
                pose(5, 0, [600.0, 0.0, 0.0], None),
                pose(6, 0, far, None),
                // A player the round does not have.
                pose(6, 9, HERE, None),
            ]
            .into_iter(),
        );
        assert_eq!(t.players[0].len(), 5);
        assert_eq!(t.before(0, 2), None);
        assert_eq!(t.before(0, 3), Some(&pose(2, 0, HERE, None)));
        let last = t.before(0, 100).unwrap();
        assert_eq!(
            (last.frame, last.position[0], last.aim),
            (6, 10.5, Some(AIMED))
        );
        assert_eq!(t.before(1, 100), None);
        assert_eq!(t.before(9, 100), None);
    }

    #[test]
    fn aim_is_forward_along_y() {
        let forward = [0.0, 1.0, 0.0];
        let ahead = rotate([0.0, 0.0, 0.0, 1.0], forward);
        assert_eq!(degrees_between(ahead, [0.0, 3.0, 0.0]), Some(0.0));
        // `AIMED` turns about the vertical by 2 * atan(0.6 / 0.8).
        let turned = degrees_between(rotate(AIMED, forward), forward).unwrap();
        assert!((turned - 73.74).abs() < 0.01, "{turned}");
        assert_eq!(degrees_between(forward, [0.0; 3]), None);
    }

    /// `23 <obj> 00000000 <hash> 04 <value>`.
    fn set(d: &mut Vec<u8>, obj: u32, hash: Hash, value: u32) {
        d.push(0x23);
        d.extend(obj.to_le_bytes());
        d.extend([0; 4]);
        d.extend(hash);
        d.push(4);
        d.extend(value.to_le_bytes());
    }

    /// `22 <hash> 04 <value>`.
    fn prop(d: &mut Vec<u8>, hash: Hash, value: u32) {
        d.push(0x22);
        d.extend(hash);
        d.push(4);
        d.extend(value.to_le_bytes());
    }

    /// `1b <parent> 00000000 <field> <child> 00000000 <class>`.
    fn link(d: &mut Vec<u8>, parent: u32, child: u32) {
        d.push(0x1B);
        d.extend(parent.to_le_bytes());
        d.extend([0; 4]);
        d.extend([9, 9, 9, 9]);
        d.extend(child.to_le_bytes());
        d.extend([0; 4]);
        d.extend([8, 8, 8, 8]);
    }

    const CONTROLLER: u32 = 0xF000_0001;
    const LIFE: u32 = 0xF000_0002;
    const GUN: u32 = 0xF000_0010;

    /// A HUD of two players; the second has a controller and a gun's
    /// ammunition two links under it.
    fn hud_of(frames: &[(Option<f64>, Vec<u8>)]) -> Hud {
        let mut tree = Tree {
            life: HashMap::from([(LIFE, 1)]),
            controllers: HashMap::from([(CONTROLLER, 1)]),
            ..Tree::default()
        };
        let mut hud = Hud::new(2);
        for (time, bytes) in frames {
            let values = seen(bytes, &tree.life);
            hud.add(&mut tree, *time, &values);
        }
        hud
    }

    #[test]
    fn the_hud_gives_health_and_shots() {
        let mut snapshot = vec![];
        link(&mut snapshot, CONTROLLER, GUN - 1);
        link(&mut snapshot, GUN - 1, GUN);
        set(&mut snapshot, GUN, TOTAL_AMMO, 31);
        set(&mut snapshot, LIFE, HEALTH, 0);
        prop(&mut snapshot, MAX_HEALTH, 0);
        // Not a player's health object.
        set(&mut snapshot, LIFE + 1, HEALTH, 55);
        let frame = |values: &[(u32, Hash, u32)]| {
            let mut d = vec![];
            for &(obj, hash, v) in values {
                set(&mut d, obj, hash, v);
            }
            d
        };
        let hud = hud_of(&[
            (None, snapshot),
            // A drop in the snapshot's wake is a shot; a rise (a reload from
            // another pool) and a resend are not.
            (
                Some(1.0),
                frame(&[(LIFE, MAX_HEALTH, 110), (LIFE, HEALTH, 110)]),
            ),
            (Some(2.0), frame(&[(GUN, TOTAL_AMMO, 30)])),
            (Some(2.5), frame(&[(GUN, TOTAL_AMMO, 30)])),
            (
                Some(3.0),
                frame(&[(GUN, TOTAL_AMMO, 61), (LIFE, HEALTH, 81)]),
            ),
            (
                Some(4.0),
                frame(&[(GUN, TOTAL_AMMO, 60), (LIFE, MAX_HEALTH, 135)]),
            ),
            // A gun under no controller.
            (Some(5.0), frame(&[(GUN + 5, TOTAL_AMMO, 9)])),
            (Some(6.0), frame(&[(GUN + 5, TOTAL_AMMO, 8)])),
        ]);
        assert_eq!(hud.shots, [vec![], vec![2.0, 4.0]]);
        assert!(hud.fired(1, 1.9, 2.0) && hud.fired(1, 2.0, 2.1));
        assert!(!hud.fired(1, 2.01, 3.99) && !hud.fired(0, 0.0, 9.0));
        assert_eq!(
            hud.health[1],
            [(f64::NEG_INFINITY, 0), (1.0, 110), (3.0, 81)]
        );
        assert_eq!(hud.health_before(1, 3.0), Some((1.0, 110)));
        assert_eq!(hud.health_before(1, 3.5), Some((3.0, 81)));
        // The max health in force; before the first, the first.
        assert_eq!(hud.max_at(1, 0.5), Some(110));
        assert_eq!(hud.max_at(1, 3.5), Some(110));
        assert_eq!(hud.max_at(1, 4.0), Some(135));
        assert_eq!(hud.max_at(0, 4.0), None);
    }

    #[test]
    fn a_shot_in_the_snapshot_is_not_one() {
        let mut snapshot = vec![];
        link(&mut snapshot, CONTROLLER, GUN);
        set(&mut snapshot, GUN, TOTAL_AMMO, 31);
        set(&mut snapshot, GUN, TOTAL_AMMO, 30);
        assert!(hud_of(&[(None, snapshot)]).shots[1].is_empty());
    }

    /// Three players: a victim, and two opponents 10 m away on either
    /// side, each aiming as given.
    fn duel(aims: [Option<[f32; 4]>; 2], shots: [&[f64]; 2]) -> Option<(usize, AttackerSource)> {
        let players: Vec<Player> = [0, 1, 1, 0]
            .into_iter()
            .map(|team_index| Player {
                team_index,
                ..Player::default()
            })
            .collect();
        let tracks = Tracks::follow(
            4,
            [
                pose(1, 0, [0.0, 10.0, 1.0], None),
                pose(1, 1, [0.0, 0.1, 1.0], aims[0]),
                pose(1, 2, [0.0, 20.0, 1.0], aims[1]),
                // A teammate aiming straight at the victim.
                pose(1, 3, [0.0, 5.0, 1.0], Some(AHEAD)),
            ]
            .into_iter(),
        );
        let mut hud = Hud::new(4);
        hud.shots[1] = shots[0].to_vec();
        hud.shots[2] = shots[1].to_vec();
        // The teammate fired, too.
        hud.shots[3] = vec![10.0];
        estimate(0, 5, 10.0, &players, &tracks, &hud)
    }

    /// Aiming along +Y, along -Y, and 16 degrees off +Y.
    const AHEAD: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    const BEHIND: [f32; 4] = [0.0, 0.0, 1.0, 0.0];
    const ASIDE: [f32; 4] = [0.0, 0.0, 0.139_173_1, 0.990_268_1];

    #[test]
    fn the_attacker_is_who_fired_then_who_aimed() {
        use AttackerSource::*;
        // Player 1 faces the victim along +Y, player 2 along -Y.
        let both = [Some(AHEAD), Some(BEHIND)];
        assert_eq!(duel(both, [&[10.0], &[]]), Some((1, Shot)));
        assert_eq!(duel(both, [&[], &[10.0]]), Some((2, Shot)));
        // A shot from 140 ms before the hit to 70 ms after it counts.
        assert_eq!(duel(both, [&[], &[9.861]]), Some((2, Shot)));
        assert_eq!(duel(both, [&[], &[10.069]]), Some((2, Shot)));
        // Nobody fired in time: the first of those aiming at the victim.
        assert_eq!(duel(both, [&[], &[9.85, 10.08]]), Some((1, Aim)));
        // Both fired: the one aiming closest, and the one who fired wins
        // over one who only aims.
        assert_eq!(
            duel([Some(ASIDE), Some(BEHIND)], [&[10.0], &[10.0]]),
            Some((2, Shot))
        );
        assert_eq!(
            duel([Some(ASIDE), Some(BEHIND)], [&[10.0], &[]]),
            Some((1, Shot))
        );
        // Aim alone has to be within 10 degrees.
        assert_eq!(duel([Some(ASIDE), Some(AHEAD)], [&[], &[]]), None);
        assert_eq!(
            duel([Some(ASIDE), Some(BEHIND)], [&[], &[]]),
            Some((2, Aim))
        );
        // An opponent whose aim is not known is never named, and neither is
        // a teammate.
        assert_eq!(duel([None, None], [&[10.0], &[10.0]]), None);
        assert_eq!(duel([None, Some(BEHIND)], [&[10.0], &[]]), Some((2, Aim)));
    }

    #[test]
    fn damage_types_are_named() {
        assert_eq!(DamageType::new(0).name, Some("bullet"));
        assert_eq!(DamageType::new(36).name, Some("fire"));
        assert_eq!(DamageType::new(12), DamageType { id: 12, name: None });
    }
}
