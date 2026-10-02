//! Markers on the map (Y11S3): the players' pings, operators spotted
//! through a drone or camera, the tracking markers of abilities and the
//! device markers of Solis, from the `MarkerChannel` stream.
//!
//! The stream's hash is the CRC-32 of that name. A record holds what
//! changed in its frame, not the markers that are showing; a record
//! without bytes says nothing:
//!
//! ```text
//! u16 n, n x marker (55 bytes), u16 n, n x device (42 bytes)
//!
//! marker
//! +0   f32 x, y, z   map coordinates; 0 on a removal
//! +12  f32           0.0 or 1.0 (unknown; always 1.0 on a ping)
//! +16  u64           class 0: the pinger's playerid
//!                    class 1, 2: the body entity of the marked player
//! +24  u64           class 0: label of what was pinged, 0 for none
//! +32  u32 class     0 ping, 1 spotted operator, 2 tracking marker
//! +36  u32 alliance  class 0: the pinger's; class 1, 2: the team that
//!                    sees the marker; 5 or 0 on removals
//! +40  u32 source    class 2: the ability, see [`source_name`]; else 14
//! +44  u32           class 0: the pinger's place in the team, 1 to 5
//! +48  u32 target    1 an enemy object, 2 an object of the own team,
//!                    3 a location, 6 the body of an operator
//! +52  u8 removed, u8, u8
//!
//! device
//! +0   u32 op        1 add, 2 remove, 3 update
//! +4   u64 handle    counts up from 0
//! +12  u64           the entity marked; 0 on a remove
//! +20  u8 0, u8      1 on an update
//! +22  f32 x, y, z   0 on a remove
//! +34  u32 0, u32 1
//! ```
//!
//! A ping is written once and never removed: how long it showed is not in
//! the file. Nor is what entity was pinged, only a label for its kind,
//! which occurs nowhere else in the file. [`label_name`] names the labels
//! that were matched to the entity at the pinged position; the kind of a
//! ping on an object (`target` 1 or 2) was confirmed by whose entity was
//! nearest. A location ping with the label `5ec64e0d3e` is the second
//! press of a double ping: it follows the same player's ping, at the same
//! spot or on the object next to it, within half a second.
//!
//! A spotted operator is written once too, with the body and the team that
//! sees it. Who spotted is not in the file; a player of the seeing team
//! was on a drone or camera at nearly every spot, which is why it is taken
//! to be the red ping.
//!
//! A tracking marker is one per body and source. It is written again as it
//! moves, and ends with a record that has `removed` set, or with one of
//! source 14 that clears every marker of a body as its player dies. Lion's
//! scan and Grim's swarm write a marker per pulse and never remove it: how
//! long those show is not in the file. The sources were named by the
//! status effect (see [`crate::vitals`]) on the marked player.
//!
//! Device markers were seen only in rounds with Solis, on attackers'
//! devices and on objects of the map. That they are what her SPEC-IO
//! detects, and an update a device identified, is inferred.
//!
//! The stream's opening snapshot is the last marker sent before the
//! recording began. It is no event of the round and is left out.

use std::collections::HashMap;

use serde::{Serialize, Serializer};

use crate::entities::Hash;
use crate::loadout::{Input, STATE_STREAM, When};

/// Name hash of the stream read here: CRC-32 of `MarkerChannel`.
const MARKER_STREAM: Hash = [0x26, 0xB9, 0xC2, 0xC1];

/// Marker classes.
const PING: u32 = 0;
const SPOT: u32 = 1;
const TRACK: u32 = 2;
/// The source of a marker no ability made. With `removed` it clears every
/// marker of the body.
const NO_SOURCE: u32 = 14;
/// Sources that write a marker per pulse and never remove it: Lion's scan
/// and Grim's swarm.
const PULSES: [u32; 2] = [3, 4];
/// The label of the second press of a double ping.
const REPEAT: u64 = 0x5E_C64E_0D3E;

/// Ping targets.
const ENEMY_OBJECT: u32 = 1;
const TEAM_OBJECT: u32 = 2;
const LOCATION: u32 = 3;

/// Device operations.
const ADD: u32 = 1;
const REMOVE: u32 = 2;
const UPDATE: u32 = 3;

/// Labels of pinged objects. Inferred, not read: each is the kind of
/// entity that sat within 0.5 m of the pings with that label (the drone
/// in 46 of 47 pings, the others in all of theirs). About 30 more labels
/// were seen without a name.
const LABELS: [(u64, &str); 22] = [
    (0x30_12E8_69BC, "Shock Wire"),
    (0x30_12E8_6A22, "Drone"),
    (0x30_12E8_6A28, "Yokai"),
    (0x30_12E8_6A70, "Active Defense System"),
    (0x30_12E8_6A76, "Rtila Electroclaw"),
    (0x30_12E8_6A7C, "Entry Denial Device"),
    (0x30_12E8_6A82, "Gu Mine"),
    (0x30_12E8_6A9A, "Black Mirror"),
    (0x30_12E8_6AAC, "Signal Disruptor"),
    (0x30_12E8_6AD6, "Armor Pack"),
    (0x30_12E8_6B0E, "Shock Drone"),
    (0x30_12E8_6B1A, "Black Eye"),
    (0x3A_6E63_90F4, "Volcan Canister"),
    (0x3D_3EE1_596D, "Proximity Alarm"),
    (0x3F_0CCD_494B, "Mag-NET System"),
    (0x3F_3E74_6ECB, "Banshee Sonic Defense"),
    (0x54_4AFA_9C2C, "Kona Station"),
    (0x57_60B1_2B5D, "Razorbloom Shell"),
    (0x5C_07AC_D56B, "Kludge Drone"),
    (0x5C_8118_BE72, "F-NATT Dread Mine"),
    (0x5D_E017_B4D1, "Zoto Canister"),
    (0x64_6C38_C31E, "T.R.I.P. Connector"),
];

/// The name of a ping's label, see [`LABELS`].
pub fn label_name(id: u64) -> Option<&'static str> {
    LABELS.iter().find(|l| l.0 == id).map(|l| l.1)
}

/// The name of a tracking marker's source. Inferred: each is the status
/// effect (see [`crate::vitals::effect_name`]) that showed on the marked
/// player; `LionScan` shows on the team that sees the marker, and
/// `DeimosTracking` marks Deimos himself. Source 8 was seen without a
/// telling effect.
pub fn source_name(source: u32) -> Option<&'static str> {
    Some(match source {
        0 => "JackalTracked",
        1 => "AlibiTracked",
        3 => "LionScan",
        4 => "GrimSwarm",
        5 => "GrimTracked",
        6 => "DeimosMarked",
        7 => "DeimosTracking",
        _ => return None,
    })
}

/// What a ping was put on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PingKind {
    /// A place on the map.
    Location,
    /// The second press of a double ping, where the first was put.
    LocationRepeat,
    /// A gadget, drone or camera of the other team.
    EnemyObject,
    /// One of the pinger's own team.
    TeamObject,
    /// A target seen on no ping so far.
    Other(u32),
}

impl Serialize for PingKind {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            PingKind::Location => s.serialize_str("location"),
            PingKind::LocationRepeat => s.serialize_str("locationRepeat"),
            PingKind::EnemyObject => s.serialize_str("enemyObject"),
            PingKind::TeamObject => s.serialize_str("teamObject"),
            PingKind::Other(target) => s.serialize_u32(*target),
        }
    }
}

/// The kind of object a ping was put on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Label {
    pub id: u64,
    /// Inferred, see [`label_name`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
}

/// A ping a player put on the map.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ping {
    pub username: String,
    /// The pinger's team, from the alliance the record states.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<usize>,
    pub kind: PingKind,
    /// The record's target number, which `kind` is read from.
    pub target: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<Label>,
    /// `[x, y, z]` in metres, z up.
    pub position: [f64; 3],
    #[serde(flatten)]
    pub when: When,
}

/// An operator spotted for a team. Who spotted them is not in the file.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spot {
    /// The player who was spotted.
    pub username: String,
    /// The team that sees the marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seen_by: Option<usize>,
    /// Where the spotted player's body was.
    pub position: [f64; 3],
    #[serde(flatten)]
    pub when: When,
}

/// The ability behind a tracking marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Source {
    pub id: u32,
    /// Inferred, see [`source_name`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
}

/// How a tracking marker ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Ended {
    /// The game removed this marker.
    Removed,
    /// The game cleared every marker of the body: its player died.
    Cleared,
}

/// A tracking marker an ability put on a player. `when` is its start.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    /// The player who was marked.
    pub username: String,
    pub source: Source,
    /// The team that sees the marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seen_by: Option<usize>,
    /// Where the marker was first put.
    pub position: [f64; 3],
    /// Each place the marker was put, when it was moved, as
    /// `[seconds since the start, x, y, z]`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<[f64; 4]>,
    /// The source writes a marker per pulse and never removes it: the file
    /// does not say how long it showed.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub pulse: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<Ended>,
    /// How long the marker stayed. Absent for a pulse.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    /// Still there when the recording ended.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub open: bool,
    #[serde(flatten)]
    pub when: When,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceOp {
    Add,
    Remove,
    Update,
}

/// A change to a device marker.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceMarker {
    pub op: DeviceOp,
    /// The marker's number: the one its `add` gave it.
    pub handle: u64,
    /// The entity marked (hex). A remove names none.
    #[serde(serialize_with = "hex_id", skip_serializing_if = "Option::is_none")]
    pub target: Option<u64>,
    /// Absent on a remove.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<[f64; 3]>,
    #[serde(flatten)]
    pub when: When,
}

fn hex_id<S: Serializer>(v: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(v) => s.collect_str(&format_args!("{v:08x}")),
        None => s.serialize_none(),
    }
}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    pub pings: Vec<Ping>,
    pub spots: Vec<Spot>,
    /// In the order they started.
    pub tracks: Vec<Track>,
    pub devices: Vec<DeviceMarker>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
}

/// A marker as the stream wrote it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Marker {
    position: [f32; 3],
    /// A playerid or a body entity, by class.
    id: u64,
    label: u64,
    class: u32,
    alliance: u32,
    source: u32,
    target: u32,
    removed: bool,
}

/// A device entry as the stream wrote it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Device {
    op: u32,
    handle: u64,
    target: u64,
    position: [f32; 3],
}

/// The first `n` bytes of `rest`, which is left with what follows.
fn take<'a>(rest: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    let (head, tail) = rest.split_at_checked(n)?;
    *rest = tail;
    Some(head)
}

fn u16_of(rest: &mut &[u8]) -> Option<u16> {
    Some(u16::from_le_bytes(take(rest, 2)?.try_into().ok()?))
}

fn u32_of(rest: &mut &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(take(rest, 4)?.try_into().ok()?))
}

fn u64_of(rest: &mut &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(take(rest, 8)?.try_into().ok()?))
}

/// Three finite `f32`s.
fn place_of(rest: &mut &[u8]) -> Option<[f32; 3]> {
    let mut out = [0.0; 3];
    for v in &mut out {
        *v = f32::from_bits(u32_of(rest)?);
        if !v.is_finite() {
            return None;
        }
    }
    Some(out)
}

fn marker(rest: &mut &[u8]) -> Option<Marker> {
    let position = place_of(rest)?;
    take(rest, 4)?;
    let (id, label) = (u64_of(rest)?, u64_of(rest)?);
    let (class, alliance, source) = (u32_of(rest)?, u32_of(rest)?, u32_of(rest)?);
    take(rest, 4)?;
    let target = u32_of(rest)?;
    let flags = take(rest, 3)?;
    Some(Marker {
        position,
        id,
        label,
        class,
        alliance,
        source,
        target,
        removed: flags.first().is_some_and(|&f| f != 0),
    })
}

fn device(rest: &mut &[u8]) -> Option<Device> {
    let (op, handle, target) = (u32_of(rest)?, u64_of(rest)?, u64_of(rest)?);
    take(rest, 2)?;
    let position = place_of(rest)?;
    take(rest, 8)?;
    Some(Device {
        op,
        handle,
        target,
        position,
    })
}

/// Parses one record. `None` when its bytes are not the two lists to the
/// last byte.
fn record(mut rest: &[u8]) -> Option<(Vec<Marker>, Vec<Device>)> {
    if rest.is_empty() {
        return Some((Vec::new(), Vec::new()));
    }
    let rest = &mut rest;
    let markers = (0..u16_of(rest)?)
        .map(|_| marker(rest))
        .collect::<Option<_>>()?;
    let devices = (0..u16_of(rest)?)
        .map(|_| device(rest))
        .collect::<Option<_>>()?;
    rest.is_empty().then_some((markers, devices))
}

fn round(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn place(p: [f32; 3]) -> [f64; 3] {
    p.map(|v| round(f64::from(v)))
}

/// A tracking marker that has not ended.
#[derive(Clone, Copy, Debug)]
struct Open {
    body: u64,
    source: u32,
    /// Index into `Decoded::tracks`.
    index: usize,
    /// Seconds since the recording started.
    start: Option<f64>,
}

/// Turns the markers of the records into events, record by record.
#[derive(Debug, Default)]
struct Walk {
    /// `playerid` -> username.
    pingers: HashMap<u64, String>,
    /// Body entity -> username.
    bodies: HashMap<u64, String>,
    /// Alliance -> team index.
    teams: HashMap<u32, usize>,
    open: Vec<Open>,
    unparsed: usize,
    strangers: usize,
    unknown: usize,
    out: Decoded,
}

impl Walk {
    fn new(players: &[crate::header::Player]) -> Self {
        let mut walk = Walk::default();
        for p in players {
            if p.id != 0 {
                walk.pingers.insert(p.id, p.username.clone());
            }
            if let Some(body) = p.entities.as_ref().and_then(|e| e.movement) {
                walk.bodies.insert(u64::from(body), p.username.clone());
            }
            if let Ok(alliance) = u32::try_from(p.alliance) {
                walk.teams.insert(alliance, p.team_index);
            }
        }
        walk
    }

    /// The events of one record, written at `when`, which is `seconds`
    /// into the recording.
    fn read(&mut self, block: &[u8], seconds: Option<f64>, when: &When) {
        let Some((markers, devices)) = record(block) else {
            self.unparsed += 1;
            return;
        };
        for m in markers {
            self.marker(m, seconds, when);
        }
        for d in devices {
            let op = match d.op {
                ADD => DeviceOp::Add,
                REMOVE => DeviceOp::Remove,
                UPDATE => DeviceOp::Update,
                _ => {
                    self.unknown += 1;
                    continue;
                }
            };
            self.out.devices.push(DeviceMarker {
                op,
                handle: d.handle,
                target: (d.target != 0).then_some(d.target),
                position: (op != DeviceOp::Remove).then(|| place(d.position)),
                when: when.clone(),
            });
        }
    }

    fn marker(&mut self, m: Marker, seconds: Option<f64>, when: &When) {
        let names = match m.class {
            PING => &self.pingers,
            SPOT | TRACK => &self.bodies,
            _ => {
                self.unknown += 1;
                return;
            }
        };
        let team = self.teams.get(&m.alliance).copied();
        let position = place(m.position);
        if m.class == PING {
            let Some(username) = names.get(&m.id).cloned() else {
                self.strangers += 1;
                return;
            };
            let kind = match m.target {
                LOCATION if m.label == REPEAT => PingKind::LocationRepeat,
                LOCATION => PingKind::Location,
                ENEMY_OBJECT => PingKind::EnemyObject,
                TEAM_OBJECT => PingKind::TeamObject,
                other => PingKind::Other(other),
            };
            let label = (m.label != 0).then(|| Label {
                id: m.label,
                name: label_name(m.label),
            });
            self.out.pings.push(Ping {
                username,
                team,
                kind,
                target: m.target,
                label,
                position,
                when: when.clone(),
            });
            return;
        }
        if m.removed {
            // A removal can come again for a marker that is gone already:
            // it then ends nothing.
            let (ended, kept): (Vec<Open>, Vec<Open>) = if m.source == NO_SOURCE {
                (self.open.iter().copied()).partition(|o| o.body == m.id)
            } else {
                (self.open.iter().copied()).partition(|o| o.body == m.id && o.source == m.source)
            };
            self.open = kept;
            let how = if m.source == NO_SOURCE {
                Ended::Cleared
            } else {
                Ended::Removed
            };
            for o in ended {
                self.end(o, seconds, Some(how));
            }
            return;
        }
        let Some(username) = names.get(&m.id).cloned() else {
            self.strangers += 1;
            return;
        };
        if m.class == SPOT {
            self.out.spots.push(Spot {
                username,
                seen_by: team,
                position,
                when: when.clone(),
            });
            return;
        }
        let pulse = PULSES.contains(&m.source);
        let open = (self.open.iter()).find(|o| o.body == m.id && o.source == m.source);
        if let Some(o) = open {
            // The marker moved, unless the record only says it again.
            let [x, y, z] = position;
            if let Some(t) = self.out.tracks.get_mut(o.index)
                && t.path.last().map_or(t.position, |p| [p[1], p[2], p[3]]) != position
            {
                if t.path.is_empty() {
                    let [x, y, z] = t.position;
                    t.path.push([0.0, x, y, z]);
                }
                let after = seconds.zip(o.start).map_or(0.0, |(now, start)| now - start);
                t.path.push([round(after), x, y, z]);
            }
            return;
        }
        if !pulse {
            self.open.push(Open {
                body: m.id,
                source: m.source,
                index: self.out.tracks.len(),
                start: seconds,
            });
        }
        self.out.tracks.push(Track {
            username,
            source: Source {
                id: m.source,
                name: source_name(m.source),
            },
            seen_by: team,
            position,
            path: Vec::new(),
            pulse,
            ended: None,
            seconds: None,
            open: false,
            when: when.clone(),
        });
    }

    /// Files the track `o` as ended `seconds` into the recording.
    fn end(&mut self, o: Open, seconds: Option<f64>, how: Option<Ended>) {
        let Some(t) = self.out.tracks.get_mut(o.index) else {
            return;
        };
        t.ended = how;
        t.open = how.is_none();
        t.seconds = (seconds.zip(o.start)).map(|(end, start)| round((end - start).max(0.0)));
    }

    /// `last` is when the recording ended.
    fn finish(mut self, last: Option<f64>) -> Decoded {
        for o in std::mem::take(&mut self.open) {
            self.end(o, last, None);
        }
        for (count, what) in [
            (
                self.unparsed,
                "marker records do not parse to their last byte",
            ),
            (self.strangers, "markers of no player of the round"),
            (self.unknown, "markers of an unknown class or operation"),
        ] {
            if count > 0 {
                self.out.warnings.push(format!("{count} {what}"));
            }
        }
        self.out
    }
}

/// Reads the pings, spots, tracking markers and device markers of the
/// marker stream.
pub(crate) fn decode(input: &Input) -> Decoded {
    let clock = input.clock;
    // The clock is read from the state stream, and each stream's records
    // are apart in the data: a marker is placed by the end of the state
    // record of its frame, or of the last one before.
    let state: Vec<(u32, usize)> = (input.blocks(STATE_STREAM))
        .filter_map(|(_, end, frame)| Some((frame?, end)))
        .collect();
    let when = |frame: Option<u32>| {
        let i = state.partition_point(|s| Some(s.0) <= frame);
        let at = i.checked_sub(1).and_then(|i| state.get(i));
        clock.when(at.map_or(0, |s| s.1), frame)
    };
    let mut walk = Walk::new(input.players);
    for (start, end, frame) in input.blocks(MARKER_STREAM) {
        let block = input.data.get(start..end).unwrap_or_default();
        // The snapshot holds a marker of before the recording.
        if frame.is_none() {
            walk.unparsed += usize::from(record(block).is_none());
            continue;
        }
        walk.read(block, clock.seconds(frame), &when(frame));
    }
    walk.finish(clock.frame_times.last().copied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{Player, PlayerEntities};

    const PLAYER: u64 = 0x1122_3344_5566_7788;
    const BODY: u32 = 0xF037_0001;

    /// A marker's 55 bytes.
    fn marker_bytes(m: &Marker) -> Vec<u8> {
        let mut d: Vec<u8> = m.position.iter().flat_map(|v| v.to_le_bytes()).collect();
        d.extend(1f32.to_le_bytes());
        d.extend(m.id.to_le_bytes());
        d.extend(m.label.to_le_bytes());
        for v in [m.class, m.alliance, m.source, 0, m.target] {
            d.extend(v.to_le_bytes());
        }
        d.extend([u8::from(m.removed), 0, 0]);
        d
    }

    /// A device entry's 42 bytes.
    fn device_bytes(e: &Device) -> Vec<u8> {
        let mut d = e.op.to_le_bytes().to_vec();
        d.extend(e.handle.to_le_bytes());
        d.extend(e.target.to_le_bytes());
        d.extend([0, u8::from(e.op == UPDATE)]);
        d.extend(e.position.iter().flat_map(|v| v.to_le_bytes()));
        d.extend(0u32.to_le_bytes());
        d.extend(1u32.to_le_bytes());
        d
    }

    fn record_bytes(markers: &[Marker], devices: &[Device]) -> Vec<u8> {
        let mut d = (markers.len() as u16).to_le_bytes().to_vec();
        d.extend(markers.iter().flat_map(marker_bytes));
        d.extend((devices.len() as u16).to_le_bytes());
        d.extend(devices.iter().flat_map(device_bytes));
        d
    }

    fn ping(target: u32, label: u64) -> Marker {
        Marker {
            position: [1.5, -2.25, 3.0],
            id: PLAYER,
            label,
            class: PING,
            alliance: 4,
            source: NO_SOURCE,
            target,
            removed: false,
        }
    }

    /// A marker on the body: a spot, a tracking marker or a removal.
    fn on_body(class: u32, source: u32, removed: bool) -> Marker {
        Marker {
            position: if removed { [0.0; 3] } else { [4.0, 5.0, 6.0] },
            id: u64::from(BODY),
            label: 0,
            class,
            alliance: if removed { 5 } else { 3 },
            source,
            target: 6,
            removed,
        }
    }

    /// One player on team 1, whose alliance is 4; alliance 3 is team 0.
    fn walk() -> Walk {
        let player = Player {
            id: PLAYER,
            username: "p".into(),
            team_index: 1,
            alliance: 4,
            entities: Some(PlayerEntities {
                movement: Some(BODY),
                ..PlayerEntities::default()
            }),
            ..Player::default()
        };
        let mut walk = Walk::new(&[player]);
        walk.teams.insert(3, 0);
        walk
    }

    fn feed(walk: &mut Walk, seconds: f64, markers: &[Marker]) {
        let when = When::default();
        walk.read(&record_bytes(markers, &[]), Some(seconds), &when);
    }

    #[test]
    fn a_ping_names_its_player_kind_and_label() {
        let mut w = walk();
        let drone = 0x30_12E8_6A22;
        let pings = [
            ping(LOCATION, 0),
            ping(LOCATION, REPEAT),
            ping(ENEMY_OBJECT, drone),
            ping(TEAM_OBJECT, 0x99),
            ping(9, 0),
        ];
        feed(&mut w, 1.0, &pings);
        let out = w.finish(Some(2.0));
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let kinds: Vec<PingKind> = out.pings.iter().map(|p| p.kind).collect();
        let expected = [
            PingKind::Location,
            PingKind::LocationRepeat,
            PingKind::EnemyObject,
            PingKind::TeamObject,
            PingKind::Other(9),
        ];
        assert_eq!(kinds, expected);
        let first = &out.pings[0];
        assert_eq!((first.username.as_str(), first.team), ("p", Some(1)));
        assert_eq!(first.position, [1.5, -2.25, 3.0]);
        assert_eq!(first.label, None);
        let label = out.pings[2].label.unwrap();
        assert_eq!((label.id, label.name), (drone, Some("Drone")));
        assert_eq!(out.pings[3].label.unwrap().name, None);
        let json = serde_json::to_value(&out.pings).unwrap();
        assert_eq!(json[1]["kind"], "locationRepeat");
        assert_eq!(json[4]["kind"], 9);
        assert!(json[0].get("label").is_none());
    }

    #[test]
    fn a_ping_of_nobody_is_counted_not_kept() {
        let mut w = walk();
        let mut stranger = ping(LOCATION, 0);
        stranger.id = 7;
        let mut odd = ping(LOCATION, 0);
        odd.class = 3;
        feed(&mut w, 1.0, &[stranger, odd]);
        let out = w.finish(None);
        assert!(out.pings.is_empty());
        assert_eq!(out.warnings.len(), 2, "{:?}", out.warnings);
    }

    #[test]
    fn a_spot_names_the_spotted_player_and_the_team_that_sees_them() {
        let mut w = walk();
        feed(&mut w, 1.0, &[on_body(SPOT, NO_SOURCE, false)]);
        let out = w.finish(Some(9.0));
        assert_eq!(out.spots.len(), 1);
        let spot = &out.spots[0];
        assert_eq!((spot.username.as_str(), spot.seen_by), ("p", Some(0)));
        assert_eq!(spot.position, [4.0, 5.0, 6.0]);
        assert!(out.tracks.is_empty());
    }

    #[test]
    fn a_removal_ends_the_marker_of_its_source() {
        let mut w = walk();
        feed(&mut w, 1.0, &[on_body(TRACK, 5, false)]);
        let mut moved = on_body(TRACK, 5, false);
        moved.position = [7.0, 8.0, 9.0];
        feed(&mut w, 2.5, &[moved, moved, on_body(TRACK, 7, false)]);
        feed(&mut w, 4.0, &[on_body(TRACK, 5, true)]);
        // Said again, it ends nothing; nor does one of another source.
        feed(&mut w, 5.0, &[on_body(TRACK, 5, true)]);
        feed(&mut w, 5.0, &[on_body(TRACK, 0, true)]);
        let out = w.finish(Some(10.0));
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(out.tracks.len(), 2);
        let grim = &out.tracks[0];
        assert_eq!(grim.source.name, Some("GrimTracked"));
        assert_eq!(
            (grim.ended, grim.seconds),
            (Some(Ended::Removed), Some(3.0))
        );
        assert_eq!(grim.position, [4.0, 5.0, 6.0]);
        assert_eq!(
            grim.path,
            [[0.0, 4.0, 5.0, 6.0], [1.5, 7.0, 8.0, 9.0]],
            "the marker moved once"
        );
        assert!(!grim.open && !grim.pulse);
        // The other one was there to the end of the recording.
        let deimos = &out.tracks[1];
        assert_eq!((deimos.ended, deimos.seconds), (None, Some(7.5)));
        assert!(deimos.open && deimos.path.is_empty());
    }

    #[test]
    fn a_clear_ends_every_marker_of_the_body() {
        let mut w = walk();
        feed(
            &mut w,
            1.0,
            &[on_body(TRACK, 6, false), on_body(TRACK, 8, false)],
        );
        feed(&mut w, 3.0, &[on_body(SPOT, NO_SOURCE, true)]);
        let out = w.finish(Some(10.0));
        assert!(out.spots.is_empty(), "a clear is no spot");
        for t in &out.tracks {
            assert_eq!((t.ended, t.seconds), (Some(Ended::Cleared), Some(2.0)));
        }
        assert_eq!(out.tracks[1].source.name, None);
    }

    #[test]
    fn a_pulse_is_a_marker_of_its_own_without_an_end() {
        let mut w = walk();
        feed(&mut w, 1.0, &[on_body(TRACK, 3, false)]);
        feed(&mut w, 2.0, &[on_body(TRACK, 3, false)]);
        feed(&mut w, 3.0, &[on_body(SPOT, NO_SOURCE, true)]);
        let out = w.finish(Some(10.0));
        assert_eq!(out.tracks.len(), 2);
        for t in &out.tracks {
            assert!(t.pulse && !t.open && t.path.is_empty());
            assert_eq!((t.ended, t.seconds), (None, None));
        }
    }

    #[test]
    fn a_device_entry_follows_the_markers() {
        let entry = Device {
            op: ADD,
            handle: 2,
            target: 0x63_1500_877C,
            position: [-66.36, 2.001, 0.846],
        };
        let update = Device {
            op: UPDATE,
            ..entry
        };
        let remove = Device {
            op: REMOVE,
            handle: 2,
            target: 0,
            position: [0.0; 3],
        };
        let odd = Device { op: 4, ..entry };
        // A record without markers still has the device list.
        let bytes = record_bytes(&[], &[entry, update, remove, odd]);
        assert_eq!(bytes.len(), 2 + 2 + 4 * 42);
        let mut w = walk();
        w.read(&bytes, Some(1.0), &When::default());
        let out = w.finish(None);
        let ops: Vec<DeviceOp> = out.devices.iter().map(|d| d.op).collect();
        assert_eq!(ops, [DeviceOp::Add, DeviceOp::Update, DeviceOp::Remove]);
        assert_eq!(out.devices[0].position, Some([-66.36, 2.001, 0.846]));
        let removed = &out.devices[2];
        assert_eq!((removed.target, removed.position), (None, None));
        let json = serde_json::to_value(removed).unwrap();
        assert!(json.get("target").is_none() && json.get("position").is_none());
        assert_eq!(out.warnings.len(), 1, "the unknown operation");
        let json = serde_json::to_value(&out.devices[0]).unwrap();
        assert_eq!(json["target"], "631500877c");
        assert_eq!((&json["op"], &json["handle"]), (&"add".into(), &2.into()));
    }

    #[test]
    fn a_record_must_parse_to_its_last_byte() {
        let entry = Device {
            op: ADD,
            handle: 0,
            target: 1,
            position: [0.0; 3],
        };
        let good = record_bytes(&[ping(LOCATION, 0)], &[entry]);
        assert_eq!(good.len(), 2 + 55 + 2 + 42);
        let (markers, devices) = record(&good).unwrap();
        assert_eq!((markers, devices), (vec![ping(LOCATION, 0)], vec![entry]));
        for cut in 1..good.len() {
            assert_eq!(record(&good[..cut]), None, "cut at {cut}");
        }
        let mut long = good.clone();
        long.push(0);
        assert_eq!(record(&long), None);
        // A record without bytes says nothing, and is no fault.
        assert_eq!(record(&[]), Some((Vec::new(), Vec::new())));
        let mut nan = ping(LOCATION, 0);
        nan.position[1] = f32::NAN;
        assert_eq!(record(&record_bytes(&[nan], &[])), None);

        let mut w = walk();
        w.read(&good[..good.len() - 1], Some(1.0), &When::default());
        w.read(&[], Some(1.0), &When::default());
        let out = w.finish(None);
        assert!(out.pings.is_empty() && out.devices.is_empty());
        assert_eq!(
            out.warnings,
            ["1 marker records do not parse to their last byte"]
        );
    }
}
