//! Destruction (Y11S3): what was damaged by what and by whom, the holes
//! that left, and what became of every breach device. All of it is read
//! from the `World` of a round, the entities and map objects of the
//! `movement` stream with their damage lists (see [`crate::world`]).
//!
//! # Records
//!
//! Whatever can be damaged has a damage list: a destructible part of the
//! map (a map object of class `6ea51c35`), a barricade, a reinforcement.
//! Each update carries the new records of the list. A record names the
//! point that was struck and the impacts it left, in the object's own
//! space; the instigator, a player's body or a gadget entity; and the
//! damage id, which says what did it (`crate::types::damage_tables`).
//! Its kind byte is 0 for anything that is not a bullet, 1 for another
//! player's bullet, 2 for a bullet of the recording player as their game
//! predicted it and 3 for the same bullet confirmed. A kind 3 record is
//! dropped when the object had the same damage id at the same point as
//! kind 2 before; one without such a record is kept as a bullet.
//!
//! The damage a map starts with is in the full state of its objects, by
//! instigator 0 or by a map object, in the first second: it is set aside.
//! A point is put on the map as `position + rotation * point` with the
//! object's transform at that moment.
//!
//! # Events
//!
//! The records of one instigator and damage id that follow each other
//! within 0.15 s are one event (`destruction[]`): one explosion lands on a
//! dozen objects over a few frames. The two damage ids of a Hard Breach
//! Charge count as one. Bullets are left out of the events, as `shots`
//! has them; they are counted in the surfaces. Melee hits are in: they
//! are also in `meleeHits`, which has the player's side of them.
//!
//! `username` is the player whose body the instigator is, or the owner of
//! the gadget entity it is (the last owner its `4c60869a` or `8490f616`
//! component wrote, also when the world found that component behind ones
//! of unknown size), which is then given as `instigator` with its asset.
//! An instigator that is a map object (an explosive prop) or has no create
//! message (debris) has no player.
//!
//! The world does not keep the order of the messages of one frame, so the
//! records of a frame are taken in the order of their objects' ids. An
//! event's `position` is the point of its first record in that order.
//!
//! # What an object is
//!
//! An entity is a panel when its classes are `4c60869a 6ea51c35` and its
//! slots `b4d93e43` and `2e4bce49`: a hatch reinforcement, a barricade or
//! a wall reinforcement by its asset. That is read.
//!
//! A map object does not say what it is. Its kind is derived and says
//! from what (`kindSource`): `catalog` is the table of 2,483 objects of 15
//! maps (`crate::types::map_tables_kinds`); `derived` is a hatch known
//! by the record a hatch reinforcement leaves on the hatch under it;
//! `impacts` is the vote of this round's bullet and melee impacts on the
//! object, when four in five agree: a floor has impacts with a vertical
//! normal in its own plane, a wall has them with a normal along its y (or
//! x) axis within its thickness. Anything else is an `object`.
//!
//! # Opened
//!
//! A panel is created with flags `8000`, intact. The flags going to
//! `0000` say it was opened; an `fe` entry in the damage list says the
//! object is destroyed (a barricade broken, a hatch gone). An event's
//! object is `opened` or `destroyed` when that follows the record within
//! 0.35 s.
//!
//! # Surfaces
//!
//! `surfaces[]` is derived, each entry says so: the impact points of
//! bullets and of the causes that remove material, on walls, floors and
//! hatches (on a reinforcement only those of causes that open one), put
//! together when closer than 0.45 m in the same plane (walls or floors).
//! Two or three objects make up one wall, 0.1 to 0.2 m apart, so a hole
//! spans them. A cluster of one or two bullet impacts is left out. The
//! game writes no hole size: `width` (horizontal) and `height` are the
//! extent of the points. The labels are rules of thumb:
//!
//! ```text
//! floor  verticalPlay   an explosive, breach or ability cause, a hatch
//!                       that was destroyed, or 8 points and more
//!        bulletHoles    else
//! wall   rotationHole   such a cause, or 12 points over 0.6 x 0.9 m and
//!                       more, most of them by defenders
//!        breach         the same, most of them by attackers
//!        murderHole     a melee hit, or 8 points within 0.6 m
//!        bulletHoles    else
//! ```
//!
//! # Breaches
//!
//! `breaches[]` has one entry per breach device:
//!
//! - a charge a player places: an entity whose asset is in the table of
//!   charges seen detonating, or one in the owner's body slot whose HUD
//!   item is a Hard Breach Charge, a Breach Charge or an Exothermic Charge
//!   (`deviceSource` `table` or `slot`). It is placed with its first
//!   position outside the pool, armed when `live` goes to 1, and
//!   `detonated` when records name it as their instigator. `destroyed` is
//!   `live` back to 0 without a record; `removed` a return to the pool, or
//!   never armed; `armedAtEnd` still there as the recording stops.
//! - a breaching projectile (Ash, Zofia, Gonne-6, Kali): `detonated` when
//!   records name it, else `noDestruction`.
//! - an X-KAIROS volley: the pellets that stuck within 0.6 s and 3 m of
//!   the first; its position is their middle. A pellet detonated when an
//!   X-KAIROS record, within 0.25 s of the pellet's delete, has an impact
//!   within 0.2 m of where it sat. The records are by Hibana's body.
//! - a run of Breaching Torch records of one player on one object, with
//!   no pause over 2 s: `burned`.
//!
//! `affected` lists the objects the device's records landed on:
//! `opened` or `destroyed` when that followed one of its records on the
//! object within 0.35 s, `alreadyOpen` when the flag was cleared before.
//! `openedReinforcement` says one of them is a reinforcement it opened.
//! A hatch reinforcement that is opened is destroyed too. The target is
//! the object the charge is fixed to; on a wall that is the map's wall,
//! so the reinforcement whose panel holds the charge is looked up by its
//! place (`reinforcement`).
//!
//! The file does not say what stopped a device that died without going
//! off. `near` lists what was around it at that moment: a Shock Wire or
//! Electroclaw within 2.5 m, a defender's bullet within 1.3 m, an
//! explosion within 3 m, each within 0.3 s. `stoppedBy` names one of the
//! three and is a guess by proximity (`stoppedBySource`); with nothing of
//! the kind near, nothing is named.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use crate::header::{Header, Player};
use crate::loadout::{Input, When};
use crate::panels::{self, PanelKind};
use crate::types::damage_tables::{Category, Cause, Device, cause, device};
use crate::types::map_tables_kinds::{ObjectKind, object_kind};
use crate::types::{PanelAsset, TeamRole, item_name, panel_asset};
use crate::world::{DAMAGE, Damage, Entity, Impact, OWNER, PLACED, World};

/// Body slots that hold what a player places, and which HUD slot names
/// each: 0 the ability, 1 the gadget.
const CARRIED: [([u8; 4], usize); 3] = [
    ([0x08, 0x2C, 0xA3, 0x1D], 0),
    ([0xD8, 0x55, 0xB4, 0xAF], 1),
    ([0x41, 0x20, 0x14, 0x8B], 1),
];

/// Launched breachers, by asset.
const PROJECTILES: [(u64, &str); 4] = [
    (391794703535, "Breaching Round"),
    (311776456053, "KS79 Lifeline impact grenade"),
    (392233335597, "Gonne-6"),
    (392233337653, "LV Explosive Lance"),
];
/// A pellet of Hibana's X-KAIROS.
const PELLET: u64 = 391794748337;
/// What catches a projectile.
const MAGNET: u64 = 238373639961;
/// What electrifies a reinforcement.
const ELECTRIC: [(u64, &str); 2] = [
    (382651791603, "Shock Wire"),
    (238373641367, "Rtila Electroclaw"),
];

/// Damage ids the rules below name.
const MELEE: u64 = 34118943362;
const HATCH_PLACED: u64 = 364196079763;
const HARD_BREACH: [u64; 2] = [261358999180, 285432852752];
const X_KAIROS: u64 = 41850534886;
const TORCH: [u64; 3] = [171524950208, 155037197079, 137413294363];

/// Records by nobody in a full state of the first second are damage the
/// map starts with.
const PRESET_WINDOW: f64 = 1.0;
/// Records of one instigator and cause this close are one event.
const GROUP_WINDOW: f64 = 0.15;
/// The intact flag or the destroyed entry follows its record within this.
const OPEN_WINDOW: f64 = 0.35;
/// Impact points closer than this make one surface.
const CLUSTER: f64 = 0.45;
/// An object's place is in the pool below this height.
const POOL_HEIGHT: f64 = -90.0;
/// What is near a device that died: seconds around its death, and metres
/// to an electric gadget, a bullet and an explosion.
const NEAR_WINDOW: f64 = 0.3;
const NEAR_ELECTRIC: f64 = 2.5;
const NEAR_SHOT: f64 = 1.3;
const NEAR_EXPLOSION: f64 = 3.0;
const NEAR_MAGNET: f64 = 0.3;

/// What a damaged object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Wall,
    Floor,
    Hatch,
    /// A map object that is none of the three, or unknown.
    Object,
    ReinforcedWall,
    ReinforcedHatch,
    Barricade,
    /// An entity that is no panel: a gadget with a damage list.
    Entity,
}

impl Kind {
    pub(crate) fn reinforced(self) -> bool {
        matches!(self, Kind::ReinforcedWall | Kind::ReinforcedHatch)
    }

    fn hatch(self) -> bool {
        matches!(self, Kind::Hatch | Kind::ReinforcedHatch)
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Wall => "wall",
            Kind::Floor => "floor",
            Kind::Hatch => "hatch",
            Kind::Object => "object",
            Kind::ReinforcedWall => "reinforcedWall",
            Kind::ReinforcedHatch => "reinforcedHatch",
            Kind::Barricade => "barricade",
            Kind::Entity => "entity",
        }
    }
}

/// Where the kind of a map object is from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KindSource {
    /// The table of map objects.
    Catalog,
    /// The vote of this round's impacts on the object.
    Impacts,
    /// A hatch reinforcement left its record on it.
    Derived,
}

/// What did the damage of an event.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CauseInfo {
    /// The damage id.
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
    pub category: Category,
    /// What the id stands for was worked out, not confirmed by who or
    /// what did it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub inferred: bool,
}

/// One object an event damaged.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Damaged {
    /// The object's id, in hex.
    pub object: String,
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind_source: Option<KindSource>,
    /// The asset of an entity that is no panel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<u64>,
    /// Where it was struck: `[x, y, z]` on the map.
    pub impacts: Vec<[f64; 3]>,
    /// Its intact flag was cleared right after.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub opened: bool,
    /// Its damage list said it was destroyed right after.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub destroyed: bool,
}

/// One thing that damaged the map or a panel: an explosion, a charge, a
/// melee hit. Not a bullet.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Destruction {
    pub cause: CauseInfo,
    /// Who did it: the player, or the owner of the gadget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// The instigator when it is no player's body, in hex: a gadget
    /// entity, a map object, debris.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instigator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instigator_asset: Option<u64>,
    /// Where the first record struck.
    pub position: [f64; 3],
    pub objects: Vec<Damaged>,
    #[serde(flatten)]
    pub when: When,
}

/// The label of a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Label {
    VerticalPlay,
    RotationHole,
    Breach,
    MurderHole,
    BulletHoles,
}

/// The plane of a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Plane {
    Wall,
    Floor,
}

/// A cluster of impacts on walls or floors: a hole, or a patch of bullet
/// holes. Derived, the label most of all. `when` is its first impact.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Surface {
    pub kind: Plane,
    pub label: Label,
    /// Always true: nothing of a surface is read as such.
    pub derived: bool,
    /// Some of it is on a reinforcement.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub reinforced: bool,
    /// Some of it is on a hatch.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hatch: bool,
    /// The objects it is on, in hex.
    pub objects: Vec<String>,
    /// The middle of the points' extent.
    pub position: [f64; 3],
    /// The horizontal and the vertical extent of the points, in metres.
    pub width: f64,
    pub height: f64,
    pub points: usize,
    /// Points per cause; `bullet` for every gun.
    pub causes: BTreeMap<&'static str, usize>,
    /// Points per player.
    pub makers: BTreeMap<String, usize>,
    /// The side that made most of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<TeamRole>,
    /// Seconds since the recording started of a hatch of it going.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hatch_destroyed: Option<f64>,
    /// Seconds since the recording started of its last impact.
    pub until: f64,
    #[serde(flatten)]
    pub when: When,
}

/// How a breach device ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// It went off: records name it. For a volley, every pellet did.
    Detonated,
    /// It died without going off.
    Destroyed,
    /// Taken back, or the placing was called off.
    Removed,
    /// Armed and still there as the recording stops.
    ArmedAtEnd,
    /// Deleted while armed, with no record by it.
    DeletedWithoutDamage,
    /// A projectile that damaged nothing.
    NoDestruction,
    /// A volley of which some pellets went off.
    Partly,
    /// A volley of which no pellet went off or died.
    None,
    /// A run of the Breaching Torch.
    Burned,
}

/// One object a breach device's records landed on.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Affected {
    pub object: String,
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind_source: Option<KindSource>,
    /// Its intact flag was cleared right after.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub opened: bool,
    /// Its intact flag was cleared before.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub already_open: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub destroyed: bool,
    /// A reinforcement: who put it up (joined in [`crate::join`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reinforced_by: Option<String>,
}

/// What was near a device that died without going off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NearKind {
    Gadget,
    Shot,
    Explosion,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Near {
    pub kind: NearKind,
    /// The gadget, or the cause of the explosion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gadget: Option<&'static str>,
    /// Its owner, the shooter, or who set the explosion off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Metres from the device.
    pub distance: f64,
}

/// What may have stopped a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StoppedBy {
    Electricity,
    Shot,
    Explosion,
}

/// One breach device and what became of it. `when` is its placing: the
/// launch of a projectile, the first pellet of a volley, the first record
/// of a torch run.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Breach {
    pub device: &'static str,
    /// What says which device a placed charge is: `slot`, the owner's
    /// body slot and its HUD item, or `table`, this module's assets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_source: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// `assumed` when the player is the round's only Hibana and nothing
    /// in the file names them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username_source: Option<&'static str>,
    /// The device's entity, in hex. A volley and a torch run have none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    /// The object it is fixed to, or the torch burns, in hex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// A [`Kind`], or `mapObject` for a map object of unknown kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<&'static str>,
    /// The reinforcement the target is, or the one whose panel holds the
    /// device on the target, in hex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reinforcement: Option<String>,
    /// Who put that reinforcement up (joined in [`crate::join`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reinforced_by: Option<String>,
    pub position: [f64; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub armed: Option<When>,
    pub outcome: Outcome,
    /// When the outcome came about: the first record, the death, the
    /// return to the pool, the delete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<When>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub affected: Vec<Affected>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opened_reinforcement: Option<bool>,
    /// A volley: its pellets, and how many went off and died.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pellets: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detonated: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroyed: Option<u32>,
    /// A torch run: its impacts and their extent along x, y and z.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extent: Option<[f64; 3]>,
    /// What was around a device that did not go off.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub near: Vec<Near>,
    /// A guess from `near`, never read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped_by: Option<StoppedBy>,
    /// `proximity`, with `stoppedBy`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped_by_source: Option<&'static str>,
    #[serde(flatten)]
    pub when: When,
}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    pub destruction: Vec<Destruction>,
    pub surfaces: Vec<Surface>,
    pub breaches: Vec<Breach>,
    /// Reinforcement entity and the frame its intact flag was cleared in.
    pub opened: Vec<(u64, u32)>,
    /// Records kept, of which bullets; and records set aside as damage
    /// the map starts with.
    pub records: usize,
    pub bullets: usize,
    pub preset: usize,
    /// Events of debris and props colliding, which are left out.
    pub physics: usize,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
}

/// A moment: the frame, and its seconds since the recording started.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct At {
    frame: Option<u32>,
    t: f64,
}

/// What an entity's updates say of its life.
#[derive(Clone, Debug, Default, PartialEq)]
struct Life {
    /// Seconds of its create message.
    born: f64,
    /// Its first position outside the pool; again after a return to the
    /// pool, until it is armed.
    placed: Option<At>,
    /// `live` going to 1 once placed.
    armed: Option<At>,
    /// `live` back to 0 after that.
    dead: Option<At>,
    /// Back to the pool without dying.
    removed: Option<At>,
    deleted: Option<At>,
    pooled: bool,
    /// Its positions outside the pool.
    track: Vec<(f64, [f64; 3])>,
    /// Its last rotation.
    rotation: [f64; 4],
    /// The playerid its owner or placed component last named.
    owner: Option<u64>,
    /// The object its placed component last fixed it to.
    target: Option<u64>,
}

impl Life {
    /// Where it last was outside the pool.
    fn last(&self) -> Option<[f64; 3]> {
        self.track.last().map(|p| p.1)
    }

    /// Where it was at `t`, a tenth of a second ahead included.
    fn place_at(&self, t: f64) -> Option<[f64; 3]> {
        let end = self.track.partition_point(|p| p.0 <= t + 0.1);
        self.track.get(end.checked_sub(1)?).map(|p| p.1)
    }
}

/// One record of a damage list, placed on the map.
#[derive(Clone, Debug, PartialEq)]
struct Record<'a> {
    damage: &'a Damage,
    at: At,
    object: u64,
    /// The object is part of the map.
    map: bool,
    /// The object's rotation at that moment.
    rotation: [f64; 4],
    bullet: bool,
    /// The struck point and the impacts, on the map.
    point: [f64; 3],
    impacts: Vec<[f64; 3]>,
}

/// Who a record is by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Who {
    /// Index into the players.
    player: Option<usize>,
    /// The instigator, when it is no player's body.
    instigator: Option<u64>,
    asset: Option<u64>,
}

fn wide<const N: usize>(v: [f32; N]) -> [f64; N] {
    v.map(f64::from)
}

/// `q` applied to `v`, as [`World::rotate`], in double precision.
fn rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
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

fn to_map(position: [f64; 3], q: [f64; 4], point: [f64; 3]) -> [f64; 3] {
    let r = rotate(q, point);
    [position[0] + r[0], position[1] + r[1], position[2] + r[2]]
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

fn round(v: f64, digits: i32) -> f64 {
    let scale = 10f64.powi(digits);
    (v * scale).round() / scale
}

fn place(p: [f64; 3]) -> [f64; 3] {
    p.map(|v| round(v, 3))
}

fn hex(id: u64) -> String {
    format!("{id:x}")
}

/// Whether one of `times` is at `t` or up to `window` after it.
fn follows(times: Option<&Vec<At>>, t: f64, window: f64) -> bool {
    times.is_some_and(|times| (times.iter()).any(|a| a.t >= t - 0.001 && a.t <= t + window))
}

/// What one impact says its object is, from where it is in the object's
/// space. Only an object that stands upright says anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Face {
    Floor,
    Wall,
    Other,
}

fn face(rotation: [f64; 4], impact: &Impact) -> Face {
    if rotation[0].abs() >= 0.01 || rotation[1].abs() >= 0.01 {
        return Face::Other;
    }
    let p = wide(impact.position);
    let n = wide(impact.normal);
    let height = (-0.05..=6.0).contains(&p[2]);
    if n[2].abs() > 0.99 && p[2].abs() <= 0.03 {
        Face::Floor
    } else if (n[1].abs() > 0.99 && p[1].abs() <= 0.12 && height)
        || (n[0].abs() > 0.99 && p[0].abs() <= 0.03 && height)
    {
        Face::Wall
    } else {
        Face::Other
    }
}

/// What an entity with a damage list is: a panel by its classes and
/// slots, and which panel by its asset (see [`crate::panels`]).
fn entity_kind(e: &Entity) -> Kind {
    if !panels::is_panel(e) {
        return Kind::Entity;
    }
    match panels::kind_of(e.asset) {
        Some(PanelKind::HatchReinforcement) => Kind::ReinforcedHatch,
        Some(PanelKind::Barricade | PanelKind::CastlePanel) => Kind::Barricade,
        Some(PanelKind::WallReinforcement) => Kind::ReinforcedWall,
        None => Kind::Entity,
    }
}

/// Half the width of a wall reinforcement in metres, by its asset; 0.8
/// for one whose width the table does not have.
fn half_width(asset: u64) -> f64 {
    match panel_asset(asset) {
        Some(PanelAsset::Wall { width: Some(w) }) => f64::from(w) / 2.0,
        _ => 0.8,
    }
}

/// The round as the rules below read it.
struct Scene<'a> {
    world: &'a World,
    players: &'a [Player],
    /// The side of each player.
    roles: Vec<Option<TeamRole>>,
    /// The map id.
    map: u64,
    lives: HashMap<u64, Life>,
    /// Object -> when its flags were written 0, and when its damage list
    /// said it was destroyed.
    cleared: HashMap<u64, Vec<At>>,
    destroyed: HashMap<u64, Vec<At>>,
    /// The records kept, by frame and then by object.
    records: Vec<Record<'a>>,
    preset: usize,
    /// Map objects a hatch reinforcement left its record on.
    hatches: HashSet<u64>,
    /// Map object -> impacts that say floor, wall and neither.
    votes: HashMap<u64, [usize; 3]>,
}

impl<'a> Scene<'a> {
    /// Walks the world once: lives, flags, and the records to keep.
    fn read(
        world: &'a World,
        players: &'a [Player],
        roles: Vec<Option<TeamRole>>,
        map: u64,
        seconds: &dyn Fn(Option<u32>) -> f64,
    ) -> Scene<'a> {
        let mut scene = Scene {
            world,
            players,
            roles,
            map,
            lives: HashMap::new(),
            cleared: HashMap::new(),
            destroyed: HashMap::new(),
            records: Vec::new(),
            preset: 0,
            hatches: HashSet::new(),
            votes: HashMap::new(),
        };
        for e in world.iter() {
            let is_map = e.is_map();
            let followed = if is_map {
                e.classes.first() == Some(&DAMAGE)
            } else {
                e.has(PLACED) || e.has(DAMAGE) || e.has(OWNER)
            };
            if !followed {
                continue;
            }
            let mut position = wide(e.position);
            let mut life = Life {
                born: seconds(e.created),
                pooled: true,
                rotation: wide(e.rotation),
                ..Life::default()
            };
            // The object's predicted bullets: damage id and point.
            let mut predicted: HashSet<(u64, [u32; 3])> = HashSet::new();
            for c in &e.changes {
                let at = At {
                    frame: c.frame,
                    t: seconds(c.frame),
                };
                if let Some(p) = c.position {
                    position = wide(p);
                    if position[2] > POOL_HEIGHT {
                        if life.placed.is_none() || (life.armed.is_none() && life.pooled) {
                            life.placed = Some(at);
                        }
                        life.pooled = false;
                        life.track.push((at.t, position));
                    } else {
                        life.pooled = true;
                        if life.placed.is_some() && life.removed.is_none() && life.dead.is_none() {
                            life.removed = Some(at);
                        }
                    }
                }
                if let Some(q) = c.rotation {
                    life.rotation = wide(q);
                }
                match c.live {
                    Some(1) if life.armed.is_none() && life.placed.is_some() => {
                        life.armed = Some(at);
                        life.removed = None;
                    }
                    Some(0) if life.armed.is_some() && life.dead.is_none() => {
                        life.dead = Some(at);
                    }
                    _ => {}
                }
                if c.flags == Some(0) {
                    scene.cleared.entry(e.id).or_default().push(at);
                }
                if let Some(placed) = c.placed {
                    life.owner = placed.owner.or(life.owner);
                    life.target = placed.host.or(life.target);
                }
                if let Some(owner) = c.owner {
                    life.owner = owner.player.or(life.owner);
                }
                if c.destroyed {
                    scene.destroyed.entry(e.id).or_default().push(at);
                }
                for d in &c.damage {
                    let key = (d.id, d.point.map(f32::to_bits));
                    if d.kind == 2 {
                        predicted.insert(key);
                    }
                    if d.kind == 3 && predicted.contains(&key) {
                        continue;
                    }
                    let nobody =
                        d.instigator == 0 || (d.instigator >= 1 << 32 && d.instigator != e.id);
                    if nobody && c.full && at.t < PRESET_WINDOW {
                        scene.preset += 1;
                        continue;
                    }
                    let on_map = |p: [f32; 3]| to_map(position, life.rotation, wide(p));
                    scene.records.push(Record {
                        damage: d,
                        at,
                        object: e.id,
                        map: is_map,
                        rotation: life.rotation,
                        bullet: matches!(d.kind, 1..=3),
                        point: on_map(d.point),
                        impacts: d.impacts.iter().map(|i| on_map(i.position)).collect(),
                    });
                }
            }
            if !is_map {
                life.deleted = e.deleted.map(|f| At {
                    frame: Some(f),
                    t: seconds(Some(f)),
                });
                scene.lives.insert(e.id, life);
            }
        }
        // An object's records are in stream order; those of one frame are
        // taken by object.
        scene.records.sort_by_key(|r| (r.at.frame, r.object));
        for r in &scene.records {
            if !r.map {
                continue;
            }
            if r.damage.id == HATCH_PLACED {
                scene.hatches.insert(r.object);
            }
            if r.bullet || r.damage.id == MELEE {
                let votes = scene.votes.entry(r.object).or_default();
                for i in &r.damage.impacts {
                    let slot = match face(r.rotation, i) {
                        Face::Floor => 0,
                        Face::Wall => 1,
                        Face::Other => 2,
                    };
                    if let Some(v) = votes.get_mut(slot) {
                        *v += 1;
                    }
                }
            }
        }
        scene
    }

    fn username(&self, player: Option<usize>) -> Option<String> {
        (self.players.get(player?)).map(|p| p.username.clone())
    }

    fn role(&self, player: Option<usize>) -> Option<TeamRole> {
        self.roles.get(player?).copied().flatten()
    }

    /// The player with this `playerid`.
    fn player(&self, id: Option<u64>) -> Option<usize> {
        let id = id.filter(|&id| id != 0)?;
        self.players.iter().position(|p| p.id == id)
    }

    /// The owner of an entity, as a player.
    fn owner(&self, entity: u64) -> Option<usize> {
        self.player(self.lives.get(&entity)?.owner)
    }

    /// What a map object is, and what says so.
    fn object_kind(&self, object: u64) -> (Kind, Option<KindSource>) {
        let listed = object_kind(self.map, object);
        if listed == Some(ObjectKind::Hatch) {
            return (Kind::Hatch, Some(KindSource::Catalog));
        }
        if self.hatches.contains(&object) {
            return (Kind::Hatch, Some(KindSource::Derived));
        }
        match listed {
            Some(ObjectKind::Wall) => return (Kind::Wall, Some(KindSource::Catalog)),
            Some(ObjectKind::Floor) => return (Kind::Floor, Some(KindSource::Catalog)),
            Some(_) => return (Kind::Object, Some(KindSource::Catalog)),
            None => {}
        }
        if self.destroyed.contains_key(&object) {
            return (Kind::Object, None);
        }
        let Some(&[floor, wall, other]) = self.votes.get(&object) else {
            return (Kind::Object, None);
        };
        // Four in five of the impacts agree.
        let all = floor + wall + other;
        let agreed = |n: usize| n > other && n * 5 >= all * 4;
        if agreed(floor) && floor >= wall {
            (Kind::Floor, Some(KindSource::Impacts))
        } else if agreed(wall) {
            (Kind::Wall, Some(KindSource::Impacts))
        } else {
            (Kind::Object, None)
        }
    }

    /// What any damaged object is.
    fn kind(&self, object: u64, map: bool) -> (Kind, Option<KindSource>) {
        if map {
            return self.object_kind(object);
        }
        (
            self.world.get(object).map_or(Kind::Entity, entity_kind),
            None,
        )
    }

    fn kind_of(&self, r: &Record) -> Kind {
        self.kind(r.object, r.map).0
    }

    /// Who a record is by: the player whose body the instigator is, or
    /// the owner of the gadget it is.
    fn who(&self, r: &Record) -> Who {
        let body = r.damage.instigator;
        if let Some(player) = self.world.player_of(body) {
            return Who {
                player: Some(player),
                ..Who::default()
            };
        }
        let instigator = (body != 0).then_some(body);
        let gadget = (self.lives.get(&body))
            .filter(|l| body < 1 << 32 && l.born <= r.at.t + 0.2)
            .and(self.world.get(body));
        Who {
            player: gadget.and_then(|g| self.owner(g.id)),
            instigator,
            asset: gadget.map(|g| g.asset),
        }
    }

    /// The object of `r` was opened, or destroyed, right after it.
    fn opened(&self, r: &Record) -> bool {
        follows(self.cleared.get(&r.object), r.at.t, OPEN_WINDOW)
    }

    fn gone(&self, r: &Record) -> bool {
        follows(self.destroyed.get(&r.object), r.at.t, OPEN_WINDOW)
    }

    /// The records that are no bullets, grouped into events.
    fn events(&self, when: &dyn Fn(At) -> When) -> Vec<Destruction> {
        let mut events: Vec<Destruction> = Vec::new();
        // (instigator, damage id) -> its last event and that one's last
        // record, and the objects of each event.
        let mut open: HashMap<(u64, u64), (usize, f64)> = HashMap::new();
        let mut objects: Vec<Vec<u64>> = Vec::new();
        for r in self.records.iter().filter(|r| !r.bullet) {
            let id = r.damage.id;
            let group = if HARD_BREACH.contains(&id) {
                HARD_BREACH[0]
            } else {
                id
            };
            let key = (r.damage.instigator, group);
            let current = (open.get(&key)).filter(|g| r.at.t - g.1 <= GROUP_WINDOW);
            let index = match current {
                Some(g) => g.0,
                None => {
                    let known = cause(id);
                    let who = self.who(r);
                    events.push(Destruction {
                        cause: CauseInfo {
                            id,
                            name: known.map(|c| c.name),
                            category: known.map_or(Category::Unknown, |c| c.category),
                            inferred: known.is_some_and(|c| c.inferred),
                        },
                        username: self.username(who.player),
                        instigator: who.instigator.map(hex),
                        instigator_asset: who.asset,
                        position: place(r.point),
                        objects: Vec::new(),
                        when: when(r.at),
                    });
                    objects.push(Vec::new());
                    events.len() - 1
                }
            };
            open.insert(key, (index, r.at.t));
            let (Some(event), Some(ids)) = (events.get_mut(index), objects.get_mut(index)) else {
                continue;
            };
            let at = match ids.iter().position(|o| *o == r.object) {
                Some(at) => at,
                None => {
                    let (kind, kind_source) = self.kind(r.object, r.map);
                    let entity = (kind == Kind::Entity).then(|| self.world.get(r.object));
                    event.objects.push(Damaged {
                        object: hex(r.object),
                        kind,
                        kind_source,
                        asset: entity.flatten().map(|e| e.asset),
                        impacts: Vec::new(),
                        opened: false,
                        destroyed: false,
                    });
                    ids.push(r.object);
                    ids.len() - 1
                }
            };
            if let Some(o) = event.objects.get_mut(at) {
                o.impacts.extend(r.impacts.iter().map(|p| place(*p)));
                o.opened |= self.opened(r);
                o.destroyed |= self.gone(r);
            }
        }
        events
    }

    /// The reinforcements and the frame each was opened in.
    fn opened_reinforcements(&self) -> Vec<(u64, u32)> {
        let mut out: Vec<(u64, u32)> = (self.cleared.iter())
            .filter(|(id, _)| (self.world.get(**id)).is_some_and(|e| entity_kind(e).reinforced()))
            .filter_map(|(id, times)| Some((*id, times.first()?.frame?)))
            .collect();
        out.sort_by_key(|o| (o.1, o.0));
        out
    }

    /// The impact points that remove material, clustered.
    fn surfaces(&self, when: &dyn Fn(At) -> When) -> Vec<Surface> {
        struct Point {
            at: [f64; 3],
            time: At,
            /// `None` for a bullet.
            cause: Option<&'static Cause>,
            player: Option<usize>,
            kind: Kind,
            object: u64,
            plane: Plane,
        }
        let mut points: Vec<Point> = Vec::new();
        for r in &self.records {
            let known = cause(r.damage.id);
            if !r.bullet && !known.is_some_and(|c| c.holes) {
                continue;
            }
            let kind = self.kind_of(r);
            let plane = match kind {
                Kind::Floor | Kind::Hatch | Kind::ReinforcedHatch => Plane::Floor,
                Kind::Wall | Kind::ReinforcedWall => Plane::Wall,
                _ => continue,
            };
            // Only what opens a reinforcement leaves a hole in one.
            if kind.reinforced() && !known.is_some_and(|c| c.hard) {
                continue;
            }
            let player = self.who(r).player;
            let own = [r.point];
            let at = if r.impacts.is_empty() {
                &own[..]
            } else {
                &r.impacts
            };
            points.extend(at.iter().map(|&at| Point {
                at,
                time: r.at,
                cause: known.filter(|_| !r.bullet),
                player,
                kind,
                object: r.object,
                plane,
            }));
        }

        // Single linkage: a point joins every earlier one closer than
        // `CLUSTER`, found through a grid of cells that size.
        fn find(parent: &mut [usize], mut i: usize) -> usize {
            while let Some(&p) = parent.get(i) {
                if p == i {
                    break;
                }
                let above = parent.get(p).copied().unwrap_or(p);
                if let Some(slot) = parent.get_mut(i) {
                    *slot = above;
                }
                i = above;
            }
            i
        }
        let cell = |p: &Point| p.at.map(|v| (v / CLUSTER).floor() as i64);
        let mut parent: Vec<usize> = (0..points.len()).collect();
        let mut grid: HashMap<(Plane, [i64; 3]), Vec<usize>> = HashMap::new();
        for (i, p) in points.iter().enumerate() {
            let c = cell(p);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let near = grid.get(&(p.plane, [c[0] + dx, c[1] + dy, c[2] + dz]));
                        for &j in near.into_iter().flatten() {
                            let close =
                                (points.get(j)).is_some_and(|q| distance(q.at, p.at) < CLUSTER);
                            if close {
                                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                                if let Some(slot) = parent.get_mut(a) {
                                    *slot = b;
                                }
                            }
                        }
                    }
                }
            }
            grid.entry((p.plane, c)).or_default().push(i);
        }
        let mut clusters: Vec<Vec<&Point>> = Vec::new();
        let mut index: HashMap<usize, usize> = HashMap::new();
        for (i, p) in points.iter().enumerate() {
            let root = find(&mut parent, i);
            let at = *index.entry(root).or_insert_with(|| {
                clusters.push(Vec::new());
                clusters.len() - 1
            });
            if let Some(c) = clusters.get_mut(at) {
                c.push(p);
            }
        }

        let mut out: Vec<(f64, Surface)> = Vec::new();
        for c in &clusters {
            let Some(first) = c.first() else {
                continue;
            };
            let mut causes: BTreeMap<&'static str, usize> = BTreeMap::new();
            let mut makers: BTreeMap<String, usize> = BTreeMap::new();
            // Points by attackers and by defenders, and who came first.
            let mut sides: Vec<(TeamRole, usize)> = Vec::new();
            let (mut lo, mut hi) = (first.at, first.at);
            let (mut start, mut end) = (first.time, first.time.t);
            for p in c {
                for i in 0..3 {
                    lo[i] = lo[i].min(p.at[i]);
                    hi[i] = hi[i].max(p.at[i]);
                }
                if p.time.t < start.t {
                    start = p.time;
                }
                end = end.max(p.time.t);
                *causes
                    .entry(p.cause.map_or("bullet", |c| c.name))
                    .or_default() += 1;
                if let Some(name) = self.username(p.player) {
                    *makers.entry(name).or_default() += 1;
                }
                if let Some(role) = self.role(p.player) {
                    match sides.iter_mut().find(|s| s.0 == role) {
                        Some(side) => side.1 += 1,
                        None => sides.push((role, 1)),
                    }
                }
            }
            let only_bullets = c.iter().all(|p| p.cause.is_none());
            if only_bullets && c.len() < 3 {
                // Stray punctures.
                continue;
            }
            let width = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2)).sqrt();
            let height = hi[2] - lo[2];
            let big = c.iter().any(|p| {
                p.cause.is_some_and(|c| {
                    matches!(
                        c.category,
                        Category::Explosive
                            | Category::SoftBreach
                            | Category::HardBreach
                            | Category::Ability
                    )
                })
            });
            // The first side with the most points.
            let side = (sides.iter())
                .fold(None, |best: Option<(TeamRole, usize)>, s| match best {
                    Some(b) if b.1 >= s.1 => Some(b),
                    _ => Some(*s),
                })
                .map(|s| s.0);
            let mut hatches: Vec<u64> = (c.iter())
                .filter(|p| p.kind.hatch())
                .map(|p| p.object)
                .collect();
            hatches.sort_unstable();
            hatches.dedup();
            let hatch_destroyed = (hatches.iter())
                .filter_map(|h| self.destroyed.get(h))
                .flatten()
                .map(|a| a.t)
                .min_by(f64::total_cmp);
            let melee = c.iter().any(|p| p.cause.is_some_and(|c| c.id == MELEE));
            let label = if first.plane == Plane::Floor {
                if big || hatch_destroyed.is_some() || c.len() >= 8 {
                    Label::VerticalPlay
                } else {
                    Label::BulletHoles
                }
            } else if big || (width >= 0.6 && height >= 0.9 && c.len() >= 12) {
                if side == Some(TeamRole::Defense) {
                    Label::RotationHole
                } else {
                    Label::Breach
                }
            } else if melee || (c.len() >= 8 && width.max(height) <= 0.6) {
                Label::MurderHole
            } else {
                Label::BulletHoles
            };
            let mut objects: Vec<u64> = c.iter().map(|p| p.object).collect();
            objects.sort_unstable();
            objects.dedup();
            let middle = [0, 1, 2].map(|i| (lo[i] + hi[i]) / 2.0);
            out.push((
                start.t,
                Surface {
                    kind: first.plane,
                    label,
                    derived: true,
                    reinforced: c.iter().any(|p| p.kind.reinforced()),
                    hatch: !hatches.is_empty(),
                    objects: objects.into_iter().map(hex).collect(),
                    position: place(middle),
                    width: round(width, 2),
                    height: round(height, 2),
                    points: c.len(),
                    causes,
                    makers,
                    side,
                    hatch_destroyed: hatch_destroyed.map(|t| round(t, 3)),
                    until: round(end, 3),
                    when: when(start),
                },
            ));
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.into_iter().map(|s| s.1).collect()
    }

    /// The completed wall reinforcement whose panel holds the map point
    /// `p` at `t`. In the panel's space x runs along the wall from its
    /// middle, y up from the floor and z through the wall; the first asset
    /// is 1.6 m wide and each next one 0.1 m more (inferred), 3 m high.
    fn reinforcement_at(&self, p: [f64; 3], t: f64) -> Option<u64> {
        self.world.iter().find_map(|e| {
            if e.is_map() || entity_kind(e) != Kind::ReinforcedWall {
                return None;
            }
            let life = self.lives.get(&e.id)?;
            if life.armed.is_none_or(|a| a.t > t) {
                return None;
            }
            let origin = life.track.first()?.1;
            let [x, y, z, w] = life.rotation;
            let from = [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]];
            let local = rotate([-x, -y, -z, w], from);
            let half = half_width(e.asset);
            let inside = local[0].abs() <= half + 0.1
                && (-0.1..=3.1).contains(&local[1])
                && local[2].abs() <= 0.5;
            inside.then_some(e.id)
        })
    }

    /// What the records of one device did: the objects, each once. An
    /// object was opened or destroyed when that followed one of the
    /// records on it, and was open already when its flag was cleared
    /// before the first.
    fn affected(&self, records: &[&Record]) -> Vec<Affected> {
        let mut ids: Vec<u64> = Vec::new();
        let mut out: Vec<Affected> = Vec::new();
        for r in records {
            let at = match ids.iter().position(|o| *o == r.object) {
                Some(at) => at,
                None => {
                    let (kind, kind_source) = self.kind(r.object, r.map);
                    let cleared = self.cleared.get(&r.object).and_then(|c| c.first());
                    out.push(Affected {
                        object: hex(r.object),
                        kind,
                        kind_source,
                        opened: false,
                        already_open: cleared.is_some_and(|c| c.t < r.at.t),
                        destroyed: false,
                        reinforced_by: None,
                    });
                    ids.push(r.object);
                    ids.len() - 1
                }
            };
            if let Some(o) = out.get_mut(at) {
                o.opened |= self.opened(r);
                o.destroyed |= self.gone(r);
            }
        }
        for o in &mut out {
            o.already_open &= !o.opened;
        }
        out
    }

    /// What was near a device that died at `t` in `p`, and what of it may
    /// have stopped the device.
    fn near(
        &self,
        device: u64,
        armed: Option<At>,
        t: f64,
        p: [f64; 3],
    ) -> (Vec<Near>, Option<StoppedBy>) {
        let mut near: Vec<Near> = Vec::new();
        let mut add = |kind: NearKind, gadget, player: Option<usize>, d: f64| {
            let username = self.username(player);
            let same =
                |n: &&mut Near| n.kind == kind && n.gadget == gadget && n.username == username;
            match near.iter_mut().find(same) {
                Some(n) => n.distance = n.distance.min(round(d, 2)),
                None => near.push(Near {
                    kind,
                    gadget,
                    username,
                    distance: round(d, 2),
                }),
            }
        };
        let (mut shot, mut explosion, mut electric, mut fresh) = (false, false, false, false);
        let from = self.records.partition_point(|r| r.at.t < t - NEAR_WINDOW);
        let around = (self.records.get(from..).unwrap_or_default().iter())
            .take_while(|r| r.at.t <= t + NEAR_WINDOW);
        for r in around {
            if r.bullet {
                let closest = (r.impacts.iter())
                    .map(|i| distance(*i, p))
                    .min_by(f64::total_cmp);
                let player = self.who(r).player;
                let defender = self.role(player) == Some(TeamRole::Defense);
                if let Some(d) = closest.filter(|&d| d < NEAR_SHOT && defender) {
                    shot = true;
                    add(NearKind::Shot, None, player, d);
                }
                continue;
            }
            let d = distance(r.point, p);
            let breaks = cause(r.damage.id).filter(|c| {
                let explosive = matches!(
                    c.category,
                    Category::Explosive | Category::SoftBreach | Category::HardBreach
                );
                explosive && c.holes
            });
            if let Some(c) = breaks.filter(|_| r.damage.instigator != device && d < NEAR_EXPLOSION)
            {
                explosion = true;
                add(NearKind::Explosion, Some(c.name), self.who(r).player, d);
            }
        }
        for e in self.world.iter() {
            let Some(name) = ELECTRIC.iter().find(|g| g.0 == e.asset).map(|g| g.1) else {
                continue;
            };
            let Some(life) = self.lives.get(&e.id) else {
                continue;
            };
            let there = life.placed.is_some_and(|a| a.t <= t)
                && life.deleted.is_none_or(|a| a.t >= t - 0.5);
            let Some(d) = (life.place_at(t).filter(|_| there)).map(|at| distance(at, p)) else {
                continue;
            };
            if d > NEAR_ELECTRIC {
                continue;
            }
            electric = true;
            // It went live in the second before.
            fresh |= life.armed.is_some_and(|a| (0.0..=1.0).contains(&(t - a.t)));
            add(NearKind::Gadget, Some(name), self.player(life.owner), d);
        }
        // An explosion first. Electricity before a shot when the gadget
        // just went live or the device had just been armed.
        let just_armed = armed.is_none_or(|a| t - a.t <= 0.5);
        let stopped = if explosion {
            Some(StoppedBy::Explosion)
        } else if electric && (fresh || just_armed || !shot) {
            Some(StoppedBy::Electricity)
        } else if shot {
            Some(StoppedBy::Shot)
        } else {
            None
        };
        near.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        (near, stopped)
    }

    /// Which device a placed entity is, and what says so.
    fn device_of(
        &self,
        e: &Entity,
        life: &Life,
        items: &[[Option<u64>; 2]],
    ) -> Option<(Device, &'static str)> {
        let owner = self.player(life.owner);
        // The slot of one of the owner's bodies that holds the asset, and
        // the HUD item of that slot.
        let slot = owner
            .filter(|_| e.has(PLACED) && e.asset != 0)
            .and_then(|o| {
                let bodies = (self.world.players.iter()).filter(|b| *b.1 == o);
                let mut bodies = bodies.filter_map(|b| self.world.get(*b.0));
                bodies.find_map(|b| {
                    let holds = |c: &&([u8; 4], usize)| b.slot(c.0) == Some(e.asset);
                    CARRIED.iter().find(holds).map(|c| (o, c.1))
                })
            });
        let named = slot.and_then(|(owner, slot)| {
            let item = (*items.get(owner)?.get(slot)?)?;
            Device::named(item_name(item)?)
        });
        (named.map(|d| (d, "slot"))).or_else(|| device(e.asset).map(|d| (d, "table")))
    }

    /// Every breach device and what became of it.
    fn breaches(&self, when: &dyn Fn(At) -> When, items: &[[Option<u64>; 2]]) -> Vec<Breach> {
        // The records by each instigator that are no bullets.
        let mut by: HashMap<u64, Vec<&Record>> = HashMap::new();
        for r in self.records.iter().filter(|r| !r.bullet) {
            by.entry(r.damage.instigator).or_default().push(r);
        }
        let blank = |device: &'static str, placed: At, position: [f64; 3]| Breach {
            device,
            device_source: None,
            username: None,
            username_source: None,
            entity: None,
            target: None,
            target_kind: None,
            reinforcement: None,
            reinforced_by: None,
            position: place(position),
            armed: None,
            outcome: Outcome::None,
            ended: None,
            affected: Vec::new(),
            opened_reinforcement: None,
            pellets: None,
            detonated: None,
            destroyed: None,
            points: None,
            extent: None,
            near: Vec::new(),
            stopped_by: None,
            stopped_by_source: None,
            when: when(placed),
        };
        let opens =
            |affected: &[Affected]| affected.iter().any(|o| o.opened && o.kind.reinforced());
        let mut out: Vec<(f64, Breach)> = Vec::new();

        // Charges and projectiles.
        let mut pellets: Vec<(&Entity, &Life, At, [f64; 3])> = Vec::new();
        for e in self.world.iter().filter(|e| !e.is_map()) {
            let Some(life) = self.lives.get(&e.id) else {
                continue;
            };
            let (Some(placed), Some(at)) = (life.placed, life.last()) else {
                continue;
            };
            if e.asset == PELLET {
                pellets.push((e, life, placed, at));
                continue;
            }
            let projectile = PROJECTILES.iter().find(|p| p.0 == e.asset).map(|p| p.1);
            let charge = match projectile {
                Some(_) => None,
                None => self.device_of(e, life, items),
            };
            let Some(name) = projectile.or(charge.map(|c| c.0.name())) else {
                continue;
            };
            let mut b = blank(name, placed, at);
            b.device_source = charge.map(|c| c.1);
            b.username = self.username(self.player(life.owner));
            b.entity = Some(hex(e.id));
            // A projectile is over with its delete, or where it stopped.
            let end = match projectile {
                Some(_) => life.deleted.or(life.track.last().map(|p| At {
                    frame: None,
                    t: p.0,
                })),
                None => life.deleted,
            };
            let own = |r: &&&Record| {
                r.at.t >= life.born - 0.1 && end.is_none_or(|end| r.at.t <= end.t + 1.0)
            };
            let records: Vec<&Record> = (by.get(&e.id).into_iter().flatten())
                .filter(own)
                .copied()
                .collect();
            if projectile.is_none() {
                b.armed = life.armed.map(when);
                b.target = life.target.map(hex);
                let target = life.target.and_then(|t| Some((t, self.world.get(t)?)));
                (b.target_kind, b.reinforcement) = match target {
                    Some((id, t)) if !t.is_map() => {
                        let kind = entity_kind(t);
                        (Some(kind.name()), kind.reinforced().then(|| hex(id)))
                    }
                    _ => match (life.target, self.reinforcement_at(at, placed.t)) {
                        (Some(_), Some(panel)) => {
                            (Some(Kind::ReinforcedWall.name()), Some(hex(panel)))
                        }
                        (Some(id), None) => match self.object_kind(id).0 {
                            Kind::Object => (Some("mapObject"), None),
                            kind => (Some(kind.name()), None),
                        },
                        (None, _) => (None, None),
                    },
                };
            }
            if let Some(first) = records.first() {
                b.outcome = Outcome::Detonated;
                b.ended = Some(when(first.at));
                b.affected = self.affected(&records);
                if projectile.is_none() {
                    b.opened_reinforcement = Some(opens(&b.affected));
                }
            } else if projectile.is_some() {
                b.outcome = Outcome::NoDestruction;
                b.ended = end.filter(|e| e.frame.is_some()).map(when);
                // A Mag-NET where it ended.
                let t = end.map_or(placed.t, |e| e.t);
                for g in self.world.iter().filter(|g| g.asset == MAGNET) {
                    let Some(magnet) = self.lives.get(&g.id) else {
                        continue;
                    };
                    let there = magnet.placed.is_some_and(|p| p.t <= t);
                    let d = magnet.place_at(t).map(|p| distance(p, at));
                    if let Some(d) = d.filter(|&d| there && d < NEAR_MAGNET) {
                        b.near.push(Near {
                            kind: NearKind::Gadget,
                            gadget: Some("Mag-NET System"),
                            username: self.username(self.player(magnet.owner)),
                            distance: round(d, 2),
                        });
                    }
                }
            } else if let Some(dead) = life.dead {
                b.outcome = Outcome::Destroyed;
                b.ended = Some(when(dead));
                let (near, stopped) = self.near(e.id, life.armed, dead.t, at);
                b.near = near;
                b.stopped_by = stopped;
                b.stopped_by_source = stopped.map(|_| "proximity");
            } else if life.removed.is_some() || life.armed.is_none() {
                b.outcome = Outcome::Removed;
                b.ended = life.removed.map(when);
            } else if let Some(deleted) = life.deleted {
                b.outcome = Outcome::DeletedWithoutDamage;
                b.ended = Some(when(deleted));
            } else {
                b.outcome = Outcome::ArmedAtEnd;
            }
            out.push((placed.t, b));
        }

        // X-KAIROS: the pellets that stuck, grouped into volleys.
        struct Volley<'s, 'a> {
            first: At,
            /// Where the first pellet sat, and the sum of all their places.
            at: [f64; 3],
            sum: [f64; 3],
            pellets: u32,
            detonated: u32,
            destroyed: u32,
            records: Vec<&'s Record<'a>>,
            /// The first pellet that died: its entity, when, and where.
            dead: Option<(u64, Option<At>, At, [f64; 3])>,
            owner: Option<usize>,
        }
        pellets.sort_by(|a, b| a.2.t.total_cmp(&b.2.t).then(a.0.id.cmp(&b.0.id)));
        let blasts: Vec<&Record> = (self.records.iter())
            .filter(|r| r.damage.id == X_KAIROS)
            .collect();
        let mut volleys: Vec<Volley> = Vec::new();
        for (e, life, placed, at) in pellets {
            let same = |v: &Volley| (v.first.t - placed.t).abs() < 0.6 && distance(v.at, at) < 3.0;
            let index = match volleys.iter().position(same) {
                Some(index) => index,
                None => {
                    volleys.push(Volley {
                        first: placed,
                        at,
                        sum: [0.0; 3],
                        pellets: 0,
                        detonated: 0,
                        destroyed: 0,
                        records: Vec::new(),
                        dead: None,
                        owner: None,
                    });
                    volleys.len() - 1
                }
            };
            let Some(v) = volleys.get_mut(index) else {
                continue;
            };
            v.pellets += 1;
            v.sum = [0, 1, 2].map(|i| v.sum[i] + at[i]);
            v.owner = v.owner.or(self.player(life.owner));
            let hits: Vec<&Record> = (blasts.iter())
                .filter(|r| {
                    life.deleted.is_some_and(|d| (r.at.t - d.t).abs() < 0.25)
                        && r.impacts.iter().any(|i| distance(*i, at) < 0.2)
                })
                .copied()
                .collect();
            if !hits.is_empty() {
                v.detonated += 1;
                v.records.extend(hits);
            } else if let Some(dead) = life.dead {
                v.destroyed += 1;
                v.dead = v.dead.or(Some((e.id, life.armed, dead, at)));
            }
        }
        let hibanas: Vec<usize> = (self.players.iter().enumerate())
            .filter(|p| p.1.operator.name() == Some("Hibana"))
            .map(|p| p.0)
            .collect();
        for mut v in volleys {
            // Where the volley is: the middle of its pellets.
            let middle = v.sum.map(|s| s / f64::from(v.pellets.max(1)));
            let mut b = blank("X-KAIROS", v.first, middle);
            v.records.sort_by(|a, b| a.at.t.total_cmp(&b.at.t));
            b.pellets = Some(v.pellets);
            b.detonated = Some(v.detonated);
            b.destroyed = Some(v.destroyed);
            let first = v.records.first();
            if let Some(first) = first {
                b.ended = Some(when(first.at));
                b.affected = self.affected(&v.records);
                b.opened_reinforcement = Some(opens(&b.affected));
            }
            b.outcome = if v.detonated == v.pellets {
                Outcome::Detonated
            } else if v.detonated > 0 {
                Outcome::Partly
            } else if v.destroyed > 0 {
                Outcome::Destroyed
            } else {
                Outcome::None
            };
            if let Some((entity, armed, dead, at)) = v.dead {
                if first.is_none() {
                    b.ended = Some(when(dead));
                }
                let (near, stopped) = self.near(entity, armed, dead.t, at);
                b.near = near;
                b.stopped_by = stopped;
                b.stopped_by_source = stopped.map(|_| "proximity");
            }
            // Who: the body the records name, the pellets' owner, or the
            // round's only Hibana.
            let read = (v.records.first().and_then(|r| self.who(r).player)).or(v.owner);
            let player = read.or(match hibanas.as_slice() {
                [only] => Some(*only),
                _ => None,
            });
            b.username = self.username(player);
            b.username_source = (read.is_none() && player.is_some()).then_some("assumed");
            out.push((v.first.t, b));
        }

        // Maverick: runs of torch records of one player on one object.
        struct Run {
            first: At,
            last: At,
            object: u64,
            map: bool,
            player: Option<usize>,
            points: Vec<[f64; 3]>,
            /// The object was destroyed right after one of the records.
            gone: bool,
        }
        let mut runs: Vec<Run> = Vec::new();
        let mut latest: HashMap<(u64, u64), usize> = HashMap::new();
        for r in (self.records.iter()).filter(|r| TORCH.contains(&r.damage.id)) {
            let key = (r.damage.instigator, r.object);
            let current = (latest.get(&key).copied())
                .filter(|&i| runs.get(i).is_some_and(|run| r.at.t - run.last.t <= 2.0));
            let index = current.unwrap_or_else(|| {
                runs.push(Run {
                    first: r.at,
                    last: r.at,
                    object: r.object,
                    map: r.map,
                    player: self.who(r).player,
                    points: Vec::new(),
                    gone: false,
                });
                runs.len() - 1
            });
            latest.insert(key, index);
            if let Some(run) = runs.get_mut(index) {
                run.last = r.at;
                run.gone |= self.gone(r);
                run.points.extend(&r.impacts);
            }
        }
        for run in runs {
            let n = run.points.len().max(1) as f64;
            let mean = [0, 1, 2].map(|i| run.points.iter().map(|p| p[i]).sum::<f64>() / n);
            let extent = [0, 1, 2].map(|i| {
                let lo = run
                    .points
                    .iter()
                    .map(|p| p[i])
                    .fold(f64::INFINITY, f64::min);
                let hi = (run.points.iter().map(|p| p[i])).fold(f64::NEG_INFINITY, f64::max);
                round((hi - lo).max(0.0), 2)
            });
            // Opened during the run, or open before it.
            let (kind, kind_source) = self.kind(run.object, run.map);
            let cleared = self.cleared.get(&run.object);
            let cleared = |from: f64, to: f64| {
                cleared.is_some_and(|c| c.iter().any(|a| a.t >= from && a.t <= to))
            };
            let opened = cleared(run.first.t - 0.01, run.last.t + OPEN_WINDOW);
            let mut b = blank("Breaching Torch", run.first, mean);
            b.username = self.username(run.player);
            b.target = Some(hex(run.object));
            b.target_kind = Some(kind.name());
            b.reinforcement = kind.reinforced().then(|| hex(run.object));
            b.outcome = Outcome::Burned;
            b.ended = Some(when(run.last));
            b.points = Some(run.points.len());
            b.extent = Some(extent);
            b.opened_reinforcement = Some(opened && kind.reinforced());
            b.affected = vec![Affected {
                object: hex(run.object),
                kind,
                kind_source,
                opened,
                already_open: !opened && cleared(f64::NEG_INFINITY, run.first.t),
                destroyed: run.gone,
                reinforced_by: None,
            }];
            out.push((run.first.t, b));
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.into_iter().map(|b| b.1).collect()
    }
}

/// Reads destruction, surfaces and breaches from the world of a round.
/// `items` is each player's ability and gadget item as the HUD has them
/// ([`crate::loadout::hud_items`]).
pub(crate) fn decode(
    input: &Input,
    world: &World,
    header: &Header,
    items: &[[Option<u64>; 2]],
) -> Decoded {
    // A frame the recording has no time for takes the last one's.
    let end = input.clock.frame_times.last().copied().unwrap_or(0.0);
    let untimed = std::cell::Cell::new(0usize);
    let seconds = |frame: Option<u32>| {
        crate::world::seconds(input, frame).unwrap_or_else(|| {
            untimed.set(untimed.get() + 1);
            end
        })
    };
    let roles = (input.players.iter())
        .map(|p| header.teams.get(p.team_index).and_then(|t| t.role))
        .collect();
    let scene = Scene::read(world, input.players, roles, header.map.0, &seconds);
    let when = |at: At| world.when(input.clock, at.frame);

    // Debris and props knocking into the map are a third of the events of
    // a real round and say nothing of what a player did: they are counted
    // and left out. Their impacts are on no surface either.
    let mut destruction = scene.events(&when);
    let events = destruction.len();
    destruction.retain(|e| e.cause.category != Category::Physics || e.username.is_some());
    let surfaces = scene.surfaces(&when);
    let breaches = scene.breaches(&when, items);
    let mut out = Decoded {
        physics: events - destruction.len(),
        opened: scene.opened_reinforcements(),
        records: scene.records.len(),
        bullets: scene.records.iter().filter(|r| r.bullet).count(),
        preset: scene.preset,
        destruction,
        surfaces,
        breaches,
        warnings: Vec::new(),
    };
    let unknown: HashSet<u64> = (out.destruction.iter())
        .filter(|e| e.cause.name.is_none())
        .map(|e| e.cause.id)
        .collect();
    let ownerless = out.breaches.iter().filter(|b| b.username.is_none()).count();
    for (count, what) in [
        (
            untimed.get(),
            "moments in frames the recording has no time for",
        ),
        (
            unknown.len(),
            "damage ids of destruction events are not known",
        ),
        (ownerless, "breach devices without a known owner"),
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
    use crate::types::map_tables_kinds::MAP_OBJECT_KINDS;
    use crate::world::{Change, Kind as Created, Owner, Placed};

    const UPRIGHT: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    /// The slots a panel has, and some panel assets.
    const PANEL_SLOTS: [[u8; 4]; 2] = [[0xB4, 0xD9, 0x3E, 0x43], [0x2E, 0x4B, 0xCE, 0x49]];
    const HATCH_REINFORCEMENT: u64 = 406076330074;
    const CASTLE_PANEL: u64 = 361321226213;
    const WALL_REINFORCEMENT: u64 = 417911060317;
    const NARROW_WALL_REINFORCEMENT: u64 = 417911059814;
    const BODY: u64 = 0xF000_0001;
    const DEFENDER: u64 = 0xF000_0002;
    const WALL: u64 = 0x60_0000_0001;
    const FLOOR: u64 = 0x60_0000_0002;
    const FRAG: u64 = 34118943342;
    const RIFLE: u64 = 39471599164;
    const THERMITE: u64 = 39838030204;
    const CHARGE: u64 = 238373642425;
    const SHOCK_WIRE: u64 = 382651791603;

    /// Frames are a hundredth of a second.
    fn seconds(frame: Option<u32>) -> f64 {
        frame.map_or(0.0, |f| f64::from(f) / 100.0)
    }

    fn when(at: At) -> When {
        When {
            recording_time: Some(at.t),
            ..When::default()
        }
    }

    fn players() -> Vec<Player> {
        [("attacker", 11, 0), ("defender", 22, 1)]
            .map(|(name, id, team_index)| Player {
                id,
                username: name.to_string(),
                team_index,
                ..Player::default()
            })
            .to_vec()
    }

    fn object(id: u64) -> Entity {
        Entity {
            id,
            kind: Created::MapObject,
            classes: vec![DAMAGE],
            rotation: UPRIGHT,
            ..Entity::default()
        }
    }

    fn entity(id: u64, asset: u64, classes: &[[u8; 4]]) -> Entity {
        Entity {
            id,
            asset,
            classes: classes.to_vec(),
            position: [0.0, 0.0, -100.0],
            rotation: UPRIGHT,
            ..Entity::default()
        }
    }

    fn panel(id: u64, asset: u64) -> Entity {
        Entity {
            slots: PANEL_SLOTS.map(|s| (s, 1)).to_vec(),
            ..entity(id, asset, &[PLACED, DAMAGE])
        }
    }

    /// A record with one impact where it struck.
    fn record(kind: u8, instigator: u64, id: u64, point: [f32; 3]) -> Damage {
        Damage {
            kind,
            point,
            instigator,
            id,
            impacts: vec![Impact {
                position: point,
                normal: [0.0, 1.0, 0.0],
                ..Impact::default()
            }],
            ..Damage::default()
        }
    }

    fn hit(frame: u32, damage: Vec<Damage>) -> Change {
        Change {
            frame: Some(frame),
            damage,
            ..Change::default()
        }
    }

    fn moved(frame: u32, position: [f32; 3]) -> Change {
        Change {
            frame: Some(frame),
            position: Some(position),
            ..Change::default()
        }
    }

    fn live(frame: u32, live: u8) -> Change {
        Change {
            frame: Some(frame),
            live: Some(live),
            ..Change::default()
        }
    }

    /// A world of `entities`, with a body for each of the two players.
    fn world(entities: Vec<Entity>) -> World {
        let mut world = World::default();
        for e in entities {
            world.order.push(e.id);
            world.entities.insert(e.id, e);
        }
        world.players.insert(BODY, 0);
        world.players.insert(DEFENDER, 1);
        world
    }

    fn scene<'a>(world: &'a World, players: &'a [Player]) -> Scene<'a> {
        let roles = vec![Some(TeamRole::Attack), Some(TeamRole::Defense)];
        Scene::read(world, players, roles, 0, &seconds)
    }

    fn breaches(scene: &Scene) -> Vec<Breach> {
        scene.breaches(&when, &[])
    }

    #[test]
    fn records_of_one_cause_close_in_time_are_one_event() {
        let mut wall = object(WALL);
        wall.changes = vec![
            hit(100, vec![record(0, BODY, FRAG, [1.0, 0.0, 1.0])]),
            hit(140, vec![record(0, BODY, FRAG, [3.0, 0.0, 1.0])]),
        ];
        let mut floor = object(FLOOR);
        floor.position = [10.0, 0.0, 0.0];
        floor.changes = vec![
            hit(110, vec![record(0, BODY, FRAG, [2.0, 0.0, 0.0])]),
            // Another cause, and another instigator, in the same frame.
            hit(110, vec![record(0, BODY, MELEE, [0.0, 0.0, 0.0])]),
            hit(110, vec![record(0, DEFENDER, FRAG, [0.0, 0.0, 0.0])]),
        ];
        let (world, players) = (world(vec![wall, floor]), players());
        let events = scene(&world, &players).events(&when);
        let seen: Vec<_> = (events.iter())
            .map(|e| (e.cause.name, e.username.as_deref(), e.objects.len()))
            .collect();
        assert_eq!(
            seen,
            [
                (Some("Frag Grenade"), Some("attacker"), 2),
                (Some("Melee"), Some("attacker"), 1),
                (Some("Frag Grenade"), Some("defender"), 1),
                // 0.3 s after the record before it: a new event.
                (Some("Frag Grenade"), Some("attacker"), 1),
            ]
        );
        let first = &events[0];
        assert_eq!(first.when.recording_time, Some(1.0));
        assert_eq!(first.position, [1.0, 0.0, 1.0]);
        assert_eq!(first.cause.category, Category::Explosive);
        // The floor's point is placed with the floor's transform.
        assert_eq!(first.objects[1].object, hex(FLOOR));
        assert_eq!(first.objects[1].impacts, [[12.0, 0.0, 0.0]]);
    }

    #[test]
    fn a_chain_of_records_stays_one_event() {
        let mut wall = object(WALL);
        wall.changes = (0..5)
            .map(|i| hit(100 + 10 * i, vec![record(0, BODY, FRAG, [0.0; 3])]))
            .collect();
        let (world, players) = (world(vec![wall]), players());
        let events = scene(&world, &players).events(&when);
        // Each record is 0.1 s after the one before, 0.4 s in all.
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].objects[0].impacts.len(), 5);
    }

    #[test]
    fn the_two_ids_of_a_hard_breach_charge_are_one_event() {
        let mut wall = object(WALL);
        let both = HARD_BREACH.map(|id| record(0, 0xF000_0100, id, [0.0; 3]));
        wall.changes = vec![hit(100, both.to_vec())];
        let (world, players) = (world(vec![wall]), players());
        let events = scene(&world, &players).events(&when);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].cause.name, Some("Hard Breach Charge"));
        // An instigator the stream never created: no player, no asset.
        assert_eq!(events[0].instigator.as_deref(), Some("f0000100"));
        assert_eq!(events[0].username, None);
        assert_eq!(events[0].instigator_asset, None);
    }

    #[test]
    fn the_damage_a_map_starts_with_is_set_aside() {
        let preset = |frame: Option<u32>, full: bool, instigator: u64| Change {
            frame,
            full,
            damage: vec![record(0, instigator, FRAG, [0.0; 3])],
            ..Change::default()
        };
        let mut wall = object(WALL);
        wall.changes = vec![
            // In the full state of the first second, by nobody and by a
            // map object: what the map starts with.
            preset(None, true, 0),
            preset(None, true, FLOOR),
            // By a player, by the object itself, in an update, and later:
            // something that happened.
            preset(None, true, BODY),
            preset(None, true, WALL),
            preset(Some(50), false, 0),
            preset(Some(150), true, 0),
        ];
        let (world, players) = (world(vec![wall]), players());
        let scene = scene(&world, &players);
        assert_eq!(scene.preset, 2);
        let by: Vec<u64> = scene.records.iter().map(|r| r.damage.instigator).collect();
        assert_eq!(by, [BODY, WALL, 0, 0]);
    }

    #[test]
    fn a_confirmed_bullet_is_dropped_after_its_prediction() {
        let mut wall = object(WALL);
        wall.changes = vec![
            hit(100, vec![record(2, BODY, RIFLE, [1.0, 0.0, 1.0])]),
            hit(105, vec![record(3, BODY, RIFLE, [1.0, 0.0, 1.0])]),
            // Confirmed without a prediction: kept. Another player's: kept.
            hit(110, vec![record(3, BODY, RIFLE, [2.0, 0.0, 1.0])]),
            hit(120, vec![record(1, DEFENDER, RIFLE, [1.0, 0.0, 1.0])]),
        ];
        let (world, players) = (world(vec![wall]), players());
        let scene = scene(&world, &players);
        let kinds: Vec<u8> = scene.records.iter().map(|r| r.damage.kind).collect();
        assert_eq!(kinds, [2, 3, 1]);
        assert!(scene.records.iter().all(|r| r.bullet));
        // Bullets make no events.
        assert!(scene.events(&when).is_empty());
    }

    #[test]
    fn a_gadget_that_did_it_names_its_owner() {
        let mut grenade = entity(0xF000_0200, 5, &[OWNER]);
        grenade.changes = vec![Change {
            frame: Some(50),
            owner: Some(Owner {
                player: Some(22),
                ..Owner::default()
            }),
            ..Change::default()
        }];
        let mut wall = object(WALL);
        wall.changes = vec![hit(100, vec![record(0, 0xF000_0200, FRAG, [0.0; 3])])];
        let (world, players) = (world(vec![grenade, wall]), players());
        let events = scene(&world, &players).events(&when);
        assert_eq!(events[0].username.as_deref(), Some("defender"));
        assert_eq!(events[0].instigator.as_deref(), Some("f0000200"));
        assert_eq!(events[0].instigator_asset, Some(5));
    }

    #[test]
    fn a_panel_is_known_by_its_classes_slots_and_asset() {
        let kind = |asset: u64| entity_kind(&panel(1, asset));
        assert_eq!(kind(HATCH_REINFORCEMENT), Kind::ReinforcedHatch);
        assert_eq!(kind(CASTLE_PANEL), Kind::Barricade);
        assert_eq!(kind(WALL_REINFORCEMENT + 3 * 623), Kind::ReinforcedWall);
        assert_eq!(kind(NARROW_WALL_REINFORCEMENT), Kind::ReinforcedWall);
        assert_eq!(kind(WALL_REINFORCEMENT + 9 * 623), Kind::Entity);
        assert_eq!(kind(WALL_REINFORCEMENT + 1), Kind::Entity);
        // The asset alone does not make a panel.
        let plain = entity(1, HATCH_REINFORCEMENT, &[PLACED, DAMAGE]);
        assert_eq!(entity_kind(&plain), Kind::Entity);
    }

    #[test]
    fn impacts_say_what_an_unlisted_object_is() {
        let impact = |position: [f32; 3], normal: [f32; 3]| Impact {
            position,
            normal,
            ..Impact::default()
        };
        let q = wide(UPRIGHT);
        let floor = impact([3.0, 2.0, 0.01], [0.0, 0.0, 1.0]);
        assert_eq!(face(q, &floor), Face::Floor);
        let wall = impact([3.0, 0.1, 2.0], [0.0, -1.0, 0.0]);
        assert_eq!(face(q, &wall), Face::Wall);
        let along_x = impact([0.02, 1.0, 2.0], [1.0, 0.0, 0.0]);
        assert_eq!(face(q, &along_x), Face::Wall);
        // Off the plane, too high, or on an object that is tilted.
        let off = impact([3.0, 0.5, 2.0], [0.0, 1.0, 0.0]);
        assert_eq!(face(q, &off), Face::Other);
        let high = impact([3.0, 0.0, 7.0], [0.0, 1.0, 0.0]);
        assert_eq!(face(q, &high), Face::Other);
        assert_eq!(face([0.3, 0.0, 0.0, 0.95], &floor), Face::Other);

        // Four bullets in five on its y plane make it a wall; three do not.
        let shots = |on: usize| -> Vec<Damage> {
            (0..5)
                .map(|i| {
                    let y = if i < on { 0.0 } else { 0.5 };
                    record(1, BODY, RIFLE, [i as f32, y, 1.0])
                })
                .collect()
        };
        let mut wall = object(WALL);
        wall.changes = vec![hit(100, shots(4))];
        let mut other = object(FLOOR);
        other.changes = vec![hit(100, shots(3))];
        let (world, players) = (world(vec![wall, other]), players());
        let scene = scene(&world, &players);
        let by_impacts = Some(KindSource::Impacts);
        assert_eq!(scene.object_kind(WALL), (Kind::Wall, by_impacts));
        assert_eq!(scene.object_kind(FLOOR), (Kind::Object, None));
        // An object nothing struck is not known.
        assert_eq!(scene.object_kind(0x60_0000_0009), (Kind::Object, None));
    }

    #[test]
    fn a_hatch_is_known_by_the_reinforcement_put_on_it() {
        let mut hatch = object(FLOOR);
        hatch.changes = vec![
            hit(100, vec![record(0, 0, HATCH_PLACED, [0.0; 3])]),
            Change {
                frame: Some(100),
                destroyed: true,
                ..Change::default()
            },
        ];
        let (world, players) = (world(vec![hatch]), players());
        let scene = scene(&world, &players);
        let derived = Some(KindSource::Derived);
        assert_eq!(scene.object_kind(FLOOR), (Kind::Hatch, derived));
        let events = scene.events(&when);
        assert_eq!(events[0].cause.category, Category::Reinforcement);
        assert_eq!(events[0].objects[0].kind_source, derived);
        assert!(events[0].objects[0].destroyed);
    }

    #[test]
    fn the_catalog_names_a_map_object() {
        // Bank's first object of each kind.
        let bank = 413779563590;
        let of = |kind| {
            let objects = MAP_OBJECT_KINDS.iter().find(|m| m.0 == bank).unwrap().1;
            objects.iter().find(|o| o.1 == kind).unwrap().0
        };
        let (world, players) = (world(Vec::new()), players());
        let mut scene = scene(&world, &players);
        scene.map = bank;
        let listed = Some(KindSource::Catalog);
        let kind = |scene: &Scene, kind| scene.object_kind(of(kind));
        assert_eq!(kind(&scene, ObjectKind::Wall), (Kind::Wall, listed));
        assert_eq!(kind(&scene, ObjectKind::Floor), (Kind::Floor, listed));
        assert_eq!(kind(&scene, ObjectKind::Hatch), (Kind::Hatch, listed));
        assert_eq!(kind(&scene, ObjectKind::Breakable), (Kind::Object, listed));
        // On another map the same id is not known.
        scene.map = 1;
        assert_eq!(kind(&scene, ObjectKind::Wall), (Kind::Object, None));
    }

    /// Bullets of `by` on the wall, `columns` x `rows`, 0.3 m apart.
    fn sprayed(by: u64, columns: usize, rows: usize) -> Vec<Damage> {
        (0..columns * rows)
            .map(|i| {
                let x = (i % columns) as f32 * 0.3;
                let z = (i / columns) as f32 * 0.3;
                record(1, by, RIFLE, [x, 0.0, z])
            })
            .collect()
    }

    fn labels(changes: Vec<Change>) -> Vec<(Label, usize)> {
        let mut wall = object(WALL);
        wall.changes = changes;
        let (world, players) = (world(vec![wall]), players());
        let surfaces = scene(&world, &players).surfaces(&when);
        assert!(surfaces.iter().all(|s| s.derived && s.kind == Plane::Wall));
        surfaces.iter().map(|s| (s.label, s.points)).collect()
    }

    #[test]
    fn a_wide_patch_of_bullets_is_a_hole_of_the_side_that_made_it() {
        // 4 x 4 bullets span 0.9 x 0.9 m.
        let defender = labels(vec![hit(100, sprayed(DEFENDER, 4, 4))]);
        assert_eq!(defender, [(Label::RotationHole, 16)]);
        let attacker = labels(vec![hit(100, sprayed(BODY, 4, 4))]);
        assert_eq!(attacker, [(Label::Breach, 16)]);
        // Too low to walk through.
        let low = labels(vec![hit(100, sprayed(BODY, 6, 2))]);
        assert_eq!(low, [(Label::BulletHoles, 12)]);
    }

    #[test]
    fn small_clusters_are_murder_holes_or_left_out() {
        // 3 x 3 bullets within 0.6 m.
        let tight = sprayed(BODY, 3, 3).into_iter().map(|mut d| {
            d.point = d.point.map(|v| v * 0.5);
            d.impacts[0].position = d.point;
            d
        });
        let tight = labels(vec![hit(100, tight.collect())]);
        assert_eq!(tight, [(Label::MurderHole, 9)]);
        // Two stray bullets are no surface; a melee hit is one.
        let far = |x: f32| record(1, BODY, RIFLE, [x, 0.0, 1.0]);
        let stray = labels(vec![hit(100, vec![far(0.0), far(0.2), far(5.0)])]);
        assert_eq!(stray, []);
        let melee = record(0, BODY, MELEE, [0.0, 0.0, 1.0]);
        assert_eq!(
            labels(vec![hit(100, vec![melee])]),
            [(Label::MurderHole, 1)]
        );
        // An explosive cause makes a hole whatever its size, and points
        // further than 0.45 m apart are two surfaces. The two stray
        // bullets are what says the object is a wall.
        let frag = |x: f32| record(0, BODY, FRAG, [x, 0.0, 1.0]);
        let blast = vec![frag(0.0), frag(0.4), frag(1.0), far(20.0), far(22.0)];
        let blast = labels(vec![hit(100, blast)]);
        assert_eq!(blast, [(Label::Breach, 2), (Label::Breach, 1)]);
        // Without them it is an object of unknown kind: no surface.
        let unknown = labels(vec![hit(100, vec![frag(0.0), frag(0.4)])]);
        assert_eq!(unknown, []);
    }

    #[test]
    fn a_surface_counts_its_causes_and_makers() {
        let mut wall = object(WALL);
        wall.changes = vec![
            hit(100, sprayed(DEFENDER, 4, 4)),
            hit(300, vec![record(0, BODY, MELEE, [0.1, 0.0, 0.1])]),
        ];
        let (world, players) = (world(vec![wall]), players());
        let surfaces = scene(&world, &players).surfaces(&when);
        let [s] = surfaces.as_slice() else {
            panic!("{surfaces:?}");
        };
        assert_eq!(s.causes, BTreeMap::from([("Melee", 1), ("bullet", 16)]));
        let makers = [("attacker".to_string(), 1), ("defender".to_string(), 16)];
        assert_eq!(s.makers, BTreeMap::from(makers));
        assert_eq!(s.side, Some(TeamRole::Defense));
        assert_eq!((s.width, s.height), (0.9, 0.9));
        assert_eq!(s.position, [0.45, 0.0, 0.45]);
        assert_eq!((s.when.recording_time, s.until), (Some(1.0), 3.0));
        assert_eq!(s.objects, [hex(WALL)]);
    }

    #[test]
    fn a_reinforcement_keeps_only_what_opens_it() {
        const PANEL: u64 = 0xF000_0300;
        let mut wall = reinforcement(PANEL, WALL_REINFORCEMENT, [0.0; 3]);
        wall.changes.extend([
            hit(100, sprayed(BODY, 4, 4)),
            hit(110, vec![record(0, BODY, FRAG, [0.0, 0.0, 1.0])]),
            hit(120, vec![record(0, 0xF000_0100, THERMITE, [0.0, 0.0, 1.0])]),
        ]);
        let (world, players) = (world(vec![wall]), players());
        let surfaces = scene(&world, &players).surfaces(&when);
        let [s] = surfaces.as_slice() else {
            panic!("{surfaces:?}");
        };
        assert_eq!(s.causes, BTreeMap::from([("Exothermic Charge", 1)]));
        assert!(s.reinforced && !s.hatch);
        assert_eq!(s.label, Label::Breach);
    }

    /// A charge of the attacker: placed at frame 100 on `target`, armed
    /// at 200.
    fn charge(id: u64, target: u64, at: [f32; 3]) -> Entity {
        let mut e = entity(id, CHARGE, &[PLACED]);
        e.changes = vec![
            Change {
                placed: Some(Placed {
                    owner: Some(11),
                    host: Some(target),
                    ..Placed::default()
                }),
                ..moved(100, at)
            },
            live(200, 1),
        ];
        e
    }

    /// A reinforcement panel completed at frame 50.
    fn reinforcement(id: u64, asset: u64, at: [f32; 3]) -> Entity {
        let mut e = panel(id, asset);
        e.changes = vec![
            Change {
                full: true,
                flags: Some(0x8000),
                ..Change::default()
            },
            moved(10, at),
            live(50, 1),
        ];
        e
    }

    fn cleared(frame: u32, destroyed: bool) -> Change {
        Change {
            frame: Some(frame),
            flags: Some(0),
            destroyed,
            ..Change::default()
        }
    }

    fn time(when: Option<&When>) -> Option<f64> {
        when.and_then(|w| w.recording_time)
    }

    #[test]
    fn a_charge_that_goes_off_opens_its_reinforcement() {
        const PANEL: u64 = 0xF000_0300;
        const DEVICE: u64 = 0xF000_0301;
        let mut hatch = reinforcement(PANEL, HATCH_REINFORCEMENT, [5.0, 5.0, 3.0]);
        hatch.changes.extend([
            hit(400, vec![record(0, DEVICE, THERMITE, [0.0; 3])]),
            cleared(410, true),
        ]);
        let mut device = charge(DEVICE, PANEL, [5.0, 5.0, 3.0]);
        device.deleted = Some(400);
        let (world, players) = (world(vec![hatch, device]), players());
        let scene = scene(&world, &players);
        let breaches = breaches(&scene);
        let [b] = breaches.as_slice() else {
            panic!("{breaches:?}");
        };
        assert_eq!(b.device, "Exothermic Charge");
        assert_eq!(b.device_source, Some("table"));
        assert_eq!(b.username.as_deref(), Some("attacker"));
        assert_eq!(b.outcome, Outcome::Detonated);
        assert_eq!(b.target_kind, Some("reinforcedHatch"));
        assert_eq!(b.reinforcement.as_deref(), Some("f0000300"));
        assert_eq!(b.opened_reinforcement, Some(true));
        let times = [Some(&b.when), b.armed.as_ref(), b.ended.as_ref()];
        assert_eq!(times.map(time), [Some(1.0), Some(2.0), Some(4.0)]);
        let [o] = b.affected.as_slice() else {
            panic!("{b:?}");
        };
        assert_eq!(o.kind, Kind::ReinforcedHatch);
        assert_eq!((o.opened, o.already_open, o.destroyed), (true, false, true));
        assert_eq!(scene.opened_reinforcements(), [(PANEL, 410)]);
        // The event says the same of the object.
        let events = scene.events(&when);
        assert!(events[0].objects[0].opened && events[0].objects[0].destroyed);
    }

    #[test]
    fn a_charge_on_a_wall_finds_the_panel_that_holds_it() {
        const PANEL: u64 = 0xF000_0300;
        const DEVICE: u64 = 0xF000_0301;
        // The panel's x runs along the wall and y up: a charge 0.5 m along
        // and 1 m up is on it, one 2 m along is on the bare wall.
        let wall = reinforcement(PANEL, WALL_REINFORCEMENT, [5.0, 5.0, 0.0]);
        let on = charge(DEVICE, WALL, [5.5, 6.0, 0.2]);
        let off = charge(DEVICE + 1, WALL, [7.0, 6.0, 0.2]);
        let (world, players) = (world(vec![wall, on, off]), players());
        let breaches = breaches(&scene(&world, &players));
        let seen: Vec<_> = (breaches.iter())
            .map(|b| (b.target_kind, b.reinforcement.as_deref(), b.outcome))
            .collect();
        let armed = Outcome::ArmedAtEnd;
        assert_eq!(
            seen,
            [
                (Some("reinforcedWall"), Some("f0000300"), armed),
                (Some("mapObject"), None, armed),
            ]
        );
        assert!(breaches.iter().all(|b| b.target == Some(hex(WALL))));
    }

    /// A charge that dies 0.2 s after it is armed, with `others` around.
    fn died(others: Vec<Entity>) -> Breach {
        const DEVICE: u64 = 0xF000_0301;
        let mut device = charge(DEVICE, WALL, [5.0, 5.0, 1.0]);
        device.changes.push(live(220, 0));
        let mut entities = vec![device];
        entities.extend(others);
        let (world, players) = (world(entities), players());
        let breaches = breaches(&scene(&world, &players));
        let [b] = breaches.as_slice() else {
            panic!("{breaches:?}");
        };
        assert_eq!(b.outcome, Outcome::Destroyed);
        assert_eq!(time(b.ended.as_ref()), Some(2.2));
        assert!(b.affected.is_empty() && b.opened_reinforcement.is_none());
        assert_eq!(
            b.stopped_by.is_some(),
            b.stopped_by_source == Some("proximity")
        );
        b.clone()
    }

    /// A Shock Wire of the defender, live from frame 30.
    fn wire(x: f32) -> Entity {
        let mut e = entity(0xF000_0400, SHOCK_WIRE, &[PLACED]);
        e.changes = vec![
            Change {
                placed: Some(Placed {
                    owner: Some(22),
                    ..Placed::default()
                }),
                ..moved(20, [x, 5.0, 1.0])
            },
            live(30, 1),
        ];
        e
    }

    #[test]
    fn what_stopped_a_charge_is_named_only_from_what_was_near() {
        // Nothing near: nothing named.
        let alone = died(Vec::new());
        assert!(alone.near.is_empty());
        assert_eq!((alone.stopped_by, alone.stopped_by_source), (None, None));

        // A Shock Wire 1 m away; one 5 m away is not near.
        let far = died(vec![wire(10.0)]);
        assert!(far.near.is_empty() && far.stopped_by.is_none());
        let wired = died(vec![wire(6.0)]);
        assert_eq!(wired.stopped_by, Some(StoppedBy::Electricity));
        let [near] = wired.near.as_slice() else {
            panic!("{wired:?}");
        };
        assert_eq!(near.kind, NearKind::Gadget);
        assert_eq!(near.gadget, Some("Shock Wire"));
        assert_eq!(near.username.as_deref(), Some("defender"));
        assert_eq!(near.distance, 1.0);
    }

    #[test]
    fn a_shot_or_an_explosion_near_a_charge_that_died_is_listed() {
        // A bullet of a defender 0.5 m away at that moment; one of an
        // attacker and one a second later are not listed.
        let shot = |frame: u32, by: u64| {
            let mut wall = object(WALL);
            let bullet = record(1, by, RIFLE, [5.5, 5.0, 1.0]);
            wall.changes = vec![hit(frame, vec![bullet])];
            wall
        };
        let hit_by = died(vec![shot(215, DEFENDER)]);
        assert_eq!(hit_by.stopped_by, Some(StoppedBy::Shot));
        assert_eq!(hit_by.near[0].kind, NearKind::Shot);
        assert_eq!(hit_by.near[0].username.as_deref(), Some("defender"));
        assert_eq!(hit_by.near[0].distance, 0.5);
        assert_eq!(died(vec![shot(215, BODY)]).stopped_by, None);
        assert_eq!(died(vec![shot(320, DEFENDER)]).stopped_by, None);
        // The charge had just been armed: a wire comes before the shot.
        let both = died(vec![shot(215, DEFENDER), wire(6.0)]);
        assert_eq!(both.stopped_by, Some(StoppedBy::Electricity));
        assert_eq!(both.near.len(), 2);

        // An explosion 2 m away comes before the rest.
        let mut blast = object(FLOOR);
        let frag = record(0, DEFENDER, FRAG, [7.0, 5.0, 1.0]);
        blast.changes = vec![hit(225, vec![frag])];
        let blown = died(vec![blast, wire(6.0)]);
        assert_eq!(blown.stopped_by, Some(StoppedBy::Explosion));
        let kinds: Vec<_> = blown.near.iter().map(|n| (n.kind, n.gadget)).collect();
        assert_eq!(
            kinds,
            [
                (NearKind::Gadget, Some("Shock Wire")),
                (NearKind::Explosion, Some("Frag Grenade")),
            ]
        );
    }

    #[test]
    fn a_charge_taken_back_is_removed() {
        const DEVICE: u64 = 0xF000_0301;
        // Back to the pool before it is armed.
        let mut device = entity(DEVICE, CHARGE, &[PLACED]);
        let pool = [0.0, 0.0, -100.0];
        device.changes = vec![moved(100, [5.0, 5.0, 1.0]), moved(150, pool)];
        let (world, players) = (world(vec![device]), players());
        let breaches = breaches(&scene(&world, &players));
        assert_eq!(breaches[0].outcome, Outcome::Removed);
        assert_eq!(breaches[0].position, [5.0, 5.0, 1.0]);
        assert_eq!(time(breaches[0].ended.as_ref()), Some(1.5));
        // No owner was written: none is named.
        assert_eq!(breaches[0].username, None);
    }

    #[test]
    fn pellets_that_stick_together_are_one_volley() {
        const HATCH: u64 = 0xF000_0300;
        let pellet = |id: u64, frame: u32, x: f32, deleted: Option<u32>| {
            let mut e = entity(id, PELLET, &[OWNER]);
            e.changes = vec![moved(frame, [x, 5.0, 3.0])];
            e.deleted = deleted;
            e
        };
        let blast = |x: f32| record(0, BODY, X_KAIROS, [x, 5.0, 3.0]);
        let mut hatch = reinforcement(HATCH, HATCH_REINFORCEMENT, [0.0; 3]);
        hatch
            .changes
            .extend([hit(500, vec![blast(5.0), blast(5.3)]), cleared(505, false)]);
        let entities = vec![
            hatch,
            pellet(0xF000_0501, 100, 5.0, Some(500)),
            pellet(0xF000_0502, 110, 5.3, Some(500)),
            // Deleted with no record where it sat.
            pellet(0xF000_0503, 120, 5.6, Some(500)),
            // A second later: another volley, still there at the end.
            pellet(0xF000_0504, 220, 5.9, None),
        ];
        let (world, players) = (world(entities), players());
        let breaches = breaches(&scene(&world, &players));
        let seen: Vec<_> = (breaches.iter())
            .map(|b| (b.outcome, b.pellets, b.detonated, b.opened_reinforcement))
            .collect();
        assert_eq!(
            seen,
            [
                (Outcome::Partly, Some(3), Some(2), Some(true)),
                (Outcome::None, Some(1), Some(0), None),
            ]
        );
        // The body the records name; without records nobody is known.
        assert_eq!(breaches[0].username.as_deref(), Some("attacker"));
        assert_eq!(breaches[0].position, [5.3, 5.0, 3.0]);
        assert_eq!(breaches[1].username, None);
        assert_eq!(breaches[1].username_source, None);
    }

    #[test]
    fn torch_records_on_one_object_are_a_run() {
        const PANEL: u64 = 0xF000_0300;
        let burn = |x: f32| record(0, BODY, TORCH[0], [x, 0.0, 1.0]);
        let mut wall = reinforcement(PANEL, WALL_REINFORCEMENT, [0.0; 3]);
        wall.changes.extend([
            hit(100, vec![burn(0.0)]),
            hit(250, vec![burn(0.5)]),
            cleared(260, false),
            // More than 2 s later: another run, on a wall already open.
            hit(500, vec![burn(1.0)]),
        ]);
        let (world, players) = (world(vec![wall]), players());
        let breaches = breaches(&scene(&world, &players));
        let seen: Vec<_> = (breaches.iter())
            .map(|b| (b.outcome, b.points, b.opened_reinforcement, b.position))
            .collect();
        assert_eq!(
            seen,
            [
                (Outcome::Burned, Some(2), Some(true), [0.25, 0.0, 1.0]),
                (Outcome::Burned, Some(1), Some(false), [1.0, 0.0, 1.0]),
            ]
        );
        assert_eq!(breaches[0].extent, Some([0.5, 0.0, 0.0]));
        let affected = |b: &Breach| (b.affected[0].opened, b.affected[0].already_open);
        assert_eq!(affected(&breaches[0]), (true, false));
        assert_eq!(affected(&breaches[1]), (false, true));
        assert_eq!(breaches[0].target_kind, Some("reinforcedWall"));
        assert_eq!(time(breaches[0].ended.as_ref()), Some(2.5));
    }
}
