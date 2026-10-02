//! Player bodies in the movement stream (Y11S3): where each one is, where
//! it looks and what it is doing, at every update.
//!
//! The stream's snapshot and each of its frame records hold messages:
//!
//! ```text
//! payload : <count u16>, count x message          an empty record has no bytes
//! message : <object u64> <size u32> <class hash> ...      size counts from the hash
//!           617385fe  create (see `entities::Spawn`); lists the object's classes
//!           607385fe  update
//!           637385fe  destroy
//! update  : <flags u8> [transform] [section of class 1] [section of class 2] ...
//!           flags 80 transform, then 40 20 10 08 04 02: one bit per class of
//!           the object, in the order its creation message lists them
//! ```
//!
//! Sections carry no length, so an update reads only when every section
//! before the wanted one is understood. A player's body is the object
//! created with the five classes of [`BODY`], and all of its sections are:
//!
//! ```text
//! transform  <sub u8>  01 <x y z f32> <u32 0>          metres, z up, at the feet
//!                      02 <qx qy qz qw f32>            the body's heading
//!                      04 <u8>                         1 shown, 0 not
//!                      08 <u16>
//!                      10 <n u8>, n x { <kind u32>; kind 0: 34 bytes }
//! d96bd5f7   <u32> <u64>
//! 56cea924   <n u8>, n x { <id u16> <u8> <k u8>, k x item, <u8> <u8> }
//!            item: <type u8> then 40 bytes (types 00, 04), 22 (03, 07) or 26 (0b)
//! 513b13b2   <sub u8>  01 <u8>;  02 <n u8>, n x 8 bytes;  04 <len u16> <blob>
//!                      08 <n u8>, n x 12 bytes;  10 <n u8>, n x 15 bytes
//!                      (ff stands for all five)
//! a6c5a8dd   <mask u32>, see `view_section`: bit 1 is the view quaternion,
//!            bit 5 the drone or camera the player operates
//! 7a8ac28f   <mask u8>  10 <u8>; 01 <u8> place in team; 02 <u8> team;
//!                       08 <u32> alliance; 04 <u64> playerid   (ff: all five)
//! ```
//!
//! Every field is sent only when it changes, so a sample carries the last
//! value of each forward. A message has no time of its own: it belongs to
//! the frame of its record, and a body has at most one message per record.
//! The layout was checked by reading every message of every body to its
//! last byte, in the ten test rounds and in players' own recordings.

use std::collections::HashMap;

use serde::Serialize;

use crate::entities::ViewChange;
use crate::header::Player;

/// A class or property hash in file byte order.
type Hash = [u8; 4];

const CREATE: Hash = [0x61, 0x73, 0x85, 0xFE];
const UPDATE: Hash = [0x60, 0x73, 0x85, 0xFE];
/// Creation of an object of the map, such as a fixed camera:
/// `<object u64> 00000000 <x y z f32> ...`.
const CREATE_FIXED: Hash = [0x62, 0x73, 0x85, 0xFE];
/// Class of a drone's camera; an object with it is a drone.
const DRONE_CAMERA: Hash = [0x47, 0xE5, 0xF6, 0x00];
/// Class of the 120-byte section drones and placed cameras start with.
const DEVICE: Hash = [0x58, 0x7F, 0x5A, 0x72];
/// Class of whatever a player puts in place: it names the player.
const PLACED: Hash = [0x4C, 0x60, 0x86, 0x9A];
/// Class of the round's objective things: the defuser, which is also
/// [`PLACED`], and the bombs of the map.
const OBJECTIVE: Hash = [0xD0, 0xF6, 0x59, 0x29];
/// Class of the section that names an object's player: [`BODY`]'s last.
const IDENTITY: Hash = BODY[4];

/// The classes a player's body is created with, in order.
pub const BODY: [Hash; 5] = [
    [0xD9, 0x6B, 0xD5, 0xF7],
    [0x56, 0xCE, 0xA9, 0x24],
    [0x51, 0x3B, 0x13, 0xB2],
    [0xA6, 0xC5, 0xA8, 0xDD],
    [0x7A, 0x8A, 0xC2, 0x8F],
];

/// Size of a body's state blob.
pub const BLOB: usize = 722;

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let out = self.d.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(out)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
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
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn quat(&mut self) -> Option<[f32; 4]> {
        Some([self.f32()?, self.f32()?, self.f32()?, self.f32()?])
    }
}

/// What one update of a body changed. `None` fields were not in the message.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Update<'a> {
    pub position: Option<[f32; 3]>,
    /// The body's rotation.
    pub heading: Option<[f32; 4]>,
    /// 1 while the body is shown.
    pub shown: Option<u8>,
    /// Whether the animation section was sent: it stops when the player dies.
    pub animated: bool,
    /// The state blob, [`BLOB`] bytes.
    pub blob: Option<&'a [u8]>,
    /// The view rotation, in the world.
    pub view: Option<[f32; 4]>,
    /// The drone or camera the player operates; 0 for none.
    pub operating: Option<u64>,
    /// Objects the body holds.
    pub holds: Option<Vec<u64>>,
    /// Small values of the view section, by mask bit (8, 14, 15).
    pub flags: [Option<u8>; 3],
    /// `(hash, value)` pairs of the view section's list (bit 18).
    pub levels: Option<Vec<(Hash, f32)>>,
    pub player_id: Option<u64>,
}

fn transform(r: &mut Reader, out: &mut Update) -> Option<()> {
    let sub = r.u8()?;
    if sub & 0xE0 != 0 {
        return None;
    }
    if sub & 0x01 != 0 {
        out.position = Some([r.f32()?, r.f32()?, r.f32()?]);
        r.u32()?;
    }
    if sub & 0x02 != 0 {
        out.heading = Some(r.quat()?);
    }
    if sub & 0x04 != 0 {
        out.shown = Some(r.u8()?);
    }
    if sub & 0x08 != 0 {
        r.u16()?;
    }
    if sub & 0x10 != 0 {
        for _ in 0..r.u8()? {
            match r.u32()? {
                0 => {
                    r.take(34)?;
                }
                1..=3 => {}
                _ => return None,
            }
        }
    }
    Some(())
}

fn animation(r: &mut Reader) -> Option<()> {
    for _ in 0..r.u8()? {
        r.take(3)?;
        for _ in 0..r.u8()? {
            let size = match r.u8()? {
                0x00 | 0x04 => 40,
                0x03 | 0x07 => 22,
                0x0B => 26,
                _ => return None,
            };
            r.take(size)?;
        }
        r.take(2)?;
    }
    Some(())
}

fn state_section<'a>(r: &mut Reader<'a>, out: &mut Update<'a>) -> Option<()> {
    let mut sub = r.u8()?;
    if sub == 0xFF {
        sub = 0x1F;
    }
    if sub & 0xE0 != 0 {
        return None;
    }
    if sub & 0x01 != 0 {
        r.u8()?;
    }
    if sub & 0x02 != 0 {
        let n = r.u8()? as usize;
        r.take(8 * n)?;
    }
    if sub & 0x04 != 0 {
        let len = r.u16()? as usize;
        let blob = r.take(len)?;
        out.blob = (len == BLOB).then_some(blob);
    }
    if sub & 0x08 != 0 {
        let n = r.u8()? as usize;
        r.take(12 * n)?;
    }
    if sub & 0x10 != 0 {
        let n = r.u8()? as usize;
        r.take(15 * n)?;
    }
    Some(())
}

/// The `a6c5a8dd` section: a mask and one field per bit, in bit order
/// except that bits 14 and 15 come after bit 19.
///
/// ```text
/// 0  <n u16>, n x <object u64>      what the body holds
/// 1  <qx qy qz qw f32>              the view
/// 2  nothing    3, 4, 6, 7  <f32>   8 <u8>   9  24 bytes   10  9 bytes
/// 5  <object u64>                   the drone or camera operated, 0 none
/// 11-13  only together, in a full state: 9 bytes here, 2 after bit 15
/// 18 <n u32>, n x { <hash> <f32> }  19 <hash> <f32>
/// 14, 15 <u8>
/// then <t u8>, and outside a full state one more byte when t is 1
/// ```
fn view_section(r: &mut Reader, out: &mut Update) -> Option<()> {
    let m = r.u32()?;
    let full = m & 0x3800 == 0x3800;
    if (m & 0x3800 != 0 && !full) || m >> 16 & 3 != 0 || m >> 20 != 0 {
        return None;
    }
    if m & 1 != 0 {
        let n = r.u16()?;
        out.holds = Some((0..n).map(|_| r.u64()).collect::<Option<_>>()?);
    }
    if m & 2 != 0 {
        out.view = Some(r.quat()?);
    }
    for bit in 3..=10 {
        if m >> bit & 1 == 0 {
            continue;
        }
        match bit {
            5 => out.operating = Some(r.u64()?),
            8 => out.flags[0] = Some(r.u8()?),
            9 => {
                r.take(24)?;
            }
            10 => {
                r.take(9)?;
            }
            _ => {
                r.f32()?;
            }
        }
    }
    if full {
        r.take(9)?;
    }
    if m >> 18 & 1 != 0 {
        let n = r.u32()?;
        if n > 200 {
            return None;
        }
        let pairs = (0..n).map(|_| Some((r.take(4)?.try_into().ok()?, r.f32()?)));
        out.levels = Some(pairs.collect::<Option<_>>()?);
    }
    if m >> 19 & 1 != 0 {
        r.take(8)?;
    }
    if m >> 14 & 1 != 0 {
        out.flags[1] = Some(r.u8()?);
    }
    if m >> 15 & 1 != 0 {
        out.flags[2] = Some(r.u8()?);
    }
    if full {
        r.take(2)?;
    }
    let t = r.u8()?;
    if !full {
        match t {
            0 => {}
            1 => {
                r.u8()?;
            }
            _ => return None,
        }
    }
    Some(())
}

fn identity(r: &mut Reader, out: &mut Update) -> Option<()> {
    let mut m = r.u8()?;
    if m == 0xFF {
        m = 0x1F;
    }
    if m & 0xE0 != 0 {
        return None;
    }
    if m & 0x10 != 0 {
        r.u8()?;
    }
    if m & 0x01 != 0 {
        r.u8()?;
    }
    if m & 0x02 != 0 {
        r.u8()?;
    }
    if m & 0x08 != 0 {
        r.u32()?;
    }
    if m & 0x04 != 0 {
        out.player_id = Some(r.u64()?);
    }
    Some(())
}

/// Reads a body's update; `msg` starts after the class hash. `None` unless
/// it reads to its last byte.
pub fn body_update(msg: &[u8]) -> Option<Update<'_>> {
    let mut r = Reader { d: msg, p: 0 };
    let mut out = Update::default();
    let flags = r.u8()?;
    if flags & 0x03 != 0 {
        return None;
    }
    if flags & 0x80 != 0 {
        transform(&mut r, &mut out)?;
    }
    if flags & 0x40 != 0 {
        r.take(12)?;
    }
    if flags & 0x20 != 0 {
        animation(&mut r)?;
        out.animated = true;
    }
    if flags & 0x10 != 0 {
        state_section(&mut r, &mut out)?;
    }
    if flags & 0x08 != 0 {
        view_section(&mut r, &mut out)?;
    }
    if flags & 0x04 != 0 {
        identity(&mut r, &mut out)?;
    }
    (r.p == msg.len()).then_some(out)
}

/// One message of a movement payload.
pub struct Message<'a> {
    pub object: u64,
    pub class: Hash,
    /// What follows the class hash.
    pub body: &'a [u8],
}

/// The messages of a snapshot or record payload. Stops at the first one
/// that does not fit.
pub fn messages(payload: &[u8]) -> impl Iterator<Item = Message<'_>> {
    let mut r = Reader { d: payload, p: 2 };
    std::iter::from_fn(move || {
        let object = r.u64()?;
        let size = r.u32()? as usize;
        let rest = r.take(size)?;
        Some(Message {
            object,
            class: rest.get(..4)?.try_into().ok()?,
            body: &rest[4..],
        })
    })
}

/// Something the stream created: a body, a drone, a camera, a gun, a
/// gadget, a reinforced wall.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entity {
    /// Where it is: where it was created, then wherever its updates put
    /// it. Deployables are created at (0, 0, -100) and moved into place.
    pub position: [f32; 3],
    /// Its classes; none for an object of the map.
    pub classes: Vec<Hash>,
    pub asset: u64,
    /// The values of its slots: for a body, the assets it carries.
    pub slots: Vec<u64>,
    /// The player it says it belongs to, as drones and placed things do.
    pub player_id: Option<u64>,
}

impl Entity {
    pub fn is_drone(&self) -> bool {
        self.classes.contains(&DRONE_CAMERA)
    }
}

/// Reads a creation message (see `entities::Spawn`); `msg` starts after the
/// class hash.
fn created(msg: &[u8]) -> Option<Entity> {
    // The object again and a zero.
    let mut r = Reader { d: msg, p: 12 };
    let position = [r.f32()?, r.f32()?, r.f32()?];
    // The rotation, a flag and a u64.
    r.take(16 + 1 + 8)?;
    let n = r.u32()? as usize;
    if n > 64 {
        return None;
    }
    let (classes, _) = r.take(4 * n)?.as_chunks::<4>();
    let asset = r.u64()?;
    r.u32()?;
    let n = r.u32()? as usize;
    if n > 64 {
        return None;
    }
    let (slots, _) = r.take(16 * n)?.as_chunks::<16>();
    let value = |s: &[u8; 16]| u64::from_le_bytes(s[..8].try_into().expect("8 bytes"));
    Some(Entity {
        position,
        classes: classes.to_vec(),
        asset,
        slots: slots.iter().map(value).collect(),
        player_id: None,
    })
}

/// What an update of a drone or of something placed says, as far as its
/// sections are understood: where it is, and whose it is.
///
/// ```text
/// 587f5a72   120 bytes
/// 47e5f600   <mask u8>  01 <4 x f32> the camera; 08 <u8>; 20 <u8>; 80 <u32>
///            (ff: 36 bytes)
/// 4c60869a   <flags u8>  02 <playerid u64> who placed it
///                        04 <object u64> what it is attached to
/// 7a8ac28f   as on a body: the owner of a drone
/// ```
///
/// The player comes with the first update that puts a placed thing where
/// it goes, when its placing starts.
fn device_update(classes: &[Hash], msg: &[u8]) -> (Option<[f32; 3]>, Option<u64>) {
    let mut r = Reader { d: msg, p: 0 };
    let mut out = Update::default();
    let mut read = || -> Option<u64> {
        let flags = r.u8()?;
        if flags & 0x80 != 0 {
            transform(&mut r, &mut out)?;
        }
        for (i, class) in classes.iter().enumerate().take(6) {
            if flags & (0x40 >> i) == 0 {
                continue;
            }
            match *class {
                DEVICE => {
                    r.take(120)?;
                }
                DRONE_CAMERA => match r.u8()? {
                    0xFF => {
                        r.take(36)?;
                    }
                    m if m & !0xA9 == 0 => {
                        let size = [(0x01, 16), (0x08, 1), (0x20, 1), (0x80, 4)];
                        let n = size.iter().filter(|s| m & s.0 != 0).map(|s| s.1).sum();
                        r.take(n)?;
                    }
                    _ => return None,
                },
                PLACED => {
                    let flags = r.u8()?;
                    return (flags & 0x02 != 0).then(|| r.u64()).flatten();
                }
                IDENTITY => {
                    identity(&mut r, &mut out)?;
                    return out.player_id;
                }
                _ => return None,
            }
        }
        None
    };
    let player = read().filter(|&id| id != 0 && id != u64::MAX);
    (out.position, player)
}

/// One update of the defuser, as far as it says anything. The defuser is
/// created in the snapshot with the classes [`PLACED`] and [`OBJECTIVE`],
/// at (0, 0, 0), where it stays while a player carries it.
///
/// ```text
/// 4c60869a   <mask u8>  01 <u32>; 02 <playerid u64> who holds it
///                       04 <object u64> what it sits on
/// d0f65929   <mask u8>  01 <u8> a bomb: 1 on the round's two
///                       02 <u8> a bomb: ff, and 0 once the round is decided
///                       04 <u8> a bomb: its number, 1 or 2
///                       08 <u8> state bits, the same on the defuser and
///                               the two bombs: 08 lying in the world,
///                               02 planted, 01 and 04 not known
///                       then <u8 0> unless the mask is 0
/// ```
///
/// A bomb is an object of the map (`627385fe`) with [`OBJECTIVE`] alone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DefuserUpdate {
    /// Frame of the record; `None` in the stream's snapshot.
    pub frame: Option<u32>,
    pub position: Option<[f32; 3]>,
    /// Whether the update carries a rotation.
    pub turned: bool,
    pub shown: Option<u8>,
    /// The player now holding it.
    pub player_id: Option<u64>,
    pub state: Option<u8>,
}

/// A bomb of the map: every site of the map has two, and the round's two
/// are `active`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BombSite {
    pub object: u64,
    pub position: [f32; 3],
    pub active: bool,
    /// 1 or 2 within its site; 0 until an update says.
    pub index: u8,
}

/// What an update of the defuser (`defuser`) or of a bomb says; the third
/// value is a bomb's `02` field. `None` unless it reads to its last byte.
fn objective_update(msg: &[u8], defuser: bool) -> Option<(DefuserUpdate, BombSite, Option<u8>)> {
    let mut r = Reader { d: msg, p: 0 };
    let mut moved = Update::default();
    let (mut out, mut bomb, mut live) = (DefuserUpdate::default(), BombSite::default(), None);
    let flags = r.u8()?;
    let (placed, own) = if defuser { (0x40, 0x20) } else { (0, 0x40) };
    if flags & !(0x80 | placed | own) != 0 {
        return None;
    }
    if flags & 0x80 != 0 {
        transform(&mut r, &mut moved)?;
    }
    if flags & placed != 0 {
        let mask = r.u8()?;
        if mask & !0x07 != 0 {
            return None;
        }
        if mask & 0x01 != 0 {
            r.u32()?;
        }
        if mask & 0x02 != 0 {
            out.player_id = Some(r.u64()?);
        }
        if mask & 0x04 != 0 {
            r.u64()?;
        }
    }
    if flags & own != 0 {
        let mask = r.u8()?;
        if mask & !0x0F != 0 {
            return None;
        }
        if mask & 0x01 != 0 {
            bomb.active = r.u8()? == 1;
        }
        if mask & 0x02 != 0 {
            live = Some(r.u8()?);
        }
        if mask & 0x04 != 0 {
            bomb.index = r.u8()?;
        }
        if mask & 0x08 != 0 {
            out.state = Some(r.u8()?);
        }
        if mask != 0 {
            r.u8()?;
        }
    }
    out.position = moved.position;
    out.turned = moved.heading.is_some();
    out.shown = moved.shown;
    (r.p == msg.len()).then_some((out, bomb, live))
}

/// A body's state blob: one fixed block of the character's state, sent
/// whole in nearly every update. No hashes inside; fields are u32 unless
/// noted, at these offsets:
///
/// ```text
///  28  stance          0 standing, 1 crouched, 2 prone
///  36  on a rope: 1 facing up the wall, 0 facing down
///  64  what the character is doing
///                      0 nothing special, 1 a scripted interaction, 4 vaulting,
///                      5 on a drone, 6 dead, 7 downed, 12 on a rope, 14 reviving
///                      or being revived
///  68  what the hands do: 8 while putting something in place
/// 136  f32: 0, 90 or -90: the side leaned to
/// 144  gait            0 still, 1 creeping, 2 walking, 3 running, 4 sprinting,
///                      5 moved by an animation (a vault, a dead body)
/// 180  on a rope       9 not on one; see `ROPE`
/// 184  6 while in the air
/// 344  1 while aiming down sights, 2 while not
/// ```
///
/// Stance, aiming, gait, the air flag and the values 4, 5, 6, 7 and 12 of
/// offset 64 were checked against speed, falls, kills, downs and drone
/// sessions in 13 rounds. Offset 68 is 8 from the moment a reinforcement,
/// barricade or placed gadget names its player until it is in place (4.5 s
/// for a reinforcement, 2.9 s for a barricade). The lean and what the rope
/// values mean are read from behaviour alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub stance: u32,
    pub doing: u32,
    pub hands: u32,
    pub gait: u32,
    pub rope: u32,
    pub facing_up: bool,
    pub airborne: bool,
    pub aiming: bool,
    /// -1 or 1 for the two sides, 0 for none.
    pub lean: i8,
}

/// `doing` of a dead body.
const DEAD: u32 = 6;
/// `hands` of a player putting something in place.
const DEPLOYING: u32 = 8;
/// `doing` of a player on a rope.
const ON_ROPE: u32 = 12;

impl State {
    fn read(blob: &[u8]) -> Option<State> {
        let u = |at: usize| Some(u32::from_le_bytes(blob.get(at..at + 4)?.try_into().ok()?));
        let side = f32::from_bits(u(136)?);
        Some(State {
            stance: u(28)?,
            doing: u(64)?,
            hands: u(68)?,
            gait: u(144)?,
            rope: u(180)?,
            facing_up: u(36)? == 1,
            airborne: u(184)? == 6,
            aiming: u(344)? == 1,
            lean: match side {
                s if s > 45.0 => 1,
                s if s < -45.0 => -1,
                _ => 0,
            },
        })
    }
}

/// The state of a body after one of its updates.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// Frame of the record; `None` in the stream's snapshot.
    pub frame: Option<u32>,
    pub position: [f32; 3],
    /// View direction in degrees: 0 along +y, counter-clockwise from above.
    pub yaw: f32,
    /// Degrees, positive up.
    pub pitch: f32,
    /// Whether the game shows the body: not before an attacker spawns.
    pub shown: bool,
    pub state: State,
}

/// One body's samples.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub object: u64,
    /// The player the body says it belongs to.
    pub player_id: Option<u64>,
    pub samples: Vec<Sample>,
    /// Updates that did not read to their last byte.
    pub unread: usize,
    position: Option<[f32; 3]>,
    view: Option<[f32; 4]>,
    heading: Option<[f32; 4]>,
    shown: bool,
    state: State,
}

/// Yaw and pitch, in degrees, of the direction a rotation turns +y to.
pub fn direction(q: [f32; 4]) -> (f32, f32) {
    let [x, y, z, w] = q.map(f64::from);
    let fx = 2.0 * (x * y - w * z);
    let fy = 1.0 - 2.0 * (x * x + z * z);
    let fz = 2.0 * (y * z + w * x);
    let yaw = (-fx).atan2(fy).to_degrees();
    let pitch = fz.clamp(-1.0, 1.0).asin().to_degrees();
    (yaw as f32, pitch as f32)
}

/// A player starting to place something: the first update of the thing
/// that names them.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub object: u64,
    pub player_id: u64,
    pub frame: Option<u32>,
    pub position: [f32; 3],
}

/// What the movement stream holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stream {
    /// Every body's track, in the order the bodies were created.
    pub tracks: Vec<Track>,
    /// Everything the stream created, bodies included, by object.
    pub entities: HashMap<u64, Entity>,
    /// Everything a player placed, in stream order.
    pub placed: Vec<Placed>,
    /// Every update of the defuser, in stream order.
    pub defuser: Vec<DefuserUpdate>,
    /// The bombs of the map, in the order they were created.
    pub bombs: Vec<BombSite>,
    /// The frame the bombs said the round was decided in.
    pub decided: Option<u32>,
    /// Updates of the defuser and the bombs that could not be read.
    pub unread: usize,
}

/// Reads the stream's blocks in order: `(payload, frame)`, the snapshot
/// first with no frame.
pub fn read<'a>(blocks: impl Iterator<Item = (&'a [u8], Option<u32>)>) -> Stream {
    let mut index: HashMap<u64, usize> = HashMap::new();
    let mut out = Stream::default();
    let mut defuser = None;
    for (payload, frame) in blocks {
        for m in messages(payload) {
            if m.class == CREATE {
                let Some(e) = created(m.body) else { continue };
                if e.classes == BODY && !index.contains_key(&m.object) {
                    index.insert(m.object, out.tracks.len());
                    out.tracks.push(Track {
                        object: m.object,
                        ..Track::default()
                    });
                }
                if e.classes == [PLACED, OBJECTIVE] {
                    defuser = Some(m.object);
                }
                out.entities.entry(m.object).or_insert(e);
                continue;
            }
            if m.class == CREATE_FIXED {
                let mut r = Reader { d: m.body, p: 12 };
                if let (Some(x), Some(y), Some(z)) = (r.f32(), r.f32(), r.f32()) {
                    out.entities.entry(m.object).or_insert(Entity {
                        position: [x, y, z],
                        ..Entity::default()
                    });
                    let bomb = created(m.body).is_some_and(|e| e.classes == [OBJECTIVE]);
                    if bomb && !out.bombs.iter().any(|b| b.object == m.object) {
                        out.bombs.push(BombSite {
                            object: m.object,
                            position: [x, y, z],
                            ..BombSite::default()
                        });
                    }
                }
                continue;
            }
            if m.class != UPDATE {
                continue;
            }
            if Some(m.object) == defuser {
                match objective_update(m.body, true) {
                    Some((u, ..)) => out.defuser.push(DefuserUpdate { frame, ..u }),
                    None => out.unread += 1,
                }
                continue;
            }
            if let Some(b) = out.bombs.iter_mut().find(|b| b.object == m.object) {
                let Some((u, said, live)) = objective_update(m.body, false) else {
                    out.unread += 1;
                    continue;
                };
                b.position = u.position.unwrap_or(b.position);
                b.active |= said.active;
                if said.index != 0 {
                    b.index = said.index;
                }
                if live == Some(0) && out.decided.is_none() {
                    out.decided = frame;
                }
                continue;
            }
            let Some(t) = index.get(&m.object).map(|&i| &mut out.tracks[i]) else {
                let Some(e) = out.entities.get_mut(&m.object) else {
                    continue;
                };
                let (position, player) = device_update(&e.classes, m.body);
                e.position = position.unwrap_or(e.position);
                if let (Some(player_id), None) = (player, e.player_id) {
                    e.player_id = Some(player_id);
                    if e.classes.contains(&PLACED) {
                        out.placed.push(Placed {
                            object: m.object,
                            player_id,
                            frame,
                            position: e.position,
                        });
                    }
                }
                continue;
            };
            let Some(u) = body_update(m.body) else {
                t.unread += 1;
                continue;
            };
            t.position = u.position.or(t.position);
            t.view = u.view.or(t.view);
            t.heading = u.heading.or(t.heading);
            t.shown = u.shown.map_or(t.shown, |s| s == 1);
            t.player_id = u.player_id.filter(|&id| id != u64::MAX).or(t.player_id);
            t.state = u.blob.and_then(State::read).unwrap_or(t.state);
            let (Some(position), Some(q)) = (t.position, t.view.or(t.heading)) else {
                continue;
            };
            let (yaw, pitch) = direction(q);
            t.samples.push(Sample {
                frame,
                position,
                yaw,
                pitch,
                shown: t.shown,
                state: t.state,
            });
        }
    }
    out
}

/// What a player looked through instead of their own eyes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewKind {
    Drone,
    Camera,
    /// What a teammate looks through, followed by a dead player.
    Teammate,
}

/// A stretch of time a player looked through one device.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewSession {
    pub username: String,
    pub kind: ViewKind,
    /// The object looked through, in hex. A camera of the map keeps its
    /// id from round to round.
    pub device: String,
    /// Whose device it is. Absent for the map's cameras.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Whether the device is part of the map.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub fixed: bool,
    /// Where a camera is. Drones move, and have none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<[f32; 3]>,
    /// Seconds since the recording started.
    pub start: f64,
    pub end: f64,
}

fn millis(t: f64) -> f64 {
    (t * 1000.0).round() / 1000.0
}

fn rounded(p: [f32; 3]) -> [f32; 3] {
    p.map(|v| (v * 1000.0).round() / 1000.0)
}

/// Turns the player table's view changes into sessions. A session ends
/// when the player moves to another device or back to their own eyes, or
/// with the recording.
fn view_sessions(
    views: &[ViewChange],
    stream: &Stream,
    players: &[Player],
    frame_times: &[f64],
) -> Vec<ViewSession> {
    let player = |id: u64| players.iter().find(|p| p.id == id && id != 0);
    let end = millis(frame_times.last().copied().unwrap_or(0.0));
    let time = |frame: Option<u32>| {
        let t = frame_times.get(frame.unwrap_or(0) as usize).copied();
        t.map_or(end, millis)
    };
    // The body that carries an asset, when only one does.
    let carrier = |asset: u64| {
        let carries = |t: &&Track| {
            let body = stream.entities.get(&t.object);
            body.is_some_and(|e| e.slots.contains(&asset))
        };
        let mut bodies = stream.tracks.iter().filter(carries);
        let first = bodies.next()?;
        bodies.next().is_none().then_some(first.player_id?)
    };
    let mut out: Vec<ViewSession> = Vec::new();
    let mut open: HashMap<u64, usize> = HashMap::new();
    for v in views {
        let at = time(v.frame);
        if let Some(i) = open.remove(&v.player_id) {
            out[i].end = at;
        }
        let (Some(viewer), true) = (player(v.player_id), v.view != 0) else {
            continue;
        };
        let entity = stream.entities.get(&v.view);
        let kind = match (v.kind, entity) {
            (2, _) => ViewKind::Teammate,
            (4, _) => ViewKind::Drone,
            (1, _) => ViewKind::Camera,
            (_, Some(e)) if e.is_drone() => ViewKind::Drone,
            _ => ViewKind::Camera,
        };
        let fixed = entity.is_some_and(|e| e.classes.is_empty());
        let owner = entity
            .filter(|_| !fixed)
            .and_then(|e| e.player_id.or_else(|| carrier(e.asset)))
            .and_then(player);
        open.insert(v.player_id, out.len());
        out.push(ViewSession {
            username: viewer.username.clone(),
            kind,
            device: format!("{:x}", v.view),
            owner: owner.map(|p| p.username.clone()),
            fixed,
            position: entity
                .filter(|e| !e.is_drone())
                .map(|e| rounded(e.position)),
            start: at,
            end,
        });
    }
    out.retain(|s| s.end > s.start);
    out
}

/// What a player placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PlacedKind {
    Reinforcement,
    Barricade,
    Gadget,
}

/// Assets of a reinforced wall or hatch; the same on every map.
const REINFORCEMENTS: [u64; 10] = [
    0x5E_8C09_105A,
    0x61_4D70_B75D,
    0x61_4D70_B9CC,
    0x61_4D70_BC3B,
    0x61_4D70_BEAA,
    0x61_4D70_C119,
    0x61_4D70_C388,
    0x61_4D70_C5F7,
    0x61_4D70_C866,
    0x61_4D70_CAD5,
];
/// Assets of a barricade on a door or window.
const BARRICADES: [u64; 4] = [
    0x5E_8C09_1069,
    0x5E_8C09_106A,
    0x5E_8C09_106B,
    0x5E_8C09_106C,
];

/// A player starting to put something in place: a reinforcement, a
/// barricade, or a gadget that is placed rather than thrown.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    pub username: String,
    pub kind: PlacedKind,
    /// The asset placed, in hex. Not the item id loadouts use, and one
    /// gadget can have a different asset per operator.
    pub asset: String,
    /// The object created, in hex: a placed camera is the `device` of the
    /// view sessions through it.
    pub object: String,
    pub position: [f32; 3],
    /// Seconds since the recording started, when the placing started.
    pub time: f64,
    /// When the player's hands were done with it. A reinforcement takes
    /// 4.5 seconds and a barricade 2.9; less is one given up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
}

/// A value from `time` on.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Change<T> {
    pub time: f64,
    pub value: T,
}

/// Adds a change when `value` differs from the last one.
fn note<T: PartialEq>(list: &mut Vec<Change<T>>, time: f64, value: T) {
    if list.last().is_none_or(|c| c.value != value) {
        list.push(Change { time, value });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stance {
    Standing,
    Crouched,
    Prone,
    /// A value not seen before.
    Other(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Lean {
    None,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Gait {
    Still,
    Creeping,
    Walking,
    Running,
    Sprinting,
    /// Moved by an animation: a vault, a rope, a dead body.
    Animated,
    Other(u32),
}

/// What the character is doing besides moving about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Doing {
    Nothing,
    /// A scripted interaction: the 8 seconds of a defuser disable, and
    /// half-second stretches whose cause is not known.
    Interacting,
    Vaulting,
    OnDrone,
    Dead,
    Downed,
    Rappelling,
    /// Reviving or being revived: both players show it. Seen on one revive.
    Reviving,
    Other(u32),
}

/// What a player on a rope is doing. The order of the values is the same
/// on every rope checked; what each is called is read from how the body
/// moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Rope {
    /// Not on a rope.
    Off,
    Attaching,
    Mounting,
    Hanging,
    Moving,
    Stopping,
    /// Fast travel: a fast descent, or running along the wall.
    Running,
    /// Turning upside down or back.
    Flipping,
    /// Swinging in through a window: the wind-up, then the entry.
    Entering,
    /// An entry given up.
    EntryAborted,
    Leaving,
    Other(u32),
}

fn rope(state: &State) -> Rope {
    if state.doing != ON_ROPE {
        return Rope::Off;
    }
    match state.rope {
        8 => Rope::Attaching,
        0 => Rope::Mounting,
        1 => Rope::Hanging,
        3 => Rope::Moving,
        4 => Rope::Stopping,
        14 => Rope::Running,
        7 => Rope::Flipping,
        6 | 15 => Rope::Entering,
        16 => Rope::EntryAborted,
        5 => Rope::Leaving,
        other => Rope::Other(other),
    }
}

/// Every player's movement in a round.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Movement {
    pub players: Vec<PlayerTrack>,
    /// What each player looked through when not their own eyes.
    pub views: Vec<ViewSession>,
    /// Reinforcements, barricades and gadgets players placed.
    pub placements: Vec<Placement>,
}

/// One player's movement. The columns `time` to `speed` hold one value per
/// sample; the lists after them hold a value from its `time` until the
/// next entry.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerTrack {
    pub username: String,
    /// Updates of the body that could not be read.
    #[serde(skip_serializing_if = "is_zero")]
    pub unread: usize,
    /// Seconds since the recording started.
    pub time: Vec<f64>,
    /// Metres, at the player's feet; `z` is the height.
    pub x: Vec<f32>,
    pub y: Vec<f32>,
    pub z: Vec<f32>,
    /// Degrees: 0 looks along +y, 90 along -x.
    pub yaw: Vec<f32>,
    /// Degrees, positive up.
    pub pitch: Vec<f32>,
    /// Metres a second over the ground, from the positions of the last
    /// quarter second.
    pub speed: Vec<f32>,
    pub stance: Vec<Change<Stance>>,
    pub lean: Vec<Change<Lean>>,
    pub aiming: Vec<Change<bool>>,
    pub gait: Vec<Change<Gait>>,
    pub doing: Vec<Change<Doing>>,
    /// Whether the hands are putting something in place: a reinforcement,
    /// a barricade, a gadget.
    pub deploying: Vec<Change<bool>>,
    pub airborne: Vec<Change<bool>>,
    pub rope: Vec<Change<Rope>>,
    /// On a rope: whether the player hangs head down.
    pub inverted: Vec<Change<bool>>,
    /// Stretches in the air that ended a metre or more lower: a hatch, a
    /// window, a ledge.
    pub falls: Vec<Fall>,
}

/// A drop through the air.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Fall {
    pub start: f64,
    pub end: f64,
    /// Metres lost.
    pub drop: f32,
}

/// A drop of less than this is a step, not a fall.
const MIN_FALL: f32 = 1.0;

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Positions further back than this give a sample its speed.
const SPEED_WINDOW: f64 = 0.25;

impl PlayerTrack {
    fn push(&mut self, time: f64, s: &Sample) {
        // Speed from the newest sample at least a window back: positions
        // are sent only when they change, and a frame is skipped now and
        // then, so two neighbours say little.
        let back = (self.time.iter()).rposition(|&t| time - t >= SPEED_WINDOW);
        let speed = back.map_or(0.0, |i| {
            let (dx, dy) = (s.position[0] - self.x[i], s.position[1] - self.y[i]);
            f64::from(dx.hypot(dy)) / (time - self.time[i])
        });
        let [x, y, z] = rounded(s.position);
        self.time.push(time);
        self.x.push(x);
        self.y.push(y);
        self.z.push(z);
        self.yaw.push((s.yaw * 100.0).round() / 100.0);
        self.pitch.push((s.pitch * 100.0).round() / 100.0);
        self.speed.push((speed * 100.0).round() as f32 / 100.0);
        let st = &s.state;
        let stance = match st.stance {
            0 => Stance::Standing,
            1 => Stance::Crouched,
            2 => Stance::Prone,
            other => Stance::Other(other),
        };
        let lean = match st.lean {
            1 => Lean::Right,
            -1 => Lean::Left,
            _ => Lean::None,
        };
        let gait = match st.gait {
            0 => Gait::Still,
            1 => Gait::Creeping,
            2 => Gait::Walking,
            3 => Gait::Running,
            4 => Gait::Sprinting,
            5 => Gait::Animated,
            other => Gait::Other(other),
        };
        let doing = match st.doing {
            0 => Doing::Nothing,
            1 => Doing::Interacting,
            4 => Doing::Vaulting,
            5 => Doing::OnDrone,
            DEAD => Doing::Dead,
            7 => Doing::Downed,
            ON_ROPE => Doing::Rappelling,
            14 => Doing::Reviving,
            other => Doing::Other(other),
        };
        note(&mut self.stance, time, stance);
        note(&mut self.lean, time, lean);
        note(&mut self.aiming, time, st.aiming);
        note(&mut self.gait, time, gait);
        note(&mut self.doing, time, doing);
        // On a drone the hands keep what they last did.
        note(
            &mut self.deploying,
            time,
            st.hands == DEPLOYING && st.doing == 0,
        );
        note(&mut self.airborne, time, st.airborne);
        note(&mut self.rope, time, rope(st));
        note(
            &mut self.inverted,
            time,
            st.doing == ON_ROPE && !st.facing_up,
        );
    }

    /// Fills `falls` from the stretches in the air.
    fn find_falls(&mut self) {
        let height = |time: f64| {
            let i = self.time.iter().position(|&t| t >= time)?;
            Some(self.z[i])
        };
        for (i, c) in self.airborne.iter().enumerate().filter(|(_, c)| c.value) {
            let end = self.airborne.get(i + 1).map(|next| next.time);
            let end = end.or(self.time.last().copied()).unwrap_or(c.time);
            if let (Some(from), Some(to)) = (height(c.time), height(end))
                && from - to >= MIN_FALL
            {
                self.falls.push(Fall {
                    start: c.time,
                    end,
                    drop: ((from - to) * 100.0).round() / 100.0,
                });
            }
        }
    }
}

/// Every player's track, view sessions and placements, from the movement
/// stream as [`read`] gives it; a body
/// belongs to the player whose `playerid` it carries, or whose
/// `entities.movement` it is.
///
/// A track runs from the moment the game shows the body (an attacker's is
/// created a moment before prep ends) to the first sample of the dead
/// body, which closes it: a dead body keeps sending for a few seconds.
pub(crate) fn decode(
    stream: &Stream,
    players: &[Player],
    views: &[ViewChange],
    frame_times: &[f64],
) -> Movement {
    let time = |frame: Option<u32>| frame_times.get(frame.unwrap_or(0) as usize).copied();
    let player = |id: u64| players.iter().find(|p| p.id == id && id != 0);
    let mut out = Movement {
        views: view_sessions(views, stream, players, frame_times),
        ..Movement::default()
    };
    for p in players {
        let body = p.entities.as_ref().and_then(|e| e.movement).map(u64::from);
        let mine = |t: &&Track| (p.id != 0 && t.player_id == Some(p.id)) || Some(t.object) == body;
        let mut track = PlayerTrack {
            username: p.username.clone(),
            ..PlayerTrack::default()
        };
        for t in stream.tracks.iter().filter(mine) {
            track.unread += t.unread;
            for s in t.samples.iter().filter(|s| s.shown) {
                let Some(at) = time(s.frame) else { continue };
                track.push(millis(at), s);
                if s.state.doing == DEAD {
                    break;
                }
            }
        }
        track.find_falls();
        out.players.push(track);
    }
    for placed in &stream.placed {
        let (Some(p), Some(e), Some(at)) = (
            player(placed.player_id),
            stream.entities.get(&placed.object),
            time(placed.frame),
        ) else {
            continue;
        };
        out.placements.push(Placement {
            username: p.username.clone(),
            kind: match e.asset {
                a if REINFORCEMENTS.contains(&a) => PlacedKind::Reinforcement,
                a if BARRICADES.contains(&a) => PlacedKind::Barricade,
                _ => PlacedKind::Gadget,
            },
            asset: format!("{:x}", e.asset),
            object: format!("{:x}", placed.object),
            position: rounded(placed.position),
            time: millis(at),
            end: (out.players.iter())
                .find(|t| t.username == p.username)
                .and_then(|t| {
                    let started = t.deploying.iter().rposition(|c| c.time <= at + 0.1)?;
                    let stopped = t.deploying.get(started + 1)?;
                    t.deploying[started].value.then_some(stopped.time)
                }),
        });
    }
    out
}

/// Where each player's body was over a round and nothing else: enough to
/// say where something happened without the whole of [`Movement`].
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Positions {
    /// Per player, in the order of the header: sample times, and the
    /// position at each.
    players: Vec<(Vec<f64>, Vec<[f32; 3]>)>,
}

impl Positions {
    /// Where the player at `index` of the header's list was `time` seconds
    /// into the recording: their last sample by then, since a body that
    /// stands still sends nothing. `None` before the body is shown, and
    /// for a player without one. A dead player stays where they died.
    pub(crate) fn at(&self, index: usize, time: f64) -> Option<[f32; 3]> {
        let (times, positions) = self.players.get(index)?;
        let last = times.iter().rposition(|&t| t <= time)?;
        positions.get(last).copied()
    }
}

/// Every player's positions and nothing else, as [`decode`] gives them.
pub(crate) fn positions(stream: &Stream, players: &[Player], frame_times: &[f64]) -> Positions {
    let time = |frame: Option<u32>| frame_times.get(frame.unwrap_or(0) as usize).copied();
    let mut out = Positions::default();
    for p in players {
        let body = p.entities.as_ref().and_then(|e| e.movement).map(u64::from);
        let mine = |t: &&Track| (p.id != 0 && t.player_id == Some(p.id)) || Some(t.object) == body;
        let (mut times, mut places) = (Vec::new(), Vec::new());
        for t in stream.tracks.iter().filter(mine) {
            for s in t.samples.iter().filter(|s| s.shown) {
                let Some(at) = time(s.frame) else { continue };
                times.push(millis(at));
                places.push(rounded(s.position));
                if s.state.doing == DEAD {
                    break;
                }
            }
        }
        out.players.push((times, places));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_of_a_turn_about_the_height_axis() {
        // No rotation looks along +y; a quarter turn counter-clockwise
        // looks along -x.
        let (yaw, pitch) = direction([0.0, 0.0, 0.0, 1.0]);
        assert!(yaw.abs() < 1e-4 && pitch.abs() < 1e-4);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let (yaw, pitch) = direction([0.0, 0.0, h, h]);
        assert!((yaw - 90.0).abs() < 1e-3 && pitch.abs() < 1e-3);
        // A turn about x lifts the view.
        let (s, c) = (15f32.to_radians().sin(), 15f32.to_radians().cos());
        let (_, pitch) = direction([s, 0.0, 0.0, c]);
        assert!((pitch - 30.0).abs() < 1e-3);
    }

    #[test]
    fn an_update_must_read_to_its_last_byte() {
        // Transform with a position only.
        let mut msg = vec![0x80, 0x01];
        for v in [1.0f32, 2.0, 3.0] {
            msg.extend(v.to_le_bytes());
        }
        msg.extend(0u32.to_le_bytes());
        let u = body_update(&msg).expect("reads");
        assert_eq!(u.position, Some([1.0, 2.0, 3.0]));
        msg.push(0);
        assert!(body_update(&msg).is_none());
    }

    #[test]
    fn a_placed_thing_names_who_placed_it() {
        // Transform with a position, then the placed section with a player.
        let mut msg = vec![0xC0, 0x01];
        for v in [4.0f32, 5.0, 6.0] {
            msg.extend(v.to_le_bytes());
        }
        msg.extend(0u32.to_le_bytes());
        msg.push(0x02);
        msg.extend(77u64.to_le_bytes());
        let classes = [PLACED, [0x6E, 0xA5, 0x1C, 0x35]];
        assert_eq!(
            device_update(&classes, &msg),
            (Some([4.0, 5.0, 6.0]), Some(77))
        );
        // A section that is not understood hides what follows it.
        assert_eq!(device_update(&[[1, 2, 3, 4], PLACED], &msg).1, None);
    }

    #[test]
    fn changes_are_noted_once() {
        let mut list = Vec::new();
        note(&mut list, 1.0, Stance::Standing);
        note(&mut list, 2.0, Stance::Standing);
        note(&mut list, 3.0, Stance::Prone);
        let times: Vec<f64> = list.iter().map(|c| c.time).collect();
        assert_eq!(times, [1.0, 3.0]);
    }
}
