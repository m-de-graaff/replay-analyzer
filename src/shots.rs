//! Shots and bullet hits (Y11S3).
//!
//! Three things in the file make up a shot that hits a player, and none of
//! them names the other two.
//!
//! **The fire event.** In the `movement` stream a gun's `607385fe` update
//! ends in a list of events, `u8 n`, then `n` times `u8 id, u16 arg,
//! payload`. Event `06` is the gun firing, 63 bytes:
//!
//! ```text
//! +0  06            +1  u16 0
//! +3  3 x f32 origin: the muzzle, world metres       +15 f32 1.0
//! +19 3 x f32 direction, a unit vector               +31 f32 0
//! +35 f32 distance from the eye to the impact
//! +39 f32 distance from the muzzle to the impact
//! +43 u32 barrel type (not read)                     +47 16 zero bytes
//! ```
//!
//! What comes before the list in the update is not decoded, so the event is
//! found by its shape (the `06`, the 1.0, the zero and a unit vector), not
//! by walking the message. The game repeats the event in every update
//! while an automatic gun fires, and a player's own recording repeats each
//! about 16 times, with the origin following the muzzle and the direction
//! and distances unchanged. A shot is therefore a run of events of one gun
//! with the same direction and eye distance; a shotgun writes one event per
//! shell, not per pellet. The gun is the entity the update belongs to: the
//! body it names in its first updates carries it, and that body's player
//! fired (see [`crate::loadout`]). Devices fire the same event (the laser
//! of Twitch's drone, the burst of a bulletproof camera); they name no
//! body, and belong to the only body with their asset in a gadget slot.
//!
//! The record's frame is when: the clock places offsets of the `state`
//! stream only, so an event here takes the reading in force at the end of
//! the state record of its frame.
//!
//! **The hit effect.** A record of the `FXChannel` stream (`f5ee6a3d`) is a
//! `u8` mask and one section per bit, each a `u32` count and its entries:
//!
//! ```text
//! 01 spawn  36 bytes: u64 asset, u64 parent, u32 instance, u32,
//!                     u64 target entity, u32
//! 04        12 bytes
//! 08 float  12 bytes: u32 instance, hash, f32
//! 10 vector 24 bytes: u32 instance, hash, 4 x f32
//! ```
//!
//! and further sections that are not read. A bullet striking a body spawns
//! asset `d5 6d 41 58` with the body as its target, and the vector
//! parameter `56 95 b5 31` of that instance is where it struck, in world
//! metres.
//!
//! **The damage block.** The update of a body that took damage ends in 24
//! bytes, followed by up to 23 more:
//!
//! ```text
//! +0  f32 health left as a share of the maximum; below zero once down
//!         or dead, to 20 health under it (-0.18 of 110)
//! +4  f32 damage multiplier: 1.0, about 0.75 for a limb
//! +8  u32 (unknown, below 64)
//! +12 u32 state after: 1 alive, 3 down, 4 dead (2 seen alive with 4
//!         health left)
//! +16 u32 (unknown, 1 or 2)
//! +20 u32 damage type: 0 for a bullet
//! ```
//!
//! The block has no marker, and what precedes it is not decoded: it is
//! taken to be there when the bytes hold values a block can.
//!
//! A hit is a hit effect on a player's body. The bullet damage block of
//! that body within [`BLOCK_WINDOW`] says whether a limb was struck and
//! what became of the victim, and gives the damage: the drop of the share
//! since the body's block before, counting no lower than zero, times the
//! player's maximum health, which is the most the `state` stream's `Health`
//! property showed for them. Health regained between two blocks is not
//! seen, so a hit after healing reads low, or has no damage when the share
//! rose. Pellets of one shell share a block, and the first has its damage.
//! A hit without a block is a bullet in a body that was already dead.
//!
//! Who fired is in neither. The shooter of a hit is the shot whose ray
//! passes within [`RAY_REACH`] of where the bullet struck, around the same
//! time; hits say so with `shooterSource: "ray"`, and have no shooter when
//! no ray fits.
//!
//! The file has no field for the body part: `limb` is all there is, and a
//! headshot is only known for a kill, from the kill feed's flag.

use std::collections::HashMap;

use rayon::prelude::*;
use serde::Serialize;

use crate::entities::Hash;
use crate::loadout::{
    Entities, Hud, Input, MOVEMENT_STREAM, Named, PRIMARY_WEAPON, SECONDARY_WEAPON, SLOT_FIELDS,
    STATE_STREAM, UPDATE, When, messages,
};

/// Name hash of the effects stream (`FXChannel`).
const EFFECTS_STREAM: Hash = [0xF5, 0xEE, 0x6A, 0x3D];
/// The event id of a gun firing, and the size of the event.
const FIRE: u8 = 0x06;
const FIRE_SIZE: usize = 63;
/// A fire event starts no earlier than this in an update.
const FIRE_FROM: usize = 6;
/// The body slots that hold what can fire, as the loadout's slot fields
/// order them, and what a shot calls each.
const PRIMARY_GADGET: Hash = [0x08, 0x2C, 0xA3, 0x1D];
const SECONDARY_GADGET: Hash = [0xD8, 0x55, 0xB4, 0xAF];
const BODY_SLOTS: [Hash; 4] = [
    PRIMARY_WEAPON,
    SECONDARY_WEAPON,
    PRIMARY_GADGET,
    SECONDARY_GADGET,
];
const SLOT_NAMES: [&str; 4] = ["primary", "secondary", "ability", "gadget"];
/// Positions are within this many metres of the map's origin.
const WORLD: f32 = 1.0e4;
/// The effect a bullet spawns on the body it strikes.
const HIT_EFFECT: u64 = 0x5841_6DD5;
/// The vector parameter of a hit effect holding where the bullet struck.
const HIT_POSITION: Hash = [0x56, 0x95, 0xB5, 0x31];
/// Effect stream sections: the bit of each and the size of its entries.
const SPAWNS: (u8, usize) = (0x01, 36);
const INTEGERS: (u8, usize) = (0x04, 12);
const FLOATS: (u8, usize) = (0x08, 12);
const VECTORS: (u8, usize) = (0x10, 24);
/// The size of a damage block, and how many bytes can follow it.
const BLOCK_SIZE: usize = 24;
const BLOCK_TAIL: usize = 23;
/// The states of a body after damage that say it is down or dead.
const DOWN: u32 = 3;
const DEAD: u32 = 4;
/// The damage type of a bullet.
const BULLET: u32 = 0;
/// A multiplier below this is a limb (0.72 to 0.75 seen).
const LIMB: f32 = 0.9;
/// The `Health` property of the state stream, followed by its size.
const HEALTH: [u8; 5] = [0x25, 0x26, 0x76, 0xC9, 0x04];
/// No player has more health than this.
const HEALTH_LIMIT: u32 = 400;
/// A hit's damage block is written within this many seconds of its effect.
const BLOCK_WINDOW: f64 = 0.05;
/// A hit's shot started no more than `SHOT_BEFORE` seconds before the hit
/// and `SHOT_AFTER` after, and its last event is no older than `SHOT_ENDED`.
const SHOT_BEFORE: f64 = 0.4;
const SHOT_AFTER: f64 = 0.2;
const SHOT_ENDED: f64 = 0.1;
/// The fire event is written this long after the hit effect, typically.
const SHOT_LAG: f64 = 0.035;
/// Metres a shot's ray may pass from the hit, and reach past its impact.
const RAY_REACH: f32 = 0.6;
/// Seconds weigh this many metres when choosing between rays.
const TIME_WEIGHT: f64 = 0.5;

/// One shot.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shot {
    /// Who fired. Absent when the gun could not be linked to a player.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// The slot of the shooter's loadout that fired: `primary` or
    /// `secondary` for a gun, `ability` or `gadget` for a device.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<&'static str>,
    /// What is in that slot, by the item id loadouts and the kill feed use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weapon: Option<Named>,
    /// The muzzle when the shot was fired, in world metres.
    pub origin: [f32; 3],
    /// A unit vector.
    pub direction: [f32; 3],
    /// Metres from the muzzle to what the bullet struck first.
    pub distance: f32,
    /// The same from the shooter's eye.
    pub eye_distance: f32,
    #[serde(flatten)]
    pub when: When,
}

/// What became of the victim of a hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HitResult {
    Alive,
    Down,
    Dead,
}

/// One bullet hit on a player.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub victim: String,
    /// Where the bullet struck, in world metres.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<[f32; 3]>,
    /// Health taken. Absent, with `limb` and `result`, for a bullet in a
    /// body that took no damage from it: one already dead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damage: Option<u32>,
    /// The bullet struck an arm or a leg. Head and torso are not told
    /// apart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limb: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<HitResult>,
    /// Who fired, with how that is known: `ray`, the shot whose ray passes
    /// through `position`. The file does not name the shooter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shooter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shooter_source: Option<&'static str>,
    /// Index of that shot in `shots`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shot: Option<usize>,
    #[serde(flatten)]
    pub when: When,
}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    pub shots: Vec<Shot>,
    pub hits: Vec<Hit>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
}

fn f32_at(d: &[u8], at: usize) -> Option<f32> {
    Some(f32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(d: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(at..at + 8)?.try_into().ok()?))
}

fn vec3_at(d: &[u8], at: usize) -> Option<[f32; 3]> {
    Some([f32_at(d, at)?, f32_at(d, at + 4)?, f32_at(d, at + 8)?])
}

/// One fire event.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fire {
    origin: [f32; 3],
    direction: [f32; 3],
    eye_distance: f32,
    distance: f32,
}

impl Fire {
    /// What tells one shot from the next: the bits of the direction and of
    /// the eye distance.
    fn key(&self) -> [u32; 4] {
        let [x, y, z] = self.direction.map(f32::to_bits);
        [x, y, z, self.eye_distance.to_bits()]
    }
}

/// The fire event at `at` of an update, when the bytes there have its
/// shape.
fn fire_at(update: &[u8], at: usize) -> Option<Fire> {
    let event = update.get(at..at.checked_add(FIRE_SIZE)?)?;
    if event[0] != FIRE || event[15..19] != 1f32.to_le_bytes() || event[31..35] != [0; 4] {
        return None;
    }
    let origin = vec3_at(event, 3)?;
    let direction = vec3_at(event, 19)?;
    let (eye_distance, distance) = (f32_at(event, 35)?, f32_at(event, 39)?);
    let length: f32 = direction.iter().map(|v| v * v).sum();
    // NaN fails every comparison.
    let sane = length > 0.98
        && length < 1.02
        && origin.iter().all(|v| v.abs() < WORLD)
        && eye_distance.is_finite()
        && distance.is_finite();
    sane.then_some(Fire {
        origin,
        direction,
        eye_distance,
        distance,
    })
}

/// `(offset, event)` of every fire event in a `607385fe` update.
fn fire_events(update: &[u8]) -> Vec<(usize, Fire)> {
    let mut out = Vec::new();
    let mut from = FIRE_FROM;
    while let Some(found) = update
        .get(from..)
        .and_then(|rest| memchr::memchr(FIRE, rest))
    {
        let at = from + found;
        if at + FIRE_SIZE > update.len() {
            break;
        }
        match fire_at(update, at) {
            Some(fire) => {
                out.push((at, fire));
                from = at + FIRE_SIZE;
            }
            None => from = at + 1,
        }
    }
    out
}

/// The damage block that ends a body's update.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Block {
    /// Health left, as a share of the maximum.
    ratio: f32,
    multiplier: f32,
    /// 1 alive, 3 down, 4 dead.
    state: u32,
    /// The damage type.
    kind: u32,
}

/// The damage block of a `607385fe` update of a body, when it has one: the
/// last 24 bytes with the values a block holds, with up to [`BLOCK_TAIL`]
/// bytes after them.
fn damage_block(update: &[u8]) -> Option<Block> {
    (1..=BLOCK_TAIL).find_map(|tail| {
        let at = update.len().checked_sub(tail + BLOCK_SIZE)?;
        if at < FIRE_FROM {
            return None;
        }
        let (ratio, multiplier) = (f32_at(update, at)?, f32_at(update, at + 4)?);
        let unknown = [u32_at(update, at + 8)?, u32_at(update, at + 16)?];
        let (state, kind) = (u32_at(update, at + 12)?, u32_at(update, at + 20)?);
        let fits = (1..=5).contains(&state)
            && unknown[0] < 64
            && unknown[1] < 8
            && kind < 256
            && ratio > -2.0
            && ratio <= 1.0
            && ratio.abs() > 1e-6
            && multiplier > 1e-6
            && multiplier <= 1.0;
        fits.then_some(Block {
            ratio,
            multiplier,
            state,
            kind,
        })
    })
}

/// A hit effect of one effects record.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Effect {
    /// The entity struck.
    target: u64,
    position: Option<[f32; 3]>,
}

/// The hit effects of an effects record, and whether the record held what
/// its counts promise as far as it is read.
fn hit_effects(record: &[u8]) -> (Vec<Effect>, bool) {
    let Some(&mask) = record.first() else {
        return (Vec::new(), true);
    };
    let mut at = 1;
    // The entries of a section the mask names: `(start, count)`.
    let mut section = |(bit, size): (u8, usize)| -> Option<(usize, usize)> {
        if mask & bit == 0 {
            return Some((at, 0));
        }
        let count = u32_at(record, at)? as usize;
        let start = at + 4;
        let end = start.checked_add(count.checked_mul(size)?)?;
        if end > record.len() {
            return None;
        }
        at = end;
        Some((start, count))
    };
    let mut hits: Vec<(u32, Effect)> = Vec::new();
    let Some((start, count)) = section(SPAWNS) else {
        return (Vec::new(), false);
    };
    for at in (0..count).map(|i| start + i * SPAWNS.1) {
        let entry = (
            u64_at(record, at),
            u32_at(record, at + 16),
            u64_at(record, at + 24),
        );
        if let (Some(HIT_EFFECT), Some(instance), Some(target)) = entry {
            let effect = Effect {
                target,
                position: None,
            };
            hits.push((instance, effect));
        }
    }
    let effects = |hits: Vec<(u32, Effect)>| hits.into_iter().map(|h| h.1).collect();
    if hits.is_empty() {
        return (Vec::new(), true);
    }
    let vectors = section(INTEGERS)
        .and_then(|_| section(FLOATS))
        .and_then(|_| section(VECTORS));
    let Some((start, count)) = vectors else {
        return (effects(hits), false);
    };
    for at in (0..count).map(|i| start + i * VECTORS.1) {
        if record.get(at + 4..at + 8) != Some(&HIT_POSITION) {
            continue;
        }
        let position = vec3_at(record, at + 8).filter(|p| p.iter().all(|v| v.abs() < WORLD));
        let instance = u32_at(record, at);
        if let Some((_, hit)) = hits.iter_mut().find(|h| Some(h.0) == instance) {
            hit.position = position;
        }
    }
    (effects(hits), true)
}

/// A gun that fired, with who carries it.
#[derive(Clone, Debug, Default)]
struct Gun {
    body: Option<u64>,
    /// Index into the players.
    player: Option<usize>,
    slot: Option<&'static str>,
    weapon: Option<Named>,
}

/// A shot while it is being put together.
struct Run {
    gun: u64,
    key: [u32; 4],
    fire: Fire,
    /// The frames of its first and last events.
    frame: u32,
    last: u32,
}

/// A fire event or a damage block, as found in a movement record.
enum Found {
    Fire(u64, Fire),
    Block(u64, Block),
}

/// A body's damage block with what it took off.
struct Damage {
    seconds: f64,
    block: Block,
    /// The drop of the health share since the body's block before.
    taken: f32,
    /// A hit already has this block's damage.
    used: bool,
}

/// The most the state stream's `Health` showed, per health object.
fn max_health(input: &Input) -> HashMap<u32, u32> {
    let mut out: HashMap<u32, u32> = HashMap::new();
    for (start, end, _) in input.blocks(STATE_STREAM) {
        let Some(block) = input.data.get(start..end) else {
            continue;
        };
        // `23 <object> 00000000 <hash> 04 <value>`.
        for at in memchr::memmem::find_iter(block, &HEALTH) {
            let Some(head) = at.checked_sub(9).and_then(|h| block.get(h..at)) else {
                continue;
            };
            if head[0] != 0x23 || head[5..9] != [0; 4] {
                continue;
            }
            let (Some(object), Some(value)) = (u32_at(head, 1), u32_at(block, at + 5)) else {
                continue;
            };
            if value <= HEALTH_LIMIT {
                let most = out.entry(object).or_default();
                *most = (*most).max(value);
            }
        }
    }
    out
}

pub(crate) fn decode(input: &Input) -> Decoded {
    let &Input {
        data,
        players,
        clock,
        ..
    } = input;
    let mut out = Decoded::default();
    let ids: Vec<u64> = players.iter().map(|p| p.id).filter(|&i| i != 0).collect();
    let records: Vec<(usize, usize, Option<u32>)> = input.blocks(MOVEMENT_STREAM).collect();
    let spans: Vec<(usize, usize)> = records.iter().map(|&(s, e, _)| (s, e)).collect();
    let entities = Entities::read(data, &spans, &ids);
    // The player a body belongs to: the one an update of it names, or the
    // one the player table links it to.
    let player_of = |body: u64| -> Option<usize> {
        let owner = entities.owner(body);
        players
            .iter()
            .position(|p| p.id != 0 && Some(p.id) == owner)
            .or_else(|| {
                players.iter().position(|p| {
                    let linked = p.entities.as_ref().and_then(|e| e.movement);
                    linked.map(u64::from) == Some(body)
                })
            })
    };
    let bodies: HashMap<u64, Option<usize>> = (entities.bodies().iter())
        .map(|&b| (b, player_of(b)))
        .collect();

    // Damage blocks of the bodies and fire events of everything else, per
    // record. What fires is a gun, or a device such as Twitch's drone.
    let found: Vec<(u32, Vec<Found>)> = records
        .par_iter()
        .filter_map(|&(start, end, frame)| {
            let (block, frame) = (data.get(start..end)?, frame?);
            let mut found = Vec::new();
            for (entity, _, payload) in messages(block) {
                if !payload.starts_with(&UPDATE) {
                    continue;
                }
                if !bodies.contains_key(&entity) {
                    if payload.len() <= FIRE_SIZE {
                        continue;
                    }
                    let fires = fire_events(payload).into_iter();
                    found.extend(fires.map(|(_, f)| Found::Fire(entity, f)));
                } else if let Some(block) = damage_block(payload) {
                    found.push(Found::Block(entity, block));
                }
            }
            (!found.is_empty()).then_some((frame, found))
        })
        .collect();
    let mut runs: Vec<Run> = Vec::new();
    // Gun -> its latest run.
    let mut latest: HashMap<u64, usize> = HashMap::new();
    let mut damage: HashMap<u64, Vec<Damage>> = HashMap::new();
    for (frame, found) in found {
        for f in found {
            match f {
                Found::Fire(gun, fire) => {
                    let key = fire.key();
                    match latest.get(&gun).and_then(|&i| runs.get_mut(i)) {
                        Some(run) if run.key == key => run.last = frame,
                        _ => {
                            latest.insert(gun, runs.len());
                            runs.push(Run {
                                gun,
                                key,
                                fire,
                                frame,
                                last: frame,
                            });
                        }
                    }
                }
                Found::Block(body, block) => {
                    let Some(seconds) = clock.seconds(Some(frame)) else {
                        continue;
                    };
                    let blocks = damage.entry(body).or_default();
                    // Below zero a body is down or dead: no health is left
                    // to take.
                    let before = blocks.last().map_or(1.0, |d| d.block.ratio.max(0.0));
                    blocks.push(Damage {
                        seconds,
                        block,
                        taken: before - block.ratio.max(0.0),
                        used: false,
                    });
                }
            }
        }
    }

    // Who carries each gun that fired.
    let mut carriers: HashMap<u64, Gun> = HashMap::new();
    if !runs.is_empty() {
        let mut hud = Hud::default();
        for (start, end, frame) in input.blocks(STATE_STREAM) {
            if let Some(block) = data.get(start..end) {
                hud.read(block, start, frame);
            }
        }
        for &gun in latest.keys() {
            let asset = entities.get(gun).map(|d| d.asset);
            let slot_of = |body: u64| {
                let body = entities.get(body)?;
                let holds = |s: &Hash| asset.is_some_and(|a| a != 0) && body.slot(*s) == asset;
                BODY_SLOTS.iter().position(holds)
            };
            // The body the gun names, or the only one with a slot for it.
            let body = entities.parent(data, gun).or_else(|| {
                let mut carrying = bodies.keys().copied().filter(|&b| slot_of(b).is_some());
                carrying.next().filter(|_| carrying.next().is_none())
            });
            let player = body.and_then(|b| bodies.get(&b).copied().flatten());
            let slot = body.and_then(slot_of);
            let weapon = player.zip(slot).and_then(|(p, slot)| {
                let controller = players.get(p)?.entities.as_ref()?.controller;
                let view = hud.view(controller)?;
                let (object, _) = hud.slot_object(view, *SLOT_FIELDS.get(slot)?)?;
                hud.item(object).map(Named::item)
            });
            if player.is_none() {
                out.warnings.push(format!(
                    "entity {gun:08x} fires but is not linked to a player: its shots have no shooter"
                ));
            }
            let slot = slot.and_then(|s| SLOT_NAMES.get(s).copied());
            carriers.insert(
                gun,
                Gun {
                    body,
                    player,
                    slot,
                    weapon,
                },
            );
        }
        out.warnings.sort();
    }
    // The clock reads offsets of the state stream: an event of another
    // stream is placed at the end of the state record of its frame.
    let ticks: Vec<(u32, usize)> = input
        .blocks(STATE_STREAM)
        .filter_map(|(_, end, frame)| Some((frame?, end)))
        .collect();
    let when = |frame: u32| {
        let state = ticks.partition_point(|t| t.0 <= frame).checked_sub(1);
        let at = state.and_then(|i| ticks.get(i)).map_or(0, |t| t.1);
        clock.when(at, Some(frame))
    };
    let name = |player: Option<usize>| player.and_then(|p| Some(players.get(p)?.username.clone()));
    out.shots = runs
        .iter()
        .map(|run| {
            let gun = carriers.get(&run.gun);
            Shot {
                username: name(gun.and_then(|g| g.player)),
                slot: gun.and_then(|g| g.slot),
                weapon: gun.and_then(|g| g.weapon),
                origin: run.fire.origin,
                direction: run.fire.direction,
                distance: run.fire.distance,
                eye_distance: run.fire.eye_distance,
                when: when(run.frame),
            }
        })
        .collect();
    // When each shot started and saw its last event; shots are in frame
    // order.
    let spans: Vec<(f64, f64)> = runs
        .iter()
        .map(|r| {
            let seconds = |frame| clock.seconds(Some(frame)).unwrap_or(f64::NAN);
            (seconds(r.frame), seconds(r.last))
        })
        .collect();
    let health = max_health(input);
    let max_of = |player: usize| -> Option<u32> {
        let object = players.get(player)?.entities.as_ref()?.health?;
        health.get(&object).copied().filter(|&h| h > 0)
    };
    let (mut malformed, mut unplaced, mut ownerless) = (0usize, 0usize, 0usize);
    for (start, end, frame) in input.blocks(EFFECTS_STREAM) {
        let (Some(record), Some(frame)) = (data.get(start..end), frame) else {
            continue;
        };
        // Most records spawn no hit effect.
        if memchr::memmem::find(record, &HIT_EFFECT.to_le_bytes()).is_none() {
            continue;
        }
        let (effects, whole) = hit_effects(record);
        malformed += usize::from(!whole);
        for effect in effects {
            // Hit effects also spawn on what is no body.
            let Some(&owner) = bodies.get(&effect.target) else {
                continue;
            };
            let Some(victim) = owner else {
                ownerless += 1;
                continue;
            };
            let seconds = clock.seconds(Some(frame));
            unplaced += usize::from(effect.position.is_none());
            let mut hit = Hit {
                victim: name(Some(victim)).unwrap_or_default(),
                position: effect.position,
                when: when(frame),
                ..Hit::default()
            };
            // The victim's bullet damage nearest in time.
            let block = seconds.and_then(|t| {
                (damage.get_mut(&effect.target)?.iter_mut())
                    .filter(|d| d.block.kind == BULLET && (d.seconds - t).abs() <= BLOCK_WINDOW)
                    .min_by(|a, b| (a.seconds - t).abs().total_cmp(&(b.seconds - t).abs()))
            });
            if let Some(d) = block {
                hit.limb = Some(d.block.multiplier < LIMB);
                hit.result = match d.block.state {
                    DEAD => Some(HitResult::Dead),
                    DOWN => Some(HitResult::Down),
                    _ if d.block.ratio > 0.0 => Some(HitResult::Alive),
                    _ => None,
                };
                // Pellets of one shell share a block: the first has its
                // damage.
                if !d.used && d.taken >= 0.0 {
                    hit.damage = max_of(victim).map(|max| (d.taken * max as f32).round() as u32);
                }
                d.used = true;
            }
            if let (Some(t), Some(position)) = (seconds, effect.position) {
                let shot = ray(&runs, &spans, &carriers, effect.target, position, t);
                if let Some(shot) = shot {
                    hit.shot = Some(shot);
                    hit.shooter = out.shots.get(shot).and_then(|s| s.username.clone());
                    hit.shooter_source = hit.shooter.as_ref().map(|_| "ray");
                }
            }
            out.hits.push(hit);
        }
    }
    if malformed > 0 {
        out.warnings.push(format!(
            "{malformed} effect records with a bullet hit do not hold what their counts promise"
        ));
    }
    if ownerless > 0 {
        out.warnings.push(format!(
            "{ownerless} bullet hits on a body that is no player's"
        ));
    }
    if unplaced > 0 {
        out.warnings
            .push(format!("{unplaced} bullet hits without a position"));
    }
    out
}

/// The shot that struck `victim` at `position`, `seconds` into the
/// recording: the one whose ray passes closest, of those fired around then
/// by someone else.
fn ray(
    runs: &[Run],
    spans: &[(f64, f64)],
    carriers: &HashMap<u64, Gun>,
    victim: u64,
    position: [f32; 3],
    seconds: f64,
) -> Option<usize> {
    let first = spans.partition_point(|s| s.0 < seconds - SHOT_BEFORE);
    let mut best: Option<(f64, usize)> = None;
    for (i, (run, &(started, ended))) in runs.iter().zip(spans).enumerate().skip(first) {
        if started > seconds + SHOT_AFTER {
            break;
        }
        let own = carriers.get(&run.gun).and_then(|g| g.body) == Some(victim);
        // A time that is no number is not recent.
        let recent = ended >= seconds - SHOT_ENDED;
        if own || !recent {
            continue;
        }
        let Fire {
            origin,
            direction,
            distance,
            ..
        } = run.fire;
        let to: [f32; 3] = std::array::from_fn(|k| position[k] - origin[k]);
        let along: f32 = (0..3).map(|k| to[k] * direction[k]).sum();
        if along <= 0.0 || along > distance + RAY_REACH {
            continue;
        }
        let off = (to.iter().map(|v| v * v).sum::<f32>() - along * along)
            .max(0.0)
            .sqrt();
        let near = off < RAY_REACH;
        if !near {
            continue;
        }
        let score = f64::from(off) + TIME_WEIGHT * (started - seconds - SHOT_LAG).abs();
        if best.is_none_or(|b| score < b.0) {
            best = Some((score, i));
        }
    }
    best.map(|b| b.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fire event.
    fn fire(origin: [f32; 3], direction: [f32; 3], eye: f32, muzzle: f32) -> Vec<u8> {
        let mut d = vec![FIRE, 0, 0];
        d.extend(origin.iter().flat_map(|v| v.to_le_bytes()));
        d.extend(1f32.to_le_bytes());
        d.extend(direction.iter().flat_map(|v| v.to_le_bytes()));
        d.extend(0f32.to_le_bytes());
        d.extend(eye.to_le_bytes());
        d.extend(muzzle.to_le_bytes());
        d.extend(1u32.to_le_bytes());
        d.extend([0; 16]);
        assert_eq!(d.len(), FIRE_SIZE);
        d
    }

    /// A `607385fe` update ending in these events, after `before`.
    fn update(before: &[u8], events: &[Vec<u8>]) -> Vec<u8> {
        let mut d = UPDATE.to_vec();
        d.extend(before);
        d.push(events.len() as u8);
        d.extend(events.iter().flatten());
        d
    }

    const UP: [f32; 3] = [0.0, 0.0, 1.0];

    #[test]
    fn reads_a_fire_event() {
        let direction = [0.6, 0.0, 0.8];
        let bytes = update(
            &[0x80, 0x10, 0, 0],
            &[fire([1.5, -2.5, 3.25], direction, 12.5, 11.75)],
        );
        let events = fire_events(&bytes);
        assert_eq!(events.len(), 1);
        let (at, f) = events[0];
        assert_eq!(at, 9);
        assert_eq!(f.origin, [1.5, -2.5, 3.25]);
        assert_eq!(f.direction, direction);
        assert_eq!((f.eye_distance, f.distance), (12.5, 11.75));
    }

    #[test]
    fn reads_fire_events_between_other_events() {
        // Event 04 carries 4 bytes, one of them the id of a fire event.
        let other = vec![0x04, 0, 0, FIRE, FIRE, 0, 0];
        let bytes = update(
            &[FIRE; 5],
            &[
                other.clone(),
                fire([0.0; 3], UP, 1.0, 1.0),
                other,
                fire([0.0; 3], [1.0, 0.0, 0.0], 2.0, 2.0),
            ],
        );
        let events = fire_events(&bytes);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].1.direction, UP);
        assert_eq!(events[1].1.eye_distance, 2.0);
        assert_ne!(events[0].1.key(), events[1].1.key());
    }

    #[test]
    fn bytes_without_the_shape_are_no_fire_event() {
        let good = update(&[0, 0], &[fire([1.0; 3], UP, 5.0, 4.0)]);
        assert_eq!(fire_events(&good).len(), 1);
        // Cut anywhere, it is gone, and nothing panics.
        for cut in 0..good.len() {
            assert!(fire_events(&good[..cut]).is_empty(), "cut at {cut}");
        }
        let at = 7;
        let broken = |change: &dyn Fn(&mut [u8])| {
            let mut d = good.clone();
            change(&mut d[at..]);
            fire_events(&d).len()
        };
        // Not 1.0 after the origin, not 0 after the direction.
        assert_eq!(broken(&|e| e[15] = 1), 0);
        assert_eq!(broken(&|e| e[31] = 1), 0);
        // A direction that is no unit vector, an origin off the map, and
        // distances that are no numbers.
        assert_eq!(
            broken(&|e| e[19..23].copy_from_slice(&1f32.to_le_bytes())),
            0
        );
        assert_eq!(
            broken(&|e| e[3..7].copy_from_slice(&1e9f32.to_le_bytes())),
            0
        );
        assert_eq!(
            broken(&|e| e[3..7].copy_from_slice(&f32::NAN.to_le_bytes())),
            0
        );
        assert_eq!(
            broken(&|e| e[35..39].copy_from_slice(&f32::NAN.to_le_bytes())),
            0
        );
        assert!(fire_events(&[FIRE; 200]).is_empty());
        assert!(fire_events(&[]).is_empty());
    }

    fn block_bytes(ratio: f32, multiplier: f32, state: u32, kind: u32, tail: usize) -> Vec<u8> {
        let mut d = UPDATE.to_vec();
        d.extend([0xAB; 40]);
        d.extend(ratio.to_le_bytes());
        d.extend(multiplier.to_le_bytes());
        for v in [3, state, 1, kind] {
            d.extend(v.to_le_bytes());
        }
        d.extend(vec![0xAB; tail]);
        d
    }

    #[test]
    fn reads_a_damage_block() {
        for tail in [1, 2, 3, 23] {
            let b = damage_block(&block_bytes(0.35, 0.75, 1, 0, tail)).unwrap();
            assert_eq!((b.ratio, b.multiplier, b.state, b.kind), (0.35, 0.75, 1, 0));
        }
        let dead = damage_block(&block_bytes(-0.2, 1.0, 4, 2, 2)).unwrap();
        assert_eq!((dead.ratio, dead.state, dead.kind), (-0.2, 4, 2));
        // No state, no multiplier, a health share no body has, and updates
        // too short to hold one.
        assert_eq!(damage_block(&block_bytes(0.35, 1.0, 9, 0, 2)), None);
        assert_eq!(damage_block(&block_bytes(0.35, 0.0, 1, 0, 2)), None);
        assert_eq!(damage_block(&block_bytes(3.0, 1.0, 1, 0, 2)), None);
        assert_eq!(damage_block(&block_bytes(f32::NAN, 1.0, 1, 0, 2)), None);
        let good = block_bytes(0.35, 1.0, 1, 0, 2);
        for cut in 0..BLOCK_SIZE + 6 {
            assert_eq!(damage_block(&good[..cut]), None, "cut at {cut}");
        }
    }

    /// An effects record: spawns of `(asset, instance, target)` and vector
    /// parameters of `(instance, hash, xyz)`, with a float section between.
    fn effects(spawns: &[(u64, u32, u64)], vectors: &[(u32, Hash, [f32; 3])]) -> Vec<u8> {
        let mut d = vec![SPAWNS.0 | FLOATS.0 | VECTORS.0];
        d.extend((spawns.len() as u32).to_le_bytes());
        for (asset, instance, target) in spawns {
            d.extend(asset.to_le_bytes());
            d.extend(0u64.to_le_bytes());
            d.extend(instance.to_le_bytes());
            d.extend(0u32.to_le_bytes());
            d.extend(target.to_le_bytes());
            d.extend(2u32.to_le_bytes());
        }
        d.extend(1u32.to_le_bytes());
        d.extend([7; 12]);
        d.extend((vectors.len() as u32).to_le_bytes());
        for (instance, hash, v) in vectors {
            d.extend(instance.to_le_bytes());
            d.extend(hash);
            d.extend(v.iter().flat_map(|v| v.to_le_bytes()));
            d.extend(1f32.to_le_bytes());
        }
        d
    }

    #[test]
    fn reads_hit_effects_with_their_position() {
        let record = effects(
            &[
                (9, 1, 0xF000_0001),
                (HIT_EFFECT, 2, 0xF000_0002),
                (HIT_EFFECT, 3, 0xF000_0003),
            ],
            &[
                (2, [1, 2, 3, 4], [9.0; 3]),
                (3, HIT_POSITION, [1.0, 2.0, 3.0]),
                (1, HIT_POSITION, [4.0; 3]),
            ],
        );
        let (hits, whole) = hit_effects(&record);
        assert!(whole);
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].target, hits[0].position), (0xF000_0002, None));
        assert_eq!(hits[1].target, 0xF000_0003);
        assert_eq!(hits[1].position, Some([1.0, 2.0, 3.0]));
    }

    #[test]
    fn malformed_effect_records_are_told() {
        let record = effects(&[(HIT_EFFECT, 2, 5)], &[(2, HIT_POSITION, [1.0; 3])]);
        // Cut in the spawns: nothing. Cut after them: the hit, unplaced.
        for cut in 1..record.len() {
            let (hits, whole) = hit_effects(&record[..cut]);
            assert!(!whole, "cut at {cut}");
            assert_eq!(hits.len(), usize::from(cut >= 5 + 36), "cut at {cut}");
            assert!(hits.iter().all(|h| h.position.is_none()));
        }
        assert_eq!(hit_effects(&[]), (vec![], true));
        // A count the record cannot hold.
        let mut count = record.clone();
        count[1..5].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(hit_effects(&count), (vec![], false));
        // A record without spawns has no hits, whatever follows.
        assert_eq!(hit_effects(&[FLOATS.0, 0xFF, 0xFF]), (vec![], true));
    }

    fn run(gun: u64, origin: [f32; 3], direction: [f32; 3], distance: f32) -> Run {
        let fire = Fire {
            origin,
            direction,
            eye_distance: distance,
            distance,
        };
        Run {
            gun,
            key: fire.key(),
            fire,
            frame: 0,
            last: 0,
        }
    }

    #[test]
    fn a_hit_goes_to_the_shot_whose_ray_passes_through_it() {
        let x = [1.0, 0.0, 0.0];
        let runs = [
            // Too early, the right one, one that passes 1 m off, one that
            // stops short, and the victim's own gun.
            run(1, [0.0; 3], x, 10.0),
            run(1, [0.0; 3], x, 10.0),
            run(2, [0.0, 1.0, 0.0], x, 10.0),
            run(3, [0.0; 3], x, 5.0),
            run(4, [0.0; 3], x, 10.0),
        ];
        let spans = [(1.0, 1.0), (5.0, 5.0), (5.0, 5.0), (5.0, 5.0), (5.0, 5.0)];
        let victim = 77;
        let own = Gun {
            body: Some(victim),
            ..Gun::default()
        };
        let carriers = HashMap::from([(4, own)]);
        let at = |position| ray(&runs, &spans, &carriers, victim, position, 5.0);
        assert_eq!(at([10.2, 0.1, 0.0]), Some(1));
        assert_eq!(at([10.2, 1.0, 0.0]), Some(2));
        // Behind the muzzle, past the impact, and off every ray.
        assert_eq!(at([-1.0, 0.0, 0.0]), None);
        assert_eq!(at([11.0, 0.0, 0.0]), None);
        assert_eq!(at([8.0, 0.0, 3.0]), None);
        // Without anyone else's shot around that time there is no shooter.
        assert_eq!(
            ray(&runs, &spans, &carriers, victim, [10.0, 0.0, 0.0], 9.0),
            None
        );
        assert_eq!(ray(&[], &[], &carriers, victim, [0.0; 3], 5.0), None);
    }
}
