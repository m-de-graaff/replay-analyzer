//! Counts of everything seen in a replay, decoded or not.
//!
//! The packet stream is not framed; values are written as properties:
//!
//! ```text
//! 23 <object u32> 00000000 <hash u32> <size u8> <value>   first property of an object
//! 22 <hash u32> <size u8> <value>                         each further property
//! ```
//!
//! Between property runs sit binary blocks (positions, animation state) this
//! parser does not read. The census counts a property only when the byte after
//! its value starts another property, which keeps chance matches in the
//! binary blocks out. It is a survey, not a decoder: use it to see which
//! fields exist, how often, and which of them nothing reads yet.

use std::collections::HashMap;

use rayon::prelude::*;
use serde::Serialize;

use crate::container::{self, Container};
use crate::header::{Header, KNOWN_KEYS};
use crate::records::{Located, RecordMap};

/// How often one known packet marker was found and what came of it.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PacketCount {
    pub name: &'static str,
    /// Hex marker the scanner looks for.
    pub marker: String,
    pub seen: u32,
    /// Markers whose handler failed (bad layout, truncated data).
    pub failed: u32,
}

/// One property hash seen in the stream.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FieldCount {
    /// The property hash as it appears in the stream (hex, stream order).
    pub hash: String,
    /// Name, when this parser decodes the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub known: Option<&'static str>,
    pub count: u32,
    /// Value sizes seen, in bytes.
    pub sizes: Vec<u8>,
    /// How many times it started an object rather than continuing one.
    pub object_starts: u32,
    /// Where it was seen most: a stream's name (`state`), its hash when
    /// unnamed, or `snapshot` for the streams' opening snapshots (Y8S4+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HeaderKeyCount {
    pub key: String,
    pub count: u32,
    pub known: bool,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Census {
    /// Known packet markers, in the order the parser defines them.
    pub packets: Vec<PacketCount>,
    /// Known markers never seen. After a patch this is the first sign of a
    /// format change.
    pub packets_not_seen: Vec<&'static str>,
    pub header_keys: Vec<HeaderKeyCount>,
    /// Header keys the parser does not use.
    pub unknown_header_keys: Vec<String>,
    /// Every property hash seen at least `MIN_FIELD_COUNT` times, most
    /// frequent first.
    pub fields: Vec<FieldCount>,
    pub known_fields: usize,
    pub unknown_fields: usize,
    /// Hashes seen fewer than `MIN_FIELD_COUNT` times; mostly chance
    /// matches in binary data, so only counted.
    pub rare_hashes: usize,
    /// Known fields never seen.
    pub fields_not_seen: Vec<&'static str>,
    /// Streams whose role is unknown, by hash (Y8S4+); record counts are in
    /// `replay.container.streams`. A new hash after a patch is a new stream.
    pub unknown_streams: Vec<String>,
    /// Streams with a known role that this replay does not have (Y8S4+).
    pub streams_not_seen: Vec<&'static str>,
}

/// Hashes seen fewer times than this are treated as noise.
pub const MIN_FIELD_COUNT: u32 = 3;

#[derive(Default, Clone)]
struct Tally {
    count: u32,
    object_starts: u32,
    sizes: Vec<u8>,
    /// Occurrences per place, by the label `tally_fields` was given.
    places: Vec<(u16, u32)>,
}

impl Tally {
    fn add(&mut self, size: u8, object_start: bool, place: u16) {
        self.count += 1;
        self.object_starts += u32::from(object_start);
        if self.sizes.len() < 8 && !self.sizes.contains(&size) {
            self.sizes.push(size);
        }
        self.count_place(place, 1);
    }

    fn count_place(&mut self, place: u16, n: u32) {
        match self.places.iter_mut().find(|p| p.0 == place) {
            Some(p) => p.1 += n,
            None => self.places.push((place, n)),
        }
    }

    fn merge(&mut self, other: Tally) {
        self.count += other.count;
        self.object_starts += other.object_starts;
        for s in other.sizes {
            if self.sizes.len() < 8 && !self.sizes.contains(&s) {
                self.sizes.push(s);
            }
        }
        for (place, n) in other.places {
            self.count_place(place, n);
        }
    }

    /// The place seen most; the first seen wins a tie.
    fn main_place(&self) -> Option<u16> {
        let mut best: Option<(u16, u32)> = None;
        for &(place, n) in &self.places {
            if best.is_none_or(|b| n > b.1) {
                best = Some((place, n));
            }
        }
        best.map(|b| b.0)
    }
}

/// `place` labels: none known, a snapshot, or the main stream's streams from
/// `FIRST_STREAM` on.
const NO_PLACE: u16 = 0;
const SNAPSHOT: u16 = 1;
const FIRST_STREAM: u16 = 2;

/// Counts property hashes in `body`; `place` labels where an offset in
/// `body` belongs.
fn tally_fields(body: &[u8], place: &(dyn Fn(usize) -> u16 + Sync)) -> HashMap<[u8; 4], Tally> {
    const CHUNK: usize = 4 << 20;
    let continues = |at: usize| matches!(body.get(at), Some(0x22 | 0x23));
    (0..body.len().div_ceil(CHUNK))
        .into_par_iter()
        .map(|i| {
            let from = i * CHUNK;
            let to = (from + CHUNK).min(body.len());
            let mut out: HashMap<[u8; 4], Tally> = HashMap::new();
            for at in memchr::memchr2_iter(0x22, 0x23, &body[from..to]) {
                let at = from + at;
                let (hash_at, object_start) = if body[at] == 0x22 {
                    (at + 1, false)
                } else if body.get(at + 5..at + 9) == Some(&[0; 4]) {
                    (at + 9, true)
                } else {
                    continue;
                };
                let Some(head) = body.get(hash_at..hash_at + 5) else {
                    continue;
                };
                let size = head[4];
                // Empty values are nearly always zero padding, not properties.
                if size == 0 || !continues(hash_at + 5 + usize::from(size)) {
                    continue;
                }
                let hash = head[..4].try_into().expect("4 bytes");
                out.entry(hash)
                    .or_default()
                    .add(size, object_start, place(at));
            }
            out
        })
        .reduce(HashMap::new, |mut a, b| {
            for (k, v) in b {
                a.entry(k).or_default().merge(v);
            }
            a
        })
}

/// Builds the census. `known_fields` names the property hashes the parser
/// reads for this replay's version. `body` starts `body_start` bytes into the
/// data that `records` maps (Y8S4+).
pub fn build(
    body: &[u8],
    body_start: usize,
    header: &Header,
    packets: Vec<PacketCount>,
    known_fields: &[(&'static str, [u8; 4])],
    records: Option<&RecordMap>,
    streams: Option<&Container>,
) -> Census {
    let packets_not_seen = packets
        .iter()
        .filter(|p| p.seen == 0)
        .map(|p| p.name)
        .collect();

    let header_keys: Vec<HeaderKeyCount> = header
        .keys
        .iter()
        .map(|(key, count)| HeaderKeyCount {
            key: key.clone(),
            count: *count,
            known: KNOWN_KEYS.contains(&key.as_str()),
        })
        .collect();
    let unknown_header_keys = header_keys
        .iter()
        .filter(|k| !k.known)
        .map(|k| k.key.clone())
        .collect();

    let place = |at: usize| match records.and_then(|m| m.locate(body_start + at)) {
        None => NO_PLACE,
        Some(Located::Snapshot(_)) => SNAPSHOT,
        Some(Located::Record(i, _)) => FIRST_STREAM + i as u16,
    };
    let place_name = |place: u16| -> Option<String> {
        match place {
            NO_PLACE => None,
            SNAPSHOT => Some("snapshot".to_owned()),
            p => {
                let hash = records?
                    .streams
                    .get(usize::from(p - FIRST_STREAM))?
                    .name_hash;
                Some(container::stream_name(hash).map_or_else(|| hex(&hash), str::to_owned))
            }
        }
    };
    let tally = tally_fields(body, &place);
    let name_of = |hash: &[u8; 4]| known_fields.iter().find(|f| &f.1 == hash).map(|f| f.0);
    let rare_hashes = tally.values().filter(|t| t.count < MIN_FIELD_COUNT).count();
    let mut fields: Vec<FieldCount> = tally
        .iter()
        .filter(|(_, t)| t.count >= MIN_FIELD_COUNT)
        .map(|(hash, t)| {
            let mut sizes = t.sizes.clone();
            sizes.sort_unstable();
            FieldCount {
                hash: hex(hash),
                known: name_of(hash),
                count: t.count,
                sizes,
                object_starts: t.object_starts,
                stream: t.main_place().and_then(place_name),
            }
        })
        .collect();
    fields.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.hash.cmp(&b.hash)));
    // Some known fields sit outside property chains, so look for the raw bytes.
    let fields_not_seen = known_fields
        .par_iter()
        .filter(|(_, h)| !tally.contains_key(h) && memchr::memmem::find(body, h).is_none())
        .map(|f| f.0)
        .collect();

    let listed = streams.map_or(&[][..], |c| c.streams.as_slice());
    let unknown_streams = listed
        .iter()
        .filter(|s| s.name.is_none())
        .map(|s| s.hash.clone())
        .collect();
    let streams_not_seen = match streams {
        Some(_) => container::STREAM_NAMES
            .iter()
            .filter(|(hash, _)| !listed.iter().any(|s| s.name_hash == *hash))
            .map(|n| n.1)
            .collect(),
        None => Vec::new(),
    };

    Census {
        packets,
        packets_not_seen,
        header_keys,
        unknown_header_keys,
        known_fields: fields.iter().filter(|f| f.known.is_some()).count(),
        unknown_fields: fields.iter().filter(|f| f.known.is_none()).count(),
        fields,
        rare_hashes,
        fields_not_seen,
        unknown_streams,
        streams_not_seen,
    }
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_chained_properties_only() {
        let mut body = vec![0x23, 1, 2, 3, 4, 0, 0, 0, 0, 0xAA, 0xBB, 0xCC, 0xDD, 1, 7];
        body.extend([0x22, 0x11, 0x22, 0x33, 0x44, 4, 1, 2, 3, 4]);
        body.extend([0x22, 0x11, 0x22, 0x33, 0x44, 4, 1, 2, 3, 4]);
        body.push(0x23); // next object
        // Not followed by a property: ignored.
        body.extend([0x99, 0x22, 0x55, 0x55, 0x55, 0x55, 1, 9, 0x00]);
        let t = tally_fields(&body, &|_| NO_PLACE);
        assert_eq!(t[&[0xAA, 0xBB, 0xCC, 0xDD]].count, 1);
        assert_eq!(t[&[0xAA, 0xBB, 0xCC, 0xDD]].object_starts, 1);
        assert_eq!(t[&[0x11, 0x22, 0x33, 0x44]].count, 2);
        assert_eq!(t[&[0x11, 0x22, 0x33, 0x44]].sizes, vec![4]);
        assert!(!t.contains_key(&[0x55; 4]));
    }
}
