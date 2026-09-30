# Replay provenance and container diagnostics Implementation Plan

> **To execute:** use the `executing-plans` skill. Steps use `- [x]` for tracking.

**Goal:** Every round says where it came from, how complete its file is, and how far each output
can be trusted, with each fact decoded from the bytes of real `.rec` files rather than assumed.

**Architecture:** A new `container` module maps the raw file (stream list, directory, CMPRV002
blocks) without decompressing, so header-only listings can spot truncated and damaged files. A
`records` module maps the decompressed data into per-stream, frame-numbered records, which turns
any packet offset into a frame and, through the frame index, into seconds since recording start.
Folder and library scans add real file-name parsing, game-session (process id) grouping via the
per-process stream-id counter, and `.tmprec` reporting.

**Tech stack:** Rust 2024, existing crates only (zstd 0.14, sha2 0.11, chrono 0.4, serde,
rayon, memchr, aho-corasick). Python 3.13 is available for byte-level probing only (scratchpad,
never committed).

## Global Constraints

- No new dependencies.
- JSON keys camelCase; new optional fields use `skip_serializing_if` like their neighbours.
- `ReadMode::Header` must not decompress Y8S4+ files (`round/header` bench stays < 1 ms).
- Never write to the game install (`D:/SteamLibrary/steamapps/common/Tom Clancy's Rainbow Six Siege`).
- Never commit replay files other than the existing `test_recordings/`. Real-data tests read
  `R6_MATCH_REPLAY` and skip when it is unset.
- Existing tests stay green; `cargo clippy --all-targets` stays warning-free.
- A decoding change bumps `decoderRevision` of every profile whose output changes (`src/decoder.rs`).

## Reference: the Y11S3 container, as decoded from 10 test and 175 real files

All little-endian. Offsets from file start; verified to account for every byte of every
complete file.

```text
"dissect\0"
u32 formatVersion                          8 (Y8S4+)
str label                                  "UNKNOWN"          str = u64 length + bytes
u32 0 | u32 lastFrame | u32 propertyCount | u32 0
propertyCount x (str key, str value)
-- frame index --
u32 ? | u32 ? | u32 frames                 the two u32 are random per round
frames x (u32 index, f64 seconds)
-- stream list --
12 zero bytes | u32 n                      n = 10 (154 files), 11 (20), 8 (1)
n x { u32 ?, u32 ?, u32 nameHash, u32 firstFrame, u32 lastFrame, u8 0, u32 streamId }   25 bytes
-- stream directory --
u32 n
n x { u32 streamId, u64 offset, u64 size, u64 size, u32 0, u32 padding }               36 bytes
   (padding = high half of a heap pointer, constant per game process; ignore)
-- per-stream data, one per listed stream, at `offset` --
u32 1 | blocks
-- main-stream descriptor (56 bytes) --
u32 1 | u64 0 | u32 mainId | u32 1 | { u32 mainId, u64 offset, u64 size, u64 size, u32 0, u32 padding }
-- main stream, at offset, `size` bytes to end of file --
u32 1 | blocks
block = "CMPRV002" (bytes 32 30 30 56 52 50 4d 43) | u32 rawSize | u32 packedSize | zstd frame (packedSize bytes)
```

Decompressed (concatenation of all blocks in file order, what `Decompressed::data` holds):

```text
per listed stream (file order):  u64 length | snapshot payload        (the "side stream")
main stream:
  u32 firstFrame (0) | u32 lastFrame | u32 n | u32 0
  n x sub-stream:
    u32 nameHash | u32 ? | u32 ? | u8 0 | u32 count | u32 0
    count x { u32 frame | u32 size | u32 0 | payload[size] }   frames strictly increasing
```

Stream roles found so far: `0bdd8fa9` state (clock, feed, health, picks: every marker the parser
reads), `e3c4a520` movement (every `607385fe` movement message). Both, plus `d354fe63`, carry a
record at ~28.5/s (35 ms median, max 46–142 ms). Others are event-driven.

Stream ids come from one counter per game process: a round uses `mainId` and `mainId+1..=mainId+n`,
and the next saved round starts at `mainId + 1 + n`. A skipped id means a recording that was
started and never saved (seen once: session 25108 after its abandoned match).

Damage seen in real data (3 of 175 rounds): the file ends on a block header whose `packedSize` is
`0xFFFFFFFF` (the game's compressor failed on a 5–11 MB raw block), the main stream is missing,
and the directory holds uninitialized memory. The frame index is still complete.

Real names: folder `Match-YYYY-MM-DD_HH-MM-SS-<pid>` (local time the match was created, Windows
process id, a multiple of 4), rounds `<folder>-R<NN>.rec`. Player reports name temporary files
`P<pid>_<streamId>_Y<yyyy>_M<m>_D<d>_H<h>_M<mm>_<Kind>.tmprec` with `Kind` in `FrameDataStream`,
`StaticData`, `StreamInfo`; the install has an empty `DissectTmp/` next to `MatchReplay/`.

Sample rates: spectator recordings (custom test files) index frames at a fixed ~29.4/s (33–35 ms);
player recordings at the client frame rate (~300/s, 0.1–66 ms). Data records arrive at ~28.5/s in
both.

---

### Task 1: Header strings are u64-length-prefixed

**Files:**
- Modify: `src/header.rs` (`read_string`, `STRING_SEPARATOR`), `src/format.rs` (`read_prelude`)
- Test: `src/header.rs` (new `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `header::read_string(c: &mut Cursor) -> Result<String>` accepting any u64 length that
  fits the remaining data; `Error::InvalidStringSeparator` kept for lengths that do not fit.

- [x] **Step 1: Write the failing test** — a header property whose value is 300 bytes long:

```rust
#[test]
fn reads_values_longer_than_255_bytes() {
    let mut data = Vec::new();
    let mut put = |s: &[u8]| {
        data.extend((s.len() as u64).to_le_bytes());
        data.extend(s);
    };
    put(b"additionaltags");
    put(&[b'x'; 300]);
    let mut c = Cursor::new(&data, 0);
    assert_eq!(read_string(&mut c).unwrap(), "additionaltags");
    assert_eq!(read_string(&mut c).unwrap().len(), 300);
}
```

- [x] **Step 2: Run** `cargo test --lib header::tests` — expect FAIL (`InvalidStringSeparator`).
- [x] **Step 3: Implement** — read `u64::from_le_bytes(c.array()?)`; reject when it exceeds
  `data.len() - pos` with `InvalidStringSeparator(pos)`; same for the prelude label.
- [x] **Step 4: Run** all tests — expect PASS.

### Task 2: Container map from the raw file

**Files:**
- Create: `src/container.rs`
- Modify: `src/lib.rs` (module + re-export), `src/decompress.rs` (return the map),
  `src/round.rs` (`Round.container`, `ReplayInfo.container`, decode report field `container`),
  `src/decoder.rs` (revision bump, see Task 7)
- Test: `src/container.rs` unit tests with synthetic files; `tests/replays.rs`

**Interfaces:**
- Consumes: `format::read_frame_index` end offset (`12 + 12 * frames` bytes after the header).
- Produces:

```rust
pub struct Container {
    /// Every byte of the file belongs to a known structure and every listed stream is present.
    pub complete: bool,
    /// The main stream's id: a per-game-process counter, 0 for the first recording.
    pub recording_id: Option<u32>,
    pub streams: Vec<StreamInfo>,
    /// CMPRV002 blocks in the file, their packed and raw sizes.
    pub blocks: usize, pub packed_bytes: u64, pub raw_bytes: u64,
    pub directory: DirectoryState,           // Valid | Damaged | Missing
    pub warnings: Vec<String>,
    #[serde(skip)] pub truncated_at: Option<u64>,
}
pub struct StreamInfo {
    pub id: u32, pub hash: String /* 8 hex, stream order */, pub name: Option<&'static str>,
    pub first_frame: u32, pub last_frame: u32,
    pub blocks: usize, pub packed_bytes: u64, pub raw_bytes: u64,
    // filled by Task 3:
    pub snapshot_bytes: Option<u64>, pub records: Option<u32>, pub record_bytes: Option<u64>,
}
pub fn map(raw: &[u8], after_index: usize) -> Option<Container>;  // None: not this layout
pub const STREAM_NAMES: &[(u32, &str)] = &[(0x0bdd8fa9, "state"), (0xe3c4a520, "movement")];
```

- [x] **Step 1: Write failing unit tests** in `container.rs` using a builder that writes the
  layout above with `zstd::bulk::compress`: (a) a complete two-stream file maps with
  `complete`, the right ids, block counts and sizes; (b) the same file with the directory
  overwritten by `0x45` bytes reports `Damaged` and still walks the streams; (c) truncating it
  inside the main stream and ending on a block header with packed size `u32::MAX` reports
  `complete: false` and a warning naming the raw size; (d) an unknown stream-list count
  (e.g. `u32::MAX`) returns a map with a warning rather than panicking.
- [x] **Step 2: Run** `cargo test --lib container` — expect FAIL (module missing).
- [x] **Step 3: Implement** `map`: parse list and directory; validate each directory entry
  (offset in range, `u32 1` + `CMPRV002` there, size matching the walked blocks); walk the data
  sequentially from the end of the directory regardless of its state (prefix `u32 1`, blocks,
  main descriptor, main blocks to EOF); every block payload must start with the zstd magic and
  fit the file. Warn on: format version not 8, label not `UNKNOWN`, non-zero reserved fields,
  stream-list and directory counts differing, a stream in the list with no data, bytes left
  over, unknown stream hashes are *not* warnings (they go to the census).
- [x] **Step 4: Wire** into `decompress::decompress` and `header_only` (chunked layout only; the
  stream layout keeps `container: None`), `Round.container`, `ReplayInfo`, and a decode-report
  field `container`: `Decoded` when complete, `Partial` when damaged but complete, `Missing` when
  truncated. When truncated, every packet-derived field that is `Missing` gets the warning
  "the file is truncated: the game did not write the round's frame data".
- [x] **Step 5: Real-file tests** in `tests/replays.rs`: every test file maps `complete` with
  10 streams, `recording_id` = `0x1a5` for `custom_1.rec` and consecutive files step by 11;
  a copy of `custom_1.rec` cut after its tenth stream, plus a `CMPRV002` header with
  `u32::MAX`, reports `complete: false` in both `ReadMode::Header` and `Full`, and the full read's
  `kills` field carries the truncation warning.
- [x] **Step 6: Run** all tests — expect PASS.

### Task 3: Frame records and calibrated timing

**Files:**
- Create: `src/records.rs`
- Modify: `src/round.rs` (build the map after decompression; `packet_at` for events),
  `src/feedback.rs` (`MatchUpdate.offset`, `recording_time`), `src/format.rs` (`Timing` fields),
  `src/timeline.rs` (`PhaseSpan.recording_start/end`), `src/container.rs` (fill per-stream counts)
- Test: `src/records.rs` unit tests; `tests/replays.rs`

**Interfaces:**
- Consumes: `Container` (stream count and order), `FrameIndex.times`.
- Produces:

```rust
pub struct RecordMap {
    pub first_frame: u32, pub last_frame: u32,
    pub streams: Vec<SubStream>,              // main-stream order
    /// Offset in `data` where the main stream starts (side streams end).
    pub main_start: usize,
    /// (payload start, payload end, stream index, frame), sorted by start.
    spans: Vec<(usize, usize, u16, u32)>,
}
pub struct SubStream { pub hash: u32, pub records: u32, pub bytes: u64, pub frames: Vec<u32>,
                       pub snapshot_bytes: u64 }
impl RecordMap {
    pub fn parse(data: &[u8], streams: usize) -> Option<RecordMap>;  // None unless exact fit
    pub fn frame_at(&self, offset: usize) -> Option<u32>;           // None in snapshots
}
```

  `Timing` gains `mean_rate: f64`, `data_rate: Option<f64>` (records/s of the state stream),
  `holes: Vec<Hole>` (`{ stream, at, seconds }`: gaps > max(0.5 s, 10 x median) in streams with
  a median record interval under 0.1 s), `prep_started_at: Option<f64>` (recording seconds of
  the first clock tick). `MatchUpdate` and `PhaseSpan` gain `recording_time` /
  `recording_start`+`recording_end`: seconds since recording start, 3 decimals.

- [x] **Step 1: Failing unit tests** in `records.rs`: a synthetic decompressed body with two
  snapshots and two sub-streams parses; `frame_at` returns the record's frame for an offset in
  its payload and `None` in a snapshot; a body one byte short returns `None`.
- [x] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** — PASS.
- [x] **Step 5: Wire**: in `Parser::run` build the map when `self.round.container` is complete;
  record `self.packet_at` before each dispatch; `update()` copies it into `MatchUpdate.offset`;
  after `place_feedback`, set `recording_time = times[frame_at(offset)]`; clock ticks record the
  offset of their first reading so phase spans get recording times; compute holes and rates.
- [x] **Step 6: Real-file tests**: every test file maps with 10 sub-streams; `data_rate` in
  25..32; feed entries have non-decreasing `recording_time` within a tick order; the kill
  closest to the end of the round lies within 1 s of `phases[End].recording_start`; no holes.
- [x] **Step 7: Run** — PASS.

### Task 4: Round files, match folders and temporary files by their real names

**Files:**
- Modify: `src/file.rs`, `src/matches.rs`, `src/error.rs` (none expected)
- Test: `src/file.rs`, `src/matches.rs` unit tests; `tests/replays.rs`

**Interfaces:**
- Produces:

```rust
pub fn round_from_file_name(path: &Path) -> Option<u32>;     // R01.rec and Match-...-R01.rec
pub struct FolderName { pub local_time: String /* 2026-09-20T00:32:29 */, pub process_id: u32 }
pub fn parse_folder_name(name: &str) -> Option<FolderName>;
pub struct TempRecording { pub file: String, pub process_id: Option<u32>, pub stream_id: Option<u32>,
                           pub local_time: Option<String>, pub kind: Option<String> }
pub fn parse_temp_name(name: &str) -> TempRecording;
```

  `FolderReport` gains `name: Option<FolderName>`, `recordings: Vec<RecordingRef>`
  (`{ file, round, recordingId, streams }`), `unsaved_recordings: Vec<UnsavedGap>`
  (`{ after: file, before: file, ids: u32 }`), `truncated: Vec<String>`, and `temporary:
  Vec<TempRecording>` replacing the plain skip reason for `.tmprec` files (they stay in
  `skipped` too).

- [x] **Step 1: Failing unit tests**: `round_from_file_name("Match-2026-09-20_00-32-29-13160-R06.rec") == Some(6)`;
  `parse_folder_name("Match-2026-09-20_00-32-29-13160")` gives local time and pid 13160;
  `parse_temp_name("P15440_50_Y2022_M1_D16_H23_M58_FrameDataStream.tmprec")` gives pid 15440,
  stream 50, `2022-01-16T23:58`, kind `FrameDataStream`; an unrelated name gives all `None`.
  `analyze()` on rounds with recording ids 0 (10 streams) and 12 reports one unsaved id.
- [x] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** — PASS.
- [x] **Step 5: Real-data test** (skips without `R6_MATCH_REPLAY`): every folder's name parses,
  every file's name round equals its header round, and no folder warns about names.

### Task 5: Library scan: sessions, duplicates, leftovers

**Files:**
- Create: `src/library.rs`
- Modify: `src/lib.rs`, `src/main.rs` (`--list` prints a `Library`)
- Test: `src/library.rs` unit tests; `tests/replays.rs`

**Interfaces:**
- Consumes: `Match::open_with`, `find_match_folders`, `FolderReport` (Task 4).
- Produces:

```rust
pub struct Library {
    pub folders: Vec<ListedFolder>,          // what --list printed per folder before
    pub sessions: Vec<Session>,              // one per game process id
    pub duplicates: Vec<Duplicate>,          // same sha256, or same matchID + round, in 2+ places
    pub temporary: Vec<TempRecording>,       // .tmprec under the root and in ../DissectTmp
    pub warnings: Vec<String>,
}
pub struct Session { pub process_id: u32, pub folders: Vec<String>, pub first_recording_id: u32,
                     pub earlier_recordings: u32, pub unsaved_recordings: Vec<UnsavedGap> }
pub fn scan(root: &Path, mode: ReadMode) -> Result<Library>;
```

- [x] **Step 1: Failing unit test**: sessions built from three folders' recording ids (pid 7:
  ids 0..=10, 12..=22; pid 9: 156..=166) report one unsaved id in pid 7 and 14 earlier
  recordings for pid 9 (`156 / 11`, rounded down).
- [x] **Step 2: Run** — FAIL. **Step 3: Implement** (`DissectTmp` looked up as
  `root.parent()/DissectTmp` only when `root` is named `MatchReplay`). **Step 4: Run** — PASS.
- [x] **Step 5: Real-data test**: 30 folders, 9 sessions, exactly one unsaved recording (pid
  25108), 3 truncated rounds, no duplicates.

### Task 6: Builds, decoders and re-parse information

**Files:**
- Modify: `src/decoder.rs`, `src/main.rs` (`--decoders`), `src/format.rs` (none)
- Test: `src/decoder.rs`

**Interfaces:**
- Produces: `KNOWN_BUILDS: &[(u32, &str)]` (build, version string) with 9883691, 9901603,
  9918362 (`Y11S3_Alpha04`); `NEWEST_TESTED_BUILD = 9_918_362`; `ParserInfo.known_build: bool`;
  `decoder::table() -> DecoderTable` serialized by `--decoders`
  (`{ parser, parserVersion, newestTestedBuild, knownBuilds, profiles: [{ name, minBuild,
  maxBuild, revision, changes }] }`).
- [x] **Step 1: Failing test**: `ParserInfo::for_build(9_918_362).untested_build == false`,
  `for_build(9_918_363).untested_build == true`; `table().profiles` covers 0..=u32::MAX without
  holes.
- [x] **Step 2: Run** — FAIL. **Step 3: Implement** after Task 8's real-data run shows the two
  new builds decode fully. Bump `revision` of `Y8S2`, `Y9S1`, `Y9S1.3`, `Y9S3`, `Y9S4` (chunked
  layout: new container output). **Step 4: Run** — PASS.

### Task 7: Census of streams

**Files:**
- Modify: `src/census.rs`, `src/round.rs`
- Test: `tests/replays.rs` (`census_counts_known_and_unknown_fields`)

**Interfaces:**
- Consumes: `RecordMap`, `Container`.
- Produces: `Census.streams: Vec<StreamCount>` (`{ hash, name, id, records, recordBytes,
  snapshotBytes, firstFrame, lastFrame }`), `Census.unknown_streams: Vec<String>` (hashes with no
  name), `Census.streams_not_seen: Vec<&'static str>` (named streams absent).
- [x] **Step 1: Failing assertion**: for every test file `census.streams.len() == 10`,
  `state` and `movement` have records, `streams_not_seen` is empty.
- [x] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** — PASS.

### Task 8: Real-data verification, README, commit

**Files:**
- Modify: `README.md`, `tests/replays.rs`
- [x] **Step 1:** Full read of all 175 real rounds (`R6_MATCH_REPLAY` test and a CLI run):
  record per-build decode status; any `partial`/`missing` field outside the 3 truncated rounds
  is investigated before Task 6 marks a build tested.
- [x] **Step 2:** README: container layout (replace the "per-player table, CMPRV002 trailer"
  guess), sample rates (spectator vs player), new JSON keys, `--list` shape, `--decoders`,
  `.tmprec` evidence, truncated files.
- [x] **Step 3:** `cargo fmt`, `cargo clippy --all-targets`, `cargo test --release`, bench
  `round/header` unchanged within noise.
- [x] **Step 4:** Commit on a branch per `git-workflow`.

---

## Execution notes

All tasks done on `feat/replay-provenance`. Where execution departed from the plan:

- **Decompression uses the container map** (Task 2). Complete Y8S4+ files are decompressed block
  by block from the map instead of scanning for the zstd magic, which could match by chance
  inside the frame index or a damaged directory. Incomplete files fall back to the scan.
- **Holes come from the movement stream only** (Task 3). The planned rule (any stream with a
  median record gap under 0.1 s) flagged hundreds of false holes in bursty event streams. The
  movement stream has a record at every update (max gap 46-69 ms in all data), so gaps over
  0.5 s there are holes. No real or test round has one.
- **`Hole` has no `stream` field**, since only one stream is measured.
- **Decode trust rules recalibrated** (not planned). On real player recordings `trusted` was
  false in 174 of 175 rounds: `movement` counted the recorder's own body, which the game never
  links in their own recording; `players` flagged rounds a player had left; `defuserPlayers`
  flagged what Y11S3 does not record. After the change 156 of 167 real rounds are trusted and
  each untrusted one has a real fault.
- **Census**: fields name their stream; `unknownStreams` and `streamsNotSeen` replace the
  planned per-stream record table, which `replay.container.streams` already holds.
- **Every decoder profile's revision was bumped** (Task 6), not only Y8S2 onward: the
  `players` trust rule changes output for every version.
- **Found, not fixed** (pre-existing, outside this plan): Y11S3 plant completions are sometimes
  registered after the clock switches to the defuser timer, or missed (3 of 167 real rounds);
  match type 7 has no name; the README benchmark table predates the current code (full round
  ~30 ms on main and branch alike, not 141 ms).
