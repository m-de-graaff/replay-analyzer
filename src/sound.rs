//! Metal-detector alarms (Y11S3), from the `SoundChannel` stream.
//!
//! The stream is the command list of the sound engine: the events it
//! posts, and what it is told of the objects that sound. A record is
//!
//! ```text
//! u16 mask; per set bit, low to high: u16 count, count x entry
//!
//! bit 0  post event   58 to 136 bytes, below
//! bit 1  position     u64 object, f32 x, y, z (metres, z up), u32 0
//! bit 2  orientation  u64 object, 3 x f32, u32 0
//! bit 3  switch       u64 object, u32 group, u32 state
//! bit 4  parameter    u64 object, u32 id, f32 value
//! bit 5  not known    11 bytes: u64 object, 3 x 0
//! bit 6  not known    8 bytes
//! bit 7  stop         u64 object, u32 playing id
//!
//! post event:
//! u64 object    a body or other entity, a static id of the map, or an
//!               emitter that lives for one sound
//! u32
//! u64 event, u64 event   the asset, or an entity for kinds 1 and 11
//! u8 0, 7 bytes not known
//! u32 playing id
//! u64 owner     the object itself, the entity it belongs to, or 0
//! u32 kind, u32 0 or 3
//! u8 0 or 1; if 1: u8 x; if x is not ff: 38 bytes
//! u8 0 or 1; the same again
//! ```
//!
//! The layout was checked by parsing every record to its last byte: 60,325
//! records of the ten test rounds and 924,852 of 176 real ones, none left
//! over. A record that does not parse so is skipped and counted.
//!
//! Most of what the stream says is in the other streams too (footsteps
//! follow the bodies, kind 1 is gunfire). The alarm of a metal detector is
//! only here: a post of kind 0 on a static object of the map (an id of 40
//! bits) whose owner is another object, the detector's entity. Other map
//! sounds of kind 0 are owned by their own object.
//!
//! An alarm lasts 3.0 seconds. A spectator's file posts it again in every
//! record of that time, a player's file while the player hears it, and
//! else only as it starts and ends. So the posts of one object, event and
//! detector within 3.1 s of the first are one alarm, which is `complete`
//! when they span 2.8 s or more: one cut short by the end of the recording
//! is not. Neither is a post seen once, which a few are that have another
//! event on a detector's object; they are no alarms and stay in the list
//! as incomplete.
//!
//! Where the alarm sounds is the last position the stream gave its object.
//! The stream does not say who walked through. It does place every body,
//! as bodies are sound objects too, so `username` is the player whose body
//! was nearest as the alarm started, when within 2.5 m: inferred, and
//! absent when no body was that near (5 of 259 alarms; the median distance
//! is 1.1 m).

use std::collections::HashMap;

use serde::Serialize;

use crate::entities::Hash;
use crate::loadout::{Input, When};

/// Name hash of the stream read here (`SoundChannel`).
const SOUND_STREAM: Hash = [0x63, 0xFE, 0x54, 0xD3];

/// Record mask bits.
const POSTS: usize = 0;
const POSITIONS: usize = 1;
/// Entry size of the bits 1 to 7.
const SIZES: [usize; 7] = [24, 24, 16, 16, 11, 8, 12];
/// Size of the block a post event can end with, twice.
const OPTIONAL: usize = 38;
/// The byte that says the block is left out.
const NO_BLOCK: u8 = 0xFF;

/// Post kind of sounds owned by an entity or an object of the map.
const OWNED: u32 = 0;
/// Static objects of the map have ids of 40 bits.
const MAP_IDS: std::ops::Range<u64> = 1 << 32..1 << 40;

/// Posts this long after an alarm's first are still the same alarm
/// (seconds); it lasts 3.0.
const ALARM: f64 = 3.1;
/// An alarm seen for at least this long was heard to its end.
const COMPLETE: f64 = 2.8;
/// A body this near a detector as its alarm starts set it off (metres).
const NEAR: f64 = 2.5;

/// One alarm of a metal detector. `when` is its start.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetalDetector {
    /// The detector's entity id, in hex. The same in every round on a map.
    pub detector: String,
    /// Where the alarm sounds: `[x, y, z]` in metres, z up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<[f64; 3]>,
    /// Who set it off: the player whose body was nearest as it started.
    /// Inferred; absent when no body was within 2.5 m.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// The alarm was recorded to its end.
    pub complete: bool,
    /// Seconds from the alarm's first post to its last.
    pub seconds: f64,
    #[serde(flatten)]
    pub when: When,
}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    pub alarms: Vec<MetalDetector>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
}

/// A post event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Post {
    pub object: u64,
    pub event: u64,
    pub playing: u32,
    pub owner: u64,
    pub kind: u32,
}

/// The entries of one record.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Entries {
    pub posts: Vec<Post>,
    /// Object and where it is.
    pub positions: Vec<(u64, [f32; 3])>,
    /// Entries per mask bit.
    pub counts: [usize; 8],
}

/// The bytes of a record that are still to be read.
struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.rest.split_at_checked(n)?;
        self.rest = tail;
        Some(head)
    }

    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.bytes(N)?.try_into().ok()
    }

    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|b| b[0])
    }

    fn u16(&mut self) -> Option<u16> {
        self.take().map(u16::from_le_bytes)
    }

    fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_le_bytes)
    }

    fn u64(&mut self) -> Option<u64> {
        self.take().map(u64::from_le_bytes)
    }

    fn f32(&mut self) -> Option<f32> {
        self.take().map(f32::from_le_bytes)
    }
}

/// Reads one post event.
fn post(r: &mut Reader) -> Option<Post> {
    let object = r.u64()?;
    r.u32()?;
    let event = r.u64()?;
    r.bytes(8 + 8)?;
    let playing = r.u32()?;
    let owner = r.u64()?;
    let kind = r.u32()?;
    r.u32()?;
    for _ in 0..2 {
        match r.u8()? {
            0 => {}
            1 => {
                if r.u8()? != NO_BLOCK {
                    r.bytes(OPTIONAL)?;
                }
            }
            _ => return None,
        }
    }
    Some(Post {
        object,
        event,
        playing,
        owner,
        kind,
    })
}

/// Reads a record. `None` when it does not follow the layout to its last
/// byte.
pub(crate) fn entries(record: &[u8]) -> Option<Entries> {
    let mut r = Reader { rest: record };
    let mask = r.u16()?;
    if mask >> 8 != 0 {
        return None;
    }
    let mut out = Entries::default();
    for (bit, n) in out.counts.iter_mut().enumerate() {
        if mask >> bit & 1 == 0 {
            continue;
        }
        let count = usize::from(r.u16()?);
        *n = count;
        match bit {
            POSTS => {
                for _ in 0..count {
                    out.posts.push(post(&mut r)?);
                }
            }
            POSITIONS => {
                for _ in 0..count {
                    let object = r.u64()?;
                    out.positions.push((object, [r.f32()?, r.f32()?, r.f32()?]));
                    r.u32()?;
                }
            }
            _ => {
                r.bytes(count * SIZES.get(bit - 1)?)?;
            }
        }
    }
    r.rest.is_empty().then_some(out)
}

/// What rings: the object, the event and the detector's entity.
type Bell = (u64, u64, u64);

/// An alarm while its posts are read.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ringing {
    /// Seconds since the recording started of its first and last post.
    start: f64,
    end: f64,
    frame: Option<u32>,
    position: Option<[f32; 3]>,
    /// Index of the player whose body was nearest, and how far.
    nearest: Option<(usize, f64)>,
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f64 {
    (a.iter().zip(b))
        .map(|(a, b)| (f64::from(b) - f64::from(*a)).powi(2))
        .sum::<f64>()
        .sqrt()
}

fn round(v: f64, digits: i32) -> f64 {
    let scale = 10f64.powi(digits);
    (v * scale).round() / scale
}

/// Reads the alarms of the metal detectors from the sound stream.
pub(crate) fn decode(input: &Input) -> Decoded {
    let &Input {
        data,
        players,
        clock,
        ..
    } = input;
    // A body's sound object is its entity of the movement stream.
    let bodies: HashMap<u64, usize> = (players.iter().enumerate())
        .filter_map(|(i, p)| Some((u64::from(p.entities.as_ref()?.movement?), i)))
        .collect();
    // The last place of each body, and of each object of the map.
    let mut body_at: Vec<Option<[f32; 3]>> = vec![None; players.len()];
    let mut placed: HashMap<u64, [f32; 3]> = HashMap::new();
    // The bells in the order they first rang, each with its alarms.
    let mut bells: Vec<(Bell, Vec<Ringing>)> = Vec::new();
    let mut index: HashMap<Bell, usize> = HashMap::new();
    let (mut records, mut unparsed) = (0usize, 0usize);
    for (start, end, frame) in input.blocks(SOUND_STREAM) {
        // A record of a frame without sound is empty.
        let Some(record) = data.get(start..end).filter(|r| r.len() >= 2) else {
            continue;
        };
        records += 1;
        let Some(found) = entries(record) else {
            unparsed += 1;
            continue;
        };
        for (object, position) in found.positions {
            if let Some(at) = bodies.get(&object).and_then(|&i| body_at.get_mut(i)) {
                *at = Some(position);
            } else if MAP_IDS.contains(&object) {
                placed.insert(object, position);
            }
        }
        // The opening snapshot is the start of the recording.
        let now = clock.seconds(frame).unwrap_or(0.0);
        for p in found.posts {
            let rings = p.kind == OWNED
                && MAP_IDS.contains(&p.object)
                && p.owner != 0
                && p.owner != p.object;
            if !rings {
                continue;
            }
            let bell = (p.object, p.event, p.owner);
            let i = *index.entry(bell).or_insert_with(|| {
                bells.push((bell, Vec::new()));
                bells.len() - 1
            });
            let Some((_, alarms)) = bells.get_mut(i) else {
                continue;
            };
            match alarms.last_mut().filter(|a| now - a.start <= ALARM) {
                Some(a) => a.end = now,
                None => {
                    let position = placed.get(&p.object).copied();
                    let near = |at: [f32; 3]| {
                        let bodies = body_at.iter().enumerate();
                        let name = |i: usize| players.get(i).map(|p| p.username.as_str());
                        bodies
                            .filter_map(|(i, b)| Some((i, distance((*b)?, at))))
                            .min_by(|a, b| a.1.total_cmp(&b.1).then(name(a.0).cmp(&name(b.0))))
                    };
                    alarms.push(Ringing {
                        start: now,
                        end: now,
                        frame,
                        position,
                        nearest: position.and_then(near),
                    });
                }
            }
        }
    }

    // The frame each clock reading was first shown in; `None` is the
    // opening snapshot.
    let readings = clock.reading_offsets;
    let shown: Vec<Option<u32>> = (readings.iter())
        .map(|&at| input.map.frame_at(at))
        .collect();
    let reading_in = |frame: Option<u32>| {
        let i = shown.partition_point(|f| *f <= frame).checked_sub(1);
        i.and_then(|i| readings.get(i)).copied().unwrap_or(0)
    };
    let mut alarms: Vec<(String, MetalDetector)> = Vec::new();
    for ((object, _, owner), rung) in bells {
        for a in rung {
            let seconds = a.end - a.start;
            let by = a.nearest.filter(|n| n.1 <= NEAR);
            let alarm = MetalDetector {
                detector: format!("{owner:x}"),
                position: a.position.map(|p| p.map(|v| round(f64::from(v), 2))),
                username: by.and_then(|n| Some(players.get(n.0)?.username.clone())),
                complete: (COMPLETE..=ALARM).contains(&seconds),
                seconds: round(seconds, 3),
                when: clock.when(reading_in(a.frame), a.frame),
            };
            alarms.push((format!("{object:x}"), alarm));
        }
    }
    // In the order they started; two that start together, by their object.
    alarms.sort_by(|a, b| {
        let time = |m: &MetalDetector| m.when.recording_time.unwrap_or(0.0);
        (time(&a.1).total_cmp(&time(&b.1))).then_with(|| a.0.cmp(&b.0))
    });
    let mut out = Decoded {
        alarms: alarms.into_iter().map(|a| a.1).collect(),
        warnings: Vec::new(),
    };
    if unparsed > 0 {
        out.warnings.push(format!(
            "{unparsed} of {records} sound records do not parse"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBJECT: u64 = 0x63_1500_8799;
    const DETECTOR: u64 = 0x63_1500_877C;

    /// A post event of `kind`, with each optional part as given: `None`
    /// for a 0, `Some(false)` for a 1 and ff, `Some(true)` for a block.
    fn post_bytes(kind: u32, parts: [Option<bool>; 2]) -> Vec<u8> {
        let mut d = OBJECT.to_le_bytes().to_vec();
        d.extend(0u32.to_le_bytes());
        d.extend((OBJECT + 5).to_le_bytes());
        d.extend((OBJECT + 5).to_le_bytes());
        d.extend([0, 1, 2, 3, 4, 5, 6, 7]);
        d.extend(77u32.to_le_bytes());
        d.extend(DETECTOR.to_le_bytes());
        d.extend(kind.to_le_bytes());
        d.extend(3u32.to_le_bytes());
        for part in parts {
            match part {
                None => d.push(0),
                Some(false) => d.extend([1, NO_BLOCK]),
                Some(true) => {
                    d.extend([1, 0]);
                    d.extend([0xAB; OPTIONAL]);
                }
            }
        }
        d
    }

    /// A record of these sections: `(bit, entries)`, in bit order.
    fn record(sections: &[(usize, &[Vec<u8>])]) -> Vec<u8> {
        let mask = sections.iter().fold(0u16, |m, s| m | 1 << s.0);
        let mut d = mask.to_le_bytes().to_vec();
        for (_, entries) in sections {
            d.extend((entries.len() as u16).to_le_bytes());
            d.extend(entries.iter().flatten());
        }
        d
    }

    fn position(object: u64, at: [f32; 3]) -> Vec<u8> {
        let mut d = object.to_le_bytes().to_vec();
        d.extend(at.iter().flat_map(|v| v.to_le_bytes()));
        d.extend([0; 4]);
        d
    }

    #[test]
    fn a_post_event_has_two_optional_blocks() {
        let wanted = Post {
            object: OBJECT,
            event: OBJECT + 5,
            playing: 77,
            owner: DETECTOR,
            kind: 4,
        };
        for (parts, size) in [
            ([None, None], 58),
            ([Some(false), None], 59),
            ([None, Some(false)], 59),
            ([Some(false), Some(false)], 60),
            ([Some(true), None], 97),
            ([None, Some(true)], 97),
            ([Some(true), Some(false)], 98),
            ([Some(true), Some(true)], 136),
        ] {
            let d = post_bytes(4, parts);
            assert_eq!(d.len(), size, "{parts:?}");
            let mut r = Reader { rest: &d };
            assert_eq!(post(&mut r), Some(wanted), "{parts:?}");
            assert!(r.rest.is_empty(), "{parts:?}");
        }
    }

    #[test]
    fn a_post_event_cut_short_or_with_another_flag_is_rejected() {
        let good = post_bytes(0, [Some(true), Some(false)]);
        for cut in 0..good.len() {
            let mut r = Reader { rest: &good[..cut] };
            assert_eq!(post(&mut r), None, "cut at {cut}");
        }
        let mut other = post_bytes(0, [None, None]);
        other[56] = 2;
        assert_eq!(post(&mut Reader { rest: &other }), None);
    }

    #[test]
    fn a_record_holds_the_sections_its_mask_names() {
        let posts = [post_bytes(0, [None, None]), post_bytes(1, [Some(true); 2])];
        let places = [
            position(OBJECT, [1.5, -2.0, 0.75]),
            position(7, [0.0, 0.0, 9.0]),
        ];
        let sized = |bit: usize, n: usize| vec![vec![bit as u8; SIZES[bit - 1]]; n];
        let d = record(&[
            (0, &posts),
            (1, &places),
            (2, &sized(2, 3)),
            (3, &sized(3, 1)),
            (4, &sized(4, 2)),
            (5, &sized(5, 1)),
            (6, &sized(6, 4)),
            (7, &sized(7, 1)),
        ]);
        let e = entries(&d).unwrap();
        assert_eq!(e.counts, [2, 2, 3, 1, 2, 1, 4, 1]);
        assert_eq!(e.posts.iter().map(|p| p.kind).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(
            e.positions,
            [(OBJECT, [1.5, -2.0, 0.75]), (7, [0.0, 0.0, 9.0])]
        );
        // Each kind alone, and a record of no sections.
        for bit in 2..8 {
            let e = entries(&record(&[(bit, &sized(bit, 2))])).unwrap();
            assert_eq!(e.counts[bit], 2, "bit {bit}");
            assert_eq!(e.counts.iter().sum::<usize>(), 2, "bit {bit}");
        }
        assert_eq!(entries(&[0, 0]), Some(Entries::default()));
        assert_eq!(entries(&record(&[(3, &[])])), Some(Entries::default()));
    }

    #[test]
    fn a_record_must_parse_to_its_last_byte() {
        let d = record(&[
            (0, &[post_bytes(0, [None, Some(true)])]),
            (1, &[position(OBJECT, [1.0, 2.0, 3.0])]),
            (7, &[vec![7; 12]]),
        ]);
        assert!(entries(&d).is_some());
        for cut in 0..d.len() {
            assert_eq!(entries(&d[..cut]), None, "cut at {cut}");
        }
        let mut long = d.clone();
        long.push(0);
        assert_eq!(entries(&long), None);
        // A bit no record has.
        assert_eq!(entries(&[0, 1, 0, 0]), None);
        // A count the record cannot hold.
        assert_eq!(entries(&[0x04, 0, 0xFF, 0xFF, 0, 0]), None);
        assert_eq!(entries(&[]), None);
    }
}
