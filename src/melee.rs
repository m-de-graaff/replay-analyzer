//! Melee hits and shield actions (Y11S3), from the `movement` stream.
//!
//! The stream's messages are framed as [`crate::loadout`] describes. An
//! update message (`607385fe`) is a mask byte, a transform group when the
//! mask has `80`, then one component per class of the entity's create
//! message, the first class behind mask bit `40`, the second behind `20`:
//!
//! ```text
//! 607385fe  u8 mask
//! mask & 80:  u8 sub
//!   sub & 01  f32 x, y, z, u32 0        sub & 02  f32 qx, qy, qz, qw
//!   sub & 04  u8                        sub & 08  u16
//!   sub & 10  u8 n, n x { u32 op, arguments }
//!     op 0  attach: u64 parent, u64 self, 16 bytes socket, u16
//!     op 1  attach with a matrix: u64 parent, u64 self, 16 x f32
//!     op 2  detach                      op 3  no arguments
//! ```
//!
//! **Melee hits.** Whatever can be damaged carries a damage list: a
//! barricade (classes `4c60869a 6ea51c35`) as its second component, a
//! destructible part of the map (an object with a 40-bit id and no create
//! message) as its first. The list is `u32 count`, then per entry one kind
//! byte, `fe` alone when the object is destroyed, else a record:
//!
//! ```text
//! +0   u8 kind        0 melee, 1 bullet
//! +1   f32 x, y, z    where it was hit, in the object's own space
//! +13  f32 1.0        +17 4 x f32
//! +33  u64 body       who did it         +41 u64 0
//! +49  u64 damage id  34118943362 for melee
//! +57  u32 ffffffff   (melee)            +61 u16 part, u16 counter
//! +65  9 x f32        +101 u32 ffffffff
//! +105 u32 n, n x 40 bytes               (melee: n = 1)
//! ```
//!
//! A melee hit is a record with the melee damage id; the body is the
//! player's, or 0 for what the map has broken when the round starts. Three hits break a barricade (more for a reinforced one), and
//! the `fe` entry follows the last hit in the same message or 0.1 s later.
//! The record's own counter also counts bullets and stays 0 on some
//! barricades, so hits are numbered here. The hit point is turned into map
//! coordinates with the object's last transform.
//!
//! Melee hits on players are not recorded unless they kill, which the kill
//! feed has. A swing itself is seen only when the knife entity (the asset
//! in the body's `MeleeWeapon` slot) is attached to the hand socket
//! `99daadd5`, which the game writes for about a third of the swings.
//!
//! **Shields.** A held shield is an entity of class `767e6385` that is
//! attached to its holder's back (socket `8c3ca2c6`) when the body spawns.
//! The socket of its later attach ops says where it is:
//!
//! ```text
//! aec150d4  in hand            60583202  in hand (Clash)
//! 8c3ca2c6  on the back        e975cd9e  put away (Blackbeard)
//! ```
//!
//! Montagne's shield also carries, in the component of class `513b13b2`,
//! the blob `u16 28, 36638d75, u32 16, u32 a, u32 state, u32, u32, u32 0`
//! with state 0 normal, 1 extending, 2 extended, 13 retracting.

use std::collections::{HashMap, HashSet};

use memchr::memmem;
use rayon::prelude::*;
use serde::Serialize;

use crate::entities::Hash;
use crate::loadout::{Input, MOVEMENT_STREAM, STATE_STREAM, When, descriptor, messages};

const CREATE: Hash = [0x61, 0x73, 0x85, 0xFE];
const UPDATE: Hash = [0x60, 0x73, 0x85, 0xFE];

/// Classes of a barricade.
const PLACED: Hash = [0x4C, 0x60, 0x86, 0x9A];
const DAMAGEABLE: Hash = [0x6E, 0xA5, 0x1C, 0x35];
/// Class of every shield, held or deployed.
const SHIELD: Hash = [0x76, 0x7E, 0x63, 0x85];
/// Body slot that names the knife's asset.
const MELEE_WEAPON: Hash = [0x6E, 0x96, 0x1A, 0x65];
/// Most classes a create message lists.
const MAX_CLASSES: usize = 16;

/// The damage id of a melee hit.
const MELEE_DAMAGE: u64 = 34_118_943_362;
/// The damage id and the `ffffffff` after it, as a record holds them.
const MELEE_ANCHOR: [u8; 12] = [
    0x82, 0xC2, 0xA5, 0xF1, 0x07, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF,
];
/// Offset of the damage id in a record.
const ID_OFFSET: usize = 49;
/// A record up to its impact list, and one impact.
const RECORD_HEAD: usize = 109;
const IMPACT: usize = 40;
const MAX_IMPACTS: usize = 64;
const MAX_RECORDS: usize = 64;
/// The entry of a damage list that says the object is destroyed.
const DESTROYED: u8 = 0xFE;
/// The destroyed entry follows the hit that did it within this long.
const BREAK_WINDOW: f64 = 0.5;
/// Hit points are within a few metres of the object's origin.
const MAX_POINT: f32 = 100.0;

const SOCKET_HAND: Hash = [0xAE, 0xC1, 0x50, 0xD4];
const SOCKET_HAND_CLASH: Hash = [0x60, 0x58, 0x32, 0x02];
const SOCKET_BACK: Hash = [0x8C, 0x3C, 0xA2, 0xC6];
const SOCKET_AWAY: Hash = [0xE9, 0x75, 0xCD, 0x9E];
const SOCKET_KNIFE: Hash = [0x99, 0xDA, 0xAD, 0xD5];
/// A knife is attached this long before its swing lands, at most.
const SWING: f64 = 1.0;

/// Montagne's state blob up to its first field: size, hash, `u32 16`.
const EXTENSION_BLOB: [u8; 10] = [0x1C, 0, 0x36, 0x63, 0x8D, 0x75, 0x10, 0, 0, 0];
const EXTENDED: u32 = 2;
const RETRACTING: u32 = 13;

/// What a melee hit landed on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Target {
    /// A barricade on a door or window, Castle's included.
    #[default]
    Barricade,
    /// A destructible part of the map: a wall, a hatch, a prop.
    MapObject,
    /// Another entity that takes damage, such as a placed gadget.
    Entity,
}

/// One melee hit on a barricade or map object. A swing that lands on a
/// barricade and the frame around it gives one hit on each.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeleeHit {
    pub username: String,
    pub target: Target,
    /// Id of the object hit, in hex. Map objects have 40-bit ids that are
    /// the same in every round on the map.
    pub object: String,
    /// Which melee hit on this object it is, counting from 1 and starting
    /// over once the object broke.
    pub hit: u32,
    /// The hit broke the barricade.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub broke: bool,
    /// Where the hit landed, in map coordinates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<[f32; 3]>,
    /// Where the hit landed relative to the object, when the object's place
    /// on the map is not known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point: Option<[f32; 3]>,
    /// The player had a shield in hand: a shield bash.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub with_shield: bool,
    /// The knife was seen in the player's hand for this swing. The game
    /// writes that for about a third of the swings, so its absence says
    /// nothing.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub knife_seen: bool,
    #[serde(flatten)]
    pub when: When,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ShieldActionType {
    /// The shield went in hand.
    #[default]
    Raise,
    /// The shield went on the back.
    Stow,
    /// The shield left the hand without being put away: its holder died.
    Drop,
    /// Montagne extended his shield. `when` is the moment it started.
    Extend,
}

/// One shield action.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShieldAction {
    pub username: String,
    pub action: ShieldActionType,
    /// `extend`: seconds until the shield was fully extended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extend_time: Option<f64>,
    /// `extend`: seconds it stayed fully extended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extended_for: Option<f64>,
    /// `extend`: seconds retracting took.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retract_time: Option<f64>,
    /// `extend`: seconds from the start until the shield was back to
    /// normal. Absent when it never was: the player died or the recording
    /// ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(flatten)]
    pub when: When,
}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    pub hits: Vec<MeleeHit>,
    pub shields: Vec<ShieldAction>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        b.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}

fn f32_at(b: &[u8], at: usize) -> Option<f32> {
    u32_at(b, at).map(f32::from_bits)
}

fn floats<const N: usize>(b: &[u8], at: usize) -> Option<[f32; N]> {
    let mut out = [0.0; N];
    for (i, v) in out.iter_mut().enumerate() {
        *v = f32_at(b, at.checked_add(4 * i)?)?;
    }
    out.iter().all(|v| v.is_finite()).then_some(out)
}

/// Where a message is in the stream: its block, then its offset.
type Order = (usize, usize);

/// One operation of an update's transform group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Attach { parent: u64, socket: Hash },
    Detach,
    Other,
}

/// The transform group of an update message.
#[derive(Clone, Debug, Default, PartialEq)]
struct Head {
    position: Option<[f32; 3]>,
    rotation: Option<[f32; 4]>,
    ops: Vec<Op>,
    /// Offset of the first component.
    at: usize,
}

/// Walks the mask and transform group of a `607385fe` payload. `None` when
/// it is another type or holds something not understood.
fn head(payload: &[u8]) -> Option<Head> {
    if payload.get(..4)? != UPDATE {
        return None;
    }
    let mask = *payload.get(4)?;
    let mut out = Head {
        at: 5,
        ..Head::default()
    };
    if mask & 0x80 == 0 {
        return Some(out);
    }
    let sub = *payload.get(5)?;
    if sub & 0xE0 != 0 {
        return None;
    }
    let mut at = 6;
    if sub & 0x01 != 0 {
        out.position = Some(floats(payload, at)?);
        at += 16;
    }
    if sub & 0x02 != 0 {
        out.rotation = Some(floats(payload, at)?);
        at += 16;
    }
    if sub & 0x04 != 0 {
        at += 1;
    }
    if sub & 0x08 != 0 {
        at += 2;
    }
    if sub & 0x10 != 0 {
        let count = *payload.get(at)?;
        at += 1;
        for _ in 0..count {
            let op = u32_at(payload, at)?;
            at += 4;
            out.ops.push(match op {
                0 => {
                    let parent = u64_at(payload, at)?;
                    let socket = payload.get(at + 16..at + 20)?.try_into().ok()?;
                    at += 34;
                    Op::Attach { parent, socket }
                }
                1 => {
                    at += 80;
                    Op::Other
                }
                2 => Op::Detach,
                3 => Op::Other,
                _ => return None,
            });
        }
    }
    (at <= payload.len()).then_some(Head { at, ..out })
}

/// A melee record of a damage list.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Record {
    /// The hit point in the object's space.
    point: [f32; 3],
    /// The body that hit; 0 for damage the map starts the round with.
    body: u64,
}

/// Reads the melee record whose damage id is at `anchor` in `payload`.
/// `None` when the bytes around it are not a record's.
fn record(payload: &[u8], anchor: usize) -> Option<Record> {
    let start = anchor.checked_sub(ID_OFFSET)?;
    let r = payload.get(start..start.checked_add(RECORD_HEAD + IMPACT)?)?;
    let body = u64_at(r, 33)?;
    let valid = r.first() == Some(&0)
        && f32_at(r, 13)? == 1.0
        && (body == 0 || body >> 24 == 0xF0)
        && u64_at(r, 41)? == 0
        && u64_at(r, ID_OFFSET)? == MELEE_DAMAGE
        && u32_at(r, 57)? == u32::MAX
        && (u32_at(r, 101)? == u32::MAX || body == 0)
        && u32_at(r, 105)? == 1;
    let point: [f32; 3] = floats(r, 1)?;
    (valid && point.iter().all(|v| v.abs() < MAX_POINT)).then_some(Record { point, body })
}

/// Whether `payload` is an update whose damage list ends in the destroyed
/// entry. The list is walked from its count to the end of the message.
fn destroyed(payload: &[u8]) -> bool {
    let walk = || -> Option<bool> {
        let mut at = head(payload)?.at;
        let count = u32_at(payload, at)? as usize;
        at += 4;
        if count == 0 || count > MAX_RECORDS {
            return None;
        }
        let mut last = 0;
        for _ in 0..count {
            last = *payload.get(at)?;
            if last == DESTROYED {
                at += 1;
                continue;
            }
            let impacts = u32_at(payload, at + RECORD_HEAD - 4)? as usize;
            if impacts > MAX_IMPACTS {
                return None;
            }
            at += RECORD_HEAD + IMPACT * impacts;
        }
        Some(at == payload.len() && last == DESTROYED)
    };
    payload.last() == Some(&DESTROYED) && walk() == Some(true)
}

/// The state of Montagne's shield, when `payload` holds its blob.
fn extension(payload: &[u8]) -> Option<u32> {
    let at = memmem::find(payload, &EXTENSION_BLOB)?;
    // The blob is 2 + 28 bytes; its state is the second field.
    payload.get(at..at + 30)?;
    u32_at(payload, at + 14)
}

/// What a create message says about its entity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Create {
    entity: u64,
    asset: u64,
    barricade: bool,
    shield: bool,
    /// The knife asset a body carries, 0 for none.
    knife: u64,
}

fn create(payload: &[u8]) -> Option<Create> {
    let d = descriptor(payload)?;
    let count = u32_at(payload, 53)? as usize;
    if count > MAX_CLASSES {
        return None;
    }
    let classes = payload.get(57..57 + 4 * count)?;
    let has = |class: Hash| classes.as_chunks::<4>().0.contains(&class);
    Some(Create {
        entity: d.entity,
        asset: d.asset,
        barricade: has(PLACED) && has(DAMAGEABLE),
        shield: has(SHIELD),
        knife: (d.slots.iter())
            .find(|s| s.0 == MELEE_WEAPON)
            .map_or(0, |s| s.1),
    })
}

/// A melee record as found in the stream.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RawHit {
    entity: u64,
    record: Record,
    order: Order,
    frame: Option<u32>,
}

/// What the first pass finds in one block.
#[derive(Debug, Default)]
struct Scan {
    creates: Vec<Create>,
    hits: Vec<RawHit>,
    /// `(entity, order, frame)` of each damage list that ends destroyed.
    breaks: Vec<(u64, Order, Option<u32>)>,
    /// Damage ids with no record around them.
    malformed: usize,
}

/// An update of an entity of interest.
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    entity: u64,
    order: Order,
    frame: Option<u32>,
    /// `None` when the transform group was not understood.
    head: Option<Head>,
    extension: Option<u32>,
}

/// A held shield while the stream is read.
#[derive(Clone, Copy, Debug, Default)]
struct Shield {
    /// It was seen on a back: a held shield, not a deployable one.
    held: bool,
    holder: Option<u64>,
    in_hand: bool,
    /// Montagne's state.
    state: u32,
    open: Option<Open>,
}

/// An extension of Montagne's shield that has not ended.
#[derive(Clone, Copy, Debug)]
struct Open {
    /// Index in the actions.
    index: usize,
    start: f64,
    extended: Option<f64>,
    retract: Option<f64>,
}

/// The place of an entity from one message on.
type Place = (Order, Option<[f32; 3]>, Option<[f32; 4]>);

fn millis(seconds: f64) -> f64 {
    (seconds * 1000.0).round() / 1000.0
}

/// `point`, in the space of an object at `position` turned by the
/// quaternion `q`, in map coordinates to the millimetre.
fn to_map(position: [f32; 3], q: [f32; 4], point: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = q;
    let [px, py, pz] = point;
    // p + 2 q x (q x p + w p)
    let c = [
        y * pz - z * py + w * px,
        z * px - x * pz + w * py,
        x * py - y * px + w * pz,
    ];
    let turned = [
        px + 2.0 * (y * c[2] - z * c[1]),
        py + 2.0 * (z * c[0] - x * c[2]),
        pz + 2.0 * (x * c[1] - y * c[0]),
    ];
    [0, 1, 2].map(|i| ((position[i] + turned[i]) * 1000.0).round() / 1000.0)
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// Reads melee hits and shield actions from the movement stream. The
/// stream is most of a replay, so its blocks are read in parallel: once
/// for create messages and damage records, and once more for the updates
/// of the objects hit, the shields and the knives.
pub(crate) fn decode(input: &Input) -> Decoded {
    let data = input.data;
    let clock = input.clock;
    let blocks: Vec<(usize, usize, Option<u32>)> = input.blocks(MOVEMENT_STREAM).collect();
    let block = |start: usize, end: usize| data.get(start..end).unwrap_or_default();
    let anchor = memmem::Finder::new(&MELEE_ANCHOR);
    // The clock is read from the state stream, and each stream's records
    // are apart in the data: an event of the movement stream is placed by
    // the end of the state record of its frame, or of the last one before.
    let state: Vec<(u32, usize)> = (input.blocks(STATE_STREAM))
        .filter_map(|(_, end, frame)| Some((frame?, end)))
        .collect();
    let when = |frame: Option<u32>| {
        let i = state.partition_point(|s| Some(s.0) <= frame);
        let at = i.checked_sub(1).and_then(|i| state.get(i));
        clock.when(at.map_or(0, |s| s.1), frame)
    };

    let scans: Vec<Scan> = blocks
        .par_iter()
        .enumerate()
        .map(|(index, &(start, end, frame))| {
            let mut scan = Scan::default();
            for (entity, at, payload) in messages(block(start, end)) {
                let Some(kind) = payload.get(..4) else {
                    continue;
                };
                if kind == CREATE {
                    scan.creates
                        .extend(create(payload).filter(|c| c.entity == entity));
                } else if kind == UPDATE && frame.is_some() {
                    // The snapshot holds what was hit before the recording
                    // started, with no time to give it.
                    if destroyed(payload) {
                        scan.breaks.push((entity, (index, at), frame));
                    }
                    if payload.len() < RECORD_HEAD + IMPACT {
                        continue;
                    }
                    for found in anchor.find_iter(payload) {
                        match record(payload, found) {
                            Some(record) if record.body == 0 => {}
                            Some(record) => scan.hits.push(RawHit {
                                entity,
                                record,
                                order: (index, at),
                                frame,
                            }),
                            None => scan.malformed += 1,
                        }
                    }
                }
            }
            scan
        })
        .collect();

    let mut out = Decoded::default();
    let mut creates: HashMap<u64, Create> = HashMap::new();
    let mut shields: HashSet<u64> = HashSet::new();
    let mut raw: Vec<RawHit> = Vec::new();
    let mut breaks: Vec<(u64, Order, Option<u32>)> = Vec::new();
    let mut malformed = 0;
    for scan in scans {
        for c in scan.creates {
            if c.shield {
                shields.insert(c.entity);
            }
            creates.insert(c.entity, c);
        }
        raw.extend(scan.hits);
        breaks.extend(scan.breaks);
        malformed += scan.malformed;
    }
    if malformed > 0 {
        out.warnings.push(format!(
            "{} not in a damage record",
            plural(malformed, "melee damage id is", "melee damage ids are")
        ));
    }

    let users: HashMap<u64, &str> = (input.players.iter())
        .filter_map(|p| {
            let body = p.entities.as_ref()?.movement?;
            Some((u64::from(body), p.username.as_str()))
        })
        .collect();
    let knife_assets: HashSet<u64> = (creates.values())
        .filter(|c| c.knife != 0 && users.contains_key(&c.entity))
        .map(|c| c.knife)
        .collect();
    let knives: HashSet<u64> = (creates.values())
        .filter(|c| knife_assets.contains(&c.asset))
        .map(|c| c.entity)
        .collect();
    let mut wanted: HashSet<u64> = raw.iter().map(|h| h.entity).collect();
    wanted.extend(&shields);
    wanted.extend(&knives);
    if wanted.is_empty() {
        return out;
    }

    let seen: Vec<Vec<Seen>> = blocks
        .par_iter()
        .enumerate()
        .map(|(index, &(start, end, frame))| {
            messages(block(start, end))
                .filter(|(entity, _, payload)| {
                    payload.starts_with(&UPDATE) && wanted.contains(entity)
                })
                .map(|(entity, at, payload)| Seen {
                    entity,
                    order: (index, at),
                    frame,
                    head: head(payload),
                    extension: extension(payload),
                })
                .collect()
        })
        .collect();

    // Shields and knives, in stream order.
    let mut unread = 0;
    let mut unowned = 0;
    let mut state: HashMap<u64, Shield> = HashMap::new();
    // Body -> `(order, in hand)` of each change of its shield.
    let mut hands: HashMap<u64, Vec<(Order, bool)>> = HashMap::new();
    // Body -> seconds of each knife attach.
    let mut swings: HashMap<u64, Vec<f64>> = HashMap::new();
    let mut places: HashMap<u64, Vec<Place>> = HashMap::new();
    for s in seen.iter().flatten() {
        let seconds = clock.seconds(s.frame);
        let Some(head) = &s.head else {
            unread += usize::from(shields.contains(&s.entity) || knives.contains(&s.entity));
            continue;
        };
        if head.position.is_some() || head.rotation.is_some() {
            let places = places.entry(s.entity).or_default();
            let (_, position, rotation) = places.last().copied().unwrap_or_default();
            places.push((
                s.order,
                head.position.or(position),
                head.rotation.or(rotation),
            ));
        }
        if knives.contains(&s.entity) {
            for op in &head.ops {
                if let Op::Attach { parent, socket } = *op
                    && socket == SOCKET_KNIFE
                    && let Some(t) = seconds
                {
                    swings.entry(parent).or_default().push(t);
                }
            }
        }
        if !shields.contains(&s.entity) {
            continue;
        }
        let shield = state.entry(s.entity).or_default();
        let was = shield.in_hand;
        let mut dropped = false;
        for op in &head.ops {
            match *op {
                Op::Attach { parent, socket } => {
                    dropped = false;
                    match socket {
                        SOCKET_BACK => (shield.held, shield.in_hand) = (true, false),
                        SOCKET_AWAY => shield.in_hand = false,
                        SOCKET_HAND | SOCKET_HAND_CLASH => shield.in_hand = shield.held,
                        _ => continue,
                    }
                    shield.holder = Some(parent);
                }
                Op::Detach => {
                    dropped = shield.in_hand || dropped;
                    shield.in_hand = false;
                }
                Op::Other => {}
            }
        }
        let user = shield.holder.and_then(|b| users.get(&b).copied());
        let mut action = |action: ShieldActionType, shields: &mut Vec<ShieldAction>| {
            let Some(user) = user else {
                unowned += 1;
                return None;
            };
            shields.push(ShieldAction {
                username: user.to_owned(),
                action,
                when: when(s.frame),
                ..ShieldAction::default()
            });
            Some(shields.len() - 1)
        };
        if shield.in_hand != was && s.frame.is_some() {
            let kind = match (shield.in_hand, dropped) {
                (true, _) => ShieldActionType::Raise,
                (false, true) => ShieldActionType::Drop,
                (false, false) => ShieldActionType::Stow,
            };
            action(kind, &mut out.shields);
        }
        if shield.in_hand != was
            && let Some(body) = shield.holder
        {
            hands
                .entry(body)
                .or_default()
                .push((s.order, shield.in_hand));
        }

        let Some(now) = s.extension.filter(|&e| e != shield.state) else {
            continue;
        };
        shield.state = now;
        let Some(t) = seconds else { continue };
        if now != 0 && shield.open.is_none() {
            shield.open = action(ShieldActionType::Extend, &mut out.shields).map(|index| Open {
                index,
                start: t,
                extended: None,
                retract: None,
            });
        }
        let Some(open) = &mut shield.open else {
            continue;
        };
        let Some(entry) = out.shields.get_mut(open.index) else {
            continue;
        };
        // A shield that flickers back to extending for a frame stays
        // extended: only the first of each state counts.
        if now == EXTENDED && open.extended.is_none() {
            open.extended = Some(t);
            entry.extend_time = Some(millis(t - open.start));
        }
        if (now == RETRACTING || now == 0) && open.retract.is_none() {
            open.retract = Some(t);
            entry.extended_for = open.extended.map(|e| millis(t - e));
        }
        if now == 0 {
            entry.retract_time = open.retract.filter(|&r| r < t).map(|r| millis(t - r));
            entry.duration = Some(millis(t - open.start));
            shield.open = None;
        }
    }
    if unread > 0 {
        out.warnings.push(format!(
            "{} not understood",
            plural(
                unread,
                "shield or knife update was",
                "shield or knife updates were"
            )
        ));
    }
    if unowned > 0 {
        out.warnings.push(format!(
            "{} by a body that is no player's",
            plural(unowned, "shield action", "shield actions")
        ));
    }

    // Hits, in stream order. Records of one swing on one object are one hit.
    let mut strangers = 0;
    let mut counts: HashMap<u64, (u32, Order)> = HashMap::new();
    let mut last: HashMap<u64, (usize, f64)> = HashMap::new();
    let mut previous: Option<(u64, u64, Order)> = None;
    breaks.sort_by_key(|b| b.1);
    let mut breaks = breaks.into_iter().peekable();
    for h in raw {
        // Breaks written before this hit end the count of their object.
        while let Some((entity, order, frame)) = breaks.next_if(|b| b.1 < h.order) {
            let seconds = clock.seconds(frame);
            close(&mut out.hits, &mut counts, &last, entity, order, seconds);
        }
        let key = (h.entity, h.record.body, h.order);
        if previous.replace(key) == Some(key) {
            continue;
        }
        let Some(user) = users.get(&h.record.body) else {
            strangers += 1;
            continue;
        };
        let target = match creates.get(&h.entity) {
            Some(c) if c.barricade => Target::Barricade,
            Some(_) => Target::Entity,
            None => Target::MapObject,
        };
        let place = places.get(&h.entity).and_then(|p| {
            let i = p.partition_point(|x| x.0 <= h.order);
            let (_, position, rotation) = *p.get(i.checked_sub(1)?)?;
            Some((position?, rotation?))
        });
        // An entity that is no barricade may sit on another: its transform
        // is not its place on the map.
        let position = place
            .filter(|_| target != Target::Entity)
            .map(|(position, rotation)| to_map(position, rotation, h.record.point));
        let seconds = clock.seconds(h.frame);
        let count = counts.entry(h.entity).or_insert((0, h.order));
        *count = (count.0 + 1, h.order);
        if let Some(t) = seconds {
            last.insert(h.entity, (out.hits.len(), t));
        }
        out.hits.push(MeleeHit {
            username: (*user).to_owned(),
            target,
            object: format!("{:08x}", h.entity),
            hit: count.0,
            broke: false,
            position,
            point: position.is_none().then_some(h.record.point),
            with_shield: hands.get(&h.record.body).is_some_and(|changes| {
                let i = changes.partition_point(|c| c.0 <= h.order);
                (i.checked_sub(1).and_then(|i| changes.get(i))).is_some_and(|c| c.1)
            }),
            knife_seen: seconds.is_some_and(|t| {
                (swings.get(&h.record.body))
                    .is_some_and(|s| s.iter().any(|&a| a - 0.05 <= t && t <= a + SWING))
            }),
            when: when(h.frame),
        });
    }
    for (entity, order, frame) in breaks {
        let seconds = clock.seconds(frame);
        close(&mut out.hits, &mut counts, &last, entity, order, seconds);
    }
    if strangers > 0 {
        out.warnings.push(format!(
            "{} by a body that is no player's",
            plural(strangers, "melee hit", "melee hits")
        ));
    }
    out
}

/// The damage list of `entity` ended destroyed at `order`: the count of its
/// hits starts over, and its last hit broke it when it was a barricade's
/// and came just before.
fn close(
    hits: &mut [MeleeHit],
    counts: &mut HashMap<u64, (u32, Order)>,
    last: &HashMap<u64, (usize, f64)>,
    entity: u64,
    order: Order,
    seconds: Option<f64>,
) {
    let Some((_, hit_order)) = counts.remove(&entity) else {
        return;
    };
    if let Some(&(index, t)) = last.get(&entity)
        && let Some(hit) = hits.get_mut(index)
        && hit.target == Target::Barricade
        && hit_order <= order
        && seconds.is_some_and(|s| s - t <= BREAK_WINDOW)
    {
        hit.broke = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A melee record by `body` at `point`, with `impacts` impacts.
    fn melee_record(body: u64, point: [f32; 3], impacts: u32) -> Vec<u8> {
        let mut r = vec![0u8];
        for v in point {
            r.extend(v.to_le_bytes());
        }
        r.extend(1.0f32.to_le_bytes());
        r.extend([0u8; 16]);
        r.extend(body.to_le_bytes());
        r.extend(0u64.to_le_bytes());
        r.extend(MELEE_ANCHOR);
        r.extend([0x0F, 0, 2, 0]);
        r.extend([0u8; 36]);
        r.extend(u32::MAX.to_le_bytes());
        r.extend(impacts.to_le_bytes());
        for _ in 0..impacts {
            r.extend([0u8; 32]);
            r.extend(u32::MAX.to_le_bytes());
            r.extend(1.0f32.to_le_bytes());
        }
        r
    }

    /// An update with mask `mask`, `prefix` after it, and a damage list.
    fn update(mask: u8, prefix: &[u8], entries: &[&[u8]]) -> Vec<u8> {
        let mut p = UPDATE.to_vec();
        p.push(mask);
        p.extend(prefix);
        p.extend((entries.len() as u32).to_le_bytes());
        for e in entries {
            p.extend(*e);
        }
        p
    }

    #[test]
    fn a_melee_record_is_read_at_its_damage_id() {
        let hit = melee_record(0xF02B_AC1D, [-0.25, -0.81, -0.3], 1);
        assert_eq!(hit.len(), RECORD_HEAD + IMPACT);
        let payload = update(0x20, &[], &[&hit]);
        assert_eq!(payload.len(), 158);
        let anchor = memmem::find(&payload, &MELEE_ANCHOR).unwrap();
        assert_eq!(anchor, 58);
        assert_eq!(
            record(&payload, anchor),
            Some(Record {
                point: [-0.25, -0.81, -0.3],
                body: 0xF02B_AC1D,
            })
        );
        assert!(!destroyed(&payload));
    }

    #[test]
    fn two_records_of_one_message_are_both_read() {
        let a = melee_record(0xF02B_8A4F, [0.1, 0.2, 0.3], 1);
        let b = melee_record(0xF02B_8A4F, [0.4, 0.5, 0.6], 1);
        let payload = update(0x40, &[], &[&a, &b]);
        assert_eq!(payload.len(), 307);
        let found: Vec<Record> = memmem::find_iter(&payload, &MELEE_ANCHOR)
            .filter_map(|at| record(&payload, at))
            .collect();
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].point, [0.4, 0.5, 0.6]);
    }

    #[test]
    fn bytes_that_are_no_record_are_refused() {
        let good = melee_record(0xF02B_AC1D, [0.0, 0.0, 0.0], 1);
        let at = |payload: &[u8]| memmem::find(payload, &MELEE_ANCHOR).unwrap();
        // A bullet's kind byte, a body that is no entity, a hit point far
        // off or not a number.
        let mut bullet = good.clone();
        bullet[0] = 1;
        let mut nobody = good.clone();
        nobody[33..41].copy_from_slice(&7u64.to_le_bytes());
        let mut far = good.clone();
        far[1..5].copy_from_slice(&5000.0f32.to_le_bytes());
        let mut nan = good.clone();
        nan[5..9].copy_from_slice(&f32::NAN.to_le_bytes());
        for bad in [bullet, nobody, far, nan] {
            let payload = update(0x20, &[], &[&bad]);
            assert_eq!(record(&payload, at(&payload)), None);
        }
        // A record cut short, an id too close to the start, an offset past
        // the end.
        let payload = update(0x20, &[], &[&good]);
        let cut = &payload[..payload.len() - 1];
        assert_eq!(record(cut, at(cut)), None);
        assert_eq!(record(&MELEE_ANCHOR, 0), None);
        assert_eq!(record(&payload, payload.len() + 5), None);
        assert_eq!(record(&payload, usize::MAX), None);
    }

    #[test]
    fn a_damage_list_that_ends_destroyed_is_a_break() {
        let hit = melee_record(0xF02B_AC1D, [0.0, -1.2, -0.3], 1);
        let bullet = {
            let mut b = melee_record(0xF02B_AC1D, [0.0, 0.0, 0.0], 2);
            b[0] = 1;
            b
        };
        // On its own, after the hit, after a bullet with two impacts, and
        // behind a transform group.
        assert!(destroyed(&update(0x20, &[], &[&[DESTROYED]])));
        assert_eq!(update(0x20, &[], &[&[DESTROYED]]).len(), 10);
        assert!(destroyed(&update(0x20, &[], &[&hit, &[DESTROYED]])));
        assert!(destroyed(&update(0x20, &[], &[&bullet, &[DESTROYED]])));
        assert!(destroyed(&update(
            0xA0,
            &[0x08, 0, 0],
            &[&hit, &[DESTROYED]]
        )));
        // Not when the list goes on, promises more than it holds, or the
        // message merely ends in the byte.
        assert!(!destroyed(&update(0x20, &[], &[&[DESTROYED], &hit])));
        let mut short = update(0x20, &[], &[&hit, &[DESTROYED]]);
        short[5] = 3;
        assert!(!destroyed(&short));
        let mut tail = update(0x20, &[], &[&hit]);
        *tail.last_mut().unwrap() = DESTROYED;
        assert!(!destroyed(&tail));
        assert!(!destroyed(&[]));
        assert!(!destroyed(&[DESTROYED]));
    }

    /// An attach op: parent, self, socket twice, `u16`.
    fn attach(parent: u64, socket: Hash) -> Vec<u8> {
        let mut op = 0u32.to_le_bytes().to_vec();
        op.extend(parent.to_le_bytes());
        op.extend(0xF01D_C833u64.to_le_bytes());
        op.extend(socket);
        op.extend([0x65, 0x56, 0xC6, 0xB6]);
        op.extend(socket);
        op.extend([0x65, 0x56, 0xC6, 0xB6, 0, 1]);
        op
    }

    fn blob(state: u32) -> Vec<u8> {
        let mut b = EXTENSION_BLOB.to_vec();
        for v in [3, state, 0x1F9, 1, 0] {
            b.extend(u32::to_le_bytes(v));
        }
        b
    }

    #[test]
    fn the_transform_group_is_walked() {
        // Position, rotation, both flags, then two ops and the blob.
        let mut p = UPDATE.to_vec();
        p.extend([0xE0, 0x1F]);
        for v in [-83.7f32, 33.48, 2.2, 0.0, 0.5, 0.5, 0.5, 0.5] {
            p.extend(v.to_le_bytes());
        }
        p.extend([0, 0, 0xE0, 2]);
        p.extend(3u32.to_le_bytes());
        p.extend(attach(0xF01D_D407, SOCKET_BACK));
        let components = p.len();
        p.extend(u32::MAX.to_le_bytes());
        p.extend(0xF01D_D407u64.to_le_bytes());
        p.extend([0xFF, 1, 0]);
        p.extend(blob(2));
        let h = head(&p).unwrap();
        assert_eq!(h.position, Some([-83.7, 33.48, 2.2]));
        assert_eq!(h.rotation, Some([0.5, 0.5, 0.5, 0.5]));
        assert_eq!(
            h.ops,
            [
                Op::Other,
                Op::Attach {
                    parent: 0xF01D_D407,
                    socket: SOCKET_BACK
                }
            ]
        );
        assert_eq!(h.at, components);
        assert_eq!(extension(&p), Some(2));

        // Ops only, as a shield going in hand is written.
        let mut p = UPDATE.to_vec();
        p.extend([0xA0, 0x10, 1]);
        p.extend(attach(0xF01D_D407, SOCKET_HAND));
        p.push(4);
        p.extend(blob(0));
        assert_eq!(p.len(), 6 + 70);
        let h = head(&p).unwrap();
        assert_eq!(h.ops.len(), 1);
        assert_eq!(extension(&p), Some(0));

        // A detach, and a message with no transform group.
        let mut p = UPDATE.to_vec();
        p.extend([0x80, 0x10, 1]);
        p.extend(2u32.to_le_bytes());
        assert_eq!(head(&p).unwrap().ops, [Op::Detach]);
        let mut p = UPDATE.to_vec();
        p.extend([0x20, 0x04]);
        p.extend(blob(13));
        let plain = Head {
            at: 5,
            ..Head::default()
        };
        assert_eq!(head(&p), Some(plain));
        assert_eq!(extension(&p), Some(13));
    }

    #[test]
    fn a_transform_group_that_does_not_fit_is_refused() {
        // An unknown op, an unknown flag, ops past the end, a position cut
        // short, another type.
        let mut p = UPDATE.to_vec();
        p.extend([0x80, 0x10, 1]);
        p.extend(9u32.to_le_bytes());
        assert_eq!(head(&p), None);
        let mut p = UPDATE.to_vec();
        p.extend([0x80, 0x40]);
        assert_eq!(head(&p), None);
        let mut p = UPDATE.to_vec();
        p.extend([0x80, 0x10, 2]);
        p.extend(attach(0xF01D_D407, SOCKET_HAND));
        assert_eq!(head(&p), None);
        let mut p = UPDATE.to_vec();
        p.extend([0x80, 0x01, 0, 0]);
        assert_eq!(head(&p), None);
        assert_eq!(head(&CREATE), None);
        assert_eq!(head(&[]), None);
        // A blob cut short has no state.
        let cut = blob(2);
        assert_eq!(extension(&cut[..cut.len() - 1]), None);
        assert_eq!(extension(&[]), None);
    }

    #[test]
    fn a_hit_point_is_turned_into_map_coordinates() {
        // A barricade turned a quarter about x: its -y is the map's -z.
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let at = to_map([-83.7, 33.48, 2.2], [s, 0.0, 0.0, s], [-0.25, -0.81, -0.3]);
        assert_eq!(at, [-83.95, 33.78, 1.39]);
        assert_eq!(
            to_map([1.0, 2.0, 3.0], [0.0, 0.0, 0.0, 1.0], [0.5, 0.5, 0.5]),
            [1.5, 2.5, 3.5]
        );
    }
}
