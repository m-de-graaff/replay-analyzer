//! Reinforcements and barricades (Y11S3), from the [`World`] of a round.
//!
//! # What a panel is
//!
//! Whatever is put on a wall, a hatch, a door or a window to close it is
//! one kind of entity: classes `4c60869a` (placed) and `6ea51c35` (damage
//! list), and the two slots `b4d93e43` and `2e4bce49`, both empty. The
//! slots are compared as a set: their order differs between builds, and
//! Aruni's and Mira's gadgets have the same classes with other slots. The
//! asset says which panel it is (`crate::types::map_tables`).
//!
//! The game creates the panels a round may need at [`crate::world::POOL`]
//! and moves one to its place when it is used, so one entity can be
//! several panels in a round, one after the other. Each is a life:
//!
//! ```text
//! start      the first position that is not the pool. A player's panel
//!            has `02 <playerid>` in its placed component in that message;
//!            a rotation comes with it or has come before (else identity)
//! complete   `live` 1. The host (the wall, hatch or frame it is fixed
//!            to, a map object) is written in that message or up to four
//!            frames later
//! called off back to the pool, or deleted, before `live` 1
//! default    placed with no owner: the map's own barricades, which get
//!            their place and `live` 1 in one message in the first frames
//!            of the round, and Quick Match's reinforcements
//! end        an `fe` entry in its damage list (destroyed; for a hatch
//!            reinforcement, opened), back to the pool, or deleted
//! ```
//!
//! A wall reinforcement takes 4.1 s from start to complete, a hatch
//! reinforcement 4.4 s, a barricade 2.5 or 2.6 s, Castle's 2.8 or 3.1 s.
//! The panel's normal is its rotation applied to (0, 0, 1): a wall's is
//! level, a hatch's points up or down.
//!
//! A panel of an asset the table does not have gets its kind from how it
//! behaves, and says so with `kindSource: derived`: one nobody placed is a
//! barricade, one lying flat a hatch reinforcement, one completed in under
//! 3.3 s a barricade, and any other a wall reinforcement.
//!
//! # How a panel ended
//!
//! The damage list (see [`crate::world`]) has a record for everything
//! that struck the panel and `fe` once it is destroyed. Records keep
//! coming after that: bullets strike what is left of a barricade. So who
//! destroyed it and how is the last record up to the `fe`, when that
//! record is at most 2 s old, leaving out the damage a reinforcement
//! going up deals to the panels next to it (id 42593091656):
//!
//! ```text
//! damage id 34118943362                    melee
//! damage id 261358999180, 285432852752     explosion
//! kind 1, 2, 3                             bullet
//! instigator an entity that is no body     gadget (with its owner)
//! anything else                            other
//! ```
//!
//! With no such record the panel was taken down by hand or walked
//! through after melee hits weakened it, which the file does not say. The
//! nearest body within 2.5 m is named then, with `source: proximity`:
//! `removed` when it stands 0.3 to 0.5 m from the panel (where a player
//! stands to take a barricade down; 0.40 m in the test rounds), else
//! `brokenThrough`. With no body that near, `how` is `unknown`.

use serde::{Serialize, Serializer};

use crate::entities::Hash;
use crate::loadout::{Input, When};
use crate::types::{PanelAsset, panel_asset};
use crate::world::{DAMAGE, Entity, PLACED, World, seconds};

/// The two slots of a panel.
const SLOTS: [Hash; 2] = [[0xB4, 0xD9, 0x3E, 0x43], [0x2E, 0x4B, 0xCE, 0x49]];
/// An object lower than this is in the pool.
const POOLED_BELOW: f32 = -90.0;
/// The rotation of a panel that never wrote one.
const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// A panel whose normal has this much of the vertical lies flat.
const FLAT: f32 = 0.7;
/// A panel of unknown asset completed sooner than this is a barricade.
const BARRICADE_SECONDS: f64 = 3.3;

/// The damage id of a melee hit, and of an explosion's two records.
const MELEE: u64 = 34_118_943_362;
const EXPLOSIONS: [u64; 2] = [261_358_999_180, 285_432_852_752];
/// The damage a reinforcement going up deals to the soft wall under it
/// and to the reinforcement next to it.
const REINFORCING: u64 = 42_593_091_656;
/// The record that destroyed a panel is at most this old.
const CAUSE_WINDOW: f64 = 2.0;
/// Whoever took a panel down stands within this of it, level and up.
const NEAR: f32 = 2.5;
const NEAR_UP: f32 = 3.5;
/// How far from a barricade a player stands to take it down.
const REMOVING: std::ops::RangeInclusive<f32> = 0.3..=0.5;

/// A frame; `None` is the opening snapshot.
type Frame = Option<u32>;

/// What a panel is, by its asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelKind {
    WallReinforcement,
    HatchReinforcement,
    Barricade,
    /// Castle's Armor Panel.
    CastlePanel,
}

/// The kind of panel `asset` is, when the table has it.
pub(crate) fn kind_of(asset: u64) -> Option<PanelKind> {
    Some(match panel_asset(asset)? {
        PanelAsset::Wall { .. } => PanelKind::WallReinforcement,
        PanelAsset::Hatch => PanelKind::HatchReinforcement,
        PanelAsset::Barricade { castle: true, .. } => PanelKind::CastlePanel,
        PanelAsset::Barricade { .. } => PanelKind::Barricade,
    })
}

/// The entity is a reinforcement or a barricade.
pub(crate) fn is_panel(entity: &Entity) -> bool {
    entity.classes == [PLACED, DAMAGE]
        && entity.slots.len() == SLOTS.len()
        && SLOTS.iter().all(|s| entity.slot(*s).is_some())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReinforcementKind {
    #[default]
    Wall,
    Hatch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BarricadeKind {
    #[default]
    Barricade,
    /// Castle's Armor Panel.
    Castle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Opening {
    Door,
    Window,
}

/// Where a value comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// A record of the panel's damage list.
    Read,
    /// The nearest body.
    Proximity,
    /// How the panel behaved.
    Derived,
}

/// How a panel was destroyed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum How {
    Bullet,
    Melee,
    Explosion,
    /// Something a gadget did that is no explosion: Maverick's torch,
    /// a pellet of Hibana's, a drone's laser.
    Gadget,
    /// Something a player did that is none of the above, such as a
    /// thermite charge burning.
    Other,
    /// Taken down by hand (inferred).
    Removed,
    /// Walked or vaulted through after it was weakened (inferred).
    BrokenThrough,
    /// No record of it and nobody near.
    #[default]
    Unknown,
}

/// The end of a panel by an `fe` entry in its damage list.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Destroyed {
    pub how: How,
    /// `read` or `proximity`; absent when `how` is `unknown`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// Who did it: the player of the record, the owner of the gadget of
    /// the record, or the player nearest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// The gadget entity that did it, when the record names one.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "hex_option")]
    pub gadget: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gadget_asset: Option<u64>,
    /// The damage id of the record: what did it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damage_id: Option<u64>,
    /// `proximity`: metres between the panel and the body, level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<f32>,
    #[serde(flatten)]
    pub when: When,
}

/// One reinforcement of a wall or a hatch.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reinforcement {
    /// Id of the panel's entity, in hex. An entity can be several panels
    /// in a round, one after the other.
    #[serde(serialize_with = "hex")]
    pub entity: u64,
    pub kind: ReinforcementKind,
    /// `derived` when the asset is not known and the kind is told from
    /// how the panel behaved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind_source: Option<Source>,
    pub asset: u64,
    /// Who placed it; absent for one the map placed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Id of the map object it is fixed to, in hex: the wall or the
    /// hatch. Set once the reinforcement is complete.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "hex_option")]
    pub host: Option<u64>,
    /// Where it is, in map coordinates, and which way it faces.
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Metres of wall it covers, by its asset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// The width is not in the file: it is inferred from the asset.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub width_inferred: bool,
    /// When the player started placing it; absent for a default one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<When>,
    /// When it was up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<When>,
    /// The player let go before it was up.
    pub cancelled: bool,
    /// Placed by the map, not by a player.
    pub default: bool,
    /// When it was called off, or when a standing one that was not
    /// destroyed was taken away.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<When>,
    /// The `fe` entry of its damage list: a hatch reinforcement that was
    /// opened has it, a wall reinforcement was not seen with one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroyed: Option<Destroyed>,
    /// When it was breached: its intact flag was cleared (see
    /// [`crate::destruction`], joined in [`crate::join`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opened: Option<When>,
}

/// One barricade of a door or a window.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Barricade {
    /// Id of the panel's entity, in hex. An entity can be several panels
    /// in a round, one after the other.
    #[serde(serialize_with = "hex")]
    pub entity: u64,
    pub kind: BarricadeKind,
    /// `derived` when the asset is not known and the kind is told from
    /// how the panel behaved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind_source: Option<Source>,
    pub asset: u64,
    /// What it closes and whether that is a wide one, by its asset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opening: Option<Opening>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wide: Option<bool>,
    /// Which asset is which opening is not in the file.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub opening_inferred: bool,
    /// Who placed it; absent for one the map placed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Id of the map object it is fixed to, in hex: the frame. The map's
    /// own barricades name none.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "hex_option")]
    pub host: Option<u64>,
    /// Where it is, in map coordinates, and which way it faces.
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Placed by the map, not by a player.
    pub default: bool,
    /// When the player started placing it; absent for a default one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<When>,
    /// When it was up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<When>,
    /// The player let go before it was up.
    pub cancelled: bool,
    /// When it was called off, or when a standing one that was not
    /// destroyed was taken away.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<When>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroyed: Option<Destroyed>,
}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    pub reinforcements: Vec<Reinforcement>,
    pub barricades: Vec<Barricade>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
}

fn hex<S: Serializer>(id: &u64, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format!("{id:x}"))
}

fn hex_option<S: Serializer>(id: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
    match id {
        Some(id) => hex(id, s),
        None => s.serialize_none(),
    }
}

/// To the millimetre, and no `-0.0`.
fn rounded(v: [f32; 3]) -> [f32; 3] {
    v.map(|x| (x * 1000.0).round() / 1000.0 + 0.0)
}

/// One record of a panel's damage list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Blow {
    frame: Frame,
    kind: u8,
    id: u64,
    instigator: u64,
}

/// How a life ended, other than destroyed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    /// Before it was complete.
    CalledOff,
    /// A standing panel went back to the pool or was deleted.
    Taken,
}

/// One use of a panel entity, from leaving the pool to going back.
#[derive(Clone, Debug, Default, PartialEq)]
struct Life {
    start: Frame,
    position: [f32; 3],
    rotation: [f32; 4],
    /// The `playerid` of who placed it.
    owner: Option<u64>,
    /// No owner was written: the map placed it.
    default: bool,
    /// The frame of `live` 1.
    complete: Option<Frame>,
    host: Option<u64>,
    blows: Vec<Blow>,
    /// The frame of the `fe` entry.
    destroyed: Option<Frame>,
    end: Option<(End, Frame)>,
}

impl Life {
    /// The panel left its place in `frame`.
    fn close(&mut self, frame: Frame) {
        if self.complete.is_none() && !self.default {
            self.end = Some((End::CalledOff, frame));
        } else if self.destroyed.is_none() {
            self.end = Some((End::Taken, frame));
        }
    }

    fn cancelled(&self) -> bool {
        matches!(self.end, Some((End::CalledOff, _)))
    }

    /// The record that destroyed the panel: the last one up to the `fe`
    /// entry that is not a neighbour going up, when it is at most
    /// [`CAUSE_WINDOW`] old. A record or an entry in a frame without a
    /// time is taken as in time.
    fn cause(&self, seconds: impl Fn(Frame) -> Option<f64>) -> Option<Blow> {
        let end = self.destroyed?;
        let before = |b: &&Blow| b.id != REINFORCING && b.frame <= end;
        let last = *self.blows.iter().rev().find(before)?;
        match (seconds(end), seconds(last.frame)) {
            (Some(end), Some(at)) if end - at > CAUSE_WINDOW => None,
            _ => Some(last),
        }
    }
}

/// The lives of a panel entity, in order.
fn lives(entity: &Entity) -> Vec<Life> {
    let mut out = Vec::new();
    let mut current: Option<Life> = None;
    let mut rotation = IDENTITY;
    for c in &entity.changes {
        if let Some(q) = c.rotation {
            rotation = q;
        }
        let placed = c.placed.unwrap_or_default();
        if let Some(position) = c.position {
            if position[2] < POOLED_BELOW {
                if let Some(mut life) = current.take() {
                    life.close(c.frame);
                    out.push(life);
                }
            } else if current.is_none() {
                current = Some(Life {
                    start: c.frame,
                    position,
                    rotation,
                    default: placed.owner.is_none(),
                    ..Life::default()
                });
            }
        }
        let Some(life) = &mut current else {
            continue;
        };
        if life.owner.is_none() && placed.owner.is_some() {
            life.owner = placed.owner;
            life.default = false;
        }
        if c.live == Some(1) && life.complete.is_none() {
            life.complete = Some(c.frame);
        }
        if placed.host.is_some() {
            life.host = placed.host;
        }
        life.blows.extend(c.damage.iter().map(|d| Blow {
            frame: c.frame,
            kind: d.kind,
            id: d.id,
            instigator: d.instigator,
        }));
        if c.destroyed && life.destroyed.is_none() {
            life.destroyed = Some(c.frame);
        }
    }
    if let Some(mut life) = current {
        if entity.deleted.is_some() {
            life.close(entity.deleted);
        }
        out.push(life);
    }
    out
}

/// What a life is, with `derived` when the asset does not say: `took` is
/// the seconds from its start to complete.
fn kind(asset: u64, life: &Life, took: Option<f64>) -> (PanelAsset, Option<Source>) {
    if let Some(known) = panel_asset(asset) {
        return (known, None);
    }
    let unknown = PanelAsset::Barricade {
        castle: false,
        door: false,
        wide: false,
    };
    let normal = World::rotate(life.rotation, [0.0, 0.0, 1.0]);
    let derived = if life.default {
        unknown
    } else if normal[2].abs() > FLAT {
        PanelAsset::Hatch
    } else if took.is_some_and(|t| t < BARRICADE_SECONDS) {
        unknown
    } else {
        PanelAsset::Wall { width: None }
    };
    (derived, Some(Source::Derived))
}

/// The record `blow` as the cause of a panel's end.
fn how(blow: &Blow, gadget: bool) -> How {
    if blow.id == MELEE {
        How::Melee
    } else if EXPLOSIONS.contains(&blow.id) {
        How::Explosion
    } else if (1..=3).contains(&blow.kind) {
        How::Bullet
    } else if gadget {
        How::Gadget
    } else {
        How::Other
    }
}

/// The body nearest to `position` in `frame`, level, of those no more
/// than [`NEAR_UP`] above or below it: `(metres, player)`.
fn nearest(world: &World, frame: Frame, position: [f32; 3]) -> Option<(f32, usize)> {
    let frame = frame.unwrap_or(0);
    let mut best: Option<(f32, usize)> = None;
    for &body in world.bodies.keys() {
        let (Some(at), Some(player)) = (world.body_at(body, frame), world.player_of(body)) else {
            continue;
        };
        if (at[2] - position[2]).abs() >= NEAR_UP {
            continue;
        }
        let d = (at[0] - position[0]).hypot(at[1] - position[1]);
        // The lower player index on a tie, so that the order of the map
        // does not show.
        if best.is_none_or(|b| (d, player) < b) {
            best = Some((d, player));
        }
    }
    best
}

/// Everything a round's panels need to be written.
struct Context<'a> {
    input: &'a Input<'a>,
    world: &'a World,
    /// Owners that are no player of the round.
    strangers: usize,
}

impl Context<'_> {
    fn when(&self, frame: Frame) -> When {
        self.world.when(self.input.clock, frame)
    }

    fn seconds(&self, frame: Frame) -> Option<f64> {
        seconds(self.input, frame)
    }

    fn username(&self, playerid: u64) -> Option<String> {
        let player = self.input.players.iter().find(|p| p.id == playerid)?;
        Some(player.username.clone())
    }

    /// The owner of a gadget entity: the last one its placed or owner
    /// component named up to `frame`.
    fn owner_of(&self, gadget: &Entity, frame: Frame) -> Option<String> {
        let named = |c: &crate::world::Change| {
            let placed = c.placed.and_then(|p| p.owner);
            placed.or(c.owner.and_then(|o| o.player))
        };
        let playerid = gadget.until(frame).iter().rev().find_map(named)?;
        self.username(playerid)
    }

    fn destroyed(&self, life: &Life) -> Option<Destroyed> {
        let frame = life.destroyed?;
        let mut out = Destroyed {
            when: self.when(frame),
            ..Destroyed::default()
        };
        if let Some(blow) = life.cause(|f| self.seconds(f)) {
            out.source = Some(Source::Read);
            out.damage_id = Some(blow.id);
            let body = self.world.player_of(blow.instigator);
            let gadget = match body {
                Some(_) => None,
                None => self.world.get(blow.instigator),
            };
            out.how = how(&blow, gadget.is_some());
            out.by = match (body, gadget) {
                (Some(player), _) => self.input.players.get(player).map(|p| p.username.clone()),
                (None, Some(gadget)) => self.owner_of(gadget, frame),
                (None, None) => None,
            };
            out.gadget = gadget.map(|g| g.id);
            out.gadget_asset = gadget.map(|g| g.asset);
        } else if let Some((distance, player)) =
            nearest(self.world, frame, life.position).filter(|n| n.0 < NEAR)
        {
            out.source = Some(Source::Proximity);
            out.how = if REMOVING.contains(&distance) {
                How::Removed
            } else {
                How::BrokenThrough
            };
            out.by = (self.input.players.get(player)).map(|p| p.username.clone());
            out.distance = Some((distance * 100.0).round() / 100.0);
        }
        Some(out)
    }
}

/// Reads the reinforcements and barricades of a round from its world,
/// each list in the order they were started.
pub(crate) fn decode(input: &Input, world: &World) -> Decoded {
    let mut cx = Context {
        input,
        world,
        strangers: 0,
    };
    let mut out = Decoded::default();
    // (start frame, entity) of each, to sort by.
    let mut reinforcements: Vec<((i64, u64), Reinforcement)> = Vec::new();
    let mut barricades: Vec<((i64, u64), Barricade)> = Vec::new();
    let panels = (world.replaced.iter().chain(world.iter())).filter(|e| is_panel(e));
    for entity in panels {
        for life in lives(entity) {
            let key = (life.start.map_or(-1, i64::from), entity.id);
            let took = match (life.complete, life.default) {
                (Some(complete), false) => (cx.seconds(complete))
                    .zip(cx.seconds(life.start))
                    .map(|(done, start)| done - start),
                _ => None,
            };
            let (what, kind_source) = kind(entity.asset, &life, took);
            let username = life.owner.and_then(|id| cx.username(id));
            if life.owner.is_some() && username.is_none() {
                cx.strangers += 1;
            }
            let position = rounded(life.position);
            let normal = rounded(World::rotate(life.rotation, [0.0, 0.0, 1.0]));
            // A default panel was not placed at a time anyone chose.
            let started = (!life.default).then(|| cx.when(life.start));
            let completed = match life.complete {
                Some(frame) if !life.default => Some(cx.when(frame)),
                _ => None,
            };
            let ended = life.end.map(|(_, frame)| cx.when(frame));
            let destroyed = cx.destroyed(&life);
            match what {
                PanelAsset::Barricade { castle, door, wide } => {
                    let known = kind_source.is_none();
                    let opening = if door { Opening::Door } else { Opening::Window };
                    let barricade = Barricade {
                        entity: entity.id,
                        kind: if castle {
                            BarricadeKind::Castle
                        } else {
                            BarricadeKind::Barricade
                        },
                        kind_source,
                        asset: entity.asset,
                        opening: known.then_some(opening),
                        wide: known.then_some(wide),
                        opening_inferred: known,
                        username,
                        host: life.host,
                        position,
                        normal,
                        default: life.default,
                        started,
                        completed,
                        cancelled: life.cancelled(),
                        ended,
                        destroyed,
                    };
                    barricades.push((key, barricade));
                }
                PanelAsset::Wall { .. } | PanelAsset::Hatch => {
                    let width = match what {
                        PanelAsset::Wall { width } => width,
                        _ => None,
                    };
                    let reinforcement = Reinforcement {
                        entity: entity.id,
                        kind: if what == PanelAsset::Hatch {
                            ReinforcementKind::Hatch
                        } else {
                            ReinforcementKind::Wall
                        },
                        kind_source,
                        asset: entity.asset,
                        username,
                        host: life.host,
                        position,
                        normal,
                        width,
                        width_inferred: width.is_some(),
                        started,
                        completed,
                        cancelled: life.cancelled(),
                        default: life.default,
                        ended,
                        destroyed,
                        opened: None,
                    };
                    reinforcements.push((key, reinforcement));
                }
            }
        }
    }
    reinforcements.sort_by_key(|r| r.0);
    barricades.sort_by_key(|b| b.0);
    out.reinforcements = reinforcements.into_iter().map(|r| r.1).collect();
    out.barricades = barricades.into_iter().map(|b| b.1).collect();
    if cx.strangers > 0 {
        out.warnings.push(format!(
            "{} panels were placed by a player the round does not list",
            cx.strangers
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_1_SQRT_2;

    use super::*;
    use crate::world::{Change, Damage, Placed};

    const WALL: u64 = 417_911_060_940;
    const OWNER: u64 = 77;
    const HOST: u64 = 0x60_572F_5567;
    const PLACE: [f32; 3] = [-53.1, -3.85, 0.0];
    /// Facing -x: (0, 0, 1) turned a quarter about y.
    const UPRIGHT: [f32; 4] = [0.0, -FRAC_1_SQRT_2, 0.0, FRAC_1_SQRT_2];

    fn panel(changes: Vec<Change>) -> Entity {
        Entity {
            id: 0xF02B_B5B3,
            asset: WALL,
            classes: vec![PLACED, DAMAGE],
            slots: vec![(SLOTS[1], 0), (SLOTS[0], 0)],
            changes,
            ..Entity::default()
        }
    }

    /// The full state a pooled panel is created with.
    fn pooled(frame: Frame) -> Change {
        Change {
            frame,
            full: frame.is_none(),
            position: Some(crate::world::POOL),
            live: Some(0),
            ..Change::default()
        }
    }

    /// A player starts placing it.
    fn start(frame: u32) -> Change {
        Change {
            frame: Some(frame),
            position: Some(PLACE),
            rotation: Some(UPRIGHT),
            placed: Some(Placed {
                owner: Some(OWNER),
                ..Placed::default()
            }),
            ..Change::default()
        }
    }

    fn live(frame: u32, host: Option<u64>) -> Change {
        Change {
            frame: Some(frame),
            live: Some(1),
            placed: host.map(|host| Placed {
                host: Some(host),
                ..Placed::default()
            }),
            ..Change::default()
        }
    }

    fn host(frame: u32) -> Change {
        Change {
            frame: Some(frame),
            placed: Some(Placed {
                host: Some(HOST),
                ..Placed::default()
            }),
            ..Change::default()
        }
    }

    fn blow(frame: u32, kind: u8, id: u64, instigator: u64) -> Change {
        Change {
            frame: Some(frame),
            damage: vec![Damage {
                kind,
                id,
                instigator,
                ..Damage::default()
            }],
            ..Change::default()
        }
    }

    fn destroyed(frame: u32) -> Change {
        Change {
            frame: Some(frame),
            destroyed: true,
            ..Change::default()
        }
    }

    /// 30 frames a second.
    fn clock(frame: Frame) -> Option<f64> {
        Some(f64::from(frame.unwrap_or(0)) / 30.0)
    }

    #[test]
    fn a_panel_is_told_by_classes_and_the_set_of_slots() {
        assert!(is_panel(&panel(Vec::new())));
        let mut other = panel(Vec::new());
        other.slots = vec![(SLOTS[0], 0), ([1, 2, 3, 4], 0)];
        assert!(!is_panel(&other));
        let mut other = panel(Vec::new());
        other.classes = vec![PLACED];
        assert!(!is_panel(&other));
        let mut other = panel(Vec::new());
        other.slots.push((SLOTS[0], 0));
        assert!(!is_panel(&other));
    }

    #[test]
    fn a_panel_in_the_pool_has_no_life() {
        assert_eq!(lives(&panel(vec![pooled(None)])), Vec::new());
    }

    #[test]
    fn a_placed_panel_completes_with_live_and_host() {
        let all = lives(&panel(vec![
            pooled(None),
            start(561),
            live(681, Some(HOST)),
        ]));
        assert_eq!(all.len(), 1);
        let life = &all[0];
        assert_eq!(life.start, Some(561));
        assert_eq!(life.position, PLACE);
        assert_eq!(life.rotation, UPRIGHT);
        assert_eq!(life.owner, Some(OWNER));
        assert!(!life.default);
        assert_eq!(life.complete, Some(Some(681)));
        assert_eq!(life.host, Some(HOST));
        assert_eq!(life.end, None);
        assert!(!life.cancelled());
    }

    #[test]
    fn the_host_may_come_after_live() {
        let all = lives(&panel(vec![
            pooled(None),
            start(100),
            live(220, None),
            host(224),
        ]));
        assert_eq!(all[0].complete, Some(Some(220)));
        assert_eq!(all[0].host, Some(HOST));
    }

    #[test]
    fn back_to_the_pool_before_live_is_called_off() {
        let all = lives(&panel(vec![pooled(None), start(725), pooled(Some(753))]));
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].complete, None);
        assert_eq!(all[0].end, Some((End::CalledOff, Some(753))));
        assert!(all[0].cancelled());
    }

    #[test]
    fn deleted_before_live_is_called_off() {
        let mut entity = panel(vec![pooled(None), start(725)]);
        entity.deleted = Some(760);
        let all = lives(&entity);
        assert_eq!(all[0].end, Some((End::CalledOff, Some(760))));
    }

    #[test]
    fn an_entity_is_one_panel_after_the_other() {
        let all = lives(&panel(vec![
            pooled(None),
            start(100),
            pooled(Some(130)),
            start(200),
            live(320, Some(HOST)),
            pooled(Some(900)),
        ]));
        assert_eq!(all.len(), 2);
        assert!(all[0].cancelled());
        assert_eq!(all[1].start, Some(200));
        assert_eq!(all[1].complete, Some(Some(320)));
        // A standing panel taken back is not called off.
        assert_eq!(all[1].end, Some((End::Taken, Some(900))));
    }

    #[test]
    fn a_panel_with_no_owner_is_the_maps() {
        let placed = Change {
            frame: Some(2),
            full: true,
            position: Some(PLACE),
            rotation: Some(UPRIGHT),
            live: Some(1),
            flags: Some(0),
            placed: Some(Placed {
                type_index: Some((1, 0)),
                ..Placed::default()
            }),
            ..Change::default()
        };
        let mut entity = panel(vec![placed]);
        let all = lives(&entity);
        assert!(all[0].default);
        assert_eq!(all[0].owner, None);
        assert_eq!(all[0].complete, Some(Some(2)));
        // Deleted, it was taken away, not called off.
        entity.deleted = Some(50);
        assert_eq!(lives(&entity)[0].end, Some((End::Taken, Some(50))));
    }

    #[test]
    fn a_start_without_a_rotation_keeps_the_last_or_identity() {
        let mut bare = start(10);
        bare.rotation = None;
        let all = lives(&panel(vec![pooled(None), bare.clone()]));
        assert_eq!(all[0].rotation, IDENTITY);
        let mut turned = pooled(Some(5));
        turned.rotation = Some(UPRIGHT);
        let all = lives(&panel(vec![pooled(None), turned, bare]));
        assert_eq!(all[0].rotation, UPRIGHT);
    }

    #[test]
    fn the_cause_is_the_last_record_up_to_the_end() {
        let body = 0xF000_0001;
        let all = lives(&panel(vec![
            start(100),
            live(180, Some(HOST)),
            blow(300, 1, 39_471_599_164, body),
            blow(330, 1, 39_471_599_164, body),
            // A neighbour going up is not what broke it.
            blow(331, 0, REINFORCING, body),
            destroyed(333),
            // What strikes the remains is not either.
            blow(1500, 0, 34_118_943_318, 0xF000_0002),
        ]));
        let life = &all[0];
        assert_eq!(life.destroyed, Some(Some(333)));
        assert_eq!(life.blows.len(), 4);
        let cause = life.cause(clock).unwrap();
        assert_eq!(cause.frame, Some(330));
        assert_eq!(how(&cause, false), How::Bullet);
        // Destroyed panels keep their place: no end besides.
        assert_eq!(life.end, None);
    }

    #[test]
    fn a_record_in_the_message_of_the_end_is_the_cause() {
        let mut last = blow(400, 0, MELEE, 0xF000_0001);
        last.destroyed = true;
        let all = lives(&panel(vec![start(100), live(180, None), last]));
        let cause = all[0].cause(clock).unwrap();
        assert_eq!(how(&cause, false), How::Melee);
    }

    #[test]
    fn an_old_record_is_no_cause() {
        let all = lives(&panel(vec![
            start(100),
            live(180, None),
            blow(300, 0, MELEE, 0xF000_0001),
            destroyed(300 + 61),
        ]));
        assert_eq!(all[0].cause(clock), None);
        let all = lives(&panel(vec![start(100), live(180, None), destroyed(400)]));
        assert_eq!(all[0].cause(clock), None);
        // Not destroyed: no cause.
        let all = lives(&panel(vec![start(100), blow(300, 0, MELEE, 1)]));
        assert_eq!(all[0].cause(clock), None);
    }

    #[test]
    fn how_goes_by_id_then_kind_then_instigator() {
        let of = |kind, id| Blow {
            kind,
            id,
            ..Blow::default()
        };
        assert_eq!(how(&of(0, MELEE), false), How::Melee);
        assert_eq!(how(&of(0, EXPLOSIONS[1]), true), How::Explosion);
        assert_eq!(how(&of(1, 5), false), How::Bullet);
        assert_eq!(how(&of(2, 5), false), How::Bullet);
        assert_eq!(how(&of(3, 5), false), How::Bullet);
        assert_eq!(how(&of(0, 5), true), How::Gadget);
        assert_eq!(how(&of(0, 5), false), How::Other);
    }

    #[test]
    fn a_known_asset_names_the_kind() {
        let life = Life::default();
        assert_eq!(
            kind(WALL, &life, None),
            (PanelAsset::Wall { width: Some(1.7) }, None)
        );
        assert_eq!(kind_of(WALL), Some(PanelKind::WallReinforcement));
        assert_eq!(
            kind_of(406_076_330_074),
            Some(PanelKind::HatchReinforcement)
        );
        assert_eq!(kind_of(406_076_330_090), Some(PanelKind::Barricade));
        assert_eq!(kind_of(361_321_226_207), Some(PanelKind::CastlePanel));
        assert_eq!(kind_of(1), None);
    }

    #[test]
    fn an_unknown_asset_goes_by_behaviour() {
        let barricade = PanelAsset::Barricade {
            castle: false,
            door: false,
            wide: false,
        };
        let derived = Some(Source::Derived);
        let upright = Life {
            rotation: UPRIGHT,
            ..Life::default()
        };
        let flat = Life::default();
        let default = Life {
            default: true,
            ..Life::default()
        };
        // Nobody placed it, even when it lies flat.
        assert_eq!(kind(1, &default, None), (barricade, derived));
        assert_eq!(kind(1, &flat, Some(2.5)), (PanelAsset::Hatch, derived));
        assert_eq!(kind(1, &upright, Some(2.5)), (barricade, derived));
        let wall = PanelAsset::Wall { width: None };
        assert_eq!(kind(1, &upright, Some(4.1)), (wall, derived));
        // Called off: nothing says it is a barricade.
        assert_eq!(kind(1, &upright, None), (wall, derived));
    }

    #[test]
    fn the_nearest_body_is_level_and_on_the_same_floor() {
        let mut world = World::default();
        for (body, player, at) in [
            (1u64, 0usize, [0.4, 0.0, 0.0]),
            // Nearer, on another floor.
            (2, 1, [0.1, 0.0, 4.0]),
            (3, 2, [3.0, 0.0, 0.0]),
        ] {
            world.bodies.insert(body, vec![(10, at)]);
            world.players.insert(body, player);
        }
        let (distance, player) = nearest(&world, Some(20), [0.0; 3]).unwrap();
        assert_eq!(player, 0);
        assert!((distance - 0.4).abs() < 1e-6);
        assert!(REMOVING.contains(&distance));
        // Before any body has a position.
        assert_eq!(nearest(&world, Some(5), [0.0; 3]), None);
    }

    #[test]
    fn positions_are_rounded_without_a_negative_zero() {
        let normal = rounded(World::rotate(UPRIGHT, [0.0, 0.0, 1.0]));
        assert_eq!(normal, [-1.0, 0.0, 0.0]);
        assert!(normal.iter().all(|v| !(*v == 0.0 && v.is_sign_negative())));
    }
}
