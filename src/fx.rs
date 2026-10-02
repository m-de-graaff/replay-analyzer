//! The effects of a round (Y11S3): what the `FXChannel` stream (`f5ee6a3d`)
//! spawns, with the parameters that place an effect and the moment it is
//! stopped. Read once per round by [`decode`]; the decoders of shots,
//! gadget statuses and areas read the [`Effects`] it returns, not the
//! stream.
//!
//! # The stream
//!
//! A record, and the stream's opening snapshot, is a `u8` mask and one
//! section per set bit, in this order and not in the order of the bits.
//! Each section is a `u32` count and its entries:
//!
//! ```text
//! 01 spawn       36 bytes: u64 asset, u64 parent, u32 instance, u32,
//!                          u64 target, u32
//! 04 int         12 bytes: u32 instance, hash, u32
//! 08 float       12 bytes: u32 instance, hash, f32
//! 10 vector      24 bytes: u32 instance, hash, 4 x f32
//! 20 quaternion  24 bytes: u32 instance, hash, 4 x f32 (x, y, z, w)
//! 02 stop         7 bytes: u32 instance, u8, u8, u8
//! 40 points      u32 instance, u32 capacity, u32 n, n x 4 x f32 (x, y, z, 1)
//! 80 sound       by its first byte: 01 = 34 bytes, 02 = 6 bytes,
//!                03 = 41 bytes, the type byte included
//! ```
//!
//! A spawn starts an instance of an effect asset. `parent` is the entity
//! or map object the effect is attached to and `target` the one it plays
//! on (the body a bullet struck); either is 0 when there is none. Entries
//! of the other sections name the instance they belong to, in the record
//! of the spawn or in a later one.
//!
//! # What is kept
//!
//! One [`Spawn`] per spawn entry, in stream order, with
//!
//! - `position`: the vector parameter `56 95 b5 31`, where the effect is
//!   in world metres (the first one written for the instance);
//! - `rotation`: the quaternion parameter `52 e9 e5 5e`;
//! - `alliance`: the int parameter `0a d9 18 33`, the alliance
//!   (`players[].alliance`) of whoever set the effect off;
//! - `points` and `capacity`: its point list, the cells of a fire, gas or
//!   swarm area. The game writes the whole list again as it grows, so the
//!   last one is kept, with the frame it was written in (`grown`);
//! - `stopped`: the frame of its first stop entry. An effect that plays
//!   out by itself (a bullet hit) gets one too; one still playing when the
//!   recording ends has none.
//!
//! Float parameters, the other int, vector and quaternion parameters and
//! the sound section are walked and not kept.
//!
//! Spawns of the snapshot have `frame: None`: they were playing when the
//! recording started. For the time of a frame see
//! [`crate::world::seconds`] and [`crate::world::World::when`].
//!
//! A record that does not hold what its counts promise is counted in
//! `unparsed`; what was read before the fault is kept. In the ten test
//! rounds and 175 real rounds (422,731 records) every record reads to its
//! last byte.

use std::collections::HashMap;

use rayon::prelude::*;

use crate::entities::Hash;
use crate::loadout::Input;

/// Name hash of the effects stream (`FXChannel`).
pub(crate) const STREAM: Hash = [0xF5, 0xEE, 0x6A, 0x3D];
/// The vector parameter that places an effect in the world.
pub(crate) const POSITION: Hash = [0x56, 0x95, 0xB5, 0x31];
/// The quaternion parameter that turns it.
pub(crate) const ROTATION: Hash = [0x52, 0xE9, 0xE5, 0x5E];
/// The int parameter with the alliance of whoever set it off.
pub(crate) const ALLIANCE: Hash = [0x0A, 0xD9, 0x18, 0x33];

/// Mask bits of the sections, in the order the sections are written.
const SPAWNS: u8 = 0x01;
const INTEGERS: u8 = 0x04;
const FLOATS: u8 = 0x08;
const VECTORS: u8 = 0x10;
const QUATERNIONS: u8 = 0x20;
const STOPS: u8 = 0x02;
const POINTS: u8 = 0x40;
const SOUNDS: u8 = 0x80;
/// Entry sizes of the sections with entries of one size.
const SPAWN_SIZE: usize = 36;
const PARAMETER_SIZE: usize = 12;
const VECTOR_SIZE: usize = 24;
const STOP_SIZE: usize = 7;
const POINT_SIZE: usize = 16;
/// Sizes of the sound entries after their type byte, by type 1, 2, 3.
const SOUND_SIZES: [usize; 3] = [33, 5, 40];

/// One effect the stream started.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Spawn {
    /// The frame of the record that spawned it; `None` in the snapshot.
    pub frame: Option<u32>,
    /// What effect it is.
    pub asset: u64,
    /// The entity or map object it is attached to; 0 for none.
    pub parent: u64,
    /// The entity it plays on; 0 for none.
    pub target: u64,
    /// Its id in the stream: what parameters and stops name.
    pub instance: u32,
    /// Where it is, in world metres.
    pub position: Option<[f32; 3]>,
    /// How it is turned, `x, y, z, w`.
    pub rotation: Option<[f32; 4]>,
    /// The alliance of whoever set it off.
    pub alliance: Option<u32>,
    /// Its last point list, in world metres, and the size the list may
    /// grow to. Empty and 0 for an effect without one.
    pub points: Vec<[f32; 3]>,
    pub capacity: u32,
    /// The frame its last point list was written in.
    pub grown: Option<u32>,
    /// The frame of its stop.
    pub stopped: Option<u32>,
}

/// What [`decode`] read.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Effects {
    /// Every spawn, in stream order: by frame, the snapshot's first.
    pub spawns: Vec<Spawn>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
    /// Records read, the snapshot included, and how many of them did not
    /// hold what their counts promise.
    pub records: usize,
    pub unparsed: usize,
    /// Instance -> index of its latest spawn.
    latest: HashMap<u32, usize>,
}

impl Effects {
    /// The spawns of an effect asset, in stream order.
    pub(crate) fn of(&self, asset: u64) -> impl Iterator<Item = &Spawn> {
        self.spawns.iter().filter(move |s| s.asset == asset)
    }

    /// The spawn of `instance`; the latest when the stream used the id
    /// twice (not seen).
    fn spawn_mut(&mut self, instance: u32) -> Option<&mut Spawn> {
        self.spawns.get_mut(*self.latest.get(&instance)?)
    }

    /// Adds what a record holds, written in `frame`.
    fn add(&mut self, record: Record, frame: Option<u32>) {
        self.records += 1;
        self.unparsed += usize::from(!record.whole);
        for &(asset, parent, instance, target) in &record.spawns {
            self.latest.insert(instance, self.spawns.len());
            self.spawns.push(Spawn {
                frame,
                asset,
                parent,
                target,
                instance,
                ..Spawn::default()
            });
        }
        for &(instance, hash, value) in &record.integers {
            if hash == ALLIANCE
                && let Some(s) = self.spawn_mut(instance)
            {
                s.alliance = Some(value);
            }
        }
        for &(instance, hash, [x, y, z, _]) in &record.vectors {
            if hash == POSITION
                && let Some(s) = self.spawn_mut(instance)
                && s.position.is_none()
            {
                s.position = Some([x, y, z]);
            }
        }
        for &(instance, hash, q) in &record.quaternions {
            if hash == ROTATION
                && let Some(s) = self.spawn_mut(instance)
                && s.rotation.is_none()
            {
                s.rotation = Some(q);
            }
        }
        for &instance in &record.stops {
            // A stop in the snapshot has no frame to give.
            if let Some(s) = self.spawn_mut(instance)
                && s.stopped.is_none()
            {
                s.stopped = frame;
            }
        }
        for (instance, capacity, points) in record.points {
            if let Some(s) = self.spawn_mut(instance) {
                s.points = points;
                s.capacity = capacity;
                s.grown = frame;
            }
        }
    }
}

/// What one record holds of the sections that are kept.
#[derive(Clone, Debug, Default, PartialEq)]
struct Record {
    /// `(asset, parent, instance, target)`.
    spawns: Vec<(u64, u64, u32, u64)>,
    /// `(instance, parameter, value)`.
    integers: Vec<(u32, Hash, u32)>,
    vectors: Vec<(u32, Hash, [f32; 4])>,
    quaternions: Vec<(u32, Hash, [f32; 4])>,
    /// The instances stopped.
    stops: Vec<u32>,
    /// `(instance, capacity, points)`.
    points: Vec<(u32, u32, Vec<[f32; 3]>)>,
    /// Every section held what its count promised, and nothing follows
    /// the last.
    whole: bool,
}

/// The bytes of a record still to read.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(head)
    }

    fn u8(&mut self) -> Option<u8> {
        self.bytes(1)?.first().copied()
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.bytes(8)?.try_into().ok()?))
    }

    fn hash(&mut self) -> Option<Hash> {
        self.bytes(4)?.try_into().ok()
    }

    fn floats<const N: usize>(&mut self) -> Option<[f32; N]> {
        let (floats, _) = self.bytes(4 * N)?.as_chunks::<4>();
        let mut out = [0.0; N];
        for (v, b) in out.iter_mut().zip(floats) {
            *v = f32::from_le_bytes(*b);
        }
        Some(out)
    }

    /// The count of a section whose entries are `size` bytes at least;
    /// `None` when the record cannot hold that many.
    fn count(&mut self, size: usize) -> Option<usize> {
        let count = self.u32()? as usize;
        (count.checked_mul(size)? <= self.0.len()).then_some(count)
    }
}

/// Reads a record. A fault leaves `whole` unset and what was read before
/// it in place.
fn record(bytes: &[u8]) -> Record {
    let mut out = Record::default();
    out.whole = sections(bytes, &mut out).is_some();
    out
}

/// Walks the sections of a record into `out`; `None` at the first that
/// does not fit, or when bytes are left over.
fn sections(bytes: &[u8], out: &mut Record) -> Option<()> {
    let mut r = Reader(bytes);
    // An empty record holds nothing.
    let Some(mask) = r.u8() else {
        return Some(());
    };
    if mask & SPAWNS != 0 {
        for _ in 0..r.count(SPAWN_SIZE)? {
            let (asset, parent, instance) = (r.u64()?, r.u64()?, r.u32()?);
            r.bytes(4)?;
            let target = r.u64()?;
            r.bytes(4)?;
            out.spawns.push((asset, parent, instance, target));
        }
    }
    if mask & INTEGERS != 0 {
        for _ in 0..r.count(PARAMETER_SIZE)? {
            out.integers.push((r.u32()?, r.hash()?, r.u32()?));
        }
    }
    if mask & FLOATS != 0 {
        let count = r.count(PARAMETER_SIZE)?;
        r.bytes(count * PARAMETER_SIZE)?;
    }
    if mask & VECTORS != 0 {
        for _ in 0..r.count(VECTOR_SIZE)? {
            out.vectors.push((r.u32()?, r.hash()?, r.floats()?));
        }
    }
    if mask & QUATERNIONS != 0 {
        for _ in 0..r.count(VECTOR_SIZE)? {
            out.quaternions.push((r.u32()?, r.hash()?, r.floats()?));
        }
    }
    if mask & STOPS != 0 {
        for _ in 0..r.count(STOP_SIZE)? {
            out.stops.push(r.u32()?);
            r.bytes(3)?;
        }
    }
    if mask & POINTS != 0 {
        for _ in 0..r.count(12)? {
            let (instance, capacity) = (r.u32()?, r.u32()?);
            let count = r.count(POINT_SIZE)?;
            let mut points = Vec::with_capacity(count);
            for _ in 0..count {
                let [x, y, z, _] = r.floats()?;
                points.push([x, y, z]);
            }
            out.points.push((instance, capacity, points));
        }
    }
    if mask & SOUNDS != 0 {
        for _ in 0..r.count(1)? {
            let kind = usize::from(r.u8()?);
            r.bytes(*SOUND_SIZES.get(kind.checked_sub(1)?)?)?;
        }
    }
    r.0.is_empty().then_some(())
}

/// Reads the effects stream of a round.
pub(crate) fn decode(input: &Input) -> Effects {
    let mut out = Effects::default();
    let blocks: Vec<(usize, usize, Option<u32>)> = input.blocks(STREAM).collect();
    // A record that is not in the data is one that does not read.
    let records: Vec<Record> = blocks
        .par_iter()
        .map(|&(start, end, _)| input.data.get(start..end).map(record).unwrap_or_default())
        .collect();
    for (block, record) in blocks.iter().zip(records) {
        out.add(record, block.2);
    }
    if out.unparsed > 0 {
        out.warnings.push(format!(
            "{} of {} effect records do not hold what their counts promise",
            out.unparsed, out.records
        ));
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a record section by section, in the order they are added.
    #[derive(Default)]
    pub(crate) struct Build {
        mask: u8,
        bytes: Vec<u8>,
    }

    impl Build {
        fn section(mut self, bit: u8, count: usize, entries: Vec<u8>) -> Self {
            self.mask |= bit;
            self.bytes.extend((count as u32).to_le_bytes());
            self.bytes.extend(entries);
            self
        }

        /// `(asset, parent, instance, target)`.
        pub(crate) fn spawns(self, spawns: &[(u64, u64, u32, u64)]) -> Self {
            let mut d = Vec::new();
            for (asset, parent, instance, target) in spawns {
                d.extend(asset.to_le_bytes());
                d.extend(parent.to_le_bytes());
                d.extend(instance.to_le_bytes());
                d.extend(0u32.to_le_bytes());
                d.extend(target.to_le_bytes());
                d.extend(2u32.to_le_bytes());
            }
            self.section(SPAWNS, spawns.len(), d)
        }

        pub(crate) fn integers(self, entries: &[(u32, Hash, u32)]) -> Self {
            let mut d = Vec::new();
            for (instance, hash, value) in entries {
                d.extend(instance.to_le_bytes());
                d.extend(hash);
                d.extend(value.to_le_bytes());
            }
            self.section(INTEGERS, entries.len(), d)
        }

        pub(crate) fn floats(self, entries: &[(u32, Hash, f32)]) -> Self {
            let mut d = Vec::new();
            for (instance, hash, value) in entries {
                d.extend(instance.to_le_bytes());
                d.extend(hash);
                d.extend(value.to_le_bytes());
            }
            self.section(FLOATS, entries.len(), d)
        }

        fn four(entries: &[(u32, Hash, [f32; 4])]) -> Vec<u8> {
            let mut d = Vec::new();
            for (instance, hash, v) in entries {
                d.extend(instance.to_le_bytes());
                d.extend(hash);
                d.extend(v.iter().flat_map(|v| v.to_le_bytes()));
            }
            d
        }

        pub(crate) fn vectors(self, entries: &[(u32, Hash, [f32; 3])]) -> Self {
            let entries: Vec<_> = (entries.iter())
                .map(|&(i, h, [x, y, z])| (i, h, [x, y, z, 1.0]))
                .collect();
            self.section(VECTORS, entries.len(), Self::four(&entries))
        }

        pub(crate) fn quaternions(self, entries: &[(u32, Hash, [f32; 4])]) -> Self {
            self.section(QUATERNIONS, entries.len(), Self::four(entries))
        }

        pub(crate) fn stops(self, instances: &[u32]) -> Self {
            let mut d = Vec::new();
            for instance in instances {
                d.extend(instance.to_le_bytes());
                d.extend([1, 0, 0]);
            }
            self.section(STOPS, instances.len(), d)
        }

        /// `(instance, capacity, points)`.
        pub(crate) fn points(self, lists: &[(u32, u32, &[[f32; 3]])]) -> Self {
            let mut d = Vec::new();
            for (instance, capacity, points) in lists {
                d.extend(instance.to_le_bytes());
                d.extend(capacity.to_le_bytes());
                d.extend((points.len() as u32).to_le_bytes());
                for p in *points {
                    d.extend(p.iter().flat_map(|v| v.to_le_bytes()));
                    d.extend(1f32.to_le_bytes());
                }
            }
            self.section(POINTS, lists.len(), d)
        }

        /// Sound entries of these types, filled with `ee`.
        pub(crate) fn sounds(self, kinds: &[u8]) -> Self {
            let mut d = Vec::new();
            for &kind in kinds {
                d.push(kind);
                d.extend(vec![0xEE; SOUND_SIZES[usize::from(kind) - 1]]);
            }
            self.section(SOUNDS, kinds.len(), d)
        }

        pub(crate) fn done(self) -> Vec<u8> {
            let mut d = vec![self.mask];
            d.extend(self.bytes);
            d
        }
    }

    fn effects(records: &[(Option<u32>, Vec<u8>)]) -> Effects {
        let mut out = Effects::default();
        for (frame, bytes) in records {
            out.add(record(bytes), *frame);
        }
        out
    }

    #[test]
    fn a_spawn_is_an_asset_on_a_parent_and_a_target() {
        let bytes = Build::default().spawns(&[(7, 0xF1, 3, 0xF2)]).done();
        assert_eq!(bytes.len(), 1 + 4 + SPAWN_SIZE);
        let fx = effects(&[(None, bytes.clone()), (Some(9), bytes)]);
        assert_eq!((fx.records, fx.unparsed), (2, 0));
        let s = &fx.spawns[0];
        assert_eq!(
            (s.frame, s.asset, s.parent, s.instance, s.target),
            (None, 7, 0xF1, 3, 0xF2)
        );
        assert_eq!((s.position, s.alliance, s.stopped), (None, None, None));
        // The id used again: the instance is the later spawn.
        assert_eq!(fx.spawns[1].frame, Some(9));
        assert_eq!(fx.latest.get(&3), Some(&1));
        assert_eq!(fx.latest.get(&4), None);
        assert_eq!(fx.of(7).count(), 2);
        assert_eq!(fx.of(8).count(), 0);
    }

    #[test]
    fn the_alliance_is_an_int_parameter() {
        let spawn = Build::default().spawns(&[(7, 0, 3, 0)]).done();
        let other = Build::default()
            .integers(&[(3, [1, 2, 3, 4], 9), (3, ALLIANCE, 4), (8, ALLIANCE, 1)])
            .done();
        assert_eq!(other.len(), 1 + 4 + 3 * PARAMETER_SIZE);
        let fx = effects(&[(Some(1), spawn), (Some(2), other)]);
        assert_eq!(fx.unparsed, 0);
        assert_eq!(fx.spawns[0].alliance, Some(4));
    }

    #[test]
    fn float_parameters_are_walked() {
        let bytes = Build::default()
            .spawns(&[(7, 0, 3, 0)])
            .floats(&[(3, POSITION, 1.0), (3, ALLIANCE, 2.0)])
            .vectors(&[(3, POSITION, [1.0, 2.0, 3.0])])
            .done();
        let fx = effects(&[(Some(1), bytes)]);
        assert_eq!(fx.unparsed, 0);
        assert_eq!(fx.spawns[0].position, Some([1.0, 2.0, 3.0]));
        assert_eq!(fx.spawns[0].alliance, None);
    }

    #[test]
    fn the_position_is_the_first_vector_written() {
        let spawn = Build::default()
            .spawns(&[(7, 0, 3, 0), (7, 0, 4, 0)])
            .vectors(&[
                (3, [9, 9, 9, 9], [9.0; 3]),
                (4, POSITION, [4.0; 3]),
                (3, POSITION, [1.0, 2.0, 3.0]),
            ])
            .done();
        let later = Build::default().vectors(&[(3, POSITION, [5.0; 3])]).done();
        let fx = effects(&[(Some(1), spawn), (Some(2), later)]);
        assert_eq!(fx.unparsed, 0);
        assert_eq!(fx.spawns[0].position, Some([1.0, 2.0, 3.0]));
        assert_eq!(fx.spawns[1].position, Some([4.0; 3]));
    }

    #[test]
    fn the_rotation_is_a_quaternion_parameter() {
        let q = [0.0, 0.6, 0.0, 0.8];
        let bytes = Build::default()
            .spawns(&[(7, 0, 3, 0)])
            .quaternions(&[(3, [9; 4], [1.0; 4]), (3, ROTATION, q)])
            .done();
        let fx = effects(&[(Some(1), bytes)]);
        assert_eq!(fx.unparsed, 0);
        assert_eq!(fx.spawns[0].rotation, Some(q));
    }

    #[test]
    fn a_stop_ends_its_instance_once() {
        let spawn = Build::default()
            .spawns(&[(7, 0, 3, 0), (7, 0, 4, 0)])
            .done();
        let stop = Build::default().stops(&[3, 99]).done();
        assert_eq!(stop.len(), 1 + 4 + 2 * STOP_SIZE);
        let fx = effects(&[(Some(1), spawn), (Some(5), stop.clone()), (Some(8), stop)]);
        assert_eq!(fx.unparsed, 0);
        assert_eq!(fx.spawns[0].stopped, Some(5));
        assert_eq!(fx.spawns[1].stopped, None);
    }

    #[test]
    fn the_last_point_list_is_kept() {
        let spawn = Build::default().spawns(&[(7, 0, 3, 0)]).done();
        let one = Build::default().points(&[(3, 12, &[[1.0; 3]])]).done();
        let two = Build::default()
            .points(&[(3, 12, &[[1.0; 3], [2.0, 3.0, 4.0]]), (4, 1, &[])])
            .done();
        let fx = effects(&[(Some(1), spawn), (Some(2), one), (Some(6), two)]);
        assert_eq!(fx.unparsed, 0);
        let s = &fx.spawns[0];
        assert_eq!(s.points, vec![[1.0; 3], [2.0, 3.0, 4.0]]);
        assert_eq!((s.capacity, s.grown), (12, Some(6)));
    }

    #[test]
    fn sound_entries_are_sized_by_their_type() {
        let bytes = Build::default()
            .spawns(&[(7, 0, 3, 0)])
            .sounds(&[1, 2, 3, 2])
            .done();
        assert_eq!(bytes.len(), 1 + 4 + SPAWN_SIZE + 4 + 34 + 6 + 41 + 6);
        let fx = effects(&[(Some(1), bytes)]);
        assert_eq!((fx.spawns.len(), fx.unparsed), (1, 0));
        // A type that is not known ends the read; the spawn stays.
        let mut unknown = Build::default().spawns(&[(7, 0, 3, 0)]).sounds(&[1]).done();
        unknown[1 + 4 + SPAWN_SIZE + 4] = 4;
        let fx = effects(&[(Some(1), unknown)]);
        assert_eq!((fx.spawns.len(), fx.unparsed), (1, 1));
    }

    /// Sections come in the order spawn, int, float, vector, quaternion,
    /// stop, points, sound, whatever their bits: a stop (`02`) follows a
    /// quaternion (`20`).
    #[test]
    fn sections_are_in_their_own_order() {
        let q = [0.0, 0.0, 0.6, 0.8];
        let bytes = Build::default()
            .spawns(&[(7, 0, 3, 0), (7, 0, 4, 0)])
            .quaternions(&[(3, ROTATION, q)])
            .stops(&[4])
            .done();
        assert_eq!(bytes[0], 0x23);
        let fx = effects(&[(Some(2), bytes)]);
        assert_eq!(fx.unparsed, 0);
        assert_eq!(fx.spawns[0].rotation, Some(q));
        assert_eq!(
            (fx.spawns[0].stopped, fx.spawns[1].stopped),
            (None, Some(2))
        );
        // Written the other way round the bytes are not a record.
        let swapped = Build::default()
            .spawns(&[(7, 0, 3, 0), (7, 0, 4, 0)])
            .stops(&[4])
            .quaternions(&[(3, ROTATION, q)])
            .done();
        assert_eq!(swapped[0], 0x23);
        assert_eq!(effects(&[(Some(2), swapped)]).unparsed, 1);
    }

    #[test]
    fn every_section_reads_in_one_record() {
        let bytes = Build::default()
            .spawns(&[(7, 0xF1, 3, 0)])
            .integers(&[(3, ALLIANCE, 2)])
            .floats(&[(3, [5; 4], 0.5)])
            .vectors(&[(3, POSITION, [1.0, 2.0, 3.0])])
            .quaternions(&[(3, ROTATION, [0.0, 0.0, 0.0, 1.0])])
            .stops(&[3])
            .points(&[(3, 4, &[[7.0; 3]])])
            .sounds(&[3, 1])
            .done();
        assert_eq!(bytes[0], 0xFF);
        let fx = effects(&[(Some(4), bytes)]);
        assert_eq!(fx.unparsed, 0);
        let s = &fx.spawns[0];
        assert_eq!((s.alliance, s.position), (Some(2), Some([1.0, 2.0, 3.0])));
        assert_eq!(s.rotation, Some([0.0, 0.0, 0.0, 1.0]));
        assert_eq!((s.stopped, s.capacity), (Some(4), 4));
        assert_eq!(s.points, vec![[7.0; 3]]);
    }

    #[test]
    fn a_malformed_record_is_counted_and_keeps_what_was_read() {
        let record = Build::default()
            .spawns(&[(7, 0, 3, 5)])
            .vectors(&[(3, POSITION, [1.0; 3])])
            .done();
        // Cut in the spawns: nothing. Cut after them: the spawn, unplaced.
        for cut in 1..record.len() {
            let fx = effects(&[(Some(1), record[..cut].to_vec())]);
            assert_eq!(fx.unparsed, 1, "cut at {cut}");
            assert_eq!(
                fx.spawns.len(),
                usize::from(cut >= 5 + SPAWN_SIZE),
                "cut at {cut}"
            );
            assert!(fx.spawns.iter().all(|s| s.position.is_none()));
        }
        // An empty record holds nothing and is no fault.
        let fx = effects(&[(Some(1), Vec::new())]);
        assert_eq!((fx.records, fx.unparsed, fx.spawns.len()), (1, 0, 0));
        // A count the record cannot hold, and bytes after the last section.
        let mut count = record.clone();
        count[1..5].copy_from_slice(&u32::MAX.to_le_bytes());
        let fx = effects(&[(Some(1), count)]);
        assert_eq!((fx.unparsed, fx.spawns.len()), (1, 0));
        let mut long = record.clone();
        long.push(0);
        let fx = effects(&[(Some(1), long)]);
        assert_eq!((fx.unparsed, fx.spawns[0].position), (1, Some([1.0; 3])));
        // A point list longer than the record.
        let mut points = Build::default().points(&[(3, 4, &[[1.0; 3]])]).done();
        points[13..17].copy_from_slice(&1000u32.to_le_bytes());
        assert_eq!(effects(&[(Some(1), points)]).unparsed, 1);
    }
}
