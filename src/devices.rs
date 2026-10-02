//! Drones and cameras (Y11S3): the observation devices as entities of the
//! `movement` stream, with who owns them, where they went, how they ended
//! and what was done to them.
//!
//! A device is an entity whose first class is `587f5a72`; with the class
//! `47e5f600` it is a drone, else a camera. Its create message is the
//! `617385fe` of [`crate::loadout`]. The cameras of the map are created by
//! `627385fe` in the stream's snapshot, with 40-bit ids that are the same
//! in every round on a map:
//!
//! ```text
//! 627385fe  u64 id, u32 0, f32 x, y, z, 4 x f32 rotation, u8 1, u64 0,
//!           u32 n, n x class
//! ```
//!
//! What a device is follows from its classes ([`DeviceKind`]). Its updates
//! are framed as [`crate::melee`] describes: a mask byte, the transform
//! group, then one component per class. In the transform group the byte
//! behind `04` is 1 while the device is in the world and 0 while it is not.
//!
//! ```text
//! 587f5a72  u64 own id
//!           u8 has aim; when 1: f32 yaw, pitch, 0, 0
//!           f32 field of view
//!           u8 1 drone, 0 camera; u8; u8 full
//!           full: 9 x u64 asset
//!                 7 x u8 flag: 0 offline, 1 disabled by a source,
//!                   2 destroyed, 3 ?, 4 in flight, 5 a timer follows,
//!                   6 signal lost
//!                 f32 x, y, z, u32 0   where the device is; while flag 1
//!                                      is set, where its source is
//!                 u8 1 once a Pest took it
//!                 with flag 5: u32 milliseconds, u32 0
//!           u64 the entity holding it, else 0
//! 4c60869a  u8 mask; 01: 3 bytes; 02: u64 playerid placing it;
//!           04: u64 the object it sits on
//! 47e5f600  u8 mask; ff: 4 x f32, then 20 bytes with the occupied byte at
//!           7 and the alliance at 16; else 01: 4 x f32; 04, 08, 10: u8;
//!           20: u8 occupied (a player drives it); 80: u32 alliance
//! 7a8ac28f  u8 mask (ff is 1f), in this order: 10: u8; 04: u8; 02: u8
//!           team; 08: u32 alliance; 01: u64 playerid driving it
//! 8490f616  the owner component of crate::throws
//! 513b13b2  u8 mask (ff is 1f); 01: u8; 02: u8 n, n x 8; 04: u16 n, n
//!           bytes; 08: u8 n, n x 12; 10: u8 n, n x 15
//! 56cea924  u8 n, n x { u16, u8, u8 m, m x { u8 type, bytes by type }, u16 }
//! 537901f8  always last; not read
//! ```
//!
//! The transform and the observation component are read exactly. An update
//! of an Evil Eye or of one of Skopos's bodies goes on with a component of
//! a class not read here; what the update says up to there is used. Any
//! other update that does not read to its last byte is a warning.
//!
//! **Owner.** The first player an entity names: the drone a player has in
//! prep names them in `7a8ac28f`, a thrown device in `8490f616`, a placed
//! one in `4c60869a`. The cameras of the map name nobody; they serve the
//! defenders.
//!
//! **Life.** A device is deployed when it goes live: the drones of the
//! prep phase are, in their first state. A throw sets the released flag of
//! `8490f616`; a drone thrown again after a pickup only goes live again.
//! Going out of the world without the destroyed flag is a pickup, and an
//! expiry when the game then removes the entity (an RCE-Ratero). Flag 2 is
//! a destruction; the entity is removed 2 seconds later, and a camera of
//! the map keeps its entity. A drone removed while live, with no flag, is
//! reported destroyed with `noFlag`; a camera, `removed`.
//!
//! **Destroyed by whom.** The file does not say. `by` is inferred from two
//! things written elsewhere: the `MatchScore` of the destroyer rises in
//! the same moment (a teammate's falls instead, `teamKill`), and a shot of
//! `shots` passes within half a metre of the device. `bySource` says which
//! was found: `score+shot` when both name the same player, `score` when a
//! single player's score moved, `shot` when only a shot fits. In the ten
//! test rounds 124 of 127 destructions are `score+shot`, 1 is `score` and
//! 2 name nobody; in 175 real rounds 1219 of 1327 are `score+shot`, 80
//! `score`, 6 `shot` and 22 name nobody. Whenever a shot fitted, it was
//! the scorer's.
//!
//! **Jams, captures, offline.** A jam is flag 1 without a timer: the
//! position of the observation component is then the jammer's, to the
//! centimetre the place of a placed gadget, and `by` is the player who
//! carries that gadget. With a timer it is a countdown: seen on a device
//! out of signal range. A capture is a change of alliance; `kind` is
//! `pest` when the Pest byte is set, else `kludge`. Who captured is
//! inferred, hence `inferred`: the thrower of the object of the new side
//! within 3 m, else the owner of that side's drone within 8 m, else the
//! first player of the new side to drive the device. Flag 0 is an offline
//! span; the file gives no cause for it.
//!
//! **Views.** The view a player table (`aca4c435`) gives a player is the
//! id of the device they look through, which links the `observation`
//! sessions to devices. [`crate::joins`] names who spotted a player from
//! them, and from where each device was and how it was turned.

use std::collections::HashMap;

use memchr::memmem;
use rayon::prelude::*;
use serde::Serialize;

use crate::details::ObservationSession;
use crate::entities::{Hash, PLAYER_TABLE_STREAM, Record, for_each_record, table_entries};
use crate::loadout::{
    DESCRIPTOR, Input, MOVEMENT_STREAM, STATE_STREAM, UPDATE, When, descriptor, messages,
};
use crate::shots::Shot;
use crate::throws::{place, thin};

/// Movement payload type that creates an object of the map.
const MAP_OBJECT: Hash = [0x62, 0x73, 0x85, 0xFE];
/// Movement payload type that removes an entity.
const DELETE: Hash = [0x63, 0x73, 0x85, 0xFE];

/// Class of the observation component: the first of every device.
const OBSERVATION: Hash = [0x58, 0x7F, 0x5A, 0x72];
const PLACED: Hash = [0x4C, 0x60, 0x86, 0x9A];
const DRIVEN: Hash = [0x47, 0xE5, 0xF6, 0x00];
const BLOB: Hash = [0x51, 0x3B, 0x13, 0xB2];
const CONTROL: Hash = [0x7A, 0x8A, 0xC2, 0x8F];
const OWNER: Hash = [0x84, 0x90, 0xF6, 0x16];
const FIRE: Hash = [0x53, 0x79, 0x01, 0xF8];
const ANIMATION: Hash = [0x56, 0xCE, 0xA9, 0x24];
/// Class of a player's body; one of Skopos's is a device too.
const BODY: Hash = [0xD9, 0x6B, 0xD5, 0xF7];
/// Classes only an Evil Eye has.
const EVIL_EYE: [Hash; 2] = [[0xC1, 0xC6, 0xA2, 0x23], [0x76, 0x7E, 0x63, 0x85]];
/// Most classes a create message lists.
const MAX_CLASSES: usize = 16;

/// Body slots that hold what a player carries: `PrimaryGadget`,
/// `SecondaryGadget`, `TertiaryGadget` and `Drone`.
const CARRIED: [Hash; 4] = [
    [0x08, 0x2C, 0xA3, 0x1D],
    [0xD8, 0x55, 0xB4, 0xAF],
    [0x41, 0x20, 0x14, 0x8B],
    [0x60, 0x3E, 0x4A, 0x2F],
];

/// Flags of a full observation component, by index.
const OFFLINE: usize = 0;
const DISABLED: usize = 1;
const DESTROYED: usize = 2;
const TIMED: usize = 5;
const SIGNAL_LOST: usize = 6;

/// Sizes of the animation component's entries, by type.
const ANIMATIONS: [(u8, usize); 6] = [
    (0x00, 40),
    (0x04, 40),
    (0x07, 22),
    (0x0B, 26),
    (0x03, 22),
    (0x08, 44),
];

/// `MatchScore`, on a player's scoreboard object.
const MATCH_SCORE: Hash = [0xEC, 0xDA, 0x4F, 0x80];

/// Entities the game keeps for later wait at (0, 0, -100).
const POOL: f32 = -99.0;
/// A release this many frames after going live is the same deployment, and
/// a pickup this close before the destroyed flag the same death.
const SAME: i64 = 3;
/// A score moves within this long before and after a destruction (seconds).
const SCORE_WINDOW: (f64, f64) = (0.10, 0.20);
/// A shot is fired within this long before and after it.
const SHOT_WINDOW: (f64, f64) = (0.5, 0.2);
/// Metres a shot's ray may pass from the device.
const RAY_REACH: f64 = 0.5;
/// A capture is the work of a thrown object this near (metres), else of a
/// drone this near.
const PEST_REACH: f64 = 3.0;
const KLUDGE_REACH: f64 = 8.0;
/// A session starts within this long of the view it is of (seconds).
const VIEW_WINDOW: f64 = 0.5;

/// What a device is, from the classes of its entity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKind {
    #[default]
    Drone,
    /// Twitch's.
    ShockDrone,
    /// Brava's.
    KludgeDrone,
    /// Echo's.
    Yokai,
    /// Flores's.
    RceRatero,
    /// A camera of the map.
    Default,
    /// Valkyrie's.
    BlackEye,
    /// Zero's.
    Argus,
    Bulletproof,
    /// Maestro's.
    EvilEye,
    /// The body Skopos is not in.
    PantheonShell,
    /// A device with classes no kind here has.
    Unknown,
}

impl DeviceKind {
    fn of(classes: &[Hash], map: bool) -> Self {
        if map {
            return DeviceKind::Default;
        }
        let rest = classes.get(1..).unwrap_or_default();
        if rest == [DRIVEN, CONTROL, OWNER] {
            DeviceKind::Drone
        } else if rest == [DRIVEN, BLOB, CONTROL, OWNER, FIRE] {
            DeviceKind::ShockDrone
        } else if rest == [DRIVEN, ANIMATION, BLOB, CONTROL, OWNER, FIRE] {
            DeviceKind::KludgeDrone
        } else if rest == [DRIVEN, BLOB, CONTROL, OWNER] {
            DeviceKind::Yokai
        } else if rest == [PLACED, DRIVEN, BLOB, CONTROL] {
            DeviceKind::RceRatero
        } else if rest == [BLOB, OWNER] {
            DeviceKind::BlackEye
        } else if rest == [BLOB, OWNER, FIRE] {
            DeviceKind::Argus
        } else if rest == [PLACED, FIRE] {
            DeviceKind::Bulletproof
        } else if rest.contains(&BODY) {
            DeviceKind::PantheonShell
        } else if EVIL_EYE.iter().all(|c| rest.contains(c)) {
            DeviceKind::EvilEye
        } else {
            DeviceKind::Unknown
        }
    }
}

/// How a device ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EndKind {
    #[default]
    Destroyed,
    /// Its owner took it back, and did not throw it again.
    PickedUp,
    /// It went out of the world by itself and the game removed it.
    Expired,
    /// The game removed it while it was in the world; why is not written.
    Removed,
}

/// What named the player who destroyed a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum BySource {
    /// Their score moved and their shot passes the device.
    #[serde(rename = "score+shot")]
    ScoreAndShot,
    /// Theirs is the only score that moved.
    #[serde(rename = "score")]
    Score,
    /// Theirs is the shot that passes the device.
    #[serde(rename = "shot")]
    Shot,
}

/// The end of a device. `when` is the moment it ended.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct End {
    pub kind: EndKind,
    /// `destroyed`: who did it. Inferred, see the module; absent when
    /// nobody can be named.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_source: Option<BySource>,
    /// `by` is of the team the device served: their score fell.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub team_kill: bool,
    /// `destroyed`: the game removed the drone while it was in the world,
    /// without the destroyed flag.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub no_flag: bool,
    #[serde(flatten)]
    pub when: When,
}

/// A stretch of time a player drove a drone. `when` is its start.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveSession {
    /// The driver. Absent when the drone named none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Absent when the drone was still driven as the recording stopped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    #[serde(flatten)]
    pub when: When,
}

/// One drone that was out in the round.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drone {
    /// The drone's entity id, in hex. `observation` sessions and
    /// `deviceEvents` name it.
    pub entity: String,
    pub kind: DeviceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Index of the owner's team.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<usize>,
    /// Each time it went out: thrown, thrown again after a pickup, or out
    /// as the recording started (the drones of the prep phase).
    pub deployments: Vec<When>,
    /// Each time its owner took it back.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pickups: Vec<When>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<DriveSession>,
    /// Where it went, as `[seconds since the recording started, x, y, z]`,
    /// thinned to at most 60 points.
    pub path: Vec<[f64; 4]>,
    /// Where it was at its end, or as the recording stopped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<[f64; 3]>,
    /// Absent when it was out and whole as the recording stopped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<End>,
}

/// One camera: of the map, or a gadget that was placed or thrown.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Camera {
    /// The camera's entity id, in hex.
    pub entity: String,
    pub kind: DeviceKind,
    /// Absent for the cameras of the map.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Index of the team it serves: the owner's, the defenders' for a
    /// camera of the map.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<[f64; 3]>,
    /// When a gadget camera was placed or thrown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placed: Option<When>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<End>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceEventType {
    /// A jammer disabled the device.
    #[default]
    Jam,
    /// The device was disabled with a countdown running: out of signal
    /// range (inferred).
    Countdown,
    /// The countdown ran out.
    SignalLost,
    /// The device changed sides.
    Capture,
    /// The device was offline; the file gives no cause.
    Offline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureKind {
    /// Mozzie's Pest: the device says so.
    Pest,
    /// Brava's Kludge Drone: a capture without a Pest.
    Kludge,
}

/// Something done to a device. `when` is the moment it happened, or the
/// start of a span.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceEvent {
    #[serde(rename = "type")]
    pub kind: DeviceEventType,
    /// The device's entity id, as `drones` and `cameras` give it.
    pub device: String,
    /// `jam`: the entity id of the jammer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jammer: Option<String>,
    /// `capture`: what took the device.
    #[serde(rename = "kind", skip_serializing_if = "Option::is_none")]
    pub capture: Option<CaptureKind>,
    /// `jam`: the player who carries the jammer. `capture`: who took the
    /// device.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// `by` is the nearest candidate, not a name the file gives.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub inferred: bool,
    /// `countdown`: seconds it started at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timer: Option<f64>,
    /// How long a span lasted. Absent when it had not ended as the
    /// recording stopped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    #[serde(flatten)]
    pub when: When,
}

/// Cameras alive on one team.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamCameras {
    /// Cameras of the map.
    pub default: u32,
    /// Cameras its players placed or threw.
    pub gadget: u32,
}

/// The cameras alive from `when` on, per team in the order of `teams`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraCount {
    pub teams: [TeamCameras; 2],
    #[serde(flatten)]
    pub when: When,
}

/// A player looking through a device, from the player tables.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct View {
    pub username: String,
    pub entity: u64,
    /// The entity is one of `drones` or `cameras`.
    pub known: bool,
    /// Seconds since the recording started.
    pub from: f64,
    pub to: Option<f64>,
}

/// Where a device a player looked through was, and how it was turned.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Pose {
    pub entity: u64,
    pub drone: bool,
    /// `(seconds since the recording started, position)`, as it changed.
    pub places: Vec<(f64, [f32; 3])>,
    /// The same for the rotation, a quaternion `[x, y, z, w]`.
    pub turns: Vec<(f64, [f32; 4])>,
}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    pub drones: Vec<Drone>,
    pub cameras: Vec<Camera>,
    pub events: Vec<DeviceEvent>,
    pub camera_counts: Vec<CameraCount>,
    pub views: Vec<View>,
    /// One per device a view names.
    pub poses: Vec<Pose>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
}

impl Decoded {
    /// Names the device of each session: the view its player was given as
    /// the session started, else the one they had then. A session goes on
    /// while the player moves between devices of one kind, so this is the
    /// first of them.
    pub(crate) fn link(&self, sessions: &mut [ObservationSession]) {
        for s in sessions {
            // A session without a time was open as the recording started.
            let t = s.recording_time.unwrap_or(0.0);
            let own = || self.views.iter().filter(|v| v.username == s.username);
            let started = own()
                .filter(|v| (v.from - t).abs() <= VIEW_WINDOW)
                .min_by(|a, b| (a.from - t).abs().total_cmp(&(b.from - t).abs()));
            let view =
                started.or_else(|| own().rfind(|v| v.from <= t && v.to.is_none_or(|end| t < end)));
            s.device = view.filter(|v| v.known).map(|v| hex(v.entity));
        }
    }
}

fn hex(entity: u64) -> String {
    format!("{entity:08x}")
}

fn millis(seconds: f64) -> f64 {
    (seconds * 1000.0).round() / 1000.0
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f64 {
    (a.iter().zip(b))
        .map(|(a, b)| f64::from(b - a).powi(2))
        .sum::<f64>()
        .sqrt()
}

/// A place to the centimetre, as a jammer and the device it jams share it.
fn centimetres(p: [f32; 3]) -> [i64; 3] {
    p.map(|v| (f64::from(v) * 100.0).round() as i64)
}

/// Bytes of a message, read from the front.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.0.split_at_checked(n)?;
        self.0 = tail;
        Some(head)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn floats<const N: usize>(&mut self) -> Option<[f32; N]> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = f32::from_bits(self.u32()?);
        }
        Some(out)
    }
}

/// The full state of an observation component.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Full {
    /// One of the nine assets is set; none is on the map's spare camera.
    assets: bool,
    flags: [u8; 7],
    /// Where the device is, or what disabled it.
    position: [f32; 3],
    pest: u8,
    /// Milliseconds of the countdown.
    timer: Option<u32>,
}

/// An observation component.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Observation {
    own: u64,
    /// Yaw and pitch of a camera that turns, in radians.
    aim: Option<[f32; 2]>,
    fov: f32,
    mobile: bool,
    full: Option<Full>,
    /// The entity holding the device, 0 for none.
    holder: u64,
}

/// Reads the observation component `r` starts with.
fn observation(r: &mut Reader) -> Option<Observation> {
    let own = r.u64()?;
    let aim = match r.u8()? {
        0 => None,
        1 => {
            let [yaw, pitch, _, _] = r.floats()?;
            Some([yaw, pitch])
        }
        _ => return None,
    };
    let [fov] = r.floats()?;
    let mobile = r.u8()? != 0;
    r.u8()?;
    let full = match r.u8()? {
        0 => None,
        1 => {
            let assets = r.take(72)?.iter().any(|&b| b != 0);
            let flags: [u8; 7] = r.take(7)?.try_into().ok()?;
            if flags[TIMED] > 1 {
                return None;
            }
            let position = r.floats()?;
            r.u32()?;
            let pest = r.u8()?;
            let timer = match flags[TIMED] {
                1 => {
                    let ms = r.u32()?;
                    r.u32()?;
                    Some(ms)
                }
                _ => None,
            };
            Some(Full {
                assets,
                flags,
                position,
                pest,
                timer,
            })
        }
        _ => return None,
    };
    Some(Observation {
        own,
        aim,
        fov,
        mobile,
        full,
        holder: r.u64()?,
    })
}

/// What an update says of a device.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Update<'a> {
    position: Option<[f32; 3]>,
    rotation: Option<[f32; 4]>,
    /// 1 in the world, 0 out of it.
    live: Option<u8>,
    observation: Option<Observation>,
    /// The `playerid` placing it.
    placer: Option<u64>,
    occupied: Option<u8>,
    /// The `playerid` driving it.
    driver: Option<u64>,
    /// The `playerid` that threw it.
    thrower: Option<u64>,
    released: Option<u8>,
    /// The alliance the driven, control and owner components give.
    alliances: [Option<u32>; 3],
    /// What was not read.
    rest: &'a [u8],
    /// `rest` starts with a component of a class not read here.
    foreign: bool,
}

/// The transform group: position and the live byte.
fn transform(r: &mut Reader, out: &mut Update) -> Option<()> {
    let sub = r.u8()?;
    if sub & 0xE0 != 0 {
        return None;
    }
    if sub & 0x01 != 0 {
        let p: [f32; 3] = r.floats()?;
        r.u32()?;
        out.position = p.iter().all(|v| v.is_finite()).then_some(p);
    }
    if sub & 0x02 != 0 {
        let q: [f32; 4] = r.floats()?;
        out.rotation = q.iter().all(|v| v.is_finite()).then_some(q);
    }
    if sub & 0x04 != 0 {
        out.live = Some(r.u8()?);
    }
    if sub & 0x08 != 0 {
        r.u16()?;
    }
    if sub & 0x10 != 0 {
        for _ in 0..r.u8()? {
            match r.u32()? {
                0 => r.take(34)?,
                1 => r.take(80)?,
                2 | 3 => &[][..],
                _ => return None,
            };
        }
    }
    Some(())
}

fn placed(r: &mut Reader, out: &mut Update) -> Option<()> {
    let mask = r.u8()?;
    if mask & 0xF8 != 0 {
        return None;
    }
    if mask & 0x01 != 0 {
        r.take(3)?;
    }
    let placer = match mask & 0x02 {
        0 => None,
        _ => Some(r.u64()?),
    };
    if mask & 0x04 != 0 {
        r.u64()?;
    }
    out.placer = placer;
    Some(())
}

fn driven(r: &mut Reader, out: &mut Update) -> Option<()> {
    let mask = r.u8()?;
    let (mut occupied, mut alliance) = (None, None);
    if mask == 0xFF {
        r.take(16)?;
        let state = r.take(20)?;
        occupied = state.get(7).copied();
        alliance = crate::entities::u32_at(state, 16);
    } else {
        if mask & 0x42 != 0 {
            return None;
        }
        if mask & 0x01 != 0 {
            r.take(16)?;
        }
        for bit in [0x04, 0x08, 0x10] {
            if mask & bit != 0 {
                r.u8()?;
            }
        }
        if mask & 0x20 != 0 {
            occupied = Some(r.u8()?);
        }
        if mask & 0x80 != 0 {
            alliance = Some(r.u32()?);
        }
    }
    out.occupied = occupied;
    out.alliances[0] = alliance;
    Some(())
}

fn control(r: &mut Reader, out: &mut Update) -> Option<()> {
    let mask = match r.u8()? {
        0xFF => 0x1F,
        mask => mask,
    };
    if mask & 0xE0 != 0 {
        return None;
    }
    // The fields are not in the order of their bits.
    for bit in [0x10, 0x04, 0x02] {
        if mask & bit != 0 {
            r.u8()?;
        }
    }
    let alliance = match mask & 0x08 {
        0 => None,
        _ => Some(r.u32()?),
    };
    let driver = match mask & 0x01 {
        0 => None,
        _ => Some(r.u64()?),
    };
    out.alliances[1] = alliance;
    out.driver = driver;
    Some(())
}

fn owner(r: &mut Reader, out: &mut Update) -> Option<()> {
    let mask = r.u8()?;
    if mask & 0xF0 != 0 {
        return None;
    }
    if mask & 0x01 != 0 {
        r.u16()?;
    }
    let thrower = match mask & 0x02 {
        0 => None,
        _ => Some(r.u64()?),
    };
    let alliance = match mask & 0x04 {
        0 => None,
        _ => Some(r.u32()?),
    };
    let released = match mask & 0x08 {
        0 => None,
        _ => Some(r.u8()?),
    };
    out.thrower = thrower;
    out.alliances[2] = alliance;
    out.released = released;
    Some(())
}

fn blob(r: &mut Reader) -> Option<()> {
    let mask = match r.u8()? {
        0xFF => 0x1F,
        mask => mask,
    };
    if mask & 0xE0 != 0 {
        return None;
    }
    if mask & 0x01 != 0 {
        r.u8()?;
    }
    if mask & 0x02 != 0 {
        let n = usize::from(r.u8()?);
        r.take(8 * n)?;
    }
    if mask & 0x04 != 0 {
        let n = usize::from(r.u16()?);
        r.take(n)?;
    }
    if mask & 0x08 != 0 {
        let n = usize::from(r.u8()?);
        r.take(12 * n)?;
    }
    if mask & 0x10 != 0 {
        let n = usize::from(r.u8()?);
        r.take(15 * n)?;
    }
    Some(())
}

fn animation(r: &mut Reader) -> Option<()> {
    for _ in 0..r.u8()? {
        r.take(3)?;
        for _ in 0..r.u8()? {
            let kind = r.u8()?;
            r.take(ANIMATIONS.iter().find(|a| a.0 == kind)?.1)?;
        }
        r.take(2)?;
    }
    Some(())
}

/// Parses a `607385fe` payload of a device with these `classes`. `None`
/// when the transform group or the observation component is not what the
/// mask promises.
fn update<'a>(payload: &'a [u8], classes: &[Hash]) -> Option<Update<'a>> {
    if payload.get(..4)? != UPDATE {
        return None;
    }
    let mut r = Reader(payload.get(4..)?);
    let mask = r.u8()?;
    let mut out = Update::default();
    if mask & 0x80 != 0 {
        transform(&mut r, &mut out)?;
    }
    for (i, class) in classes.iter().enumerate() {
        let bit = if i < 7 { 0x40u8 >> i } else { 0 };
        if mask & bit == 0 {
            continue;
        }
        let before = r.0;
        let read = match *class {
            OBSERVATION => observation(&mut r).map(|o| out.observation = Some(o)),
            PLACED => placed(&mut r, &mut out),
            DRIVEN => driven(&mut r, &mut out),
            CONTROL => control(&mut r, &mut out),
            OWNER => owner(&mut r, &mut out),
            BLOB => blob(&mut r),
            ANIMATION => animation(&mut r),
            // Always the last class.
            FIRE => r.take(r.0.len()).map(|_| ()),
            _ => None,
        };
        if read.is_none() {
            if i == 0 {
                return None;
            }
            out.rest = before;
            let known = [
                OBSERVATION,
                PLACED,
                DRIVEN,
                CONTROL,
                OWNER,
                BLOB,
                ANIMATION,
                FIRE,
            ];
            out.foreign = !known.contains(class);
            return Some(out);
        }
    }
    out.rest = r.0;
    Some(out)
}

/// What a create message says of its entity.
#[derive(Clone, Debug, Default, PartialEq)]
struct Made {
    classes: Vec<Hash>,
    /// 0 for an object of the map.
    asset: u64,
    position: [f32; 3],
    rotation: [f32; 4],
    map: bool,
}

/// Parses a `617385fe` or `627385fe` payload.
fn made(payload: &[u8]) -> Option<Made> {
    let map = match payload.get(..4)? {
        kind if kind == DESCRIPTOR => false,
        kind if kind == MAP_OBJECT => true,
        _ => return None,
    };
    let position = Reader(payload.get(16..)?).floats()?;
    let rotation = Reader(payload.get(28..)?).floats()?;
    let mut r = Reader(payload.get(53..)?);
    let count = r.u32()? as usize;
    if count > MAX_CLASSES {
        return None;
    }
    let classes = r.take(4 * count)?.as_chunks::<4>().0.to_vec();
    let asset = if map { 0 } else { r.u64()? };
    Some(Made {
        classes,
        asset,
        position,
        rotation,
        map,
    })
}

/// Which component named a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Named {
    Control,
    Owner,
    Placed,
    /// Found in the bytes of an update that was not read to its end.
    Scan,
}

/// A change of a device's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum What {
    Live(u8),
    Offline(u8),
    Disabled(u8),
    Destroyed(u8),
    SignalLost(u8),
    Pest(u8),
    /// Milliseconds of a countdown.
    Timer(u32),
    /// Where the source that disabled the device is.
    Source([i64; 3]),
    Player(Named, u64),
    Occupied(u8),
    Released(u8),
    Alliance(u32),
    Deleted,
}

/// The last value of everything [`What`] tracks.
#[derive(Clone, Copy, Debug, Default)]
struct State {
    live: Option<u8>,
    flags: [Option<u8>; 7],
    pest: Option<u8>,
    timer: Option<u32>,
    source: Option<[i64; 3]>,
    players: [Option<u64>; 4],
    occupied: Option<u8>,
    released: Option<u8>,
    alliances: [Option<u32>; 3],
}

/// Sets `slot` to `value`; whether that changed it.
fn changed<T: PartialEq>(slot: &mut Option<T>, value: T) -> bool {
    if slot.as_ref() == Some(&value) {
        return false;
    }
    *slot = Some(value);
    true
}

/// The frame of a message as a number that sorts: the snapshot is -1.
type Frame = i64;

/// A device while its messages are read.
#[derive(Clone, Debug, Default)]
struct Tracked {
    entity: u64,
    classes: Vec<Hash>,
    asset: u64,
    map: bool,
    /// Where and when it was created.
    created: [f32; 3],
    made_at: Frame,
    events: Vec<(Frame, What)>,
    /// Each new position.
    track: Vec<(Frame, [f32; 3])>,
    /// Each new rotation, the one it was created with first.
    turns: Vec<(Frame, [f32; 4])>,
    state: State,
    /// Whether its first full state names an asset.
    assets: Option<bool>,
    deleted: Option<Frame>,
}

impl Tracked {
    fn has(&self, class: Hash) -> bool {
        self.classes.contains(&class)
    }

    fn is_drone(&self) -> bool {
        self.has(DRIVEN)
    }

    /// Notes what `u` changes. `players` are the `playerid`s of the round.
    fn apply(&mut self, frame: Frame, u: &Update, players: &HashMap<u64, usize>) {
        if let Some(p) = u.position
            && self.track.last().is_none_or(|l| l.1 != p)
        {
            self.track.push((frame, p));
        }
        if let Some(q) = u.rotation
            && self.turns.last().is_none_or(|l| l.1 != q)
        {
            self.turns.push((frame, q));
        }
        let (s, events) = (&mut self.state, &mut self.events);
        let mut note = |is: bool, what: What| {
            if is {
                events.push((frame, what));
            }
        };
        if let Some(v) = u.live {
            note(changed(&mut s.live, v), What::Live(v));
        }
        if let Some(full) = u.observation.and_then(|o| o.full) {
            let was = s.flags[DISABLED].unwrap_or(0) != 0;
            for (i, &v) in full.flags.iter().enumerate() {
                let is = changed(&mut s.flags[i], v);
                match i {
                    OFFLINE => note(is, What::Offline(v)),
                    DISABLED => note(is, What::Disabled(v)),
                    DESTROYED => note(is, What::Destroyed(v)),
                    SIGNAL_LOST => note(is, What::SignalLost(v)),
                    _ => {}
                }
            }
            let disabled = full.flags[DISABLED] != 0;
            if disabled && !was {
                (s.source, s.timer) = (None, None);
            }
            note(changed(&mut s.pest, full.pest), What::Pest(full.pest));
            if let Some(ms) = full.timer {
                note(changed(&mut s.timer, ms), What::Timer(ms));
            }
            if disabled {
                let at = centimetres(full.position);
                note(changed(&mut s.source, at), What::Source(at));
            }
            self.assets.get_or_insert(full.assets);
        }
        if let Some(id) = u.placer {
            let is = changed(&mut s.players[2], id);
            note(is, What::Player(Named::Placed, id));
        }
        if let Some(v) = u.occupied {
            note(changed(&mut s.occupied, v), What::Occupied(v));
        }
        if let Some(a) = u.alliances[0] {
            note(changed(&mut s.alliances[0], a), What::Alliance(a));
        }
        if let Some(id) = u.driver {
            let is = changed(&mut s.players[0], id);
            note(is, What::Player(Named::Control, id));
        }
        if let Some(a) = u.alliances[1] {
            note(changed(&mut s.alliances[1], a), What::Alliance(a));
        }
        if let Some(id) = u.thrower {
            let is = changed(&mut s.players[1], id);
            note(is, What::Player(Named::Owner, id));
        }
        if let Some(a) = u.alliances[2] {
            note(changed(&mut s.alliances[2], a), What::Alliance(a));
        }
        if let Some(v) = u.released {
            note(changed(&mut s.released, v), What::Released(v));
        }
        // What was not read may still name a player.
        let named = (u.rest.windows(8))
            .filter_map(|w| w.try_into().ok().map(u64::from_le_bytes))
            .find(|id| players.contains_key(id));
        if let Some(id) = named {
            let is = changed(&mut s.players[3], id);
            note(is, What::Player(Named::Scan, id));
        }
    }

    /// The last position at or before `frame`.
    fn position_at(&self, frame: Frame) -> Option<[f32; 3]> {
        position_at(&self.track, frame)
    }
}

fn position_at(track: &[(Frame, [f32; 3])], frame: Frame) -> Option<[f32; 3]> {
    let i = track.partition_point(|p| p.0 <= frame).checked_sub(1)?;
    track.get(i).map(|p| p.1)
}

/// When a device went out, came back and ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Life {
    deploys: Vec<Frame>,
    pickups: Vec<Frame>,
    end: Option<(EndKind, Frame)>,
    /// Destroyed by the game removing it, with no flag.
    no_flag: bool,
}

fn life(t: &Tracked) -> Life {
    let mut out = Life::default();
    let body = t.has(BODY);
    let mut destroyed = None;
    let (mut live, mut released, mut first) = (false, false, true);
    for &(frame, what) in &t.events {
        match what {
            What::Live(v) => {
                let now = v != 0;
                // Live in its first state: out since before the recording.
                if now && !live && (t.map || first || released || t.has(PLACED) || body) {
                    out.deploys.push(frame);
                } else if live && !now && destroyed.is_none() {
                    out.pickups.push(frame);
                }
                (live, first) = (now, false);
            }
            What::Released(v) => {
                let now = v != 0;
                let fresh = out.deploys.last().is_none_or(|&d| d < frame - SAME);
                if now && !released && live && fresh {
                    out.deploys.push(frame);
                }
                released = now;
            }
            What::Destroyed(v) if v != 0 && destroyed.is_none() => destroyed = Some(frame),
            _ => {}
        }
    }
    let last = out.deploys.last().copied();
    if let Some(at) = destroyed {
        // Going out of the world right before the flag is the same death.
        out.pickups.retain(|&f| f < at - SAME);
        out.end = Some((EndKind::Destroyed, at));
    } else if let (Some(deleted), Some(last)) = (t.deleted, last) {
        let dropped = out.pickups.iter().copied().filter(|&f| f > last).max();
        match dropped {
            Some(at) => {
                out.pickups.retain(|&f| f != at);
                out.end = Some((EndKind::Expired, at));
            }
            None if t.is_drone() || body => {
                out.end = Some((EndKind::Destroyed, deleted));
                out.no_flag = true;
            }
            None => out.end = Some((EndKind::Removed, deleted)),
        }
    } else if let (Some(&at), Some(last)) = (out.pickups.last(), last)
        && at > last
    {
        out.end = Some((EndKind::PickedUp, at));
    }
    out
}

/// A change of a player's `MatchScore`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Score {
    /// Seconds since the recording started.
    time: f64,
    /// Index of the player.
    player: usize,
    rose: bool,
}

/// Every change of a player's `MatchScore` after the snapshot.
fn scores(input: &Input) -> Vec<Score> {
    let boards: HashMap<u32, usize> = (input.players.iter().enumerate())
        .filter_map(|(i, p)| Some((p.entities.as_ref()?.scoreboard?, i)))
        .collect();
    let finder = memmem::Finder::new(&MATCH_SCORE);
    let mut current: HashMap<u32, u32> = HashMap::new();
    let mut out = Vec::new();
    for (start, end, frame) in input.blocks(STATE_STREAM) {
        let Some(block) = input.data.get(start..end) else {
            continue;
        };
        if finder.find(block).is_none() {
            continue;
        }
        let time = input.clock.seconds(frame);
        let mut object = None;
        for_each_record(block, |_, record| {
            let (hash, from, to) = match record {
                Record::Set(obj, hash, from, to) => {
                    object = Some(obj);
                    (hash, from, to)
                }
                Record::Prop(hash, from, to) => (hash, from, to),
                Record::ParentChild(parent, ..) => {
                    object = Some(parent);
                    return;
                }
                _ => return,
            };
            if hash != MATCH_SCORE {
                return;
            }
            let value = (block.get(from..to))
                .and_then(|v| v.try_into().ok())
                .map(u32::from_le_bytes);
            let (Some(obj), Some(value)) = (object, value) else {
                return;
            };
            let Some(&player) = boards.get(&obj) else {
                return;
            };
            let old = current.insert(obj, value).unwrap_or(0);
            if let Some(time) = time
                && value != old
            {
                out.push(Score {
                    time,
                    player,
                    rose: value > old,
                });
            }
        });
    }
    out
}

/// What can name the player who destroyed a device.
struct Evidence<'a> {
    scores: Vec<Score>,
    shots: &'a [Shot],
    /// Username -> index of the player.
    index: HashMap<&'a str, usize>,
    /// Team of each player.
    teams: Vec<usize>,
}

impl Evidence<'_> {
    /// Who destroyed the device of `team` at `position`, `time` seconds
    /// into the recording: the player, how they were found, and whether
    /// they are of that team.
    fn destroyer(
        &self,
        time: f64,
        position: Option<[f32; 3]>,
        team: Option<usize>,
    ) -> Option<(usize, BySource, bool)> {
        // A rise for an opponent of the device's team, a fall for its own.
        let scorers: Vec<usize> = (self.scores.iter())
            .filter(|s| time - SCORE_WINDOW.0 <= s.time && s.time <= time + SCORE_WINDOW.1)
            .filter(|s| team.is_none_or(|t| s.rose == (self.teams.get(s.player) != Some(&t))))
            .map(|s| s.player)
            .collect();
        let mut rays: Vec<(f64, usize)> = Vec::new();
        for shot in self.shots {
            let (Some(at), Some(p)) = (shot.when.recording_time, position) else {
                continue;
            };
            let Some(&player) = (shot.username.as_deref()).and_then(|u| self.index.get(u)) else {
                continue;
            };
            if at - time < -SHOT_WINDOW.0 || at - time > SHOT_WINDOW.1 {
                continue;
            }
            let to = [0, 1, 2].map(|i| f64::from(p[i]) - f64::from(shot.origin[i]));
            let along: f64 = (0..3).map(|i| to[i] * f64::from(shot.direction[i])).sum();
            let off = (to.iter().map(|v| v * v).sum::<f64>() - along * along)
                .max(0.0)
                .sqrt();
            if along > 0.0 && off < RAY_REACH {
                rays.push((off, player));
            }
        }
        rays.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut shooters: Vec<usize> = Vec::new();
        for (_, player) in rays {
            if !shooters.contains(&player) {
                shooters.push(player);
            }
        }
        let both = scorers.iter().find(|p| shooters.contains(p));
        let (player, source) = match (both, scorers.as_slice(), shooters.as_slice()) {
            (Some(&p), _, _) => (p, BySource::ScoreAndShot),
            (None, &[p], _) => (p, BySource::Score),
            (None, _, &[p]) | (None, [], &[p, ..]) => (p, BySource::Shot),
            _ => return None,
        };
        let own = team.is_some() && self.teams.get(player).copied() == team;
        Some((player, source, own))
    }
}

/// A movement block: `(start, end, frame)`.
type Block = (usize, usize, Option<u32>);
/// A message: `(entity, start, end, frame)`, the offsets being those of
/// its payload in the data.
type Message = (u64, usize, usize, Frame);

/// What the first pass finds in one block.
#[derive(Debug, Default)]
struct Scan {
    devices: Vec<u64>,
    /// Placed entities that are no device: a jammer is one.
    gadgets: Vec<u64>,
    /// `(asset, index of the player)` of each thing a body carries.
    carried: Vec<(u64, usize)>,
}

/// Whether `payload` is an update that gives a place in the world, and it.
fn world_position(payload: &[u8]) -> Option<[f32; 3]> {
    let moved = payload.starts_with(&UPDATE)
        && payload.get(4).is_some_and(|m| m & 0x80 != 0)
        && payload.get(5).is_some_and(|s| s & 0x01 != 0);
    let p: [f32; 3] = Reader(payload.get(6..).filter(|_| moved)?).floats()?;
    p.iter().all(|v| v.is_finite()).then_some(p)
}

/// The messages of `ids` (sorted) that `keep` wants, in stream order.
fn collect(
    data: &[u8],
    blocks: &[Block],
    ids: &[u64],
    keep: impl Fn(u64, &[u8]) -> bool + Sync,
) -> Vec<Message> {
    let found: Vec<Vec<Message>> = blocks
        .par_iter()
        .map(|b| {
            messages(data.get(b.0..b.1).unwrap_or_default())
                .filter(|m| ids.binary_search(&m.0).is_ok() && keep(m.0, m.2))
                .map(|(entity, at, payload)| {
                    let frame = b.2.map_or(-1, Frame::from);
                    (entity, b.0 + at, b.0 + at + payload.len(), frame)
                })
                .collect()
        })
        .collect();
    found.into_iter().flatten().collect()
}

/// A thrown object that is no device: a Pest is one.
#[derive(Clone, Debug, Default)]
struct Thrown {
    /// Index of the player who threw it.
    player: Option<usize>,
    track: Vec<(Frame, [f32; 3])>,
}

/// Every thrown object as the stream leaves it, with whose it is: the
/// player one of its updates ends with.
fn thrown(data: &[u8], blocks: &[Block], players: &HashMap<u64, usize>) -> Vec<Thrown> {
    let is_thrown =
        |m: &Made| m.classes.contains(&OWNER) && m.classes.first() != Some(&OBSERVATION);
    let block = |b: &Block| data.get(b.0..b.1).unwrap_or_default();
    let mut ids: Vec<u64> = blocks
        .par_iter()
        .flat_map_iter(|b| {
            messages(block(b))
                .filter(|m| made(m.2).is_some_and(|m| is_thrown(&m)))
                .map(|m| m.0)
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let mut out: HashMap<u64, Thrown> = HashMap::new();
    for (entity, from, to, frame) in collect(data, blocks, &ids, |_, _| true) {
        let Some(payload) = data.get(from..to) else {
            continue;
        };
        if !payload.starts_with(&UPDATE) {
            match made(payload) {
                Some(m) if is_thrown(&m) => {
                    out.insert(entity, Thrown::default());
                }
                Some(_) => {
                    out.remove(&entity);
                }
                None => {}
            }
            continue;
        }
        let Some(t) = out.get_mut(&entity) else {
            continue;
        };
        if let Some(p) = world_position(payload).filter(|p| p[2] > POOL) {
            t.track.push((frame, p));
        }
        if t.player.is_none() && payload.len() >= 20 {
            let tail = payload
                .get((payload.len() - 16).max(5)..)
                .unwrap_or_default();
            t.player = (tail.windows(8))
                .filter_map(|w| w.try_into().ok().map(u64::from_le_bytes))
                .find_map(|id| players.get(&id).copied());
        }
    }
    out.into_values().collect()
}

/// A device that was out, on its way to the output.
struct Out<'a> {
    tracked: &'a Tracked,
    kind: DeviceKind,
    /// Index of the owner.
    owner: Option<usize>,
    team: Option<usize>,
    life: Life,
}

/// A capture before who did it is known.
struct Capture {
    /// Index into the devices that were out.
    device: usize,
    /// Index into the events.
    event: usize,
    frame: Frame,
    /// The team the device went to.
    team: usize,
    /// The first player other than the owner to drive it afterwards.
    driver: Option<usize>,
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// Reads the drones and cameras of the round. `shots` are the round's
/// shots and `defense` the index of the defending team.
pub(crate) fn decode(input: &Input, shots: &[Shot], defense: Option<usize>) -> Decoded {
    let data = input.data;
    let clock = input.clock;
    let blocks: Vec<Block> = input.blocks(MOVEMENT_STREAM).collect();
    let block = |b: &Block| data.get(b.0..b.1).unwrap_or_default();
    let players: HashMap<u64, usize> = (input.players.iter().enumerate())
        .filter(|(_, p)| p.id != 0)
        .map(|(i, p)| (p.id, i))
        .collect();
    let bodies: HashMap<u64, usize> = (input.players.iter().enumerate())
        .filter_map(|(i, p)| Some((u64::from(p.entities.as_ref()?.movement?), i)))
        .collect();
    let name = |player: usize| input.players.get(player).map(|p| p.username.clone());
    let team_of = |player: usize| input.players.get(player).map(|p| p.team_index);

    // The stream is most of a replay, so its blocks are read in parallel:
    // once for the create messages, and once more for the messages of the
    // devices and of the gadgets that can jam them.
    let scans: Vec<Scan> = blocks
        .par_iter()
        .map(|b| {
            let mut scan = Scan::default();
            for (entity, _, payload) in messages(block(b)) {
                let Some(m) = made(payload) else {
                    continue;
                };
                if m.classes.first() == Some(&OBSERVATION) {
                    scan.devices.push(entity);
                } else if m.classes.contains(&PLACED) {
                    scan.gadgets.push(entity);
                }
                if let Some(&player) = bodies.get(&entity)
                    && let Some(d) = descriptor(payload)
                {
                    let held = d
                        .slots
                        .iter()
                        .filter(|s| CARRIED.contains(&s.0) && s.1 != 0);
                    scan.carried.extend(held.map(|s| (s.1, player)));
                }
            }
            scan
        })
        .collect();
    let mut out = Decoded::default();
    let mut devices: Vec<u64> = scans.iter().flat_map(|s| &s.devices).copied().collect();
    devices.sort_unstable();
    devices.dedup();
    if devices.is_empty() {
        return out;
    }
    let mut wanted = devices.clone();
    wanted.extend(scans.iter().flat_map(|s| &s.gadgets));
    wanted.sort_unstable();
    wanted.dedup();
    // Asset -> the players who carry it.
    let mut carried: HashMap<u64, Vec<usize>> = HashMap::new();
    for &(asset, player) in scans.iter().flat_map(|s| &s.carried) {
        let who = carried.entry(asset).or_default();
        if !who.contains(&player) {
            who.push(player);
        }
    }
    let carrier = |asset: u64| match carried.get(&asset).map(Vec::as_slice) {
        Some(&[player]) => Some(player),
        _ => None,
    };
    // Of a gadget only where it is matters.
    let found = collect(data, &blocks, &wanted, |entity, payload| {
        devices.binary_search(&entity).is_ok()
            || !payload.starts_with(&UPDATE)
            || world_position(payload).is_some()
    });

    let mut tracked: Vec<Tracked> = Vec::new();
    let mut current: HashMap<u64, usize> = HashMap::new();
    // Gadget -> its asset, and where each gadget was last: `(entity, asset)`.
    let mut gadgets: HashMap<u64, u64> = HashMap::new();
    let mut gadget_at: HashMap<[i64; 3], (u64, u64)> = HashMap::new();
    let (mut unread, mut partial) = (0usize, 0usize);
    for (entity, from, to, frame) in found {
        let Some(payload) = data.get(from..to) else {
            continue;
        };
        if payload.starts_with(&DESCRIPTOR) || payload.starts_with(&MAP_OBJECT) {
            current.remove(&entity);
            let Some(m) = made(payload) else {
                continue;
            };
            gadgets.remove(&entity);
            if m.classes.first() == Some(&OBSERVATION) {
                current.insert(entity, tracked.len());
                tracked.push(Tracked {
                    entity,
                    classes: m.classes,
                    asset: m.asset,
                    map: m.map,
                    created: m.position,
                    made_at: frame,
                    turns: vec![(frame, m.rotation)],
                    ..Tracked::default()
                });
            } else if m.classes.contains(&PLACED) {
                gadgets.insert(entity, m.asset);
            }
            continue;
        }
        if let Some(&asset) = gadgets.get(&entity)
            && let Some(p) = world_position(payload).filter(|p| p[2] > POOL)
        {
            gadget_at.insert(centimetres(p), (entity, asset));
        }
        let Some(t) = current.get(&entity).and_then(|&i| tracked.get_mut(i)) else {
            continue;
        };
        if payload.starts_with(&DELETE) {
            t.events.push((frame, What::Deleted));
            t.deleted = Some(frame);
            current.remove(&entity);
        } else if payload.starts_with(&UPDATE) {
            match update(payload, &t.classes) {
                Some(u) => {
                    partial += usize::from(!u.rest.is_empty() && !u.foreign);
                    t.apply(frame, &u, &players);
                }
                None => unread += 1,
            }
        }
    }
    if unread > 0 {
        out.warnings.push(format!(
            "{} not understood",
            plural(unread, "device update was", "device updates were")
        ));
    }
    if partial > 0 {
        out.warnings.push(format!(
            "{} not read to the last byte",
            plural(partial, "device update was", "device updates were")
        ));
    }

    // The clock is read from the state stream: an event of the movement
    // stream is placed by the end of the state record of its frame, or of
    // the last one before.
    let state: Vec<(u32, usize)> = (input.blocks(STATE_STREAM))
        .filter_map(|(_, end, frame)| Some((frame?, end)))
        .collect();
    let when = |frame: Frame| {
        let frame = u32::try_from(frame).ok();
        let i = state.partition_point(|s| Some(s.0) <= frame);
        let at = i.checked_sub(1).and_then(|i| state.get(i));
        clock.when(at.map_or(0, |s| s.1), frame)
    };
    // The snapshot is the state as the recording starts.
    let seconds = |frame: Frame| match u32::try_from(frame) {
        Ok(frame) => clock.seconds(Some(frame)),
        Err(_) => Some(0.0),
    };
    let span = |from: Frame, to: Frame| Some(millis(seconds(to)? - seconds(from)?));
    let alliances: HashMap<u32, usize> = (input.players.iter())
        .filter_map(|p| Some((u32::try_from(p.alliance).ok()?, p.team_index)))
        .collect();
    let evidence = Evidence {
        scores: scores(input),
        shots,
        index: (input.players.iter().enumerate())
            .map(|(i, p)| (p.username.as_str(), i))
            .collect(),
        teams: input.players.iter().map(|p| p.team_index).collect(),
    };

    // The devices that were out, in the order they were created.
    let mut outs: Vec<Out> = Vec::new();
    for t in &tracked {
        let life = life(t);
        // Never deployed, or the spare camera of the map.
        if (life.deploys.is_empty() && !t.map) || (t.map && t.assets == Some(false)) {
            continue;
        }
        let named = t.events.iter().find_map(|e| match e.1 {
            What::Player(_, id) => players.get(&id).copied(),
            _ => None,
        });
        let owner = named.or_else(|| carrier(t.asset));
        outs.push(Out {
            tracked: t,
            kind: DeviceKind::of(&t.classes, t.map),
            owner,
            team: owner.and_then(team_of).or(defense.filter(|_| t.map)),
            life,
        });
    }

    let mut captures: Vec<Capture> = Vec::new();
    // `(frame, event)`, to be put in order.
    let mut events: Vec<(Frame, DeviceEvent)> = Vec::new();
    // `(frame, team, a camera of the map, one more or less)`.
    let mut changes: Vec<(Frame, usize, bool, bool)> = Vec::new();
    for (index, o) in outs.iter().enumerate() {
        let t = o.tracked;
        let mobile = t.is_drone() || t.has(BODY);
        let first = o.life.deploys.first().copied();
        // Where it was while it was out, up to its end.
        let until = o.life.end.map_or(Frame::MAX, |e| e.1);
        let track: Vec<(Frame, [f32; 3])> = (t.track.iter())
            .filter(|p| p.1[2] > POOL && (!mobile || first.is_none_or(|f| p.0 >= f)))
            .copied()
            .collect();
        let last = (track.partition_point(|p| p.0 <= until).checked_sub(1))
            .map_or(track.first(), |i| track.get(i))
            .map(|p| p.1);
        // The team it serves at its end: a capture changes it.
        let mut side = o.team;
        for &(frame, what) in &t.events {
            if let What::Alliance(a) = what
                && frame <= until
                && let Some(&team) = alliances.get(&a)
            {
                side = Some(team);
            }
        }
        let end = o.life.end.map(|(kind, frame)| {
            let by = (kind == EndKind::Destroyed)
                .then(|| evidence.destroyer(millis(seconds(frame)?), last, side))
                .flatten();
            End {
                kind,
                by: by.and_then(|b| name(b.0)),
                by_source: by.map(|b| b.1),
                team_kill: by.is_some_and(|b| b.2),
                no_flag: o.life.no_flag,
                when: when(frame),
            }
        });
        let entity = hex(t.entity);
        let owner = o.owner.and_then(name);

        // Disabled and offline spans, and captures.
        let mut open: [Option<usize>; 2] = [None, None];
        // The place of the source is read once, as a jam starts.
        let mut sourced = false;
        let (mut alliance, mut capture) = (None, None);
        let pest = (t.events.iter()).any(|e| matches!(e.1, What::Pest(v) if v != 0));
        for &(frame, what) in &t.events {
            let event = |kind: DeviceEventType| {
                let event = DeviceEvent {
                    kind,
                    device: entity.clone(),
                    when: when(frame),
                    ..DeviceEvent::default()
                };
                (frame, event)
            };
            let start = |i: usize| open.get(i).copied().flatten();
            match what {
                What::Disabled(v) | What::Offline(v) => {
                    let (i, kind) = match what {
                        What::Disabled(_) => (0, DeviceEventType::Jam),
                        _ => (1, DeviceEventType::Offline),
                    };
                    if v != 0 && start(i).is_none() {
                        sourced = sourced && i != 0;
                        open[i] = Some(events.len());
                        events.push(event(kind));
                    } else if v == 0
                        && let Some(e) = start(i).and_then(|e| events.get_mut(e))
                    {
                        e.1.seconds = span(e.0, frame);
                        open[i] = None;
                    }
                }
                What::Source(at) => {
                    let jam = start(0).and_then(|e| events.get_mut(e));
                    if let Some((_, jam)) = jam.filter(|_| !sourced) {
                        sourced = true;
                        if let Some(&(jammer, asset)) = gadget_at.get(&at) {
                            jam.jammer = Some(hex(jammer));
                            jam.by = carrier(asset).and_then(name);
                        }
                    }
                }
                What::Timer(ms) => {
                    let jam = start(0).and_then(|e| events.get_mut(e));
                    if let Some((_, jam)) = jam.filter(|e| e.1.timer.is_none()) {
                        jam.kind = DeviceEventType::Countdown;
                        jam.timer = Some(f64::from(ms) / 1000.0);
                    }
                }
                What::SignalLost(v) if v != 0 => events.push(event(DeviceEventType::SignalLost)),
                What::Alliance(a) => {
                    let Some(&team) = alliances.get(&a) else {
                        continue;
                    };
                    if alliance.replace(a).is_some_and(|was| was != a) {
                        capture = Some(captures.len());
                        captures.push(Capture {
                            device: index,
                            event: events.len(),
                            frame,
                            team,
                            driver: None,
                        });
                        let (frame, mut event) = event(DeviceEventType::Capture);
                        event.capture = Some(if pest {
                            CaptureKind::Pest
                        } else {
                            CaptureKind::Kludge
                        });
                        events.push((frame, event));
                    }
                }
                What::Player(Named::Control, id) => {
                    let driver = players.get(&id).copied().filter(|&p| Some(p) != o.owner);
                    if let Some(c) = capture.and_then(|c| captures.get_mut(c))
                        && c.driver.is_none()
                    {
                        c.driver = driver;
                    }
                }
                _ => {}
            }
        }

        if !t.is_drone() {
            if let Some(team) = o.team {
                let map = o.kind == DeviceKind::Default;
                changes.push((if map { -1 } else { first.unwrap_or(-1) }, team, map, true));
                if let Some((_, frame)) = o.life.end {
                    changes.push((frame, team, map, false));
                }
            }
            out.cameras.push(Camera {
                entity,
                kind: o.kind,
                owner,
                team: o.team,
                position: Some(place(last.unwrap_or(t.created))),
                placed: first.filter(|_| !t.map).map(when),
                end,
            });
            continue;
        }
        // Who drove it: the control component names the player, the driven
        // one says when.
        let mut sessions: Vec<(Frame, DriveSession)> = Vec::new();
        let (mut driver, mut driving) = (None, false);
        for &(frame, what) in &t.events {
            match what {
                What::Player(Named::Control, id) => {
                    driver = players.get(&id).copied();
                    if let Some((_, s)) = sessions.last_mut().filter(|_| driving)
                        && s.username.is_none()
                    {
                        s.username = driver.and_then(name);
                    }
                }
                What::Occupied(v) if v != 0 && !driving => {
                    driving = true;
                    let session = DriveSession {
                        username: driver.and_then(name),
                        seconds: None,
                        when: when(frame),
                    };
                    sessions.push((frame, session));
                }
                What::Occupied(0) if driving => {
                    driving = false;
                    if let Some((from, s)) = sessions.last_mut() {
                        s.seconds = span(*from, frame);
                    }
                }
                _ => {}
            }
        }
        let points: Vec<(f64, [f32; 3])> = (track.iter())
            .filter(|p| p.0 <= until)
            .filter_map(|p| Some((seconds(p.0)?, p.1)))
            .collect();
        out.drones.push(Drone {
            entity,
            kind: o.kind,
            owner,
            team: o.team,
            deployments: o.life.deploys.iter().map(|&f| when(f)).collect(),
            pickups: o.life.pickups.iter().map(|&f| when(f)).collect(),
            sessions: sessions.into_iter().map(|s| s.1).collect(),
            path: thin(&points)
                .into_iter()
                .map(|(t, p)| {
                    let [x, y, z] = place(p);
                    [millis(t), x, y, z]
                })
                .collect(),
            position: last.map(place),
            end,
        });
    }

    // Who captured: the thrown object of the new side next to the device (a
    // Pest), else that side's drone near it (a Kludge Drone), else the
    // first of that side to drive it.
    let pests = match captures.is_empty() {
        true => Vec::new(),
        false => thrown(data, &blocks, &players),
    };
    for c in &captures {
        let Some(device) = outs.get(c.device) else {
            continue;
        };
        let here = device.tracked.position_at(c.frame);
        let nearest =
            |reach: f64, candidates: &mut dyn Iterator<Item = (Option<usize>, [f32; 3])>| {
                let here = here?;
                candidates
                    .map(|(player, p)| (distance(here, p), player))
                    .filter(|c| c.0 < reach)
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|c| c.1)
            };
        let thrower = nearest(
            PEST_REACH,
            &mut pests.iter().filter_map(|t| {
                let player = t.player.filter(|&p| team_of(p) == Some(c.team))?;
                Some((Some(player), position_at(&t.track, c.frame)?))
            }),
        );
        let by = thrower
            .or_else(|| {
                nearest(
                    KLUDGE_REACH,
                    &mut outs.iter().enumerate().filter_map(|(i, o)| {
                        let other = o.tracked.is_drone() && o.team == Some(c.team) && i != c.device;
                        let p = o.tracked.position_at(c.frame).filter(|p| p[2] > POOL)?;
                        other.then_some((o.owner, p))
                    }),
                )
            })
            .flatten()
            .or(c.driver);
        if let Some((_, event)) = events.get_mut(c.event) {
            event.by = by.and_then(name);
            event.inferred = event.by.is_some();
        }
    }
    events.sort_by_key(|e| e.0);
    out.events = events.into_iter().map(|e| e.1).collect();

    // Cameras alive after each change.
    changes.sort_unstable_by_key(|c| (c.0, !c.3));
    let mut alive = [TeamCameras::default(); 2];
    let mut at = None;
    for (frame, team, map, more) in changes {
        let Some(cameras) = alive.get_mut(team) else {
            continue;
        };
        let count = if map {
            &mut cameras.default
        } else {
            &mut cameras.gadget
        };
        *count = if more {
            *count + 1
        } else {
            count.saturating_sub(1)
        };
        match out.camera_counts.last_mut().filter(|_| at == Some(frame)) {
            Some(last) => last.teams = alive,
            None => out.camera_counts.push(CameraCount {
                teams: alive,
                when: when(frame),
            }),
        }
        at = Some(frame);
    }

    out.views = views(input, &outs, &seconds);
    // The devices looked through, as the stream placed and turned them.
    let timed = |frame: Frame| seconds(frame).map(millis);
    for t in &tracked {
        if !out.views.iter().any(|v| v.entity == t.entity) {
            continue;
        }
        let places = std::iter::once((t.made_at, t.created)).chain(t.track.iter().copied());
        out.poses.push(Pose {
            entity: t.entity,
            drone: t.is_drone(),
            places: places.filter_map(|p| Some((timed(p.0)?, p.1))).collect(),
            turns: (t.turns.iter())
                .filter_map(|q| Some((timed(q.0)?, q.1)))
                .collect(),
        });
    }
    out
}

/// Each device a player was given to look through, from the player tables.
fn views(input: &Input, outs: &[Out], seconds: &dyn Fn(Frame) -> Option<f64>) -> Vec<View> {
    let mut out: Vec<View> = Vec::new();
    // Player -> index of the view they have.
    let mut open: HashMap<u64, usize> = HashMap::new();
    for (start, end, frame) in input.blocks(PLAYER_TABLE_STREAM) {
        let entries = (input.data.get(start..end)).and_then(table_entries);
        let Some(time) = seconds(frame.map_or(-1, Frame::from)) else {
            continue;
        };
        for e in entries.unwrap_or_default() {
            let Some(entity) = e.view else {
                continue;
            };
            let current = open.get(&e.player_id).and_then(|&i| out.get_mut(i));
            if current.as_ref().is_some_and(|v| v.entity == entity) {
                continue;
            }
            if let Some(v) = current {
                v.to = Some(time);
                open.remove(&e.player_id);
            }
            let player = input.players.iter().find(|p| p.id == e.player_id);
            if let (Some(player), true) = (player, entity != 0) {
                open.insert(e.player_id, out.len());
                out.push(View {
                    username: player.username.clone(),
                    entity,
                    known: outs.iter().any(|o| o.tracked.entity == entity),
                    from: time,
                    to: None,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE: u64 = 0xF02B_8C68;
    const PLAYER: u64 = 0x1122_3344_5566_7788;

    /// An observation component: `aim` and the full state with these
    /// `flags` are written when given.
    fn component(aim: Option<[f32; 2]>, flags: Option<[u8; 7]>) -> Vec<u8> {
        let mut d = DEVICE.to_le_bytes().to_vec();
        d.push(u8::from(aim.is_some()));
        for v in aim
            .iter()
            .flatten()
            .chain(&[0.0, 0.0])
            .take(aim.map_or(0, |_| 4))
        {
            d.extend(v.to_le_bytes());
        }
        d.extend(60.0f32.to_le_bytes());
        d.extend([1, 0, u8::from(flags.is_some())]);
        if let Some(flags) = flags {
            for asset in 1..=9u64 {
                d.extend(asset.to_le_bytes());
            }
            d.extend(flags);
            for v in [-45.5f32, -9.25, 5.375] {
                d.extend(v.to_le_bytes());
            }
            d.extend([0; 4]);
            d.push(1);
            if flags[TIMED] == 1 {
                d.extend(10_000u32.to_le_bytes());
                d.extend([0; 4]);
            }
        }
        d.extend(0xF02B_0001u64.to_le_bytes());
        d
    }

    fn read(bytes: &[u8]) -> Option<(Observation, usize)> {
        let mut r = Reader(bytes);
        let o = observation(&mut r)?;
        Some((o, bytes.len() - r.0.len()))
    }

    #[test]
    fn a_short_observation_component_is_24_bytes() {
        let bytes = component(None, None);
        assert_eq!(bytes.len(), 24);
        let (o, size) = read(&bytes).unwrap();
        assert_eq!(size, 24);
        assert_eq!((o.own, o.holder), (DEVICE, 0xF02B_0001));
        assert_eq!((o.fov, o.mobile, o.aim, o.full), (60.0, true, None, None));
    }

    #[test]
    fn a_full_observation_component_is_120_bytes() {
        let bytes = component(None, Some([0, 0, 1, 0, 0, 0, 0]));
        assert_eq!(bytes.len(), 120);
        let (o, size) = read(&bytes).unwrap();
        assert_eq!(size, 120);
        let full = o.full.unwrap();
        assert!(full.assets);
        assert_eq!(full.flags[DESTROYED], 1);
        assert_eq!(full.position, [-45.5, -9.25, 5.375]);
        assert_eq!((full.pest, full.timer), (1, None));
        assert_eq!(o.holder, 0xF02B_0001);
    }

    #[test]
    fn aim_adds_16_bytes_and_a_timer_8() {
        let aimed = component(Some([1.5, -0.25]), None);
        assert_eq!(aimed.len(), 24 + 16);
        let (o, size) = read(&aimed).unwrap();
        assert_eq!((o.aim, size), (Some([1.5, -0.25]), 40));

        let timed = component(None, Some([0, 1, 0, 0, 0, 1, 0]));
        assert_eq!(timed.len(), 120 + 8);
        let (o, size) = read(&timed).unwrap();
        assert_eq!((o.full.unwrap().timer, size), (Some(10_000), 128));

        let both = component(Some([0.0, 0.5]), Some([0, 1, 0, 0, 0, 1, 0]));
        assert_eq!(both.len(), 120 + 16 + 8);
        let (o, size) = read(&both).unwrap();
        assert_eq!(size, both.len());
        assert_eq!(o.holder, 0xF02B_0001);
    }

    #[test]
    fn an_observation_component_cut_short_is_refused() {
        for bytes in [
            component(None, None),
            component(Some([1.0, 2.0]), None),
            component(None, Some([0; 7])),
            component(Some([1.0, 2.0]), Some([0, 1, 0, 0, 0, 1, 0])),
        ] {
            for cut in 0..bytes.len() {
                assert_eq!(read(&bytes[..cut]), None, "cut at {cut} of {}", bytes.len());
            }
        }
        // Bytes that are no flag: the aim, the full state, the timer.
        let mut aim = component(None, None);
        aim[8] = 2;
        assert_eq!(read(&aim), None);
        let mut full = component(None, None);
        full[15] = 2;
        assert_eq!(read(&full), None);
        assert_eq!(read(&component(None, Some([0, 0, 0, 0, 0, 2, 0]))), None);
    }

    /// A `607385fe` payload with this mask and body.
    fn message(mask: u8, body: &[u8]) -> Vec<u8> {
        let mut d = UPDATE.to_vec();
        d.push(mask);
        d.extend(body);
        d
    }

    const DRONE: [Hash; 4] = [OBSERVATION, DRIVEN, CONTROL, OWNER];

    #[test]
    fn an_update_is_read_component_by_component() {
        // Position and live byte, the full state, then a driver taking
        // over and the throw.
        let mut body = vec![0x05];
        for v in [1.5f32, -2.0, 3.25] {
            body.extend(v.to_le_bytes());
        }
        body.extend([0, 0, 0, 0, 1]);
        body.extend(component(None, Some([0; 7])));
        body.extend([0x20, 1]);
        body.push(0x09);
        body.extend(4u32.to_le_bytes());
        body.extend(PLAYER.to_le_bytes());
        body.push(0x0E);
        body.extend(PLAYER.to_le_bytes());
        body.extend(4u32.to_le_bytes());
        body.push(1);
        let whole = message(0xF8, &body);
        let u = update(&whole, &DRONE).unwrap();
        assert_eq!((u.position, u.live), (Some([1.5, -2.0, 3.25]), Some(1)));
        assert!(u.observation.unwrap().full.is_some());
        assert_eq!((u.occupied, u.driver), (Some(1), Some(PLAYER)));
        assert_eq!((u.thrower, u.released), (Some(PLAYER), Some(1)));
        assert_eq!(u.alliances, [None, Some(4), Some(4)]);
        assert!(u.rest.is_empty() && !u.foreign);
    }

    #[test]
    fn what_follows_a_class_not_read_is_left() {
        let foreign: Hash = [0xC1, 0xC6, 0xA2, 0x23];
        let mut body = component(None, None);
        body.extend([0x02]);
        body.extend(PLAYER.to_le_bytes());
        body.extend([9, 9, 9]);
        let classes = [OBSERVATION, PLACED, foreign];
        let cut = message(0x70, &body);
        let u = update(&cut, &classes).unwrap();
        assert_eq!(u.placer, Some(PLAYER));
        assert_eq!((u.rest, u.foreign), (&[9u8, 9, 9][..], true));
        // A known class that does not fit, and bytes after the last one.
        let long = message(0x60, &body);
        let u = update(&long, &classes).unwrap();
        assert_eq!((u.rest.len(), u.foreign), (3, false));
        let mut bad = component(None, None);
        bad.push(0xF8);
        let bad = message(0x60, &bad);
        let u = update(&bad, &classes).unwrap();
        assert_eq!((u.rest, u.foreign, u.placer), (&[0xF8u8][..], false, None));
        // Without its observation component an update is not understood.
        assert_eq!(update(&message(0x40, &body[..20]), &classes), None);
        assert_eq!(update(&message(0x80, &[0x40]), &classes), None);
        assert_eq!(update(&[], &classes), None);
    }

    fn device(classes: &[Hash], events: &[(Frame, What)], deleted: Option<Frame>) -> Tracked {
        Tracked {
            classes: classes.to_vec(),
            events: events.to_vec(),
            deleted,
            ..Tracked::default()
        }
    }

    #[test]
    fn a_drone_is_thrown_picked_up_thrown_again_and_destroyed() {
        let events = [
            (10, What::Live(0)),
            (50, What::Live(1)),
            (51, What::Released(1)),
            (90, What::Live(0)),
            (120, What::Live(1)),
            (200, What::Live(0)),
            (200, What::Destroyed(1)),
        ];
        let l = life(&device(&DRONE, &events, Some(260)));
        assert_eq!((l.deploys, l.pickups), (vec![51, 120], vec![90]));
        assert_eq!((l.end, l.no_flag), (Some((EndKind::Destroyed, 200)), false));
        // Taken back and not thrown again.
        let l = life(&device(&DRONE, &events[..4], None));
        assert_eq!(l.end, Some((EndKind::PickedUp, 90)));
        // Out of the world, then removed: it expired.
        let l = life(&device(&DRONE, &events[..4], Some(95)));
        assert_eq!((l.end, l.pickups), (Some((EndKind::Expired, 90)), vec![]));
    }

    #[test]
    fn a_device_removed_while_live_has_no_flag() {
        // A drone of the prep phase: live in its first state.
        let events = [(2, What::Live(1)), (300, What::Deleted)];
        let l = life(&device(&DRONE, &events, Some(300)));
        assert_eq!(l.deploys, [2]);
        assert_eq!((l.end, l.no_flag), (Some((EndKind::Destroyed, 300)), true));
        let l = life(&device(&[OBSERVATION, BLOB, OWNER], &events, Some(300)));
        assert_eq!((l.end, l.no_flag), (Some((EndKind::Removed, 300)), false));
        // Never out: no deployment and no end.
        let l = life(&device(&DRONE, &[(2, What::Live(0))], Some(300)));
        assert_eq!(l, Life::default());
    }

    #[test]
    fn kinds_follow_from_the_classes() {
        let kind = |rest: &[Hash]| {
            let classes: Vec<Hash> = [OBSERVATION]
                .into_iter()
                .chain(rest.iter().copied())
                .collect();
            DeviceKind::of(&classes, false)
        };
        assert_eq!(kind(&[DRIVEN, CONTROL, OWNER]), DeviceKind::Drone);
        assert_eq!(kind(&[DRIVEN, BLOB, CONTROL, OWNER]), DeviceKind::Yokai);
        assert_eq!(
            kind(&[PLACED, DRIVEN, BLOB, CONTROL]),
            DeviceKind::RceRatero
        );
        assert_eq!(kind(&[BLOB, OWNER]), DeviceKind::BlackEye);
        assert_eq!(kind(&[BLOB, OWNER, FIRE]), DeviceKind::Argus);
        assert_eq!(kind(&[PLACED, FIRE]), DeviceKind::Bulletproof);
        assert_eq!(kind(&[]), DeviceKind::Unknown);
        assert_eq!(DeviceKind::of(&[OBSERVATION], true), DeviceKind::Default);
    }
}
