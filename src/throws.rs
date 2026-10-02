//! Thrown and launched objects (Y11S3): grenades, thrown gadgets, drones
//! and the projectiles of launchers, from the `movement` stream.
//!
//! The stream's messages (see [`crate::loadout`]) create an entity with a
//! `617385fe` payload, which lists the classes of its components, move it
//! with `607385fe` payloads and remove it with a `637385fe` payload. An
//! update holds the parts its mask names, in this order:
//!
//! ```text
//! +0  607385fe      +4  u16 mask
//! mask & 0080   the entity has a place of its own in the world, and then
//!   & 0100   f32 x, y, z (metres, z up), 4 zero bytes
//!   & 0200   f32 x 4 rotation quaternion
//!   & 0400   u8
//!   & 0800   u16
//!   & 1000   variable
//! mask & (0040 >> i)   the component of class i of the create message
//! ```
//!
//! Without `0080` the entity is attached to another one, and the bits
//! `0100`-`0800` are not a place; they are not read.
//!
//! Everything a player can let go of has a component of class `8490f616`,
//! which says whose it is and whether it is out of their hands:
//!
//! ```text
//! u8 submask
//!   & 01   u16
//!   & 02   u64 the owner's playerid
//!   & 04   u32 the owner's alliance
//!   & 08   u8  1 released, 0 taken back
//! ```
//!
//! A throw is an update that sets the flag to 1. The same message carries
//! where the object left the hand, and every update after it one position,
//! about 30 a second while the object moves. The game writes no velocity:
//! direction and speed are taken from the second and third position, as
//! the first step is often cut short. The flight is the run of positions
//! without a pause; where it stops is where the object came to rest. A
//! drone goes on to be driven without a pause, so its flight is cut at the
//! landing: the first impact that leaves it without vertical speed.
//!
//! The component comes in a few forms. `0e` (owner, alliance, flag) is a
//! throw by hand, `08` (the flag alone) the projectile of a launcher, whose
//! owner was written when it was loaded, and `0a` (owner, flag) an object
//! that another one let go: a charge of a Candela, a puck of a cluster
//! charge, a swarm of a Kawan hive. The component is the last thing in the
//! message. When components of unknown size come before it, it is found
//! from the end: the longest form that fits and names a player. When one
//! comes after it (the full state a shock drone or an ARGUS camera is
//! created with), it is found by the player it names. The form `0a` is
//! also written for a drone and for a launcher's grenade, so it marks a
//! sub-munition only for what no player carries or fires.
//!
//! What was thrown is the slot of the thrower's body whose asset is the
//! object's: `PrimaryGadget` (the ability), `SecondaryGadget` or `Drone`.
//! A launcher's ammunition is in no slot; [`AMMUNITION`] names it.
//!
//! The object ends with its `637385fe` message, or with the flag going
//! back to 0 as the game returns it to its pool. Neither is a detonation:
//! the game writes no marker for one.
//!
//! The round clock is in another stream, so a release takes the reading
//! that was in force in its frame.

use std::collections::HashMap;

use rayon::prelude::*;
use serde::Serialize;

use crate::entities::Hash;
use crate::loadout::{
    DESCRIPTOR, Descriptor, Input, MOVEMENT_STREAM, UPDATE, When, descriptor, hud_items, messages,
};
use crate::types::item_name;

/// Movement payload type that removes an entity.
const DELETE: Hash = [0x63, 0x73, 0x85, 0xFE];
/// Class of the component that names an object's owner.
const OWNER: Hash = [0x84, 0x90, 0xF6, 0x16];
/// Class only objects that are driven have: drones.
const DRIVEN: Hash = [0x47, 0xE5, 0xF6, 0x00];
/// Body slots that hold what a player throws: `PrimaryGadget`,
/// `SecondaryGadget`, `TertiaryGadget` and `Drone`.
const CARRIED: [(Hash, Slot); 4] = [
    ([0x08, 0x2C, 0xA3, 0x1D], Slot::Ability),
    ([0xD8, 0x55, 0xB4, 0xAF], Slot::Gadget),
    ([0x41, 0x20, 0x14, 0x8B], Slot::Gadget),
    ([0x60, 0x3E, 0x4A, 0x2F], Slot::Drone),
];

/// Update mask: the entity has a place of its own, and what of it follows.
const WORLD: u16 = 0x0080;
const POSITION: u16 = 0x0100;
const ROTATION: u16 = 0x0200;
const LIVE: u16 = 0x0400;
const FLAGS: u16 = 0x0800;
/// Parts of unknown size.
const VARIABLE: u16 = 0x3000;
/// Every part of the place: the full state an entity is created with.
const FULL: u16 = 0x1F80;
/// Bit of the first class's component; each next class is one bit lower.
const FIRST_CLASS: u16 = 0x0040;

/// Owner component submask.
const HAS_COUNTER: u8 = 0x01;
const HAS_OWNER: u8 = 0x02;
const HAS_ALLIANCE: u8 = 0x04;
const HAS_RELEASED: u8 = 0x08;
/// The form of an object let go by another object.
const SUB_MUNITION: u8 = HAS_OWNER | HAS_RELEASED;
/// The longest owner component: every part present.
const OWNER_SIZE: usize = 16;
/// Alliances are small numbers; a larger one is not an owner component.
const MAX_ALLIANCE: u32 = 5;

/// Positions come every 0.03 s while an object moves; a pause longer than
/// this ends its flight.
const PAUSE: f64 = 0.25;
/// A rise of the vertical speed by this much between two steps (m/s) is an
/// impact, and an object leaving one slower than this has landed.
const IMPACT: f64 = 1.0;
/// Steps whose velocity is within this of the first step's are the steady
/// ones a drone leaves the hand with (m/s; gravity adds 0.3 a step).
const STEADY: f64 = 0.5;
/// Path points closer than this to the one before are left out (metres).
const STEP: f64 = 0.05;
/// The most points a path keeps.
const PATH_POINTS: usize = 60;

/// How an object no body slot names leaves its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// Fired from a launcher.
    Launcher,
    /// Thrown by hand, from the ability slot.
    Hand,
    /// Let go by another object.
    Object,
}

/// Ammunition of launchers and what it splits into, by asset. No slot
/// names these: each was given to the launcher of the only operator who
/// fires it, so the names are inferred.
const AMMUNITION: [(u64, &str, Source); 20] = [
    (391794748337, "X-KAIROS Pellet", Source::Launcher),
    (385049526209, "Kawan Hive", Source::Launcher),
    (385049618000, "Kawan Hive Swarm", Source::Object),
    (391794703535, "Breaching Round", Source::Launcher),
    (391794756179, "Shumikha Grenade", Source::Launcher),
    (391794727583, "Tactical Crossbow Bolt", Source::Launcher),
    (391794729374, "Tactical Crossbow Bolt", Source::Launcher),
    (311776456053, "KS79 Lifeline Grenade", Source::Launcher),
    (392233332963, "KS79 Lifeline Grenade", Source::Launcher),
    (392233335597, "Gonne-6 Round", Source::Launcher),
    (446645529444, "Horus Lance", Source::Launcher),
    (392233337653, "LV Explosive Lance", Source::Launcher),
    (383322885052, "Airjab Repulsion Grenade", Source::Launcher),
    (391794754080, "Pest", Source::Launcher),
    (391794758284, "ARGUS Camera", Source::Launcher),
    (416011594318, "D.O.M. Panel", Source::Launcher),
    (391794738235, "Stim Dart", Source::Launcher),
    (370092345902, "Candela", Source::Hand),
    (378015312553, "Candela Flash Charge", Source::Object),
    (378015312549, "Cluster Charge Puck", Source::Object),
];

/// The loadout slot a thrown object came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Slot {
    Ability,
    Gadget,
    Drone,
}

/// How a thrown object ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Ended {
    /// The game removed it: it went off, was destroyed or was picked up.
    Deleted,
    /// The game took it back into its pool.
    Returned,
}

/// One throw or launch. `when` is the release.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Throw {
    pub username: String,
    /// The slot of the thrower's loadout the object is from. Absent for
    /// the ammunition of launchers and for sub-munitions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<Slot>,
    /// The object's asset id.
    pub asset: u64,
    /// The item id of the slot, as `loadouts` gives it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
    /// What the object is comes from this module's table, not from the
    /// thrower's body.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub inferred: bool,
    /// Let go by another object, not by the player: a charge of a Candela,
    /// a puck of a cluster charge, a swarm of a hive.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub sub_munition: bool,
    /// Where the object was released: `[x, y, z]` in metres, z up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<[f64; 3]>,
    /// Unit vector of the first step of the flight.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<[f64; 3]>,
    /// Metres a second over that step.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    /// The flight as `[seconds since the release, x, y, z]`, thinned to at
    /// most 60 points.
    pub path: Vec<[f64; 4]>,
    /// Where the flight ended: the landing of a drone, else where the
    /// object came to rest or went off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<[f64; 3]>,
    /// Seconds from the release to `end`.
    pub flight_time: f64,
    /// How the object ended. Absent when it was still there as the
    /// recording stopped, or was thrown again before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<Ended>,
    /// Seconds from the release to that end.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_after: Option<f64>,
    #[serde(flatten)]
    pub when: When,
}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    pub throws: Vec<Throw>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
}

/// An owner component.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Owner {
    submask: u8,
    /// The owner's `playerid`.
    player: Option<u64>,
    alliance: Option<u32>,
    /// 1 released, 0 taken back.
    released: Option<u8>,
}

/// The first `n` bytes of `rest`, which is left with what follows.
fn take<'a>(rest: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    let (head, tail) = rest.split_at_checked(n)?;
    *rest = tail;
    Some(head)
}

/// Parses an owner component that takes up all of `bytes`.
fn owner(bytes: &[u8]) -> Option<Owner> {
    let (out, size) = component(bytes)?;
    (size == bytes.len()).then_some(out)
}

/// Parses the owner component `bytes` start with, and gives its size.
fn component(bytes: &[u8]) -> Option<(Owner, usize)> {
    let (&submask, mut rest) = bytes.split_first()?;
    if submask & !(HAS_COUNTER | HAS_OWNER | HAS_ALLIANCE | HAS_RELEASED) != 0 {
        return None;
    }
    let mut out = Owner {
        submask,
        ..Owner::default()
    };
    if submask & HAS_COUNTER != 0 {
        take(&mut rest, 2)?;
    }
    if submask & HAS_OWNER != 0 {
        out.player = Some(u64::from_le_bytes(take(&mut rest, 8)?.try_into().ok()?));
    }
    if submask & HAS_ALLIANCE != 0 {
        out.alliance = Some(u32::from_le_bytes(take(&mut rest, 4)?.try_into().ok()?));
    }
    if submask & HAS_RELEASED != 0 {
        out.released = Some(*take(&mut rest, 1)?.first()?);
    }
    Some((out, bytes.len() - rest.len()))
}

/// What an update says of a thrown object.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Update {
    position: Option<[f32; 3]>,
    owner: Option<Owner>,
    /// The update has an owner component that could not be read.
    unread: bool,
}

/// Parses a `607385fe` payload of an entity with these `classes`.
/// `players` are the `playerid`s an owner can be. `None` when the payload
/// does not hold what its mask promises.
fn update(payload: &[u8], classes: &[Hash], players: &[u64]) -> Option<Update> {
    if payload.get(..4)? != UPDATE {
        return None;
    }
    let mask = u16::from_le_bytes(payload.get(4..6)?.try_into().ok()?);
    let mut at = 6;
    let mut out = Update::default();
    let world = mask & WORLD != 0;
    if world {
        if mask & POSITION != 0 {
            let p = payload.get(at..at + 12)?;
            let axis = |i: usize| -> Option<f32> {
                let v = f32::from_le_bytes(p.get(4 * i..4 * i + 4)?.try_into().ok()?);
                v.is_finite().then_some(v)
            };
            out.position = Some([axis(0)?, axis(1)?, axis(2)?]);
            at += 16;
        }
        for (bit, size) in [(ROTATION, 16), (LIVE, 1), (FLAGS, 2)] {
            if mask & bit != 0 {
                at += size;
            }
        }
    }
    let rest = payload.get(at..)?;
    let bit = |i: usize| if i < 16 { FIRST_CLASS >> i } else { 0 };
    let Some(i) = classes.iter().position(|c| *c == OWNER) else {
        return Some(out);
    };
    if mask & bit(i) == 0 {
        return Some(out);
    }
    let plausible = |o: &Owner| {
        o.player.is_none_or(|p| players.contains(&p))
            && o.alliance.is_none_or(|a| a <= MAX_ALLIANCE)
            && o.released.is_none_or(|r| r <= 1)
    };
    // A component after the owner's has no known size to step back over:
    // the owner's is the one that names a player, after its submask or
    // after the submask and the u16.
    if (i + 1..classes.len()).any(|j| mask & bit(j) != 0) {
        out.owner = players.iter().find_map(|id| {
            memchr::memmem::find_iter(rest, &id.to_le_bytes()).find_map(|at| {
                [3, 1].into_iter().find_map(|back| {
                    let (o, _) = component(rest.get(at.checked_sub(back)?..)?)?;
                    (o.player == Some(*id) && plausible(&o)).then_some(o)
                })
            })
        });
        // The full state of an object nobody has taken names no player.
        out.unread = out.owner.is_none() && mask & FULL != FULL;
        return Some(out);
    }
    let alone = world && mask & VARIABLE == 0 && (0..i).all(|j| mask & bit(j) == 0);
    out.owner = if alone {
        owner(rest)
    } else {
        // It is the tail of the message: the longest form that fits, down
        // to the single byte of an empty one.
        (1..=OWNER_SIZE.min(rest.len()))
            .rev()
            .find_map(|n| owner(rest.get(rest.len() - n..)?).filter(plausible))
    };
    out.unread = out.owner.is_none();
    Some(out)
}

/// A thrown object while its messages are read.
#[derive(Debug, Default)]
struct Tracked {
    classes: Vec<Hash>,
    asset: u64,
    owner: Option<u64>,
    /// Index of its release that has not ended.
    open: Option<usize>,
}

/// A message of a thrown object: `(entity, start, end, frame)`, the offsets
/// being those of its payload in the data.
type Message = (u64, usize, usize, Option<u32>);

/// One release as the stream gave it.
#[derive(Debug)]
struct Release {
    asset: u64,
    owner: Option<u64>,
    submask: u8,
    driven: bool,
    frame: Option<u32>,
    /// Seconds since the recording started.
    time: f64,
    /// `(seconds since the release, position)`.
    points: Vec<(f64, [f32; 3])>,
    ended: Option<(Ended, f64)>,
}

/// How many of `points` are the flight: up to the first pause, and for a
/// driven object up to its landing.
fn flight(points: &[(f64, [f32; 3])], driven: bool) -> usize {
    let run = points
        .windows(2)
        .position(|w| w[1].0 - w[0].0 > PAUSE)
        .map_or(points.len(), |i| i + 1);
    if !driven {
        return run;
    }
    // Velocity of each step; step `k` ends at point `k + 1`.
    let velocity: Vec<[f64; 3]> = points
        .windows(2)
        .take(run.saturating_sub(1))
        .map(|w| {
            let dt = w[1].0 - w[0].0;
            [0, 1, 2].map(|i| {
                if dt > 0.0 {
                    f64::from(w[1].1[i] - w[0].1[i]) / dt
                } else {
                    0.0
                }
            })
        })
        .collect();
    let rise = |k: usize| velocity.get(k).map_or(0.0, |v| v[2]);
    // A drone leaves the hand at a steady 15.9 m/s for a few steps and
    // then slows to its own speed at once. That is no impact: impacts are
    // looked for after the step that ends the steady ones.
    let steady = |v: &[f64; 3]| {
        let first = velocity.first().unwrap_or(v);
        (0..3)
            .map(|i| (v[i] - first[i]).powi(2))
            .sum::<f64>()
            .sqrt()
            <= STEADY
    };
    let lead = velocity.iter().position(|v| !steady(v));
    let mut k = lead.unwrap_or(velocity.len()) + 2;
    while k < velocity.len() {
        if rise(k) - rise(k - 1) > IMPACT {
            // The impact lasts while the speed keeps rising.
            let first = k;
            while k + 1 < velocity.len() && rise(k + 1) - rise(k) > IMPACT {
                k += 1;
            }
            if rise(k).abs() < IMPACT {
                return first + 2;
            }
        }
        k += 1;
    }
    run
}

/// `points` without those closer than [`STEP`] to the one kept before,
/// and no more than [`PATH_POINTS`] of them. The first and last stay.
pub(crate) fn thin(points: &[(f64, [f32; 3])]) -> Vec<(f64, [f32; 3])> {
    let mut out: Vec<(f64, [f32; 3])> = Vec::new();
    for (i, p) in points.iter().enumerate() {
        let far = out.last().is_none_or(|l| distance(l.1, p.1) >= STEP);
        if far || i + 1 == points.len() {
            out.push(*p);
        }
    }
    if out.len() > PATH_POINTS {
        let last = out.len() - 1;
        out = (0..PATH_POINTS)
            .filter_map(|i| out.get(i * last / (PATH_POINTS - 1)).copied())
            .collect();
    }
    out
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f64 {
    (a.iter().zip(b))
        .map(|(a, b)| f64::from(b - a).powi(2))
        .sum::<f64>()
        .sqrt()
}

fn round(v: f64, digits: i32) -> f64 {
    let scale = 10f64.powi(digits);
    (v * scale).round() / scale
}

pub(crate) fn place(p: [f32; 3]) -> [f64; 3] {
    p.map(|v| round(f64::from(v), 3))
}

/// Unit vector and metres a second of the step from `a` to `b`.
fn heading(a: (f64, [f32; 3]), b: (f64, [f32; 3])) -> Option<([f64; 3], f64)> {
    let (dt, length) = (b.0 - a.0, distance(a.1, b.1));
    (dt > 0.0 && length > 0.0).then(|| {
        let unit = [0, 1, 2].map(|i| round(f64::from(b.1[i] - a.1[i]) / length, 4));
        (unit, round(length / dt, 2))
    })
}

/// The slot of `body` that holds `asset`.
fn slot_of(body: &Descriptor, asset: u64) -> Option<Slot> {
    let holds = |slot: Hash| asset != 0 && body.slot(slot) == Some(asset);
    CARRIED.iter().find(|c| holds(c.0)).map(|c| c.1)
}

/// Reads every throw and launch from the movement stream.
pub(crate) fn decode(input: &Input) -> Decoded {
    let data = input.data;
    let blocks: Vec<(usize, usize, Option<u32>)> = input.blocks(MOVEMENT_STREAM).collect();
    let block = |b: &(usize, usize, Option<u32>)| data.get(b.0..b.1).unwrap_or_default();
    // The stream is most of a replay, so its blocks are read in parallel:
    // once for the descriptors, and once more for the messages of the
    // entities that can be thrown.
    let described: Vec<Vec<Descriptor>> = blocks
        .par_iter()
        .map(|b| {
            messages(block(b))
                .filter(|m| m.2.starts_with(&DESCRIPTOR))
                .filter_map(|(entity, _, payload)| {
                    descriptor(payload).filter(|d| d.entity == entity)
                })
                // What can be thrown, and the bodies that carry it.
                .filter(|d| {
                    d.classes.contains(&OWNER) || CARRIED.iter().any(|c| d.slot(c.0).is_some())
                })
                .collect()
        })
        .collect();
    // Every body there was, and the latest of each entity.
    let mut bodies: Vec<&Descriptor> = Vec::new();
    let mut latest: HashMap<u64, &Descriptor> = HashMap::new();
    let mut thrown: Vec<u64> = Vec::new();
    for d in described.iter().flatten() {
        if d.classes.contains(&OWNER) {
            thrown.push(d.entity);
        } else {
            bodies.push(d);
            latest.insert(d.entity, d);
        }
    }
    thrown.sort_unstable();
    thrown.dedup();
    let mut out = Decoded::default();
    if thrown.is_empty() {
        return out;
    }
    let found: Vec<Vec<Message>> = blocks
        .par_iter()
        .map(|b| {
            messages(block(b))
                .filter(|m| thrown.binary_search(&m.0).is_ok())
                .map(|(entity, at, payload)| (entity, b.0 + at, b.0 + at + payload.len(), b.2))
                .collect()
        })
        .collect();

    let ids: Vec<u64> = (input.players.iter())
        .map(|p| p.id)
        .filter(|&i| i != 0)
        .collect();
    let mut tracked: HashMap<u64, Tracked> = HashMap::new();
    let mut releases: Vec<Release> = Vec::new();
    let (mut malformed, mut unread) = (0usize, 0usize);
    for (entity, from, to, frame) in found.into_iter().flatten() {
        let Some(payload) = data.get(from..to) else {
            continue;
        };
        let seconds = input.clock.seconds(frame);
        if payload.starts_with(&DESCRIPTOR) {
            match descriptor(payload) {
                Some(d) => {
                    let t = Tracked {
                        classes: d.classes,
                        asset: d.asset,
                        ..Tracked::default()
                    };
                    tracked.insert(entity, t);
                }
                None => malformed += 1,
            }
            continue;
        }
        let Some(t) = tracked.get_mut(&entity) else {
            continue;
        };
        if payload.starts_with(&DELETE) {
            if let (Some(i), Some(now)) = (t.open.take(), seconds)
                && let Some(r) = releases.get_mut(i)
            {
                r.ended = Some((Ended::Deleted, now - r.time));
            }
            continue;
        }
        if !payload.starts_with(&UPDATE) || !t.classes.contains(&OWNER) {
            continue;
        }
        let Some(u) = update(payload, &t.classes, &ids) else {
            malformed += 1;
            continue;
        };
        unread += usize::from(u.unread);
        if let Some(o) = u.owner {
            t.owner = o.player.or(t.owner);
            match (o.released, seconds) {
                (Some(1), Some(time)) => {
                    t.open = Some(releases.len());
                    releases.push(Release {
                        asset: t.asset,
                        owner: t.owner,
                        submask: o.submask,
                        driven: t.classes.contains(&DRIVEN),
                        frame,
                        time,
                        points: Vec::new(),
                        ended: None,
                    });
                }
                // Released before the recording started: not a throw.
                (Some(1), None) => t.open = None,
                (Some(_), _) => {
                    if let (Some(i), Some(now)) = (t.open.take(), seconds)
                        && let Some(r) = releases.get_mut(i)
                    {
                        r.ended = Some((Ended::Returned, now - r.time));
                    }
                }
                (None, _) => {}
            }
        }
        if let (Some(i), Some(p), Some(now)) = (t.open, u.position, seconds)
            && let Some(r) = releases.get_mut(i)
        {
            r.points.push((now - r.time, p));
        }
    }

    // The frame each clock reading was first shown in; `None` is the
    // opening snapshot.
    let readings = input.clock.reading_offsets;
    let shown: Vec<Option<u32>> = (readings.iter())
        .map(|&at| input.map.frame_at(at))
        .collect();
    let reading_in = |frame: Option<u32>| {
        let i = shown.partition_point(|f| *f <= frame).checked_sub(1);
        i.and_then(|i| readings.get(i)).copied().unwrap_or(0)
    };
    let items = hud_items(input);
    let body = |player: &crate::header::Player| {
        let entity = player.entities.as_ref()?.movement?;
        latest.get(&u64::from(entity)).copied()
    };
    let mut ownerless = 0usize;
    for r in releases {
        let player = (r.owner).and_then(|id| input.players.iter().position(|p| p.id == id));
        let Some((index, player)) = player.and_then(|i| Some((i, input.players.get(i)?))) else {
            ownerless += 1;
            continue;
        };
        // The thrower's body names the slot. A drone can be another
        // body's: the one of the operator the player was before a swap.
        // Without a body of the thrower, any body does when all that
        // carry the asset agree on its slot.
        let own = body(player);
        let carried = own.and_then(|b| slot_of(b, r.asset)).or_else(|| {
            let mut slots = bodies.iter().filter_map(|b| slot_of(b, r.asset));
            let first = slots.next()?;
            (slots.all(|s| s == first) && (own.is_none() || first == Slot::Drone)).then_some(first)
        });
        let ammunition = AMMUNITION.iter().find(|a| a.0 == r.asset);
        let source = ammunition.map(|a| a.2);
        let slot = carried.or((source == Some(Source::Hand)).then_some(Slot::Ability));
        let hud = |i: usize| items.get(index).and_then(|slots| *slots.get(i)?);
        let id = match slot {
            Some(Slot::Ability) => hud(0),
            Some(Slot::Gadget) => hud(1),
            Some(Slot::Drone) | None => None,
        };
        let name = match slot {
            Some(Slot::Drone) => Some("Drone"),
            _ => id.and_then(item_name).or(ammunition.map(|a| a.1)),
        };
        let run = r.points.get(..flight(&r.points, r.driven)).unwrap_or(&[]);
        let step = match run {
            [_, a, b, ..] | [a, b] => heading(*a, *b),
            _ => None,
        };
        out.throws.push(Throw {
            username: player.username.clone(),
            slot,
            asset: r.asset,
            id,
            name,
            inferred: carried.is_none() && ammunition.is_some(),
            sub_munition: match source {
                Some(source) => source == Source::Object,
                None => r.submask == SUB_MUNITION && carried.is_none() && !r.driven,
            },
            origin: run.first().map(|p| place(p.1)),
            direction: step.map(|s| s.0),
            speed: step.map(|s| s.1),
            path: thin(run)
                .into_iter()
                .map(|(t, p)| {
                    let [x, y, z] = place(p);
                    [round(t, 3), x, y, z]
                })
                .collect(),
            end: run.last().map(|p| place(p.1)),
            flight_time: run.last().map_or(0.0, |p| round(p.0, 3)),
            ended: r.ended.map(|e| e.0),
            ended_after: r.ended.map(|e| round(e.1, 3)),
            when: input.clock.when(reading_in(r.frame), r.frame),
        });
    }
    for (count, what) in [
        (malformed, "messages of thrown objects do not parse"),
        (unread, "updates with an owner component that was not read"),
        (ownerless, "releases without a known owner"),
    ] {
        if count > 0 {
            out.warnings.push(format!("{count} {what}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYER: u64 = 0x1122_3344_5566_7788;
    const OTHER: Hash = [0x51, 0x3B, 0x13, 0xB2];

    /// A `607385fe` payload with this mask and body.
    fn message(mask: u16, body: &[u8]) -> Vec<u8> {
        let mut d = UPDATE.to_vec();
        d.extend(mask.to_le_bytes());
        d.extend(body);
        d
    }

    fn position(p: [f32; 3]) -> Vec<u8> {
        let mut d: Vec<u8> = p.iter().flat_map(|v| v.to_le_bytes()).collect();
        d.extend([0; 4]);
        d
    }

    /// A hand throw's component: owner, alliance, released.
    fn thrown(flag: u8) -> Vec<u8> {
        let mut d = vec![0x0E];
        d.extend(PLAYER.to_le_bytes());
        d.extend(4u32.to_le_bytes());
        d.push(flag);
        d
    }

    #[test]
    fn parses_every_form_of_the_owner_component() {
        assert_eq!(
            owner(&thrown(1)),
            Some(Owner {
                submask: 0x0E,
                player: Some(PLAYER),
                alliance: Some(4),
                released: Some(1)
            })
        );
        let launched = owner(&[0x08, 1]).unwrap();
        assert_eq!((launched.player, launched.released), (None, Some(1)));
        assert_eq!(owner(&[0x08, 0]).unwrap().released, Some(0));
        assert_eq!(owner(&[0x01, 7, 0]).unwrap().released, None);
        let mut sub = vec![0x0A];
        sub.extend(PLAYER.to_le_bytes());
        sub.push(1);
        let sub = owner(&sub).unwrap();
        assert_eq!((sub.submask, sub.player), (SUB_MUNITION, Some(PLAYER)));
        let mut full = vec![0x0F, 9, 0];
        full.extend(&thrown(1)[1..]);
        assert_eq!(full.len(), OWNER_SIZE);
        assert_eq!(owner(&full).unwrap().alliance, Some(4));
    }

    #[test]
    fn an_owner_component_must_fill_its_bytes() {
        let good = thrown(1);
        for cut in 0..good.len() {
            assert_eq!(owner(&good[..cut]), None, "cut at {cut}");
        }
        let mut long = good.clone();
        long.push(0);
        assert_eq!(owner(&long), None);
        assert_eq!(owner(&[0x10]), None, "a bit no submask has");
        assert_eq!(owner(&[]), None);
    }

    #[test]
    fn a_release_carries_position_and_owner() {
        // World transform with position, rotation and the live byte, then
        // the component of the second class.
        let mut body = position([1.5, -2.0, 3.25]);
        body.extend([0; 16]);
        body.push(1);
        body.extend(thrown(1));
        let u = update(&message(0x07A0, &body), &[OTHER, OWNER], &[PLAYER]).unwrap();
        assert_eq!(u.position, Some([1.5, -2.0, 3.25]));
        let o = u.owner.unwrap();
        assert_eq!((o.player, o.released), (Some(PLAYER), Some(1)));
        assert!(!u.unread);
        // Without the class's bit there is no owner component.
        let u = update(&message(0x0780, &body[..33]), &[OTHER, OWNER], &[PLAYER]).unwrap();
        assert_eq!(
            (u.position.is_some(), u.owner, u.unread),
            (true, None, false)
        );
    }

    #[test]
    fn a_plain_move_has_no_owner() {
        let u = update(&message(0x0180, &position([1.0, 2.0, 3.0])), &[OWNER], &[]).unwrap();
        assert_eq!((u.position, u.owner), (Some([1.0, 2.0, 3.0]), None));
        assert!(!u.unread);
    }

    #[test]
    fn the_owner_is_found_from_the_end_behind_other_components() {
        // A component of unknown size before the owner's.
        let mut body = position([0.0, 0.0, 1.0]);
        body.extend([0xAB; 7]);
        body.extend(thrown(1));
        let classes = [OTHER, OWNER];
        let u = update(&message(0x01E0, &body), &classes, &[PLAYER]).unwrap();
        assert_eq!(u.owner.unwrap().player, Some(PLAYER));
        // Not a player of the round: no form fits the tail.
        let u = update(&message(0x01E0, &body), &classes, &[1]).unwrap();
        assert!(u.owner.is_none() && u.unread);
        // An empty component is the single byte that is left.
        body.truncate(16 + 7);
        body.push(0);
        let u = update(&message(0x01E0, &body), &classes, &[PLAYER]).unwrap();
        assert_eq!(u.owner, Some(Owner::default()));
        // Attached to a parent: the transform bits are not a place.
        let u = update(&message(0x0120, &[0x08, 0]), &classes, &[]).unwrap();
        assert_eq!(u.position, None);
        assert_eq!(u.owner.unwrap().released, Some(0));
    }

    #[test]
    fn behind_a_later_component_the_owner_is_found_by_its_player() {
        let classes = [OWNER, OTHER];
        // The form of a full state: u16, owner, alliance; then the other
        // class's component.
        let mut held = vec![0x07, 0x19, 0x02];
        held.extend(PLAYER.to_le_bytes());
        held.extend(3u32.to_le_bytes());
        let mut body = position([0.0, 0.0, -100.0]);
        body.extend([0; 16]);
        body.extend([1, 0, 0x80, 0]);
        body.extend(&held);
        body.push(0);
        let u = update(&message(0x1FE0, &body), &classes, &[9, PLAYER]).unwrap();
        let o = u.owner.unwrap();
        assert_eq!(
            (o.submask, o.player, o.alliance),
            (0x07, Some(PLAYER), Some(3))
        );
        // A throw by hand in front of two unknown bytes.
        let mut body = thrown(1);
        body.extend([9, 9]);
        let u = update(&message(0x0060, &body), &classes, &[PLAYER]).unwrap();
        assert_eq!(u.owner.unwrap().released, Some(1));
        // The id alone, with no submask before it that fits, is no owner.
        let mut body = vec![0xFF, 0xFF, 0xFF];
        body.extend(PLAYER.to_le_bytes());
        body.extend([0; 5]);
        let u = update(&message(0x0060, &body), &classes, &[PLAYER]).unwrap();
        assert!(u.unread && u.owner.is_none());
        // The full state of an object nobody has taken names no player,
        // and that is as expected.
        let mut body = position([0.0; 3]);
        body.extend([0; 40]);
        let u = update(&message(0x1FE0, &body), &classes, &[PLAYER]).unwrap();
        assert!(!u.unread && u.owner.is_none());
    }

    #[test]
    fn an_empty_component_of_an_attached_object_is_read() {
        let u = update(&message(0x0820, &[0]), &[OTHER, OWNER], &[]).unwrap();
        assert_eq!(u.owner, Some(Owner::default()));
        assert!(!u.unread);
    }

    #[test]
    fn updates_cut_short_are_rejected() {
        let mut body = position([1.0, 2.0, 3.0]);
        body.extend(thrown(1));
        let good = message(0x01C0, &body);
        assert!(update(&good, &[OWNER], &[PLAYER]).unwrap().owner.is_some());
        for cut in 0..22 {
            assert_eq!(update(&good[..cut], &[OWNER], &[PLAYER]), None, "cut {cut}");
        }
        // Cut inside the component: the message parses, the owner does not.
        let u = update(&good[..good.len() - 1], &[OWNER], &[PLAYER]).unwrap();
        assert!(u.unread);
        let mut other = good.clone();
        other[0] = 0x63;
        assert_eq!(update(&other, &[OWNER], &[PLAYER]), None);
        let nan = message(0x0180, &position([f32::NAN, 0.0, 0.0]));
        assert_eq!(update(&nan, &[OWNER], &[]), None);
    }

    /// Points every 1/30 s at these heights, moving 0.2 m a step along x.
    fn falling(heights: &[f32]) -> Vec<(f64, [f32; 3])> {
        (heights.iter().enumerate())
            .map(|(i, z)| (i as f64 / 30.0, [0.2 * i as f32, 0.0, *z]))
            .collect()
    }

    #[test]
    fn a_flight_ends_at_the_first_pause() {
        let mut points = falling(&[2.0, 1.9, 1.7, 1.4]);
        points.push((1.0, [5.0, 0.0, 0.0]));
        assert_eq!(flight(&points, false), 4);
        assert_eq!(flight(&points[..4], false), 4);
        assert_eq!(flight(&[], true), 0);
        assert_eq!(flight(&points[..1], true), 1);
    }

    #[test]
    fn a_driven_object_flies_until_it_lands() {
        // Falls, bounces up once, falls again, then stays down and drives.
        let heights = [
            2.0, 1.9, 1.7, 1.4, 1.0, 0.5, 0.0, 0.2, 0.3, 0.3, 0.2, 0.0, 0.0, 0.0, 0.0, 0.0,
        ];
        let points = falling(&heights);
        assert_eq!(flight(&points, false), heights.len());
        // The fall stops in the step after point 11, its second touch of
        // the ground.
        assert_eq!(flight(&points, true), 13);
    }

    #[test]
    fn thinning_keeps_the_ends_and_drops_points_that_barely_move() {
        let mut points = falling(&[1.0; 200]);
        points.push((7.0, [39.81, 0.0, 1.0]));
        let thinned = thin(&points);
        assert_eq!(thinned.len(), PATH_POINTS);
        assert_eq!(thinned.first(), points.first());
        assert_eq!(thinned.last(), points.last());
        let still = [
            (0.0, [0.0; 3]),
            (0.1, [0.01, 0.0, 0.0]),
            (0.2, [0.02, 0.0, 0.0]),
        ];
        assert_eq!(thin(&still), [still[0], still[2]]);
        assert!(thin(&[]).is_empty());
    }

    #[test]
    fn heading_is_a_unit_vector_and_a_speed() {
        let (unit, speed) = heading((0.0, [0.0; 3]), (0.5, [3.0, 0.0, 4.0])).unwrap();
        assert_eq!((unit, speed), ([0.6, 0.0, 0.8], 10.0));
        assert_eq!(heading((0.0, [0.0; 3]), (0.0, [1.0, 0.0, 0.0])), None);
        assert_eq!(heading((0.0, [0.0; 3]), (1.0, [0.0; 3])), None);
    }
}
