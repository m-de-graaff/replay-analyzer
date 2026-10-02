//! Line of sight (Y11S3): who could see whom, how exposed a player was and
//! where their crosshair was when an enemy came into view.
//!
//! # What the file gives, and what it does not
//!
//! A replay holds each body's origin, its view direction and its posture
//! (see [`crate::movement`]). It holds **no head or camera position** and
//! **no level geometry**: no mesh, no wall that was never touched.
//!
//! **The eye is derived.** A fire event (see [`crate::shots`]) carries the
//! distance from the shooter's eye to what the bullet struck, and the
//! strike point follows from the muzzle, the direction and the muzzle
//! distance. Fitting an eye offset to those distances over the 4,618 shots
//! of the ten test rounds fired off a rope gives, above the body's origin:
//!
//! ```text
//! standing  1.44 m   (median residual 1.5 cm)
//! crouched  0.96 m   (1.3 cm)
//! prone     0.39 m   (0.3 cm)
//! on a rope 0        the origin is at the eye there (13 cm)
//! lean      0.12 m to the side leaned to
//! ```
//!
//! with no offset forwards. The head a target shows is taken to be at the
//! same height: the bullets of headshot kills struck standing bodies 1.42 m
//! up (median of 17). Chest, pelvis, knees and feet are fractions of it,
//! and a prone body lies back from its head along the view direction;
//! those are assumed, not measured.
//!
//! **Geometry comes from outside**, as a [`Geometry`]: vertical wall quads
//! and horizontal slabs with openings. What a round did to it is applied
//! from the round ([`Scene::apply`]): reinforcements, barricades, holes
//! ([`crate::destruction::Surface`]), hatches and smoke. Without one every
//! function still answers, as an open field: distance and field of view
//! only. The output then says `geometry: "none"` and `occlusion: false`,
//! and nothing in it means that a wall was tested.
//!
//! # What a line is tested against
//!
//! | Element | Sight | Bullet |
//! |---|---|---|
//! | `solid` wall, slab | blocked | blocked |
//! | `soft` wall, `soft` slab | blocked | passes |
//! | the same, reinforced | blocked | blocked |
//! | `window`, `door` | passes | passes |
//! | the same, barricaded | blocked | passes |
//! | the same, with an Armor Panel | blocked | blocked |
//! | `seeThrough` | passes | blocked |
//! | a hole of a wall, an open hatch, an opening of a slab | passes | passes |
//! | smoke | blocked | passes |
//!
//! A bullet that struck a player therefore proves that nothing in the
//! bullet column stood between the shooter's eye and the point struck,
//! and so does every shot up to where it ended. [`validate`] counts the
//! hits and the shots a geometry contradicts.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::Phase;
use crate::Round;
use crate::areas::AreaKind;
use crate::destruction::{Label, Plane};
use crate::loadout::When;
use crate::movement::{Change, Doing, Lean, PlayerTrack, Stance};
use crate::panels::{BarricadeKind, Opening as PanelOpening, ReinforcementKind};

/// Metres from a body's origin to its eye, by stance. Measured: see the
/// module documentation.
pub const EYE_STANDING: f32 = 1.44;
pub const EYE_CROUCHED: f32 = 0.96;
pub const EYE_PRONE: f32 = 0.39;
/// Metres the eye moves to the side leaned to. Measured.
pub const LEAN_SHIFT: f32 = 0.12;
/// Metres from the head of an upright body on a rope down to its feet.
/// Assumed.
const ROPE_BODY: f32 = 1.4;
/// Metres a prone body reaches back from its head. Assumed.
const PRONE_BODY: f32 = 1.6;
/// A crossing closer than this to either end of a line does not block it:
/// an eye or a target point is never exactly clear of the wall it touches.
const MARGIN: f64 = 0.01;
/// Side of a grid cell, in metres.
const CELL: f64 = 2.0;
/// A track sample older than this says nothing of where the player is.
const STALE: f64 = 1.0;

type V3 = [f64; 3];

fn v3(p: [f32; 3]) -> V3 {
    p.map(f64::from)
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// To the millimetre, and no `-0.0`.
fn rounded(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0 + 0.0
}

/// What a wall quad is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WallKind {
    /// Stops sight and bullets.
    #[default]
    Solid,
    /// Stops sight; bullets go through, and it can be reinforced.
    Soft,
    /// An opening unless a barricade is up.
    Window,
    Door,
    /// Stops bullets and not sight: bulletproof glass.
    SeeThrough,
}

/// A vertical quad: the segment `a` to `b` seen from above, from `bottom`
/// to `top`. It has no thickness.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub bottom: f32,
    pub top: f32,
    #[serde(default)]
    pub kind: WallKind,
    /// The map object it is, in hex: the `host` of a reinforcement or a
    /// barricade. A panel of a round is matched by it, else by where it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
}

/// A gap in a slab.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Opening {
    pub polygon: Vec<[f32; 2]>,
    /// A hatch: closed until the round destroys it. Any other opening (a
    /// stairwell) is always open.
    #[serde(default)]
    pub hatch: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
}

/// A floor or a ceiling: a polygon at one height.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Slab {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub z: f32,
    pub polygon: Vec<[f32; 2]>,
    /// Bullets go through it.
    #[serde(default)]
    pub soft: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub openings: Vec<Opening>,
}

/// The geometry of a map, in the map's coordinates (metres, z up).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Geometry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map: Option<String>,
    /// Where it is from: `authored`, `harvested`, ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub walls: Vec<Wall>,
    #[serde(default)]
    pub slabs: Vec<Slab>,
}

/// Height of a wall panel, door and window harvested from a round, and the
/// widths of what a barricade closes. None is in the file: all assumed.
/// Of the 60,866 shots of 175 real rounds, 18 went on over a standing
/// reinforcement 2.6 m and more up, and none between 2.0 and 2.6.
const PANEL_HEIGHT: f32 = 2.6;
const PANEL_WIDTH: f32 = 2.0;
/// The share of a reinforcement's width (inferred from its asset, see
/// [`crate::panels`]) taken to stop a bullet: 84 of the 148 shots that
/// went on behind a standing one crossed its last tenth.
const PANEL_SHARE: f32 = 0.9;
const DOOR_HEIGHT: f32 = 2.2;
const WINDOW_HEIGHT: f32 = 1.45;
const OPENING_WIDTH: f32 = 1.3;
const HATCH_SIDE: f32 = 2.0;
/// A reinforcement stands this far in front of its wall, a barricade this
/// far in front of the middle of its frame (two panels on one host are
/// 0.2 and 0.25 m apart).
const REINFORCEMENT_OFFSET: f32 = 0.1;
const BARRICADE_OFFSET: f32 = 0.125;

impl Geometry {
    /// The panels rounds put up, as geometry: every wall that was
    /// reinforced as a `soft` wall, every door and window that had a
    /// barricade, every reinforced hatch as a patch of floor with a hatch
    /// in it. That is a small part of a map (Bank: 24 walls of the ten
    /// test rounds), and every extent is assumed; it is here to test the
    /// module against replays, not to stand in for a map.
    pub fn harvest<'a>(rounds: impl IntoIterator<Item = &'a Round>) -> Geometry {
        let mut out = Geometry {
            source: Some("harvested".to_owned()),
            ..Geometry::default()
        };
        let mut seen: HashMap<(u8, [i32; 3]), ()> = HashMap::new();
        let key = |p: [f32; 3]| p.map(|v| (v * 2.0).round() as i32);
        for round in rounds {
            out.map.get_or_insert_with(|| round.header.map.to_string());
            for r in &round.reinforcements {
                if r.cancelled || !(r.completed.is_some() || r.default) {
                    continue;
                }
                let [x, y, z] = r.position;
                let object = r.host.map(|h| format!("{h:x}"));
                if r.kind == ReinforcementKind::Hatch {
                    if seen.insert((0, key(r.position)), ()).is_some() {
                        continue;
                    }
                    let h = HATCH_SIDE / 2.0;
                    let square = vec![
                        [x - h, y - h],
                        [x + h, y - h],
                        [x + h, y + h],
                        [x - h, y + h],
                    ];
                    out.slabs.push(Slab {
                        id: Some(format!("hatch {x:.1} {y:.1} {z:.1}")),
                        z,
                        polygon: square.clone(),
                        soft: true,
                        openings: vec![Opening {
                            polygon: square,
                            hatch: true,
                            object,
                        }],
                    });
                    continue;
                }
                let [nx, ny, _] = r.normal;
                let (cx, cy) = (x - nx * REINFORCEMENT_OFFSET, y - ny * REINFORCEMENT_OFFSET);
                if seen.insert((1, key([cx, cy, z])), ()).is_some() {
                    continue;
                }
                let half = r.width.unwrap_or(PANEL_WIDTH) * PANEL_SHARE / 2.0;
                out.walls.push(Wall {
                    id: Some(format!("wall {cx:.1} {cy:.1} {z:.1}")),
                    a: [cx + ny * half, cy - nx * half],
                    b: [cx - ny * half, cy + nx * half],
                    bottom: z,
                    top: z + PANEL_HEIGHT,
                    kind: WallKind::Soft,
                    object,
                });
            }
            for b in &round.barricades {
                if b.cancelled {
                    continue;
                }
                let [x, y, z] = b.position;
                let [nx, ny, _] = b.normal;
                let (cx, cy) = (x - nx * BARRICADE_OFFSET, y - ny * BARRICADE_OFFSET);
                if seen.insert((2, key([cx, cy, z])), ()).is_some() {
                    continue;
                }
                let window = b.opening == Some(PanelOpening::Window);
                let wide = if b.wide == Some(true) { 2.0 } else { 1.0 };
                let half = OPENING_WIDTH * wide / 2.0;
                let name = if window { "window" } else { "door" };
                out.walls.push(Wall {
                    id: Some(format!("{name} {cx:.1} {cy:.1} {z:.1}")),
                    a: [cx + ny * half, cy - nx * half],
                    b: [cx - ny * half, cy + nx * half],
                    // A barricade's position is the top of its frame.
                    bottom: z - if window { WINDOW_HEIGHT } else { DOOR_HEIGHT },
                    top: z,
                    kind: if window {
                        WallKind::Window
                    } else {
                        WallKind::Door
                    },
                    object: b.host.map(|h| format!("{h:x}")),
                });
            }
        }
        out
    }
}

/// What a line is: see the table of the module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ray {
    Sight,
    Bullet,
    /// Stopped by whatever a bullet would strike, and so by everything
    /// that stops sight or bullets but smoke.
    Any,
}

/// What covers a wall for a while.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverKind {
    Reinforcement,
    Barricade,
    /// Castle's Armor Panel.
    ArmorPanel,
}

/// A panel on a wall from `start` until `end`, in seconds since the
/// recording started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cover {
    pub kind: CoverKind,
    pub start: f64,
    pub end: f64,
}

/// A hole in a wall from `from` on: metres along the wall from `a`, and
/// heights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hole {
    pub from: f64,
    pub along: [f32; 2],
    pub bottom: f32,
    pub top: f32,
}

/// A cloud that stops sight from `start` until `end`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Smoke {
    pub position: [f32; 3],
    pub radius: f32,
    pub start: f64,
    pub end: f64,
}

/// The state of one hatch: when it was reinforced, and when it went.
#[derive(Clone, Debug, Default, PartialEq)]
struct Hatch {
    reinforced: Vec<[f64; 2]>,
    opened: Option<f64>,
}

/// What [`Scene::apply`] took from a round, and what it found no place for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub reinforcements: usize,
    pub reinforcements_unmatched: usize,
    pub barricades: usize,
    pub barricades_unmatched: usize,
    pub holes: usize,
    pub holes_unmatched: usize,
    pub hatches_opened: usize,
    pub smokes: usize,
}

/// The kind of thing that stops a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Element {
    Wall,
    Slab,
    Smoke,
}

/// What stops a line first.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Blocker {
    pub element: Element,
    /// Index in `walls` or `slabs` of the geometry, or of the smoke.
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The panel on it that stops the line, when it is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover: Option<CoverKind>,
    /// Metres from the start of the line.
    pub distance: f32,
    pub point: [f32; 3],
}

/// A uniform grid over x and y: the elements whose bounds touch each cell.
#[derive(Clone, Debug, Default)]
struct Grid {
    /// The cell of the lowest x and y, and how many there are each way.
    origin: [i32; 2],
    size: [i32; 2],
    cells: Vec<Vec<u32>>,
}

fn cell(v: f64) -> i32 {
    (v / CELL).floor() as i32
}

/// The cells a set of points spans, with a little around them.
fn span(points: impl Iterator<Item = [f32; 2]>) -> Option<([i32; 2], [i32; 2])> {
    let (mut lo, mut hi) = ([i32::MAX; 2], [i32::MIN; 2]);
    for p in points {
        for k in 0..2 {
            lo[k] = lo[k].min(cell(f64::from(p[k]) - 0.05));
            hi[k] = hi[k].max(cell(f64::from(p[k]) + 0.05));
        }
    }
    (lo[0] <= hi[0]).then_some((lo, hi))
}

impl Grid {
    /// A grid over `elements`, each the points that bound it.
    fn new(elements: &[Vec<[f32; 2]>]) -> Grid {
        let Some((lo, hi)) = span(elements.iter().flatten().copied()) else {
            return Grid::default();
        };
        let size = [hi[0] - lo[0] + 1, hi[1] - lo[1] + 1];
        let mut cells = vec![Vec::new(); (size[0] * size[1]) as usize];
        for (id, points) in elements.iter().enumerate() {
            let Some((a, b)) = span(points.iter().copied()) else {
                continue;
            };
            for cx in a[0]..=b[0] {
                for cy in a[1]..=b[1] {
                    cells[((cx - lo[0]) * size[1] + cy - lo[1]) as usize].push(id as u32);
                }
            }
        }
        Grid {
            origin: lo,
            size,
            cells,
        }
    }

    fn get(&self, c: [i32; 2]) -> &[u32] {
        let (x, y) = (c[0] - self.origin[0], c[1] - self.origin[1]);
        if x < 0 || y < 0 || x >= self.size[0] || y >= self.size[1] {
            return &[];
        }
        &self.cells[(x * self.size[1] + y) as usize]
    }

    /// Calls `f` with the elements of every cell the segment passes, in
    /// order, until it returns `true`.
    fn walk(&self, a: [f64; 2], b: [f64; 2], mut f: impl FnMut(&[u32]) -> bool) {
        let (mut c, end) = ([cell(a[0]), cell(a[1])], [cell(b[0]), cell(b[1])]);
        let mut step = [0i32; 2];
        let mut next = [f64::INFINITY; 2];
        let mut delta = [f64::INFINITY; 2];
        for k in 0..2 {
            let d = b[k] - a[k];
            if c[k] != end[k] {
                step[k] = if d > 0.0 { 1 } else { -1 };
                let edge = f64::from(c[k] + i32::from(d > 0.0)) * CELL;
                next[k] = (edge - a[k]) / d;
                delta[k] = CELL / d.abs();
            }
        }
        let steps = (end[0] - c[0]).abs() + (end[1] - c[1]).abs();
        for i in 0..=steps {
            let ids = self.get(c);
            if !ids.is_empty() && f(ids) {
                return;
            }
            if i == steps {
                break;
            }
            // A side that reached its end cell is never stepped again.
            let k = match (c[0] == end[0], c[1] == end[1]) {
                (true, _) => 1,
                (_, true) => 0,
                _ => usize::from(next[1] < next[0]),
            };
            c[k] += step[k];
            next[k] += delta[k];
        }
    }
}

fn inside(polygon: &[[f32; 2]], x: f64, y: f64) -> bool {
    let mut odd = false;
    let n = polygon.len();
    for i in 0..n {
        let (a, b) = (
            polygon[i].map(f64::from),
            polygon[(i + n - 1) % n].map(f64::from),
        );
        if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
            odd = !odd;
        }
    }
    odd
}

/// Metres from a point to a segment, seen from above, and how far along
/// the segment the nearest point is.
fn to_segment(a: [f32; 2], b: [f32; 2], x: f64, y: f64) -> (f64, f64) {
    let (a, b) = (a.map(f64::from), b.map(f64::from));
    let (sx, sy) = (b[0] - a[0], b[1] - a[1]);
    let len = sx.hypot(sy);
    if len == 0.0 {
        return ((x - a[0]).hypot(y - a[1]), 0.0);
    }
    let along = (((x - a[0]) * sx + (y - a[1]) * sy) / len).clamp(0.0, len);
    let (px, py) = (a[0] + sx / len * along, a[1] + sy / len * along);
    ((x - px).hypot(y - py), along)
}

/// A panel of a round belongs to the wall within this many metres of it.
const PANEL_REACH: f64 = 0.6;
/// A hole belongs to the wall within this many metres of its middle.
const HOLE_REACH: f64 = 0.35;
/// A hatch of a round is the one of the geometry within this many metres.
const HATCH_REACH: f64 = 1.5;
/// A hole is the extent of its impacts and this much around it, and no
/// smaller than [`hole_least`] says. Assumed: the file has no hole sizes.
const HOLE_MARGIN: f64 = 0.15;

/// Half the width and half the height a hole of a label has at least.
fn hole_least(label: Label) -> Option<(f64, f64)> {
    match label {
        Label::Breach | Label::RotationHole => Some((0.5, 0.7)),
        Label::MurderHole => Some((0.15, 0.15)),
        Label::VerticalPlay | Label::BulletHoles => None,
    }
}

fn seconds(when: &Option<When>) -> Option<f64> {
    when.as_ref()?.recording_time
}

/// A crossing of a line: how far along it, as a share, and what with.
type Found = (f64, Element, usize, Option<CoverKind>);

fn nearer(best: &mut Option<Found>, found: Found) {
    if best.is_none_or(|b| found.0 < b.0) {
        *best = Some(found);
    }
}

/// A geometry with its index and the state a round put it in. Every query
/// takes the time, in seconds since the recording started.
#[derive(Clone, Debug, Default)]
pub struct Scene<'a> {
    geometry: Option<&'a Geometry>,
    grid: Grid,
    covers: Vec<Vec<Cover>>,
    holes: Vec<Vec<Hole>>,
    hatches: Vec<Vec<Hatch>>,
    smokes: Vec<Smoke>,
    applied: Option<Applied>,
}

impl Scene<'static> {
    /// No geometry: nothing ever stops a line.
    pub fn open_field() -> Self {
        Scene::default()
    }
}

impl<'a> Scene<'a> {
    /// The geometry as authored: no panel, no hole, every hatch closed.
    pub fn new(geometry: &'a Geometry) -> Self {
        let walls = geometry.walls.len();
        let bounds: Vec<Vec<[f32; 2]>> = (geometry.walls.iter().map(|w| vec![w.a, w.b]))
            .chain(geometry.slabs.iter().map(|s| s.polygon.clone()))
            .collect();
        let grid = Grid::new(&bounds);
        Scene {
            geometry: Some(geometry),
            grid,
            covers: vec![Vec::new(); walls],
            holes: vec![Vec::new(); walls],
            hatches: (geometry.slabs.iter())
                .map(|s| vec![Hatch::default(); s.openings.len()])
                .collect(),
            smokes: Vec::new(),
            applied: None,
        }
    }

    /// The geometry in the state of a round.
    pub fn for_round(geometry: &'a Geometry, round: &Round) -> Self {
        let mut scene = Scene::new(geometry);
        scene.apply(round);
        scene
    }

    /// Whether lines are tested against anything. `false` for an open
    /// field.
    pub fn occlusion(&self) -> bool {
        self.geometry.is_some() || !self.smokes.is_empty()
    }

    pub fn geometry(&self) -> Option<&'a Geometry> {
        self.geometry
    }

    /// What [`Scene::apply`] took from the round.
    pub fn applied(&self) -> Option<&Applied> {
        self.applied.as_ref()
    }

    /// Puts a panel on a wall.
    pub fn cover(&mut self, wall: usize, cover: Cover) {
        self.covers[wall].push(cover);
    }

    /// Cuts a hole into a wall.
    pub fn hole(&mut self, wall: usize, hole: Hole) {
        self.holes[wall].push(hole);
    }

    /// Opens a hatch for good at `time`.
    pub fn open_hatch(&mut self, slab: usize, opening: usize, time: f64) {
        self.hatches[slab][opening].opened = Some(time);
    }

    /// Reinforces a hatch from `start` until `end`.
    pub fn reinforce_hatch(&mut self, slab: usize, opening: usize, start: f64, end: f64) {
        self.hatches[slab][opening].reinforced.push([start, end]);
    }

    pub fn smoke(&mut self, smoke: Smoke) {
        self.smokes.push(smoke);
    }

    /// The wall a panel at `position` is on: the one that names `object`,
    /// else the nearest that `fits`.
    fn wall_at(
        &self,
        object: Option<u64>,
        position: [f32; 3],
        reach: f64,
        fits: impl Fn(WallKind) -> bool,
    ) -> Option<(usize, f64)> {
        let walls = &self.geometry?.walls;
        let [x, y, z] = v3(position);
        if let Some(object) = object.map(|o| format!("{o:x}")) {
            let named = (walls.iter()).position(|w| w.object.as_deref() == Some(object.as_str()));
            if let Some(i) = named.filter(|&i| fits(walls[i].kind)) {
                return Some((i, to_segment(walls[i].a, walls[i].b, x, y).1));
            }
        }
        (walls.iter().enumerate())
            .filter(|(_, w)| fits(w.kind))
            .filter(|(_, w)| z > f64::from(w.bottom) - 0.5 && z < f64::from(w.top) + 0.5)
            .map(|(i, w)| (i, to_segment(w.a, w.b, x, y)))
            .filter(|(_, (d, _))| *d < reach)
            .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
            .map(|(i, (_, along))| (i, along))
    }

    fn hatch_at(&self, object: Option<u64>, position: [f64; 3]) -> Option<(usize, usize)> {
        let slabs = &self.geometry?.slabs;
        let object = object.map(|o| format!("{o:x}"));
        let mut best: Option<(f64, usize, usize)> = None;
        for (i, s) in slabs.iter().enumerate() {
            for (j, o) in s.openings.iter().enumerate().filter(|(_, o)| o.hatch) {
                if object.is_some() && o.object == object {
                    return Some((i, j));
                }
                let n = o.polygon.len().max(1) as f64;
                let cx = o.polygon.iter().map(|p| f64::from(p[0])).sum::<f64>() / n;
                let cy = o.polygon.iter().map(|p| f64::from(p[1])).sum::<f64>() / n;
                let d = (position[0] - cx).hypot(position[1] - cy);
                let level = (position[2] - f64::from(s.z)).abs() < 1.0;
                if level && d < HATCH_REACH && best.is_none_or(|b| d < b.0) {
                    best = Some((d, i, j));
                }
            }
        }
        best.map(|(_, i, j)| (i, j))
    }

    /// Applies what the round did to the map: reinforcements and
    /// barricades with their times, the holes of `surfaces`, hatches that
    /// went, and smoke. A panel or hole with no wall of the geometry near
    /// it is counted as unmatched and changes nothing.
    pub fn apply(&mut self, round: &Round) -> &Applied {
        let mut applied = Applied::default();
        let reinforced = |kind| kind == WallKind::Soft || kind == WallKind::Solid;
        let opening = |kind| kind == WallKind::Window || kind == WallKind::Door;
        // Hatches the round destroyed, before the reinforcements that may
        // say otherwise.
        for s in &round.surfaces {
            let Some(time) = s.hatch_destroyed else {
                continue;
            };
            if let Some((i, j)) = self.hatch_at(None, s.position) {
                let h = &mut self.hatches[i][j];
                h.opened = Some(h.opened.map_or(time, |t| t.min(time)));
            }
        }
        for r in &round.reinforcements {
            let start = match seconds(&r.completed) {
                Some(t) => t,
                None if r.default && !r.cancelled => f64::NEG_INFINITY,
                None => continue,
            };
            let ends = [
                seconds(&r.opened),
                r.destroyed.as_ref().and_then(|d| d.when.recording_time),
                seconds(&r.ended),
            ];
            let end = ends.into_iter().flatten().fold(f64::INFINITY, f64::min);
            if r.kind == ReinforcementKind::Hatch {
                let Some((i, j)) = self.hatch_at(r.host, v3(r.position)) else {
                    applied.reinforcements_unmatched += 1;
                    continue;
                };
                let h = &mut self.hatches[i][j];
                h.reinforced.push([start, end]);
                // A hatch that stood to be reinforced went no earlier
                // than its reinforcement.
                if h.opened.is_none_or(|t| t < end) {
                    h.opened = end.is_finite().then_some(end);
                }
                applied.reinforcements += 1;
                continue;
            }
            match self.wall_at(r.host, r.position, PANEL_REACH, reinforced) {
                Some((wall, _)) => {
                    let kind = CoverKind::Reinforcement;
                    self.covers[wall].push(Cover { kind, start, end });
                    applied.reinforcements += 1;
                }
                None => applied.reinforcements_unmatched += 1,
            }
        }
        for b in &round.barricades {
            let start = match seconds(&b.completed) {
                Some(t) => t,
                None if b.default && !b.cancelled => f64::NEG_INFINITY,
                None => continue,
            };
            let ends = [
                b.destroyed.as_ref().and_then(|d| d.when.recording_time),
                seconds(&b.ended),
            ];
            let end = ends.into_iter().flatten().fold(f64::INFINITY, f64::min);
            // A barricade's position is the top of its frame.
            let [x, y, z] = b.position;
            match self.wall_at(b.host, [x, y, z - 0.6], PANEL_REACH, opening) {
                Some((wall, _)) => {
                    let kind = match b.kind {
                        BarricadeKind::Barricade => CoverKind::Barricade,
                        BarricadeKind::Castle => CoverKind::ArmorPanel,
                    };
                    self.covers[wall].push(Cover { kind, start, end });
                    applied.barricades += 1;
                }
                None => applied.barricades_unmatched += 1,
            }
        }
        for s in &round.surfaces {
            let Some((least_w, least_h)) = hole_least(s.label) else {
                continue;
            };
            if s.kind != Plane::Wall {
                continue;
            }
            // A hole bullets made is there once the last of them struck.
            let bullets = s.causes.keys().all(|c| *c == "bullet");
            let Some(from) = (if bullets {
                Some(s.until)
            } else {
                s.when.recording_time
            }) else {
                continue;
            };
            let position = s.position.map(|v| v as f32);
            let Some((wall, along)) = self.wall_at(None, position, HOLE_REACH, reinforced) else {
                applied.holes_unmatched += 1;
                continue;
            };
            let w = (s.width / 2.0 + HOLE_MARGIN).max(least_w);
            let h = (s.height / 2.0 + HOLE_MARGIN).max(least_h);
            self.holes[wall].push(Hole {
                from,
                along: [(along - w) as f32, (along + w) as f32],
                bottom: (s.position[2] - h) as f32,
                top: (s.position[2] + h) as f32,
            });
            applied.holes += 1;
        }
        for a in &round.areas {
            let (AreaKind::Smoke | AreaKind::Extinguisher, Some(radius)) = (a.kind, a.radius)
            else {
                continue;
            };
            self.smokes.push(Smoke {
                position: a.position,
                radius,
                start: a.start_seconds,
                end: a.end_seconds.unwrap_or(f64::INFINITY),
            });
            applied.smokes += 1;
        }
        applied.hatches_opened = (self.hatches.iter().flatten())
            .filter(|h| h.opened.is_some())
            .count();
        self.applied.insert(applied)
    }

    /// Whether wall `i` stops the ray at a point `along` it and `z` up,
    /// and the panel that does.
    fn wall_stops(
        &self,
        i: usize,
        kind: WallKind,
        along: f64,
        z: f64,
        time: f64,
        ray: Ray,
    ) -> Option<Option<CoverKind>> {
        let open = self.holes[i].iter().any(|h| {
            h.from <= time
                && along >= f64::from(h.along[0])
                && along <= f64::from(h.along[1])
                && z >= f64::from(h.bottom)
                && z <= f64::from(h.top)
        });
        if open {
            return None;
        }
        let cover = (self.covers[i].iter())
            .filter(|c| c.start <= time && time < c.end)
            .map(|c| c.kind)
            .max_by_key(|k| *k != CoverKind::Barricade);
        let hard = matches!(
            cover,
            Some(CoverKind::Reinforcement | CoverKind::ArmorPanel)
        );
        let (sight, bullet) = match kind {
            WallKind::Solid => (true, true),
            WallKind::Soft => (true, hard),
            WallKind::Window | WallKind::Door => (cover.is_some(), hard),
            WallKind::SeeThrough => (false, true),
        };
        let stops = match ray {
            Ray::Sight => sight,
            Ray::Bullet => bullet,
            Ray::Any => sight || bullet,
        };
        // The wall itself stops the line unless only its panel does.
        let panel = match kind {
            WallKind::Window | WallKind::Door => cover,
            WallKind::Soft if ray == Ray::Bullet => cover,
            _ => None,
        };
        stops.then_some(panel)
    }

    fn slab_stops(&self, i: usize, slab: &Slab, x: f64, y: f64, time: f64, ray: Ray) -> bool {
        if !inside(&slab.polygon, x, y) {
            return false;
        }
        for (o, state) in slab.openings.iter().zip(&self.hatches[i]) {
            if !inside(&o.polygon, x, y) {
                continue;
            }
            if !o.hatch || state.opened.is_some_and(|t| t <= time) {
                return false;
            }
            let reinforced = (state.reinforced.iter()).any(|r| r[0] <= time && time < r[1]);
            return ray != Ray::Bullet || reinforced;
        }
        ray != Ray::Bullet || !slab.soft
    }

    /// The first thing that stops the line from `from` to `to` at `time`.
    pub fn blocker(&self, from: [f32; 3], to: [f32; 3], time: f64, ray: Ray) -> Option<Blocker> {
        self.trace(from, to, time, ray, false)
    }

    /// Whether nothing stops sight from `eye` to `point` at `time`.
    pub fn visible(&self, eye: [f32; 3], point: [f32; 3], time: f64) -> bool {
        self.trace(eye, point, time, Ray::Sight, true).is_none()
    }

    /// `first`: any blocker will do, not the nearest.
    fn trace(
        &self,
        from: [f32; 3],
        to: [f32; 3],
        time: f64,
        ray: Ray,
        first: bool,
    ) -> Option<Blocker> {
        let (p, q) = (v3(from), v3(to));
        let d = sub(q, p);
        let len = norm(d);
        if len <= 2.0 * MARGIN {
            return None;
        }
        let eps = MARGIN / len;
        let mut best: Option<Found> = None;
        if let Some(g) = self.geometry {
            let walls = g.walls.len();
            self.grid.walk([p[0], p[1]], [q[0], q[1]], |ids| {
                let mut found = false;
                for &id in ids {
                    let id = id as usize;
                    if id < walls {
                        let w = &g.walls[id];
                        let (a, b) = (w.a.map(f64::from), w.b.map(f64::from));
                        let s = [b[0] - a[0], b[1] - a[1]];
                        let denom = d[0] * s[1] - d[1] * s[0];
                        if denom.abs() < 1e-12 {
                            continue;
                        }
                        let (ex, ey) = (a[0] - p[0], a[1] - p[1]);
                        let t = (ex * s[1] - ey * s[0]) / denom;
                        let u = (ex * d[1] - ey * d[0]) / denom;
                        if t <= eps || t >= 1.0 - eps || !(0.0..=1.0).contains(&u) {
                            continue;
                        }
                        let z = p[2] + t * d[2];
                        if z < f64::from(w.bottom) || z > f64::from(w.top) {
                            continue;
                        }
                        let along = u * s[0].hypot(s[1]);
                        if let Some(cover) = self.wall_stops(id, w.kind, along, z, time, ray) {
                            nearer(&mut best, (t, Element::Wall, id, cover));
                            found = true;
                        }
                    } else {
                        let slab = &g.slabs[id - walls];
                        if d[2].abs() < 1e-12 {
                            continue;
                        }
                        let t = (f64::from(slab.z) - p[2]) / d[2];
                        if t <= eps || t >= 1.0 - eps {
                            continue;
                        }
                        let (x, y) = (p[0] + t * d[0], p[1] + t * d[1]);
                        if self.slab_stops(id - walls, slab, x, y, time, ray) {
                            nearer(&mut best, (t, Element::Slab, id - walls, None));
                            found = true;
                        }
                    }
                }
                first && found
            });
        }
        if ray == Ray::Sight && !(first && best.is_some()) {
            for (i, s) in self.smokes.iter().enumerate() {
                if time < s.start || time >= s.end {
                    continue;
                }
                // Where the line enters the cloud; 0 from inside it.
                let m = sub(p, v3(s.position));
                let (b, c) = (dot(m, d) / len, dot(m, m) - f64::from(s.radius).powi(2));
                let disc = b * b - c;
                if disc < 0.0 {
                    continue;
                }
                let (near, far) = (-b - disc.sqrt(), -b + disc.sqrt());
                if far < 0.0 || near > len {
                    continue;
                }
                nearer(&mut best, (near.max(0.0) / len, Element::Smoke, i, None));
            }
        }
        let (t, element, index, cover) = best?;
        let id = match (element, self.geometry) {
            (Element::Wall, Some(g)) => g.walls[index].id.clone(),
            (Element::Slab, Some(g)) => g.slabs[index].id.clone(),
            _ => None,
        };
        Some(Blocker {
            element,
            index,
            id,
            cover,
            distance: (t * len) as f32,
            point: [0, 1, 2].map(|k| (p[k] + t * d[k]) as f32),
        })
    }

    /// Metres from a point to the plane of what a blocker is.
    fn off_plane(&self, blocker: &Blocker, point: [f32; 3]) -> f64 {
        let (Some(g), [x, y, z]) = (self.geometry, v3(point)) else {
            return f64::INFINITY;
        };
        match blocker.element {
            Element::Wall => {
                let w = &g.walls[blocker.index];
                let (a, b) = (w.a.map(f64::from), w.b.map(f64::from));
                let (sx, sy) = (b[0] - a[0], b[1] - a[1]);
                ((x - a[0]) * sy - (y - a[1]) * sx).abs() / sx.hypot(sy).max(1e-9)
            }
            Element::Slab => (z - f64::from(g.slabs[blocker.index].z)).abs(),
            Element::Smoke => f64::INFINITY,
        }
    }

    /// How much of a body an eye sees at `time`.
    pub fn visibility(&self, eye: [f32; 3], body: &Body, time: f64) -> Visibility {
        let points = body.points();
        let seen = points.map(|p| self.visible(eye, p, time));
        let count = seen.iter().filter(|s| **s).count();
        Visibility {
            any: count > 0,
            head: seen[0],
            fraction: count as f32 / points.len() as f32,
        }
    }
}

/// Whether nothing stops sight from `eye` to `point` at `time`.
pub fn visible(scene: &Scene, eye: [f32; 3], point: [f32; 3], time: f64) -> bool {
    scene.visible(eye, point, time)
}

/// What stops sight from `eye` to `point` at `time`, if anything.
pub fn blocking(scene: &Scene, eye: [f32; 3], point: [f32; 3], time: f64) -> Option<Blocker> {
    scene.blocker(eye, point, time, Ray::Sight)
}

/// How much of a body is seen: any of it, its head, and the share of its
/// sample points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Visibility {
    pub any: bool,
    pub head: bool,
    pub fraction: f32,
}

/// The points a body is sampled at.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Body {
    pub head: [f32; 3],
    pub chest: [f32; 3],
    pub pelvis: [f32; 3],
    pub knees: [f32; 3],
    pub feet: [f32; 3],
}

impl Body {
    /// Head first.
    pub fn points(&self) -> [[f32; 3]; 5] {
        [self.head, self.chest, self.pelvis, self.knees, self.feet]
    }
}

/// Where a player is and how, at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// The body's origin: at the feet, and at the eye on a rope.
    pub position: [f32; 3],
    /// Degrees: 0 looks along +y, 90 along -x.
    pub yaw: f32,
    /// Degrees, positive up.
    pub pitch: f32,
    pub stance: Stance,
    pub lean: Lean,
    pub doing: Doing,
    /// On a rope, head down.
    pub inverted: bool,
}

/// The value in force at `time`.
fn value_at<T: Copy>(list: &[Change<T>], time: f64) -> Option<T> {
    let i = list.partition_point(|c| c.time <= time);
    Some(list[i.checked_sub(1)?].value)
}

impl Pose {
    /// A player's pose at `time`: the last sample at or before it. `None`
    /// before the track starts, after it ends and for a dead body.
    pub fn at(track: &PlayerTrack, time: f64) -> Option<Pose> {
        let i = track.time.partition_point(|&t| t <= time).checked_sub(1)?;
        if i + 1 == track.time.len() && time - track.time[i] > STALE {
            return None;
        }
        let doing = value_at(&track.doing, time).unwrap_or(Doing::Nothing);
        if doing == Doing::Dead {
            return None;
        }
        Some(Pose {
            position: [track.x[i], track.y[i], track.z[i]],
            yaw: track.yaw[i],
            pitch: track.pitch[i],
            stance: value_at(&track.stance, time).unwrap_or(Stance::Standing),
            lean: value_at(&track.lean, time).unwrap_or(Lean::None),
            doing,
            inverted: value_at(&track.inverted, time).unwrap_or(false),
        })
    }

    /// Metres from the origin up to the eye.
    pub fn eye_height(&self) -> f32 {
        match (self.doing, self.stance) {
            (Doing::Rappelling, _) => 0.0,
            // Not measured: a downed player is taken to be as low as a
            // prone one.
            (Doing::Downed, _) | (_, Stance::Prone) => EYE_PRONE,
            (_, Stance::Crouched) => EYE_CROUCHED,
            _ => EYE_STANDING,
        }
    }

    /// Unit vectors along the view, level, and to its right.
    fn axes(&self) -> ([f32; 2], [f32; 2]) {
        let (sin, cos) = self.yaw.to_radians().sin_cos();
        ([-sin, cos], [cos, sin])
    }

    fn offset(&self, forward: f32, right: f32, up: f32) -> [f32; 3] {
        let (f, r) = self.axes();
        let [x, y, z] = self.position;
        [
            x + f[0] * forward + r[0] * right,
            y + f[1] * forward + r[1] * right,
            z + up,
        ]
    }

    fn lean_shift(&self) -> f32 {
        match self.lean {
            Lean::None => 0.0,
            Lean::Left => -LEAN_SHIFT,
            Lean::Right => LEAN_SHIFT,
        }
    }

    /// Where the player's eye is.
    pub fn eye(&self) -> [f32; 3] {
        self.offset(0.0, self.lean_shift(), self.eye_height())
    }

    /// The view direction, a unit vector.
    pub fn view(&self) -> [f32; 3] {
        let (f, _) = self.axes();
        let (sin, cos) = self.pitch.to_radians().sin_cos();
        [f[0] * cos, f[1] * cos, sin]
    }

    /// The points the body is sampled at. The head is at the eye; the rest
    /// is assumed: fractions of the eye height when upright, along the
    /// view direction back from the head when prone or down, and hanging
    /// from the head on a rope.
    pub fn body(&self) -> Body {
        let head = self.eye();
        let lean = self.lean_shift();
        if self.doing == Doing::Rappelling {
            let down = if self.inverted { ROPE_BODY } else { -ROPE_BODY };
            let at = |share: f32| self.offset(0.0, 0.0, down * share);
            return Body {
                head,
                chest: at(0.2),
                pelvis: at(0.45),
                knees: at(0.7),
                feet: at(0.95),
            };
        }
        let h = self.eye_height();
        if h <= EYE_PRONE {
            let at = |share: f32, up: f32| self.offset(-PRONE_BODY * share, 0.0, up);
            return Body {
                head,
                chest: at(0.25, 0.2),
                pelvis: at(0.5, 0.15),
                knees: at(0.75, 0.12),
                feet: at(1.0, 0.1),
            };
        }
        Body {
            head,
            chest: self.offset(0.0, lean / 2.0, h * 0.8),
            pelvis: self.offset(0.0, 0.0, h * 0.6),
            knees: self.offset(0.0, 0.0, h * 0.3),
            feet: self.offset(0.0, 0.0, h * 0.07),
        }
    }

    /// Whether the player looks through their own eyes: not while on a
    /// drone or a camera.
    pub fn observes(&self) -> bool {
        self.doing != Doing::OnDrone
    }
}

/// How far a view direction is off a target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Aim {
    /// Degrees the target is to the left of the view direction (counter-
    /// clockwise from above); negative to the right.
    pub yaw: f32,
    /// Degrees the target is above the view direction.
    pub pitch: f32,
    /// Degrees between the view direction and the target.
    pub angle: f32,
    /// Metres the view direction passes above the target, at the target's
    /// distance; negative below.
    pub height: f32,
    /// Metres to the target.
    pub distance: f32,
}

/// How far the view of a player at `eye` is off `target`.
pub fn aim(eye: [f32; 3], yaw: f32, pitch: f32, target: [f32; 3]) -> Aim {
    let d = sub(v3(target), v3(eye));
    let level = d[0].hypot(d[1]);
    let distance = norm(d);
    let bearing = (-d[0]).atan2(d[1]).to_degrees();
    let off = (bearing - f64::from(yaw) + 540.0).rem_euclid(360.0) - 180.0;
    let up = d[2].atan2(level).to_degrees();
    let (ys, yc) = f64::from(yaw).to_radians().sin_cos();
    let (ps, pc) = f64::from(pitch).to_radians().sin_cos();
    let view = [-ys * pc, yc * pc, ps];
    let cos = if distance > 0.0 {
        dot(view, d) / distance
    } else {
        1.0
    };
    let slope = f64::from(pitch.clamp(-89.0, 89.0)).to_radians().tan();
    Aim {
        yaw: off as f32,
        pitch: (up - f64::from(pitch)) as f32,
        angle: cos.clamp(-1.0, 1.0).acos().to_degrees() as f32,
        height: (level * slope - d[2]) as f32,
        distance: distance as f32,
    }
}

/// What the round-level outputs are computed with.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    /// The field of view, in degrees from edge to edge. The file does not
    /// hold a player's setting: the defaults are the game's default of 60
    /// degrees vertical, which is 90 horizontal on a 16:9 screen.
    pub fov_horizontal: f32,
    pub fov_vertical: f32,
    /// Metres past which nobody is taken to see anybody.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_distance: Option<f32>,
    /// Seconds between two tests of a pair.
    pub step: f64,
    /// A sightline lost for no longer than this is one sightline.
    pub gap: f64,
    /// Also pairs of teammates.
    pub teammates: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            fov_horizontal: 90.0,
            fov_vertical: 60.0,
            max_distance: None,
            step: 0.1,
            gap: 0.25,
            teammates: false,
        }
    }
}

impl Options {
    /// Whether a target that far off the view direction is in the field
    /// of view.
    pub fn in_fov(&self, aim: &Aim) -> bool {
        aim.yaw.abs() <= self.fov_horizontal / 2.0 && aim.pitch.abs() <= self.fov_vertical / 2.0
    }
}

/// A stretch in which `from` could see `to`: nothing stood between the eye
/// of the one and the body of the other. Whether they looked that way is
/// `inFov`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sightline {
    pub from: String,
    pub to: String,
    /// Of different teams.
    pub enemies: bool,
    /// Seconds since the recording started.
    pub start: f64,
    pub end: f64,
    pub phase: Phase,
    /// `to` could see `from` at some time of it.
    pub mutual: bool,
    /// Seconds of it in which both could see the other.
    pub mutual_seconds: f64,
    /// Seconds of it in which `to` was in the field of view of `from`,
    /// and each such stretch.
    pub in_fov_seconds: f64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub in_fov: Vec<[f64; 2]>,
    /// Seconds of it in which the head of `to` could be seen.
    pub head_seconds: f64,
    /// The share of the body's sample points seen, on average.
    pub fraction: f32,
    /// Metres from eye to head: at the start, and the least and the most.
    pub distance: f32,
    pub min_distance: f32,
    pub max_distance: f32,
}

/// How exposed a player was in one phase, or over the round.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseExposure {
    pub phase: Phase,
    /// Seconds alive in it.
    pub alive: f64,
    /// Seconds in which at least one enemy could see the player.
    pub exposed: f64,
    /// Seconds in which at least one of those had the player in their
    /// field of view.
    pub in_view: f64,
    /// The most enemies that could see the player at once.
    pub max_enemies: usize,
}

/// How exposed a player was.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exposure {
    pub username: String,
    pub alive: f64,
    pub exposed: f64,
    pub in_view: f64,
    /// Enemies that could see the player at some time.
    pub enemies: usize,
    pub max_enemies: usize,
    /// Enemies that could see the player, on average while any could.
    pub mean_enemies: f64,
    pub phases: Vec<PhaseExposure>,
}

/// How an enemy came into view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Entry {
    /// The line cleared while the player looked that way: the enemy came
    /// round a corner, or the player did. Needs geometry.
    Appeared,
    /// The line was clear and the player turned to it.
    Turned,
    /// In view from the first moment both were there, or as the player
    /// came off a drone or a camera.
    Start,
}

/// The moment an enemy came into a player's view, and where the player's
/// crosshair was then.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirstSight {
    pub username: String,
    pub enemy: String,
    /// Seconds since the recording started.
    pub time: f64,
    pub phase: Phase,
    pub entry: Entry,
    /// How far the view direction was off the enemy's head.
    #[serde(flatten)]
    pub aim: Aim,
}

/// The first bullet of a player to strike an enemy after
/// [`ENGAGEMENT_GAP`] seconds without one: a clear line at that moment,
/// whatever the geometry.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Engagement {
    pub username: String,
    pub enemy: String,
    pub time: f64,
    /// Metres from the eye to where the bullet struck.
    pub distance: f32,
    /// Degrees between the view direction and the point struck, at the
    /// hit.
    pub error_at_hit: f32,
    /// How far the view direction was off the enemy's head
    /// [`ENGAGEMENT_LEAD`] seconds before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Aim>,
}

/// Seconds without a hit on the same enemy that make the next one a new
/// engagement, and how long before it the crosshair is measured.
pub const ENGAGEMENT_GAP: f64 = 5.0;
pub const ENGAGEMENT_LEAD: f64 = 0.5;

/// A player's crosshair placement over the round. The errors are medians
/// of absolute values, off the enemy's head.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Crosshair {
    pub username: String,
    /// What the errors are measured at: `appeared`, the first sights of
    /// enemies who came into the open while the player looked that way,
    /// or `fovEntry` without geometry, where an enemy comes into view only
    /// by the edge of the field of view and the yaw error is near half of
    /// it by construction.
    pub basis: &'static str,
    /// The first sights the errors are taken from.
    pub sights: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yaw_error: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch_error: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angle_error: Option<f32>,
    /// Metres the crosshair was above or below head height.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_height_error: Option<f32>,
    /// The same with its sign, on average: positive is above the head.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_height_bias: Option<f32>,
    /// Engagements, and the median angle off the enemy's head
    /// [`ENGAGEMENT_LEAD`] seconds before the first hit of each. Needs no
    /// geometry.
    pub engagements: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_before_hit: Option<f32>,
}

/// The line-of-sight outputs of a round.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sight {
    /// `none` for an open field, else the geometry's source and map.
    pub geometry: String,
    /// Whether lines were tested against anything. `false`: every pair
    /// alive "could see" each other, and only distance and the field of
    /// view say anything.
    pub occlusion: bool,
    /// What the round did to the geometry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Applied>,
    pub options: Options,
    pub sightlines: Vec<Sightline>,
    pub exposure: Vec<Exposure>,
    pub first_sights: Vec<FirstSight>,
    pub engagements: Vec<Engagement>,
    pub crosshair: Vec<Crosshair>,
}

/// Stretches of consecutive `true` samples, `times` apart by `step`:
/// each from its first sample to `step` past its last, and two no further
/// apart than `gap` are one.
pub fn intervals(times: &[f64], flags: &[bool], step: f64, gap: f64) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::new();
    for (&t, _) in times.iter().zip(flags).filter(|(_, f)| **f) {
        match out.last_mut() {
            Some(last) if t - last[1] <= gap + step * 1e-6 => last[1] = t + step,
            _ => out.push([t, t + step]),
        }
    }
    out
}

fn median(values: &mut [f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f32::total_cmp);
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    })
}

/// One test of a pair.
#[derive(Clone, Copy)]
struct Seen {
    time: f64,
    sight: Visibility,
    fov: bool,
    mutual: bool,
    distance: f32,
}

fn sightlines(
    from: &str,
    to: &str,
    enemies: bool,
    seen: &[Seen],
    options: &Options,
    phase: impl Fn(f64) -> Phase,
) -> Vec<Sightline> {
    let times: Vec<f64> = seen.iter().map(|s| s.time).collect();
    let clear: Vec<bool> = seen.iter().map(|s| s.sight.any).collect();
    let step = options.step;
    (intervals(&times, &clear, step, options.gap).into_iter())
        .map(|[start, end]| {
            let lo = seen.partition_point(|s| s.time < start - step / 2.0);
            let hi = seen.partition_point(|s| s.time < end - step / 2.0);
            let run: Vec<&Seen> = seen[lo..hi].iter().filter(|s| s.sight.any).collect();
            let count =
                |f: &dyn Fn(&Seen) -> bool| run.iter().filter(|s| f(s)).count() as f64 * step;
            let run_times: Vec<f64> = run.iter().map(|s| s.time).collect();
            let fov: Vec<bool> = run.iter().map(|s| s.fov).collect();
            let distances = || run.iter().map(|s| s.distance);
            Sightline {
                from: from.to_owned(),
                to: to.to_owned(),
                enemies,
                start: rounded(start),
                end: rounded(end),
                phase: phase(start),
                mutual: run.iter().any(|s| s.mutual),
                mutual_seconds: rounded(count(&|s| s.mutual)),
                in_fov_seconds: rounded(count(&|s| s.fov)),
                in_fov: (intervals(&run_times, &fov, step, 0.0).into_iter())
                    .map(|i| i.map(rounded))
                    .collect(),
                head_seconds: rounded(count(&|s| s.sight.head)),
                fraction: run.iter().map(|s| s.sight.fraction).sum::<f32>() / run.len() as f32,
                distance: run[0].distance,
                min_distance: distances().fold(f32::MAX, f32::min),
                max_distance: distances().fold(0.0, f32::max),
            }
        })
        .collect()
}

const PHASES: [Phase; 4] = [Phase::Prep, Phase::Action, Phase::Planted, Phase::End];

/// The line-of-sight outputs of a round read with its movement, against a
/// geometry in the state the round put it in, or as an open field without
/// one. `None` when the round has no movement.
pub fn analyze(round: &Round, geometry: Option<&Geometry>, options: &Options) -> Option<Sight> {
    match geometry {
        Some(g) => analyze_scene(round, &Scene::for_round(g, round), options),
        None => analyze_scene(round, &Scene::open_field(), options),
    }
}

/// [`analyze`] against a scene whose state the caller set.
pub fn analyze_scene(round: &Round, scene: &Scene, options: &Options) -> Option<Sight> {
    let movement = round.movement.as_ref()?;
    let tracks: Vec<&PlayerTrack> = movement
        .players
        .iter()
        .filter(|t| !t.time.is_empty())
        .collect();
    let team = |name: &str| {
        let players = &round.header.players;
        players
            .iter()
            .find(|p| p.username == name)
            .map(|p| p.team_index)
    };
    let teams: Vec<Option<usize>> = tracks.iter().map(|t| team(&t.username)).collect();
    let spans = round.timeline.spans();
    let phase = |time: f64| {
        (spans.iter())
            .rfind(|s| s.recording_start.is_some_and(|t| t <= time))
            .map_or(Phase::Prep, |s| s.phase)
    };
    let n = tracks.len();
    let step = options.step;
    let first = tracks.iter().map(|t| t.time[0]).fold(f64::MAX, f64::min);
    let last = tracks
        .iter()
        .map(|t| t.time[t.time.len() - 1])
        .fold(0.0, f64::max);
    let mut seen: Vec<Vec<Seen>> = vec![Vec::new(); n * n];
    let mut sights = Vec::new();
    // Per pair: the last test, and when the one last had the other in view.
    let mut before: Vec<Option<(f64, bool)>> = vec![None; n * n];
    let mut in_view_at: Vec<Option<f64>> = vec![None; n * n];
    // Per player and phase: alive, exposed, in view, most enemies; and
    // over the round the tests exposed and the enemies counted in them.
    let mut exposure = vec![[(0.0f64, 0.0f64, 0.0f64, 0usize); 4]; n];
    let mut counted = vec![(0usize, 0usize); n];
    let mut seen_by = vec![vec![false; n]; n];
    let mut now: Vec<Option<(Visibility, Aim, bool)>> = vec![None; n * n];
    let steps = if n == 0 {
        0
    } else {
        ((last - first) / step).floor() as usize + 1
    };
    for k in 0..steps {
        let time = first + k as f64 * step;
        let poses: Vec<Option<Pose>> = tracks.iter().map(|t| Pose::at(t, time)).collect();
        let bodies: Vec<Option<(Body, [f32; 3])>> = poses
            .iter()
            .map(|p| p.map(|p| (p.body(), p.eye())))
            .collect();
        for i in 0..n {
            for j in 0..n {
                now[i * n + j] = None;
                let enemies = teams[i] != teams[j];
                if i == j || !(enemies || options.teammates) {
                    continue;
                }
                let (Some(a), Some((body, _))) = (poses[i].filter(Pose::observes), bodies[j])
                else {
                    continue;
                };
                let eye = bodies[i].unwrap().1;
                let off = aim(eye, a.yaw, a.pitch, body.head);
                let near = options.max_distance.is_none_or(|m| off.distance <= m);
                let sight = if near {
                    scene.visibility(eye, &body, time)
                } else {
                    Visibility::default()
                };
                now[i * n + j] = Some((sight, off, options.in_fov(&off)));
            }
        }
        let p = PHASES.iter().position(|p| *p == phase(time)).unwrap_or(0);
        for j in 0..n {
            if poses[j].is_none() {
                continue;
            }
            let (mut could, mut looking) = (0, 0);
            for i in (0..n).filter(|&i| teams[i] != teams[j]) {
                if let Some((sight, _, fov)) = now[i * n + j].filter(|s| s.0.any) {
                    could += 1;
                    looking += usize::from(fov);
                    seen_by[j][i] = true;
                    let _ = sight;
                }
            }
            let e = &mut exposure[j][p];
            e.0 += step;
            e.1 += if could > 0 { step } else { 0.0 };
            e.2 += if looking > 0 { step } else { 0.0 };
            e.3 = e.3.max(could);
            if could > 0 {
                counted[j].0 += 1;
                counted[j].1 += could;
            }
        }
        for i in 0..n {
            for j in 0..n {
                let pair = i * n + j;
                let Some((sight, off, fov)) = now[pair] else {
                    before[pair] = None;
                    continue;
                };
                let mutual = sight.any && now[j * n + i].is_some_and(|s| s.0.any);
                seen[pair].push(Seen {
                    time,
                    sight,
                    fov,
                    mutual,
                    distance: off.distance,
                });
                if sight.any && fov && teams[i] != teams[j] {
                    if in_view_at[pair].is_none_or(|t| time - t > options.gap + step * 1.5) {
                        sights.push(FirstSight {
                            username: tracks[i].username.clone(),
                            enemy: tracks[j].username.clone(),
                            time: rounded(time),
                            phase: phase(time),
                            entry: match before[pair] {
                                None => Entry::Start,
                                Some((_, true)) => Entry::Turned,
                                Some((_, false)) => Entry::Appeared,
                            },
                            aim: off,
                        });
                    }
                    in_view_at[pair] = Some(time);
                }
                before[pair] = Some((time, sight.any));
            }
        }
    }
    let mut lines = Vec::new();
    for i in 0..n {
        for j in 0..n {
            let (from, to) = (&tracks[i].username, &tracks[j].username);
            let enemies = teams[i] != teams[j];
            lines.extend(sightlines(
                from,
                to,
                enemies,
                &seen[i * n + j],
                options,
                phase,
            ));
        }
    }
    lines.sort_by(|a, b| a.start.total_cmp(&b.start));
    let engagements = engagements(round, &tracks, &teams);
    let exposure = (0..n)
        .map(|j| {
            let phases: Vec<PhaseExposure> = (PHASES.iter().zip(exposure[j]))
                .filter(|(_, e)| e.0 > 0.0)
                .map(|(&phase, e)| PhaseExposure {
                    phase,
                    alive: rounded(e.0),
                    exposed: rounded(e.1),
                    in_view: rounded(e.2),
                    max_enemies: e.3,
                })
                .collect();
            Exposure {
                username: tracks[j].username.clone(),
                alive: rounded(phases.iter().map(|p| p.alive).sum()),
                exposed: rounded(phases.iter().map(|p| p.exposed).sum()),
                in_view: rounded(phases.iter().map(|p| p.in_view).sum()),
                enemies: seen_by[j].iter().filter(|s| **s).count(),
                max_enemies: phases.iter().map(|p| p.max_enemies).max().unwrap_or(0),
                mean_enemies: match counted[j] {
                    (0, _) => 0.0,
                    (tests, enemies) => rounded(enemies as f64 / tests as f64),
                },
                phases,
            }
        })
        .collect();
    let occlusion = scene.occlusion();
    let crosshair = (tracks.iter())
        .map(|t| {
            let of = |s: &&FirstSight| {
                s.username == t.username && (!occlusion || s.entry == Entry::Appeared)
            };
            let mine: Vec<&FirstSight> = sights.iter().filter(of).collect();
            let column = |f: &dyn Fn(&Aim) -> f32| -> Option<f32> {
                median(&mut mine.iter().map(|s| f(&s.aim).abs()).collect::<Vec<f32>>())
            };
            let fights: Vec<&Engagement> = engagements
                .iter()
                .filter(|e| e.username == t.username)
                .collect();
            let mut led: Vec<f32> = fights
                .iter()
                .filter_map(|e| Some(e.before?.angle))
                .collect();
            Crosshair {
                username: t.username.clone(),
                basis: if occlusion { "appeared" } else { "fovEntry" },
                sights: mine.len(),
                yaw_error: column(&|a| a.yaw),
                pitch_error: column(&|a| a.pitch),
                angle_error: column(&|a| a.angle),
                head_height_error: column(&|a| a.height),
                head_height_bias: (!mine.is_empty())
                    .then(|| mine.iter().map(|s| s.aim.height).sum::<f32>() / mine.len() as f32),
                engagements: fights.len(),
                error_before_hit: median(&mut led),
            }
        })
        .collect();
    let geometry = match scene.geometry() {
        None => "none".to_owned(),
        Some(g) => {
            let source = g.source.as_deref().unwrap_or("authored");
            match &g.map {
                Some(map) => format!("{source}: {map}"),
                None => source.to_owned(),
            }
        }
    };
    Some(Sight {
        geometry,
        occlusion,
        state: scene.applied().cloned(),
        options: *options,
        sightlines: lines,
        exposure,
        first_sights: sights,
        engagements,
        crosshair,
    })
}

/// A bullet hit with its shooter: the time, the indices of the shooter's
/// and the victim's track, and where it struck.
fn bullet_hits(round: &Round, tracks: &[&PlayerTrack]) -> Vec<(f64, usize, usize, [f32; 3])> {
    let index = |name: &str| tracks.iter().position(|t| t.username == name);
    let mut hits: Vec<_> = (round.bullet_hits.iter())
        .filter_map(|h| {
            let shooter = index(h.shooter.as_deref()?)?;
            Some((
                h.when.recording_time?,
                shooter,
                index(&h.victim)?,
                h.position?,
            ))
        })
        .collect();
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    hits
}

fn engagements(round: &Round, tracks: &[&PlayerTrack], teams: &[Option<usize>]) -> Vec<Engagement> {
    let mut last: HashMap<(usize, usize), f64> = HashMap::new();
    let mut out = Vec::new();
    for (time, shooter, victim, point) in bullet_hits(round, tracks) {
        if teams[shooter] == teams[victim] {
            continue;
        }
        let fresh =
            (last.insert((shooter, victim), time)).is_none_or(|t| time - t > ENGAGEMENT_GAP);
        let Some(pose) = Pose::at(tracks[shooter], time).filter(|_| fresh) else {
            continue;
        };
        let at_hit = aim(pose.eye(), pose.yaw, pose.pitch, point);
        let then = time - ENGAGEMENT_LEAD;
        let before = Pose::at(tracks[shooter], then)
            .zip(Pose::at(tracks[victim], then))
            .map(|(a, b)| aim(a.eye(), a.yaw, a.pitch, b.eye()));
        out.push(Engagement {
            username: tracks[shooter].username.clone(),
            enemy: tracks[victim].username.clone(),
            time: rounded(time),
            distance: at_hit.distance,
            error_at_hit: at_hit.angle,
            before,
        });
    }
    out
}

/// A bullet hit a geometry says could not have happened.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Miss {
    pub time: f64,
    pub shooter: String,
    pub victim: String,
    pub blocker: Blocker,
}

/// A geometry checked against what a round proves. Counts add up over
/// rounds; the lists of values are there to take percentiles of.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Validation {
    /// Whether there was anything to test against. `false`: nothing can
    /// be blocked, and the counts below say nothing of any geometry.
    pub occlusion: bool,
    /// Bullet hits on a player with a shooter and both poses.
    pub hits: usize,
    /// Of those, hits whose line from the shooter's eye to the point
    /// struck the geometry stops a bullet on: it is wrong there.
    pub blocked: usize,
    /// Hits whose line it stops sight on: a shot through a soft wall, a
    /// barricade or smoke, or the geometry is wrong there.
    pub sight_blocked: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub misses: Vec<Miss>,
    /// Shots of a gun by a player on their feet, and those whose line
    /// from the eye to where the bullet ended crosses something the
    /// geometry stops a bullet on: it is wrong there. A shot's distance
    /// is to what stopped the bullet, not to the first thing in its way:
    /// of the first shots at a soft wall in the test rounds, 44 of 62 end
    /// behind it.
    pub shots: usize,
    pub shots_blocked: usize,
    /// Shots whose line crosses something that stops sight and no
    /// bullet: through a soft wall, a floor or a barricade.
    pub shots_through: usize,
    /// Per shot, metres between the eye distance the file states and the
    /// one from the derived eye to where the bullet ended. The recording
    /// player's own shots are left out: theirs is measured from 0.20 m
    /// behind the eye.
    #[serde(skip)]
    pub eye_errors: Vec<f32>,
    /// Per hit, degrees between the shooter's view direction and the
    /// point struck.
    #[serde(skip)]
    pub view_errors: Vec<f32>,
}

impl Validation {
    /// The share of bullet hits the geometry contradicts.
    pub fn miss_rate(&self) -> Option<f64> {
        (self.hits > 0).then(|| self.blocked as f64 / self.hits as f64)
    }

    pub fn add(&mut self, other: Validation) {
        self.occlusion |= other.occlusion;
        self.hits += other.hits;
        self.blocked += other.blocked;
        self.sight_blocked += other.sight_blocked;
        self.misses.extend(other.misses);
        self.shots += other.shots;
        self.shots_blocked += other.shots_blocked;
        self.shots_through += other.shots_through;
        self.eye_errors.extend(other.eye_errors);
        self.view_errors.extend(other.view_errors);
    }
}

/// A shot went on behind a wall or a slab when it ended further than this
/// from its plane: a wall is some 0.2 m thick, and has none here.
const SHOT_SHORT: f64 = 0.3;

/// Checks a scene, and the derived eye, against what a round proves: a
/// bullet that struck a player had a line no solid thing crossed, and so
/// had every shot up to where the file says it ended. `None` when the
/// round has no movement.
pub fn validate(scene: &Scene, round: &Round) -> Option<Validation> {
    let movement = round.movement.as_ref()?;
    let tracks: Vec<&PlayerTrack> = movement.players.iter().collect();
    let mut out = Validation {
        occlusion: scene.occlusion(),
        ..Validation::default()
    };
    for (time, shooter, victim, point) in bullet_hits(round, &tracks) {
        let Some(pose) = Pose::at(tracks[shooter], time) else {
            continue;
        };
        let eye = pose.eye();
        out.hits += 1;
        out.view_errors
            .push(aim(eye, pose.yaw, pose.pitch, point).angle);
        out.sight_blocked += usize::from(!scene.visible(eye, point, time));
        if let Some(blocker) = scene.blocker(eye, point, time, Ray::Bullet) {
            out.blocked += 1;
            out.misses.push(Miss {
                time,
                shooter: tracks[shooter].username.clone(),
                victim: tracks[victim].username.clone(),
                blocker,
            });
        }
    }
    // The eye distance of the recording player's own shots is 0.20 m
    // longer than that of the same shot seen by anybody else: measured
    // from behind the eye, in every stance (5,694 shots of 175 rounds).
    let own = round.header.recording_player().map(|p| p.username.as_str());
    for s in &round.shots {
        let (Some(name), Some(time)) = (s.username.as_deref(), s.when.recording_time) else {
            continue;
        };
        if !matches!(s.slot, Some("primary" | "secondary")) {
            continue;
        }
        let pose = (tracks.iter().find(|t| t.username == name)).and_then(|t| Pose::at(t, time));
        let Some(pose) = pose.filter(|p| p.doing == Doing::Nothing) else {
            continue;
        };
        let eye = pose.eye();
        let struck = [0, 1, 2].map(|k| s.origin[k] + s.direction[k] * s.distance);
        let line = sub(v3(struck), v3(eye));
        let length = norm(line) as f32;
        out.shots += 1;
        if own != Some(name) {
            out.eye_errors.push((length - s.eye_distance).abs());
        }
        // A bullet that ended in the wall it crossed, or in the panel on
        // the far side of it, did not go on behind it.
        let behind = |ray| {
            (scene.blocker(eye, struck, time, ray))
                .is_some_and(|b| scene.off_plane(&b, struck) > SHOT_SHORT)
        };
        if behind(Ray::Bullet) {
            out.shots_blocked += 1;
        } else if behind(Ray::Any) {
            out.shots_through += 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall(a: [f32; 2], b: [f32; 2], kind: WallKind) -> Wall {
        Wall {
            a,
            b,
            bottom: 0.0,
            top: 3.0,
            kind,
            ..Wall::default()
        }
    }

    fn square(x: f32, y: f32, half: f32) -> Vec<[f32; 2]> {
        vec![
            [x - half, y - half],
            [x + half, y - half],
            [x + half, y + half],
            [x - half, y + half],
        ]
    }

    /// A wall along y at x = 5, a floor at z = 3 with a hatch and a
    /// stairwell.
    fn room() -> Geometry {
        Geometry {
            walls: vec![wall([5.0, -2.0], [5.0, 2.0], WallKind::Soft)],
            slabs: vec![Slab {
                z: 3.0,
                polygon: square(0.0, 0.0, 20.0),
                openings: vec![
                    Opening {
                        polygon: square(-5.0, 0.0, 1.0),
                        hatch: true,
                        object: None,
                    },
                    Opening {
                        polygon: square(-10.0, 0.0, 1.0),
                        hatch: false,
                        object: None,
                    },
                ],
                ..Slab::default()
            }],
            ..Geometry::default()
        }
    }

    #[test]
    fn a_wall_stops_a_line_through_it_and_no_other() {
        let g = room();
        let s = Scene::new(&g);
        let eye = [0.0, 0.0, 1.5];
        let b = s.blocker(eye, [10.0, 0.0, 1.5], 0.0, Ray::Sight).unwrap();
        assert_eq!((b.element, b.index), (Element::Wall, 0));
        assert!((b.distance - 5.0).abs() < 1e-5 && (b.point[0] - 5.0).abs() < 1e-5);
        // Past its end, over its top, short of it, and along it.
        assert!(s.visible(eye, [10.0, 5.0, 1.5], 0.0));
        assert!(!s.visible([0.0, 0.0, 2.9], [10.0, 0.0, 2.95], 0.0));
        assert!(s.visible([4.0, -5.0, 1.0], [4.0, 5.0, 1.0], 0.0));
        assert!(s.visible(eye, [4.9, 0.0, 1.5], 0.0));
        assert!(s.visible([5.0, -5.0, 1.0], [5.0, 5.0, 1.0], 0.0));
        let high = Geometry {
            walls: vec![Wall {
                top: 1.0,
                ..wall([5.0, -2.0], [5.0, 2.0], WallKind::Solid)
            }],
            ..Geometry::default()
        };
        assert!(Scene::new(&high).visible(eye, [10.0, 0.0, 1.5], 0.0));
        assert!(!Scene::new(&high).visible(eye, [10.0, 0.0, 0.0], 0.0));
        // A point on the wall is not hidden by it.
        assert!(s.visible(eye, [5.0, 0.0, 1.5], 0.0));
    }

    #[test]
    fn visibility_is_the_same_both_ways() {
        let g = room();
        let s = Scene::new(&g);
        let mut n = 0u32;
        let mut next = || {
            n = n.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (n >> 8) as f32 / (1u32 << 24) as f32
        };
        let (mut clear, mut blocked) = (0, 0);
        for _ in 0..2000 {
            let a = [next() * 24.0 - 12.0, next() * 8.0 - 4.0, next() * 6.0];
            let b = [next() * 24.0 - 12.0, next() * 8.0 - 4.0, next() * 6.0];
            let see = s.visible(a, b, 0.0);
            assert_eq!(see, s.visible(b, a, 0.0), "{a:?} {b:?}");
            *(if see { &mut clear } else { &mut blocked }) += 1;
        }
        assert!(clear > 200 && blocked > 200, "{clear} {blocked}");
    }

    #[test]
    fn the_index_finds_what_every_wall_tested_finds() {
        // Walls at all angles over many cells: the grid walk must not
        // skip one a line crosses.
        let mut n = 7u32;
        let mut next = || {
            n = n.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (n >> 8) as f32 / (1u32 << 24) as f32 * 60.0 - 30.0
        };
        let walls: Vec<Wall> = (0..60)
            .map(|_| {
                let a = [next(), next()];
                wall(
                    a,
                    [a[0] + next() / 6.0, a[1] + next() / 6.0],
                    WallKind::Solid,
                )
            })
            .collect();
        let g = Geometry {
            walls,
            ..Geometry::default()
        };
        let s = Scene::new(&g);
        for _ in 0..3000 {
            let (a, b) = ([next(), next(), 1.0], [next(), next(), 2.0]);
            let slow = g.walls.iter().any(|w| {
                let one = Geometry {
                    walls: vec![w.clone()],
                    ..Geometry::default()
                };
                !Scene::new(&one).visible(a, b, 0.0)
            });
            assert_eq!(!s.visible(a, b, 0.0), slow, "{a:?} {b:?}");
        }
    }

    #[test]
    fn a_slab_stops_a_line_but_for_its_openings() {
        let g = room();
        let mut s = Scene::new(&g);
        let up = |x: f32| ([x, 0.0, 1.5], [x, 0.3, 4.5]);
        let (a, b) = up(0.0);
        assert_eq!(
            s.blocker(a, b, 0.0, Ray::Sight).unwrap().element,
            Element::Slab
        );
        // Level lines never cross it, and it ends at its edge.
        assert!(s.visible([0.0, 0.0, 1.5], [3.0, 0.0, 1.5], 0.0));
        assert!(s.visible([25.0, 0.0, 1.5], [25.0, 0.3, 4.5], 0.0));
        // The stairwell is open; the hatch is closed until it goes.
        let (a, b) = up(-10.0);
        assert!(s.visible(a, b, 0.0));
        let (a, b) = up(-5.0);
        assert!(!s.visible(a, b, 0.0));
        // It is wood: a bullet goes through it.
        assert!(s.blocker(a, b, 0.0, Ray::Bullet).is_none());
        s.open_hatch(0, 0, 30.0);
        assert!(!s.visible(a, b, 29.0));
        assert!(s.visible(a, b, 30.0) && s.visible(b, a, 31.0));
        // A soft floor lets bullets through, a reinforced hatch does not.
        let mut soft = room();
        soft.slabs[0].soft = true;
        let mut s = Scene::new(&soft);
        let (c, d) = up(0.0);
        assert!(!s.visible(c, d, 0.0) && s.blocker(c, d, 0.0, Ray::Bullet).is_none());
        assert!(s.blocker(a, b, 0.0, Ray::Bullet).is_none());
        s.reinforce_hatch(0, 0, 10.0, 20.0);
        assert!(s.blocker(a, b, 15.0, Ray::Bullet).is_some());
        assert!(s.blocker(a, b, 25.0, Ray::Bullet).is_none());
    }

    #[test]
    fn what_stops_what() {
        let line = ([0.0, 0.0, 1.5], [10.0, 0.0, 1.5]);
        let stops = |kind, cover: Option<CoverKind>, ray| {
            let g = Geometry {
                walls: vec![wall([5.0, -2.0], [5.0, 2.0], kind)],
                ..Geometry::default()
            };
            let mut s = Scene::new(&g);
            if let Some(kind) = cover {
                s.cover(
                    0,
                    Cover {
                        kind,
                        start: 0.0,
                        end: 10.0,
                    },
                );
            }
            s.blocker(line.0, line.1, 5.0, ray).is_some()
        };
        use CoverKind::*;
        use WallKind::*;
        for (kind, cover, sight, bullet) in [
            (Solid, None, true, true),
            (Soft, None, true, false),
            (Soft, Some(Reinforcement), true, true),
            (Window, None, false, false),
            (Door, Some(Barricade), true, false),
            (Door, Some(ArmorPanel), true, true),
            (SeeThrough, None, false, true),
        ] {
            assert_eq!(stops(kind, cover, Ray::Sight), sight, "{kind:?} {cover:?}");
            assert_eq!(
                stops(kind, cover, Ray::Bullet),
                bullet,
                "{kind:?} {cover:?}"
            );
            assert_eq!(
                stops(kind, cover, Ray::Any),
                sight || bullet,
                "{kind:?} {cover:?}"
            );
        }
    }

    #[test]
    fn state_changes_with_time() {
        let g = Geometry {
            walls: vec![
                wall([5.0, -2.0], [5.0, 2.0], WallKind::Soft),
                wall([8.0, -1.0], [8.0, 1.0], WallKind::Door),
            ],
            ..Geometry::default()
        };
        let mut s = Scene::new(&g);
        let (a, b) = ([0.0, 0.0, 1.5], [6.0, 0.0, 1.5]);
        // Reinforced from 10 to 50, a hole at y -0.5 to 0.5 from 50 on.
        s.cover(
            0,
            Cover {
                kind: CoverKind::Reinforcement,
                start: 10.0,
                end: 50.0,
            },
        );
        s.hole(
            0,
            Hole {
                from: 50.0,
                along: [1.5, 2.5],
                bottom: 1.0,
                top: 2.0,
            },
        );
        assert!(s.blocker(a, b, 5.0, Ray::Bullet).is_none());
        let hard = s.blocker(a, b, 20.0, Ray::Bullet).unwrap();
        assert_eq!(hard.cover, Some(CoverKind::Reinforcement));
        assert!(!s.visible(a, b, 20.0) && s.visible(a, b, 50.0));
        // Beside the hole and over it the wall still stands.
        assert!(!s.visible(a, [6.0, 1.5, 1.5], 60.0));
        assert!(!s.visible(a, [6.0, 0.0, 2.6], 60.0));
        // The door: open, barricaded from 0 to 30, open again.
        let (c, d) = ([7.0, 0.0, 1.5], [9.0, 0.0, 1.5]);
        assert!(s.visible(c, d, 0.0));
        s.cover(
            1,
            Cover {
                kind: CoverKind::Barricade,
                start: f64::NEG_INFINITY,
                end: 30.0,
            },
        );
        assert!(!s.visible(c, d, 0.0) && s.visible(c, d, 30.0));
        assert!(s.blocker(c, d, 0.0, Ray::Bullet).is_none());
        // Smoke stops sight for as long as it lasts, from inside too.
        let mut s = Scene::new(&g);
        s.smoke(Smoke {
            position: [-5.0, 0.0, 1.0],
            radius: 3.0,
            start: 100.0,
            end: 114.0,
        });
        let (e, f) = ([-10.0, 0.0, 1.5], [0.0, 0.0, 1.5]);
        assert!(s.visible(e, f, 99.0) && !s.visible(e, f, 100.0) && s.visible(e, f, 114.0));
        assert!(!s.visible([-5.0, 0.0, 1.0], f, 105.0));
        assert!(s.visible([-10.0, 5.0, 1.5], [0.0, 5.0, 1.5], 105.0));
        assert!(s.blocker(e, f, 105.0, Ray::Bullet).is_none());
        let b = s.blocker(e, f, 105.0, Ray::Sight).unwrap();
        assert!(
            b.element == Element::Smoke && (b.distance - 2.04).abs() < 0.01,
            "{b:?}"
        );
    }

    #[test]
    fn an_open_field_stops_nothing_and_says_so() {
        let s = Scene::open_field();
        assert!(!s.occlusion());
        assert!(visible(&s, [0.0, 0.0, 1.0], [500.0, 3.0, 9.0], 0.0));
        assert!(blocking(&s, [0.0, 0.0, 1.0], [500.0, 3.0, 9.0], 0.0).is_none());
    }

    fn pose(stance: Stance) -> Pose {
        Pose {
            position: [10.0, 20.0, 4.0],
            yaw: 0.0,
            pitch: 0.0,
            stance,
            lean: Lean::None,
            doing: Doing::Nothing,
            inverted: false,
        }
    }

    #[test]
    fn the_eye_is_as_high_as_the_stance() {
        assert_eq!(pose(Stance::Standing).eye(), [10.0, 20.0, 5.44]);
        assert_eq!(pose(Stance::Crouched).eye(), [10.0, 20.0, 4.96]);
        assert_eq!(pose(Stance::Prone).eye(), [10.0, 20.0, 4.39]);
        let downed = Pose {
            doing: Doing::Downed,
            ..pose(Stance::Standing)
        };
        assert_eq!(downed.eye_height(), EYE_PRONE);
        let roped = Pose {
            doing: Doing::Rappelling,
            ..pose(Stance::Standing)
        };
        assert_eq!(roped.eye(), [10.0, 20.0, 4.0]);
        assert!(roped.body().feet[2] < 3.0);
        assert!(
            Pose {
                inverted: true,
                ..roped
            }
            .body()
            .feet[2]
                > 5.0
        );
        // Yaw 0 looks along +y, so right is +x; yaw 90 looks along -x.
        let right = Pose {
            lean: Lean::Right,
            ..pose(Stance::Standing)
        };
        assert!((right.eye()[0] - 10.12).abs() < 1e-5);
        let turned = Pose {
            yaw: 90.0,
            lean: Lean::Left,
            ..pose(Stance::Standing)
        };
        assert!((turned.eye()[1] - 19.88).abs() < 1e-5, "{:?}", turned.eye());
        assert!((turned.view()[0] + 1.0).abs() < 1e-6);
        // A body's points go down from its head; a prone one lies back.
        let body = pose(Stance::Standing).body();
        let z: Vec<f32> = body.points().iter().map(|p| p[2]).collect();
        assert!(z.windows(2).all(|w| w[0] > w[1]) && body.head == pose(Stance::Standing).eye());
        let prone = pose(Stance::Prone).body();
        assert!((prone.feet[1] - 18.4).abs() < 1e-5 && prone.feet[2] < 4.2);
    }

    #[test]
    fn a_body_behind_a_low_wall_shows_its_head() {
        let g = Geometry {
            walls: vec![Wall {
                top: 1.2,
                ..wall([5.0, -2.0], [5.0, 2.0], WallKind::Solid)
            }],
            ..Geometry::default()
        };
        let s = Scene::new(&g);
        let target = Pose {
            position: [6.0, 0.0, 0.0],
            ..pose(Stance::Standing)
        };
        let eye = [0.0, 0.0, 1.44];
        let seen = s.visibility(eye, &target.body(), 0.0);
        assert!(
            seen.any && seen.head && (seen.fraction - 0.4).abs() < 1e-6,
            "{seen:?}"
        );
        let low = Pose {
            stance: Stance::Prone,
            ..target
        };
        assert_eq!(s.visibility(eye, &low.body(), 0.0), Visibility::default());
        let open = Scene::open_field().visibility(eye, &low.body(), 0.0);
        assert_eq!(
            open,
            Visibility {
                any: true,
                head: true,
                fraction: 1.0
            }
        );
    }

    #[test]
    fn the_field_of_view_is_a_window_around_the_view_direction() {
        let o = Options::default();
        let eye = [0.0, 0.0, 1.0];
        // Looking along +y: a target ahead, 30 degrees left, behind.
        let ahead = aim(eye, 0.0, 0.0, [0.0, 10.0, 1.0]);
        assert!(ahead.angle.abs() < 1e-3 && ahead.height.abs() < 1e-5 && o.in_fov(&ahead));
        let left = aim(eye, 0.0, 0.0, [-5.0, 8.660254, 1.0]);
        assert!(
            (left.yaw - 30.0).abs() < 1e-3 && o.in_fov(&left),
            "{left:?}"
        );
        assert!(!o.in_fov(&aim(eye, 0.0, 0.0, [-10.0, 8.0, 1.0])));
        let behind = aim(eye, 0.0, 0.0, [0.0, -10.0, 1.0]);
        assert!((behind.angle - 180.0).abs() < 1e-3 && !o.in_fov(&behind));
        // Yaw wraps: looking at 170, a target at -170 is 20 to the left.
        let wrap = aim(eye, 170.0, 0.0, [1.7364818, -9.848078, 1.0]);
        assert!((wrap.yaw - 20.0).abs() < 1e-3, "{wrap:?}");
        // Above: 45 degrees up is outside 60 vertical, 20 is inside; a
        // level crosshair passes a head 3.64 m up that much below it.
        assert!(!o.in_fov(&aim(eye, 0.0, 0.0, [0.0, 10.0, 11.0])));
        let up = aim(eye, 0.0, 0.0, [0.0, 10.0, 4.6397023]);
        assert!((up.pitch - 20.0).abs() < 1e-3 && o.in_fov(&up));
        assert!((up.height + 3.6397).abs() < 1e-3, "{up:?}");
        assert!(aim(eye, 0.0, 20.0, [0.0, 10.0, 4.6397023]).height.abs() < 1e-3);
    }

    #[test]
    fn intervals_join_samples_no_further_apart_than_the_gap() {
        let times: Vec<f64> = (0..20).map(|k| f64::from(k) * 0.1).collect();
        let mut flags = vec![false; 20];
        for k in [2, 3, 4, 6, 7, 12, 19] {
            flags[k] = true;
        }
        let got = intervals(&times, &flags, 0.1, 0.25);
        let want = [[0.2, 0.8], [1.2, 1.3], [1.9, 2.0]];
        assert_eq!(got.len(), want.len(), "{got:?}");
        for (g, w) in got.iter().zip(want) {
            assert!(
                (g[0] - w[0]).abs() < 1e-9 && (g[1] - w[1]).abs() < 1e-9,
                "{got:?}"
            );
        }
        // No gap allowed: the lost sample splits the first.
        assert_eq!(intervals(&times, &flags, 0.1, 0.0).len(), 4);
        assert!(intervals(&times, &[false; 20], 0.1, 0.25).is_empty());
    }

    #[test]
    fn geometry_reads_from_camel_case_json() {
        let json = r#"{
            "map": "Bank",
            "walls": [
                {"a": [0, 0], "b": [4, 0], "bottom": 0, "top": 3, "kind": "seeThrough"},
                {"id": "w2", "a": [0, 0], "b": [0, 4], "bottom": 0, "top": 3, "object": "60572f5567"}
            ],
            "slabs": [{"z": 4, "polygon": [[0, 0], [4, 0], [4, 4]],
                       "openings": [{"polygon": [[1, 0.2], [2, 0.2], [2, 1]], "hatch": true}]}]
        }"#;
        let g: Geometry = serde_json::from_str(json).unwrap();
        assert_eq!(g.walls[0].kind, WallKind::SeeThrough);
        assert_eq!((g.walls[1].kind, g.walls[1].top), (WallKind::Solid, 3.0));
        assert!(g.slabs[0].openings[0].hatch && !g.slabs[0].soft);
        let again: Geometry = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        assert_eq!(again, g);
    }
}
