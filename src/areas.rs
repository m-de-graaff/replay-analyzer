//! Areas, environment objects and light screens (Y11S3): the clouds and
//! burning floors of a round, the gas pipes, fire extinguishers and metal
//! detectors of the map that players set off, and the light screens of
//! Sens. All of it is read from the effects of the round
//! ([`crate::fx`]) joined with the entities of the movement stream
//! ([`crate::world`]).
//!
//! # Areas
//!
//! An area is an effect with a position and no parent, of one of seven
//! assets (the names are inferred from what the effects do, see
//! [`crate::types::fx_asset`]):
//!
//! ```text
//! 27012166679   smoke         a smoke grenade's cloud, 14.0 s
//! 73007452394   smoke         a smoke bolt's cloud, 17.0 s
//! 223502673865  fire          every burning floor, 1.8 to 19.9 s
//! 440770340286  fire          a fire of one cell, 4.8 s
//! 297391115747  gas           a remote gas grenade's cloud, 9.8 s
//! 379510214638  swarm         the swarm of a Kawan hive, 15.8 s
//! 401787054484  extinguisher  the burst of a fire extinguisher, 7.0 s;
//!                             its parent is the extinguisher
//! ```
//!
//! It starts in the frame of its spawn and ends in the frame of its stop.
//! Areas that were there when the recording started are in the snapshot
//! of the stream and are left out: when they started is not known.
//!
//! Fire, gas and swarm effects carry a point list, the cells the area
//! covers, which the game writes again as the area spreads; the last list
//! is kept. Smoke and extinguisher clouds have none, and nothing in the
//! file says how large they are: their `radius` is an assumption (3.0 m
//! and 2.5 m) and says so with `radiusSource: "assumed"`.
//!
//! What made a fire, a gas cloud or a swarm is told by the entity without
//! classes the game puts at the effect's position in the same frame (the
//! "area entity", within 0.1 m and from 0.7 s before to 0.2 s after):
//!
//! ```text
//! 339570061825  Volcan Canister      361147040848  Remote Gas Grenade
//! 361809340830  Shumikha Grenade     385049618011  Kawan Hive
//! 361809343208  Fire Bolt            319577383929  Logic Bomb (inferred:
//! 407899098848  Gas Pipe                           seen in real rounds
//!                                                  with Dokkaebi only)
//! ```
//!
//! The area entity is deleted about 1.5 s after the effect stops; the
//! area's end is the stop.
//!
//! Whose area it is, is not written with it. It is the owner of the
//! gadget that was within 0.5 m of the effect when it started and was
//! deleted in the 0.7 s before (a grenade, a canister) or with the
//! effect's stop (a bolt): `usernameSource: "proximity"`. A hive stays
//! where it stuck while its swarm is out, so a swarm without such a gadget
//! takes the nearest owned entity within 1 m (`"nearest"`). The fire of a
//! Logic Bomb takes the Dokkaebi whose ability count dropped 7.0 to 8.2 s
//! before (`"abilityUse"`). The fire of a gas pipe and the cloud of a fire
//! extinguisher have no owner.
//!
//! Who set off a Volcan canister is not in the file. `triggeredBy` is the
//! shooter of a shot that ended within 0.6 m of the fire in the second
//! before the canister was deleted (`triggerSource: "shotRay"`), else the
//! thrower of an object that ended within 5 m and 0.6 s of it
//! (`"explosion"`, with the object's name in `triggeredWith`).
//!
//! # Environment
//!
//! - A fire extinguisher bursts with the area effect above, whose parent
//!   is the map object. Who shot it is the last body its damage list named
//!   in the second before (`bySource: "read"`).
//! - A gas pipe blows up with effect 407368224089 and burns as a fire
//!   area 0.3 s later. The effect has no parent. The pipe is the map
//!   object the per-map table lists within 1.5 m of the explosion
//!   (`objectSource: "table"`); on a map or for a pipe the table does not
//!   have, a map object within 1.5 m, preferring those outside the map's
//!   largest id family and those whose flags change as it blows
//!   (`"stateChange"`), else the nearest (`"nearest"`). Who shot it is the
//!   last body its damage list named in the 30 s before (`"read"`), else
//!   the thrower of an object that ended within 5 m in the 0.7 s before
//!   (`"explosion"`).
//! - A metal detector sounds with effect 418737478887 for 3.0 s, and has
//!   effect 418737478950 while it is switched off (seen in rounds with
//!   Mute). The effects are on light objects that are no map objects; the
//!   detector is the map object with the next lower id of the same id
//!   family (`objectSource: "idOrder"`, or `"table"` when the per-map
//!   table has it). Who walked through is not written: `by` is the nearest
//!   body within 1.5 m horizontally (`bySource: "nearest"`).
//!
//! # Light screens
//!
//! Each post an R.O.U. Projector System drops (entity asset 377423931277)
//! carries effect 385282566546 while its light is up. A post has no
//! owner. Its light comes up as the projector rolls past, so posts are
//! grouped by the thrown projector that is within 0.5 m of the post within
//! 0.3 s of that moment, and take its owner (`usernameSource:
//! "proximity"`); `throw` is the throw of [`crate::throws`] that let that
//! projector go. With no projector seen passing, the roll is the R.O.U.
//! throw in the 12 s before whose path passes within 1.5 m of the post or
//! which ended within 9 m, the nearest of them.
//!
//! # For other decoders
//!
//! [`inside`] says whether a body stands in a fire, gas or swarm area and
//! [`through_smoke`] whether a shot passes through a smoke cloud; the
//! burst of a fire extinguisher is not smoke and does not count.
//! [`crate::join`] puts both on the kills, hits and shots.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::details::Loadout;
use crate::fx::{Effects, Spawn};
use crate::loadout::{Input, When};
use crate::shots::Shot;
use crate::throws::Throw;
use crate::types::{EnvObject, env_object, env_objects};
use crate::world::{Entity, World};

/// Effect assets of areas.
const SMOKE_GRENADE: u64 = 27012166679;
const SMOKE_BOLT: u64 = 73007452394;
const FIRE: u64 = 223502673865;
const FIRE_CELL: u64 = 440770340286;
const GAS: u64 = 297391115747;
const SWARM: u64 = 379510214638;
const EXTINGUISHER: u64 = 401787054484;
/// A gas pipe blowing up; its fire follows.
const PIPE_EXPLOSION: u64 = 407368224089;
/// A metal detector sounding, and switched off.
const DETECTOR_ALARM: u64 = 418737478887;
const DETECTOR_OFF: u64 = 418737478950;
/// The light of an R.O.U. post.
const SCREEN: u64 = 385282566546;

/// Entity assets: the area entity of a Logic Bomb's fire, Goyo's placed
/// canister and a post of an R.O.U. Projector System.
const LOGIC_BOMB: u64 = 319577383929;
const VOLCAN: u64 = 375624856243;
/// `(asset, what made the area)` of the entities without classes the game
/// puts at the centre of an area.
const AREA_ENTITIES: [(u64, &str); 7] = [
    (339570061825, "Volcan Canister"),
    (361809340830, "Shumikha Grenade"),
    (361809343208, "Fire Bolt"),
    (407899098848, "Gas Pipe"),
    (361147040848, "Remote Gas Grenade"),
    (385049618011, "Kawan Hive"),
    (LOGIC_BOMB, "Logic Bomb"),
];
const GAS_PIPE: &str = "Gas Pipe";

/// Radii the file does not give (metres).
const SMOKE_RADIUS: f32 = 3.0;
const EXTINGUISHER_RADIUS: f32 = 2.5;
/// A cell of a fire, gas or swarm area reaches this far horizontally
/// (damage was taken up to 0.90 m from one), and from this far below a
/// body's origin to this far above it.
const CELL_REACH: f64 = 1.0;
const CELL_BELOW: f64 = 1.0;
const CELL_ABOVE: f64 = 2.2;

/// Pooled objects wait at z = -100; a position within this of it is not a
/// place in the world.
const POOL_Z: f32 = -100.0;
/// The area entity is placed this long before to this long after its
/// effect starts (seconds, both exclusive) and this close to it.
const ENTITY_LEAD: f64 = 0.7;
const ENTITY_LAG: f64 = 0.2;
const ENTITY_REACH: f64 = 0.1;
/// The gadget an area came from was this close to it, a twentieth of a
/// second after the area started.
const GADGET_REACH: f64 = 0.5;
const GADGET_SETTLE: f64 = 0.05;
/// It was deleted from this long before the area started to this long
/// after, or within this of the area's stop.
const GADGET_LEAD: f64 = 0.7;
const GADGET_LAG: f64 = 0.1;
const BOLT_LAG: f64 = 0.3;
/// A hive is within this of its swarm.
const HIVE_REACH: f64 = 1.0;
/// A Logic Bomb's fire starts this long after the ability was used.
const LOGIC_BOMB_DELAY: (f64, f64) = (7.0, 8.2);
/// A shot that set off a canister ended this close to its fire, from this
/// long before the canister was deleted to this long after.
const SHOT_REACH: f64 = 0.6;
const SHOT_LEAD: f64 = 1.0;
const SHOT_LAG: f64 = 0.1;
/// An explosive that set something off ended this close to it, and for a
/// canister within this of its deletion.
const BLAST_REACH: f64 = 5.0;
const BLAST_WINDOW: f64 = 0.6;
/// A fire extinguisher's shooter is in its damage list this long before
/// the burst, a gas pipe's this long before the explosion.
const EXTINGUISHER_LEAD: f64 = 1.0;
const PIPE_LEAD: f64 = 30.0;
const DAMAGE_LAG: f64 = 0.1;
/// A gas pipe is this close to its explosion, horizontally and in height.
const PIPE_REACH: f64 = 1.5;
const PIPE_HEIGHT: f64 = 6.0;
/// Its flags change from this long before the explosion to this long
/// after.
const PIPE_CHANGE: (f64, f64) = (-0.1, 0.6);
/// Its fire starts within this of the explosion, in time and place.
const PIPE_FIRE: f64 = 1.0;
const PIPE_FIRE_REACH: f64 = 0.5;
/// An explosive that blew a pipe up ended this long before it at most.
const PIPE_BLAST_LEAD: f64 = 0.7;
/// Map objects of one prefab share the upper bits of their id.
const FAMILY_SHIFT: u32 = 16;
/// Who sets off a metal detector is within this of it horizontally (73 of
/// 73 alarms), on its floor.
const DETECTOR_REACH: f64 = 1.5;
const DETECTOR_HEIGHT: f64 = 2.5;
/// Two kept positions of a body this far apart at most are one movement
/// (seconds).
const BODY_GAP: f64 = 0.5;
/// A post's light comes up as the projector rolls past: within this many
/// seconds of the projector being this close.
const ROLLER_TIME: f64 = 0.3;
const ROLLER_REACH: f64 = 0.5;
/// A throw starts where its object was, this exactly.
const RELEASE_TIME: f64 = 0.05;
const RELEASE_REACH: f64 = 0.1;
/// Without a projector seen passing: a post's light comes up this long
/// after its roll at most, with the roll's path this close to the post or
/// its end this close.
const ROLL_WINDOW: f64 = 12.0;
const ROLL_PATH_REACH: f64 = 1.5;
const ROLL_END_REACH: f64 = 9.0;
/// Posts no roll explains are one screen while they come up this close
/// after each other (seconds).
const POST_STEP: f64 = 1.0;
const ROLL_NAME: &str = "R.O.U";
const LOGIC_BOMB_NAME: &str = "Logic Bomb";

/// What an area is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AreaKind {
    #[default]
    Smoke,
    Fire,
    Gas,
    Swarm,
    /// The cloud of a fire extinguisher.
    Extinguisher,
}

/// One cloud or burning floor.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Area {
    pub kind: AreaKind,
    /// What made it: `Smoke Grenade`, `Volcan Canister`, `Gas Pipe`, ...
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<&'static str>,
    /// The source was never seen with what it is named after.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub source_inferred: bool,
    /// Whose gadget made it, and how that is known: `proximity` (their
    /// gadget ended there), `nearest` (their gadget is there) or
    /// `abilityUse`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username_source: Option<&'static str>,
    /// Where the effect is, in world metres.
    pub position: [f32; 3],
    /// The alliance of whoever set it off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alliance: Option<u32>,
    /// The cells of a fire, gas or swarm area as it was largest, and how
    /// many it could have had.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity: Option<u32>,
    /// Metres. Not in the file: `radiusSource` is `assumed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius_source: Option<&'static str>,
    /// The entity the game put at the centre of the area, in hex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    /// The gadget entity it came from, in hex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_entity: Option<String>,
    /// The fire extinguisher that burst, in hex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    pub started: When,
    /// When the effect stopped. Absent for one still there as the
    /// recording ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<When>,
    /// Who set off the canister, pipe or extinguisher, and how that is
    /// known: `read`, `shotRay` or `explosion`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triggered_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_source: Option<&'static str>,
    /// The thrown object an `explosion` trigger is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triggered_with: Option<&'static str>,
    /// Seconds since the recording started of its start and its end, for
    /// [`inside`] and [`through_smoke`].
    #[serde(skip)]
    pub start_seconds: f64,
    #[serde(skip)]
    pub end_seconds: Option<f64>,
}

impl Area {
    /// The area was there `seconds` into the recording.
    fn alive(&self, seconds: f64) -> bool {
        self.start_seconds <= seconds && self.end_seconds.is_none_or(|end| seconds <= end)
    }
}

/// The area a player stood in: which entry of `areas[]`, what it is and
/// whose. Derived from the body's place and the area's cells.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InArea {
    /// Index in `areas`.
    pub area: usize,
    pub kind: AreaKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<&'static str>,
    /// Whose gadget made the area.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

/// What an environment event is about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EnvironmentKind {
    #[default]
    GasPipe,
    FireExtinguisher,
    MetalDetector,
}

/// A gas pipe blowing up, a fire extinguisher bursting, or a metal
/// detector sounding or being switched off. `when` is that moment.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentEvent {
    pub kind: EnvironmentKind,
    /// A metal detector's `alarm` or `off`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<&'static str>,
    /// The map object, in hex, and how it is known: `read` (the effect is
    /// on it), `table`, `stateChange`, `nearest` or `idOrder`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_source: Option<&'static str>,
    /// Where the object is; for a gas pipe, where it blew up.
    pub position: [f32; 3],
    /// The alliance of whoever blew the pipe up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alliance: Option<u32>,
    /// Who set it off, and how that is known: `read`, `explosion` or
    /// `nearest`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_source: Option<&'static str>,
    /// The thrown object an `explosion` is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_with: Option<&'static str>,
    /// Metres between the detector and the nearest body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<f32>,
    /// Index in `areas` of the pipe's fire or the extinguisher's cloud.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<usize>,
    #[serde(flatten)]
    pub when: When,
    /// When the alarm, the switched-off state or the cloud ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<When>,
}

/// One post of a light screen.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    /// The post's entity, in hex.
    pub entity: String,
    pub position: [f32; 3],
    /// When its light came up, and when it went out.
    pub started: When,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<When>,
}

/// The posts one roll of an R.O.U. Projector System dropped.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LightScreen {
    /// Who rolled it: the owner of the projector, else the thrower of
    /// `throw`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username_source: Option<&'static str>,
    /// The projector that rolled, in hex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    /// Index of the roll in `throws`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throw: Option<usize>,
    pub posts: Vec<Post>,
    /// When the first light came up.
    pub started: When,
    /// When the last one went out. Absent while a post is still lit as
    /// the recording ends.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<When>,
}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    pub areas: Vec<Area>,
    pub environment: Vec<EnvironmentEvent>,
    pub light_screens: Vec<LightScreen>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
}

/// The index of the first fire, gas or swarm area a body at `position`
/// (its origin, at the feet) stands in `seconds` into the recording:
/// within 1.0 m horizontally of one of its cells, the cell from 1.0 m
/// below the origin to 2.2 m above it. Fitted on the fire and gas damage
/// of the test rounds, all of which is taken in such an area.
pub(crate) fn inside(areas: &[Area], position: [f32; 3], seconds: f64) -> Option<usize> {
    let covers = |cell: &[f32; 3]| {
        let across = f64::from(cell[0] - position[0]).hypot(f64::from(cell[1] - position[1]));
        let up = f64::from(cell[2] - position[2]);
        across <= CELL_REACH && (-CELL_BELOW..=CELL_ABOVE).contains(&up)
    };
    let holds = |a: &Area| a.alive(seconds) && a.points.iter().any(covers);
    areas.iter().position(holds)
}

/// A shot from `origin` along the unit vector `direction` for `distance`
/// metres passes through a smoke cloud (of a grenade or a bolt) that is
/// there `seconds` into the recording: within the cloud's assumed radius
/// of its centre. The burst of a fire extinguisher is no smoke and does
/// not count.
pub(crate) fn through_smoke(
    areas: &[Area],
    origin: [f64; 3],
    direction: [f64; 3],
    distance: f64,
    seconds: f64,
) -> bool {
    areas.iter().any(|a| {
        let (AreaKind::Smoke, Some(radius)) = (a.kind, a.radius) else {
            return false;
        };
        let centre = wide(a.position);
        // The point of the segment nearest the centre.
        let along: f64 = (0..3).map(|i| (centre[i] - origin[i]) * direction[i]).sum();
        let along = along.clamp(0.0, distance.max(0.0));
        let nearest = [0, 1, 2].map(|i| origin[i] + direction[i] * along);
        a.alive(seconds) && apart(nearest, centre) <= f64::from(radius)
    })
}

fn wide(p: [f32; 3]) -> [f64; 3] {
    p.map(f64::from)
}

/// Metres between two points.
fn apart(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
}

/// The same, seen from above.
fn across(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn hex(id: u64) -> String {
    format!("{id:08x}")
}

/// What the decoder reads besides the world and the effects.
pub(crate) struct Context<'a> {
    pub shots: &'a [Shot],
    pub throws: &'a [Throw],
    /// For the uses of a Logic Bomb.
    pub loadouts: &'a [Loadout],
    /// The map's id, for the table of its environment objects.
    pub map: u64,
}

/// The places an entity was moved to, with when, and what it is.
struct Track {
    id: u64,
    asset: u64,
    /// `(seconds, position)` of every position written that is not the
    /// pool's, in stream order.
    points: Vec<(f64, [f64; 3])>,
    deleted: Option<f64>,
    /// Index of the player its components name.
    owner: Option<usize>,
}

impl Track {
    /// Where it was `seconds` into the recording.
    fn at(&self, seconds: f64) -> Option<[f64; 3]> {
        let i = self.points.partition_point(|p| p.0 <= seconds);
        self.points.get(i.checked_sub(1)?).map(|p| p.1)
    }
}

/// An area before it is placed on the clock.
struct Found {
    area: Area,
    frame: Option<u32>,
    stopped: Option<u32>,
    /// The object its effect is on, for a fire extinguisher's cloud.
    extinguisher: Option<u64>,
}

/// An environment event before it is placed on the clock.
struct Event {
    event: EnvironmentEvent,
    seconds: f64,
    frame: Option<u32>,
    stopped: Option<u32>,
}

/// A post before it is placed on the clock.
struct Lit {
    entity: u64,
    position: [f32; 3],
    seconds: f64,
    frame: Option<u32>,
    stopped: Option<u32>,
    /// The projector that dropped it, and the player that one names.
    roller: Option<u64>,
    owner: Option<usize>,
    throw: Option<usize>,
}

/// The round as the joins see it.
struct Scene<'a> {
    world: &'a World,
    fx: &'a Effects,
    context: &'a Context<'a>,
    /// `(playerid, username)` in the order of the player table.
    players: &'a [(u64, String)],
    /// Seconds since the recording started of a frame.
    seconds: &'a dyn Fn(Option<u32>) -> Option<f64>,
    /// Entities with a place in the world, in creation order.
    tracks: Vec<Track>,
    by_id: HashMap<u64, usize>,
    /// Frames the recording has no time for.
    untimed: usize,
}

impl<'a> Scene<'a> {
    fn new(
        world: &'a World,
        fx: &'a Effects,
        context: &'a Context<'a>,
        players: &'a [(u64, String)],
        seconds: &'a dyn Fn(Option<u32>) -> Option<f64>,
    ) -> Self {
        let mut scene = Scene {
            world,
            fx,
            context,
            players,
            seconds,
            tracks: Vec::new(),
            by_id: HashMap::new(),
            untimed: 0,
        };
        // Tracks are only needed in a round with something to join.
        let wanted = |s: &Spawn| area_kind(s.asset).is_some() || s.asset == SCREEN;
        if !fx.spawns.iter().any(wanted) {
            return scene;
        }
        for e in world.iter().filter(|e| !e.is_map()) {
            let points: Vec<(f64, [f64; 3])> = e
                .changes
                .iter()
                .filter_map(|c| {
                    let p = c.position.filter(|p| (p[2] - POOL_Z).abs() > 1.0)?;
                    Some(((seconds)(c.frame)?, wide(p)))
                })
                .collect();
            if points.is_empty() {
                continue;
            }
            scene.by_id.insert(e.id, scene.tracks.len());
            scene.tracks.push(Track {
                id: e.id,
                asset: e.asset,
                points,
                deleted: e.deleted.and_then(|f| (seconds)(Some(f))),
                owner: owner_of(e, players),
            });
        }
        scene
    }

    /// Seconds of `frame`, counting the frames without a time.
    fn time(&mut self, frame: Option<u32>) -> Option<f64> {
        let t = (self.seconds)(frame);
        if t.is_none() {
            self.untimed += 1;
        }
        t
    }

    fn username(&self, player: usize) -> Option<String> {
        self.players.get(player).map(|p| p.1.clone())
    }

    fn track(&self, id: u64) -> Option<&Track> {
        self.tracks.get(*self.by_id.get(&id)?)
    }

    /// Gadget entities: those with a place that are not area entities.
    /// Bodies, guns and attachments have no track.
    fn gadgets(&self) -> impl Iterator<Item = &Track> {
        self.tracks.iter().filter(|t| source_of(t.asset).is_none())
    }
}

/// The player an entity's placed or owner component names.
fn owner_of(entity: &Entity, players: &[(u64, String)]) -> Option<usize> {
    entity.changes.iter().find_map(|c| {
        let named = [
            c.placed.and_then(|p| p.owner),
            c.owner.and_then(|o| o.player),
        ];
        let known = |id: u64| players.iter().position(|p| p.0 == id);
        named.into_iter().flatten().find_map(known)
    })
}

fn area_kind(asset: u64) -> Option<AreaKind> {
    Some(match asset {
        SMOKE_GRENADE | SMOKE_BOLT => AreaKind::Smoke,
        FIRE | FIRE_CELL => AreaKind::Fire,
        GAS => AreaKind::Gas,
        SWARM => AreaKind::Swarm,
        EXTINGUISHER => AreaKind::Extinguisher,
        _ => return None,
    })
}

/// What made the area an area entity of `asset` is at the centre of.
fn source_of(asset: u64) -> Option<&'static str> {
    AREA_ENTITIES.iter().find(|a| a.0 == asset).map(|a| a.1)
}

/// The last body with a player that the damage list of `object` named
/// between `from` and `to` seconds, as an index into the player table.
fn instigator(scene: &Scene, object: &Entity, from: f64, to: f64) -> Option<usize> {
    let timed = |frame: Option<u32>| (scene.seconds)(frame).is_some_and(|t| from <= t && t <= to);
    let changes = object.changes.iter().filter(|c| timed(c.frame));
    let named = changes.flat_map(|c| &c.damage);
    named
        .filter_map(|d| scene.world.player_of(d.instigator))
        .next_back()
}

/// `(seconds it ended, where)` of a throw that ended.
fn ending(throw: &Throw) -> Option<(f64, [f64; 3])> {
    let ended = throw.when.recording_time? + throw.ended_after?;
    Some((ended, throw.end?))
}

/// The last thrown object that ended within [`BLAST_REACH`] of `position`
/// and from `lead` seconds before `seconds` to `lag` after.
fn blast(
    throws: &[Throw],
    position: [f64; 3],
    seconds: f64,
    lead: f64,
    lag: f64,
) -> Option<&Throw> {
    let near = |t: &&Throw| {
        ending(t).is_some_and(|(ended, end)| {
            let d = ended - seconds;
            -lead <= d && d <= lag && apart(end, position) < BLAST_REACH
        })
    };
    throws.iter().rfind(near)
}

/// The areas of the round, in the order their effects started.
fn areas(scene: &mut Scene) -> Vec<Found> {
    let mut out = Vec::new();
    let fx = scene.fx;
    for x in &fx.spawns {
        let (Some(kind), Some(_), Some(position)) = (area_kind(x.asset), x.frame, x.position)
        else {
            continue;
        };
        let Some(start) = scene.time(x.frame) else {
            continue;
        };
        let stop = x.stopped.and_then(|f| scene.time(Some(f)));
        let place = wide(position);
        let mut area = Area {
            kind,
            position,
            alliance: x.alliance,
            start_seconds: start,
            end_seconds: stop,
            ..Area::default()
        };
        if !x.points.is_empty() {
            area.points = x.points.clone();
            area.capacity = Some(x.capacity);
        } else if let Some(radius) = match kind {
            AreaKind::Smoke => Some(SMOKE_RADIUS),
            AreaKind::Extinguisher => Some(EXTINGUISHER_RADIUS),
            _ => None,
        } {
            area.radius = Some(radius);
            area.radius_source = Some("assumed");
        }

        // The area entity: put at the effect's position in the same frame.
        let centre = scene.tracks.iter().find(|t| {
            let first = t.points.first();
            source_of(t.asset).is_some()
                && first.is_some_and(|&(placed, p)| {
                    let d = placed - start;
                    -ENTITY_LEAD < d && d < ENTITY_LAG && apart(p, place) < ENTITY_REACH
                })
        });
        if let Some(centre) = centre {
            area.entity = Some(hex(centre.id));
            area.source = source_of(centre.asset);
            area.source_inferred = centre.asset == LOGIC_BOMB;
        } else {
            area.source = match x.asset {
                SMOKE_GRENADE => Some("Smoke Grenade"),
                SMOKE_BOLT => Some("Smoke Bolt"),
                EXTINGUISHER => Some("Fire Extinguisher"),
                _ => None,
            };
        }

        // The gadget it came from: one that ended at that place then, or
        // a bolt that goes when the area does. One with an owner goes
        // before one without. The fire of a pipe and the cloud of an
        // extinguisher come from the map: a gadget that ends there is
        // one they destroyed.
        let made = kind != AreaKind::Extinguisher && area.source != Some(GAS_PIPE);
        let mut gadget: Option<&Track> = None;
        let mut how = "proximity";
        for t in scene.gadgets().filter(|_| made) {
            let there = t.at(start + GADGET_SETTLE);
            if !there.is_some_and(|p| apart(p, place) <= GADGET_REACH) {
                continue;
            }
            let Some(deleted) = t.deleted else {
                continue;
            };
            let ended = -GADGET_LEAD <= deleted - start && deleted - start <= GADGET_LAG;
            let bolt = stop.is_some_and(|stop| (deleted - stop).abs() < BOLT_LAG);
            let better = gadget.is_none_or(|g| g.owner.is_none() && t.owner.is_some());
            if (ended || bolt) && better {
                gadget = Some(t);
            }
        }
        if gadget.is_none() && kind == AreaKind::Swarm {
            // A hive stays where it stuck while its swarm is out.
            let mut nearest = f64::INFINITY;
            for t in scene.gadgets() {
                let gone = t.deleted.is_some_and(|d| d < start);
                let Some(p) = t.at(start + GADGET_SETTLE).filter(|_| !gone) else {
                    continue;
                };
                let d = apart(p, place);
                if d <= HIVE_REACH && t.owner.is_some() && d < nearest {
                    (gadget, nearest, how) = (Some(t), d, "nearest");
                }
            }
        }
        if centre.is_some_and(|c| c.asset == LOGIC_BOMB) {
            // Inferred: the Logic Bomb used 7.6 s before (7 of 7).
            let used = |l: &&Loadout| {
                let ability = l.ability.as_ref();
                let bomb = ability.filter(|a| a.name == Some(LOGIC_BOMB_NAME));
                let uses = bomb.and_then(|a| a.counts.as_ref()).map(|c| &c.uses);
                uses.into_iter().flatten().any(|u| {
                    let since = u.recording_time.map(|t| start - t);
                    since.is_some_and(|s| LOGIC_BOMB_DELAY.0 < s && s < LOGIC_BOMB_DELAY.1)
                })
            };
            if let Some(l) = scene.context.loadouts.iter().rfind(used) {
                area.username = Some(l.username.clone());
                area.username_source = Some("abilityUse");
            }
        }
        if let Some(g) = gadget {
            area.source_entity = Some(hex(g.id));
            if let Some(owner) = g.owner.and_then(|o| scene.username(o)) {
                area.username = Some(owner);
                area.username_source = Some(how);
            }
            if let (VOLCAN, Some(deleted)) = (g.asset, g.deleted) {
                trigger(&mut area, scene.context, place, deleted);
            }
        }
        if kind == AreaKind::Extinguisher {
            area.object = Some(hex(x.parent));
            let object = scene.world.get(x.parent);
            let by = object
                .and_then(|o| instigator(scene, o, start - EXTINGUISHER_LEAD, start + DAMAGE_LAG));
            if let Some(by) = by.and_then(|b| scene.username(b)) {
                area.triggered_by = Some(by);
                area.trigger_source = Some("read");
            }
        }
        out.push(Found {
            area,
            frame: x.frame,
            stopped: x.stopped.filter(|_| stop.is_some()),
            extinguisher: (kind == AreaKind::Extinguisher).then_some(x.parent),
        });
    }
    out
}

/// Who set off the Volcan canister that was deleted `deleted` seconds
/// into the recording and burns at `place`: the file does not say, so the
/// shot that ended on it, else an explosive that went off next to it.
fn trigger(area: &mut Area, context: &Context, place: [f64; 3], deleted: f64) {
    let on_it = |s: &&Shot| {
        let Some(fired) = s.when.recording_time else {
            return false;
        };
        let reach = f64::from(s.distance);
        let (origin, direction) = (wide(s.origin), wide(s.direction));
        let end = [0, 1, 2].map(|i| origin[i] + direction[i] * reach);
        let timed = deleted - SHOT_LEAD <= fired && fired <= deleted + SHOT_LAG;
        timed && s.username.is_some() && apart(end, place) < SHOT_REACH
    };
    if let Some(shot) = context.shots.iter().rfind(on_it) {
        area.triggered_by = shot.username.clone();
        area.trigger_source = Some("shotRay");
        return;
    }
    // The window is open at both ends for a canister.
    let near = |t: &&Throw| {
        ending(t).is_some_and(|(ended, end)| {
            (ended - deleted).abs() < BLAST_WINDOW && apart(end, place) < BLAST_REACH
        })
    };
    if let Some(throw) = context.throws.iter().rfind(near) {
        area.triggered_by = Some(throw.username.clone());
        area.trigger_source = Some("explosion");
        area.triggered_with = throw.name;
    }
}

/// Where `body` was in `frame`. A body's track keeps a position every
/// tenth of a second: between two of them it is on the line from one to
/// the other.
fn body_at(scene: &Scene, body: u64, frame: u32) -> Option<[f64; 3]> {
    let track = scene.world.bodies.get(&body)?;
    let next = track.partition_point(|p| p.0 <= frame);
    let &(before, from) = track.get(next.checked_sub(1)?)?;
    let from = wide(from);
    let Some(&(after, to)) = track.get(next) else {
        return Some(from);
    };
    let time = |frame: u32| (scene.seconds)(Some(frame));
    let (Some(start), Some(now), Some(end)) = (time(before), time(frame), time(after)) else {
        return Some(from);
    };
    // A longer gap is a body that stood still, or was not there.
    if end - start > BODY_GAP || end <= start {
        return Some(from);
    }
    let share = ((now - start) / (end - start)).clamp(0.0, 1.0);
    let to = wide(to);
    Some([0, 1, 2].map(|i| from[i] + (to[i] - from[i]) * share))
}

/// The id family most map objects are of.
fn main_family(world: &World) -> Option<u64> {
    let mut counts: HashMap<u64, usize> = HashMap::new();
    let mut best: Option<(u64, usize)> = None;
    for e in world.iter().filter(|e| e.is_map()) {
        *counts.entry(e.id >> FAMILY_SHIFT).or_default() += 1;
    }
    // The first family created wins a tie.
    for e in world.iter().filter(|e| e.is_map()) {
        let family = e.id >> FAMILY_SHIFT;
        let n = counts.get(&family).copied().unwrap_or(0);
        if best.is_none_or(|b| n > b.1) {
            best = Some((family, n));
        }
    }
    best.map(|b| b.0)
}

/// The gas pipe that blew up at `place`, `seconds` into the recording,
/// and how it was picked.
fn pipe<'a>(
    scene: &Scene<'a>,
    place: [f64; 3],
    seconds: f64,
) -> Option<(&'a Entity, &'static str)> {
    let world = scene.world;
    let near = |e: &&Entity| {
        let home = wide(e.position);
        e.is_map() && across(home, place) <= PIPE_REACH && (home[2] - place[2]).abs() <= PIPE_HEIGHT
    };
    let by_distance = |a: &&Entity, b: &&Entity| {
        let (a, b) = (
            across(wide(a.position), place),
            across(wide(b.position), place),
        );
        a.total_cmp(&b)
    };
    let known = env_objects(scene.context.map, EnvObject::GasPipe);
    let known = known.filter_map(|id| world.get(id)).filter(near);
    if let Some(found) = known.min_by(by_distance) {
        return Some((found, "table"));
    }
    // Walls are that close too. Pipes come from small prefab families, so
    // objects outside the map's largest family go first, then those whose
    // flags change as the pipe blows, then the nearest.
    let main = main_family(world);
    let changed = |e: &Entity| {
        e.changes.iter().any(|c| {
            let flags_only = c.flags.is_some()
                && c.position.is_none()
                && c.rotation.is_none()
                && c.live.is_none()
                && c.damage.is_empty()
                && !c.destroyed
                && c.rest.is_none();
            let d = (scene.seconds)(c.frame).map(|t| t - seconds);
            flags_only && d.is_some_and(|d| PIPE_CHANGE.0 <= d && d <= PIPE_CHANGE.1)
        })
    };
    let rank = |e: &&Entity| (Some(e.id >> FAMILY_SHIFT) == main, !changed(e));
    let found = world.iter().filter(near).min_by(|a, b| {
        let order = rank(a).cmp(&rank(b));
        order.then_with(|| by_distance(a, b))
    })?;
    let how = if changed(found) {
        "stateChange"
    } else {
        "nearest"
    };
    Some((found, how))
}

/// Fire extinguishers, gas pipes and metal detectors, by time. `found`
/// are the round's areas; a pipe's fire takes who blew the pipe up.
fn environment(scene: &mut Scene, found: &mut [Found]) -> Vec<Event> {
    let mut out = Vec::new();
    let (fx, world, context) = (scene.fx, scene.world, scene.context);

    for (i, f) in found.iter().enumerate() {
        let Some(object) = f.extinguisher.and_then(|id| world.get(id)) else {
            continue;
        };
        out.push(Event {
            event: EnvironmentEvent {
                kind: EnvironmentKind::FireExtinguisher,
                object: Some(hex(object.id)),
                object_source: Some("read"),
                position: object.position,
                by: f.area.triggered_by.clone(),
                by_source: f.area.trigger_source,
                area: Some(i),
                ..EnvironmentEvent::default()
            },
            seconds: f.area.start_seconds,
            frame: f.frame,
            stopped: f.stopped,
        });
    }

    for x in fx.of(PIPE_EXPLOSION) {
        let (Some(_), Some(position)) = (x.frame, x.position) else {
            continue;
        };
        let Some(seconds) = scene.time(x.frame) else {
            continue;
        };
        let place = wide(position);
        let mut event = EnvironmentEvent {
            kind: EnvironmentKind::GasPipe,
            position,
            alliance: x.alliance,
            ..EnvironmentEvent::default()
        };
        if let Some((object, how)) = pipe(scene, place, seconds) {
            event.object = Some(hex(object.id));
            event.object_source = Some(how);
            let by = instigator(scene, object, seconds - PIPE_LEAD, seconds + DAMAGE_LAG);
            if let Some(by) = by.and_then(|b| scene.username(b)) {
                event.by = Some(by);
                event.by_source = Some("read");
            }
        }
        if event.by.is_none() {
            // No bullet named a player: an explosive that went off next
            // to it.
            let throws = context.throws;
            if let Some(t) = blast(throws, place, seconds, PIPE_BLAST_LEAD, DAMAGE_LAG) {
                event.by = Some(t.username.clone());
                event.by_source = Some("explosion");
                event.by_with = t.name;
            }
        }
        let fire = found.iter().position(|f| {
            let d = f.area.start_seconds - seconds;
            f.area.kind == AreaKind::Fire
                && f.area.source == Some(GAS_PIPE)
                && (0.0..PIPE_FIRE).contains(&d)
                && apart(wide(f.area.position), place) < PIPE_FIRE_REACH
        });
        if let Some(f) = fire.and_then(|i| found.get_mut(i)) {
            event.area = fire;
            if event.by.is_some() {
                f.area.triggered_by = event.by.clone();
                f.area.trigger_source = event.by_source;
                f.area.triggered_with = event.by_with;
            }
        }
        out.push(Event {
            event,
            seconds,
            frame: x.frame,
            stopped: None,
        });
    }

    // Metal detectors: the effect is on a light object whose id follows
    // the detector's.
    let mut map_objects: Vec<u64> = world.iter().filter(|e| e.is_map()).map(|e| e.id).collect();
    map_objects.sort_unstable();
    let mut seen = HashSet::new();
    for x in &fx.spawns {
        let alarm = x.asset == DETECTOR_ALARM;
        if !(alarm || x.asset == DETECTOR_OFF) || x.frame.is_none() {
            continue;
        }
        let below = map_objects
            .partition_point(|&id| id <= x.parent)
            .checked_sub(1);
        let Some(&detector) = below.and_then(|i| map_objects.get(i)) else {
            continue;
        };
        if detector >> FAMILY_SHIFT != x.parent >> FAMILY_SHIFT {
            continue;
        }
        let (Some(object), Some(seconds)) = (world.get(detector), scene.time(x.frame)) else {
            continue;
        };
        // The effect is on each light of the detector: one event a tenth
        // of a second.
        if !seen.insert((detector, alarm, (seconds * 10.0).round() as i64)) {
            continue;
        }
        let listed = env_object(context.map, detector) == Some(EnvObject::MetalDetector);
        let mut event = EnvironmentEvent {
            kind: EnvironmentKind::MetalDetector,
            event: Some(if alarm { "alarm" } else { "off" }),
            object: Some(hex(detector)),
            object_source: Some(if listed { "table" } else { "idOrder" }),
            position: object.position,
            ..EnvironmentEvent::default()
        };
        if let (true, Some(frame)) = (alarm, x.frame) {
            let home = wide(object.position);
            let bodies = (0..scene.players.len()).filter_map(|player| {
                let at = body_at(scene, world.body_of(player)?, frame)?;
                let level = (at[2] - home[2]).abs() < DETECTOR_HEIGHT;
                level.then(|| (across(at, home), player))
            });
            let nearest = bodies.min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((distance, player)) = nearest.filter(|n| n.0 <= DETECTOR_REACH) {
                event.by = scene.username(player);
                event.by_source = event.by.as_ref().map(|_| "nearest");
                event.distance = Some((distance * 100.0).round() as f32 / 100.0);
            }
        }
        out.push(Event {
            event,
            seconds,
            frame: x.frame,
            stopped: x.stopped,
        });
    }
    out.sort_by(|a, b| a.seconds.total_cmp(&b.seconds));
    out
}

/// The projector that dropped a post at `place` whose light came up
/// `seconds` into the recording: the thrown R.O.U. entity that rolled
/// past that place at that time.
fn roller<'s>(scene: &'s Scene, place: [f64; 3], seconds: f64) -> Option<&'s Track> {
    let throws = scene.context.throws;
    let rolled = |asset: u64| throws.iter().any(|t| is_roll(t) && t.asset == asset);
    let passes = scene.tracks.iter().filter_map(|t| {
        let from = t.points.partition_point(|p| p.0 < seconds - ROLLER_TIME);
        let then = t.points.get(from..)?.iter();
        let then = then.take_while(|p| p.0 <= seconds + ROLLER_TIME);
        let nearest = then
            .map(|p| apart(p.1, place))
            .fold(f64::INFINITY, f64::min);
        (nearest <= ROLLER_REACH && rolled(t.asset)).then_some((t, nearest))
    });
    passes.min_by(|a, b| a.1.total_cmp(&b.1)).map(|p| p.0)
}

fn is_roll(throw: &Throw) -> bool {
    throw.name.is_some_and(|n| n.starts_with(ROLL_NAME))
}

/// The throw that let `roller` go, for a post lit `seconds` into the
/// recording: the last one before that started where the entity was.
fn release(throws: &[Throw], roller: &Track, seconds: f64) -> Option<usize> {
    throws.iter().rposition(|t| {
        let (Some(thrown), Some(origin)) = (t.when.recording_time, t.origin) else {
            return false;
        };
        let from = roller
            .points
            .partition_point(|p| p.0 < thrown - RELEASE_TIME);
        let then = roller.points.get(from..).into_iter().flatten();
        let mut then = then.take_while(|p| p.0 <= thrown + RELEASE_TIME);
        is_roll(t)
            && t.asset == roller.asset
            && thrown <= seconds
            && then.any(|p| apart(p.1, origin) < RELEASE_REACH)
    })
}

/// The roll that dropped a post no projector was seen passing: the
/// nearest of the rolls in the 12 s before that pass or end near it, as
/// an index into the throws.
fn roll(throws: &[Throw], place: [f64; 3], seconds: f64) -> Option<usize> {
    let rolls = throws.iter().enumerate().filter_map(|(i, t)| {
        let thrown = t.when.recording_time?;
        if !is_roll(t) || !(thrown <= seconds && seconds <= thrown + ROLL_WINDOW) {
            return None;
        }
        let path = t.path.iter().map(|p| apart([p[1], p[2], p[3]], place));
        let path = path.fold(f64::INFINITY, f64::min);
        let end = t.end.map_or(f64::INFINITY, |e| apart(e, place));
        (path < ROLL_PATH_REACH || end < ROLL_END_REACH).then_some((i, path.min(end)))
    });
    rolls.min_by(|a, b| a.1.total_cmp(&b.1)).map(|r| r.0)
}

/// The posts of the round's light screens, in the order they lit up.
fn posts(scene: &mut Scene) -> Vec<Lit> {
    let mut out = Vec::new();
    let fx = scene.fx;
    for x in fx.of(SCREEN) {
        let first = scene.track(x.parent).and_then(|t| t.points.first());
        let (Some(_), Some(&(_, place))) = (x.frame, first) else {
            continue;
        };
        let Some(seconds) = scene.time(x.frame) else {
            continue;
        };
        let throws = scene.context.throws;
        let roller = roller(scene, place, seconds);
        let throw = match roller {
            Some(roller) => release(throws, roller, seconds),
            None => roll(throws, place, seconds),
        };
        out.push(Lit {
            entity: x.parent,
            position: place.map(|v| v as f32),
            seconds,
            frame: x.frame,
            stopped: x.stopped,
            roller: roller.map(|r| r.id),
            owner: roller.and_then(|r| r.owner),
            throw,
        });
    }
    out
}

/// Y11S3: the areas, environment events and light screens of a round.
pub(crate) fn decode(input: &Input, world: &World, fx: &Effects, context: &Context) -> Decoded {
    let players: Vec<(u64, String)> = input
        .players
        .iter()
        .map(|p| (p.id, p.username.clone()))
        .collect();
    let seconds = |frame: Option<u32>| crate::world::seconds(input, frame);
    let when = |frame: Option<u32>| world.when(input.clock, frame);
    build(world, fx, context, &players, &seconds, &when)
}

/// [`decode`] with the clock as two functions: seconds since the
/// recording started of a frame, and the [`When`] of one.
fn build(
    world: &World,
    fx: &Effects,
    context: &Context,
    players: &[(u64, String)],
    seconds: &dyn Fn(Option<u32>) -> Option<f64>,
    when: &dyn Fn(Option<u32>) -> When,
) -> Decoded {
    let mut scene = Scene::new(world, fx, context, players, seconds);
    let mut found = areas(&mut scene);
    let events = environment(&mut scene, &mut found);
    let lit = posts(&mut scene);

    let areas = found.into_iter().map(|f| Area {
        started: when(f.frame),
        ended: f.stopped.map(|s| when(Some(s))),
        ..f.area
    });
    let environment = events.into_iter().map(|e| EnvironmentEvent {
        when: when(e.frame),
        ended: e.stopped.map(|s| when(Some(s))),
        ..e.event
    });

    let mut light_screens: Vec<LightScreen> = Vec::new();
    let mut last: Option<f64> = None;
    for p in lit {
        let post = Post {
            entity: hex(p.entity),
            position: p.position,
            started: when(p.frame),
            ended: p.stopped.map(|s| when(Some(s))),
        };
        // A post joins the screen of its roll; one without a roll the
        // screen of the post before it, when that has none either.
        let follows = last.is_some_and(|l| p.seconds - l <= POST_STEP);
        last = Some(p.seconds);
        let roller = p.roller.map(hex);
        let unknown = roller.is_none() && p.throw.is_none();
        let screen = match unknown {
            false => light_screens
                .iter_mut()
                .find(|s| s.entity == roller && s.throw == p.throw),
            true => light_screens
                .last_mut()
                .filter(|s| s.entity.is_none() && s.throw.is_none() && follows),
        };
        if let Some(screen) = screen {
            screen.posts.push(post);
            continue;
        }
        let thrower = p.throw.and_then(|i| context.throws.get(i));
        let owner = p.owner.and_then(|o| players.get(o));
        let username = owner.map(|o| &o.1).or(thrower.map(|t| &t.username));
        light_screens.push(LightScreen {
            username: username.cloned(),
            username_source: username.map(|_| "proximity"),
            entity: roller,
            throw: p.throw,
            started: post.started.clone(),
            posts: vec![post],
            ended: None,
        });
    }
    for screen in &mut light_screens {
        let ends = screen.posts.iter().map(|p| p.ended.as_ref());
        let ends: Option<Vec<&When>> = ends.collect();
        let latest = ends.into_iter().flatten().max_by(|a, b| {
            let (a, b) = (a.recording_time, b.recording_time);
            a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
        });
        screen.ended = latest.cloned();
    }

    let mut warnings = Vec::new();
    if scene.untimed > 0 {
        warnings.push(format!(
            "{} effects are in frames the recording has no time for",
            scene.untimed
        ));
    }
    Decoded {
        areas: areas.collect(),
        environment: environment.collect(),
        light_screens,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Change, Damage, Kind, Owner, Placed};

    const BANK: u64 = 413779563590;
    /// Sixty frames a second.
    fn seconds(frame: Option<u32>) -> Option<f64> {
        Some(f64::from(frame.unwrap_or(0)) / 60.0)
    }

    fn when(frame: Option<u32>) -> When {
        When {
            recording_time: seconds(frame),
            ..When::default()
        }
    }

    fn players() -> Vec<(u64, String)> {
        vec![(11, "ana".to_owned()), (22, "bo".to_owned())]
    }

    /// A world with a body for each of [`players`]: `0xB0` and `0xB1`.
    fn world() -> World {
        let mut world = World::default();
        for (i, body) in [0xB0u64, 0xB1].into_iter().enumerate() {
            world.order.push(body);
            world.players.insert(body, i);
            world.bodies.insert(body, vec![(0, [50.0, 50.0, 0.0])]);
        }
        world
    }

    fn moved(frame: u32, position: [f32; 3]) -> Change {
        Change {
            frame: Some(frame),
            position: Some(position),
            ..Change::default()
        }
    }

    fn add(world: &mut World, entity: Entity) {
        world.order.push(entity.id);
        world.entities.insert(entity.id, entity);
    }

    /// An entity created in the pool and moved by `changes`.
    fn entity(id: u64, asset: u64, changes: Vec<Change>, deleted: Option<u32>) -> Entity {
        Entity {
            id,
            asset,
            position: crate::world::POOL,
            changes,
            deleted,
            ..Entity::default()
        }
    }

    /// A gadget of the player with `playerid`, at `position` from
    /// `frame`.
    fn gadget(id: u64, asset: u64, playerid: u64, frame: u32, position: [f32; 3]) -> Entity {
        let owned = Change {
            owner: Some(Owner {
                player: Some(playerid),
                ..Owner::default()
            }),
            ..moved(frame, position)
        };
        entity(id, asset, vec![owned], None)
    }

    fn map_object(id: u64, position: [f32; 3]) -> Entity {
        Entity {
            id,
            kind: Kind::MapObject,
            position,
            ..Entity::default()
        }
    }

    fn spawn(asset: u64, frame: u32, position: [f32; 3], stopped: Option<u32>) -> Spawn {
        Spawn {
            frame: Some(frame),
            asset,
            position: Some(position),
            alliance: Some(3),
            stopped,
            ..Spawn::default()
        }
    }

    fn effects(spawns: Vec<Spawn>) -> Effects {
        let mut fx = Effects::default();
        fx.spawns = spawns;
        fx
    }

    fn run(world: &World, fx: &Effects, shots: &[Shot], throws: &[Throw]) -> Decoded {
        let context = Context {
            shots,
            throws,
            loadouts: &[],
            map: BANK,
        };
        build(world, fx, &context, &players(), &seconds, &when)
    }

    fn area(points: &[[f32; 3]], radius: Option<f32>, from: f64, to: Option<f64>) -> Area {
        Area {
            points: points.to_vec(),
            radius,
            position: [0.0, 0.0, 0.0],
            start_seconds: from,
            end_seconds: to,
            ..Area::default()
        }
    }

    #[test]
    fn a_body_is_inside_near_a_cell_while_the_area_is_there() {
        let areas = [
            area(&[], Some(3.0), 0.0, None),
            area(&[[5.0, 5.0, 1.0], [6.0, 5.0, 1.0]], None, 10.0, Some(20.0)),
        ];
        // Within a metre of a cell, the cell at the feet.
        assert_eq!(inside(&areas, [6.9, 5.0, 1.0], 15.0), Some(1));
        assert_eq!(inside(&areas, [7.1, 5.0, 1.0], 15.0), None);
        // The cell from 1.0 m below the origin to 2.2 m above it.
        assert_eq!(inside(&areas, [5.0, 5.0, 2.0], 15.0), Some(1));
        assert_eq!(inside(&areas, [5.0, 5.0, 2.1], 15.0), None);
        assert_eq!(inside(&areas, [5.0, 5.0, -1.1], 15.0), Some(1));
        assert_eq!(inside(&areas, [5.0, 5.0, -1.3], 15.0), None);
        // Only while it is there, both ends included.
        assert_eq!(inside(&areas, [5.0, 5.0, 1.0], 9.9), None);
        assert_eq!(inside(&areas, [5.0, 5.0, 1.0], 10.0), Some(1));
        assert_eq!(inside(&areas, [5.0, 5.0, 1.0], 20.0), Some(1));
        assert_eq!(inside(&areas, [5.0, 5.0, 1.0], 20.1), None);
        // A cloud has no cells: nobody is inside it by this rule.
        assert_eq!(inside(&areas, [0.0, 0.0, 0.0], 15.0), None);
    }

    #[test]
    fn a_shot_goes_through_a_cloud_its_segment_passes() {
        let areas = [
            area(&[[0.0, 0.0, 0.0]], None, 0.0, None),
            area(&[], Some(3.0), 10.0, Some(24.0)),
        ];
        let x = [1.0, 0.0, 0.0];
        // The burst of a fire extinguisher is no smoke.
        let burst = Area {
            kind: AreaKind::Extinguisher,
            ..area(&[], Some(2.5), 10.0, Some(17.0))
        };
        assert!(!through_smoke(&[burst], [-10.0, 0.0, 0.0], x, 20.0, 12.0));
        // Past the centre at 2.9 m, and at 3.1 m.
        assert!(through_smoke(&areas, [-10.0, 2.9, 0.0], x, 20.0, 12.0));
        assert!(!through_smoke(&areas, [-10.0, 3.1, 0.0], x, 20.0, 12.0));
        // The bullet stopped short of the cloud, or flew away from it.
        assert!(!through_smoke(&areas, [-10.0, 0.0, 0.0], x, 6.9, 12.0));
        assert!(through_smoke(&areas, [-10.0, 0.0, 0.0], x, 7.1, 12.0));
        assert!(!through_smoke(&areas, [3.5, 0.0, 0.0], x, 20.0, 12.0));
        // Fired inside it.
        assert!(through_smoke(&areas, [1.0, 1.0, 0.0], x, 0.5, 12.0));
        // Before and after the cloud; an area with cells is no cloud.
        assert!(!through_smoke(&areas, [-10.0, 0.0, 0.0], x, 20.0, 9.0));
        assert!(!through_smoke(&areas, [-10.0, 0.0, 0.0], x, 20.0, 24.5));
        assert!(through_smoke(&areas, [-10.0, 0.0, 0.0], x, 20.0, 24.0));
    }

    #[test]
    fn a_gas_cloud_takes_its_source_and_the_owner_of_the_grenade() {
        let at = [4.0, 5.0, 1.0];
        let mut world = world();
        // The grenade came to rest there and was deleted as it went off;
        // the area entity is placed in the frame of the effect and
        // deleted well after the effect stops.
        let mut grenade = gadget(0xA1, 339570004345, 22, 500, [4.1, 5.0, 1.0]);
        grenade.deleted = Some(598);
        add(&mut world, grenade);
        let centre = entity(0xA2, 361147040848, vec![moved(600, at)], Some(1300));
        add(&mut world, centre);
        let mut gas = spawn(GAS, 600, at, Some(1190));
        gas.points = vec![[4.0, 5.0, 1.0], [5.0, 5.0, 1.0]];
        gas.capacity = 30;
        let decoded = run(&world, &effects(vec![gas]), &[], &[]);

        let [a] = decoded.areas.as_slice() else {
            panic!("{:?}", decoded.areas);
        };
        assert_eq!(a.kind, AreaKind::Gas);
        assert_eq!(a.source, Some("Remote Gas Grenade"));
        assert_eq!(a.entity.as_deref(), Some("000000a2"));
        assert_eq!(a.source_entity.as_deref(), Some("000000a1"));
        assert_eq!(a.username.as_deref(), Some("bo"));
        assert_eq!(a.username_source, Some("proximity"));
        assert_eq!((a.alliance, a.capacity), (Some(3), Some(30)));
        assert_eq!(a.points.len(), 2);
        assert_eq!((a.radius, a.radius_source), (None, None));
        // The end is the stop of the effect, not the area entity's.
        assert_eq!(a.start_seconds, 10.0);
        assert_eq!(a.end_seconds, Some(1190.0 / 60.0));
        assert_eq!(a.started.recording_time, Some(10.0));
        assert_eq!(
            a.ended.as_ref().and_then(|e| e.recording_time),
            a.end_seconds
        );
        assert!(decoded.warnings.is_empty());
    }

    #[test]
    fn a_smoke_cloud_has_an_assumed_radius_and_no_area_entity() {
        let at = [4.0, 5.0, 1.0];
        let mut world = world();
        let mut grenade = gadget(0xA1, 1, 11, 500, at);
        grenade.deleted = Some(590);
        add(&mut world, grenade);
        // A gadget of the other player that is still there is not it, nor
        // one deleted long before.
        add(&mut world, gadget(0xA3, 2, 22, 100, at));
        let mut old = gadget(0xA4, 2, 22, 100, at);
        old.deleted = Some(400);
        add(&mut world, old);
        let fx = effects(vec![spawn(SMOKE_GRENADE, 600, at, None)]);
        let decoded = run(&world, &fx, &[], &[]);

        let [a] = decoded.areas.as_slice() else {
            panic!("{:?}", decoded.areas);
        };
        assert_eq!((a.kind, a.source), (AreaKind::Smoke, Some("Smoke Grenade")));
        assert_eq!((a.radius, a.radius_source), (Some(3.0), Some("assumed")));
        assert_eq!(a.username.as_deref(), Some("ana"));
        assert_eq!((a.entity.as_ref(), a.ended.as_ref()), (None, None));
        assert_eq!(a.end_seconds, None);
    }

    #[test]
    fn areas_of_the_snapshot_and_other_effects_are_left_out() {
        let at = [4.0, 5.0, 1.0];
        let mut running = spawn(GAS, 0, at, Some(100));
        running.frame = None;
        let mut unplaced = spawn(FIRE, 10, at, None);
        unplaced.position = None;
        let fx = effects(vec![running, unplaced, spawn(1480682965, 20, at, None)]);
        let decoded = run(&world(), &fx, &[], &[]);
        assert!(decoded.areas.is_empty());
    }

    #[test]
    fn a_swarm_takes_the_hive_that_is_still_there() {
        let at = [4.0, 5.0, 1.0];
        let mut world = world();
        add(
            &mut world,
            gadget(0xA1, 385049526209, 11, 300, [4.0, 5.9, 1.0]),
        );
        add(
            &mut world,
            gadget(0xA5, 385049526209, 22, 300, [4.0, 5.4, 1.0]),
        );
        add(
            &mut world,
            entity(0xA2, 385049618011, vec![moved(600, at)], None),
        );
        let fx = effects(vec![spawn(SWARM, 600, at, Some(1500))]);
        let decoded = run(&world, &fx, &[], &[]);
        let a = decoded.areas.first().unwrap();
        assert_eq!(a.source, Some("Kawan Hive"));
        assert_eq!(a.username.as_deref(), Some("bo"));
        assert_eq!(a.username_source, Some("nearest"));
        assert_eq!(a.source_entity.as_deref(), Some("000000a5"));
    }

    /// A Volcan canister of `ana` at `at`, deleted in frame 598, and the
    /// fire it makes in frame 600.
    fn volcan(at: [f32; 3]) -> (World, Effects) {
        let mut world = world();
        let placed = Change {
            placed: Some(Placed {
                owner: Some(11),
                ..Placed::default()
            }),
            ..moved(100, at)
        };
        add(&mut world, entity(0xA1, VOLCAN, vec![placed], Some(598)));
        add(
            &mut world,
            entity(0xA2, 339570061825, vec![moved(600, at)], None),
        );
        (world, effects(vec![spawn(FIRE, 600, at, Some(1800))]))
    }

    fn shot(by: &str, seconds: f64, origin: [f32; 3], distance: f32) -> Shot {
        Shot {
            username: Some(by.to_owned()),
            origin,
            direction: [1.0, 0.0, 0.0],
            distance,
            when: When {
                recording_time: Some(seconds),
                ..When::default()
            },
            ..Shot::default()
        }
    }

    fn throw(by: &str, seconds: f64, flight: f64, end: [f64; 3]) -> Throw {
        Throw {
            username: by.to_owned(),
            name: Some("Frag Grenade"),
            end: Some(end),
            ended_after: Some(flight),
            when: When {
                recording_time: Some(seconds),
                ..When::default()
            },
            ..Throw::default()
        }
    }

    #[test]
    fn a_canister_is_set_off_by_the_shot_that_ended_on_it() {
        let (world, fx) = volcan([4.0, 5.0, 1.0]);
        let shots = [
            // Too early, ending elsewhere, and the one that hit it.
            shot("bo", 8.0, [0.0, 5.0, 1.0], 4.0),
            shot("ana", 9.9, [0.0, 5.0, 1.0], 2.0),
            shot("bo", 9.9, [0.0, 5.0, 1.0], 3.9),
        ];
        let throws = [throw("ana", 8.0, 1.9, [5.0, 5.0, 1.0])];
        let decoded = run(&world, &fx, &shots, &throws);
        let a = decoded.areas.first().unwrap();
        assert_eq!(a.source, Some("Volcan Canister"));
        assert_eq!(a.username.as_deref(), Some("ana"));
        assert_eq!(a.triggered_by.as_deref(), Some("bo"));
        assert_eq!(a.trigger_source, Some("shotRay"));
        assert_eq!(a.triggered_with, None);
    }

    #[test]
    fn without_a_shot_a_canister_is_set_off_by_an_explosive_near_it() {
        let (world, fx) = volcan([4.0, 5.0, 1.0]);
        let throws = [
            // Ended too far away, at another time, and next to it.
            throw("ana", 8.0, 1.9, [10.0, 5.0, 1.0]),
            throw("ana", 5.0, 2.0, [5.0, 5.0, 1.0]),
            throw("bo", 8.0, 1.9, [7.0, 5.0, 1.0]),
        ];
        let decoded = run(&world, &fx, &[], &throws);
        let a = decoded.areas.first().unwrap();
        assert_eq!(a.triggered_by.as_deref(), Some("bo"));
        assert_eq!(a.trigger_source, Some("explosion"));
        assert_eq!(a.triggered_with, Some("Frag Grenade"));

        let decoded = run(&world, &fx, &[], &[]);
        let a = decoded.areas.first().unwrap();
        assert_eq!((a.triggered_by.as_ref(), a.trigger_source), (None, None));
    }

    /// A damage list entry by `instigator` in `frame`.
    fn damaged(frame: u32, instigator: u64) -> Change {
        Change {
            frame: Some(frame),
            damage: vec![Damage {
                kind: 1,
                instigator,
                ..Damage::default()
            }],
            ..Change::default()
        }
    }

    #[test]
    fn a_fire_extinguisher_bursts_by_the_body_in_its_damage_list() {
        let mut world = world();
        let mut object = map_object(0x60_7AFD_5716, [4.0, 5.0, 1.0]);
        // Shot long before by one player, then by the other; a gadget's
        // damage names no player.
        object.changes = vec![damaged(100, 0xB0), damaged(590, 0xB1), damaged(595, 0xA9)];
        add(&mut world, object);
        let mut burst = spawn(EXTINGUISHER, 600, [4.1, 5.0, 1.2], Some(1020));
        burst.parent = 0x60_7AFD_5716;
        let decoded = run(&world, &effects(vec![burst]), &[], &[]);

        let a = decoded.areas.first().unwrap();
        assert_eq!(a.kind, AreaKind::Extinguisher);
        assert_eq!(a.source, Some("Fire Extinguisher"));
        assert_eq!(a.object.as_deref(), Some("607afd5716"));
        assert_eq!((a.radius, a.radius_source), (Some(2.5), Some("assumed")));
        assert_eq!(a.triggered_by.as_deref(), Some("bo"));
        assert_eq!(a.trigger_source, Some("read"));
        let [e] = decoded.environment.as_slice() else {
            panic!("{:?}", decoded.environment);
        };
        assert_eq!(e.kind, EnvironmentKind::FireExtinguisher);
        assert_eq!(e.object.as_deref(), Some("607afd5716"));
        assert_eq!(e.object_source, Some("read"));
        assert_eq!(e.position, [4.0, 5.0, 1.0]);
        assert_eq!((e.by.as_deref(), e.by_source), (Some("bo"), Some("read")));
        assert_eq!(e.area, Some(0));
        assert_eq!(e.when.recording_time, Some(10.0));
        assert_eq!(e.ended.as_ref().and_then(|w| w.recording_time), Some(17.0));
    }

    /// A pipe explosion in frame 600 at `at` and its fire 18 frames
    /// later, with the fire's area entity.
    fn explosion(world: &mut World, at: [f32; 3]) -> Effects {
        add(
            world,
            entity(0xA2, 407899098848, vec![moved(618, at)], None),
        );
        let mut blown = spawn(PIPE_EXPLOSION, 600, at, Some(700));
        blown.alliance = Some(4);
        effects(vec![blown, spawn(FIRE, 618, at, Some(730))])
    }

    #[test]
    fn a_gas_pipe_of_the_table_is_preferred_and_names_its_shooter() {
        let at = [-67.0, 0.6, -3.1];
        let mut world = world();
        // A wall right at the explosion, and the pipe the table lists a
        // metre away.
        add(&mut world, map_object(0x5F_0000_0001, at));
        let mut listed = map_object(0x61_E910_A0E2, [-67.1, 1.5, -3.8]);
        listed.changes = vec![damaged(400, 0xB0)];
        add(&mut world, listed);
        // A gadget the fire destroys does not make the fire its owner's.
        let mut burnt = gadget(0xA7, 1, 22, 100, at);
        burnt.deleted = Some(620);
        add(&mut world, burnt);
        let fx = explosion(&mut world, at);
        let decoded = run(&world, &fx, &[], &[]);

        let [e] = decoded.environment.as_slice() else {
            panic!("{:?}", decoded.environment);
        };
        assert_eq!(e.kind, EnvironmentKind::GasPipe);
        assert_eq!(e.object.as_deref(), Some("61e910a0e2"));
        assert_eq!(e.object_source, Some("table"));
        assert_eq!((e.position, e.alliance), (at, Some(4)));
        assert_eq!((e.by.as_deref(), e.by_source), (Some("ana"), Some("read")));
        assert_eq!((e.area, e.ended.as_ref()), (Some(0), None));
        // The fire takes who blew the pipe up.
        let fire = decoded.areas.first().unwrap();
        assert_eq!(fire.source, Some(GAS_PIPE));
        assert_eq!(fire.username, None);
        assert_eq!(fire.triggered_by.as_deref(), Some("ana"));
        assert_eq!(fire.trigger_source, Some("read"));
    }

    #[test]
    fn an_unlisted_gas_pipe_is_picked_by_family_then_change_then_distance() {
        let at = [10.0, 10.0, 0.0];
        let mut world = world();
        // The largest family: walls, one of them nearest of all.
        for i in 0..4 {
            add(
                &mut world,
                map_object(0x5F_0000_0000 + i, [10.0, 10.0 + i as f32, 0.0]),
            );
        }
        // Two objects of small families: the nearer one, and the one
        // whose flags change as the pipe blows.
        add(&mut world, map_object(0x61_0000_0001, [10.2, 10.0, 0.0]));
        let mut pipe = map_object(0x62_0000_0001, [11.0, 10.0, 0.0]);
        pipe.changes = vec![Change {
            frame: Some(606),
            flags: Some(0x4000),
            ..Change::default()
        }];
        add(&mut world, pipe);
        // Out of reach and on another floor.
        add(&mut world, map_object(0x63_0000_0001, [12.0, 10.0, 0.0]));
        add(&mut world, map_object(0x64_0000_0001, [10.0, 10.0, 7.0]));
        let fx = explosion(&mut world, at);
        let throws = [throw("bo", 8.0, 1.9, [12.0, 10.0, 0.0])];
        let decoded = run(&world, &fx, &[], &throws);
        let e = decoded.environment.first().unwrap();
        assert_eq!(e.object.as_deref(), Some("6200000001"));
        assert_eq!(e.object_source, Some("stateChange"));
        // No bullet named a player: the grenade that went off there.
        assert_eq!(
            (e.by.as_deref(), e.by_source),
            (Some("bo"), Some("explosion"))
        );
        assert_eq!(e.by_with, Some("Frag Grenade"));
        let fire = decoded.areas.first().unwrap();
        assert_eq!(fire.trigger_source, Some("explosion"));
        assert_eq!(fire.triggered_with, Some("Frag Grenade"));

        // Without the change, the nearest outside the largest family.
        if let Some(pipe) = world.entities.get_mut(&0x62_0000_0001) {
            pipe.changes.clear();
        }
        let decoded = run(&world, &fx, &[], &[]);
        let e = decoded.environment.first().unwrap();
        assert_eq!(e.object.as_deref(), Some("6100000001"));
        assert_eq!(e.object_source, Some("nearest"));
        assert_eq!((e.by.as_ref(), e.by_source), (None, None));
    }

    #[test]
    fn a_metal_detector_is_the_map_object_before_its_lights() {
        let mut world = world();
        add(&mut world, map_object(0x63_0F39_FA15, [1.0, 1.0, 0.8]));
        add(&mut world, map_object(0x63_0F39_FA59, [30.0, 1.0, 0.8]));
        add(&mut world, map_object(0x64_0000_0001, [60.0, 1.0, 0.8]));
        // One body walks through the first; the other is nearer on the
        // floor above.
        let walk = vec![
            (0, [50.0, 50.0, 0.0]),
            (597, [1.0, 2.5, 0.0]),
            (603, [1.0, 1.9, 0.0]),
        ];
        world.bodies.insert(0xB0, walk);
        world.bodies.insert(0xB1, vec![(0, [1.0, 1.0, 4.0])]);
        let light = |parent: u64, asset: u64, frame: u32| Spawn {
            parent,
            position: None,
            ..spawn(asset, frame, [0.0; 3], Some(frame + 180))
        };
        let fx = effects(vec![
            // Two lights of the first detector in one frame are one alarm.
            light(0x63_0F39_FA17, DETECTOR_ALARM, 600),
            light(0x63_0F39_FA19, DETECTOR_ALARM, 600),
            // Switched off; nobody is named for that.
            light(0x63_0F39_FA5B, DETECTOR_OFF, 660),
            // A light with no map object of its family before it.
            light(0x65_0000_0001, DETECTOR_ALARM, 700),
            // Nobody within reach of the second detector.
            light(0x63_0F39_FA5B, DETECTOR_ALARM, 900),
        ]);
        let decoded = run(&world, &fx, &[], &[]);

        let events = decoded.environment;
        assert_eq!(events.len(), 3, "{events:?}");
        let alarm = events.first().unwrap();
        assert_eq!(alarm.kind, EnvironmentKind::MetalDetector);
        assert_eq!(alarm.event, Some("alarm"));
        assert_eq!(alarm.object.as_deref(), Some("630f39fa15"));
        assert_eq!(alarm.object_source, Some("table"));
        assert_eq!(alarm.position, [1.0, 1.0, 0.8]);
        assert_eq!(
            (alarm.by.as_deref(), alarm.by_source),
            (Some("ana"), Some("nearest"))
        );
        assert_eq!(alarm.distance, Some(1.2));
        assert_eq!(alarm.when.recording_time, Some(10.0));
        assert_eq!(
            alarm.ended.as_ref().and_then(|w| w.recording_time),
            Some(13.0)
        );
        let off = events.get(1).unwrap();
        assert_eq!((off.event, off.by.as_ref()), (Some("off"), None));
        assert_eq!(off.object.as_deref(), Some("630f39fa59"));
        let unattended = events.get(2).unwrap();
        assert_eq!(
            (unattended.event, unattended.by.as_ref()),
            (Some("alarm"), None)
        );
    }

    #[test]
    fn a_detector_no_table_lists_says_it_is_found_by_id_order() {
        let mut world = world();
        add(&mut world, map_object(0x66_DECB_6015, [1.0, 1.0, 0.8]));
        let alarm = Spawn {
            parent: 0x66_DECB_6017,
            position: None,
            ..spawn(DETECTOR_ALARM, 600, [0.0; 3], None)
        };
        let decoded = run(&world, &effects(vec![alarm]), &[], &[]);
        let e = decoded.environment.first().unwrap();
        assert_eq!(e.object_source, Some("idOrder"));
        assert_eq!(e.ended, None);
    }

    const PROJECTOR: u64 = 376357912467;

    /// A post at `at` whose light comes up in `frame`.
    fn post(world: &mut World, id: u64, frame: u32, at: [f32; 3], stopped: Option<u32>) -> Spawn {
        let placed = vec![moved(frame - 2, at)];
        add(world, entity(id, 377423931277, placed, None));
        Spawn {
            parent: id,
            position: None,
            ..spawn(SCREEN, frame, [0.0; 3], stopped)
        }
    }

    /// A projector of the player with `playerid` that rolls along x from
    /// frame 600 on, 2 m every 18 frames, at `y`.
    fn projector(world: &mut World, id: u64, playerid: u64, y: f32) {
        let mut rolling = gadget(id, PROJECTOR, playerid, 600, [0.0, y, 0.0]);
        let steps = (1..=60u32).map(|i| moved(600 + i, [i as f32 / 9.0, y, 0.0]));
        rolling.changes.extend(steps);
        add(world, rolling);
    }

    fn rolled(by: &str, seconds: f64, origin: [f64; 3]) -> Throw {
        Throw {
            asset: PROJECTOR,
            name: Some("R.O.U. Projector System"),
            origin: Some(origin),
            path: vec![[0.0, origin[0], origin[1], origin[2]]],
            ..throw(by, seconds, 1.0, origin)
        }
    }

    #[test]
    fn posts_are_grouped_by_the_projector_that_rolled_past() {
        let mut world = world();
        // Two projectors roll side by side, half a metre apart, at the
        // same time: the posts of each light up as it passes.
        projector(&mut world, 0xF1, 11, 0.0);
        projector(&mut world, 0xF2, 22, 0.6);
        let mut spawns = Vec::new();
        for i in 0..3u32 {
            let (frame, x) = (618 + 18 * i, 2.0 * (i + 1) as f32);
            let stopped = Some(1700 + i);
            spawns.push(post(
                &mut world,
                0xC0 + u64::from(i),
                frame,
                [x, 0.1, 0.0],
                stopped,
            ));
            let stopped = (i < 2).then_some(1800);
            spawns.push(post(
                &mut world,
                0xD0 + u64::from(i),
                frame,
                [x, 0.5, 0.0],
                stopped,
            ));
        }
        let throws = [
            // A throw of the same player before, from elsewhere.
            rolled("ana", 5.0, [30.0, 0.0, 0.0]),
            rolled("ana", 10.0, [0.0, 0.0, 0.0]),
            throw("bo", 10.0, 1.0, [0.0, 0.6, 0.0]),
            rolled("bo", 10.0, [0.0, 0.6, 0.0]),
        ];
        let decoded = run(&world, &effects(spawns), &[], &throws);

        let screens = decoded.light_screens;
        assert_eq!(screens.len(), 2, "{screens:?}");
        let first = screens.first().unwrap();
        assert_eq!(first.username.as_deref(), Some("ana"));
        assert_eq!(first.username_source, Some("proximity"));
        assert_eq!(first.entity.as_deref(), Some("000000f1"));
        assert_eq!(first.throw, Some(1));
        let ids: Vec<&str> = first.posts.iter().map(|p| p.entity.as_str()).collect();
        assert_eq!(ids, ["000000c0", "000000c1", "000000c2"]);
        assert_eq!(first.started.recording_time, Some(10.3));
        assert_eq!(
            first.posts.first().map(|p| p.position),
            Some([2.0, 0.1, 0.0])
        );
        // The screen ends when its last light goes out.
        let ended = first.ended.as_ref().and_then(|w| w.recording_time);
        assert_eq!(ended, seconds(Some(1702)));
        let second = screens.get(1).unwrap();
        assert_eq!(second.username.as_deref(), Some("bo"));
        assert_eq!(
            (second.entity.as_deref(), second.throw),
            (Some("000000f2"), Some(3))
        );
        assert_eq!(second.posts.len(), 3);
        // One of its posts is still lit.
        assert_eq!(second.ended, None);
    }

    #[test]
    fn without_a_projector_posts_take_the_nearest_roll_or_none() {
        let mut world = world();
        let mut spawns = Vec::new();
        // Two posts along the path of a roll whose projector is not in
        // the world, and two posts no roll explains, lit together.
        for i in 0..2u32 {
            let at = [2.0 * (i + 1) as f32, 0.0, 0.0];
            spawns.push(post(
                &mut world,
                0xC0 + u64::from(i),
                618 + 18 * i,
                at,
                None,
            ));
        }
        for i in 0..2u32 {
            let at = [90.0, 2.0 * i as f32, 0.0];
            spawns.push(post(
                &mut world,
                0xE0 + u64::from(i),
                3000 + 18 * i,
                at,
                None,
            ));
        }
        let along = Throw {
            path: (0..5)
                .map(|i| [0.1 * f64::from(i), f64::from(i), 0.0, 0.0])
                .collect(),
            ..rolled("bo", 10.0, [0.0, 0.0, 0.0])
        };
        let throws = [rolled("ana", 9.0, [0.0, 3.0, 0.0]), along];
        let decoded = run(&world, &effects(spawns), &[], &throws);

        let screens = decoded.light_screens;
        assert_eq!(screens.len(), 2, "{screens:?}");
        let first = screens.first().unwrap();
        assert_eq!(
            (first.username.as_deref(), first.throw),
            (Some("bo"), Some(1))
        );
        assert_eq!((first.entity.as_ref(), first.posts.len()), (None, 2));
        let second = screens.get(1).unwrap();
        assert_eq!((second.username.as_ref(), second.throw), (None, None));
        assert_eq!((second.username_source, second.posts.len()), (None, 2));
    }

    #[test]
    fn a_frame_without_a_time_is_counted_not_guessed() {
        let at = [4.0, 5.0, 1.0];
        let fx = effects(vec![spawn(GAS, 600, at, None), spawn(GAS, 9999, at, None)]);
        let context = Context {
            shots: &[],
            throws: &[],
            loadouts: &[],
            map: BANK,
        };
        let known = |frame: Option<u32>| seconds(frame).filter(|_| frame != Some(9999));
        let decoded = build(&world(), &fx, &context, &players(), &known, &when);
        assert_eq!(decoded.areas.len(), 1);
        assert_eq!(decoded.warnings.len(), 1);
    }
}
