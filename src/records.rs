//! What each stream recorded, frame by frame (Y8S4+).
//!
//! The decompressed data holds each stream's opening snapshot, in stream-list
//! order, then the main stream with every stream's frame records:
//!
//! ```text
//! per stream:   u64 length, snapshot
//! main stream:  u32 first frame, u32 last frame, u32 n, u32 0
//!               n x { u32 name hash, u32 ?, u32 ?, u8 0, u32 count, u32 0,
//!                     count x { u32 frame, u32 size, u32 0, payload } }
//! ```
//!
//! Streams appear in the main stream in id order; frames rise strictly within
//! each. The frame index (see [`crate::format`]) turns a frame into seconds
//! since the recording started, so every packet the parser reads can be
//! placed to the frame.

/// The records of one stream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubStream {
    pub name_hash: [u8; 4],
    /// Payload bytes of the stream's records.
    pub bytes: u64,
    /// Frame of each record, rising.
    pub frames: Vec<u32>,
}

/// A record's payload in the decompressed data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
    frame: u32,
}

/// The decompressed data split into snapshots and frame records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordMap {
    /// `(start, end)` of each snapshot's payload, in stream-list order.
    pub snapshots: Vec<(usize, usize)>,
    /// Where the main stream starts. Equal to the data length when the file
    /// ends after the snapshots.
    pub main_start: usize,
    pub first_frame: u32,
    pub last_frame: u32,
    /// In the main stream's order (stream id order).
    pub streams: Vec<SubStream>,
    /// Every record, sorted by where its payload starts.
    spans: Vec<Span>,
}

impl RecordMap {
    /// Splits `data`, which must start with `streams` snapshots. `None` when
    /// the bytes do not follow the layout exactly, to the last byte.
    pub fn parse(data: &[u8], streams: usize) -> Option<RecordMap> {
        let u32_at = |at: usize| -> Option<u32> {
            Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
        };
        let mut map = RecordMap::default();
        let mut pos = 0;
        for _ in 0..streams {
            let len = u64::from_le_bytes(data.get(pos..pos + 8)?.try_into().ok()?);
            let end = usize::try_from(len).ok()?.checked_add(pos + 8)?;
            if end > data.len() {
                return None;
            }
            map.snapshots.push((pos + 8, end));
            pos = end;
        }
        map.main_start = pos;
        if pos == data.len() {
            return Some(map);
        }

        map.first_frame = u32_at(pos)?;
        map.last_frame = u32_at(pos + 4)?;
        let n = u32_at(pos + 8)? as usize;
        if n != streams || u32_at(pos + 12)? != 0 {
            return None;
        }
        pos += 16;
        for _ in 0..n {
            let name_hash: [u8; 4] = data.get(pos..pos + 4)?.try_into().ok()?;
            let count = u32_at(pos + 13)?;
            pos += 21;
            let mut s = SubStream {
                name_hash,
                bytes: 0,
                frames: Vec::with_capacity(count as usize),
            };
            for _ in 0..count {
                let frame = u32_at(pos)?;
                let size = u32_at(pos + 4)? as usize;
                let start = pos + 12;
                let end = start.checked_add(size)?;
                if end > data.len() || s.frames.last().is_some_and(|&f| f >= frame) {
                    return None;
                }
                s.frames.push(frame);
                s.bytes += size as u64;
                map.spans.push(Span { start, end, frame });
                pos = end;
            }
            map.streams.push(s);
        }
        if pos != data.len() {
            return None;
        }
        map.spans.sort_unstable_by_key(|s| s.start);
        Some(map)
    }

    /// The frame whose record holds `offset`. `None` in a snapshot or
    /// between records.
    pub fn frame_at(&self, offset: usize) -> Option<u32> {
        let i = self.spans.partition_point(|s| s.start <= offset);
        let span = self.spans.get(i.checked_sub(1)?)?;
        (offset < span.end).then_some(span.frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Records<'a> = ([u8; 4], &'a [(u32, &'a [u8])]);

    const STREAMS: [Records<'static>; 2] = [
        ([1, 2, 3, 4], &[(0, b""), (3, b"abc"), (9, b"xyz!")]),
        ([5, 6, 7, 8], &[(4, b"moved")]),
    ];

    /// Two snapshots, then a main stream with two streams of records.
    fn body() -> Vec<u8> {
        body_with(&STREAMS)
    }

    fn body_with(streams: &[Records]) -> Vec<u8> {
        let mut d = Vec::new();
        for snapshot in [&b"first"[..], b"second one"] {
            d.extend((snapshot.len() as u64).to_le_bytes());
            d.extend(snapshot);
        }
        for v in [0u32, 9, streams.len() as u32, 0] {
            d.extend(v.to_le_bytes());
        }
        for &(hash, records) in streams {
            d.extend(hash);
            d.extend([0xAA; 8]);
            d.push(0);
            d.extend((records.len() as u32).to_le_bytes());
            d.extend(0u32.to_le_bytes());
            for (frame, payload) in records {
                d.extend(frame.to_le_bytes());
                d.extend((payload.len() as u32).to_le_bytes());
                d.extend(0u32.to_le_bytes());
                d.extend(*payload);
            }
        }
        d
    }

    fn find(d: &[u8], pattern: &[u8]) -> usize {
        d.windows(pattern.len()).position(|w| w == pattern).unwrap()
    }

    #[test]
    fn splits_snapshots_and_frame_records() {
        let d = body();
        let map = RecordMap::parse(&d, 2).unwrap();
        assert_eq!(map.snapshots.len(), 2);
        assert_eq!(&d[map.snapshots[1].0..map.snapshots[1].1], b"second one");
        assert_eq!((map.first_frame, map.last_frame), (0, 9));
        assert_eq!(map.streams.len(), 2);
        assert_eq!(map.streams[0].frames, [0, 3, 9]);
        assert_eq!(map.streams[0].bytes, 7);
        assert_eq!(map.streams[1].name_hash, [5, 6, 7, 8]);
    }

    #[test]
    fn places_an_offset_in_its_record() {
        let d = body();
        let map = RecordMap::parse(&d, 2).unwrap();
        assert_eq!(map.frame_at(find(&d, b"xyz!") + 3), Some(9));
        assert_eq!(map.frame_at(find(&d, b"moved")), Some(4));
        assert_eq!(
            map.frame_at(find(&d, b"first")),
            None,
            "snapshots have no frame"
        );
    }

    #[test]
    fn rejects_data_that_does_not_fit_exactly() {
        let d = body();
        assert!(RecordMap::parse(&d[..d.len() - 1], 2).is_none());
        let mut longer = d.clone();
        longer.push(0);
        assert!(RecordMap::parse(&longer, 2).is_none());
        assert!(RecordMap::parse(&d, 3).is_none(), "wrong snapshot count");
    }

    #[test]
    fn a_file_that_ends_after_its_snapshots_has_no_records() {
        let d = body();
        let main = find(&d, &[0, 0, 0, 0, 9, 0, 0, 0]);
        let map = RecordMap::parse(&d[..main], 2).unwrap();
        assert_eq!(map.main_start, main);
        assert!(map.streams.is_empty());
        assert_eq!(map.frame_at(main - 1), None);
    }

    #[test]
    fn the_main_stream_must_hold_every_listed_stream() {
        // Fits to the last byte, but has records for one of two streams.
        let d = body_with(&STREAMS[..1]);
        assert!(RecordMap::parse(&d, 2).is_none());
    }

    #[test]
    fn frames_must_rise_within_a_stream() {
        let mut d = body();
        // Make the record at frame 9 claim frame 2, before frame 3.
        let at = find(&d, b"xyz!") - 12;
        d[at..at + 4].copy_from_slice(&2u32.to_le_bytes());
        assert!(RecordMap::parse(&d, 2).is_none());
    }
}
