//! The file around the packet data, from Y8S4: the streams a round was
//! recorded in, where each one's compressed blocks sit, and whether the file
//! holds all of them. Read from the raw file, so listing a folder needs no
//! decompression.
//!
//! After the frame index (see [`crate::format`]), as observed in Y11S3:
//!
//! ```text
//! 12 zero bytes, u32 n                        stream list
//! n x { u32 ?, u32 ?, u32 name hash, u32 first frame, u32 last frame, u8 0, u32 id }
//! u32 n                                       stream directory
//! n x { u32 id, u64 offset, u64 size, u64 size, u32 0, u32 padding }
//! n x { u32 1, blocks }                       each stream's opening snapshot
//! u32 1, u64 0, u32 main id, u32 1,           main-stream descriptor, then one
//!   { u32 id, u64 offset, u64 size, ... }     directory entry for the main stream
//! u32 1, blocks                               main stream, to the end of the file
//!
//! block: "CMPRV002" (stored as a u64), u32 raw size, u32 packed size, zstd frame
//! ```
//!
//! The main stream holds every stream's frame records (see
//! [`crate::records`]). Snapshots come in stream-list order; the directory
//! is not needed to read them, which matters because files the game failed
//! to finish carry uninitialized memory there.
//!
//! Stream ids come from one counter per game process: a round takes the main
//! id and the next `n`, and the next round recorded starts right after. A
//! skipped id is a recording that was started and never saved.

use serde::Serialize;

use crate::census::hex;

/// Start of every compressed block: `CMPRV002` written as a little-endian
/// u64.
pub const BLOCK_MAGIC: &[u8; 8] = b"200VRPMC";
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
/// A stream's data starts with this u32.
const RUN_PREFIX: [u8; 4] = [1, 0, 0, 0];
const LIST_ENTRY: usize = 25;
const DIRECTORY_ENTRY: usize = 36;
const DESCRIPTOR: usize = 56;
/// More streams than this means the list is not a stream list.
const MAX_STREAMS: u32 = 64;
/// A block's packed size when the game failed to compress it.
const FAILED_BLOCK: u32 = u32::MAX;

/// Streams whose role is known, by name hash in file byte order.
pub const STREAM_NAMES: &[([u8; 4], &str)] = &[
    // Clock, kill feed, health, picks: every packet this parser decodes.
    ([0xA9, 0x8F, 0xDD, 0x0B], "state"),
    // Every movement message (`607385fe`).
    ([0x20, 0xA5, 0xC4, 0xE3], "movement"),
];

pub fn stream_name(hash: [u8; 4]) -> Option<&'static str> {
    STREAM_NAMES.iter().find(|n| n.0 == hash).map(|n| n.1)
}

/// What the container of a Y8S4+ file holds.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Container {
    /// Every byte belongs to a known structure, and every stream's snapshot
    /// and the main stream are there. False for files the game did not
    /// finish writing.
    pub complete: bool,
    /// The main stream's id. The game numbers streams with one counter per
    /// process, so this orders the rounds a game session recorded; 0 is the
    /// first recording after the game started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_id: Option<u32>,
    pub streams: Vec<StreamInfo>,
    /// The main stream's blocks: every stream's frame records.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main: Option<BlockRun>,
    pub directory: DirectoryState,
    /// Compressed blocks in the file, and their sizes.
    pub blocks: usize,
    pub packed_bytes: u64,
    pub raw_bytes: u64,
    /// Where the file stops making sense, when it is incomplete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated_at: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// `(start, end)` of each block's zstd frame, in file order.
    #[serde(skip)]
    pub frames: Vec<(usize, usize)>,
}

/// One stream a round was recorded in.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub id: u32,
    /// Name hash, hex in file byte order.
    pub hash: String,
    /// The stream's role, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
    /// Frames the stream covers; it starts later than frame 0 when it was
    /// created during the round.
    pub first_frame: u32,
    pub last_frame: u32,
    /// Its opening snapshot's blocks. Absent when the file ends first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<BlockRun>,
    /// Frame records in the main stream (full and partial reads).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records: Option<u32>,
    /// Payload bytes of those records.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_bytes: Option<u64>,
    #[serde(skip)]
    pub name_hash: [u8; 4],
}

/// Consecutive compressed blocks holding one stream's data.
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BlockRun {
    /// Where the data starts in the file.
    pub offset: u64,
    pub blocks: usize,
    pub packed_bytes: u64,
    pub raw_bytes: u64,
}

/// Whether the stream directory describes the data that follows it.
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DirectoryState {
    #[default]
    Valid,
    /// Offsets or sizes that do not match the data. Seen only in files the
    /// game did not finish, where it holds uninitialized memory.
    Damaged,
    /// The file ends before it.
    Missing,
    /// Not read: the stream list before it was not recognised.
    NotRead,
}

struct Reader<'a> {
    raw: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn u32(&mut self) -> Option<u32> {
        let v = self.raw.get(self.pos..self.pos + 4)?;
        self.pos += 4;
        Some(u32::from_le_bytes(v.try_into().expect("4 bytes")))
    }

    fn u64(&mut self) -> Option<u64> {
        let v = self.raw.get(self.pos..self.pos + 8)?;
        self.pos += 8;
        Some(u64::from_le_bytes(v.try_into().expect("8 bytes")))
    }

    fn at(&self, pattern: &[u8]) -> bool {
        self.raw
            .get(self.pos..)
            .is_some_and(|rest| rest.starts_with(pattern))
    }
}

/// One entry of the stream directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DirectoryEntry {
    id: u32,
    offset: u64,
    size: u64,
}

fn directory_entry(r: &mut Reader) -> Option<DirectoryEntry> {
    let id = r.u32()?;
    let offset = r.u64()?;
    let size = r.u64()?;
    let size2 = r.u64()?;
    r.pos += 8; // u32 0, then four bytes of padding
    (size == size2).then_some(DirectoryEntry { id, offset, size })
}

/// Maps the container of a Y8S4+ file. `list_start` is where the stream list
/// begins: just past the frame index.
pub fn map(raw: &[u8], list_start: usize) -> Container {
    let mut c = Container::default();
    let mut r = Reader {
        raw,
        pos: list_start,
    };

    // Stream list.
    let reserved = raw.get(list_start..list_start + 12);
    if reserved.is_none() {
        c.directory = DirectoryState::Missing;
        c.truncated_at = Some(raw.len() as u64);
        c.warnings
            .push("the file ends before the stream list".into());
        return c;
    }
    if reserved != Some(&[0; 12][..]) {
        c.warnings
            .push("the 12 bytes before the stream list are not zero".into());
    }
    r.pos += 12;
    let n = r.u32().unwrap_or(0);
    if n == 0 || n > MAX_STREAMS {
        c.directory = DirectoryState::NotRead;
        c.warnings
            .push(format!("stream list claims {n} streams; not mapped"));
        return c;
    }
    let list_end = r.pos + n as usize * LIST_ENTRY;
    let Some(list) = raw.get(r.pos..list_end) else {
        c.directory = DirectoryState::Missing;
        c.truncated_at = Some(raw.len() as u64);
        c.warnings
            .push("the file ends inside the stream list".into());
        return c;
    };
    for e in list.as_chunks::<LIST_ENTRY>().0 {
        let u = |at: usize| u32::from_le_bytes(e[at..at + 4].try_into().expect("4 bytes"));
        let name_hash: [u8; 4] = e[8..12].try_into().expect("4 bytes");
        if e[20] != 0 {
            c.warnings.push(format!(
                "stream {} has flag byte {} (always 0 so far)",
                u(21),
                e[20]
            ));
        }
        c.streams.push(StreamInfo {
            id: u(21),
            hash: hex(&name_hash),
            name: stream_name(name_hash),
            first_frame: u(12),
            last_frame: u(16),
            name_hash,
            ..StreamInfo::default()
        });
    }
    r.pos = list_end;

    // Directory. Its space is reserved even when the game never fills it in,
    // so the data after it starts at the same place either way.
    let data_start = list_end + 4 + n as usize * DIRECTORY_ENTRY;
    let directory: Option<Vec<DirectoryEntry>> = (r.u32() == Some(n))
        .then(|| (0..n).map(|_| directory_entry(&mut r)).collect())
        .flatten();
    if raw.len() < data_start {
        c.directory = DirectoryState::Missing;
        c.truncated_at = Some(raw.len() as u64);
        c.warnings
            .push("the file ends inside the stream directory".into());
        return c;
    }

    // The data, walked block by block.
    r.pos = data_start;
    let mut runs: Vec<BlockRun> = Vec::new();
    let mut main_entry: Option<DirectoryEntry> = None;
    let mut main_id: Option<u32> = None;
    let mut in_main = false;
    loop {
        if r.pos == raw.len() {
            break;
        }
        if r.at(&RUN_PREFIX) && raw.get(r.pos + 4..r.pos + 12) == Some(BLOCK_MAGIC) {
            if in_main && c.main.is_some() {
                c.warnings.push(format!(
                    "a stream starts at {} inside the main stream",
                    r.pos
                ));
                c.truncated_at = Some(r.pos as u64);
                break;
            }
            let run = BlockRun {
                offset: r.pos as u64,
                ..BlockRun::default()
            };
            if in_main {
                c.main = Some(run);
            } else {
                runs.push(run);
            }
            r.pos += 4;
        } else if r.at(&RUN_PREFIX)
            && raw.get(r.pos + 4..r.pos + 12) == Some(&[0; 8][..])
            && !in_main
        {
            let descriptor = &mut Reader {
                raw,
                pos: r.pos + 12,
            };
            let id = descriptor.u32();
            let count = descriptor.u32();
            let entry = directory_entry(descriptor);
            if raw.len() < r.pos + DESCRIPTOR || count != Some(1) || entry.map(|e| e.id) != id {
                c.warnings
                    .push(format!("unrecognised main-stream descriptor at {}", r.pos));
                c.truncated_at = Some(r.pos as u64);
                break;
            }
            main_id = id;
            main_entry = entry;
            in_main = true;
            r.pos += DESCRIPTOR;
            continue;
        } else if !r.at(BLOCK_MAGIC) || (runs.is_empty() && c.main.is_none()) {
            c.warnings.push(format!(
                "{} bytes at {} are not a known structure",
                raw.len() - r.pos,
                r.pos
            ));
            c.truncated_at = Some(r.pos as u64);
            break;
        }
        // One block.
        let at = r.pos;
        r.pos += 8;
        let (Some(raw_size), Some(packed)) = (r.u32(), r.u32()) else {
            c.warnings
                .push(format!("the file ends inside the block header at {at}"));
            c.truncated_at = Some(at as u64);
            break;
        };
        let frame_end = r.pos.checked_add(packed as usize);
        if packed == FAILED_BLOCK || frame_end.is_none_or(|end| end > raw.len()) {
            c.warnings.push(if packed == FAILED_BLOCK {
                format!(
                    "the game failed to write a block of {raw_size} bytes at {at} \
                     (packed size 0xFFFFFFFF) and stopped there"
                )
            } else {
                format!(
                    "the block at {at} needs {packed} bytes but the file has {}",
                    raw.len() - r.pos
                )
            });
            c.truncated_at = Some(at as u64);
            break;
        }
        let end = frame_end.expect("checked");
        if !r.at(&ZSTD_MAGIC) {
            c.warnings
                .push(format!("the block at {at} does not hold a zstd frame"));
            c.truncated_at = Some(at as u64);
            break;
        }
        c.frames.push((r.pos, end));
        let run = if in_main {
            c.main.as_mut()
        } else {
            runs.last_mut()
        };
        if let Some(run) = run {
            run.blocks += 1;
            run.packed_bytes += 16 + u64::from(packed);
            run.raw_bytes += u64::from(raw_size);
        }
        c.blocks += 1;
        c.packed_bytes += 16 + u64::from(packed);
        c.raw_bytes += u64::from(raw_size);
        r.pos = end;
    }

    // A walk that reached the end without every stream's data means the file
    // ends early, right there.
    if c.truncated_at.is_none() && (c.main.is_none() || runs.len() < c.streams.len()) {
        c.truncated_at = Some(raw.len() as u64);
    }

    // Snapshots come in stream-list order.
    if runs.len() > c.streams.len() {
        c.warnings.push(format!(
            "{} snapshots for {} streams",
            runs.len(),
            c.streams.len()
        ));
    }
    for (s, run) in c.streams.iter_mut().zip(&runs) {
        s.snapshot = Some(*run);
    }

    c.directory = match &directory {
        Some(entries) if directory_matches(entries, &c.streams) => DirectoryState::Valid,
        _ => DirectoryState::Damaged,
    };
    if c.directory == DirectoryState::Damaged {
        c.warnings.push(
            "the stream directory does not describe the data (uninitialized in files the game did not finish); \
             snapshots were matched to streams in list order"
                .into(),
        );
    }
    if let (Some(e), Some(main)) = (main_entry, c.main) {
        let expected = raw.len() as u64 - main.offset;
        if e.offset != main.offset || e.size != expected {
            c.warnings.push(format!(
                "main-stream descriptor says {} bytes at {}, the file has {expected} at {}",
                e.size, e.offset, main.offset
            ));
        }
    }

    // Stream ids are consecutive after the main id.
    let first_id = c.streams.iter().map(|s| s.id).min();
    c.recording_id = main_id.or_else(|| first_id.and_then(|i| i.checked_sub(1)));
    let mut ids: Vec<u32> = c.streams.iter().map(|s| s.id).collect();
    ids.sort_unstable();
    let consecutive = ids.windows(2).all(|w| w[0].checked_add(1) == Some(w[1]));
    if !consecutive || (main_id.is_some() && first_id.map(|i| i.wrapping_sub(1)) != main_id) {
        c.warnings.push(format!(
            "stream ids {ids:?} do not follow main stream id {main_id:?} one by one"
        ));
    }

    let missing_snapshots = c.streams.iter().filter(|s| s.snapshot.is_none()).count();
    if missing_snapshots > 0 {
        c.warnings.push(format!(
            "{missing_snapshots} of {} streams have no snapshot",
            c.streams.len()
        ));
    }
    if c.main.is_none() && c.truncated_at.is_some() {
        c.warnings
            .push("the main stream, which holds every frame record, is missing".into());
    }
    c.complete = c.truncated_at.is_none() && missing_snapshots == 0 && c.main.is_some();
    c
}

/// Whether every directory entry points at its stream's snapshot.
fn directory_matches(entries: &[DirectoryEntry], streams: &[StreamInfo]) -> bool {
    entries.len() == streams.len()
        && entries.iter().all(|e| {
            streams.iter().any(|s| {
                s.id == e.id
                    && s.snapshot
                        .is_some_and(|run| run.offset == e.offset && run.packed_bytes + 4 == e.size)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a container: stream list, directory, snapshots, main stream.
    struct Builder {
        streams: Vec<(u32, [u8; 4], Vec<Vec<u8>>)>,
        main_id: u32,
        main: Vec<Vec<u8>>,
    }

    fn block(out: &mut Vec<u8>, data: &[u8]) {
        let packed = zstd::bulk::compress(data, 1).unwrap();
        out.extend(BLOCK_MAGIC);
        out.extend((data.len() as u32).to_le_bytes());
        out.extend((packed.len() as u32).to_le_bytes());
        out.extend(packed);
    }

    impl Builder {
        fn new() -> Self {
            Builder {
                streams: vec![
                    (
                        8,
                        [0xA9, 0x8F, 0xDD, 0x0B],
                        vec![b"state snapshot".to_vec()],
                    ),
                    (
                        9,
                        [1, 2, 3, 4],
                        vec![b"first half".to_vec(), b"second half".to_vec()],
                    ),
                ],
                main_id: 7,
                main: vec![vec![0x11; 3000], vec![0x22; 100]],
            }
        }

        /// The file bytes and where the stream list starts.
        fn build(&self) -> (Vec<u8>, usize) {
            let mut out = b"header and index".to_vec();
            let list_start = out.len();
            out.extend([0; 12]);
            out.extend((self.streams.len() as u32).to_le_bytes());
            for (id, hash, _) in &self.streams {
                out.extend(0x0012_3456u32.to_le_bytes());
                out.extend(0x0065_4321u32.to_le_bytes());
                out.extend(hash);
                out.extend(0u32.to_le_bytes());
                out.extend(99u32.to_le_bytes());
                out.push(0);
                out.extend(id.to_le_bytes());
            }
            let directory_at = out.len();
            out.extend(vec![0; 4 + self.streams.len() * DIRECTORY_ENTRY]);
            let mut entries = Vec::new();
            for (id, _, blocks) in &self.streams {
                let at = out.len();
                out.extend(RUN_PREFIX);
                for b in blocks {
                    block(&mut out, b);
                }
                entries.push((*id, at as u64, (out.len() - at) as u64));
            }
            let mut d = (self.streams.len() as u32).to_le_bytes().to_vec();
            for (id, at, size) in entries {
                d.extend(id.to_le_bytes());
                d.extend(at.to_le_bytes());
                d.extend(size.to_le_bytes());
                d.extend(size.to_le_bytes());
                d.extend([0, 0, 0, 0, 0x45, 0x02, 0, 0]);
            }
            out[directory_at..directory_at + d.len()].copy_from_slice(&d);
            let mut main = RUN_PREFIX.to_vec();
            for b in &self.main {
                block(&mut main, b);
            }
            out.extend(RUN_PREFIX);
            out.extend([0; 8]);
            out.extend(self.main_id.to_le_bytes());
            out.extend(1u32.to_le_bytes());
            out.extend(self.main_id.to_le_bytes());
            // The rest of the descriptor: offset, size, size, 0 and padding.
            let main_at = (out.len() + 32) as u64;
            out.extend(main_at.to_le_bytes());
            out.extend((main.len() as u64).to_le_bytes());
            out.extend((main.len() as u64).to_le_bytes());
            out.extend([0; 8]);
            out.extend(main);
            (out, list_start)
        }
    }

    #[test]
    fn maps_a_complete_file() {
        let (raw, list_start) = Builder::new().build();
        let c = map(&raw, list_start);
        assert!(c.complete, "{:?}", c.warnings);
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
        assert_eq!(c.recording_id, Some(7));
        assert_eq!(c.directory, DirectoryState::Valid);
        assert_eq!(c.streams.len(), 2);
        assert_eq!(c.streams[0].name, Some("state"));
        assert_eq!(c.streams[0].hash, "a98fdd0b");
        assert_eq!(c.streams[1].snapshot.unwrap().blocks, 2);
        assert_eq!(c.streams[1].snapshot.unwrap().raw_bytes, 21);
        let main = c.main.unwrap();
        assert_eq!((main.blocks, main.raw_bytes), (2, 3100));
        assert_eq!(c.blocks, 5);
        assert_eq!(c.frames.len(), 5);
        assert!(
            c.frames
                .iter()
                .all(|&(s, _)| raw[s..].starts_with(&ZSTD_MAGIC))
        );
    }

    #[test]
    fn a_directory_of_garbage_is_damaged_but_the_data_still_maps() {
        let (mut raw, list_start) = Builder::new().build();
        let directory = list_start + 16 + 2 * LIST_ENTRY;
        raw[directory..directory + 4 + 2 * DIRECTORY_ENTRY].fill(0x45);
        let c = map(&raw, list_start);
        assert_eq!(c.directory, DirectoryState::Damaged);
        assert!(c.complete);
        assert_eq!(c.streams[1].snapshot.unwrap().blocks, 2);
    }

    #[test]
    fn a_block_the_game_failed_to_write_ends_the_file() {
        let (raw, list_start) = Builder::new().build();
        // Cut after the snapshots and add a header with a failed packed size,
        // as the game leaves it.
        let full = map(&raw, list_start);
        let cut = full.main.unwrap().offset as usize - DESCRIPTOR;
        let mut truncated = raw[..cut].to_vec();
        truncated.extend(BLOCK_MAGIC);
        truncated.extend(10_822_076u32.to_le_bytes());
        truncated.extend(u32::MAX.to_le_bytes());
        let c = map(&truncated, list_start);
        assert!(!c.complete);
        assert_eq!(c.truncated_at, Some(cut as u64));
        assert!(c.main.is_none());
        assert!(
            c.warnings.iter().any(|w| w.contains("10822076")),
            "{:?}",
            c.warnings
        );
        // The snapshots before it are still there.
        assert!(c.streams.iter().all(|s| s.snapshot.is_some()));
        assert_eq!(c.recording_id, Some(7), "falls back to the stream ids");
    }

    #[test]
    fn a_file_ending_after_its_snapshots_says_where_it_ends() {
        let (raw, list_start) = Builder::new().build();
        let full = map(&raw, list_start);
        let cut = full.main.unwrap().offset as usize - DESCRIPTOR;
        let c = map(&raw[..cut], list_start);
        assert!(!c.complete);
        assert_eq!(c.truncated_at, Some(cut as u64));
        assert!(
            c.warnings.iter().any(|w| w.contains("main stream")),
            "{:?}",
            c.warnings
        );
    }

    #[test]
    fn a_file_cut_inside_a_block_is_incomplete() {
        let (raw, list_start) = Builder::new().build();
        let c = map(&raw[..raw.len() - 5], list_start);
        assert!(!c.complete);
        assert!(c.truncated_at.is_some());
    }

    #[test]
    fn a_stream_list_that_is_not_one_is_not_mapped() {
        let (mut raw, list_start) = Builder::new().build();
        raw[list_start + 12..list_start + 16].copy_from_slice(&u32::MAX.to_le_bytes());
        let c = map(&raw, list_start);
        assert!(!c.complete);
        assert!(c.streams.is_empty());
        assert_eq!(c.directory, DirectoryState::NotRead);
        assert_eq!(c.warnings.len(), 1, "{:?}", c.warnings);
    }

    #[test]
    fn garbage_stream_ids_are_reported_not_overflowed() {
        let mut b = Builder::new();
        b.streams[0].0 = u32::MAX;
        b.streams[1].0 = u32::MAX;
        b.main_id = u32::MAX;
        let (raw, list_start) = b.build();
        let c = map(&raw, list_start);
        assert!(
            c.warnings.iter().any(|w| w.contains("stream ids")),
            "{:?}",
            c.warnings
        );
    }

    #[test]
    fn a_file_ending_at_the_index_has_no_container() {
        let (raw, list_start) = Builder::new().build();
        let c = map(&raw[..list_start], list_start);
        assert!(!c.complete);
        assert_eq!(c.directory, DirectoryState::Missing);
    }
}
