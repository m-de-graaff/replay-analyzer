//! The world of a round (Y11S3): every entity and map object of the
//! `movement` stream with what its messages say of it over time. Read once
//! per round by [`decode`]; the decoders of gadgets, panels, destruction
//! and areas read the [`World`] it returns, not the stream.
//!
//! # The stream
//!
//! A snapshot or record of the stream (see [`crate::loadout::messages`]) is
//! a `u16` count and that many messages, `u64 object, u32 size, payload`.
//! A payload starts with one of four hashes:
//!
//! ```text
//! 617385fe create  +4 u64 id, +16 3 x f32 position, +28 4 x f32 rotation,
//!                  +44 u8, +45 u64 archetype, +53 u32 n, n class hashes,
//!                  u64 asset, u32, u32 count,
//!                  count x {u64 value, slot hash, u32}, 9 bytes,
//!                  u64 size of the state that follows (0)
//! 627385fe create of a map object: the same, with one class, no slots and
//!                  a 37-byte state (its transform group), 131 bytes in all
//! 637385fe delete
//! 607385fe update  u8 mask, then
//!   mask & 80        the transform group: u8 sub, then in this order
//!      sub & 01      f32 x, y, z (world metres, z up), u32 0
//!      sub & 02      f32 x, y, z, w rotation quaternion
//!      sub & 04      u8 live
//!      sub & 08      u16 flags
//!      sub & 10      u8 n, n x {u32 op; op 0 (attach): u64 parent, u64 the
//!                    object, 16 bytes socket, u8 m, u8, and 64 bytes (a
//!                    4 x 4 matrix) when m is 1; op 1: 80 bytes;
//!                    op 2 and 3: nothing}
//!   mask & (40 >> i) the component of class i of the create message, in
//!                    class order
//! ```
//!
//! `sub` `1f` is the full state: what an object is created with, in the
//! snapshot or right after its create message. Objects the game keeps in a
//! pool are created at [`POOL`] long before they are used.
//!
//! Components, by class:
//!
//! ```text
//! 4c60869a placed    u8 m; m & 01: u16 type index, u8 variant;
//!                    m & 02: u64 playerid of who places it;
//!                    m & 04: u64 the object it is fixed to (the host)
//! 8490f616 owner     u8 m; m & 01: u16 type index; m & 02: u64 playerid;
//!                    m & 04: u32 alliance; m & 08: u8 released (1 out of
//!                    the owner's hands, 0 taken back)
//! 513b13b2 state     u8 m; m & 01: u8; m & 02: u8;
//!                    m & 04: u16 n, hash, n - 4 bytes (a state machine's
//!                    blob); m & f8 (all five or none, only in the full
//!                    state): 2 bytes
//! 587f5a72 device    u64 the object's own id, u8 aim (1: 16 bytes follow),
//!                    f32 field of view, u8, u8, u8 full;
//!                    full 0: u64 mount; full 1: 104 bytes, of which
//!                    +74 u8 destroyed, +95 u8 captured, +96 u64 mount;
//!                    when the byte at +77 is 1, a u64 before the mount
//!                    (seen once: 10000)
//! 6ea51c35 damage    u32 count, then per entry `fe` (the object is
//!                    destroyed) or a record of 109 + 40 n bytes
//! ```
//!
//! and a damage record:
//!
//! ```text
//! +0   u8 kind             +1   3 x f32 point, in the object's space
//! +13  f32                 +17  4 x f32 direction
//! +33  u64 instigator      +41  u64 0
//! +49  u64 damage id       +57  u32 shooter slot      +61  u32 part
//! +65  9 x f32             +101 u32                   +105 u32 n
//! +109 n x {3 x f32 position, f32 0, 3 x f32 normal, f32 0, u32 index,
//!           f32 scale}
//! ```
//!
//! An update carries only the damage entries that are new; the full state
//! carries all the object has. A component of any other class has no known
//! size: the walk of a message stops there and keeps what it read.
//!
//! # What is kept
//!
//! [`World::entities`] has every object created, by id, as an [`Entity`]:
//! its create message (classes, asset, slots, place) and when it was
//! deleted. [`World::order`] lists the ids in creation order and
//! [`World::iter`] walks the entities in that order.
//!
//! [`Entity::changes`] is what the object's updates said, in stream order,
//! one [`Change`] per update, each field set only when the update wrote it.
//! [`Entity::until`] gives the part of the list up to a frame.
//!
//! Bodies, guns and attachments have no changes. They are the entities
//! with the class `d96bd5f7`: their updates are most of the stream and
//! belong to other decoders, and their `513b13b2` component has another
//! layout. Of those, the players' bodies (the entities with a slot for a
//! primary weapon or a primary gadget, and those the player table names)
//! get a track in [`World::bodies`] instead: `(frame, position)` about ten
//! times a second, read by [`World::body_at`]. [`World::player_of`] and
//! [`World::body_of`] link a body and a player (an index into
//! `input.players`).
//!
//! Everything else is followed: every map object, and every entity
//! without that class, whatever its classes are (area entities have none).
//!
//! What a `Change` leaves out, to keep a round's list in the tens of
//! thousands and not the hundreds of thousands:
//!
//! - `state` is set only when the component's bytes differ from the
//!   entity's previous one. The game writes the same state every frame.
//! - `device` is set only when it differs from the entity's previous one.
//!   A drone writes it with every position.
//! - an update left with nothing to say after that is not kept.
//!
//! When the walk stops at a component of unknown size, `rest` is the range
//! of the unread bytes in `input.data`, for a decoder that knows what to
//! look for there. An owner component that ends such a message is still
//! read, from the end: the longest form that fits and names a player of
//! the round, else the longest that fits at all, as [`crate::throws`]
//! does. It says so with [`Owner::tail`].
//!
//! # Frames and time
//!
//! `frame` is the frame of the record a message is in; `None` is the
//! opening snapshot, which sorts before every frame. [`seconds`] gives a
//! frame's seconds since the recording started and [`World::when`] the
//! [`When`] events are written with.
//!
//! # Faults
//!
//! A message that does not hold what its mask promises is skipped and
//! counted: [`World::unparsed`] of [`World::updates`]. None is in the ten
//! test rounds, nor in 175 real rounds with 13.8 million such updates.
//! [`World::unread`] counts the updates whose walk stopped at a component
//! of unknown size, which is no fault: about one in fifteen.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use rayon::prelude::*;

use crate::entities::Hash;
use crate::format::Skip;
use crate::loadout::{
    Clock, DESCRIPTOR, Input, MOVEMENT_STREAM, PRIMARY_WEAPON, STATE_STREAM, UPDATE, When, messages,
};

/// Movement payload types besides the update: create of an entity
/// (`617385fe`), create of a map object, delete.
pub(crate) const CREATE: Hash = DESCRIPTOR;
pub(crate) const MAP_CREATE: Hash = [0x62, 0x73, 0x85, 0xFE];
pub(crate) const DELETE: Hash = [0x63, 0x73, 0x85, 0xFE];

/// The component classes with a known layout.
pub(crate) const PLACED: Hash = [0x4C, 0x60, 0x86, 0x9A];
pub(crate) const OWNER: Hash = [0x84, 0x90, 0xF6, 0x16];
pub(crate) const STATE: Hash = [0x51, 0x3B, 0x13, 0xB2];
pub(crate) const DEVICE: Hash = [0x58, 0x7F, 0x5A, 0x72];
pub(crate) const DAMAGE: Hash = [0x6E, 0xA5, 0x1C, 0x35];
/// The class bodies, guns and attachments have, most as their first.
pub(crate) const CARRIED: Hash = [0xD9, 0x6B, 0xD5, 0xF7];
/// The body slot `PrimaryGadget`, which only a body has.
const PRIMARY_GADGET: Hash = [0x08, 0x2C, 0xA3, 0x1D];

/// Where the game keeps an object it has created and not yet put to use.
pub(crate) const POOL: [f32; 3] = [0.0, 0.0, -100.0];

/// A create message lists no more classes and slots than this.
const MAX_CLASSES: usize = 64;
const MAX_SLOTS: usize = 64;
/// The mask of an update has a bit for this many classes.
const MASKED: usize = 7;
/// Update mask: the transform group, and the first class's component.
const TRANSFORM: u8 = 0x80;
const FIRST_CLASS: u8 = 0x40;
/// Every part of the transform group but the operations: the full state.
const FULL: u8 = 0x1F;
/// A damage list has no more entries, and a record no more impacts.
const MAX_DAMAGE: u32 = 4096;
/// The entry of a damage list that says the object is destroyed.
const DESTROYED: u8 = 0xFE;
/// Where the destroyed and captured bytes and the mount are in the full
/// form of a device component, the 104 bytes after its `full` byte.
const DEVICE_DESTROYED: usize = 74;
const DEVICE_CAPTURED: usize = 95;
const DEVICE_MOUNT: usize = 96;
/// The byte of the full form that says 8 more bytes come before the mount.
const DEVICE_LONGER: usize = 77;
/// The longest owner component, and the largest alliance one names.
const OWNER_SIZE: usize = 16;
const MAX_ALLIANCE: u32 = 5;
/// A body's track keeps a position this many seconds after the last.
const BODY_STEP: f64 = 0.1;
/// Nobody moves this fast over the ground (metres a second): a sprint is
/// under 6. Two kept positions further apart than that, at most
/// [`JUMP_STEP`] seconds from each other, are a jump.
const JUMP_SPEED: f64 = 12.0;
const JUMP_STEP: f64 = 0.3;
/// The speed a player had before a jump is taken from this many steps,
/// and one slower than this (metres a second) was not walking.
const JUMP_LEAD: usize = 5;
const WALKING: f64 = 1.0;

/// What kind of object an [`Entity`] is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Created by `617385fe`: something the round spawned.
    #[default]
    Entity,
    /// Created by `627385fe`: part of the map (a wall, a floor, a hatch, a
    /// default camera). Its id is the same in every round on the map.
    MapObject,
}

/// One object of the movement stream.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Entity {
    pub id: u64,
    pub kind: Kind,
    /// What it is: the asset, and the archetype at `+45` of the create
    /// message.
    pub asset: u64,
    pub archetype: u64,
    /// The classes of its components, in the order its updates write them.
    pub classes: Vec<Hash>,
    /// `(slot, item)`; an item of 0 is an empty slot.
    pub slots: Vec<(Hash, u64)>,
    /// Bytes of the create message.
    pub create_size: usize,
    /// The frame it was created in; `None` in the snapshot.
    pub created: Option<u32>,
    /// Where the create message put it.
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    /// The frame of its `637385fe`.
    pub deleted: Option<u32>,
    /// What its updates said, in stream order. Empty for bodies, guns and
    /// attachments.
    pub changes: Vec<Change>,
}

impl Entity {
    pub(crate) fn has(&self, class: Hash) -> bool {
        self.classes.contains(&class)
    }

    /// The item in `slot`; `None` when the entity has no such slot.
    pub(crate) fn slot(&self, slot: Hash) -> Option<u64> {
        self.slots.iter().find(|s| s.0 == slot).map(|s| s.1)
    }

    pub(crate) fn is_map(&self) -> bool {
        self.kind == Kind::MapObject
    }

    /// The changes written up to `frame`, that frame included.
    pub(crate) fn until(&self, frame: Option<u32>) -> &[Change] {
        let end = self.changes.partition_point(|c| c.frame <= frame);
        self.changes.get(..end).unwrap_or_default()
    }
}

/// What one update said of an entity. A field is set when the update
/// wrote it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Change {
    /// The frame of the update; `None` in the snapshot.
    pub frame: Option<u32>,
    /// The update is the full state (`sub` `1f`): what the object was
    /// created with, not something that happened.
    pub full: bool,
    pub position: Option<[f32; 3]>,
    pub rotation: Option<[f32; 4]>,
    /// 1 while the object is in use.
    pub live: Option<u8>,
    pub flags: Option<u16>,
    /// The `4c60869a` component.
    pub placed: Option<Placed>,
    /// The `8490f616` component.
    pub owner: Option<Owner>,
    /// The `513b13b2` component, when it differs from the entity's last.
    pub state: Option<State>,
    /// The `587f5a72` component, when it differs from the entity's last.
    pub device: Option<Device>,
    /// The records of the `6ea51c35` component: the new ones only, but
    /// for a full state, which has every record of the object.
    pub damage: Vec<Damage>,
    /// The damage list has an `fe` entry: the object is destroyed.
    pub destroyed: bool,
    /// `(start, end)` in `input.data` of what the walk did not read: the
    /// bytes from the first component of unknown size on.
    pub rest: Option<(usize, usize)>,
}

impl Change {
    /// The update wrote nothing a field shows.
    fn is_empty(&self) -> bool {
        let transform = self.position.is_some()
            || self.rotation.is_some()
            || self.live.is_some()
            || self.flags.is_some();
        let components = self.placed.is_some()
            || self.owner.is_some()
            || self.state.is_some()
            || self.device.is_some()
            || !self.damage.is_empty()
            || self.destroyed;
        !(transform || components || self.rest.is_some())
    }
}

/// The component of something placed on a surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Placed {
    /// `(type index, variant)`.
    pub type_index: Option<(u16, u8)>,
    /// The `playerid` of who places it.
    pub owner: Option<u64>,
    /// The object it is fixed to.
    pub host: Option<u64>,
}

/// The component of something a player can let go of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Owner {
    pub type_index: Option<u16>,
    /// The owner's `playerid`.
    pub player: Option<u64>,
    pub alliance: Option<u32>,
    /// 1 out of the owner's hands, 0 taken back.
    pub released: Option<u8>,
    /// The component's submask: which of the four it wrote.
    pub form: u8,
    /// Found from the end of the message, behind components of unknown
    /// size, by the player it names.
    pub tail: bool,
}

/// The component of something with a state machine.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct State {
    pub a: Option<u8>,
    pub b: Option<u8>,
    /// The hash that names the state machine and the bytes after it. In
    /// most blobs they are a `u32` size, that many bytes and a `u32` 0;
    /// the bytes after the size start with the state (the CRC-32 of its
    /// name) or a count.
    pub blob: Option<(Hash, Vec<u8>)>,
}

/// The component of a camera or a drone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Device {
    /// Set by the full form of the component only.
    pub destroyed: Option<bool>,
    pub captured: Option<bool>,
    /// The entity it sits on: a bulletproof camera's mount.
    pub mount: Option<u64>,
}

/// One record of a damage list.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Damage {
    /// 0 not a bullet, 1 another player's bullet, 2 the recorder's own
    /// bullet as predicted, 3 the same confirmed.
    pub kind: u8,
    /// Where it struck, in the object's space: `position + rotation *
    /// point`, with the object's transform at that time, puts it on the
    /// map (see [`World::rotate`]).
    pub point: [f32; 3],
    pub direction: [f32; 4],
    /// The body or gadget entity that did it; 0 for damage the map starts
    /// the round with.
    pub instigator: u64,
    /// The damage id: what did it.
    pub id: u64,
    pub slot: u32,
    pub part: u32,
    pub impacts: Vec<Impact>,
}

/// One impact of a damage record, in the object's space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Impact {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub index: u32,
    pub scale: f32,
}

/// What [`decode`] read.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct World {
    /// Every object created, by id. An id the stream created twice has
    /// its later object here and the earlier in `replaced` (not seen).
    pub entities: HashMap<u64, Entity>,
    /// The ids in creation order, each once.
    pub order: Vec<u64>,
    pub replaced: Vec<Entity>,
    /// Body -> `(frame, position)`, about ten a second; the snapshot's
    /// position has frame 0.
    pub bodies: HashMap<u64, Vec<(u32, [f32; 3])>>,
    /// Body -> index into `input.players` of whose it is.
    pub players: HashMap<u64, usize>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
    /// Updates of the entities that are followed; how many of them did
    /// not hold what their mask promises; and how many stopped at a
    /// component of unknown size.
    pub updates: usize,
    pub unparsed: usize,
    pub unread: usize,
    /// `(frame, end of its state record)`, to place a frame on the clock.
    ticks: Vec<(u32, usize)>,
}

impl World {
    pub(crate) fn get(&self, id: u64) -> Option<&Entity> {
        self.entities.get(&id)
    }

    /// The entities in creation order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &Entity> {
        self.order.iter().filter_map(|id| self.entities.get(id))
    }

    /// `q` applied to `v`.
    pub(crate) fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
        let [x, y, z, w] = q;
        let [vx, vy, vz] = v;
        let c = [
            y * vz - z * vy + w * vx,
            z * vx - x * vz + w * vy,
            x * vy - y * vx + w * vz,
        ];
        [
            vx + 2.0 * (y * c[2] - z * c[1]),
            vy + 2.0 * (z * c[0] - x * c[2]),
            vz + 2.0 * (x * c[1] - y * c[0]),
        ]
    }

    /// Where `body` was in `frame`: its last kept position at or before
    /// it, up to a tenth of a second old.
    pub(crate) fn body_at(&self, body: u64, frame: u32) -> Option<[f32; 3]> {
        let track = self.bodies.get(&body)?;
        let i = track.partition_point(|p| p.0 <= frame).checked_sub(1)?;
        track.get(i).map(|p| p.1)
    }

    /// The player `body` belongs to, as an index into `input.players`.
    pub(crate) fn player_of(&self, body: u64) -> Option<usize> {
        self.players.get(&body).copied()
    }

    /// The body of a player: the last one created, when a player got a
    /// new body during the round.
    pub(crate) fn body_of(&self, player: usize) -> Option<u64> {
        let owned = |id: &&u64| self.players.get(id) == Some(&player);
        self.order.iter().rev().find(owned).copied()
    }

    /// The moments the game moved on by more than the recording's clock
    /// did (see [`Skip`]), from the bodies: a player who was walking and
    /// is, one kept position later, further on than anyone can run
    /// ([`JUMP_SPEED`]) has jumped, and two or more players jumping at the
    /// same moment is no teleport of one of them but time the recording
    /// left out. `frame_times` is the seconds since the recording started
    /// of each frame.
    pub(crate) fn skips(&self, frame_times: &[f64]) -> Vec<Skip> {
        let time = |frame: u32| frame_times.get(frame as usize).copied();
        // `(from, until, seconds missing, body)` of each jump.
        let mut jumps: Vec<(f64, f64, f64, u64)> = Vec::new();
        for (&body, track) in &self.bodies {
            // `(end, seconds, metres)` of each step, level.
            let steps: Vec<(f64, f64, f64)> = track
                .windows(2)
                .filter_map(|w| {
                    let (a, b) = (w.first()?, w.get(1)?);
                    let (from, to) = (time(a.0)?, time(b.0)?);
                    let level = f64::from(b.1[0] - a.1[0]).hypot(f64::from(b.1[1] - a.1[1]));
                    Some((to, to - from, level))
                })
                .collect();
            for (i, &(end, took, metres)) in steps.iter().enumerate() {
                if took <= 0.0 || took > JUMP_STEP || metres / took <= JUMP_SPEED {
                    continue;
                }
                // The speed the player had: the median of the steps of
                // the second before.
                let lead = steps
                    .get(i.saturating_sub(JUMP_LEAD)..i)
                    .unwrap_or_default();
                let mut before: Vec<f64> = lead
                    .iter()
                    .filter(|s| end - s.0 <= 1.0 && s.1 > 0.0 && s.1 <= JUMP_STEP)
                    .map(|s| s.2 / s.1)
                    .collect();
                before.sort_by(f64::total_cmp);
                let Some(&speed) = before.get(before.len() / 2).filter(|_| before.len() >= 3)
                else {
                    continue;
                };
                // Someone standing who is suddenly elsewhere was moved.
                if speed >= WALKING {
                    jumps.push((end - took, end, metres / speed - took, body));
                }
            }
        }
        jumps.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.3.cmp(&b.3)));
        let mut out: Vec<Skip> = Vec::new();
        let mut used = vec![false; jumps.len()];
        for (i, first) in jumps.iter().enumerate() {
            if used.get(i).copied().unwrap_or(true) {
                continue;
            }
            // The jumps of other bodies over the same moment.
            let mut group = vec![*first];
            for (k, other) in jumps.iter().enumerate().skip(i + 1) {
                let overlaps = other.0 < first.1 && other.1 > first.0;
                let new = group.iter().all(|g| g.3 != other.3);
                if let (Some(u @ false), true) = (used.get_mut(k), overlaps && new) {
                    *u = true;
                    group.push(*other);
                }
            }
            if group.len() < 2 {
                continue;
            }
            let mut missing: Vec<f64> = group.iter().map(|g| g.2).collect();
            missing.sort_by(f64::total_cmp);
            let seconds = missing.get(missing.len() / 2).copied().unwrap_or(0.0);
            let round = |v: f64| (v * 1000.0).round() / 1000.0;
            out.push(Skip {
                at: round(first.0),
                until: round(group.iter().map(|g| g.1).fold(first.1, f64::max)),
                seconds: (seconds * 100.0).round() / 100.0,
                bodies: group.len(),
            });
        }
        out
    }

    /// When `frame` was, as events say it. The clock is read from the
    /// state stream: a frame takes the reading in force at the end of its
    /// state record, or of the last one before.
    pub(crate) fn when(&self, clock: &Clock, frame: Option<u32>) -> When {
        let i = self.ticks.partition_point(|t| Some(t.0) <= frame);
        let at = i.checked_sub(1).and_then(|i| self.ticks.get(i));
        clock.when(at.map_or(0, |t| t.1), frame)
    }
}

/// Seconds since the recording started for `frame`; the snapshot (`None`)
/// is the start. `None` for a frame the recording does not have.
pub(crate) fn seconds(input: &Input, frame: Option<u32>) -> Option<f64> {
    match frame {
        Some(_) => input.clock.seconds(frame),
        None => Some(0.0),
    }
}

/// The bytes of a payload still to read.
struct Reader<'a> {
    d: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let out = self.d.get(self.at..end)?;
        self.at = end;
        Some(out)
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        self.bytes(n).map(|_| ())
    }

    fn u8(&mut self) -> Option<u8> {
        self.bytes(1)?.first().copied()
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.bytes(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.bytes(8)?.try_into().ok()?))
    }

    fn hash(&mut self) -> Option<Hash> {
        self.bytes(4)?.try_into().ok()
    }

    /// `N` floats, none of them infinite or not a number.
    fn floats<const N: usize>(&mut self) -> Option<[f32; N]> {
        let (floats, _) = self.bytes(4 * N)?.as_chunks::<4>();
        let mut out = [0.0; N];
        for (v, b) in out.iter_mut().zip(floats) {
            *v = f32::from_le_bytes(*b);
        }
        out.iter().all(|v| v.is_finite()).then_some(out)
    }

    fn left(&self) -> usize {
        self.d.len().saturating_sub(self.at)
    }
}

/// Parses a `617385fe` or `627385fe` payload of the object `id`. `None`
/// when it is another type, names another object or does not hold what
/// its counts promise.
fn create(payload: &[u8], id: u64) -> Option<Entity> {
    let mut r = Reader { d: payload, at: 0 };
    let kind = match r.hash()? {
        CREATE => Kind::Entity,
        MAP_CREATE => Kind::MapObject,
        _ => return None,
    };
    if r.u64()? != id {
        return None;
    }
    r.skip(4)?;
    let (position, rotation) = (r.floats()?, r.floats()?);
    r.skip(1)?;
    let archetype = r.u64()?;
    let count = r.u32()? as usize;
    if count > MAX_CLASSES || count * 4 > r.left() {
        return None;
    }
    let classes = (0..count).map(|_| r.hash()).collect::<Option<Vec<_>>>()?;
    let asset = r.u64()?;
    r.skip(4)?;
    let count = r.u32()? as usize;
    if count > MAX_SLOTS || count * 16 > r.left() {
        return None;
    }
    let slots = (0..count)
        .map(|_| {
            let (value, slot) = (r.u64()?, r.hash()?);
            r.skip(4)?;
            Some((slot, value))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Entity {
        id,
        kind,
        asset,
        archetype,
        classes,
        slots,
        create_size: payload.len(),
        position,
        rotation,
        ..Entity::default()
    })
}

/// What a component is, by its class.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Component {
    Placed,
    Owner,
    State,
    Device,
    Damage,
    /// A class with no known layout.
    #[default]
    Unknown,
}

impl Component {
    fn of(class: Hash) -> Component {
        match class {
            PLACED => Component::Placed,
            OWNER => Component::Owner,
            STATE => Component::State,
            DEVICE => Component::Device,
            DAMAGE => Component::Damage,
            _ => Component::Unknown,
        }
    }
}

/// What is read of an entity's updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// Every update, into `changes`.
    Followed,
    /// The position, into the body's track.
    Body,
    /// Nothing: a gun or an attachment.
    Skipped,
}

/// What reading an entity's updates takes: known from its create message.
#[derive(Clone, Copy, Debug)]
struct Known {
    role: Role,
    /// The components the mask of an update has a bit for.
    components: [Component; MASKED],
    count: usize,
}

impl Known {
    fn of(entity: &Entity, bodies: &[u64]) -> Known {
        let is_body = bodies.contains(&entity.id)
            || entity.slot(PRIMARY_WEAPON).is_some()
            || entity.slot(PRIMARY_GADGET).is_some();
        let role = if entity.is_map() {
            Role::Followed
        } else if is_body {
            Role::Body
        } else if entity.has(CARRIED) {
            Role::Skipped
        } else {
            Role::Followed
        };
        let mut components = [Component::Unknown; MASKED];
        for (c, class) in components.iter_mut().zip(&entity.classes) {
            *c = Component::of(*class);
        }
        Known {
            role,
            components,
            count: entity.classes.len().min(MASKED),
        }
    }

    fn components(&self) -> &[Component] {
        self.components.get(..self.count).unwrap_or_default()
    }
}

fn placed(r: &mut Reader) -> Option<Placed> {
    let m = r.u8()?;
    if m & 0xF8 != 0 {
        return None;
    }
    let mut out = Placed::default();
    if m & 0x01 != 0 {
        out.type_index = Some((r.u16()?, r.u8()?));
    }
    if m & 0x02 != 0 {
        out.owner = Some(r.u64()?);
    }
    if m & 0x04 != 0 {
        out.host = Some(r.u64()?);
    }
    Some(out)
}

fn owner(r: &mut Reader) -> Option<Owner> {
    let m = r.u8()?;
    if m & 0xF0 != 0 {
        return None;
    }
    let mut out = Owner {
        form: m,
        ..Owner::default()
    };
    if m & 0x01 != 0 {
        out.type_index = Some(r.u16()?);
    }
    if m & 0x02 != 0 {
        out.player = Some(r.u64()?);
    }
    if m & 0x04 != 0 {
        out.alliance = Some(r.u32()?);
    }
    if m & 0x08 != 0 {
        out.released = Some(r.u8()?);
    }
    Some(out)
}

/// The owner component that ends `tail`: the longest form that fits and
/// names one of `players`.
fn owner_at_end(tail: &[u8], players: &[u64]) -> Option<Owner> {
    let form = |n: usize| {
        let d = tail.get(tail.len().checked_sub(n)?..)?;
        let mut r = Reader { d, at: 0 };
        let o = owner(&mut r)?;
        let plausible = r.left() == 0
            && o.player.is_none_or(|p| players.contains(&p))
            && o.alliance.is_none_or(|a| a <= MAX_ALLIANCE)
            && o.released.is_none_or(|r| r <= 1);
        plausible.then_some(Owner { tail: true, ..o })
    };
    // A form with a player is told from other bytes by the id; one
    // without is a few small numbers, so it is taken only when no form
    // with a player fits.
    let sizes = || (2..=OWNER_SIZE).rev();
    let named = sizes().find_map(|n| form(n).filter(|o| o.player.is_some()));
    named.or_else(|| sizes().find_map(form))
}

/// A state component whose blob borrows the payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StateRef<'a> {
    a: Option<u8>,
    b: Option<u8>,
    blob: Option<(Hash, &'a [u8])>,
}

impl StateRef<'_> {
    fn owned(self) -> State {
        State {
            a: self.a,
            b: self.b,
            blob: self.blob.map(|(hash, bytes)| (hash, bytes.to_vec())),
        }
    }
}

fn state<'a>(r: &mut Reader<'a>) -> Option<StateRef<'a>> {
    let m = r.u8()?;
    let a = if m & 0x01 != 0 { Some(r.u8()?) } else { None };
    let b = if m & 0x02 != 0 { Some(r.u8()?) } else { None };
    let blob = if m & 0x04 != 0 {
        let size = usize::from(r.u16()?);
        Some((r.hash()?, r.bytes(size.checked_sub(4)?)?))
    } else {
        None
    };
    match m & 0xF8 {
        0 => {}
        0xF8 => r.skip(2)?,
        _ => return None,
    }
    Some(StateRef { a, b, blob })
}

fn device(r: &mut Reader, id: u64) -> Option<Device> {
    if r.u64()? != id {
        return None;
    }
    match r.u8()? {
        0 => {}
        1 => r.skip(16)?,
        _ => return None,
    }
    r.skip(6)?;
    let entity = |id: u64| (id != 0).then_some(id);
    match r.u8()? {
        0 => Some(Device {
            mount: entity(r.u64()?),
            ..Device::default()
        }),
        1 => {
            let full = r.bytes(DEVICE_MOUNT)?;
            match *full.get(DEVICE_LONGER)? {
                0 => {}
                1 => r.skip(8)?,
                _ => return None,
            }
            Some(Device {
                destroyed: Some(*full.get(DEVICE_DESTROYED)? != 0),
                captured: Some(*full.get(DEVICE_CAPTURED)? != 0),
                mount: entity(r.u64()?),
            })
        }
        _ => None,
    }
}

/// Reads a damage list into `change`.
fn damage(r: &mut Reader, change: &mut Change) -> Option<()> {
    let count = r.u32()?;
    if count > MAX_DAMAGE {
        return None;
    }
    for _ in 0..count {
        let kind = r.u8()?;
        if kind == DESTROYED {
            change.destroyed = true;
            continue;
        }
        let point = r.floats()?;
        r.skip(4)?;
        let direction = r.floats()?;
        let instigator = r.u64()?;
        r.skip(8)?;
        let (id, slot, part) = (r.u64()?, r.u32()?, r.u32()?);
        r.skip(40)?;
        let impacts = r.u32()?;
        if impacts > MAX_DAMAGE || impacts as usize * 40 > r.left() {
            return None;
        }
        let impacts = (0..impacts)
            .map(|_| {
                let position = r.floats()?;
                r.skip(4)?;
                let normal = r.floats()?;
                r.skip(4)?;
                Some(Impact {
                    position,
                    normal,
                    index: r.u32()?,
                    scale: f32::from_le_bytes(r.bytes(4)?.try_into().ok()?),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        change.damage.push(Damage {
            kind,
            point,
            direction,
            instigator,
            id,
            slot,
            part,
            impacts,
        });
    }
    Some(())
}

/// The mask of an update and its transform group, read into `change`.
fn transform(r: &mut Reader, change: &mut Change) -> Option<u8> {
    let mask = r.u8()?;
    if mask & TRANSFORM == 0 {
        return Some(mask);
    }
    let sub = r.u8()?;
    if sub & 0xE0 != 0 {
        return None;
    }
    change.full = sub == FULL;
    if sub & 0x01 != 0 {
        change.position = Some(r.floats()?);
        r.skip(4)?;
    }
    if sub & 0x02 != 0 {
        change.rotation = Some(r.floats()?);
    }
    if sub & 0x04 != 0 {
        change.live = Some(r.u8()?);
    }
    if sub & 0x08 != 0 {
        change.flags = Some(r.u16()?);
    }
    if sub & 0x10 != 0 {
        for _ in 0..r.u8()? {
            match r.u32()? {
                0 => {
                    r.skip(32)?;
                    let matrix = r.u8()?;
                    r.skip(1)?;
                    match matrix {
                        0 => {}
                        1 => r.skip(64)?,
                        _ => return None,
                    }
                }
                1 => r.skip(80)?,
                2 | 3 => {}
                _ => return None,
            }
        }
    }
    Some(mask)
}

/// Walks a `607385fe` payload of the object `id`, whose classes are
/// `components`. Returns the change, without its frame and state, and the
/// `(start, end)` of the state component in the payload. `None` when the
/// payload does not hold what its mask promises. `players` are the
/// `playerid`s an owner found from the end can be. `rest` is in the
/// payload too.
fn update(
    payload: &[u8],
    id: u64,
    components: &[Component],
    players: &[u64],
) -> Option<(Change, Option<(usize, usize)>)> {
    let mut r = Reader { d: payload, at: 0 };
    if r.hash()? != UPDATE {
        return None;
    }
    let mut change = Change::default();
    let mask = transform(&mut r, &mut change)?;
    let written = |i: usize| mask & (FIRST_CLASS >> i) != 0;
    let mut span = None;
    for (i, component) in components.iter().enumerate() {
        if !written(i) {
            continue;
        }
        match component {
            Component::Placed => change.placed = Some(placed(&mut r)?),
            Component::Owner => change.owner = Some(owner(&mut r)?),
            Component::State => {
                let start = r.at;
                state(&mut r)?;
                span = Some((start, r.at));
            }
            Component::Device => change.device = Some(device(&mut r, id)?),
            Component::Damage => damage(&mut r, &mut change)?,
            Component::Unknown => {
                change.rest = Some((r.at, payload.len()));
                // An owner component that ends the message can still be
                // read.
                let last = (i + 1..components.len()).rfind(|&j| written(j));
                if let Some(j) = last
                    && components.get(j) == Some(&Component::Owner)
                {
                    change.owner = owner_at_end(payload.get(r.at..)?, players);
                }
                return Some((change, span));
            }
        }
    }
    (r.left() == 0).then_some((change, span))
}

/// The position a body's update writes.
fn body_position(payload: &[u8]) -> Option<[f32; 3]> {
    let mut r = Reader { d: payload, at: 4 };
    let (mask, sub) = (r.u8()?, r.u8()?);
    if mask & TRANSFORM == 0 || sub & 0x01 == 0 {
        return None;
    }
    r.floats()
}

/// A delete message, or an update of an entity that is followed: which
/// entity (an index into those created), in which block, and where the
/// payload is in the data.
#[derive(Clone, Copy, Debug, Default)]
struct Message {
    entity: u32,
    block: u32,
    start: usize,
    len: u32,
}

/// What one block holds.
#[derive(Default)]
struct Scan {
    messages: Vec<Message>,
    /// `(body, position, index of the player its update names)`, the body
    /// as an index into the entities created.
    bodies: Vec<(u32, Option<[f32; 3]>, Option<usize>)>,
}

/// What an entity's messages said.
#[derive(Default)]
struct Followed {
    changes: Vec<Change>,
    deleted: Option<u32>,
    updates: usize,
    unparsed: usize,
    unread: usize,
}

/// Reads the messages of one entity, in stream order. `frames` are the
/// frames of the blocks.
fn follow(
    data: &[u8],
    frames: &[(usize, usize, Option<u32>)],
    messages: &[Message],
    id: u64,
    known: &Known,
    players: &[u64],
) -> Followed {
    let mut out = Followed::default();
    // The entity's last state component and device, to leave repeats out.
    let mut last_state: &[u8] = &[];
    let mut last_device = None;
    for m in messages {
        let frame = frames.get(m.block as usize).and_then(|b| b.2);
        let Some(payload) = data.get(m.start..m.start + m.len as usize) else {
            continue;
        };
        if payload.starts_with(&DELETE) {
            out.deleted = frame;
            continue;
        }
        out.updates += 1;
        let Some((mut change, span)) = update(payload, id, known.components(), players) else {
            out.unparsed += 1;
            continue;
        };
        if let Some(bytes) = span.and_then(|(start, end)| payload.get(start..end))
            && bytes != last_state
        {
            last_state = bytes;
            let mut r = Reader { d: bytes, at: 0 };
            change.state = state(&mut r).map(StateRef::owned);
        }
        if change.device.is_some() {
            if change.device == last_device {
                change.device = None;
            } else {
                last_device = change.device;
            }
        }
        if let Some((start, end)) = change.rest {
            out.unread += 1;
            // Offsets in the data, not in the payload.
            change.rest = Some((m.start + start, m.start + end));
        }
        if !change.is_empty() {
            change.frame = frame;
            out.changes.push(change);
        }
    }
    out
}

/// Reads the movement stream of a round.
pub(crate) fn decode(input: &Input) -> World {
    let data = input.data;
    let blocks: Vec<(usize, usize, Option<u32>)> = input.blocks(MOVEMENT_STREAM).collect();
    let block = |b: &(usize, usize, Option<u32>)| data.get(b.0..b.1).unwrap_or_default();
    let ids: Vec<u64> = (input.players.iter())
        .map(|p| p.id)
        .filter(|&i| i != 0)
        .collect();
    let player = |id: u64| input.players.iter().position(|p| p.id == id);
    // The bodies the player table names.
    let linked: Vec<(u64, usize)> = (input.players.iter().enumerate())
        .filter_map(|(i, p)| Some((u64::from(p.entities.as_ref()?.movement?), i)))
        .collect();
    let bodies: Vec<u64> = linked.iter().map(|l| l.0).collect();

    // The stream is most of a replay, so it is read in parallel: its
    // blocks once for the create messages, which say how to read an
    // entity's updates, and once more for where each entity's messages
    // are; then the entities, each with its own messages in order.
    let created: Vec<Vec<(usize, Option<Entity>)>> = blocks
        .par_iter()
        .map(|b| {
            messages(block(b))
                .filter(|m| m.2.starts_with(&CREATE) || m.2.starts_with(&MAP_CREATE))
                .map(|(entity, at, payload)| (at, create(payload, entity)))
                .collect()
        })
        .collect();
    let mut malformed = 0usize;
    let mut entities: Vec<Entity> = Vec::new();
    let mut known: Vec<Known> = Vec::new();
    // Id -> where its create message is and its index in `entities`; an
    // id created again (not seen) is in `again`.
    type At = (usize, usize);
    let mut first: HashMap<u64, (At, u32)> = HashMap::new();
    let mut again: Vec<(u64, At, u32)> = Vec::new();
    for (index, (b, created)) in blocks.iter().zip(created).enumerate() {
        for (at, entity) in created {
            let Some(mut entity) = entity else {
                malformed += 1;
                continue;
            };
            entity.created = b.2;
            let Ok(dense) = u32::try_from(entities.len()) else {
                break;
            };
            known.push(Known::of(&entity, &bodies));
            match first.entry(entity.id) {
                Entry::Vacant(v) => {
                    v.insert(((index, at), dense));
                }
                Entry::Occupied(_) => again.push((entity.id, (index, at), dense)),
            }
            entities.push(entity);
        }
    }
    // The entity a message at `at` belongs to: the last one created with
    // its id before it.
    let entity_at = |id: u64, at: At| -> Option<u32> {
        let later = again.iter().rev().find(|a| a.0 == id && a.1 < at);
        later
            .map(|a| a.2)
            .or_else(|| first.get(&id).filter(|f| f.0 < at).map(|f| f.1))
    };

    let scans: Vec<Scan> = blocks
        .par_iter()
        .enumerate()
        .map(|(index, b)| {
            let mut scan = Scan::default();
            for (id, at, payload) in messages(block(b)) {
                let is_update = payload.starts_with(&UPDATE);
                if !is_update && !payload.starts_with(&DELETE) {
                    continue;
                }
                let Some(entity) = entity_at(id, (index, at)) else {
                    continue;
                };
                let Some(role) = known.get(entity as usize).map(|k| k.role) else {
                    continue;
                };
                if role == Role::Followed || !is_update {
                    if let (Ok(block), Ok(len)) = (index.try_into(), payload.len().try_into()) {
                        scan.messages.push(Message {
                            entity,
                            block,
                            start: b.0 + at,
                            len,
                        });
                    }
                } else if role == Role::Body {
                    let named = payload
                        .len()
                        .checked_sub(8)
                        .filter(|&t| t >= 4)
                        .and_then(|t| payload.get(t..)?.try_into().ok())
                        .map(u64::from_le_bytes)
                        .filter(|id| *id != 0 && ids.contains(id))
                        .and_then(player);
                    let position = body_position(payload);
                    if position.is_some() || named.is_some() {
                        scan.bodies.push((entity, position, named));
                    }
                }
            }
            scan
        })
        .collect();

    // Each entity's messages, in stream order: the messages sorted by
    // entity, and where each entity's start.
    let found = || scans.iter().flat_map(|s| &s.messages);
    let mut starts = vec![0usize; entities.len() + 1];
    for m in found() {
        if let Some(count) = starts.get_mut(m.entity as usize + 1) {
            *count += 1;
        }
    }
    let mut total = 0;
    for start in &mut starts {
        total += *start;
        *start = total;
    }
    let mut sorted = vec![Message::default(); total];
    let mut next = starts.clone();
    for m in found() {
        if let Some(at) = next.get_mut(m.entity as usize)
            && let Some(slot) = sorted.get_mut(*at)
        {
            *slot = *m;
            *at += 1;
        }
    }
    let followed: Vec<Followed> = (entities.par_iter().zip(&known).zip(starts.par_windows(2)))
        .map(|((entity, known), span)| {
            let own = span.first().zip(span.get(1));
            let own = own.and_then(|(&from, &to)| sorted.get(from..to));
            let own = own.unwrap_or_default();
            follow(data, &blocks, own, entity.id, known, &ids)
        })
        .collect();

    // A body's position about ten times a second, and whose it is.
    let mut tracks: Vec<Vec<(u32, [f32; 3])>> = vec![Vec::new(); entities.len()];
    let mut owners: Vec<Option<usize>> = vec![None; entities.len()];
    for (b, scan) in blocks.iter().zip(&scans) {
        let now = input.clock.seconds(b.2);
        let frame = b.2.unwrap_or(0);
        for &(body, position, player) in &scan.bodies {
            if let Some(owner) = owners.get_mut(body as usize) {
                *owner = owner.or(player);
            }
            let (Some(position), Some(track)) = (position, tracks.get_mut(body as usize)) else {
                continue;
            };
            let due = match (track.last(), now) {
                (None, _) => true,
                (Some(l), Some(now)) => input
                    .clock
                    .seconds(Some(l.0))
                    .is_none_or(|then| now - then >= BODY_STEP),
                (Some(l), None) => frame >= l.0.saturating_add(3),
            };
            if due {
                track.push((frame, position));
            }
        }
    }

    let mut world = World::default();
    let parts = entities.into_iter().zip(followed).zip(tracks).zip(owners);
    for (((mut entity, followed), track), owner) in parts {
        world.updates += followed.updates;
        world.unparsed += followed.unparsed;
        world.unread += followed.unread;
        entity.changes = followed.changes;
        entity.deleted = followed.deleted;
        let id = entity.id;
        // Whose a body is: the player its updates name, as
        // [`crate::shots`] has it, else the one the player table links.
        let table = || linked.iter().find(|l| l.0 == id).map(|l| l.1);
        if let Some(owner) = owner.or_else(table) {
            world.players.insert(id, owner);
        }
        if !track.is_empty() {
            world.bodies.insert(id, track);
        }
        match world.entities.insert(id, entity) {
            Some(earlier) => world.replaced.push(earlier),
            None => world.order.push(id),
        }
    }
    world.ticks = input
        .blocks(STATE_STREAM)
        .filter_map(|(_, end, frame)| Some((frame?, end)))
        .collect();
    for (count, of, what) in [
        (malformed, None, "create messages do not parse"),
        (
            world.unparsed,
            Some(world.updates),
            "updates of the entities followed do not hold what their mask promises",
        ),
    ] {
        if count > 0 {
            let of = of.map_or(String::new(), |n| format!(" of {n}"));
            world.warnings.push(format!("{count}{of} {what}"));
        }
    }
    world
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The full form of a device component after its `full` byte.
    const DEVICE_FULL: usize = 104;

    const ID: u64 = 0xF000_1234;
    const PLAYER: u64 = 0x1122_3344_5566_7788;

    fn floats(d: &mut Vec<u8>, v: &[f32]) {
        d.extend(v.iter().flat_map(|v| v.to_le_bytes()));
    }

    /// A `607385fe` payload: the mask and what follows it.
    fn message(mask: u8, body: &[u8]) -> Vec<u8> {
        let mut d = UPDATE.to_vec();
        d.push(mask);
        d.extend(body);
        d
    }

    fn read(payload: &[u8], components: &[Component]) -> Option<(Change, Option<(usize, usize)>)> {
        update(payload, ID, components, &[PLAYER])
    }

    /// The change of a payload that must parse and has no state.
    fn change(payload: &[u8], components: &[Component]) -> Change {
        let (change, span) = read(payload, components).expect("parses");
        assert_eq!(span, None);
        change
    }

    #[test]
    fn each_part_of_the_transform_group_is_read() {
        let position = [1.5, -2.5, 3.25];
        let rotation = [0.0, 0.0, 0.6, 0.8];
        let mut p = vec![0x01];
        floats(&mut p, &position);
        p.extend([0; 4]);
        let c = change(&message(0x80, &p), &[]);
        assert_eq!(c.position, Some(position));
        assert_eq!(
            (c.rotation, c.live, c.flags, c.full),
            (None, None, None, false)
        );

        let mut q = vec![0x02];
        floats(&mut q, &rotation);
        let c = change(&message(0x80, &q), &[]);
        assert_eq!((c.position, c.rotation), (None, Some(rotation)));

        let c = change(&message(0x80, &[0x04, 1]), &[]);
        assert_eq!((c.live, c.flags), (Some(1), None));
        let c = change(&message(0x80, &[0x08, 0x00, 0x40]), &[]);
        assert_eq!((c.live, c.flags), (None, Some(0x4000)));

        // Operations: 0 is 34 bytes, or 98 with its matrix, 1 is 80, 2
        // and 3 are nothing.
        let mut ops = vec![0x10, 5];
        ops.extend(0u32.to_le_bytes());
        ops.extend([7; 32]);
        ops.extend([0, 7]);
        ops.extend(0u32.to_le_bytes());
        ops.extend([7; 32]);
        ops.extend([1, 7]);
        ops.extend([7; 64]);
        ops.extend(2u32.to_le_bytes());
        ops.extend(1u32.to_le_bytes());
        ops.extend([7; 80]);
        ops.extend(3u32.to_le_bytes());
        let c = change(&message(0x80, &ops), &[]);
        assert!(c.is_empty());
        // An operation that is not known, an attachment whose matrix byte
        // is no flag, and a part that is not known.
        let mut unknown = vec![0x10, 1];
        unknown.extend(4u32.to_le_bytes());
        assert_eq!(read(&message(0x80, &unknown), &[]), None);
        let mut flag = vec![0x10, 1];
        flag.extend(0u32.to_le_bytes());
        flag.extend([2; 34]);
        assert_eq!(read(&message(0x80, &flag), &[]), None);
        assert_eq!(read(&message(0x80, &[0x20]), &[]), None);

        // All of it, in order: the full state.
        let mut full = vec![0x1F];
        floats(&mut full, &position);
        full.extend([0; 4]);
        floats(&mut full, &rotation);
        full.extend([1, 0x00, 0x80, 0]);
        let c = change(&message(0x80, &full), &[]);
        assert_eq!((c.position, c.rotation), (Some(position), Some(rotation)));
        assert_eq!((c.live, c.flags, c.full), (Some(1), Some(0x8000), true));
        // Without the transform bit nothing of it is there.
        assert!(change(&message(0x00, &[]), &[]).is_empty());
        // A position that is no number is no position.
        let mut nan = vec![0x01];
        floats(&mut nan, &[f32::NAN, 0.0, 0.0]);
        nan.extend([0; 4]);
        assert_eq!(read(&message(0x80, &nan), &[]), None);
    }

    #[test]
    fn a_placed_component_names_type_owner_and_host() {
        let mut p = vec![0x07, 0x2A, 0x00, 3];
        p.extend(PLAYER.to_le_bytes());
        p.extend(0x60_0000_0001u64.to_le_bytes());
        let c = change(&message(0x40, &p), &[Component::Placed]);
        let placed = c.placed.unwrap();
        assert_eq!(placed.type_index, Some((42, 3)));
        assert_eq!(placed.owner, Some(PLAYER));
        assert_eq!(placed.host, Some(0x60_0000_0001));
        // The owner alone, as a placement starts.
        let mut p = vec![0x02];
        p.extend(PLAYER.to_le_bytes());
        let c = change(&message(0x40, &p), &[Component::Placed]);
        let alone = Placed {
            owner: Some(PLAYER),
            ..Placed::default()
        };
        assert_eq!(c.placed, Some(alone));
        // A submask with bits no form has.
        assert_eq!(read(&message(0x40, &[0x08]), &[Component::Placed]), None);
    }

    #[test]
    fn an_owner_component_names_player_alliance_and_release() {
        let mut p = vec![0x0F, 0x10, 0x00];
        p.extend(PLAYER.to_le_bytes());
        p.extend(2u32.to_le_bytes());
        p.push(1);
        let c = change(&message(0x40, &p), &[Component::Owner]);
        let o = c.owner.unwrap();
        assert_eq!((o.type_index, o.player), (Some(16), Some(PLAYER)));
        assert_eq!((o.alliance, o.released), (Some(2), Some(1)));
        assert_eq!((o.form, o.tail), (0x0F, false));
        // The flag alone: a launcher's projectile.
        let c = change(&message(0x40, &[0x08, 0]), &[Component::Owner]);
        assert_eq!(c.owner.unwrap().released, Some(0));
        assert_eq!(read(&message(0x40, &[0x10]), &[Component::Owner]), None);
    }

    /// A state component with a blob of `hash` and `bytes`.
    fn state_bytes(m: u8, hash: Hash, bytes: &[u8]) -> Vec<u8> {
        let mut p = vec![m];
        if m & 0x01 != 0 {
            p.push(5);
        }
        if m & 0x02 != 0 {
            p.push(6);
        }
        if m & 0x04 != 0 {
            p.extend((bytes.len() as u16 + 4).to_le_bytes());
            p.extend(hash);
            p.extend(bytes);
        }
        if m & 0xF8 != 0 {
            p.extend([0; 2]);
        }
        p
    }

    #[test]
    fn a_state_component_is_two_bytes_and_a_blob() {
        let hash = [0xAD, 0x55, 0x0A, 0x19];
        for m in [0x01, 0x02, 0x04, 0x05, 0x07, 0xFF] {
            let p = state_bytes(m, hash, &[1, 2, 3, 4, 5, 6, 7, 8]);
            let payload = message(0x40, &p);
            let (c, span) = read(&payload, &[Component::State]).expect("parses");
            // The walk leaves the component for the merge, which knows
            // the entity's last.
            assert!(c.is_empty(), "form {m:02x}");
            assert_eq!(span, Some((5, payload.len())), "form {m:02x}");
            let mut r = Reader { d: &p, at: 0 };
            let s = state(&mut r).unwrap().owned();
            assert_eq!(s.a, (m & 0x01 != 0).then_some(5));
            assert_eq!(s.b, (m & 0x02 != 0).then_some(6));
            let bytes = vec![1, 2, 3, 4, 5, 6, 7, 8];
            assert_eq!(s.blob, (m & 0x04 != 0).then_some((hash, bytes)));
        }
        // Some of the upper bits and not all, and a blob too short for
        // its hash.
        let p = state_bytes(0x10, hash, &[]);
        assert_eq!(read(&message(0x40, &p), &[Component::State]), None);
        let short = [0x04, 0x02, 0x00, 0xAD, 0x55];
        assert_eq!(read(&message(0x40, &short), &[Component::State]), None);
    }

    /// A device component of `ID`.
    fn device_bytes(aim: bool, full: Option<(bool, bool, u64)>, mount: u64) -> Vec<u8> {
        let mut p = ID.to_le_bytes().to_vec();
        p.push(u8::from(aim));
        if aim {
            p.extend([9; 16]);
        }
        floats(&mut p, &[60.0]);
        p.extend([0, 0]);
        match full {
            Some((destroyed, captured, mount)) => {
                p.push(1);
                let mut f = [7u8; DEVICE_FULL];
                f[DEVICE_LONGER] = 0;
                f[DEVICE_DESTROYED] = u8::from(destroyed);
                f[DEVICE_CAPTURED] = u8::from(captured);
                f[DEVICE_MOUNT..].copy_from_slice(&mount.to_le_bytes());
                p.extend(f);
            }
            None => {
                p.push(0);
                p.extend(mount.to_le_bytes());
            }
        }
        p
    }

    #[test]
    fn a_device_component_has_four_sizes() {
        let sizes = [
            (false, None, 24),
            (true, None, 40),
            (false, Some((false, false, 0)), 120),
            (true, Some((true, false, 0xF0AA)), 136),
        ];
        for (aim, full, size) in sizes {
            let p = device_bytes(aim, full, 0xF0BB);
            assert_eq!(p.len(), size);
            let c = change(&message(0x40, &p), &[Component::Device]);
            let d = c.device.unwrap();
            match full {
                Some((destroyed, captured, mount)) => {
                    assert_eq!(d.destroyed, Some(destroyed));
                    assert_eq!(d.captured, Some(captured));
                    assert_eq!(d.mount, (mount != 0).then_some(mount));
                }
                None => {
                    assert_eq!((d.destroyed, d.captured), (None, None));
                    assert_eq!(d.mount, Some(0xF0BB));
                }
            }
        }
        let captured = device_bytes(false, Some((false, true, 0)), 0);
        let c = change(&message(0x40, &captured), &[Component::Device]);
        assert_eq!(c.device.unwrap().captured, Some(true));
        // The form with a u64 before the mount.
        let mut longer = device_bytes(true, Some((false, false, 0)), 0);
        let at = longer.len() - DEVICE_FULL + DEVICE_LONGER;
        longer[at] = 1;
        assert_eq!(read(&message(0x40, &longer), &[Component::Device]), None);
        let mount = longer.len() - 8;
        longer.splice(mount..mount, 10_000u64.to_le_bytes());
        longer[mount + 8..].copy_from_slice(&0xF0CCu64.to_le_bytes());
        assert_eq!(longer.len(), 144);
        let c = change(&message(0x40, &longer), &[Component::Device]);
        assert_eq!(c.device.unwrap().mount, Some(0xF0CC));
        // Another object's id, and an aim byte that is no flag.
        let mut other = device_bytes(false, None, 0);
        other[0] ^= 1;
        assert_eq!(read(&message(0x40, &other), &[Component::Device]), None);
        let mut aim = device_bytes(false, None, 0);
        aim[8] = 2;
        assert_eq!(read(&message(0x40, &aim), &[Component::Device]), None);
    }

    /// A damage record with these impacts.
    fn record(kind: u8, instigator: u64, id: u64, impacts: &[Impact]) -> Vec<u8> {
        let mut p = vec![kind];
        floats(&mut p, &[0.5, 0.25, 1.5]);
        floats(&mut p, &[9.0]);
        floats(&mut p, &[0.0, 1.0, 0.0, 0.0]);
        p.extend(instigator.to_le_bytes());
        p.extend(0u64.to_le_bytes());
        p.extend(id.to_le_bytes());
        p.extend(4u32.to_le_bytes());
        p.extend(17u32.to_le_bytes());
        floats(&mut p, &[1.0; 9]);
        p.extend(3u32.to_le_bytes());
        p.extend((impacts.len() as u32).to_le_bytes());
        assert_eq!(p.len(), 109);
        for i in impacts {
            floats(&mut p, &i.position);
            floats(&mut p, &[0.0]);
            floats(&mut p, &i.normal);
            floats(&mut p, &[0.0]);
            p.extend(i.index.to_le_bytes());
            floats(&mut p, &[i.scale]);
        }
        p
    }

    fn damage_list(entries: &[Vec<u8>]) -> Vec<u8> {
        let mut p = (entries.len() as u32).to_le_bytes().to_vec();
        p.extend(entries.iter().flatten());
        p
    }

    #[test]
    fn a_damage_record_has_its_impacts() {
        let impacts = [
            Impact {
                position: [0.1, 0.2, 0.3],
                normal: [0.0, 1.0, 0.0],
                index: 7,
                scale: 0.5,
            },
            Impact {
                position: [0.4, 0.5, 0.6],
                normal: [0.0, 0.0, 1.0],
                index: 8,
                scale: 1.5,
            },
        ];
        let one = record(1, 0xF0AA, 34_118_943_362, &impacts);
        assert_eq!(one.len(), 109 + 2 * 40);
        let list = damage_list(&[one.clone(), record(0, 0, 5, &[])]);
        let c = change(&message(0x40, &list), &[Component::Damage]);
        assert!(!c.destroyed);
        assert_eq!(c.damage.len(), 2);
        let d = &c.damage[0];
        assert_eq!((d.kind, d.instigator, d.id), (1, 0xF0AA, 34_118_943_362));
        assert_eq!((d.point, d.slot, d.part), ([0.5, 0.25, 1.5], 4, 17));
        assert_eq!(d.direction, [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(d.impacts, impacts);
        assert_eq!((c.damage[1].kind, c.damage[1].impacts.len()), (0, 0));
        // An empty list is nothing.
        let none = damage_list(&[]);
        assert!(change(&message(0x40, &none), &[Component::Damage]).is_empty());
        // More impacts than the message holds, and more entries.
        let mut many = damage_list(std::slice::from_ref(&one));
        many[4 + 105..4 + 109].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(read(&message(0x40, &many), &[Component::Damage]), None);
        let mut count = damage_list(&[one]);
        count[..4].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(read(&message(0x40, &count), &[Component::Damage]), None);
    }

    #[test]
    fn an_fe_entry_is_the_object_destroyed() {
        let list = damage_list(&[record(0, 0xF0AA, 5, &[]), vec![DESTROYED]]);
        let c = change(&message(0x40, &list), &[Component::Damage]);
        assert!(c.destroyed);
        assert_eq!(c.damage.len(), 1);
        let alone = damage_list(&[vec![DESTROYED]]);
        let c = change(&message(0x40, &alone), &[Component::Damage]);
        assert!(c.destroyed && c.damage.is_empty() && !c.is_empty());
    }

    #[test]
    fn components_follow_the_transform_in_class_order() {
        let classes = [Component::Placed, Component::Damage];
        let mut p = vec![0x04, 1];
        p.extend([0x04]);
        p.extend(0x60_0000_0002u64.to_le_bytes());
        p.extend(damage_list(&[vec![DESTROYED]]));
        let c = change(&message(0xE0, &p), &classes);
        assert_eq!(c.live, Some(1));
        assert_eq!(c.placed.unwrap().host, Some(0x60_0000_0002));
        assert!(c.destroyed);
        // The second class alone.
        let list = damage_list(&[vec![DESTROYED]]);
        let c = change(&message(0x20, &list), &classes);
        assert!(c.placed.is_none() && c.destroyed);
        // Bytes after the last component, and a bit no class has.
        let mut long = list.clone();
        long.push(0);
        assert_eq!(read(&message(0x20, &long), &classes), None);
        assert_eq!(read(&message(0x10, &[1]), &classes), None);
        assert!(change(&message(0x10, &[]), &classes).is_empty());
    }

    #[test]
    fn an_unknown_class_stops_the_walk() {
        let classes = [Component::Placed, Component::Unknown, Component::Damage];
        let mut p = vec![0x04, 0, 0x02];
        p.extend(PLAYER.to_le_bytes());
        p.extend([0xAB; 9]);
        p.extend(damage_list(&[vec![DESTROYED]]));
        let payload = message(0xF0, &p);
        let c = change(&payload, &classes);
        // What came before it is kept, what comes after is not read.
        assert_eq!(c.live, Some(0));
        assert_eq!(c.placed.unwrap().owner, Some(PLAYER));
        assert!(!c.destroyed);
        assert_eq!(c.rest, Some((7 + 9, payload.len())));
        // Without the unknown class's bit the walk goes on.
        let mut p = vec![0x00];
        p.extend(damage_list(&[vec![DESTROYED]]));
        let c = change(&message(0x50, &p), &classes);
        assert!(c.destroyed && c.rest.is_none());
    }

    #[test]
    fn an_owner_behind_an_unknown_class_is_read_from_the_end() {
        let classes = [Component::Device, Component::Unknown, Component::Owner];
        let mut owner = vec![0x0E];
        owner.extend(PLAYER.to_le_bytes());
        owner.extend(1u32.to_le_bytes());
        owner.push(1);
        let mut p = device_bytes(false, Some((false, false, 0)), 0);
        p.extend([0xAB; 13]);
        p.extend(&owner);
        let c = change(&message(0x70, &p), &classes);
        let o = c.owner.unwrap();
        assert_eq!(
            (o.player, o.alliance, o.released),
            (Some(PLAYER), Some(1), Some(1))
        );
        assert!(o.tail);
        assert!(c.device.is_some() && c.rest.is_some());
        // Not when it names no player of the round, when the owner's bit
        // is not set, or when a class behind it wrote too.
        let mut other = p.clone();
        let at = other.len() - 13;
        other[at] ^= 0xFF;
        assert_eq!(change(&message(0x70, &other), &classes).owner, None);
        assert_eq!(change(&message(0x60, &p), &classes).owner, None);
        let more = [
            Component::Device,
            Component::Unknown,
            Component::Owner,
            Component::Unknown,
        ];
        assert_eq!(change(&message(0x78, &p), &more).owner, None);
    }

    #[test]
    fn a_truncated_message_is_no_change() {
        let classes = [Component::Placed, Component::Damage];
        let mut p = vec![0x1F];
        floats(&mut p, &[1.0, 2.0, 3.0]);
        p.extend([0; 4]);
        floats(&mut p, &[0.0, 0.0, 0.0, 1.0]);
        p.extend([1, 0x00, 0x80, 0]);
        p.push(0x07);
        p.extend([0x2A, 0x00, 3]);
        p.extend(PLAYER.to_le_bytes());
        p.extend(9u64.to_le_bytes());
        let impact = Impact::default();
        p.extend(damage_list(&[record(0, 1, 2, &[impact]), vec![DESTROYED]]));
        let whole = message(0xE0, &p);
        assert!(read(&whole, &classes).is_some());
        // Cut anywhere it does not parse, and nothing panics.
        for cut in 0..whole.len() {
            assert_eq!(read(&whole[..cut], &classes), None, "cut at {cut}");
        }
        assert_eq!(read(&[], &[]), None);
        assert_eq!(read(&CREATE, &[]), None);
    }

    /// A create message of `ID`.
    fn create_bytes(kind: Hash, classes: &[Hash], asset: u64, slots: &[(Hash, u64)]) -> Vec<u8> {
        let mut d = kind.to_vec();
        d.extend(ID.to_le_bytes());
        d.extend([0; 4]);
        floats(&mut d, &[1.0, 2.0, 3.0]);
        floats(&mut d, &[0.0, 0.0, 0.0, 1.0]);
        d.push(1);
        d.extend(77u64.to_le_bytes());
        d.extend((classes.len() as u32).to_le_bytes());
        d.extend(classes.iter().flatten());
        d.extend(asset.to_le_bytes());
        d.extend([0; 4]);
        d.extend((slots.len() as u32).to_le_bytes());
        for (slot, value) in slots {
            d.extend(value.to_le_bytes());
            d.extend(slot);
            d.extend([0; 4]);
        }
        d.extend([0; 17]);
        d
    }

    #[test]
    fn a_create_message_describes_the_entity() {
        let slots = [([1, 2, 3, 4], 5), ([5, 6, 7, 8], 0)];
        let bytes = create_bytes(CREATE, &[PLACED, DAMAGE], 406_076_330_089, &slots);
        // A barricade's create message is this long.
        assert_eq!(bytes.len(), 130);
        let e = create(&bytes, ID).unwrap();
        assert_eq!((e.id, e.kind), (ID, Kind::Entity));
        assert_eq!((e.asset, e.archetype), (406_076_330_089, 77));
        assert_eq!(e.classes, vec![PLACED, DAMAGE]);
        assert_eq!(e.slots, slots.to_vec());
        assert_eq!((e.create_size, e.created, e.deleted), (130, None, None));
        assert_eq!(e.position, [1.0, 2.0, 3.0]);
        assert_eq!(e.rotation, [0.0, 0.0, 0.0, 1.0]);
        assert!(e.has(DAMAGE) && !e.has(OWNER) && !e.is_map());
        assert_eq!((e.slot([1, 2, 3, 4]), e.slot([9; 4])), (Some(5), None));

        let map = create(&create_bytes(MAP_CREATE, &[DAMAGE], 0, &[]), ID).unwrap();
        assert!(map.is_map());
        // Another object's, another type's, and cut short.
        assert_eq!(create(&bytes, ID + 1), None);
        assert_eq!(create(&message(0, &bytes[5..]), ID), None);
        for cut in 0..bytes.len() - 17 {
            assert_eq!(create(&bytes[..cut], ID), None, "cut at {cut}");
        }
        // Counts no message holds.
        let mut many = bytes.clone();
        many[53..57].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(create(&many, ID), None);
    }

    #[test]
    fn roles_follow_the_first_class_and_the_slots() {
        let known = |kind: Hash, classes: &[Hash], slots: &[(Hash, u64)]| {
            let e = create(&create_bytes(kind, classes, 1, slots), ID).unwrap();
            Known::of(&e, &[])
        };
        let k = known(CREATE, &[PLACED, DAMAGE], &[]);
        assert_eq!(k.role, Role::Followed);
        assert_eq!(k.components(), [Component::Placed, Component::Damage]);
        // No class at all: an area entity.
        assert_eq!(known(CREATE, &[], &[]).role, Role::Followed);
        // A class this module does not know is followed for its transform.
        let k = known(CREATE, &[[1, 2, 3, 4], OWNER], &[]);
        assert_eq!(k.role, Role::Followed);
        assert_eq!(k.components(), [Component::Unknown, Component::Owner]);
        // Guns and attachments are not, bodies get a track.
        assert_eq!(known(CREATE, &[CARRIED, STATE], &[]).role, Role::Skipped);
        assert_eq!(known(CREATE, &[OWNER, CARRIED], &[]).role, Role::Skipped);
        let body = known(CREATE, &[CARRIED, STATE], &[(PRIMARY_WEAPON, 0)]);
        assert_eq!(body.role, Role::Body);
        let e = create(&create_bytes(CREATE, &[CARRIED], 1, &[]), ID).unwrap();
        assert_eq!(Known::of(&e, &[ID]).role, Role::Body);
        // A map object is followed whatever its class.
        assert_eq!(known(MAP_CREATE, &[CARRIED], &[]).role, Role::Followed);
        // The mask has bits for seven classes.
        let k = known(CREATE, &[OWNER; 9], &[]);
        assert_eq!(k.components().len(), MASKED);
    }

    #[test]
    fn a_rotation_turns_a_point_of_an_object() {
        // A quarter turn about z takes x to y.
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let p = World::rotate([0.0, 0.0, h, h], [1.0, 0.0, 2.0]);
        let want = [0.0, 1.0, 2.0];
        assert!(
            p.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-5),
            "{p:?}"
        );
        let same = World::rotate([0.0, 0.0, 0.0, 1.0], [4.0, 5.0, 6.0]);
        assert_eq!(same, [4.0, 5.0, 6.0]);
    }

    #[test]
    fn the_changes_up_to_a_frame_include_that_frame() {
        let at = |frame| Change {
            frame,
            ..Change::default()
        };
        let e = Entity {
            changes: vec![at(None), at(Some(10)), at(Some(20)), at(Some(30))],
            ..Entity::default()
        };
        assert_eq!(e.until(None).len(), 1);
        assert_eq!(e.until(Some(19)).len(), 2);
        assert_eq!(e.until(Some(20)).len(), 3);
        assert_eq!(e.until(Some(99)).len(), 4);
        assert!(Entity::default().until(Some(5)).is_empty());
    }

    /// A track of positions a tenth of a second apart, walking along x at
    /// 3 m/s, that is `jump` metres further on from frame 10.
    fn walk(y: f32, jump: f32) -> Vec<(u32, [f32; 3])> {
        (0..20u32)
            .map(|f| {
                let x = 0.3 * f as f32 + if f >= 10 { jump } else { 0.0 };
                (f, [x, y, 0.0])
            })
            .collect()
    }

    #[test]
    fn two_walkers_jumping_at_once_are_a_skip() {
        let times: Vec<f64> = (0..20).map(|f| f64::from(f) * 0.1).collect();
        let mut world = World::default();
        world.bodies.insert(1, walk(0.0, 3.0));
        world.bodies.insert(2, walk(5.0, 2.7));
        // Someone standing still who is moved is no part of it.
        let moved = (0..20u32).map(|f| (f, [if f >= 10 { 40.0 } else { 0.0 }, 9.0, 0.0]));
        world.bodies.insert(3, moved.collect());
        let skips = world.skips(&times);
        let [skip] = skips.as_slice() else {
            panic!("{skips:?}");
        };
        assert_eq!((skip.at, skip.until, skip.bodies), (0.9, 1.0, 2));
        // 3.3 m and 3.0 m in a step, at 3 m/s: 1.1 s and 1.0 s of walking
        // in a tenth of a second. The median of two is the later one.
        assert_eq!(skip.seconds, 1.0);

        // One walker alone was moved by something: a fall, a dash.
        let mut alone = World::default();
        alone.bodies.insert(1, walk(0.0, 3.0));
        alone.bodies.insert(2, walk(5.0, 0.0));
        assert!(alone.skips(&times).is_empty());
        // Nobody jumps: no skip.
        assert!(World::default().skips(&times).is_empty());
        // Frames the recording has no time for are passed over.
        assert!(world.skips(&times[..5]).is_empty());
    }

    #[test]
    fn a_body_is_where_its_track_last_put_it() {
        let mut world = World::default();
        world
            .bodies
            .insert(7, vec![(0, [0.0; 3]), (4, [1.0; 3]), (8, [2.0; 3])]);
        world.players.insert(7, 2);
        world.players.insert(9, 2);
        world.order = vec![7, 8, 9];
        assert_eq!(world.body_at(7, 0), Some([0.0; 3]));
        assert_eq!(world.body_at(7, 7), Some([1.0; 3]));
        assert_eq!(world.body_at(7, 100), Some([2.0; 3]));
        assert_eq!(world.body_at(8, 5), None);
        assert_eq!((world.player_of(7), world.player_of(8)), (Some(2), None));
        // The body created last is the player's.
        assert_eq!((world.body_of(2), world.body_of(3)), (Some(9), None));
    }

    /// The messages of one entity laid out as data: each in a block of its
    /// own, block `i` being frame `i`, the first the snapshot.
    fn followed(payloads: &[Vec<u8>], components: &[Component]) -> Followed {
        let mut data = vec![0xEE; 3];
        let mut messages = Vec::new();
        let mut blocks = Vec::new();
        for (i, p) in payloads.iter().enumerate() {
            messages.push(Message {
                entity: 0,
                block: i as u32,
                start: data.len(),
                len: p.len() as u32,
            });
            blocks.push((0, 0, (i as u32).checked_sub(1)));
            data.extend(p);
        }
        let mut known = Known {
            role: Role::Followed,
            components: [Component::Unknown; MASKED],
            count: components.len(),
        };
        known.components[..components.len()].copy_from_slice(components);
        follow(&data, &blocks, &messages, ID, &known, &[PLAYER])
    }

    #[test]
    fn a_state_that_repeats_is_left_out() {
        let hash = [0xAD, 0x55, 0x0A, 0x19];
        let closed = message(0x40, &state_bytes(0x04, hash, &[1, 2, 3, 4]));
        let opening = message(0x40, &state_bytes(0x04, hash, &[5, 6, 7, 8]));
        // The same state again, with a flag.
        let mut flagged = vec![0x08, 0x00, 0x40];
        flagged.extend(state_bytes(0x04, hash, &[5, 6, 7, 8]));
        let flagged = message(0xC0, &flagged);
        let payloads = [
            closed.clone(),
            closed.clone(),
            closed.clone(),
            opening.clone(),
            opening,
            flagged,
            closed,
        ];
        let f = followed(&payloads, &[Component::State]);
        assert_eq!((f.updates, f.unparsed, f.unread), (7, 0, 0));
        let seen: Vec<_> = (f.changes.iter())
            .map(|c| {
                let blob = c.state.as_ref().and_then(|s| s.blob.clone());
                (c.frame, c.flags, blob.map(|b| b.1))
            })
            .collect();
        assert_eq!(
            seen,
            vec![
                (None, None, Some(vec![1, 2, 3, 4])),
                (Some(2), None, Some(vec![5, 6, 7, 8])),
                (Some(4), Some(0x4000), None),
                (Some(5), None, Some(vec![1, 2, 3, 4])),
            ]
        );
    }

    #[test]
    fn a_device_that_repeats_is_left_out() {
        let fine = device_bytes(false, Some((false, false, 0)), 0);
        let broken = device_bytes(false, Some((true, false, 0)), 0);
        let mut moved = vec![0x01];
        floats(&mut moved, &[1.0, 2.0, 3.0]);
        moved.extend([0; 4]);
        moved.extend(&fine);
        let payloads = [
            message(0x40, &fine),
            message(0x40, &fine),
            message(0xC0, &moved),
            message(0x40, &broken),
            message(0x40, &broken),
        ];
        let f = followed(&payloads, &[Component::Device]);
        assert_eq!((f.updates, f.unparsed), (5, 0));
        let seen: Vec<_> = (f.changes.iter())
            .map(|c| {
                (
                    c.frame,
                    c.position.is_some(),
                    c.device.and_then(|d| d.destroyed),
                )
            })
            .collect();
        assert_eq!(
            seen,
            vec![
                (None, false, Some(false)),
                (Some(1), true, None),
                (Some(2), false, Some(true)),
            ]
        );
    }

    #[test]
    fn an_entity_is_followed_to_its_delete() {
        let mut unknown = vec![0x04, 1];
        unknown.extend([0xAB; 6]);
        let unknown = message(0xC0, &unknown);
        let payloads = [
            message(0x80, &[0x04, 0]),
            // Does not parse: counted, and the walk goes on.
            message(0x80, &[0x04]),
            unknown.clone(),
            DELETE.to_vec(),
        ];
        let f = followed(&payloads, &[Component::Unknown]);
        assert_eq!((f.updates, f.unparsed, f.unread), (3, 1, 1));
        assert_eq!(f.deleted, Some(2));
        assert_eq!(f.changes.len(), 2);
        assert_eq!((f.changes[0].frame, f.changes[0].live), (None, Some(0)));
        assert_eq!((f.changes[1].frame, f.changes[1].live), (Some(1), Some(1)));
        // The unread bytes are where they are in the data: after the two
        // messages before, the mask, the sub byte and the live byte.
        let start = 3 + 7 + 6 + 7;
        assert_eq!(f.changes[1].rest, Some((start, start + 6)));
        // A message the data does not hold is passed over.
        let gone = Message {
            entity: 0,
            block: 0,
            start: 1000,
            len: 8,
        };
        let known = Known {
            role: Role::Followed,
            components: [Component::Unknown; MASKED],
            count: 0,
        };
        let f = follow(&[0; 10], &[], &[gone], ID, &known, &[]);
        assert_eq!((f.updates, f.changes.len()), (0, 0));
    }
}
