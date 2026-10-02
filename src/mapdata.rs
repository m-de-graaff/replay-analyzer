//! Map data per map id (Y11S3): floors, rooms, bomb sites, spawns, doors,
//! windows, hatches, walls and default cameras, as far as replays record
//! them, in one schema with what is drawn by hand.
//!
//! A replay holds no floor plan. What it does hold is every object of the
//! map a round touched, with the same id and place in every round on that
//! map, so a map fills in as replays are read: [`harvest`] makes a
//! [`MapData`] of some rounds, [`merge`] puts two together, and an app
//! keeps one JSON file per map id.
//!
//! # Where each thing is from
//!
//! Every element says so in `source`:
//!
//! - `read`: the file states it. A bomb, a camera, a wall a reinforcement
//!   named as its host, a door or window a barricade was on, a hatch.
//! - `derived`: worked out from what was read. Floors (the heights walls,
//!   hatches, doors and bombs stand on), spawn points (the median of where
//!   attackers stood), the stretch of a wall nobody reinforced (the reach
//!   of the impacts on it), where players walked.
//! - `authored`: drawn by hand. Room outlines, walls nothing can break,
//!   doorways nobody barricades. [`merge`] never changes or drops one.
//!
//! A field the file does not hold and nothing measures is listed in the
//! element's `assumed`: the width of a door, the top of a wall.
//!
//! # What a round holds
//!
//! The `movement` stream creates a map object when the round has
//! something to say of it (see [`crate::world`]): `627385fe`, 131 bytes,
//! with its id, position and rotation. [`map_objects`] reads those from a
//! round's decompressed bytes. A round has the map's cameras, its eight
//! bombs and the destructible objects that were damaged in it: 11 to 283,
//! 102 at the median, of the thousand and more a map has (187 rounds).
//! Nothing lists the rest, so the list grows with every round.
//!
//! A map object has no size. What gives one:
//!
//! - **A reinforcement** names the wall it is on (`host`, an object the
//!   stream never creates) and is put 0.1 m off the wall's middle plane,
//!   on the side of the player, at the foot of the wall and in the middle
//!   of its width. The width is the asset's ([`crate::panels`]).
//! - **A destructible wall** has its origin at its foot, its own x axis
//!   along the wall and y across it. Where it ends is not written; the
//!   impacts on it give the stretch that was hit. One whose origin is on
//!   a reinforced wall is that wall again ([`Wall::part_of`]).
//! - **A hatch** has its origin at a corner and is 2 m square: a hatch
//!   reinforcement lies at (1, 1) of the hatch under it.
//! - **A barricade** is put at the top of its door or window, in the
//!   middle, 0.125 m off the frame's plane. A door is 2.2 m high: 134 of
//!   140 door barricades are 2.2 m above a height a reinforced wall, a
//!   hatch or a bomb stands on.
//!
//! What a destructible object is (a wall, a floor panel, a hatch, a prop)
//! is not written either: it is the catalog's word, or that of the
//! round's destruction ([`crate::destruction`]), and an object neither
//! knows is a [`Piece`] of unknown kind.
//!
//! Rounds read without [`crate::ReadOptions::movement`] give everything
//! but [`MapData::walkable`]. [`harvest`] takes rounds alone and so has no
//! map objects: no walls but the reinforced ones, no bombs but the two in
//! play. [`harvest_with`] takes them with their objects ([`read`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use memchr::memmem;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::destruction::{Kind as DamagedKind, Plane};
use crate::movement::{Doing, Movement, Rope};
use crate::panels::{Opening as PanelOpening, ReinforcementKind};
use crate::summary::MapInfo;
use crate::types::map_tables_kinds::{ObjectKind, object_kind};
use crate::world::{DAMAGE, DEVICE, MAP_CREATE};
use crate::{ReadMode, ReadOptions, Result, Round, TeamRole, decompressed_bytes};

/// The version of the schema [`MapData`] is written in.
pub const SCHEMA: u32 = 1;

/// The class of a bomb, the one component of its map object.
const BOMB: [u8; 4] = [0xD0, 0xF6, 0x59, 0x29];
/// Bytes of a `627385fe` message, and where its fields are.
const MAP_CREATE_SIZE: usize = 131;
const CREATE_ID: usize = 4;
const CREATE_POSITION: usize = 16;
const CREATE_ROTATION: usize = 28;
const CREATE_CLASSES: usize = 53;
const CREATE_CLASS: usize = 57;

/// A reinforcement stands this far off the middle plane of its wall, and
/// a barricade this far off the plane of its frame (metres).
const REINFORCEMENT_OFFSET: f32 = 0.1;
const BARRICADE_OFFSET: f32 = 0.125;
/// A hatch is this wide and long.
const HATCH_SIZE: f32 = 2.0;
/// A barricade is at the top of its door, this far above the floor.
const DOOR_HEIGHT: f32 = 2.2;
/// Not in the file and not measured: how wide a door or a window is, how
/// high a window is, how high a wall is, and the width of a reinforced
/// wall whose asset the table does not know.
const OPENING_WIDTH: f32 = 1.2;
const WIDE_OPENING_WIDTH: f32 = 2.2;
const WINDOW_HEIGHT: f32 = 1.4;
const WALL_HEIGHT: f32 = 3.0;
const WALL_WIDTH: f32 = 2.0;
/// The top of a window is at most this far above its floor.
const WINDOW_TOP: f32 = 3.2;

/// Heights further apart than this are two floors: a storey is 3.4 m and
/// more, a split level 1.9 m at most.
const FLOOR_SPAN: f32 = 2.0;
/// A body this far below a floor still stands on it (steps, a ramp).
const FLOOR_BELOW: f32 = 0.6;
/// The storey above the top floor when the map has one floor.
const STOREY: f32 = 4.0;
/// A derived floor this close to an authored one is that floor.
const SAME_FLOOR: f32 = 1.0;

/// Two harvested things of a kind this close together are one (metres).
const SAME_PLACE: f32 = 0.35;
/// A destructible wall is part of a reinforced one when its origin is
/// this close to that wall's line, and no further past its ends.
const PART_ACROSS: f32 = 0.3;
const PART_ALONG: f32 = 0.2;
/// An impact counts for a wall's stretch when it is this close to its
/// plane, between these heights above its foot, and this near its origin.
const IMPACT_ACROSS: f32 = 0.35;
const IMPACT_BELOW: f32 = -0.5;
const IMPACT_ABOVE: f32 = 3.6;
const IMPACT_ALONG: f32 = 8.0;

/// Side of a cell of [`Walkable`], and the height step walked cells are
/// kept at until the floors are known (metres).
const CELL: f32 = 1.0;
const WALKED_STEP: f32 = 0.2;

/// Where an element is from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// The file states it.
    #[default]
    Read,
    /// Worked out from what the file states.
    Derived,
    /// Drawn by hand.
    Authored,
}

/// The map, as `summary.map` names it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MapRef {
    pub id: u64,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl From<MapInfo> for MapRef {
    fn from(m: MapInfo) -> Self {
        MapRef {
            id: m.id,
            name: m.name,
            base: m.base,
            version: m.version,
        }
    }
}

/// The box around everything the map data holds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub source: Source,
}

/// One storey.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Floor {
    /// Its place in [`MapData::floors`], lowest first. An index is not
    /// stable: a floor found later below this one moves it up.
    pub index: i32,
    /// `B`, `1F`, `2F`: the prefix of the names of the bomb sites and
    /// rooms on it. Absent when none was seen on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The height most of it stands on.
    pub z: f32,
    /// Where the floor above starts. Above the top floor this is a guess.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ceiling: Option<f32>,
    /// Every height things stand on that belongs to it: split levels.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<f32>,
    pub source: Source,
    #[serde(skip_serializing_if = "is_zero")]
    pub rounds: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub assumed: Vec<String>,
}

/// A room or callout. A harvested one has a name and a point in it
/// (`anchor`); its outline is drawn by hand.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Room {
    pub name: String,
    pub floor: i32,
    /// The outline, `[x, y]` corners in order. Empty until drawn.
    pub polygon: Vec<[f32; 2]>,
    /// A point the file names this room at: its bomb, or a Fenrir mine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<[f32; 3]>,
    pub source: Source,
    #[serde(skip_serializing_if = "is_zero")]
    pub rounds: u32,
}

/// One bomb of a site. The two of a site name each other in `partner`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Site {
    /// The header's name for it in the rounds it was played; absent for a
    /// bomb no round was played on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(with = "hex_option", skip_serializing_if = "Option::is_none")]
    pub object_id: Option<u64>,
    pub position: [f32; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor: Option<i32>,
    #[serde(with = "hex_option", skip_serializing_if = "Option::is_none")]
    pub partner: Option<u64>,
    pub source: Source,
    /// Rounds it was seen in.
    #[serde(skip_serializing_if = "is_zero")]
    pub rounds: u32,
    /// Rounds it was the objective of.
    #[serde(skip_serializing_if = "is_zero")]
    pub played: u32,
}

/// Where attackers start.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Spawn {
    pub name: String,
    /// The median of where the attackers who picked it first stood.
    pub position: [f32; 3],
    /// The median distance of those places from `position`, over the
    /// ground.
    pub radius: f32,
    pub source: Source,
    #[serde(skip_serializing_if = "is_zero")]
    pub rounds: u32,
    /// Attackers it is the median of.
    #[serde(skip_serializing_if = "is_zero")]
    pub players: u32,
}

/// A door or a window: an opening in a wall, seen through unless
/// something closes it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Opening {
    /// The two ends of the opening, `[x, y]`.
    pub a: [f32; 2],
    pub b: [f32; 2],
    /// Across the opening, with the first component that is not zero
    /// positive.
    pub normal: [f32; 2],
    pub bottom: f32,
    pub top: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor: Option<i32>,
    pub width: f32,
    /// One of the game's wide openings: a double door, a wide window.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub wide: bool,
    /// The frame a player's barricade named as its host.
    #[serde(with = "hex_option", skip_serializing_if = "Option::is_none")]
    pub object_id: Option<u64>,
    /// Rounds the map itself had it barricaded at the start: an opening
    /// to the outside in every mode but Quick Match.
    #[serde(skip_serializing_if = "is_zero")]
    pub default_rounds: u32,
    /// Nothing solid fills it. False for an authored one that is glazed.
    pub see_through: bool,
    pub source: Source,
    #[serde(skip_serializing_if = "is_zero")]
    pub rounds: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub assumed: Vec<String>,
}

/// A hatch: an opening in the floor of `floor`, down to the floor below.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Hatch {
    /// Its middle; `z` is the floor it is in.
    pub position: [f32; 3],
    /// Its four corners, `[x, y]`.
    pub corners: [[f32; 2]; 4],
    pub size: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor: Option<i32>,
    /// What a hatch reinforcement named as its host.
    #[serde(with = "hex_option", skip_serializing_if = "Option::is_none")]
    pub object_id: Option<u64>,
    /// The destructible hatch itself, as destruction names it.
    #[serde(with = "hex_option", skip_serializing_if = "Option::is_none")]
    pub panel_id: Option<u64>,
    /// Seen reinforced.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub reinforceable: bool,
    pub source: Source,
    #[serde(skip_serializing_if = "is_zero")]
    pub rounds: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub assumed: Vec<String>,
}

/// What a wall is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WallKind {
    /// It breaks, and was not seen reinforced.
    #[default]
    Soft,
    /// It breaks, and a reinforcement was seen on it.
    Reinforceable,
    /// Nothing breaks it. Authored only: the file has no such wall.
    Solid,
}

/// A stretch of wall, as a segment over the ground with a bottom and a
/// top.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Wall {
    /// The two ends, `[x, y]`, on the middle plane of the wall. Equal
    /// when only the wall's origin is known.
    pub a: [f32; 2],
    pub b: [f32; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor: Option<i32>,
    pub bottom: f32,
    pub top: f32,
    pub kind: WallKind,
    pub width: f32,
    /// The wall a reinforcement names, or the destructible wall itself.
    #[serde(with = "hex_option", skip_serializing_if = "Option::is_none")]
    pub object_id: Option<u64>,
    /// The destructible walls whose origin is on a reinforceable wall:
    /// what destruction names when that wall is shot or blown open.
    #[serde(with = "hex_list", skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<u64>,
    /// A destructible wall's own origin, `[x, y]`: at an end of it or in
    /// its middle, which differs from wall to wall.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<[f32; 2]>,
    /// The reinforceable wall this destructible wall is a part of: the
    /// same stretch of wall twice, so one of the two is to be left out of
    /// a test of what a wall hides.
    #[serde(with = "hex_option", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<u64>,
    /// Glass, a grille: it stops a body and not a line of sight.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub see_through: bool,
    pub source: Source,
    #[serde(skip_serializing_if = "is_zero")]
    pub rounds: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub assumed: Vec<String>,
}

/// A camera of the map.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Camera {
    #[serde(with = "hex_option", skip_serializing_if = "Option::is_none")]
    pub object_id: Option<u64>,
    pub position: [f32; 3],
    /// The quaternion `[x, y, z, w]` it is mounted with.
    pub rotation: [f32; 4],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor: Option<i32>,
    pub source: Source,
    #[serde(skip_serializing_if = "is_zero")]
    pub rounds: u32,
}

/// What a destructible map object that is no wall and no hatch is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PieceKind {
    /// A floor panel.
    Floor,
    /// A prop, a pane, a frame.
    Breakable,
    #[default]
    Unknown,
}

/// A destructible map object that is no wall and no hatch: its origin and
/// how it is turned, nothing of its size.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Piece {
    #[serde(with = "hex_option", skip_serializing_if = "Option::is_none")]
    pub object_id: Option<u64>,
    pub kind: PieceKind,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub source: Source,
    #[serde(skip_serializing_if = "is_zero")]
    pub rounds: u32,
}

/// Where players stood on one floor: a grid of cells [`Walkable::cell`]
/// metres wide. Cell `(i, j)` covers `x` from `i * cell` and `y` from
/// `j * cell`; `rows[r]` is the row `j = origin[1] + r`, its character
/// `c` the cell `i = origin[0] + c`, `#` where somebody stood.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Walkable {
    pub floor: i32,
    pub cell: f32,
    pub origin: [i32; 2],
    pub rows: Vec<String>,
    pub source: Source,
    #[serde(skip_serializing_if = "is_zero")]
    pub rounds: u32,
}

/// Everything known of one map.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MapData {
    pub schema: u32,
    pub map: MapRef,
    /// The rounds harvested into it, as `<match id>/<round number>`,
    /// sorted: an app skips a round that is listed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rounds: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Bounds>,
    pub floors: Vec<Floor>,
    pub rooms: Vec<Room>,
    pub sites: Vec<Site>,
    pub spawns: Vec<Spawn>,
    pub doors: Vec<Opening>,
    pub windows: Vec<Opening>,
    pub hatches: Vec<Hatch>,
    pub walls: Vec<Wall>,
    pub cameras: Vec<Camera>,
    /// Destructible objects that are no wall and no hatch.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pieces: Vec<Piece>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub walkable: Vec<Walkable>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// An object id as the other events write it: hex without a prefix.
mod hex_option {
    use super::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(id: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
        match id {
            Some(id) => s.serialize_str(&format!("{id:x}")),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
        let text = Option::<String>::deserialize(d)?;
        text.map(|t| u64::from_str_radix(&t, 16).map_err(serde::de::Error::custom))
            .transpose()
    }
}

mod hex_list {
    use super::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(ids: &[u64], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(ids.iter().map(|id| format!("{id:x}")))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
        let text = Vec::<String>::deserialize(d)?;
        text.iter()
            .map(|t| u64::from_str_radix(t, 16).map_err(serde::de::Error::custom))
            .collect()
    }
}

// ------------------------------------------------------------ map objects

/// What a map object is, by the class of its one component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapObjectKind {
    /// It has a damage list: a wall, a floor, a hatch, a prop.
    Destructible,
    /// A camera of the map.
    Camera,
    /// A bomb. A map has two per site.
    Bomb,
    Other([u8; 4]),
}

/// A map object as its `627385fe` message creates it. Its id and place
/// are the same in every round on the map.
#[derive(Clone, Debug, PartialEq)]
pub struct MapObject {
    pub id: u64,
    pub kind: MapObjectKind,
    /// Metres, z up.
    pub position: [f32; 3],
    /// The quaternion `[x, y, z, w]`.
    pub rotation: [f32; 4],
}

/// Every map object the round created, from its decompressed bytes
/// ([`decompressed_bytes`]), in the order of the file. A message is taken
/// when it is 131 bytes, names its object before and after its type, and
/// has one class; none is in a file before Y11S3.
pub fn map_objects(data: &[u8]) -> Vec<MapObject> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for at in memmem::find_iter(data, &MAP_CREATE) {
        let Some(msg) = data.get(at..at + MAP_CREATE_SIZE) else {
            continue;
        };
        let (Some(id), Some(1)) = (u64_at(msg, CREATE_ID), u32_at(msg, CREATE_CLASSES)) else {
            continue;
        };
        // The message's own header: `u64 object, u32 size`.
        let named = at >= 12
            && u64_at(data, at - 12) == Some(id)
            && u32_at(data, at - 4) == Some(MAP_CREATE_SIZE as u32);
        if !named || seen.contains(&id) {
            continue;
        }
        let floats = |from: usize, n: usize| -> Option<Vec<f32>> {
            let v: Vec<f32> = (0..n).filter_map(|i| f32_at(msg, from + 4 * i)).collect();
            (v.len() == n && v.iter().all(|f| f.is_finite())).then_some(v)
        };
        let (Some(p), Some(q)) = (floats(CREATE_POSITION, 3), floats(CREATE_ROTATION, 4)) else {
            continue;
        };
        let Some(class) = msg.get(CREATE_CLASS..CREATE_CLASS + 4) else {
            continue;
        };
        let class = [class[0], class[1], class[2], class[3]];
        seen.insert(id);
        out.push(MapObject {
            id,
            kind: match class {
                DAMAGE => MapObjectKind::Destructible,
                DEVICE => MapObjectKind::Camera,
                BOMB => MapObjectKind::Bomb,
                other => MapObjectKind::Other(other),
            },
            position: [p[0], p[1], p[2]],
            rotation: [q[0], q[1], q[2], q[3]],
        });
    }
    out
}

fn u64_at(d: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(at..at + 8)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn f32_at(d: &[u8], at: usize) -> Option<f32> {
    Some(f32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// A round with the map objects its file created.
#[derive(Clone, Debug)]
pub struct Observed {
    pub round: Round,
    pub objects: Vec<MapObject>,
}

/// Reads a round file for [`harvest_with`]: a full read with the movement,
/// and the map objects.
pub fn read(path: impl AsRef<Path>) -> Result<Observed> {
    let raw = std::fs::read(path)?;
    read_bytes(&raw)
}

/// [`read`] of a file already in memory.
pub fn read_bytes(raw: &[u8]) -> Result<Observed> {
    let options = ReadOptions {
        mode: ReadMode::Full,
        census: false,
        movement: true,
    };
    let round = Round::from_bytes(raw, options)?;
    let objects = map_objects(&decompressed_bytes(raw)?);
    Ok(Observed { round, objects })
}

// ----------------------------------------------------------------- maths

/// Thousandths of a metre: what places are counted and compared in.
type Mm = i32;

fn mm(v: f32) -> Mm {
    (v * 1000.0).round() as Mm
}

fn metres(v: Mm) -> f32 {
    v as f32 / 1000.0
}

fn mm3(p: [f32; 3]) -> [Mm; 3] {
    [mm(p[0]), mm(p[1]), mm(p[2])]
}

fn metres3(p: [Mm; 3]) -> [f32; 3] {
    [metres(p[0]), metres(p[1]), metres(p[2])]
}

/// To the centimetre.
fn cm(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

/// To the millimetre, and no `-0.0`.
fn tidy(v: f32) -> f32 {
    metres(mm(v))
}

fn tidy2(p: [f32; 2]) -> [f32; 2] {
    [tidy(p[0]), tidy(p[1])]
}

fn tidy3(p: [f32; 3]) -> [f32; 3] {
    [tidy(p[0]), tidy(p[1]), tidy(p[2])]
}

/// `v` turned by the quaternion `q`.
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = q;
    let t = [
        2.0 * (y * v[2] - z * v[1]),
        2.0 * (z * v[0] - x * v[2]),
        2.0 * (x * v[1] - y * v[0]),
    ];
    [
        v[0] + w * t[0] + (y * t[2] - z * t[1]),
        v[1] + w * t[1] + (z * t[0] - x * t[2]),
        v[2] + w * t[2] + (x * t[1] - y * t[0]),
    ]
}

fn inverse(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

/// The unit vector of `v` over the ground; `None` for one that points up.
fn unit2(v: [f32; 2]) -> Option<[f32; 2]> {
    let len = v[0].hypot(v[1]);
    (len > 1e-3).then(|| [v[0] / len, v[1] / len])
}

/// `n` or its opposite: the one whose first component that is not zero is
/// positive, so the two sides of a wall give one normal.
fn canonical(n: [f32; 2]) -> [f32; 2] {
    let flip = n[0] < -1e-3 || (n[0].abs() <= 1e-3 && n[1] < 0.0);
    if flip { [-n[0], -n[1]] } else { n }
}

fn distance2(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn distance3(a: [f32; 3], b: [f32; 3]) -> f32 {
    distance2([a[0], a[1]], [b[0], b[1]]).hypot(a[2] - b[2])
}

/// Metres from `p` to the segment `a`-`b`, over the ground.
fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len2 = d[0] * d[0] + d[1] * d[1];
    if len2 < 1e-9 {
        return distance2(p, a);
    }
    let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len2).clamp(0.0, 1.0);
    distance2(p, [a[0] + t * d[0], a[1] + t * d[1]])
}

fn middle(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
}

/// Whether `p` is inside `polygon` (even-odd).
fn inside(polygon: &[[f32; 2]], p: [f32; 2]) -> bool {
    let mut hit = false;
    let n = polygon.len();
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + n - 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            hit = !hit;
        }
    }
    n >= 3 && hit
}

fn median(values: &mut [Mm]) -> Mm {
    values.sort_unstable();
    values.get(values.len() / 2).copied().unwrap_or(0)
}

/// Counts of a value; the winner is the most seen, the smallest of those.
#[derive(Clone, Debug)]
struct Votes<K: Ord>(BTreeMap<K, u32>);

impl<K: Ord> Default for Votes<K> {
    fn default() -> Self {
        Votes(BTreeMap::new())
    }
}

impl<K: Ord + Clone> Votes<K> {
    fn add(&mut self, key: K) {
        *self.0.entry(key).or_default() += 1;
    }

    fn add_all(&mut self, other: &Votes<K>) {
        for (key, n) in &other.0 {
            *self.0.entry(key.clone()).or_default() += n;
        }
    }

    fn winner(&self) -> Option<K> {
        let most = self.0.values().max()?;
        self.0.iter().find(|v| v.1 == most).map(|v| v.0.clone())
    }
}

/// `B`, `1F`, `2F`: the floor a site or room name starts with.
fn floor_prefix(name: &str) -> Option<&str> {
    let first = name.split_whitespace().next()?;
    let storey = first
        .strip_suffix('F')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    let basement = first
        .strip_prefix('B')
        .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()));
    (storey || basement).then_some(first)
}

// --------------------------------------------------------------- harvest

/// A reinforced wall or hatch: the host a reinforcement names, or the
/// place of one the map put up, which names none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PanelKey {
    Host(u64),
    At([Mm; 3]),
}

impl PanelKey {
    fn host(self) -> Option<u64> {
        match self {
            PanelKey::Host(host) => Some(host),
            PanelKey::At(_) => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct PanelAcc {
    /// `(middle of the wall's foot, normal, width)`.
    places: Votes<([Mm; 3], [Mm; 2], Option<Mm>)>,
    /// The rounds it was seen in, by their number in the harvest.
    seen: BTreeSet<u32>,
}

#[derive(Clone, Debug, Default)]
struct OpeningAcc {
    /// `(door, wide)`.
    kinds: Votes<(bool, bool)>,
    normals: Votes<[Mm; 2]>,
    hosts: Votes<u64>,
    rounds: u32,
    default_rounds: u32,
}

#[derive(Clone, Debug, Default)]
struct BombAcc {
    created: Option<[Mm; 3]>,
    positions: Votes<[Mm; 3]>,
    names: Votes<String>,
    partners: Votes<u64>,
    rounds: u32,
    played: u32,
}

#[derive(Clone, Debug, Default)]
struct CameraAcc {
    places: Votes<([Mm; 3], [Mm; 4])>,
    rounds: u32,
}

#[derive(Clone, Debug, Default)]
struct ObjectAcc {
    /// `(position, rotation)`.
    place: Option<([Mm; 3], [f32; 4])>,
    kinds: Votes<u8>,
    /// Points that struck it, and stretches of it that were struck: a
    /// middle and half a width.
    impacts: BTreeSet<[Mm; 3]>,
    patches: BTreeSet<([Mm; 3], Mm)>,
    seen: BTreeSet<u32>,
}

/// What an object is said to be, the one that counts first: an object a
/// round takes for a wall is a wall, whatever other rounds make of it.
/// So a harvest of many rounds and a merge of theirs agree.
const KIND_WALL: u8 = 0;
const KIND_HATCH: u8 = 1;
const KIND_FLOOR: u8 = 2;
const KIND_BREAKABLE: u8 = 3;

/// Takes rounds of one map one at a time and makes its [`MapData`]. The
/// result does not depend on the order the rounds are added in.
#[derive(Clone, Debug, Default)]
pub struct Harvester {
    map: Option<u64>,
    keys: BTreeSet<String>,
    walls: BTreeMap<PanelKey, PanelAcc>,
    hatches: BTreeMap<PanelKey, PanelAcc>,
    openings: BTreeMap<[Mm; 3], OpeningAcc>,
    bombs: BTreeMap<u64, BombAcc>,
    cameras: BTreeMap<u64, CameraAcc>,
    objects: BTreeMap<u64, ObjectAcc>,
    /// Per spawn name: where each attacker stood, and the rounds.
    spawns: BTreeMap<String, (Vec<[Mm; 3]>, u32)>,
    /// Per room name: where the file named it, and the rounds.
    callouts: BTreeMap<String, (Vec<[Mm; 3]>, u32)>,
    /// Cells somebody stood in: `(height step, x cell, y cell)`.
    walked: BTreeSet<(i32, i32, i32)>,
    walked_rounds: u32,
}

impl Harvester {
    pub fn new() -> Self {
        Self::default()
    }

    /// The map of the rounds added so far.
    pub fn map(&self) -> Option<u64> {
        self.map
    }

    /// Adds a round and the map objects of its file (empty when they were
    /// not read). False, and nothing added, for a round of another map
    /// than the first one's, and for a round already added.
    pub fn add(&mut self, round: &Round, objects: &[MapObject]) -> bool {
        let map = round.header.map.0;
        if *self.map.get_or_insert(map) != map {
            return false;
        }
        if !self.keys.insert(round_key(round)) {
            return false;
        }
        self.add_panels(round);
        self.add_barricades(round);
        self.add_objective(round);
        self.add_objects(round, objects);
        self.add_names(round);
        if let Some(movement) = &round.movement {
            self.add_movement(movement);
        }
        true
    }

    fn add_panels(&mut self, round: &Round) {
        let number = self.keys.len() as u32;
        for r in &round.reinforcements {
            // One that was called off has no host and may be on no wall.
            if r.cancelled || (r.completed.is_none() && !r.default) {
                continue;
            }
            let wall = r.kind == ReinforcementKind::Wall;
            let (place, normal) = if wall {
                let Some(n) = unit2([r.normal[0], r.normal[1]]) else {
                    continue;
                };
                // To the centimetre: the two sides of a wall are 0.2 m
                // apart to a millimetre or two.
                let p = [
                    cm(r.position[0] - REINFORCEMENT_OFFSET * n[0]),
                    cm(r.position[1] - REINFORCEMENT_OFFSET * n[1]),
                    r.position[2],
                ];
                let n = canonical(n);
                (mm3(p), [mm(n[0]), mm(n[1])])
            } else {
                // A hatch reinforcement lies 0 to 0.02 m under the floor.
                let z = (r.position[2] * 10.0).round() / 10.0;
                (mm3([r.position[0], r.position[1], z]), [0, 0])
            };
            let key = match r.host {
                Some(host) => PanelKey::Host(host),
                None => PanelKey::At(place),
            };
            let list = if wall {
                &mut self.walls
            } else {
                &mut self.hatches
            };
            let acc = list.entry(key).or_default();
            acc.places.add((place, normal, r.width.map(mm)));
            acc.seen.insert(number);
        }
    }

    fn add_barricades(&mut self, round: &Round) {
        let mut seen = BTreeSet::new();
        for b in &round.barricades {
            let (Some(opening), false) = (b.opening, b.cancelled) else {
                continue;
            };
            let Some(n) = unit2([b.normal[0], b.normal[1]]) else {
                continue;
            };
            let p = [
                cm(b.position[0] - BARRICADE_OFFSET * n[0]),
                cm(b.position[1] - BARRICADE_OFFSET * n[1]),
                b.position[2],
            ];
            let n = canonical(n);
            let key = mm3(p);
            let acc = self.openings.entry(key).or_default();
            let door = opening == PanelOpening::Door;
            acc.kinds.add((door, b.wide.unwrap_or(false)));
            acc.normals.add([mm(n[0]), mm(n[1])]);
            if let Some(host) = b.host {
                acc.hosts.add(host);
            }
            if seen.insert((key, false)) {
                acc.rounds += 1;
            }
            if b.default && seen.insert((key, true)) {
                acc.default_rounds += 1;
            }
        }
    }

    /// The ids of the round's two bombs.
    fn played(round: &Round) -> Vec<u64> {
        (round.objective_state.iter())
            .filter_map(|o| o.bomb.as_ref())
            .flat_map(|b| &b.sites)
            .filter_map(|s| u64::from_str_radix(&s.object, 16).ok())
            .collect()
    }

    fn add_objective(&mut self, round: &Round) {
        let Some(bomb) = round.objective_state.as_ref().and_then(|o| o.bomb.as_ref()) else {
            return;
        };
        let ids = Self::played(round);
        for site in &bomb.sites {
            let Ok(id) = u64::from_str_radix(&site.object, 16) else {
                continue;
            };
            let acc = self.bombs.entry(id).or_default();
            acc.positions.add(mm3(site.position));
            acc.played += 1;
            acc.rounds += 1;
            if let Some(name) = &site.name {
                acc.names.add(squeeze(name));
            }
            for other in ids.iter().filter(|o| **o != id) {
                acc.partners.add(*other);
            }
        }
    }

    fn add_objects(&mut self, round: &Round, objects: &[MapObject]) {
        let map = round.header.map.0;
        let played = Self::played(round);
        for c in &round.map_cameras {
            let acc = self.cameras.entry(c.object).or_default();
            let p = c.position.map(|v| v as f32);
            acc.places.add((mm3(p), c.rotation.map(|v| mm(v as f32))));
            acc.rounds += 1;
        }
        for o in objects {
            match o.kind {
                MapObjectKind::Bomb => {
                    let acc = self.bombs.entry(o.id).or_default();
                    acc.created = Some(mm3(o.position));
                    // One in play was counted with the objective.
                    if !played.contains(&o.id) {
                        acc.rounds += 1;
                    }
                }
                MapObjectKind::Destructible => {
                    let acc = self.objects.entry(o.id).or_default();
                    acc.place = Some((mm3(o.position), o.rotation));
                    acc.seen.insert(self.keys.len() as u32);
                    match object_kind(map, o.id) {
                        Some(ObjectKind::Wall) => acc.kinds.add(KIND_WALL),
                        Some(ObjectKind::Floor) => acc.kinds.add(KIND_FLOOR),
                        Some(ObjectKind::Hatch) => acc.kinds.add(KIND_HATCH),
                        Some(ObjectKind::Breakable) => acc.kinds.add(KIND_BREAKABLE),
                        None => {}
                    }
                }
                // The cameras are in `map_cameras`.
                MapObjectKind::Camera | MapObjectKind::Other(_) => {}
            }
        }
        // What destruction says of an object the catalog does not have,
        // and where each was struck.
        for d in round.destruction.iter().flat_map(|d| &d.objects) {
            let kind = match d.kind {
                DamagedKind::Wall => KIND_WALL,
                DamagedKind::Floor => KIND_FLOOR,
                DamagedKind::Hatch => KIND_HATCH,
                _ => continue,
            };
            let Ok(id) = u64::from_str_radix(&d.object, 16) else {
                continue;
            };
            let acc = self.objects.entry(id).or_default();
            if object_kind(map, id).is_none() {
                acc.kinds.add(kind);
            }
            for p in &d.impacts {
                acc.impacts.insert(mm3(p.map(|v| v as f32)));
            }
        }
        for s in &round.surfaces {
            let ([object], Plane::Wall) = (s.objects.as_slice(), s.kind) else {
                continue;
            };
            let Ok(id) = u64::from_str_radix(object, 16) else {
                continue;
            };
            let acc = self.objects.entry(id).or_default();
            if object_kind(map, id).is_none() {
                acc.kinds.add(KIND_WALL);
            }
            let at = mm3(s.position.map(|v| v as f32));
            acc.patches.insert((at, mm(s.width as f32 / 2.0)));
        }
    }

    /// Spawn names with where their attackers stood, and the rooms the
    /// file names at a point.
    fn add_names(&mut self, round: &Round) {
        let header = &round.header;
        let mut seen = BTreeSet::new();
        for p in &header.players {
            let attacker =
                (header.teams.get(p.team_index)).is_some_and(|t| t.role == Some(TeamRole::Attack));
            let (Some(position), true, false) = (p.spawn_position, attacker, p.spawn.is_empty())
            else {
                continue;
            };
            let name = squeeze(&p.spawn);
            let acc = self.spawns.entry(name.clone()).or_default();
            acc.0.push(mm3(position));
            if seen.insert(name) {
                acc.1 += 1;
            }
        }
        let mut seen = BTreeSet::new();
        for t in round.gadget_events.iter().flat_map(|g| &g.traps) {
            let (Some(name), Some(position)) = (&t.trigger.location, t.trigger.position) else {
                continue;
            };
            let name = squeeze(name);
            if name.is_empty() {
                continue;
            }
            let acc = self.callouts.entry(name.clone()).or_default();
            acc.0.push(mm3(position));
            if seen.insert(name) {
                acc.1 += 1;
            }
        }
    }

    /// The cells players stood in: not in the air, not on a rope, not
    /// vaulting, not dead.
    fn add_movement(&mut self, movement: &Movement) {
        let mut any = false;
        let cell = |v: f32, size: f32| (v / size).floor() as i32;
        for t in &movement.players {
            let (mut air, mut rope, mut doing) = (0, 0, 0);
            for (i, time) in t.time.iter().enumerate() {
                while t.airborne.get(air + 1).is_some_and(|c| c.time <= *time) {
                    air += 1;
                }
                while t.rope.get(rope + 1).is_some_and(|c| c.time <= *time) {
                    rope += 1;
                }
                while t.doing.get(doing + 1).is_some_and(|c| c.time <= *time) {
                    doing += 1;
                }
                let flying = (t.airborne.get(air)).is_some_and(|c| c.time <= *time && c.value);
                let roped =
                    (t.rope.get(rope)).is_some_and(|c| c.time <= *time && c.value != Rope::Off);
                let busy = t.doing.get(doing).is_some_and(|c| {
                    c.time <= *time
                        && matches!(c.value, Doing::Vaulting | Doing::Dead | Doing::Rappelling)
                });
                let (Some(x), Some(y), Some(z)) = (t.x.get(i), t.y.get(i), t.z.get(i)) else {
                    continue;
                };
                if flying || roped || busy || !(x.is_finite() && y.is_finite() && z.is_finite()) {
                    continue;
                }
                let step = cell(*z + WALKED_STEP / 2.0, WALKED_STEP);
                self.walked.insert((step, cell(*x, CELL), cell(*y, CELL)));
                any = true;
            }
        }
        self.walked_rounds += u32::from(any);
    }

    /// The map data of the rounds added. Empty but for the map when none
    /// was.
    pub fn finish(&self) -> MapData {
        let mut data = MapData {
            schema: SCHEMA,
            map: MapRef::from(MapInfo::new(crate::Map(self.map.unwrap_or(0)))),
            rounds: self.keys.iter().cloned().collect(),
            ..MapData::default()
        };
        self.finish_walls(&mut data);
        self.finish_openings(&mut data);
        let mut hatches = self.finish_hatches();
        self.finish_objects(&mut data, &mut hatches);
        // A hatch was seen in the rounds it was reinforced or damaged in.
        data.hatches = (hatches.into_iter())
            .map(|(hatch, seen)| Hatch {
                rounds: seen.len() as u32,
                ..hatch
            })
            .collect();
        self.finish_named(&mut data);
        data.rebuild();
        self.finish_walked(&mut data);
        data.rebuild();
        data
    }

    fn finish_walls(&self, data: &mut MapData) {
        // Those that name their wall come first: one the map put up names
        // none, and is the wall a player's reinforcement named there.
        let mut walls: Vec<(Wall, BTreeSet<u32>)> = Vec::new();
        for (key, acc) in &self.walls {
            let Some((place, normal, width)) = acc.places.winner() else {
                continue;
            };
            let (p, n) = (metres3(place), [metres(normal[0]), metres(normal[1])]);
            let half = width.map_or(WALL_WIDTH, metres) / 2.0;
            let along = [-n[1], n[0]];
            let mut assumed = vec!["top".to_owned()];
            if width.is_none() {
                assumed.push("width".to_owned());
            }
            let wall = Wall {
                a: tidy2([p[0] - along[0] * half, p[1] - along[1] * half]),
                b: tidy2([p[0] + along[0] * half, p[1] + along[1] * half]),
                floor: None,
                bottom: tidy(p[2]),
                top: tidy(p[2] + WALL_HEIGHT),
                kind: WallKind::Reinforceable,
                width: tidy(half * 2.0),
                object_id: key.host(),
                parts: Vec::new(),
                origin: None,
                part_of: None,
                see_through: false,
                source: Source::Read,
                rounds: 0,
                assumed,
            };
            let named = walls
                .iter_mut()
                .find(|w| key.host().is_none() && same_wall(&w.0, &wall));
            match named {
                Some(known) => known.1.extend(&acc.seen),
                None => walls.push((wall, acc.seen.clone())),
            }
        }
        data.walls
            .extend(walls.into_iter().map(|(wall, seen)| Wall {
                rounds: seen.len() as u32,
                ..wall
            }));
    }

    fn finish_openings(&self, data: &mut MapData) {
        // Places a hand's width apart are one opening: the one seen most.
        let mut groups: Vec<([Mm; 3], OpeningAcc)> = Vec::new();
        for (key, acc) in &self.openings {
            let near = groups
                .iter_mut()
                .find(|g| distance3(metres3(g.0), metres3(*key)) <= SAME_PLACE);
            let Some(group) = near else {
                groups.push((*key, acc.clone()));
                continue;
            };
            if acc.rounds > group.1.rounds {
                group.0 = *key;
            }
            group.1.kinds.add_all(&acc.kinds);
            group.1.normals.add_all(&acc.normals);
            group.1.hosts.add_all(&acc.hosts);
            group.1.rounds = group.1.rounds.max(acc.rounds);
            group.1.default_rounds = group.1.default_rounds.max(acc.default_rounds);
        }
        for (key, acc) in groups {
            let (Some((door, wide)), Some(normal)) = (acc.kinds.winner(), acc.normals.winner())
            else {
                continue;
            };
            let (p, n) = (metres3(key), [metres(normal[0]), metres(normal[1])]);
            let width = if wide {
                WIDE_OPENING_WIDTH
            } else {
                OPENING_WIDTH
            };
            let along = [-n[1] * width / 2.0, n[0] * width / 2.0];
            let height = if door { DOOR_HEIGHT } else { WINDOW_HEIGHT };
            let mut assumed = vec!["width".to_owned()];
            if !door {
                assumed.push("bottom".to_owned());
            }
            let opening = Opening {
                a: tidy2([p[0] - along[0], p[1] - along[1]]),
                b: tidy2([p[0] + along[0], p[1] + along[1]]),
                normal: tidy2(n),
                bottom: tidy(p[2] - height),
                top: tidy(p[2]),
                floor: None,
                width,
                wide,
                object_id: acc.hosts.winner(),
                default_rounds: acc.default_rounds,
                see_through: true,
                source: Source::Read,
                rounds: acc.rounds,
                assumed,
            };
            if door {
                data.doors.push(opening);
            } else {
                data.windows.push(opening);
            }
        }
    }

    /// The hatches a reinforcement was on, each with the rounds it was
    /// seen in.
    fn finish_hatches(&self) -> Vec<(Hatch, BTreeSet<u32>)> {
        let mut out: Vec<(Hatch, BTreeSet<u32>)> = Vec::new();
        // Those that name their hatch first: one the map put up names
        // none, and is the hatch a player's reinforcement named there.
        for (key, acc) in &self.hatches {
            let Some((place, _, _)) = acc.places.winner() else {
                continue;
            };
            let hatch = Hatch {
                object_id: key.host(),
                reinforceable: true,
                // Until the hatch itself says how it is turned.
                assumed: vec!["corners".to_owned()],
                ..hatch_at(metres3(place), [1.0, 0.0])
            };
            match out.iter_mut().find(|h| same_hatch(&h.0, &hatch)) {
                Some(known) => known.1.extend(&acc.seen),
                None => out.push((hatch, acc.seen.clone())),
            }
        }
        out
    }

    /// The destructible objects: walls, hatches and the rest.
    fn finish_objects(&self, data: &mut MapData, hatches: &mut Vec<(Hatch, BTreeSet<u32>)>) {
        for (id, acc) in &self.objects {
            let Some((place, q)) = acc.place else {
                continue;
            };
            let p = metres3(place);
            match acc.kinds.0.keys().next().copied() {
                Some(KIND_WALL) => soft_wall(data, *id, acc, p, q),
                Some(KIND_HATCH) => {
                    let x = rotate(q, [1.0, 0.0, 0.0]);
                    let middle = rotate(q, [HATCH_SIZE / 2.0, HATCH_SIZE / 2.0, 0.0]);
                    let at = [p[0] + middle[0], p[1] + middle[1], p[2]];
                    let found = hatch_at(at, unit2([x[0], x[1]]).unwrap_or([1.0, 0.0]));
                    let known = (hatches.iter_mut())
                        .find(|h| h.0.panel_id.is_none() && same_hatch(&h.0, &found));
                    match known {
                        Some((hatch, seen)) => {
                            hatch.panel_id = Some(*id);
                            hatch.corners = found.corners;
                            hatch.assumed.clear();
                            seen.extend(&acc.seen);
                        }
                        None => hatches.push((
                            Hatch {
                                panel_id: Some(*id),
                                ..found
                            },
                            acc.seen.clone(),
                        )),
                    }
                }
                kind => data.pieces.push(Piece {
                    object_id: Some(*id),
                    kind: match kind {
                        Some(KIND_FLOOR) => PieceKind::Floor,
                        Some(KIND_BREAKABLE) => PieceKind::Breakable,
                        _ => PieceKind::Unknown,
                    },
                    position: tidy3(p),
                    rotation: q.map(tidy),
                    source: Source::Read,
                    rounds: acc.seen.len() as u32,
                }),
            }
        }
    }

    fn finish_named(&self, data: &mut MapData) {
        for (id, acc) in &self.bombs {
            let Some(place) = acc.positions.winner().or(acc.created) else {
                continue;
            };
            let name = acc.names.winner();
            data.sites.push(Site {
                name: name.clone(),
                object_id: Some(*id),
                position: metres3(place),
                floor: None,
                partner: acc.partners.winner(),
                source: Source::Read,
                rounds: acc.rounds,
                played: acc.played,
            });
            // A site is named after its room.
            if let Some(name) = name.filter(|n| !data.rooms.iter().any(|r| r.name == *n)) {
                data.rooms.push(Room {
                    name,
                    floor: 0,
                    polygon: Vec::new(),
                    anchor: Some(metres3(place)),
                    source: Source::Read,
                    rounds: acc.played,
                });
            }
        }
        for (name, (places, rounds)) in &self.callouts {
            if data.rooms.iter().any(|r| r.name == *name) {
                continue;
            }
            data.rooms.push(Room {
                name: name.clone(),
                floor: 0,
                polygon: Vec::new(),
                anchor: Some(median3(places)),
                source: Source::Read,
                rounds: *rounds,
            });
        }
        for (id, acc) in &self.cameras {
            let Some((place, turn)) = acc.places.winner() else {
                continue;
            };
            data.cameras.push(Camera {
                object_id: Some(*id),
                position: metres3(place),
                rotation: turn.map(metres),
                floor: None,
                source: Source::Read,
                rounds: acc.rounds,
            });
        }
        for (name, (places, rounds)) in &self.spawns {
            let at = median3(places);
            let mut away: Vec<Mm> = (places.iter())
                .map(|p| mm(distance2([metres(p[0]), metres(p[1])], [at[0], at[1]])))
                .collect();
            data.spawns.push(Spawn {
                name: name.clone(),
                position: at,
                radius: metres(median(&mut away)),
                source: Source::Derived,
                rounds: *rounds,
                players: places.len() as u32,
            });
        }
    }

    /// The walked cells, each on the floor its height is on.
    fn finish_walked(&self, data: &mut MapData) {
        let mut cells: BTreeMap<i32, BTreeSet<(i32, i32)>> = BTreeMap::new();
        for (step, x, y) in &self.walked {
            let z = *step as f32 * WALKED_STEP;
            if let Some(floor) = data.floor_at(z) {
                cells.entry(floor.index).or_default().insert((*x, *y));
            }
        }
        data.walkable = cells
            .into_iter()
            .map(|(floor, cells)| grid(floor, &cells, Source::Derived, self.walked_rounds))
            .collect();
    }
}

/// A destructible wall, with the stretch of it that was struck.
fn soft_wall(data: &mut MapData, id: u64, acc: &ObjectAcc, p: [f32; 3], q: [f32; 4]) {
    let x = rotate(q, [1.0, 0.0, 0.0]);
    let Some(along) = unit2([x[0], x[1]]) else {
        return;
    };
    // The stretch of it that was struck, along its own x axis.
    let back = inverse(q);
    let (mut low, mut high) = (0f32, 0f32);
    let mut reach = |point: [Mm; 3], half: f32| {
        let w = metres3(point);
        let l = rotate(back, [w[0] - p[0], w[1] - p[1], w[2] - p[2]]);
        let on = l[1].abs() <= IMPACT_ACROSS
            && (IMPACT_BELOW..=IMPACT_ABOVE).contains(&l[2])
            && l[0].abs() <= IMPACT_ALONG;
        if on {
            low = low.min(l[0] - half);
            high = high.max(l[0] + half);
        }
    };
    acc.impacts.iter().for_each(|i| reach(*i, 0.0));
    acc.patches.iter().for_each(|s| reach(s.0, metres(s.1)));
    let measured = high - low >= 1e-3;
    let end = |t: f32| tidy2([p[0] + along[0] * t, p[1] + along[1] * t]);
    let mut assumed = vec!["top".to_owned()];
    if !measured {
        assumed.push("b".to_owned());
    }
    data.walls.push(Wall {
        a: end(low),
        b: end(high),
        floor: None,
        bottom: tidy(p[2]),
        top: tidy(p[2] + WALL_HEIGHT),
        kind: WallKind::Soft,
        width: tidy(high - low),
        object_id: Some(id),
        parts: Vec::new(),
        origin: Some(tidy2([p[0], p[1]])),
        part_of: None,
        see_through: false,
        // Where it stands is read; how far it runs is the impacts' reach.
        source: if measured {
            Source::Derived
        } else {
            Source::Read
        },
        rounds: acc.seen.len() as u32,
        assumed,
    });
}

/// `<match id>/<round number>`: the same for every recording of a round.
fn round_key(round: &Round) -> String {
    let header = &round.header;
    if header.match_id.is_empty() {
        return format!("{}/{}", header.timestamp.to_rfc3339(), header.round_number);
    }
    format!("{}/{}", header.match_id, header.round_number)
}

/// A name with single spaces: the game writes `1F  Hallway`.
fn squeeze(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn median3(places: &[[Mm; 3]]) -> [f32; 3] {
    let axis = |i: usize| {
        let mut v: Vec<Mm> = places.iter().map(|p| p[i]).collect();
        metres(median(&mut v))
    };
    [axis(0), axis(1), axis(2)]
}

/// A hatch with its middle at `at` and its sides along `along`.
fn hatch_at(at: [f32; 3], along: [f32; 2]) -> Hatch {
    let half = HATCH_SIZE / 2.0;
    let u = [along[0] * half, along[1] * half];
    let v = [-along[1] * half, along[0] * half];
    let corner = |s: f32, t: f32| tidy2([at[0] + s * u[0] + t * v[0], at[1] + s * u[1] + t * v[1]]);
    Hatch {
        position: tidy3(at),
        corners: [
            corner(-1.0, -1.0),
            corner(1.0, -1.0),
            corner(1.0, 1.0),
            corner(-1.0, 1.0),
        ],
        size: HATCH_SIZE,
        floor: None,
        object_id: None,
        panel_id: None,
        reinforceable: false,
        source: Source::Read,
        rounds: 0,
        assumed: Vec::new(),
    }
}

fn grid(floor: i32, cells: &BTreeSet<(i32, i32)>, source: Source, rounds: u32) -> Walkable {
    let xs = || cells.iter().map(|c| c.0);
    let ys = || cells.iter().map(|c| c.1);
    let (x0, x1) = (xs().min().unwrap_or(0), xs().max().unwrap_or(-1));
    let (y0, y1) = (ys().min().unwrap_or(0), ys().max().unwrap_or(-1));
    let row = |y: i32| {
        (x0..=x1)
            .map(|x| if cells.contains(&(x, y)) { '#' } else { '.' })
            .collect()
    };
    Walkable {
        floor,
        cell: CELL,
        origin: [x0, y0],
        rows: (y0..=y1).map(row).collect(),
        source,
        rounds,
    }
}

impl Walkable {
    /// The cells somebody stood in.
    fn cells(&self) -> BTreeSet<(i32, i32)> {
        let mut out = BTreeSet::new();
        for (r, row) in self.rows.iter().enumerate() {
            for (c, cell) in row.bytes().enumerate() {
                if cell == b'#' {
                    out.insert((self.origin[0] + c as i32, self.origin[1] + r as i32));
                }
            }
        }
        out
    }

    /// Whether somebody stood in the cell of `(x, y)`.
    pub fn walked(&self, x: f32, y: f32) -> bool {
        if self.cell <= 0.0 {
            return false;
        }
        let i = (x / self.cell).floor() as i32 - self.origin[0];
        let j = (y / self.cell).floor() as i32 - self.origin[1];
        let row = usize::try_from(j).ok().and_then(|j| self.rows.get(j));
        let cell = usize::try_from(i).ok().zip(row);
        cell.is_some_and(|(i, row)| row.as_bytes().get(i) == Some(&b'#'))
    }
}

/// The map data of `rounds`, from what a [`Round`] holds: no wall but the
/// reinforced ones and no bomb but those in play (see [`harvest_with`]).
/// Rounds of another map than the first one's are left out.
pub fn harvest(rounds: &[Round]) -> MapData {
    let mut harvester = Harvester::new();
    for round in rounds {
        harvester.add(round, &[]);
    }
    harvester.finish()
}

/// The map data of rounds read with their map objects ([`read`]). Rounds
/// of another map than the first one's are left out.
pub fn harvest_with(rounds: &[Observed]) -> MapData {
    let mut harvester = Harvester::new();
    for o in rounds {
        harvester.add(&o.round, &o.objects);
    }
    harvester.finish()
}

// ----------------------------------------------------------------- merge

fn same_wall(a: &Wall, b: &Wall) -> bool {
    match (a.object_id, b.object_id) {
        (Some(x), Some(y)) => x == y,
        _ => {
            (a.bottom - b.bottom).abs() <= SAME_PLACE
                && distance2(middle(a.a, a.b), middle(b.a, b.b)) <= SAME_PLACE
        }
    }
}

fn same_opening(a: &Opening, b: &Opening) -> bool {
    (a.top - b.top).abs() <= SAME_PLACE
        && distance2(middle(a.a, a.b), middle(b.a, b.b)) <= SAME_PLACE
}

fn same_hatch(a: &Hatch, b: &Hatch) -> bool {
    match (a.object_id, b.object_id) {
        (Some(x), Some(y)) => x == y,
        // The hatch itself and what a reinforcement names differ by a few
        // centimetres.
        _ => distance3(a.position, b.position) <= HATCH_SIZE / 2.0,
    }
}

/// Whether two things with an id and a place are the same one.
fn same_object(a: (Option<u64>, [f32; 3]), b: (Option<u64>, [f32; 3])) -> bool {
    match (a.0, b.0) {
        (Some(x), Some(y)) => x == y,
        _ => distance3(a.1, b.1) <= SAME_PLACE,
    }
}

/// How two counts of rounds add up: the sum when the two sides share no
/// round, the larger when they do. That is exact when one side holds
/// every round of the other, and too low when they overlap in part.
#[derive(Clone, Copy)]
struct Count {
    shared: bool,
}

impl Count {
    fn of(self, a: u32, b: u32) -> u32 {
        if self.shared { a.max(b) } else { a + b }
    }
}

/// The elements of `a` and `b` as one list. Two that are `same` become
/// one: the authored one as it is (`a`'s of two), else `both` of the two.
fn merge_list<T: Clone>(
    a: &[T],
    b: &[T],
    source: impl Fn(&T) -> Source,
    same: impl Fn(&T, &T) -> bool,
    both: impl Fn(&T, &T) -> T,
) -> Vec<T> {
    let mut out: Vec<T> = a.to_vec();
    let mut taken = vec![false; a.len()];
    for item in b {
        let found = (0..a.len()).find(|i| !taken[*i] && same(&a[*i], item));
        let Some(i) = found else {
            out.push(item.clone());
            continue;
        };
        taken[i] = true;
        out[i] = match (source(&a[i]), source(item)) {
            (Source::Authored, _) => a[i].clone(),
            (_, Source::Authored) => item.clone(),
            _ => both(&a[i], item),
        };
    }
    out
}

/// `a` and `b` as one: everything either holds, the same thing once.
///
/// - An authored element is never changed: it wins over a harvested one
///   at the same place (the same object id, or within 0.35 m), and of two
///   authored ones `a`'s is kept.
/// - Two harvested elements become the one seen in more rounds, with the
///   parts and the reach of both.
/// - `rounds` are added up when `a` and `b` share no round of
///   [`MapData::rounds`], and are the larger of the two when they do, so
///   `merge(a, a)` is `a`.
/// - Floors are worked out again from what the result holds, authored
///   floors kept; every element's `floor` follows.
///
/// The map is `a`'s. Data of two maps is not to be merged: `b` is left
/// out when both name a map and not the same one.
pub fn merge(a: &MapData, b: &MapData) -> MapData {
    if a.map.id != 0 && b.map.id != 0 && a.map.id != b.map.id {
        return a.clone();
    }
    let shared = a.rounds.iter().any(|k| b.rounds.contains(k))
        || (a.rounds.is_empty() && b.rounds.is_empty());
    let count = Count { shared };
    let keys: BTreeSet<&String> = a.rounds.iter().chain(&b.rounds).collect();
    let authored = |d: &MapData| d.bounds.clone().filter(|b| b.source == Source::Authored);
    let mut out = MapData {
        schema: SCHEMA,
        map: if a.map.id != 0 {
            a.map.clone()
        } else {
            b.map.clone()
        },
        rounds: keys.into_iter().cloned().collect(),
        bounds: authored(a).or(authored(b)),
        ..MapData::default()
    };
    // Authored floors stay; the others are worked out again.
    let drawn = |d: &MapData| -> Vec<Floor> {
        (d.floors.iter())
            .filter(|f| f.source == Source::Authored)
            .cloned()
            .collect()
    };
    out.floors = merge_list(
        &drawn(a),
        &drawn(b),
        |f| f.source,
        |x, y| (x.z - y.z).abs() <= SAME_FLOOR,
        |x, _| x.clone(),
    );
    out.rooms = merge_list(
        &a.rooms,
        &b.rooms,
        |r| r.source,
        |x, y| x.name == y.name,
        |x, y| Room {
            rounds: count.of(x.rounds, y.rounds),
            ..if y.rounds > x.rounds { y } else { x }.clone()
        },
    );
    out.sites = merge_list(
        &a.sites,
        &b.sites,
        |s| s.source,
        |x, y| same_object((x.object_id, x.position), (y.object_id, y.position)),
        |x, y| {
            let more = if y.played > x.played { y } else { x };
            Site {
                name: more.name.clone().or(x.name.clone()).or(y.name.clone()),
                partner: more.partner.or(x.partner).or(y.partner),
                rounds: count.of(x.rounds, y.rounds),
                played: count.of(x.played, y.played),
                ..more.clone()
            }
        },
    );
    out.spawns = merge_list(
        &a.spawns,
        &b.spawns,
        |s| s.source,
        |x, y| x.name == y.name,
        |x, y| Spawn {
            rounds: count.of(x.rounds, y.rounds),
            players: count.of(x.players, y.players),
            ..if y.players > x.players { y } else { x }.clone()
        },
    );
    let opening = |x: &Opening, y: &Opening| Opening {
        object_id: x.object_id.or(y.object_id),
        rounds: count.of(x.rounds, y.rounds),
        default_rounds: count.of(x.default_rounds, y.default_rounds),
        ..if y.rounds > x.rounds { y } else { x }.clone()
    };
    out.doors = merge_list(&a.doors, &b.doors, |o| o.source, same_opening, opening);
    out.windows = merge_list(&a.windows, &b.windows, |o| o.source, same_opening, opening);
    // What one side has as a piece and the other as a wall or a hatch.
    let (a_walls, a_hatches, a_pieces) = promoted(a, b);
    let (b_walls, b_hatches, b_pieces) = promoted(b, a);
    out.hatches = merge_list(
        &a_hatches,
        &b_hatches,
        |h| h.source,
        same_hatch,
        |x, y| {
            // The one whose corners are the hatch's own, else the one
            // seen more; the place a reinforcement gave when one did.
            let base = match (x.panel_id.is_some(), y.panel_id.is_some()) {
                (false, true) => y,
                (true, false) => x,
                _ if y.rounds > x.rounds => y,
                _ => x,
            };
            let named = if x.object_id.is_some() { x } else { y };
            Hatch {
                object_id: x.object_id.or(y.object_id),
                panel_id: x.panel_id.or(y.panel_id),
                reinforceable: x.reinforceable || y.reinforceable,
                position: named.position,
                rounds: count.of(x.rounds, y.rounds),
                ..base.clone()
            }
        },
    );
    out.walls = merge_walls(&a_walls, &b_walls, count);
    out.cameras = merge_list(
        &a.cameras,
        &b.cameras,
        |c| c.source,
        |x, y| same_object((x.object_id, x.position), (y.object_id, y.position)),
        |x, y| Camera {
            rounds: count.of(x.rounds, y.rounds),
            ..if y.rounds > x.rounds { y } else { x }.clone()
        },
    );
    out.pieces = merge_list(
        &a_pieces,
        &b_pieces,
        |p| p.source,
        |x, y| same_object((x.object_id, x.position), (y.object_id, y.position)),
        |x, y| Piece {
            // A floor panel before a prop, and either before nothing.
            kind: [PieceKind::Floor, PieceKind::Breakable]
                .into_iter()
                .find(|k| x.kind == *k || y.kind == *k)
                .unwrap_or_default(),
            rounds: count.of(x.rounds, y.rounds),
            ..x.clone()
        },
    );
    // A room's floor is an index into its own side's floors: by height,
    // an authored room's last.
    let mut heights = BTreeMap::new();
    for authored in [false, true] {
        for side in [b, a] {
            let rooms = side.rooms.iter();
            for room in rooms.filter(|r| (r.source == Source::Authored) == authored) {
                if let Some(floor) = side.floors.iter().find(|f| f.index == room.floor) {
                    heights.insert(room.name.clone(), floor.z);
                }
            }
        }
    }
    out.rebuild_with(&heights);
    // Walked cells by height too.
    let mut cells: BTreeMap<i32, (BTreeSet<(i32, i32)>, Source)> = BTreeMap::new();
    for side in [a, b] {
        for walk in &side.walkable {
            let z = side.floors.iter().find(|f| f.index == walk.floor);
            let Some(floor) = z.and_then(|f| out.floor_at(f.z)) else {
                continue;
            };
            let entry = (cells.entry(floor.index)).or_insert((BTreeSet::new(), Source::Derived));
            entry.0.extend(walk.cells());
            if walk.source == Source::Authored {
                entry.1 = Source::Authored;
            }
        }
    }
    let walked = |d: &MapData| d.walkable.iter().map(|w| w.rounds).max().unwrap_or(0);
    let rounds = count.of(walked(a), walked(b));
    out.walkable = cells
        .into_iter()
        .map(|(floor, (cells, source))| grid(floor, &cells, source, rounds))
        .collect();
    out.rebuild();
    out
}

/// The walls, hatches and pieces of `side`, with each harvested piece
/// that `other` knows as a wall or a hatch made that: an object is a
/// piece where nothing says what it is, and a round that damaged it says
/// so. The wall or hatch is the other side's, seen in the rounds the
/// piece was.
fn promoted(side: &MapData, other: &MapData) -> (Vec<Wall>, Vec<Hatch>, Vec<Piece>) {
    let (mut walls, mut hatches) = (side.walls.clone(), side.hatches.clone());
    let harvested = |s: Source| s != Source::Authored;
    let mut pieces = Vec::new();
    for piece in &side.pieces {
        let Some(id) = piece.object_id.filter(|_| harvested(piece.source)) else {
            pieces.push(piece.clone());
            continue;
        };
        let mut theirs = other.walls.iter().filter(|w| harvested(w.source));
        let wall = theirs.find(|w| w.object_id == Some(id));
        let hatch = (other.hatches.iter()).find(|h| harvested(h.source) && h.panel_id == Some(id));
        if let Some(wall) = wall.filter(|w| w.kind == WallKind::Soft) {
            walls.push(Wall {
                rounds: piece.rounds,
                ..wall.clone()
            });
        } else if let Some(hatch) = hatch {
            match hatches.iter_mut().find(|h| same_hatch(h, hatch)) {
                // Reinforced here: the rounds it was damaged in are not
                // known to be other rounds.
                Some(mine) => {
                    mine.panel_id = Some(id);
                    mine.corners = hatch.corners;
                    mine.assumed.clear();
                    mine.rounds = mine.rounds.max(piece.rounds);
                }
                None => hatches.push(Hatch {
                    object_id: None,
                    reinforceable: false,
                    rounds: piece.rounds,
                    ..hatch.clone()
                }),
            }
        } else {
            pieces.push(piece.clone());
        }
    }
    (walls, hatches, pieces)
}

/// Walls of the two sides.
fn merge_walls(a: &[Wall], b: &[Wall], count: Count) -> Vec<Wall> {
    merge_list(
        a,
        b,
        |w| w.source,
        same_wall,
        |x, y| {
            // The one that names the wall, else the one seen more.
            let second = match (x.object_id.is_some(), y.object_id.is_some()) {
                (false, true) => true,
                (true, false) => false,
                _ => y.rounds > x.rounds,
            };
            let (base, other) = if second { (y, x) } else { (x, y) };
            let mut wall = Wall {
                parts: x.parts.iter().chain(&y.parts).copied().collect(),
                rounds: count.of(x.rounds, y.rounds),
                ..base.clone()
            };
            if wall.kind == WallKind::Soft && other.kind == WallKind::Soft {
                widen(&mut wall, other);
            }
            wall
        },
    )
}

/// Makes `wall` reach as far as `other`, a harvest of the same soft wall,
/// does.
fn widen(wall: &mut Wall, other: &Wall) {
    let known = |w: &Wall| distance2(w.a, w.b) > 1e-3;
    if !known(other) {
        return;
    }
    let dir = unit2([wall.b[0] - wall.a[0], wall.b[1] - wall.a[1]]);
    let Some(dir) = dir.filter(|_| known(wall)) else {
        (wall.a, wall.b, wall.width) = (other.a, other.b, other.width);
        (wall.source, wall.assumed) = (other.source, other.assumed.clone());
        return;
    };
    let from = wall.a;
    let along = |p: [f32; 2]| (p[0] - from[0]) * dir[0] + (p[1] - from[1]) * dir[1];
    let ends = [0.0, along(wall.b), along(other.a), along(other.b)];
    let low = ends.iter().copied().fold(f32::INFINITY, f32::min);
    let high = ends.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let at = |t: f32| tidy2([from[0] + dir[0] * t, from[1] + dir[1] * t]);
    (wall.a, wall.b, wall.width) = (at(low), at(high), tidy(high - low));
}

// --------------------------------------------------------------- lookups

/// A door, a window or a hatch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OpeningRef<'a> {
    Door(&'a Opening),
    Window(&'a Opening),
    Hatch(&'a Hatch),
}

impl Opening {
    /// The middle of the opening, half way up.
    pub fn center(&self) -> [f32; 3] {
        let m = middle(self.a, self.b);
        [m[0], m[1], (self.bottom + self.top) / 2.0]
    }

    /// Metres from `(x, y, z)` to the opening: to its segment over the
    /// ground, and to its span in height.
    pub fn distance(&self, x: f32, y: f32, z: f32) -> f32 {
        let over = segment_distance([x, y], self.a, self.b);
        let up = (self.bottom - z).max(z - self.top).max(0.0);
        over.hypot(up)
    }
}

impl Wall {
    /// Metres from `(x, y, z)` to the wall: to its segment over the
    /// ground, and to its span in height.
    pub fn distance(&self, x: f32, y: f32, z: f32) -> f32 {
        let over = segment_distance([x, y], self.a, self.b);
        let up = (self.bottom - z).max(z - self.top).max(0.0);
        over.hypot(up)
    }
}

impl Hatch {
    /// Metres from `(x, y, z)` to the middle of the hatch.
    pub fn distance(&self, x: f32, y: f32, z: f32) -> f32 {
        distance3(self.position, [x, y, z])
    }
}

impl Floor {
    /// The lowest height that belongs to the floor.
    pub fn low(&self) -> f32 {
        self.levels.iter().copied().fold(self.z, f32::min)
    }
}

fn nearest<T>(items: &[T], distance: impl Fn(&T) -> f32) -> Option<(&T, f32)> {
    (items.iter())
        .map(|i| (i, distance(i)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// The floor of `floors` a body with its feet at `z` is on.
fn floor_of(floors: &[Floor], z: f32) -> Option<&Floor> {
    let floor = (floors.iter())
        .filter(|f| f.low() - FLOOR_BELOW <= z)
        .max_by(|a, b| a.low().total_cmp(&b.low()))?;
    let under = floor.ceiling.is_none_or(|c| z < c - FLOOR_BELOW);
    under.then_some(floor)
}

impl MapData {
    /// The floor a body with its feet at `z` is on: the highest floor that
    /// starts at or below `z`, with 0.6 m to spare for steps. Stairs up
    /// belong to the floor they leave. `None` below the lowest floor, at
    /// or above the top floor's ceiling (a roof), and when no floor is
    /// known.
    pub fn floor_at(&self, z: f32) -> Option<&Floor> {
        floor_of(&self.floors, z)
    }

    /// The room `(x, y, z)` is in: the first room of the floor at `z`
    /// whose outline holds `(x, y)`. Harvested rooms have no outline.
    pub fn room_at(&self, x: f32, y: f32, z: f32) -> Option<&Room> {
        let floor = self.floor_at(z)?.index;
        (self.rooms.iter()).find(|r| r.floor == floor && inside(&r.polygon, [x, y]))
    }

    /// The room on the floor at `z` whose anchor is nearest `(x, y)`, with
    /// the metres to it: a guess at the callout where no outline is drawn.
    pub fn nearest_room(&self, x: f32, y: f32, z: f32) -> Option<(&Room, f32)> {
        let floor = self.floor_at(z)?.index;
        (self.rooms.iter())
            .filter(|r| r.floor == floor)
            .filter_map(|r| Some((r, distance2([r.anchor?[0], r.anchor?[1]], [x, y]))))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// The nearest door, with the metres to it.
    pub fn nearest_door(&self, x: f32, y: f32, z: f32) -> Option<(&Opening, f32)> {
        nearest(&self.doors, |o| o.distance(x, y, z))
    }

    pub fn nearest_window(&self, x: f32, y: f32, z: f32) -> Option<(&Opening, f32)> {
        nearest(&self.windows, |o| o.distance(x, y, z))
    }

    pub fn nearest_hatch(&self, x: f32, y: f32, z: f32) -> Option<(&Hatch, f32)> {
        nearest(&self.hatches, |h| h.distance(x, y, z))
    }

    /// The nearest door, window or hatch, with the metres to it.
    pub fn nearest_opening(&self, x: f32, y: f32, z: f32) -> Option<(OpeningRef<'_>, f32)> {
        let door = self.nearest_door(x, y, z);
        let window = self.nearest_window(x, y, z);
        let hatch = self.nearest_hatch(x, y, z);
        [
            door.map(|o| (OpeningRef::Door(o.0), o.1)),
            window.map(|o| (OpeningRef::Window(o.0), o.1)),
            hatch.map(|o| (OpeningRef::Hatch(o.0), o.1)),
        ]
        .into_iter()
        .flatten()
        .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// The nearest wall, with the metres to it.
    pub fn nearest_wall(&self, x: f32, y: f32, z: f32) -> Option<(&Wall, f32)> {
        nearest(&self.walls, |w| w.distance(x, y, z))
    }

    /// The wall with the object id `id`.
    pub fn wall(&self, id: u64) -> Option<&Wall> {
        (self.walls.iter()).find(|w| w.object_id == Some(id))
    }

    /// Works out what follows from the elements: the floors that are not
    /// authored, every element's `floor`, the bounds unless authored, and
    /// the order of every list. [`harvest`] and [`merge`] end with it;
    /// call it after editing a map data by hand.
    pub fn rebuild(&mut self) {
        let heights = (self.rooms.iter())
            .filter_map(|r| {
                let floor = self.floors.iter().find(|f| f.index == r.floor)?;
                Some((r.name.clone(), floor.z))
            })
            .collect();
        self.rebuild_with(&heights);
    }

    /// [`MapData::rebuild`], with the height of the floor of each room by
    /// its name: a room's `floor` is an index, which changes here.
    fn rebuild_with(&mut self, heights: &BTreeMap<String, f32>) {
        self.sort();
        self.link_parts();
        self.rebuild_floors();
        let floors = &self.floors;
        let on = |z: f32| floor_of(floors, z).map(|f| f.index);
        let near = |z: f32| {
            (floors.iter())
                .min_by(|a, b| (a.z - z).abs().total_cmp(&(b.z - z).abs()))
                .map(|f| f.index)
        };
        for room in &mut self.rooms {
            // A room the file names at a point is on the floor that point
            // stands on; a drawn one stays on its floor.
            let read = room.anchor.and_then(|a| on(a[2]).or(near(a[2])));
            let drawn = heights.get(&room.name).and_then(|z| near(*z));
            room.floor = read.or(drawn).unwrap_or(0);
        }
        for wall in &mut self.walls {
            wall.floor = on(wall.bottom);
            if wall.source != Source::Authored && wall.assumed.iter().any(|a| a == "top") {
                // Up to the floor above, where that is lower.
                let ceiling = (wall.floor)
                    .and_then(|i| floors.iter().find(|f| f.index == i))
                    .and_then(|f| f.ceiling);
                let top = wall.bottom + WALL_HEIGHT;
                wall.top = tidy(ceiling.map_or(top, |c| top.min(c.max(wall.bottom + 1.0))));
            }
        }
        for door in &mut self.doors {
            door.floor = on(door.bottom);
        }
        for window in &mut self.windows {
            window.floor = on(window.top - WINDOW_TOP + FLOOR_BELOW).or(on(window.bottom));
        }
        for hatch in &mut self.hatches {
            hatch.floor = on(hatch.position[2]);
        }
        for site in &mut self.sites {
            site.floor = on(site.position[2]);
        }
        for camera in &mut self.cameras {
            camera.floor = on(camera.position[2]);
        }
        self.sort();
        if (self.bounds.as_ref()).is_none_or(|b| b.source != Source::Authored) {
            self.bounds = self.measure();
        }
    }

    /// Links each harvested destructible wall to the reinforceable wall
    /// its origin is on: the first in the order of the list, since an
    /// origin where two walls meet is on both. Worked out anew each time.
    fn link_parts(&mut self) {
        let harvested = |w: &Wall| w.source != Source::Authored;
        let on = |soft: &Wall, w: &Wall| {
            let (Some(o), Some(dir)) = (soft.origin, unit2([w.b[0] - w.a[0], w.b[1] - w.a[1]]))
            else {
                return false;
            };
            let rel = [o[0] - w.a[0], o[1] - w.a[1]];
            let t = rel[0] * dir[0] + rel[1] * dir[1];
            let across = (rel[0] * dir[1] - rel[1] * dir[0]).abs();
            // A wall with a stretch runs the same way.
            let run = unit2([soft.b[0] - soft.a[0], soft.b[1] - soft.a[1]]);
            (w.bottom - soft.bottom).abs() <= PART_ACROSS
                && run.is_none_or(|r| (dir[0] * r[0] + dir[1] * r[1]).abs() > 0.95)
                && across <= PART_ACROSS
                && (-PART_ALONG..=w.width + PART_ALONG).contains(&t)
        };
        // Per wall: the wall it is a part of.
        let of: Vec<Option<usize>> = (self.walls.iter())
            .map(|soft| {
                let free = soft.kind == WallKind::Soft && harvested(soft);
                let hard = |w: &Wall| w.kind == WallKind::Reinforceable && w.object_id.is_some();
                (self.walls.iter())
                    .position(|w| free && hard(w) && on(soft, w))
                    .filter(|_| soft.object_id.is_some())
            })
            .collect();
        for wall in self.walls.iter_mut().filter(|w| harvested(w)) {
            wall.parts.clear();
            wall.part_of = None;
        }
        for (soft, hard) in of.iter().enumerate() {
            let (Some(hard), Some(id)) = (hard, self.walls[soft].object_id) else {
                continue;
            };
            self.walls[soft].part_of = self.walls[*hard].object_id;
            if harvested(&self.walls[*hard]) {
                self.walls[*hard].parts.push(id);
            }
        }
        for wall in &mut self.walls {
            wall.parts.sort_unstable();
        }
    }

    /// The heights things stand on, put together into floors.
    fn rebuild_floors(&mut self) {
        // Per height in tenths: `(things on it, most rounds one was seen
        // in)`.
        let mut levels: BTreeMap<i32, (u32, u32)> = BTreeMap::new();
        let mut stand = |source: Source, z: f32, rounds: u32| {
            if source != Source::Authored {
                let level = levels.entry((z * 10.0).round() as i32).or_default();
                *level = (level.0 + 1, level.1.max(rounds));
            }
        };
        // A destructible wall is not always one that stands on the floor.
        for w in self.walls.iter().filter(|w| w.kind != WallKind::Soft) {
            stand(w.source, w.bottom, w.rounds);
        }
        for d in &self.doors {
            stand(d.source, d.bottom, d.rounds);
        }
        for h in &self.hatches {
            stand(h.source, h.position[2], h.rounds);
        }
        for s in &self.sites {
            stand(s.source, s.position[2], s.rounds);
        }
        let mut floors: Vec<Floor> = (self.floors.iter())
            .filter(|f| f.source == Source::Authored)
            .cloned()
            .collect();
        // Heights within a span of the lowest are one floor.
        let mut groups: Vec<Vec<(f32, u32, u32)>> = Vec::new();
        for (level, (things, rounds)) in &levels {
            let z = *level as f32 / 10.0;
            match groups.last_mut() {
                Some(group) if z - group[0].0 <= FLOOR_SPAN => group.push((z, *things, *rounds)),
                _ => groups.push(vec![(z, *things, *rounds)]),
            }
        }
        for group in groups {
            let most = group.iter().map(|l| l.1).max().unwrap_or(0);
            let z = group.iter().find(|l| l.1 == most).map_or(0.0, |l| l.0);
            let (low, high) = (group[0].0, group[group.len() - 1].0);
            let drawn = (floors.iter()).any(|f| {
                f.source == Source::Authored
                    && (low - SAME_FLOOR..=high + SAME_FLOOR).contains(&f.z)
            });
            if !drawn {
                floors.push(Floor {
                    index: 0,
                    name: None,
                    z,
                    ceiling: None,
                    levels: group.iter().map(|l| l.0).collect(),
                    source: Source::Derived,
                    rounds: group.iter().map(|l| l.2).max().unwrap_or(0),
                    assumed: Vec::new(),
                });
            }
        }
        floors.sort_by(|a, b| a.z.total_cmp(&b.z));
        // The ceiling of a floor is where the next one starts; above the
        // top floor, a storey like the ones below it.
        let lows: Vec<f32> = floors.iter().map(Floor::low).collect();
        let mut gaps: Vec<Mm> = lows.windows(2).map(|w| mm(w[1] - w[0])).collect();
        let storey = if gaps.is_empty() {
            STOREY
        } else {
            metres(median(&mut gaps))
        };
        for (i, floor) in floors.iter_mut().enumerate() {
            floor.index = i as i32;
            if floor.source == Source::Authored {
                continue;
            }
            let above = lows.get(i + 1).copied();
            floor.ceiling = Some(above.unwrap_or(tidy(floor.z + storey)));
            if above.is_none() {
                floor.assumed.push("ceiling".to_owned());
            }
            // Named by the sites and rooms that stand on it.
            let top = above.unwrap_or(f32::INFINITY) - FLOOR_BELOW;
            let on = |z: f32| (floor.low() - FLOOR_BELOW..top).contains(&z);
            let mut names: Votes<String> = Votes::default();
            let sites = (self.sites.iter()).filter_map(|s| Some((s.name.as_ref()?, s.position[2])));
            let rooms = (self.rooms.iter()).filter_map(|r| Some((&r.name, r.anchor?[2])));
            for (name, z) in sites.chain(rooms) {
                if let (Some(prefix), true) = (floor_prefix(name), on(z)) {
                    names.add(prefix.to_owned());
                }
            }
            floor.name = names.winner();
        }
        self.floors = floors;
    }

    fn sort(&mut self) {
        let key3 = |p: [f32; 3]| (mm(p[2]), mm(p[0]), mm(p[1]));
        let key2 =
            |z: f32, a: [f32; 2], b: [f32; 2]| [mm(z), mm(a[0]), mm(a[1]), mm(b[0]), mm(b[1])];
        (self.rooms).sort_by(|a, b| (a.floor, &a.name).cmp(&(b.floor, &b.name)));
        self.sites.sort_by_key(|s| (key3(s.position), s.object_id));
        self.spawns.sort_by(|a, b| a.name.cmp(&b.name));
        self.doors.sort_by_key(|o| key2(o.top, o.a, o.b));
        self.windows.sort_by_key(|o| key2(o.top, o.a, o.b));
        (self.hatches).sort_by_key(|h| (key3(h.position), h.object_id));
        for wall in &mut self.walls {
            wall.parts.sort_unstable();
            wall.parts.dedup();
        }
        (self.walls).sort_by_key(|w| (key2(w.bottom, w.a, w.b), w.object_id));
        (self.cameras).sort_by_key(|c| (key3(c.position), c.object_id));
        (self.pieces).sort_by_key(|p| (p.object_id, key3(p.position)));
        self.walkable.sort_by_key(|w| w.floor);
    }

    /// The box around everything held.
    fn measure(&self) -> Option<Bounds> {
        let height = |index: i32| {
            let floor = self.floors.iter().find(|f| f.index == index);
            floor.map_or(0.0, |f| f.z)
        };
        let mut points: Vec<[f32; 3]> = Vec::new();
        for w in &self.walls {
            points.push([w.a[0], w.a[1], w.bottom]);
            points.push([w.b[0], w.b[1], w.top]);
        }
        for o in self.doors.iter().chain(&self.windows) {
            points.push([o.a[0], o.a[1], o.bottom]);
            points.push([o.b[0], o.b[1], o.top]);
        }
        points.extend(self.hatches.iter().map(|h| h.position));
        points.extend(self.sites.iter().map(|s| s.position));
        points.extend(self.spawns.iter().map(|s| s.position));
        points.extend(self.cameras.iter().map(|c| c.position));
        points.extend(self.pieces.iter().map(|p| p.position));
        for room in &self.rooms {
            let z = height(room.floor);
            points.extend(room.polygon.iter().map(|p| [p[0], p[1], z]));
        }
        for walk in self.walkable.iter().filter(|w| !w.rows.is_empty()) {
            let z = height(walk.floor);
            let wide = walk.rows.iter().map(String::len).max().unwrap_or(0) as f32;
            let long = walk.rows.len() as f32;
            let (x, y) = (
                walk.origin[0] as f32 * walk.cell,
                walk.origin[1] as f32 * walk.cell,
            );
            points.push([x, y, z]);
            points.push([x + wide * walk.cell, y + long * walk.cell, z]);
        }
        let first = *points.first()?;
        let (mut min, mut max) = (first, first);
        for p in &points {
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        Some(Bounds {
            min: tidy3(min),
            max: tidy3(max),
            source: Source::Derived,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall(id: u64, a: [f32; 2], b: [f32; 2], bottom: f32) -> Wall {
        Wall {
            a,
            b,
            bottom,
            top: bottom + WALL_HEIGHT,
            kind: WallKind::Reinforceable,
            width: distance2(a, b),
            object_id: Some(id),
            rounds: 1,
            assumed: vec!["top".to_owned()],
            ..Wall::default()
        }
    }

    fn create(id: u64, class: [u8; 4], position: [f32; 3]) -> Vec<u8> {
        let mut d = id.to_le_bytes().to_vec();
        d.extend((MAP_CREATE_SIZE as u32).to_le_bytes());
        d.extend(MAP_CREATE);
        d.extend(id.to_le_bytes());
        d.extend([0; 4]);
        position.iter().for_each(|f| d.extend(f.to_le_bytes()));
        [0f32, 0.0, 0.0, 1.0]
            .iter()
            .for_each(|f| d.extend(f.to_le_bytes()));
        d.push(0);
        d.extend(0u64.to_le_bytes());
        d.extend(1u32.to_le_bytes());
        d.extend(class);
        d.resize(12 + MAP_CREATE_SIZE, 0);
        d
    }

    #[test]
    fn a_map_create_message_gives_the_object() {
        let mut data = vec![7; 5];
        data.extend(create(0x60572f1958, DEVICE, [1.0, 2.0, 3.0]));
        data.extend(create(0x60572f71b8, BOMB, [4.0, 5.0, 6.0]));
        // The same object again, and a message whose header names another
        // object.
        data.extend(create(0x60572f71b8, BOMB, [9.0, 9.0, 9.0]));
        let mut other = create(0x61e910a0e2, DAMAGE, [0.0; 3]);
        other[0] ^= 1;
        data.extend(other);
        data.extend(create(0x61e910a0e8, DAMAGE, [7.0, 8.0, 9.0]));
        let objects = map_objects(&data);
        let kinds: Vec<_> = objects.iter().map(|o| (o.id, o.kind)).collect();
        assert_eq!(
            kinds,
            [
                (0x60572f1958, MapObjectKind::Camera),
                (0x60572f71b8, MapObjectKind::Bomb),
                (0x61e910a0e8, MapObjectKind::Destructible),
            ]
        );
        assert_eq!(objects[1].position, [4.0, 5.0, 6.0]);
        assert_eq!(objects[0].rotation, [0.0, 0.0, 0.0, 1.0]);
        // Cut short anywhere, nothing is read past the end.
        for cut in 0..data.len() {
            assert!(map_objects(&data[..cut]).len() <= 3);
        }
    }

    #[test]
    fn heights_group_into_floors() {
        let mut data = MapData::default();
        // A basement with a raised part, a split ground floor, a top
        // floor.
        for (i, z) in [-4.4, -2.5, -0.5, 0.4, 0.4, 4.3].iter().enumerate() {
            let y = i as f32;
            data.walls.push(wall(i as u64 + 1, [0.0, y], [2.0, y], *z));
        }
        data.sites.push(Site {
            name: Some("2F Gym".to_owned()),
            position: [1.0, 1.0, 4.3],
            ..Site::default()
        });
        data.rebuild();
        let found: Vec<_> = (data.floors.iter())
            .map(|f| (f.index, f.z, f.low()))
            .collect();
        assert_eq!(found, [(0, -4.4, -4.4), (1, 0.4, -0.5), (2, 4.3, 4.3)]);
        assert_eq!(data.floors[2].name.as_deref(), Some("2F"));
        assert_eq!(data.floors[0].ceiling, Some(-0.5));
        assert_eq!(data.floors[2].assumed, ["ceiling"]);
        assert_eq!(data.walls.iter().filter(|w| w.floor == Some(1)).count(), 3);
        // Steps down, stairs up, the roof.
        let at = |z: f32| data.floor_at(z).map(|f| f.index);
        assert_eq!((at(-5.2), at(-4.9), at(-1.2)), (None, Some(0), Some(0)));
        assert_eq!((at(-1.0), at(3.0), at(3.8)), (Some(1), Some(1), Some(2)));
        assert_eq!((at(5.0), at(9.0)), (Some(2), None));
        // A wall under the next floor stops at it.
        assert_eq!(data.wall(1).map(|w| w.top), Some(-1.4));
        // Working it out again changes nothing.
        let again = {
            let mut d = data.clone();
            d.rebuild();
            d
        };
        assert_eq!(again, data);
    }

    #[test]
    fn floor_names_are_read_off_site_names() {
        assert_eq!(floor_prefix("B Lockers"), Some("B"));
        assert_eq!(floor_prefix("2F CEO Office"), Some("2F"));
        assert_eq!(floor_prefix("1F  Hallway"), Some("1F"));
        assert_eq!(floor_prefix("Bar"), None);
        assert_eq!(floor_prefix("F"), None);
        assert_eq!(squeeze("1F  Hallway "), "1F Hallway");
    }

    #[test]
    fn authored_data_survives_a_merge() {
        let mut harvested = MapData {
            rounds: vec!["m/1".to_owned()],
            ..MapData::default()
        };
        harvested.walls.push(wall(9, [0.0, 0.0], [2.0, 0.0], 0.0));
        harvested.walls.push(wall(10, [0.0, 5.0], [2.0, 5.0], 4.0));
        harvested.rebuild();
        let mut authored = MapData::default();
        authored.floors.push(Floor {
            name: Some("Ground".to_owned()),
            z: 0.1,
            ceiling: Some(4.0),
            source: Source::Authored,
            ..Floor::default()
        });
        authored.walls.push(Wall {
            b: [2.5, 0.0],
            top: 2.8,
            source: Source::Authored,
            ..wall(9, [0.0; 2], [0.0; 2], 0.0)
        });
        authored.rooms.push(Room {
            name: "Lobby".to_owned(),
            floor: 0,
            polygon: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
            source: Source::Authored,
            ..Room::default()
        });
        authored.rebuild();
        for merged in [merge(&harvested, &authored), merge(&authored, &harvested)] {
            assert_eq!(merged.walls.len(), 2);
            let kept = merged.wall(9).unwrap();
            assert_eq!(
                (kept.b, kept.top, kept.source),
                ([2.5, 0.0], 2.8, Source::Authored)
            );
            // The authored floor stands for the derived one at its height.
            let floors: Vec<_> = merged.floors.iter().map(|f| (f.z, f.source)).collect();
            assert_eq!(floors, [(0.1, Source::Authored), (4.0, Source::Derived)]);
            assert_eq!(merged.floors[0].name.as_deref(), Some("Ground"));
            let room = |z: f32| merged.room_at(1.0, 1.0, z).map(|r| r.name.as_str());
            assert_eq!((room(0.3), room(4.2)), (Some("Lobby"), None));
            assert_eq!(merge(&merged, &merged), merged);
        }
    }

    #[test]
    fn rounds_add_up_once() {
        let mut a = MapData {
            schema: SCHEMA,
            rounds: vec!["m/1".to_owned(), "m/2".to_owned()],
            ..MapData::default()
        };
        a.walls.push(Wall {
            rounds: 2,
            ..wall(9, [0.0, 0.0], [2.0, 0.0], 0.0)
        });
        a.rebuild();
        let mut b = a.clone();
        b.rounds = vec!["m/3".to_owned()];
        b.walls[0].rounds = 1;
        assert_eq!(merge(&a, &a), a);
        let both = merge(&a, &b);
        assert_eq!((both.walls[0].rounds, both.rounds.len()), (3, 3));
        // Merging one side in again changes nothing.
        assert_eq!(merge(&both, &b), both);
    }

    #[test]
    fn a_room_keeps_its_floor_when_a_floor_is_found_below() {
        let mut upper = MapData::default();
        upper.floors.push(Floor {
            z: 4.0,
            source: Source::Authored,
            ..Floor::default()
        });
        upper.rooms.push(Room {
            name: "Office".to_owned(),
            floor: 0,
            polygon: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0]],
            source: Source::Authored,
            ..Room::default()
        });
        upper.rebuild();
        let mut lower = MapData::default();
        lower.walls.push(wall(1, [0.0, 0.0], [2.0, 0.0], 0.0));
        lower.rebuild();
        let merged = merge(&upper, &lower);
        assert_eq!(merged.floors.len(), 2);
        assert_eq!(merged.rooms[0].floor, 1);
        let room = merged.room_at(3.0, 1.0, 4.0);
        assert_eq!(room.map(|r| r.name.as_str()), Some("Office"));
    }

    #[test]
    fn soft_walls_take_the_reach_of_both_sides() {
        let soft = |a: [f32; 2], b: [f32; 2]| Wall {
            kind: WallKind::Soft,
            source: Source::Derived,
            ..wall(5, a, b, 0.0)
        };
        let merged = merge_walls(
            &[soft([0.0, 0.0], [1.0, 0.0])],
            &[soft([-0.5, 0.0], [0.4, 0.0])],
            Count { shared: false },
        );
        assert_eq!((merged[0].a, merged[0].b), ([-0.5, 0.0], [1.0, 0.0]));
        assert_eq!((merged[0].width, merged[0].rounds), (1.5, 2));
    }

    #[test]
    fn a_soft_wall_on_a_reinforced_wall_is_a_part_of_it() {
        let soft = |id: u64, origin: [f32; 2], bottom: f32| Wall {
            kind: WallKind::Soft,
            origin: Some(origin),
            ..wall(id, origin, origin, bottom)
        };
        let mut data = MapData::default();
        data.walls.push(wall(7, [0.0, 0.0], [2.0, 0.0], 0.0));
        data.walls.push(wall(8, [2.0, 0.0], [4.0, 0.0], 0.0));
        // At the end two walls share, in the middle of one, on another
        // floor, and off the line.
        data.walls.push(soft(1, [2.0, 0.1], 0.0));
        data.walls.push(soft(2, [3.0, 0.0], 0.0));
        data.walls.push(soft(3, [1.0, 0.0], 4.0));
        data.walls.push(soft(4, [1.0, 0.6], 0.0));
        data.rebuild();
        let parts = |id: u64| data.wall(id).map(|w| w.parts.clone());
        let part_of = |d: &MapData, id: u64| d.wall(id).and_then(|w| w.part_of);
        assert_eq!((parts(7), parts(8)), (Some(vec![1]), Some(vec![2])));
        assert_eq!((part_of(&data, 1), part_of(&data, 2)), (Some(7), Some(8)));
        assert_eq!((part_of(&data, 3), part_of(&data, 4)), (None, None));
        assert_eq!(data.walls.len(), 6);
        // A wall that only one side of a merge has reinforced.
        let mut other = MapData::default();
        other.walls.push(soft(2, [3.0, 0.0], 0.0));
        other.walls.push(soft(5, [3.5, 0.0], 0.0));
        other.walls.push(soft(6, [9.0, 9.0], 0.0));
        other.rebuild();
        assert_eq!(part_of(&other, 5), None);
        for merged in [merge(&data, &other), merge(&other, &data)] {
            assert_eq!(merged.walls.len(), 8);
            assert_eq!(merged.wall(8).map(|w| w.parts.clone()), Some(vec![2, 5]));
            assert_eq!((part_of(&merged, 5), part_of(&merged, 6)), (Some(8), None));
        }
    }

    #[test]
    fn openings_are_found_by_distance() {
        let mut data = MapData::default();
        data.doors.push(Opening {
            a: [0.0, 0.0],
            b: [1.2, 0.0],
            bottom: 0.0,
            top: 2.2,
            ..Opening::default()
        });
        data.hatches.push(hatch_at([5.0, 5.0, 4.0], [1.0, 0.0]));
        let (found, distance) = data.nearest_opening(0.6, 1.0, 1.0).unwrap();
        assert!(matches!(found, OpeningRef::Door(_)) && (distance - 1.0).abs() < 1e-5);
        let (found, _) = data.nearest_opening(5.0, 4.0, 4.0).unwrap();
        assert!(matches!(found, OpeningRef::Hatch(_)));
        assert_eq!(data.hatches[0].corners[0], [4.0, 4.0]);
        assert_eq!(MapData::default().nearest_opening(0.0, 0.0, 0.0), None);
    }

    #[test]
    fn a_walked_grid_reads_back() {
        let cells: BTreeSet<(i32, i32)> = [(-2, 3), (0, 3), (-1, 5)].into_iter().collect();
        let walk = grid(1, &cells, Source::Derived, 2);
        assert_eq!(walk.origin, [-2, 3]);
        assert_eq!(walk.rows, ["#.#", "...", ".#."]);
        assert_eq!(walk.cells(), cells);
        assert!(walk.walked(-1.5, 3.2) && walk.walked(-0.5, 5.9));
        assert!(!walk.walked(-0.5, 3.5) && !walk.walked(9.0, 9.0));
        assert!(!walk.walked(-3.5, 3.5) && !walk.walked(-1.5, 2.5));
    }
}
