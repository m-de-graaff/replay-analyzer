//! Gadget objects (Y11S3): what was placed on a surface or thrown, by
//! whom, where, when, and how the game took it away again; and the map's
//! own cameras. Read from the [`World`], not from the stream.
//!
//! # Placed gadgets
//!
//! A placed gadget is an entity with the class `4c60869a` (see
//! [`crate::world`] for the components). Two kinds of entity have that
//! class and are no gadget: the defuser (class `d0f65929`), and the panels
//! of barricades and reinforcements (classes `4c60869a 6ea51c35` and the
//! slots `b4d93e43` and `2e4bce49`; a Black Mirror has the same classes
//! and other slots).
//!
//! The game creates most of them long before they are used, at
//! [`POOL`]. The life of one, as its updates tell it:
//!
//! ```text
//! a position that is not the pool   the placement starts; the same message
//!                                   or one before names who places it
//!                                   (placed component, `02 <playerid>`),
//!                                   sometimes many seconds before: an
//!                                   attempt called off before the object
//!                                   left the pool
//! the pool again, before `live` 1   called off. The next attempt does not
//!                                   repeat the owner
//! `live` 1                          deployed. Its position is kept from
//!                                   here on: later ones are the object
//!                                   being pushed or carried along
//! `live` 0                          out of use
//! the pool again, or `637385fe`     gone
//! ```
//!
//! The component's host is the object it is fixed to: a wall, a floor, a
//! reinforcement. The full state an object is created with (`sub` `1f`)
//! is no event: `live` 0 in it does not end anything.
//!
//! # Thrown gadgets
//!
//! Everything a player can let go of has the owner component `8490f616`
//! (see [`crate::throws`]). The entities with the classes `[8490f616]`,
//! `[513b13b2, 8490f616]` or `[587f5a72, 513b13b2, 8490f616]` are followed
//! here: grenades, mines, cameras, what launchers fire and what those
//! leave behind. Drones have more classes and stay with `throws`. A
//! release (the flag going to 1) starts a gadget, where the object was at
//! that time; where the first run of positions without a pause of
//! [`PAUSE`] ends is where it came to rest. The flag back at 0, `live` 0
//! and the delete end it. An entity released again is another gadget.
//!
//! # Whose it is
//!
//! The `playerid` of the placed or owner component. Three objects are left
//! behind by a thrown one and name nobody: the expanded Kiba Barrier, the
//! posts of the R.O.U. Projector System and the deployed D.O.M. panel.
//! They take the owner of the nearest thrown object of their parent asset
//! (see [`crate::types::gadget_tables`]) within [`PARENT_RANGE`] at that
//! moment, and say so with `usernameSource`. The Pantheon shell a round
//! starts with can name nobody either, and stays without an owner.
//!
//! # What it is
//!
//! `typeIndex` is the type index of the component. The name is that of
//! the owner's loadout slot whose asset is the object's (`nameSource`
//! `slot`); else the asset's row in the table (`table`); else the type
//! index's (`typeIndex`). A name no loadout slot ever gave carries
//! `inferred`.
//!
//! # State
//!
//! The state component's blob starts with a hash that names the state
//! machine, then a `u32` size and the state: for some machines the CRC-32
//! of the state's name ([`STATE_NAMES`]), for others a number or a
//! progress. `states` lists each change of those four bytes, at most
//! [`MAX_STATES`]. The world keeps a state only when it changes, so the
//! state a gadget starts with is listed at its first update after the
//! start.
//!
//! # How it ended
//!
//! `end.signals` is what the entity showed, as read: `returned` (the owner
//! flag back to 0), `notLive` (`live` 0), `broken` (an `fe` entry in its
//! damage list), `destroyedFlag` (the device component of a camera),
//! `inert` (flags `4000` on barbed wire and a Welcome Mat, which stay in
//! the world), `deleted`, `pooled`. `end.how` is what little they say by
//! themselves:
//!
//! - `presentAtEnd`: no signal.
//! - `destroyed`: `broken`, `destroyedFlag`, or barbed wire gone `inert`.
//! - `wentOff`: a Welcome Mat gone `inert`.
//! - `pickedUp`, with `source` `inferred`: a placed gadget deleted 0.7 to
//!   1.1 s after `live` 0, the time it takes to pick one up.
//! - `removed`: anything else. The same signals mean a detonation for one
//!   type and a destruction for another, and the time between `live` 0
//!   and the delete is a property of the type, not of the cause; who
//!   destroyed what is read elsewhere, from the score.
//!
//! `end` then takes what [`crate::gadget_events`] works out for the
//! entity (see [`crate::join`]): `cause` with `causeSource`, and for a
//! destruction `by` with `bySource` and `means` with `meansSource`. A
//! cause turns `removed` and a guessed `pickedUp` into `destroyed`,
//! `wentOff` or `pickedUp`, with `source` `inferred` unless the cause was
//! read from the signals. `statuses` and `triggers` are from there too.
//!
//! # Map cameras
//!
//! The map's default cameras are map objects with the one class
//! `587f5a72`. A destroyed one writes the flags `4000` alone.

use serde::{Serialize, Serializer};

use crate::entities::Hash;
use crate::gadget_events::{Shown, Trigger, Verdict};
use crate::loadout::{Input, When};
use crate::throws::Slot;
use crate::types::gadget_tables::{gadget_asset, gadget_type};
use crate::types::item_name;
use crate::world::{Change, DAMAGE, DEVICE, Entity, OWNER, PLACED, POOL, STATE, World, seconds};

/// Class only the defuser has.
const DEFUSER: Hash = [0xD0, 0xF6, 0x59, 0x29];
/// The slots of a barricade or reinforcement panel.
const PANEL_SLOTS: [Hash; 2] = [[0xB4, 0xD9, 0x3E, 0x43], [0x2E, 0x4B, 0xCE, 0x49]];
/// Body slots that hold what a player places or throws: `PrimaryGadget`,
/// `SecondaryGadget` and `TertiaryGadget`.
const CARRIED: [(Hash, Slot); 3] = [
    ([0x08, 0x2C, 0xA3, 0x1D], Slot::Ability),
    ([0xD8, 0x55, 0xB4, 0xAF], Slot::Gadget),
    ([0x41, 0x20, 0x14, 0x8B], Slot::Gadget),
];

/// Positions come every 0.03 s while an object moves; a pause longer than
/// this ends its flight (seconds), or this many frames when the frames
/// have no time.
const PAUSE: f64 = 0.25;
const PAUSE_FRAMES: u32 = 8;
/// A rotation written this many frames after the last position of the
/// flight still says how the object lies.
const SETTLE_FRAMES: u32 = 2;
/// An object that names nobody is at most this far from the thrown object
/// that left it behind (metres), which is looked for up to this many
/// frames after it appeared.
const PARENT_RANGE: f64 = 1.0;
const PARENT_FRAMES: u32 = 3;
/// A gadget lists at most this many states.
const MAX_STATES: usize = 64;
/// A gadget picked up is deleted this long after its `live` 0 (seconds).
const PICK_UP: (f64, f64) = (0.7, 1.1);
/// Flags of an object that is still in the world and does nothing.
const INERT: u16 = 0x4000;
/// Type indices of the placed gadgets that go inert: barbed wire and the
/// Welcome Mat.
const BARBED_WIRE: u16 = 206;
const WELCOME_MAT: u16 = 215;

/// States a state machine names: the CRC-32 of the name, in stream order.
const STATE_NAMES: [([u8; 4], &str); 8] = [
    ([0x1C, 0x43, 0x24, 0x34], "Invalid"),
    ([0xD2, 0x48, 0x6B, 0x03], "Closed"),
    ([0x5F, 0xED, 0x88, 0xC1], "Opening"),
    ([0x6D, 0x22, 0xFA, 0xAA], "Opened"),
    ([0x28, 0xD2, 0xFF, 0x9A], "Closing"),
    ([0x2B, 0x1A, 0x16, 0x7C], "Idle"),
    ([0x1C, 0x6A, 0x54, 0x33], "InAir"),
    ([0x89, 0xF7, 0x87, 0x20], "Landing"),
];

/// The frame of a message; `None` is the opening snapshot.
type Frame = Option<u32>;

/// How a gadget got where it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// Put on a surface.
    #[default]
    Placed,
    /// Thrown, or fired from a launcher.
    Thrown,
}

/// Where a gadget's name is from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NameSource {
    /// The owner's loadout slot whose asset is the object's.
    Slot,
    /// The asset's row in the table.
    Table,
    /// The type index's row in the table.
    TypeIndex,
}

/// Something an entity showed as it left play.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Signal {
    /// The owner flag went back to 0.
    Returned,
    /// `live` went to 0.
    NotLive,
    /// Its damage list got an `fe` entry.
    Broken,
    /// The device component says destroyed.
    DestroyedFlag,
    /// Flags `4000` on an object that stays in the world.
    Inert,
    /// Its `637385fe` message.
    Deleted,
    /// It went back to the pool.
    Pooled,
}

/// What the signals say by themselves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum How {
    #[default]
    PresentAtEnd,
    Destroyed,
    WentOff,
    PickedUp,
    /// Out of play, and the signals do not say why.
    Removed,
}

/// Whether a value was read or worked out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    #[default]
    Read,
    Inferred,
}

/// How a gadget ended. The time is that of the first signal.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct End {
    pub how: How,
    pub source: Source,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub signals: Vec<Signal>,
    /// Seconds from the first signal to the entity being deleted or going
    /// back to the pool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gone_after: Option<f64>,
    /// Why, by whom and with what, from the removal
    /// [`crate::gadget_events`] found for the entity (see [`crate::join`]).
    #[serde(flatten)]
    pub verdict: Option<Verdict>,
    #[serde(flatten)]
    pub when: Option<When>,
}

/// One state of a gadget's state machine.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateChange {
    /// The hash that names the state machine.
    pub machine: String,
    /// The state's name where it is one of the known ones, else its four
    /// bytes.
    pub state: String,
    #[serde(flatten)]
    pub when: When,
}

/// The frames of a gadget's life, for the decoders that join on them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frames {
    /// The placement start or the release; `None` in the snapshot.
    pub start: Frame,
    pub deployed: Option<Frame>,
    pub rest: Option<Frame>,
    pub returned: Option<Frame>,
    pub not_live: Option<Frame>,
    pub broken: Option<Frame>,
    pub destroyed_flag: Option<Frame>,
    pub inert: Option<Frame>,
    /// Deleted, or back in the pool.
    pub gone: Option<Frame>,
    /// Every write of the flags, in order.
    pub flags: Vec<(Frame, u16)>,
}

/// One gadget object.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gadget {
    /// The entity of the movement stream. One the game uses again is
    /// another gadget with the same entity.
    #[serde(serialize_with = "hex")]
    pub entity: u64,
    pub kind: Kind,
    /// The type index its component states.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_index: Option<u16>,
    /// The object's asset id.
    pub asset: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_source: Option<NameSource>,
    /// The name is one this crate's table gives an object no loadout slot
    /// holds, or the name of its type index.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub inferred: bool,
    /// The loadout slot it is from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<Slot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// `nearest` when the owner is that of the thrown object that left
    /// this one behind (`parent`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username_source: Option<&'static str>,
    /// Where it is: `[x, y, z]` in metres, z up. A placed gadget's place
    /// when it was deployed, a thrown one's where it came to rest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<[f64; 3]>,
    /// Its rotation quaternion `[x, y, z, w]` there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<[f64; 4]>,
    /// Where it left the hand, or where its placement started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<[f64; 3]>,
    /// The object it is fixed to: a map object or an entity.
    #[serde(serialize_with = "hex_opt", skip_serializing_if = "Option::is_none")]
    pub host: Option<u64>,
    /// What the host is when it is a panel: `reinforcedWall`,
    /// `reinforcedHatch` or `barricade`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_kind: Option<&'static str>,
    /// The entity of the thrown gadget that left it behind.
    #[serde(serialize_with = "hex_opt", skip_serializing_if = "Option::is_none")]
    pub parent: Option<u64>,
    /// Metres between the owner's body and `origin` at that time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_distance: Option<f64>,
    /// When its placement started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placing: Option<When>,
    /// When it left the hand or the launcher.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub released: Option<When>,
    /// When it went live. Absent for one that never did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployed: Option<When>,
    /// When a thrown gadget came to rest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rested: Option<When>,
    /// Placements of this object called off before this one.
    pub cancels: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<StateChange>,
    /// What was put on it: disabled, frozen, hacked, caught, captured
    /// (see [`crate::gadget_events`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<Shown>,
    /// Each time it went off, for a trap.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<Trigger>,
    pub end: End,
    #[serde(skip)]
    pub frames: Frames,
}

/// A default camera of the map.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapCamera {
    /// The map object: the same in every round on the map.
    #[serde(serialize_with = "hex")]
    pub object: u64,
    pub position: [f64; 3],
    pub rotation: [f64; 4],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroyed: Option<When>,
    #[serde(skip)]
    pub destroyed_frame: Option<Frame>,
}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    /// In the order they started.
    pub gadgets: Vec<Gadget>,
    pub cameras: Vec<MapCamera>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
}

fn hex<S: Serializer>(id: &u64, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format!("{id:x}"))
}

fn hex_opt<S: Serializer>(id: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
    match id {
        Some(id) => hex(id, s),
        None => s.serialize_none(),
    }
}

fn round(v: f64, digits: i32) -> f64 {
    let scale = 10f64.powi(digits);
    (v * scale).round() / scale
}

fn place(p: [f32; 3]) -> [f64; 3] {
    p.map(|v| round(f64::from(v), 3))
}

fn turn(q: [f32; 4]) -> [f64; 4] {
    q.map(|v| round(f64::from(v), 4))
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f64 {
    let squares = a.iter().zip(b).map(|(x, y)| f64::from(x - y).powi(2));
    squares.sum::<f64>().sqrt()
}

/// What an entity is to this decoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Placed,
    Thrown,
}

/// The role of an entity by its create message.
fn role(entity: &Entity) -> Option<Role> {
    if entity.is_map() {
        return None;
    }
    let classes = entity.classes.as_slice();
    if entity.has(PLACED) {
        let panel = classes == [PLACED, DAMAGE]
            && entity.slots.len() == PANEL_SLOTS.len()
            && PANEL_SLOTS.iter().all(|s| entity.slot(*s).is_some());
        return (!panel && !entity.has(DEFUSER)).then_some(Role::Placed);
    }
    let thrown =
        classes == [OWNER] || classes == [STATE, OWNER] || classes == [DEVICE, STATE, OWNER];
    thrown.then_some(Role::Thrown)
}

/// How an entity went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gone {
    Deleted,
    Pooled,
}

/// One gadget while its entity is read.
#[derive(Clone, Debug, Default)]
struct Object {
    kind: Kind,
    entity: u64,
    asset: u64,
    type_index: Option<u16>,
    owner: Option<u64>,
    host: Option<u64>,
    parent: Option<u64>,
    start: Frame,
    origin: Option<[f32; 3]>,
    position: Option<[f32; 3]>,
    rotation: Option<[f32; 4]>,
    deployed: Option<Frame>,
    rest: Option<Frame>,
    cancels: u32,
    flags: Vec<(Frame, u16)>,
    /// `(frame, machine, state)`.
    states: Vec<(Frame, Hash, Vec<u8>)>,
    /// The blob last seen, and whether the state it started with is still
    /// to be listed.
    blob: Option<Vec<u8>>,
    initial: bool,
    returned: Option<Frame>,
    not_live: Option<Frame>,
    broken: Option<Frame>,
    destroyed_flag: Option<Frame>,
    gone: Option<(Frame, Gone)>,
    /// Positions since the release, and rotations.
    track: Vec<(Frame, [f32; 3])>,
    turns: Vec<(Frame, [f32; 4])>,
    /// A placement called off: not a gadget.
    dropped: bool,
}

/// What is known of an entity across its gadgets.
#[derive(Clone, Debug, Default)]
struct Tracked {
    type_index: Option<u16>,
    owner: Option<u64>,
    cancels: u32,
    position: Option<[f32; 3]>,
    rotation: Option<[f32; 4]>,
    /// The state blob last written: machine and bytes.
    blob: Option<(Hash, Vec<u8>)>,
    /// The entity's gadget in the making, as an index into the objects.
    open: Option<usize>,
}

/// The four bytes of a blob that are the state: those after its `u32`
/// size, or all of a blob too short for that.
fn state_of(blob: &[u8]) -> &[u8] {
    blob.get(4..8).unwrap_or(blob)
}

impl Object {
    fn new(kind: Kind, entity: &Entity, t: &mut Tracked, frame: Frame) -> Object {
        Object {
            kind,
            entity: entity.id,
            asset: entity.asset,
            type_index: t.type_index,
            owner: t.owner,
            start: frame,
            cancels: std::mem::take(&mut t.cancels),
            initial: true,
            ..Object::default()
        }
    }

    /// Lists `blob` when its state is another than the last listed.
    fn state(&mut self, frame: Frame, machine: Hash, blob: &[u8]) {
        self.initial = false;
        if self.blob.as_deref() == Some(blob) {
            return;
        }
        let state = state_of(blob);
        let last = self.states.last().map(|s| s.2.as_slice());
        if self.states.len() < MAX_STATES && last != Some(state) {
            self.states.push((frame, machine, state.to_vec()));
        }
        self.blob = Some(blob.to_vec());
    }

    /// What every update of a gadget in the making can say, whatever its
    /// kind: flags, a damage list's `fe`, the device's flag, a state.
    fn common(&mut self, c: &Change, t: &Tracked) {
        if let Some(flags) = c.flags {
            self.flags.push((c.frame, flags));
        }
        if self.deployed.is_some() && !c.full {
            if c.destroyed && self.broken.is_none() {
                self.broken = Some(c.frame);
            }
            let destroyed = c.device.and_then(|d| d.destroyed) == Some(true);
            if destroyed && self.destroyed_flag.is_none() {
                self.destroyed_flag = Some(c.frame);
            }
        }
        let written = c.state.as_ref().and_then(|s| s.blob.as_ref());
        match (written, &t.blob) {
            (Some((machine, blob)), _) => self.state(c.frame, *machine, blob),
            // The state it started with, written before the start.
            (None, Some((machine, blob))) if self.initial => self.state(c.frame, *machine, blob),
            _ => {}
        }
    }
}

/// Reads the gadgets of one entity into `objects`.
fn follow(entity: &Entity, role: Role, objects: &mut Vec<Object>) {
    let mut t = Tracked::default();
    for c in &entity.changes {
        match role {
            Role::Placed => placed(entity, c, &mut t, objects),
            Role::Thrown => thrown(entity, c, &mut t, objects),
        }
        if let Some(blob) = c.state.as_ref().and_then(|s| s.blob.as_ref()) {
            t.blob = Some(blob.clone());
        }
    }
    if let (Some(frame), Some(o)) = (entity.deleted, t.open.and_then(|i| objects.get_mut(i))) {
        o.gone = Some((Some(frame), Gone::Deleted));
    }
}

/// One update of a placed entity.
fn placed(entity: &Entity, c: &Change, t: &mut Tracked, objects: &mut Vec<Object>) {
    if let Some(p) = c.placed {
        t.type_index = p.type_index.map(|i| i.0).or(t.type_index);
        t.owner = p.owner.or(t.owner);
    }
    let start = |t: &mut Tracked, objects: &mut Vec<Object>| {
        objects.push(Object::new(Kind::Placed, entity, t, c.frame));
        t.open = Some(objects.len() - 1);
    };
    if let Some(position) = c.position {
        if position == POOL {
            // The state a pooled object is created with says nothing.
            if c.full {
                return;
            }
            if let Some(o) = t.open.take().and_then(|i| objects.get_mut(i)) {
                if o.deployed.is_none() {
                    o.dropped = true;
                    t.cancels = o.cancels + 1;
                } else {
                    o.gone = Some((c.frame, Gone::Pooled));
                }
            }
            return;
        }
        if t.open.is_none() {
            start(t, objects);
        }
    }
    if t.open.is_none() {
        let named = c
            .placed
            .is_some_and(|p| p.owner.is_some() || p.host.is_some());
        if !named && c.live.is_none_or(|l| l == 0) {
            return;
        }
        start(t, objects);
    }
    let Some(o) = t.open.and_then(|i| objects.get_mut(i)) else {
        return;
    };
    if let Some(position) = c.position {
        if o.deployed.is_none() || o.position.is_none() {
            o.position = Some(position);
        }
        if o.origin.is_none() {
            // The placement starts with its first place. The owner and
            // the host can be written long before: an attempt called off
            // before the object left the pool.
            (o.origin, o.start) = (Some(position), c.frame);
        }
    }
    if o.deployed.is_none() {
        o.rotation = c.rotation.or(o.rotation);
    }
    if let Some(p) = c.placed {
        o.owner = p.owner.or(o.owner);
        o.host = p.host.or(o.host);
    }
    match c.live {
        Some(1) if o.deployed.is_none() => o.deployed = Some(c.frame),
        Some(0) if o.deployed.is_some() && o.not_live.is_none() && !c.full => {
            o.not_live = Some(c.frame);
        }
        _ => {}
    }
    o.common(c, t);
}

/// One update of a thrown entity.
fn thrown(entity: &Entity, c: &Change, t: &mut Tracked, objects: &mut Vec<Object>) {
    if let Some(owner) = c.owner {
        t.type_index = owner.type_index.or(t.type_index);
        t.owner = owner.player.or(t.owner);
    }
    t.position = c.position.or(t.position);
    if let Some(rotation) = c.rotation {
        t.rotation = Some(rotation);
        if let Some(o) = t.open.and_then(|i| objects.get_mut(i)) {
            o.turns.push((c.frame, rotation));
        }
    }
    let released = c.owner.and_then(|o| o.released);
    if released == Some(1) {
        let mut o = Object::new(Kind::Thrown, entity, t, c.frame);
        o.origin = t.position;
        o.position = t.position;
        o.rotation = t.rotation;
        o.track.extend(t.position.map(|p| (c.frame, p)));
        if c.live == Some(1) {
            o.deployed = Some(c.frame);
        }
        objects.push(o);
        t.open = Some(objects.len() - 1);
        return;
    }
    let Some(o) = t.open.and_then(|i| objects.get_mut(i)) else {
        return;
    };
    if let Some(position) = c.position {
        if position == POOL {
            o.gone = Some((c.frame, Gone::Pooled));
            t.open = None;
            return;
        }
        o.track.push((c.frame, position));
    }
    match c.live {
        Some(1) if o.deployed.is_none() => o.deployed = Some(c.frame),
        Some(0) if o.not_live.is_none() && !c.full => o.not_live = Some(c.frame),
        _ => {}
    }
    if released == Some(0) && o.returned.is_none() {
        o.returned = Some(c.frame);
    }
    o.common(c, t);
}

/// Where a thrown object's flight ends: the index into its track of the
/// last position of the first run without a pause. `time` gives a frame's
/// seconds.
fn rest(track: &[(Frame, [f32; 3])], time: impl Fn(Frame) -> Option<f64>) -> Option<usize> {
    let paused = |w: &[(Frame, [f32; 3])]| match w {
        [a, b] => match (time(a.0), time(b.0)) {
            (Some(a), Some(b)) => b - a > PAUSE,
            _ => b.0.unwrap_or(0).saturating_sub(a.0.unwrap_or(0)) > PAUSE_FRAMES,
        },
        _ => false,
    };
    let last = track.len().checked_sub(1)?;
    Some(track.windows(2).position(paused).unwrap_or(last))
}

/// What the signals of `o` say by themselves. `gone_after` is the time
/// from the first signal to the entity going.
fn how(o: &Object, inert: bool, gone_after: Option<f64>) -> (How, Source) {
    let signalled = o.returned.is_some() || o.not_live.is_some() || o.gone.is_some();
    let wire = o.kind == Kind::Placed && o.type_index == Some(BARBED_WIRE);
    if o.broken.is_some() || o.destroyed_flag.is_some() || (inert && wire) {
        return (How::Destroyed, Source::Read);
    }
    if inert {
        return (How::WentOff, Source::Read);
    }
    if !signalled {
        return (How::PresentAtEnd, Source::Read);
    }
    let picked = o.kind == Kind::Placed
        && o.not_live.is_some()
        && matches!(o.gone, Some((_, Gone::Deleted)))
        && gone_after.is_some_and(|t| (PICK_UP.0..=PICK_UP.1).contains(&round(t, 1)));
    if picked {
        (How::PickedUp, Source::Inferred)
    } else {
        (How::Removed, Source::Read)
    }
}

/// Reads the gadgets and the map's cameras from the world. `items` is
/// each player's ability and gadget item as the HUD has them
/// ([`crate::loadout::hud_items`]).
pub(crate) fn decode(input: &Input, world: &World, items: &[[Option<u64>; 2]]) -> Decoded {
    let mut out = Decoded::default();
    let time = |frame: Frame| seconds(input, frame);
    let when = |frame: Frame| world.when(input.clock, frame);

    let mut objects: Vec<Object> = Vec::new();
    for entity in world.iter() {
        if let Some(role) = role(entity) {
            follow(entity, role, &mut objects);
        } else if entity.is_map() && entity.classes == [DEVICE] {
            // Flags `4000` written alone: the camera is destroyed.
            let alone = |c: &&Change| {
                c.flags == Some(INERT)
                    && c.position.is_none()
                    && c.rotation.is_none()
                    && c.live.is_none()
            };
            let destroyed = entity.changes.iter().rfind(alone).map(|c| c.frame);
            out.cameras.push(MapCamera {
                object: entity.id,
                position: place(entity.position),
                rotation: turn(entity.rotation),
                destroyed: destroyed.map(when),
                destroyed_frame: destroyed,
            });
        }
    }
    objects.retain(|o| !o.dropped);

    // Where each thrown object came to rest, and how it lies there: the
    // last rotation of the flight.
    for o in objects.iter_mut().filter(|o| o.kind == Kind::Thrown) {
        let Some(&(frame, position)) = rest(&o.track, time).and_then(|i| o.track.get(i)) else {
            continue;
        };
        o.rest = Some(frame);
        o.position = Some(position);
        let settled = frame.map_or(0, |f| f.saturating_add(SETTLE_FRAMES));
        let lies = o.turns.iter().rfind(|q| q.0.unwrap_or(0) <= settled);
        o.rotation = lies.map(|q| q.1).or(o.rotation);
    }

    // What names nobody takes the owner of the thrown object that left it
    // behind: the nearest of its parent asset at that moment.
    let mut found: Vec<(usize, u64, Option<u64>)> = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        let (Kind::Placed, None, Some(origin), Some(start)) = (o.kind, o.owner, o.origin, o.start)
        else {
            continue;
        };
        let want = gadget_asset(o.asset).and_then(|r| r.parent);
        let until = start.saturating_add(PARENT_FRAMES);
        let near = objects
            .iter()
            .filter(|p| p.kind == Kind::Thrown && p.start.is_some_and(|s| s <= start))
            .filter(|p| want.is_none_or(|a| a == p.asset))
            .filter_map(|p| {
                let at = p.track.iter().take_while(|s| s.0 <= Some(until)).last()?;
                Some((distance(at.1, origin), p))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((d, p)) = near
            && d <= PARENT_RANGE
        {
            found.push((i, p.entity, p.owner));
        }
    }
    for (i, parent, owner) in found {
        if let Some(o) = objects.get_mut(i) {
            o.parent = Some(parent);
            o.owner = owner;
        }
    }
    // A placement that went nowhere and names nobody is not a gadget.
    objects.retain(|o| {
        o.kind == Kind::Thrown || o.deployed.is_some() || o.gone.is_some() || o.owner.is_some()
    });
    objects.sort_by_key(|o| (o.start.map_or(-1, i64::from), o.entity));

    let (mut ownerless, mut unnamed) = (0usize, 0usize);
    for o in &objects {
        let index =
            (o.owner).and_then(|id| input.players.iter().position(|p| p.id == id && id != 0));
        let player = index.and_then(|i| input.players.get(i));
        ownerless += usize::from(player.is_none());
        // The body the player table names, else the last the world gave
        // the player.
        let body = player
            .and_then(|p| p.entities.as_ref()?.movement.map(u64::from))
            .or_else(|| world.body_of(index?));
        let holds = |slot: Hash| {
            o.asset != 0 && body.and_then(|b| world.get(b)?.slot(slot)) == Some(o.asset)
        };
        let slot = CARRIED.iter().rfind(|c| holds(c.0)).map(|c| c.1);
        let item = |slot: Slot| {
            let slots = items.get(index?)?;
            match slot {
                Slot::Ability => slots[0],
                Slot::Gadget => slots[1],
                Slot::Drone => None,
            }
        };
        let by_slot = slot.and_then(|s| Some((item_name(item(s)?)?, s)));
        let row = gadget_asset(o.asset);
        let by_type = (o.type_index).and_then(|i| gadget_type(o.kind == Kind::Placed, i));
        let (name, name_source, slot, inferred) = match (by_slot, row, by_type) {
            (Some((name, slot)), _, _) => (Some(name), Some(NameSource::Slot), Some(slot), false),
            (None, Some(r), _) => (Some(r.name), Some(NameSource::Table), r.slot, r.inferred),
            (None, None, Some(name)) => (Some(name), Some(NameSource::TypeIndex), None, true),
            (None, None, None) => (None, None, None, false),
        };
        unnamed += usize::from(name.is_none());

        let owner_distance = match (body, o.origin, o.start) {
            (Some(body), Some(origin), Some(frame)) => world
                .body_at(body, frame)
                .map(|at| round(distance(at, origin), 3)),
            _ => None,
        };
        let inert = (o.kind == Kind::Placed
            && matches!(o.type_index, Some(BARBED_WIRE | WELCOME_MAT)))
        .then(|| {
            let after = |f: &&(Frame, u16)| f.1 == INERT && o.deployed.is_some_and(|d| f.0 > d);
            o.flags.iter().find(after).map(|f| f.0)
        })
        .flatten();
        let gone = o.gone.map(|g| g.0);
        let marks = [
            (o.returned, Signal::Returned),
            (o.not_live, Signal::NotLive),
            (o.broken, Signal::Broken),
            (o.destroyed_flag, Signal::DestroyedFlag),
            (inert, Signal::Inert),
            (
                gone,
                match o.gone {
                    Some((_, Gone::Pooled)) => Signal::Pooled,
                    _ => Signal::Deleted,
                },
            ),
        ];
        let mut signals: Vec<(Frame, Signal)> = (marks.iter())
            .filter_map(|(frame, signal)| Some(((*frame)?, *signal)))
            .collect();
        signals.sort_by_key(|s| s.0);
        let first = signals.first().map(|s| s.0);
        let gone_after = match (first.and_then(time), gone.and_then(time)) {
            (Some(a), Some(b)) => Some(b - a),
            _ => None,
        };
        let (how, source) = how(o, inert.is_some(), gone_after);
        let start = Some(when(o.start));
        let states = (o.states.iter())
            .map(|(frame, machine, state)| {
                let known = STATE_NAMES.iter().find(|n| n.0 == state.as_slice());
                StateChange {
                    machine: machine.iter().map(|b| format!("{b:02x}")).collect(),
                    state: match known {
                        Some(n) => n.1.to_owned(),
                        None => state.iter().map(|b| format!("{b:02x}")).collect(),
                    },
                    when: when(*frame),
                }
            })
            .collect();
        out.gadgets.push(Gadget {
            entity: o.entity,
            kind: o.kind,
            type_index: o.type_index,
            asset: o.asset,
            name,
            name_source,
            inferred,
            slot,
            username: player.map(|p| p.username.clone()),
            username_source: (o.parent.is_some() && player.is_some()).then_some("nearest"),
            position: o.position.map(place),
            rotation: o.rotation.map(turn),
            origin: o.origin.map(place),
            host: o.host,
            host_kind: None,
            parent: o.parent,
            owner_distance,
            placing: start.clone().filter(|_| o.kind == Kind::Placed),
            released: start.filter(|_| o.kind == Kind::Thrown),
            deployed: o.deployed.map(when),
            rested: o.rest.map(when),
            cancels: o.cancels,
            states,
            statuses: Vec::new(),
            triggers: Vec::new(),
            end: End {
                how,
                source,
                signals: signals.iter().map(|s| s.1).collect(),
                gone_after: gone_after.map(|t| round(t, 3)),
                verdict: None,
                when: first.map(when),
            },
            frames: Frames {
                start: o.start,
                deployed: o.deployed,
                rest: o.rest,
                returned: o.returned,
                not_live: o.not_live,
                broken: o.broken,
                destroyed_flag: o.destroyed_flag,
                inert,
                gone,
                flags: o.flags.clone(),
            },
        });
    }

    for (count, what) in [
        (ownerless, "gadgets without a known owner"),
        (unnamed, "gadgets of an unknown type"),
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
    use crate::world::{Device, Owner, Placed, State};

    const PLAYER: u64 = 0x1122_3344_5566_7788;
    const SPOT: [f32; 3] = [10.0, 20.0, 1.5];

    fn entity(classes: &[Hash]) -> Entity {
        Entity {
            id: 0xF02B_8349,
            asset: 367839440370,
            classes: classes.to_vec(),
            ..Entity::default()
        }
    }

    fn at(frame: u32) -> Change {
        Change {
            frame: Some(frame),
            ..Change::default()
        }
    }

    fn moved(frame: u32, position: [f32; 3]) -> Change {
        Change {
            position: Some(position),
            ..at(frame)
        }
    }

    fn live(frame: u32, live: u8) -> Change {
        Change {
            live: Some(live),
            ..at(frame)
        }
    }

    fn owned(c: Change) -> Change {
        let placed = Placed {
            owner: Some(PLAYER),
            ..Placed::default()
        };
        Change {
            placed: Some(placed),
            ..c
        }
    }

    fn pooled() -> Change {
        let placed = Placed {
            type_index: Some((206, 0)),
            ..Placed::default()
        };
        Change {
            frame: None,
            full: true,
            position: Some(POOL),
            live: Some(0),
            placed: Some(placed),
            ..Change::default()
        }
    }

    fn read(mut entity: Entity, changes: Vec<Change>) -> Vec<Object> {
        entity.changes = changes;
        let mut objects = Vec::new();
        follow(&entity, role(&entity).unwrap(), &mut objects);
        objects.retain(|o| !o.dropped);
        objects
    }

    #[test]
    fn panels_the_defuser_drones_and_bodies_are_no_gadgets() {
        assert_eq!(role(&entity(&[PLACED])), Some(Role::Placed));
        assert_eq!(role(&entity(&[PLACED, DEFUSER])), None);
        let mut panel = entity(&[PLACED, DAMAGE]);
        // A Black Mirror has the classes of a panel and other slots.
        assert_eq!(role(&panel), Some(Role::Placed));
        panel.slots = vec![(PANEL_SLOTS[1], 0), (PANEL_SLOTS[0], 7)];
        assert_eq!(role(&panel), None);
        assert_eq!(role(&entity(&[STATE, OWNER])), Some(Role::Thrown));
        assert_eq!(role(&entity(&[DEVICE, STATE, OWNER])), Some(Role::Thrown));
        let drone = [DEVICE, [0x47, 0xE5, 0xF6, 0x00], STATE, OWNER];
        assert_eq!(role(&entity(&drone)), None);
        assert_eq!(role(&entity(&[])), None);
    }

    #[test]
    fn a_placement_called_off_and_tried_again_keeps_its_owner() {
        let objects = read(
            entity(&[PLACED]),
            vec![
                pooled(),
                owned(moved(100, SPOT)),
                moved(110, POOL),
                moved(200, [11.0, 20.0, 1.5]),
                live(230, 1),
            ],
        );
        let [o] = objects.as_slice() else {
            panic!("{objects:?}");
        };
        assert_eq!((o.owner, o.type_index), (Some(PLAYER), Some(206)));
        assert_eq!((o.start, o.deployed), (Some(200), Some(Some(230))));
        assert_eq!(o.cancels, 1);
        assert_eq!(o.position, Some([11.0, 20.0, 1.5]));
        assert_eq!(how(o, false, None), (How::PresentAtEnd, Source::Read));
    }

    #[test]
    fn a_deployed_gadget_keeps_the_place_it_was_deployed_at() {
        let mut deploy = live(130, 1);
        deploy.position = Some([10.0, 20.5, 1.5]);
        let objects = read(
            entity(&[PLACED]),
            vec![
                pooled(),
                owned(moved(100, SPOT)),
                deploy,
                moved(140, [10.0, 21.0, 1.5]),
            ],
        );
        let [o] = objects.as_slice() else {
            panic!("{objects:?}");
        };
        assert_eq!(o.origin, Some(SPOT));
        assert_eq!(o.position, Some([10.0, 20.5, 1.5]));
    }

    #[test]
    fn live_0_of_a_full_state_ends_nothing() {
        let full = Change {
            full: true,
            ..live(150, 0)
        };
        let objects = read(
            entity(&[PLACED]),
            vec![owned(moved(100, SPOT)), live(130, 1), full],
        );
        assert_eq!(objects[0].not_live, None);
    }

    #[test]
    fn a_gadget_deleted_a_second_after_live_0_was_picked_up() {
        let mut e = entity(&[PLACED]);
        e.deleted = Some(330);
        let objects = read(e, vec![owned(moved(100, SPOT)), live(130, 1), live(300, 0)]);
        let o = &objects[0];
        assert_eq!(o.not_live, Some(Some(300)));
        assert_eq!(o.gone, Some((Some(330), Gone::Deleted)));
        assert_eq!(how(o, false, Some(1.0)), (How::PickedUp, Source::Inferred));
        // Any other delay is the type's own, and says nothing.
        assert_eq!(how(o, false, Some(3.0)), (How::Removed, Source::Read));
        assert_eq!(how(o, false, Some(0.0)), (How::Removed, Source::Read));
    }

    #[test]
    fn a_gadget_back_in_the_pool_is_gone_and_the_entity_is_used_again() {
        let objects = read(
            entity(&[PLACED]),
            vec![
                owned(moved(100, SPOT)),
                live(130, 1),
                moved(300, POOL),
                moved(400, SPOT),
                live(430, 1),
            ],
        );
        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].gone, Some((Some(300), Gone::Pooled)));
        // The owner is written once.
        assert_eq!(objects[1].owner, Some(PLAYER));
        assert_eq!(objects[1].gone, None);
    }

    #[test]
    fn wire_going_inert_is_destroyed_and_a_mat_went_off() {
        let flags = Change {
            flags: Some(INERT),
            ..at(500)
        };
        let objects = read(
            entity(&[PLACED]),
            vec![pooled(), owned(moved(100, SPOT)), live(130, 1), flags],
        );
        let o = &objects[0];
        assert_eq!(o.flags, [(Some(500), INERT)]);
        assert_eq!(how(o, true, None), (How::Destroyed, Source::Read));
        let mat = Object {
            type_index: Some(WELCOME_MAT),
            ..o.clone()
        };
        assert_eq!(how(&mat, true, None), (How::WentOff, Source::Read));
    }

    #[test]
    fn a_broken_shield_and_a_camera_flagged_destroyed_are_destroyed() {
        let broken = Change {
            destroyed: true,
            ..at(300)
        };
        let objects = read(
            entity(&[PLACED, DAMAGE, [0xC1, 0xC6, 0xA2, 0x23]]),
            vec![owned(moved(100, SPOT)), live(130, 1), broken],
        );
        assert_eq!(objects[0].broken, Some(Some(300)));
        assert_eq!(how(&objects[0], false, None).0, How::Destroyed);

        let device = |destroyed: bool| Device {
            destroyed: Some(destroyed),
            ..Device::default()
        };
        let flagged = |frame: u32, destroyed: bool| Change {
            device: Some(device(destroyed)),
            ..at(frame)
        };
        let objects = read(
            entity(&[DEVICE, PLACED]),
            vec![
                owned(moved(100, SPOT)),
                flagged(101, false),
                live(130, 1),
                flagged(300, true),
            ],
        );
        assert_eq!(objects[0].destroyed_flag, Some(Some(300)));
        assert_eq!(how(&objects[0], false, None).0, How::Destroyed);
    }

    fn release(frame: u32, position: [f32; 3]) -> Change {
        let owner = Owner {
            player: Some(PLAYER),
            released: Some(1),
            ..Owner::default()
        };
        Change {
            owner: Some(owner),
            live: Some(1),
            ..moved(frame, position)
        }
    }

    fn blob(frame: u32, state: [u8; 4]) -> Change {
        let mut bytes = vec![12, 0, 0, 0];
        bytes.extend(state);
        bytes.extend([0; 8]);
        let state = State {
            blob: Some(([0xAD, 0x55, 0x0A, 0x19], bytes)),
            ..State::default()
        };
        Change {
            state: Some(state),
            ..at(frame)
        }
    }

    #[test]
    fn a_release_starts_a_thrown_gadget_and_a_second_one_another() {
        let back = Change {
            owner: Some(Owner {
                released: Some(0),
                ..Owner::default()
            }),
            ..at(60)
        };
        let objects = read(
            entity(&[STATE, OWNER]),
            vec![
                release(10, SPOT),
                moved(11, [10.0, 21.0, 1.5]),
                back,
                live(61, 0),
                moved(200, POOL),
                release(300, SPOT),
            ],
        );
        assert_eq!(objects.len(), 2);
        let o = &objects[0];
        assert_eq!(
            (o.kind, o.owner, o.start),
            (Kind::Thrown, Some(PLAYER), Some(10))
        );
        assert_eq!(o.deployed, Some(Some(10)));
        assert_eq!(o.track.len(), 2);
        assert_eq!((o.returned, o.not_live), (Some(Some(60)), Some(Some(61))));
        assert_eq!(o.gone, Some((Some(200), Gone::Pooled)));
        assert_eq!(how(o, false, Some(5.0)), (How::Removed, Source::Read));
        assert_eq!(objects[1].start, Some(300));
    }

    #[test]
    fn states_are_listed_when_their_four_bytes_change() {
        let (invalid, closed) = (STATE_NAMES[0].0, STATE_NAMES[1].0);
        let mut progress = blob(14, closed);
        if let Some((_, bytes)) = progress.state.as_mut().and_then(|s| s.blob.as_mut()) {
            bytes[9] = 0x80;
        }
        let objects = read(
            entity(&[STATE, OWNER]),
            vec![
                blob(5, invalid),
                release(10, SPOT),
                moved(11, SPOT),
                blob(13, closed),
                progress,
                blob(20, invalid),
            ],
        );
        let states: Vec<(Frame, &[u8])> = (objects[0].states.iter())
            .map(|s| (s.0, s.2.as_slice()))
            .collect();
        // The state it had before the release is listed at the first
        // update after it.
        assert_eq!(
            states,
            [
                (Some(11), &invalid[..]),
                (Some(13), &closed[..]),
                (Some(20), &invalid[..])
            ]
        );
        assert_eq!(state_of(&[1, 2]), [1, 2]);
    }

    #[test]
    fn a_flight_ends_at_the_first_pause() {
        let time = |frame: Frame| frame.map(|f| f64::from(f) / 30.0);
        let track: Vec<(Frame, [f32; 3])> = [10, 11, 12, 13, 40, 41]
            .iter()
            .map(|&f| (Some(f), [f as f32, 0.0, 0.0]))
            .collect();
        assert_eq!(rest(&track, time), Some(3));
        assert_eq!(rest(&track[..3], time), Some(2));
        assert_eq!(rest(&[], time), None);
        // Frames without a time are told apart by their number.
        assert_eq!(rest(&track, |_| None), Some(3));
    }
}
