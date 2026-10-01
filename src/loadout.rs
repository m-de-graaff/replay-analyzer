//! Per-player loadouts (Y11S3): guns with their attachments and ammunition,
//! and the ability and gadget with how many were used and when.
//!
//! Two streams hold a loadout, and neither holds all of it.
//!
//! The `state` stream carries the HUD: each controller links (field
//! `e8d1e539`) a `PlayerLoadoutViewModel` with one field per slot:
//!
//! ```text
//! 9327a12b primary     03994083 secondary     4cd6a0c7 ability
//! d890b5f7 gadget      ede5acbc drone (not read)
//! ```
//!
//! A slot is a `WeaponViewModel` (class `0eaa2be0`), which links a
//! `WeaponAmmoViewModel` (class `af99e10b`) with `TotalAmmo` and
//! `MagazineSize`, or a `GadgetViewModel` (class `9f44690a`) with `Ammo` and
//! `MaxAmmo`. Either names its item through field `d19c5716`, then
//! `a57aaa29`, to an object whose `0e9ebe88` is the item id the kill feed
//! uses. The slot field decides what an item is, not its class: a shield
//! sits in the primary field as a gadget, a launcher in the ability field
//! as a weapon, and operators without an ability carry a gadget there.
//! Slots are linked again when an attacker swaps operator in prep, so the
//! last link of each field is the loadout the player spawned with.
//!
//! The `movement` stream carries the entities. Its snapshot and each of its
//! records hold `u16 count`, then `count` messages of `u64 entity, u32 size,
//! payload`. A payload of type `617385fe` describes its entity:
//!
//! ```text
//! +0  617385fe      +4  u64 entity     +44 u8 tag     +45 u64 link
//! +53 u32 n, n x u32 hash
//!     u64 asset, u32 0, u32 slots, slots x { u64 item, hash slot, u32 n }
//! ```
//!
//! A body's slots name the assets it carries (`PrimaryWeapon`,
//! `SecondaryWeapon`, gadgets, headgear, uniform); a gun's slots name its
//! attachments (`Sight`, `Barrel`, `Grip`, `Underbarrel`, `Magazine`, skin,
//! charm), 0 where the gun has no such slot. A body belongs to the player
//! whose header `playerid` ends one of the body's `607385fe` messages. A
//! gun belongs to the body whose slot holds its asset; players carrying the
//! same gun are told apart by the body id the gun's first `607385fe`
//! messages contain.

use std::collections::HashMap;

use rayon::prelude::*;
use serde::Serialize;

use crate::container::StreamInfo;
use crate::details::{Loadout, Phase};
use crate::entities::{Hash, Record, for_each_record};
use crate::feedback::display_clock;
use crate::header::Player;
use crate::records::RecordMap;
use crate::round::Round;
use crate::timeline::Timeline;
use crate::types::{Operator, TeamRole, attachment_info, item_name};

/// Name hashes of the two streams read here.
pub(crate) const STATE_STREAM: Hash = [0xA9, 0x8F, 0xDD, 0x0B];
pub(crate) const MOVEMENT_STREAM: Hash = [0x20, 0xA5, 0xC4, 0xE3];

/// Controller -> the player's `PlayerLoadoutViewModel`.
const LOADOUT_FIELD: Hash = [0xE8, 0xD1, 0xE5, 0x39];
/// Loadout view -> its slots.
const PRIMARY_FIELD: Hash = [0x93, 0x27, 0xA1, 0x2B];
const SECONDARY_FIELD: Hash = [0x03, 0x99, 0x40, 0x83];
const ABILITY_FIELD: Hash = [0x4C, 0xD6, 0xA0, 0xC7];
const GADGET_FIELD: Hash = [0xD8, 0x90, 0xB5, 0xF7];
/// The four slot fields: primary, secondary, ability, gadget.
pub(crate) const SLOT_FIELDS: [Hash; 4] =
    [PRIMARY_FIELD, SECONDARY_FIELD, ABILITY_FIELD, GADGET_FIELD];
/// Slot classes (`GadgetViewModel`, `WeaponViewModel`) and the class of a
/// weapon slot's ammunition object (`WeaponAmmoViewModel`).
const GADGET_VIEW: Hash = [0x9F, 0x44, 0x69, 0x0A];
const WEAPON_VIEW: Hash = [0x0E, 0xAA, 0x2B, 0xE0];
const WEAPON_AMMO: Hash = [0xAF, 0x99, 0xE1, 0x0B];
/// Weapon slot -> the gun's own `WeaponAmmoViewModel`.
const AMMO_FIELD: Hash = [0x1E, 0xD1, 0x7F, 0xD6];
/// Gadget slot: how many are left (`Ammo`) and how many it can hold
/// (`MaxAmmo`).
const AMMO: Hash = [0x14, 0xD1, 0xBD, 0x4F];
const MAX_AMMO: Hash = [0x3E, 0xB2, 0x43, 0xDC];
/// Weapon ammunition: rounds in the gun plus in reserve (`TotalAmmo`), and
/// what a magazine holds (`MagazineSize`).
pub(crate) const TOTAL_AMMO: Hash = [0x40, 0x0A, 0xC8, 0x29];
const MAGAZINE_SIZE: Hash = [0x56, 0xF5, 0x44, 0x0A];
/// Slot -> item object -> item data, which carries the item id.
const ITEM_OBJECT_FIELD: Hash = [0xD1, 0x9C, 0x57, 0x16];
const ITEM_DATA_FIELD: Hash = [0xA5, 0x7A, 0xAA, 0x29];
const ITEM_ID: Hash = [0x0E, 0x9E, 0xBE, 0x88];
/// A `MaxAmmo` of 99 marks an ability that refills over time.
const REGENERATES: u32 = 99;

/// Movement payload types: an entity's descriptor, and its updates.
const DESCRIPTOR: Hash = [0x61, 0x73, 0x85, 0xFE];
pub(crate) const UPDATE: Hash = [0x60, 0x73, 0x85, 0xFE];
/// Body slots that hold a gun's asset.
pub(crate) const PRIMARY_WEAPON: Hash = [0x64, 0xDC, 0xCF, 0xC2];
pub(crate) const SECONDARY_WEAPON: Hash = [0xB4, 0xA5, 0xD8, 0x70];
/// Gun slots that hold an attachment.
const SIGHT: Hash = [0xA0, 0x73, 0x6B, 0x48];
const BARREL: Hash = [0x81, 0x32, 0xCD, 0x9D];
const GRIP: Hash = [0x74, 0xA2, 0xC8, 0x94];
const UNDERBARREL: Hash = [0x28, 0xDD, 0x90, 0x64];
const MAGAZINE: Hash = [0xB2, 0x4D, 0xFA, 0xCE];
/// Bounds on a descriptor's two lists; real ones have up to 5 hashes and 23
/// slots.
const MAX_HASHES: usize = 16;
const MAX_SLOTS: usize = 64;
/// A gun names its body within its first updates. Only updates of at least
/// `LINK_SIZE` bytes can, and no more than `LINK_UPDATES` of them are kept.
const LINK_UPDATES: usize = 50;
const LINK_SIZE: usize = 60;

/// An item or attachment id with its name, when the lookup table has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Named {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
    /// An attachment's name is inferred, not read from the file.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub inferred: bool,
    /// A sight magnifies, when that is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magnified: Option<bool>,
}

impl Named {
    pub(crate) fn item(id: u64) -> Self {
        Named {
            id,
            name: item_name(id),
            inferred: false,
            magnified: None,
        }
    }

    /// `None` for 0: the gun has no such slot.
    fn attachment(id: u64) -> Option<Self> {
        (id != 0).then(|| {
            let info = attachment_info(id);
            Named {
                id,
                name: info.name,
                inferred: info.inferred,
                magnified: info.magnified,
            }
        })
    }
}

/// A gun's ammunition over the round.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ammo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magazine_size: Option<u32>,
    /// Rounds in the gun and in reserve when the player spawned, and at the
    /// end of the recording.
    pub start: u32,
    pub end: u32,
    /// Every drop of the total added up.
    pub fired: u32,
}

/// A primary or secondary: a gun with its attachments, or a shield.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Weapon {
    /// The HUD's item id, which kill feed `weapon` ids use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
    /// The gun's asset id, from the body's slot. Absent for shields and
    /// when the player's body was not found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sight: Option<Named>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub barrel: Option<Named>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grip: Option<Named>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underbarrel: Option<Named>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magazine: Option<Named>,
    /// The slot holds a shield, not a gun.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub shield: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ammo: Option<Ammo>,
    /// Whether the gun's entity was found, so absent attachments mean the
    /// gun has none.
    #[serde(skip)]
    pub linked: bool,
}

/// One drop of an ability's or gadget's count.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Use {
    /// How many were left after it.
    pub count: u32,
    pub time: String,
    pub phase: Phase,
    /// Seconds since the prep phase started.
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
    /// Seconds since the recording started, to the frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_time: Option<f64>,
}

/// How many of an ability or gadget a player had over the round.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    /// When the player spawned, and at the end of the recording.
    pub start: u32,
    pub end: u32,
    /// The most the slot holds. Absent for abilities that refill
    /// (`regenerates`) and for launchers, which count ammunition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
    /// Every drop and every rise of the count added up, so
    /// `start - used + gained` is `end`.
    pub used: u32,
    pub gained: u32,
    pub regenerates: bool,
    pub uses: Vec<Use>,
}

/// An ability or gadget. Without `counts` the game shows no count for it,
/// so its use is not recorded.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counted {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
    #[serde(flatten)]
    pub counts: Option<Counts>,
}

/// One value of a counter, with the frame and offset it was written at.
/// The frame is `None` in the opening snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sample {
    pub value: u32,
    pub frame: Option<u32>,
    pub at: usize,
}

#[derive(Debug, Default)]
struct Node {
    /// `(property, sample)` of every counter value, in stream order.
    counters: Vec<(Hash, Sample)>,
    /// The latest item id written to the object.
    item: Option<u64>,
    /// `(field, child, class)` of every link to a child, in stream order.
    links: Vec<(Hash, u32, Hash)>,
}

/// A HUD slot as the state stream left it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Slot {
    item: Option<u64>,
    /// A `WeaponViewModel`: `counts` is its total ammunition. Otherwise a
    /// `GadgetViewModel`, and `counts` is how many are left.
    weapon: bool,
    magazine_size: Option<u32>,
    /// A gadget's latest `MaxAmmo`.
    max: Option<u32>,
    /// Distinct consecutive values, from the spawn on.
    counts: Vec<Sample>,
}

/// The loadout objects of the state stream.
#[derive(Debug, Default)]
pub(crate) struct Hud {
    nodes: HashMap<u32, Node>,
}

impl Hud {
    /// Reads the records of one snapshot or frame record. `base` is where
    /// `block` starts in the data.
    pub(crate) fn read(&mut self, block: &[u8], base: usize, frame: Option<u32>) {
        let hash_at = |at: usize| -> Option<Hash> { block.get(at..at + 4)?.try_into().ok() };
        // Each record block names its object before writing to it.
        let mut current: Option<u32> = None;
        for_each_record(block, |at, r| match r {
            Record::Set(obj, hash, from, to) => {
                current = Some(obj);
                self.property(obj, hash, &block[from..to], frame, base + at);
            }
            // `26` array elements are not counters.
            Record::Prop(hash, from, to) if block[at] == 0x22 => {
                if let Some(obj) = current {
                    self.property(obj, hash, &block[from..to], frame, base + at);
                }
            }
            Record::ParentChild(parent, field, child) => {
                current = Some(parent);
                if let Some(class) = hash_at(at + 21).filter(|_| child != 0) {
                    let links = &mut self.nodes.entry(parent).or_default().links;
                    links.push((field, child, class));
                }
            }
            Record::Child(field, child) => {
                if let (Some(parent), true) = (current, child != 0)
                    && let Some(class) = hash_at(at + 13)
                {
                    let links = &mut self.nodes.entry(parent).or_default().links;
                    links.push((field, child, class));
                }
            }
            Record::Prop(..) | Record::Element(..) => {}
        });
    }

    fn property(&mut self, obj: u32, hash: Hash, value: &[u8], frame: Option<u32>, at: usize) {
        if hash == ITEM_ID {
            if let Ok(id) = <[u8; 8]>::try_from(value) {
                self.nodes.entry(obj).or_default().item = Some(u64::from_le_bytes(id));
            }
        } else if [AMMO, MAX_AMMO, TOTAL_AMMO, MAGAZINE_SIZE].contains(&hash)
            || crate::weapons::COUNTERS.contains(&hash)
        {
            if let Ok(v) = <[u8; 4]>::try_from(value) {
                let value = u32::from_le_bytes(v);
                let counters = &mut self.nodes.entry(obj).or_default().counters;
                counters.push((hash, Sample { value, frame, at }));
            }
        } else if crate::weapons::FLAGS.contains(&hash)
            && let [v] = *value
        {
            let value = u32::from(v);
            let counters = &mut self.nodes.entry(obj).or_default().counters;
            counters.push((hash, Sample { value, frame, at }));
        }
    }

    /// The last child linked to `obj` that `wanted` accepts, with its class.
    pub(crate) fn last_link(
        &self,
        obj: u32,
        wanted: impl Fn(Hash, Hash) -> bool,
    ) -> Option<(u32, Hash)> {
        let links = &self.nodes.get(&obj)?.links;
        let (_, child, class) = links.iter().rev().find(|l| wanted(l.0, l.2))?;
        Some((*child, *class))
    }

    pub(crate) fn child(&self, obj: u32, field: Hash) -> Option<u32> {
        Some(self.last_link(obj, |f, _| f == field)?.0)
    }

    /// Every value of one counter of `obj`, in stream order.
    pub(crate) fn series(&self, obj: u32, counter: Hash) -> impl Iterator<Item = Sample> + '_ {
        self.nodes
            .get(&obj)
            .into_iter()
            .flat_map(|n| &n.counters)
            .filter(move |c| c.0 == counter)
            .map(|c| c.1)
    }

    /// The item id of the slot object `slot`.
    pub(crate) fn item(&self, slot: u32) -> Option<u64> {
        self.child(slot, ITEM_OBJECT_FIELD)
            .and_then(|o| self.child(o, ITEM_DATA_FIELD))
            .and_then(|o| self.nodes.get(&o)?.item)
    }

    /// The slot object last linked to `field` of the loadout view `view`,
    /// and whether it is a `WeaponViewModel`.
    pub(crate) fn slot_object(&self, view: u32, field: Hash) -> Option<(u32, bool)> {
        let (obj, class) = self.last_link(view, |f, c| {
            f == field && (c == GADGET_VIEW || c == WEAPON_VIEW)
        })?;
        Some((obj, class == WEAPON_VIEW))
    }

    /// The `WeaponAmmoViewModel` of a weapon slot object.
    pub(crate) fn ammo_object(&self, slot: u32) -> Option<u32> {
        // A gun with a launcher under it links a second one, for the
        // launcher, through another field.
        let ammo =
            |own: bool| self.last_link(slot, |f, c| c == WEAPON_AMMO && (!own || f == AMMO_FIELD));
        Some(ammo(true).or_else(|| ammo(false))?.0)
    }

    /// The loadout view of the player with this controller that was
    /// played: the last one with a slot (see [`Hud::slots`]).
    pub(crate) fn view(&self, controller: u32) -> Option<u32> {
        let links = &self.nodes.get(&controller)?.links;
        links
            .iter()
            .rev()
            .filter(|l| l.0 == LOADOUT_FIELD)
            .map(|l| l.1)
            .find(|&view| SLOT_FIELDS.iter().any(|&f| self.slot(view, f).is_some()))
    }

    /// The slot last linked to `field` of the loadout view `view`.
    fn slot(&self, view: u32, field: Hash) -> Option<Slot> {
        let (obj, class) = self.last_link(view, |f, c| {
            f == field && (c == GADGET_VIEW || c == WEAPON_VIEW)
        })?;
        let mut slot = Slot {
            item: self.item(obj),
            weapon: class == WEAPON_VIEW,
            ..Slot::default()
        };
        if slot.weapon {
            if let Some(ammo) = self.ammo_object(obj) {
                slot.magazine_size = self.series(ammo, MAGAZINE_SIZE).next().map(|s| s.value);
                slot.counts = distinct(self.series(ammo, TOTAL_AMMO));
            }
            return Some(slot);
        }
        // A gadget's count is set up before the player spawns, and the
        // slot's capacity is zero until then: the count that holds when the
        // capacity first shows is the one the player starts with.
        let spawned = self.series(obj, MAX_AMMO).find(|s| s.value > 0);
        let Some(spawned) = spawned else {
            return Some(slot);
        };
        slot.max = self.series(obj, MAX_AMMO).last().map(|s| s.value);
        slot.counts = distinct(self.series(obj, AMMO).filter(|s| s.frame >= spawned.frame));
        if slot.counts.is_empty() {
            // Not written again since: the last value before still holds.
            slot.counts = self.series(obj, AMMO).last().into_iter().collect();
        }
        Some(slot)
    }

    /// The four slots of the player with this controller: primary,
    /// secondary, ability, gadget. `None` without a loadout view that has
    /// a slot.
    fn slots(&self, controller: u32) -> Option<[Option<Slot>; 4]> {
        // A player who leaves has their view unlinked, and whoever takes
        // the seat gets a new one that stays empty until they spawn: the
        // last view with a slot is the one that was played.
        let view = self.view(controller)?;
        Some(SLOT_FIELDS.map(|f| self.slot(view, f)))
    }
}

/// `samples` without values repeating the one before: the game sends a
/// value again without it having changed.
pub(crate) fn distinct(samples: impl Iterator<Item = Sample>) -> Vec<Sample> {
    let mut out: Vec<Sample> = Vec::new();
    for s in samples {
        if out.last().is_none_or(|l| l.value != s.value) {
            out.push(s);
        }
    }
    out
}

/// An entity's descriptor: what it is and what sits in its slots.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Descriptor {
    pub entity: u64,
    pub asset: u64,
    /// `(slot, item)`; an item of 0 is an empty slot.
    pub slots: Vec<(Hash, u64)>,
}

impl Descriptor {
    /// The item in `slot`; `None` when the entity has no such slot.
    pub(crate) fn slot(&self, slot: Hash) -> Option<u64> {
        self.slots.iter().find(|s| s.0 == slot).map(|s| s.1)
    }

    /// A player's body: the entity with a slot for a primary weapon, even
    /// an empty one.
    fn is_body(&self) -> bool {
        self.slot(PRIMARY_WEAPON).is_some()
    }

    /// Sight, barrel, grip, underbarrel, magazine.
    fn attachments(&self) -> [u64; 5] {
        [SIGHT, BARREL, GRIP, UNDERBARREL, MAGAZINE].map(|s| self.slot(s).unwrap_or(0))
    }
}

/// Parses a `617385fe` payload. `None` when it is another type or does not
/// hold what its counts promise.
pub(crate) fn descriptor(payload: &[u8]) -> Option<Descriptor> {
    let u32_at = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(
            payload.get(at..at + 4)?.try_into().ok()?,
        ))
    };
    let u64_at = |at: usize| -> Option<u64> {
        Some(u64::from_le_bytes(
            payload.get(at..at + 8)?.try_into().ok()?,
        ))
    };
    if payload.get(..4)? != DESCRIPTOR {
        return None;
    }
    let hashes = u32_at(53)? as usize;
    if hashes > MAX_HASHES {
        return None;
    }
    let mut at = 57 + 4 * hashes;
    let asset = u64_at(at)?;
    let count = u32_at(at + 12)? as usize;
    at += 16;
    if count > MAX_SLOTS || at + 16 * count > payload.len() {
        return None;
    }
    let slots = (0..count)
        .map(|i| {
            let at = at + 16 * i;
            let slot: Hash = payload.get(at + 8..at + 12)?.try_into().ok()?;
            Some((slot, u64_at(at)?))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Descriptor {
        entity: u64_at(4)?,
        asset,
        slots,
    })
}

/// The messages of a movement snapshot or record: `(entity, offset of the
/// payload in block, payload)`. Stops at a message the block cannot hold.
pub(crate) fn messages(block: &[u8]) -> impl Iterator<Item = (u64, usize, &[u8])> {
    let count = block
        .get(..2)
        .map_or(0, |c| u16::from_le_bytes([c[0], c[1]]));
    let mut at = 2;
    (0..count).map_while(move |_| {
        let entity = u64::from_le_bytes(block.get(at..at + 8)?.try_into().ok()?);
        let size = u32::from_le_bytes(block.get(at + 8..at + 12)?.try_into().ok()?) as usize;
        let start = at + 12;
        let payload = block.get(start..start.checked_add(size)?)?;
        at = start + size;
        Some((entity, start, payload))
    })
}

/// How a gun slot of a body resolved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Gun {
    /// The body has no gun in the slot.
    #[default]
    Empty,
    /// The slot names an asset, but which entity carries it is not known.
    Unlinked(u64),
    Linked {
        asset: u64,
        attachments: [u64; 5],
    },
}

/// What one block of the movement stream holds: its descriptors, and the
/// `(entity, player id)` of each update that ends in a player's id.
type Found = (Vec<Descriptor>, Vec<(u64, u64)>);

/// The entities of the movement stream that make up loadouts.
#[derive(Debug, Default)]
pub(crate) struct Entities {
    /// Every descriptor in stream order.
    descriptors: Vec<Descriptor>,
    /// Entity -> its latest descriptor.
    latest: HashMap<u64, usize>,
    /// Entity -> the player id that ends one of its updates.
    owners: HashMap<u64, u64>,
    /// The bodies, in stream order.
    bodies: Vec<u64>,
    /// Gun -> where its first large updates are in the data.
    early: HashMap<u64, Vec<(usize, usize)>>,
}

impl Entities {
    /// Reads the movement stream. `blocks` are the `(start, end)` of its
    /// snapshot and records in `data`; `players` are the header
    /// `playerid`s. The stream is most of a replay, so its blocks are read
    /// in parallel: once for descriptors and owners, and once more for the
    /// updates of the guns the bodies carry.
    pub(crate) fn read(data: &[u8], blocks: &[(usize, usize)], players: &[u64]) -> Entities {
        let block = |&(start, end): &(usize, usize)| data.get(start..end).unwrap_or_default();
        let mut e = Entities::default();
        let found: Vec<Found> = blocks
            .par_iter()
            .map(|b| {
                let (mut descriptors, mut owners) = (Vec::new(), Vec::new());
                for (entity, _, payload) in messages(block(b)) {
                    let Some(kind) = payload.get(..4) else {
                        continue;
                    };
                    if kind == DESCRIPTOR {
                        // A descriptor that names another entity is not one.
                        descriptors.extend(descriptor(payload).filter(|d| d.entity == entity));
                    } else if kind == UPDATE
                        && let Some(tail) = payload.len().checked_sub(8).filter(|&t| t >= 4)
                        && let Ok(id) = payload[tail..].try_into().map(u64::from_le_bytes)
                        && id != 0
                        && players.contains(&id)
                    {
                        owners.push((entity, id));
                    }
                }
                (descriptors, owners)
            })
            .collect();
        for (descriptors, owners) in found {
            for d in descriptors {
                e.latest.insert(d.entity, e.descriptors.len());
                e.descriptors.push(d);
            }
            for (entity, id) in owners {
                e.owners.entry(entity).or_insert(id);
            }
        }
        let current = |(i, d): &(usize, &Descriptor)| e.latest.get(&d.entity) == Some(i);
        let latest = || {
            e.descriptors
                .iter()
                .enumerate()
                .filter(current)
                .map(|(_, d)| d)
        };
        e.bodies = latest().filter(|d| d.is_body()).map(|d| d.entity).collect();
        let carried: Vec<u64> = latest()
            .filter(|d| d.is_body())
            .flat_map(|d| [d.slot(PRIMARY_WEAPON), d.slot(SECONDARY_WEAPON)])
            .flatten()
            .filter(|&asset| asset != 0)
            .collect();
        let guns: Vec<u64> = latest()
            .filter(|d| !d.is_body() && carried.contains(&d.asset))
            .map(|d| d.entity)
            .collect();
        if guns.is_empty() {
            return e;
        }
        let updates: Vec<Vec<(u64, usize, usize)>> = blocks
            .par_iter()
            .map(|b| {
                messages(block(b))
                    .filter(|(entity, _, payload)| {
                        payload.len() >= LINK_SIZE
                            && payload.starts_with(&UPDATE)
                            && guns.contains(entity)
                    })
                    .map(|(entity, at, payload)| (entity, b.0 + at, b.0 + at + payload.len()))
                    .collect()
            })
            .collect();
        for (entity, from, to) in updates.into_iter().flatten() {
            let early = e.early.entry(entity).or_default();
            if early.len() < LINK_UPDATES {
                early.push((from, to));
            }
        }
        e
    }

    /// The bodies, in stream order.
    pub(crate) fn bodies(&self) -> &[u64] {
        &self.bodies
    }

    /// The `playerid` that ends one of `entity`'s updates.
    pub(crate) fn owner(&self, entity: u64) -> Option<u64> {
        self.owners.get(&entity).copied()
    }

    pub(crate) fn get(&self, entity: u64) -> Option<&Descriptor> {
        self.latest.get(&entity).map(|&i| &self.descriptors[i])
    }

    /// The latest body of the player with this `playerid`; `fallback` is
    /// the body the player table gave them.
    fn body(&self, player: u64, fallback: Option<u32>) -> Option<&Descriptor> {
        let owned = self.descriptors.iter().enumerate().rev().find(|&(i, d)| {
            d.is_body()
                && self.latest.get(&d.entity) == Some(&i)
                && player != 0
                && self.owners.get(&d.entity) == Some(&player)
        });
        owned
            .map(|(_, d)| d)
            .or_else(|| self.get(u64::from(fallback?)).filter(|d| d.is_body()))
    }

    /// The first body whose id one of `entity`'s early updates contains.
    pub(crate) fn parent(&self, data: &[u8], entity: u64) -> Option<u64> {
        self.early.get(&entity)?.iter().find_map(|&(from, to)| {
            let update = data.get(from..to)?;
            self.bodies
                .iter()
                .copied()
                .find(|b| memchr::memmem::find(update, &b.to_le_bytes()).is_some())
        })
    }

    /// The gun in `slot` of `body`: the entity with the slot's asset that
    /// names the body. Without one, any entity with that asset will do as
    /// long as they all carry the same attachments.
    fn gun(&self, data: &[u8], body: &Descriptor, slot: Hash) -> Gun {
        let Some(asset) = body.slot(slot).filter(|&a| a != 0) else {
            return Gun::Empty;
        };
        let candidates: Vec<&Descriptor> = self
            .descriptors
            .iter()
            .enumerate()
            .filter(|&(i, d)| {
                d.asset == asset && !d.is_body() && self.latest.get(&d.entity) == Some(&i)
            })
            .map(|(_, d)| d)
            .collect();
        let named = candidates
            .iter()
            .rev()
            .find(|d| self.parent(data, d.entity) == Some(body.entity));
        let attachments = match (named, candidates.first()) {
            (Some(d), _) => d.attachments(),
            (None, Some(first))
                if candidates
                    .iter()
                    .all(|d| d.attachments() == first.attachments()) =>
            {
                first.attachments()
            }
            _ => return Gun::Unlinked(asset),
        };
        Gun::Linked { asset, attachments }
    }
}

/// What the clock said when something was written at an offset.
pub(crate) struct Clock<'a> {
    pub timeline: &'a Timeline,
    /// Offset of the packet that first showed each clock reading.
    pub reading_offsets: &'a [usize],
    /// Seconds since the recording started, per frame.
    pub frame_times: &'a [f64],
}

impl Clock<'_> {
    /// Seconds since the recording started for something written in
    /// `frame`.
    pub(crate) fn seconds(&self, frame: Option<u32>) -> Option<f64> {
        self.frame_times.get(frame? as usize).copied()
    }

    /// When something written at offset `at`, in `frame`, happened: the
    /// clock reading in force, and the recording time to the millisecond.
    pub(crate) fn when(&self, at: usize, frame: Option<u32>) -> When {
        // The reading in force is the last one shown before the offset.
        let tick = self
            .reading_offsets
            .partition_point(|&o| o <= at)
            .checked_sub(1);
        let at = self.timeline.at(tick);
        When {
            time: display_clock(at.seconds),
            phase: at.phase,
            elapsed: at.elapsed,
            recording_time: (self.seconds(frame)).map(|t| (t * 1000.0).round() / 1000.0),
        }
    }

    fn place(&self, count: u32, s: Sample) -> Use {
        let when = self.when(s.at, s.frame);
        Use {
            count,
            time: when.time,
            phase: when.phase,
            elapsed: when.elapsed,
            recording_time: when.recording_time,
        }
    }
}

/// When an event happened, as every timed event of a round says it: the
/// round clock, the phase, seconds since prep started and seconds since
/// the recording started.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct When {
    pub time: String,
    pub phase: Phase,
    /// Seconds since the prep phase started.
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
    /// Seconds since the recording started, to the frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_time: Option<f64>,
}

/// What a decoder of one kind of event reads from: the decompressed data
/// with its streams, the players, and the clock to place events with.
pub(crate) struct Input<'a> {
    pub data: &'a [u8],
    pub map: &'a RecordMap,
    pub streams: &'a [StreamInfo],
    pub players: &'a [Player],
    pub clock: &'a Clock<'a>,
}

impl Input<'_> {
    /// `(start, end, frame)` in `data` of the opening snapshot (frame
    /// `None`) and of each record of the stream named `name`, in order.
    pub(crate) fn blocks(
        &self,
        name: Hash,
    ) -> impl Iterator<Item = (usize, usize, Option<u32>)> + '_ {
        blocks(self.map, self.streams, name)
    }
}

/// `(drops, rises)` of a series of counts, each added up.
fn totals(counts: &[Sample]) -> (u32, u32) {
    counts.windows(2).fold((0, 0), |(down, up), w| {
        let (a, b) = (w[0].value, w[1].value);
        (down + a.saturating_sub(b), up + b.saturating_sub(a))
    })
}

fn weapon(slot: Option<&Slot>, gun: &Gun) -> Option<Weapon> {
    let id = slot.and_then(|s| s.item);
    let name = id.and_then(item_name);
    // A shield sits in the slot as a gadget; it has no entity of its own
    // in the body's weapon slots.
    if slot.is_some_and(|s| !s.weapon) {
        return Some(Weapon {
            id,
            name,
            shield: true,
            linked: true,
            ..Weapon::default()
        });
    }
    let (asset, attachments) = match gun {
        Gun::Empty if slot.is_none() => return None,
        Gun::Empty => (None, None),
        Gun::Unlinked(asset) => (Some(*asset), None),
        Gun::Linked { asset, attachments } => (Some(*asset), Some(*attachments)),
    };
    let [sight, barrel, grip, underbarrel, magazine] =
        attachments.unwrap_or_default().map(Named::attachment);
    let ammo = slot.and_then(|s| {
        let (fired, _) = totals(&s.counts);
        Some(Ammo {
            magazine_size: s.magazine_size,
            start: s.counts.first()?.value,
            end: s.counts.last()?.value,
            fired,
        })
    });
    Some(Weapon {
        id,
        name,
        asset,
        sight,
        barrel,
        grip,
        underbarrel,
        magazine,
        shield: false,
        ammo,
        linked: attachments.is_some(),
    })
}

fn counted(slot: &Slot, clock: &Clock) -> Counted {
    let counts = slot.counts.first().zip(slot.counts.last()).map(|(a, b)| {
        let (used, gained) = totals(&slot.counts);
        let regenerates = slot.max == Some(REGENERATES);
        Counts {
            start: a.value,
            end: b.value,
            max: slot.max.filter(|_| !regenerates),
            used,
            gained,
            regenerates,
            uses: slot
                .counts
                .windows(2)
                .filter(|w| w[1].value < w[0].value)
                .map(|w| clock.place(w[1].value, w[1]))
                .collect(),
        }
    });
    Counted {
        id: slot.item,
        name: slot.item.and_then(item_name),
        counts,
    }
}

/// The loadout a player spawned with.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Detail {
    pub primary: Option<Weapon>,
    pub secondary: Option<Weapon>,
    pub ability: Option<Counted>,
    pub gadget: Option<Counted>,
}

/// What `decode` found, for `decodeStatus.loadouts`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    /// `(index into the players, loadout)`.
    pub details: Vec<(usize, Detail)>,
    /// What each player held, fired and reloaded.
    pub activity: Vec<crate::weapons::Activity>,
    /// Players whose HUD loadout was found, of how many.
    pub resolved: usize,
    pub expected: usize,
    /// Players whose loadout, body or gun could not be found.
    pub warnings: Vec<String>,
}

/// `(start, end, frame)` of a stream's opening snapshot and of each of its
/// records, in order.
pub(crate) fn blocks(
    map: &RecordMap,
    streams: &[StreamInfo],
    name: Hash,
) -> impl Iterator<Item = (usize, usize, Option<u32>)> {
    // Snapshots are in stream-list order, records in main-stream order.
    let snapshot = streams
        .iter()
        .position(|s| s.name_hash == name)
        .and_then(|i| map.snapshots.get(i))
        .map(|&(start, end)| (start, end, None));
    let records = map
        .stream_index(name)
        .into_iter()
        .flat_map(|i| map.records_of(i))
        .map(|(frame, start, end)| (start, end, Some(frame)));
    snapshot.into_iter().chain(records)
}

/// Reads every player's loadout from the state and movement streams.
pub(crate) fn decode(input: &Input) -> Decoded {
    let &Input {
        data,
        map,
        streams,
        players,
        clock,
    } = input;
    let mut hud = Hud::default();
    for (start, end, frame) in blocks(map, streams, STATE_STREAM) {
        if let Some(block) = data.get(start..end) {
            hud.read(block, start, frame);
        }
    }
    let ids: Vec<u64> = players.iter().map(|p| p.id).filter(|&i| i != 0).collect();
    let movement: Vec<(usize, usize)> = blocks(map, streams, MOVEMENT_STREAM)
        .map(|(start, end, _)| (start, end))
        .collect();
    let entities = Entities::read(data, &movement, &ids);

    let mut out = Decoded {
        expected: players.len(),
        ..Decoded::default()
    };
    for (i, p) in players.iter().enumerate() {
        let slots = p.entities.as_ref().and_then(|e| hud.slots(e.controller));
        let view = p.entities.as_ref().and_then(|e| hud.view(e.controller));
        out.activity
            .extend(view.map(|v| crate::weapons::activity(&hud, v, &p.username, clock)));
        let fallback = p.entities.as_ref().and_then(|e| e.movement);
        let body = entities.body(p.id, fallback);
        if slots.is_none() && body.is_none() {
            out.warnings
                .push(format!("{}: no loadout and no body found", p.username));
            continue;
        }
        if slots.is_some() {
            out.resolved += 1;
        } else {
            out.warnings
                .push(format!("{}: no HUD loadout found", p.username));
        }
        if body.is_none() {
            out.warnings.push(format!(
                "{}: no body found, so the guns have no attachments",
                p.username
            ));
        }
        let [primary, secondary, ability, gadget] = slots.unwrap_or_default();
        let gun = |hud: Option<&Slot>, slot: Hash, name: &str, warnings: &mut Vec<String>| {
            // A shield has no entity of its own, though some bodies name
            // an asset for it (Fuze's).
            if hud.is_some_and(|s| !s.weapon) {
                return Gun::Empty;
            }
            let gun = body.map_or(Gun::Empty, |b| entities.gun(data, b, slot));
            if let Gun::Unlinked(asset) = gun {
                warnings.push(format!(
                    "{}: {name} weapon entity not linked (asset {asset})",
                    p.username
                ));
            }
            gun
        };
        let warnings = &mut out.warnings;
        let detail = Detail {
            primary: weapon(
                primary.as_ref(),
                &gun(primary.as_ref(), PRIMARY_WEAPON, "primary", warnings),
            ),
            secondary: weapon(
                secondary.as_ref(),
                &gun(secondary.as_ref(), SECONDARY_WEAPON, "secondary", warnings),
            ),
            ability: ability.as_ref().map(|s| counted(s, clock)),
            gadget: gadget.as_ref().map(|s| counted(s, clock)),
        };
        out.details.push((i, detail));
    }
    out
}

/// Files `detail` under the loadout of the operator `player` ended on,
/// adding one when the pick packets gave none.
pub(crate) fn apply(loadouts: &mut Vec<Loadout>, player: &Player, detail: Detail) {
    let at = loadouts
        .iter()
        .rposition(|l| l.username == player.username && l.operator == player.operator)
        .unwrap_or_else(|| {
            loadouts.push(Loadout::new(&player.username, player.operator));
            loadouts.len() - 1
        });
    let l = &mut loadouts[at];
    // The HUD names every slot; the items sent before a pick can miss one.
    let hud = detail.primary.iter().chain(&detail.secondary);
    if hud.clone().any(|w| w.id.is_some()) {
        l.weapons = hud.filter(|w| !w.shield).filter_map(|w| w.id).collect();
        let mut gadgets: Vec<u64> = Vec::new();
        for id in [&detail.ability, &detail.gadget]
            .into_iter()
            .flatten()
            .filter_map(|c| c.id)
        {
            if !gadgets.contains(&id) {
                gadgets.push(id);
            }
        }
        l.gadgets = gadgets;
    }
    l.primary = detail.primary;
    l.secondary = detail.secondary;
    l.ability = detail.ability;
    l.gadget = detail.gadget;
}

/// A value a loadout field changed from or to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ChangeValue {
    Operator(Operator),
    Item(Named),
}

/// One field of a loadout that differs from the round before on that side.
/// `from` and `to` are null for a slot that held nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FieldChange {
    /// `operator`, `primary`, `secondary`, `ability`, `gadget`, or an
    /// attachment as `primary.sight`.
    pub field: &'static str,
    pub from: Option<ChangeValue>,
    pub to: Option<ChangeValue>,
}

/// A round a player's loadout differs from their previous round on the
/// same side.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadoutChange {
    pub username: String,
    /// Round numbers count from 1.
    pub round: u32,
    pub previous_round: u32,
    pub side: TeamRole,
    pub changes: Vec<FieldChange>,
}

const ATTACHMENT_FIELDS: [[&str; 5]; 2] = [
    [
        "primary.sight",
        "primary.barrel",
        "primary.grip",
        "primary.underbarrel",
        "primary.magazine",
    ],
    [
        "secondary.sight",
        "secondary.barrel",
        "secondary.grip",
        "secondary.underbarrel",
        "secondary.magazine",
    ],
];

/// What differs between two loadouts of one player.
fn differences(before: &Loadout, after: &Loadout) -> Vec<FieldChange> {
    let mut out = Vec::new();
    let item = |id: Option<u64>| id.map(|id| ChangeValue::Item(Named::item(id)));
    let attachment = |a: Option<Named>| a.map(ChangeValue::Item);
    if before.operator != after.operator {
        out.push(FieldChange {
            field: "operator",
            from: Some(ChangeValue::Operator(before.operator)),
            to: Some(ChangeValue::Operator(after.operator)),
        });
    }
    let weapons = [
        ("primary", &before.primary, &after.primary),
        ("secondary", &before.secondary, &after.secondary),
    ];
    for (i, (field, a, b)) in weapons.into_iter().enumerate() {
        let (from, to) = (a.as_ref().and_then(|w| w.id), b.as_ref().and_then(|w| w.id));
        if from != to {
            out.push(FieldChange {
                field,
                from: item(from),
                to: item(to),
            });
            continue;
        }
        // Attachment ids are options of one gun, so they compare only on
        // the same gun, and only when both guns' entities were found.
        let (Some(a), Some(b)) = (a, b) else { continue };
        if !a.linked || !b.linked || a.asset != b.asset {
            continue;
        }
        let of = |w: &Weapon| [w.sight, w.barrel, w.grip, w.underbarrel, w.magazine];
        for ((from, to), field) in of(a).into_iter().zip(of(b)).zip(ATTACHMENT_FIELDS[i]) {
            if from != to {
                out.push(FieldChange {
                    field,
                    from: attachment(from),
                    to: attachment(to),
                });
            }
        }
    }
    let counted = [
        ("ability", &before.ability, &after.ability),
        ("gadget", &before.gadget, &after.gadget),
    ];
    for (field, a, b) in counted {
        let (from, to) = (a.as_ref().and_then(|c| c.id), b.as_ref().and_then(|c| c.id));
        // An ability follows from the operator, except for operators who
        // choose a gadget in its place.
        if from != to && (field != "ability" || before.operator == after.operator) {
            out.push(FieldChange {
                field,
                from: item(from),
                to: item(to),
            });
        }
    }
    out
}

/// Every round in which a player's loadout differs from the one of their
/// previous round on the same side. `rounds` are in play order. Rounds
/// without a decoded loadout for the player are passed over.
pub fn changes(rounds: &[Round]) -> Vec<LoadoutChange> {
    let mut last: HashMap<(&str, TeamRole), (u32, &Loadout)> = HashMap::new();
    let mut out = Vec::new();
    for r in rounds {
        let number = r.header.round_number + 1;
        for p in &r.header.players {
            let Some(side) = r.header.teams.get(p.team_index).and_then(|t| t.role) else {
                continue;
            };
            let Some(loadout) = r.loadouts.iter().rev().find(|l| {
                l.username == p.username
                    && l.operator == p.operator
                    && (l.primary.is_some() || l.secondary.is_some())
            }) else {
                continue;
            };
            let key = if p.key.is_empty() {
                p.username.as_str()
            } else {
                p.key.as_str()
            };
            let Some((previous_round, before)) = last.insert((key, side), (number, loadout)) else {
                continue;
            };
            let changes = differences(before, loadout);
            if !changes.is_empty() {
                out.push(LoadoutChange {
                    username: p.username.clone(),
                    round: number,
                    previous_round,
                    side,
                    changes,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `617385fe` payload for `entity` with these type hashes and slots.
    fn descriptor_bytes(
        entity: u64,
        hashes: &[Hash],
        asset: u64,
        slots: &[(Hash, u64)],
    ) -> Vec<u8> {
        let mut d = DESCRIPTOR.to_vec();
        d.extend(entity.to_le_bytes());
        // Position and rotation: 28 bytes, then 1.0.
        d.extend([0; 28]);
        d.extend(1f32.to_le_bytes());
        d.push(0);
        d.extend(0xF000_0001u64.to_le_bytes());
        d.extend((hashes.len() as u32).to_le_bytes());
        for h in hashes {
            d.extend(h);
        }
        d.extend(asset.to_le_bytes());
        d.extend(0u32.to_le_bytes());
        d.extend((slots.len() as u32).to_le_bytes());
        for (slot, item) in slots {
            d.extend(item.to_le_bytes());
            d.extend(slot);
            d.extend(0u32.to_le_bytes());
        }
        d.extend([0; 17]);
        d
    }

    /// A movement block holding these `(entity, payload)` messages.
    fn block(messages: &[(u64, Vec<u8>)]) -> Vec<u8> {
        let mut d = (messages.len() as u16).to_le_bytes().to_vec();
        for (entity, payload) in messages {
            d.extend(entity.to_le_bytes());
            d.extend((payload.len() as u32).to_le_bytes());
            d.extend(payload);
        }
        d
    }

    /// A `607385fe` update of `size` bytes holding `inside` and ending in
    /// `tail`.
    fn update(size: usize, inside: u64, tail: u64) -> Vec<u8> {
        let mut d = UPDATE.to_vec();
        d.resize(size, 0xAB);
        d[10..18].copy_from_slice(&inside.to_le_bytes());
        d[size - 8..].copy_from_slice(&tail.to_le_bytes());
        d
    }

    const BODY: u64 = 0xF02B_8AEF;
    const GUN: u64 = 0xF02B_8979;

    #[test]
    fn parses_a_descriptor() {
        let bytes = descriptor_bytes(
            GUN,
            &[[1, 2, 3, 4], [5, 6, 7, 8]],
            393596493099,
            &[(SIGHT, 258614298894), (BARREL, 258614298875), (GRIP, 0)],
        );
        let d = descriptor(&bytes).unwrap();
        assert_eq!((d.entity, d.asset), (GUN, 393596493099));
        assert_eq!(d.slot(SIGHT), Some(258614298894));
        assert_eq!(d.slot(GRIP), Some(0), "the gun has the slot, empty");
        assert_eq!(d.slot(UNDERBARREL), None, "the gun has no such slot");
        assert_eq!(d.attachments(), [258614298894, 258614298875, 0, 0, 0]);
        assert!(!d.is_body());
    }

    #[test]
    fn a_descriptor_without_type_hashes_or_slots_parses() {
        let d = descriptor(&descriptor_bytes(7, &[], 9, &[])).unwrap();
        assert_eq!((d.entity, d.asset, d.slots.len()), (7, 9, 0));
    }

    #[test]
    fn malformed_descriptors_are_rejected() {
        let good = descriptor_bytes(GUN, &[[1, 2, 3, 4]], 9, &[(SIGHT, 1), (BARREL, 2)]);
        assert!(descriptor(&good).is_some());
        // Another payload type.
        let mut other = good.clone();
        other[0] = 0x60;
        assert!(descriptor(&other).is_none());
        // Cut anywhere before the end of its slots.
        let slots_end = good.len() - 17;
        for cut in 0..slots_end {
            assert!(descriptor(&good[..cut]).is_none(), "cut at {cut}");
        }
        // Counts no descriptor has.
        let mut hashes = good.clone();
        hashes[53..57].copy_from_slice(&17u32.to_le_bytes());
        assert!(descriptor(&hashes).is_none());
        let mut slots = good.clone();
        let count_at = 57 + 4 + 12;
        slots[count_at..count_at + 4].copy_from_slice(&65u32.to_le_bytes());
        assert!(descriptor(&slots).is_none());
    }

    #[test]
    fn messages_stop_at_one_the_block_cannot_hold() {
        let mut d = block(&[(1, vec![1, 2, 3]), (2, vec![4])]);
        let all: Vec<_> = messages(&d).map(|(e, _, p)| (e, p.to_vec())).collect();
        assert_eq!(all, [(1, vec![1, 2, 3]), (2, vec![4])]);
        // A count of three with two messages, and a size past the end.
        d[0] = 3;
        assert_eq!(messages(&d).count(), 2);
        d[2 + 8] = 0xFF;
        assert_eq!(messages(&d).count(), 0);
        assert_eq!(messages(&[]).count(), 0);
    }

    #[test]
    fn a_gun_is_linked_to_the_body_it_names() {
        let player = 0x1122_3344_5566_7788;
        let (other_body, other_gun) = (0xF02B_8A3A, 0xF02B_88A3);
        let body = |entity| descriptor_bytes(entity, &[], 1, &[(PRIMARY_WEAPON, 500)]);
        let gun = |entity, sight| descriptor_bytes(entity, &[], 500, &[(SIGHT, sight)]);
        let data = block(&[
            (BODY, body(BODY)),
            (other_body, body(other_body)),
            (GUN, gun(GUN, 11)),
            (other_gun, gun(other_gun, 22)),
            (BODY, update(80, 0, player)),
            // Too small to name a body, then one that does.
            (GUN, update(40, BODY, 0)),
            (GUN, update(80, BODY, 0)),
            (other_gun, update(80, other_body, 0)),
        ]);
        let e = Entities::read(&data, &[(0, data.len())], &[player]);

        let found = e.body(player, None).unwrap();
        assert_eq!(found.entity, BODY);
        assert_eq!(
            e.gun(&data, found, PRIMARY_WEAPON),
            Gun::Linked {
                asset: 500,
                attachments: [11, 0, 0, 0, 0]
            }
        );
        assert_eq!(e.gun(&data, found, SECONDARY_WEAPON), Gun::Empty);
        // A body no update gives to a player is found through the player
        // table's link.
        assert!(e.body(1, None).is_none());
        let other = e.body(1, Some(other_body as u32)).unwrap();
        assert_eq!(
            e.gun(&data, other, PRIMARY_WEAPON),
            Gun::Linked {
                asset: 500,
                attachments: [22, 0, 0, 0, 0]
            }
        );
    }

    #[test]
    fn guns_that_name_no_body_link_only_when_they_agree() {
        let body = descriptor_bytes(BODY, &[], 1, &[(PRIMARY_WEAPON, 500)]);
        let gun = |entity, sight| {
            (
                entity,
                descriptor_bytes(entity, &[], 500, &[(SIGHT, sight)]),
            )
        };
        let read = |sights: [u64; 2]| {
            let data = block(&[(BODY, body.clone()), gun(1, sights[0]), gun(2, sights[1])]);
            let e = Entities::read(&data, &[(0, data.len())], &[]);
            let body = e.get(BODY).unwrap();
            e.gun(&data, body, PRIMARY_WEAPON)
        };
        assert_eq!(
            read([11, 11]),
            Gun::Linked {
                asset: 500,
                attachments: [11, 0, 0, 0, 0]
            }
        );
        assert_eq!(read([11, 22]), Gun::Unlinked(500));
    }

    /// `23 <obj> 00000000 <hash> <size> <value>`.
    fn set(d: &mut Vec<u8>, obj: u32, hash: Hash, value: &[u8]) {
        d.push(0x23);
        d.extend(obj.to_le_bytes());
        d.extend([0; 4]);
        d.extend(hash);
        d.push(value.len() as u8);
        d.extend(value);
    }

    /// `22 <hash> <size> <value>`.
    fn prop(d: &mut Vec<u8>, hash: Hash, value: &[u8]) {
        d.push(0x22);
        d.extend(hash);
        d.push(value.len() as u8);
        d.extend(value);
    }

    /// `1b <parent> 00000000 <field> <child> 00000000 <class>`.
    fn link(d: &mut Vec<u8>, parent: u32, field: Hash, child: u32, class: Hash) {
        d.push(0x1B);
        d.extend(parent.to_le_bytes());
        d.extend([0; 4]);
        d.extend(field);
        d.extend(child.to_le_bytes());
        d.extend([0; 4]);
        d.extend(class);
    }

    const CONTROLLER: u32 = 0xF000_0001;
    const VIEW: u32 = 0xF000_0002;
    const OTHER: Hash = [9, 9, 9, 9];

    /// Links a slot of `class` to `field` of the view, holding `item`.
    fn slot(d: &mut Vec<u8>, field: Hash, obj: u32, class: Hash, item: u64) {
        link(d, VIEW, field, obj, class);
        link(d, obj, ITEM_OBJECT_FIELD, obj + 1, OTHER);
        link(d, obj + 1, ITEM_DATA_FIELD, obj + 2, OTHER);
        set(d, obj + 2, ITEM_ID, &item.to_le_bytes());
    }

    fn frame(hud: &mut Hud, frame: u32, obj: u32, values: &[(Hash, u32)]) {
        let mut d = vec![];
        for (i, (hash, v)) in values.iter().enumerate() {
            if i == 0 {
                set(&mut d, obj, *hash, &v.to_le_bytes());
            } else {
                prop(&mut d, *hash, &v.to_le_bytes());
            }
        }
        hud.read(&d, 1000 * frame as usize, Some(frame));
    }

    #[test]
    fn a_gadget_starts_at_the_count_in_force_when_its_capacity_shows() {
        let gadget = 0xF000_0010;
        let mut d = vec![];
        link(&mut d, CONTROLLER, LOADOUT_FIELD, VIEW, OTHER);
        slot(&mut d, GADGET_FIELD, gadget, GADGET_VIEW, 77);
        // Stale values in the snapshot, before the player spawns.
        set(&mut d, gadget, AMMO, &9u32.to_le_bytes());
        prop(&mut d, MAX_AMMO, &0u32.to_le_bytes());
        let mut hud = Hud::default();
        hud.read(&d, 0, None);
        frame(&mut hud, 5, gadget, &[(AMMO, 2), (MAX_AMMO, 2)]);
        frame(&mut hud, 6, gadget, &[(AMMO, 2)]);
        frame(&mut hud, 9, gadget, &[(AMMO, 1)]);
        frame(&mut hud, 12, gadget, &[(AMMO, 3)]);

        let [primary, _, ability, gadget] = hud.slots(CONTROLLER).unwrap();
        assert!(primary.is_none() && ability.is_none());
        let gadget = gadget.unwrap();
        assert_eq!(
            (gadget.item, gadget.weapon, gadget.max),
            (Some(77), false, Some(2))
        );
        let values: Vec<u32> = gadget.counts.iter().map(|s| s.value).collect();
        assert_eq!(values, [2, 1, 3], "resends are dropped");
        assert_eq!(totals(&gadget.counts), (1, 2));
        assert_eq!(gadget.counts[1].frame, Some(9));
    }

    #[test]
    fn the_last_link_of_a_slot_field_wins() {
        let (stale, gun, ammo) = (0xF000_0010, 0xF000_0020, 0xF000_0030);
        let mut d = vec![];
        link(&mut d, CONTROLLER, LOADOUT_FIELD, VIEW, OTHER);
        slot(&mut d, PRIMARY_FIELD, stale, WEAPON_VIEW, 1);
        // The operator swap links a new slot, and an empty link in between
        // does not undo it.
        slot(&mut d, PRIMARY_FIELD, gun, WEAPON_VIEW, 2);
        link(&mut d, VIEW, PRIMARY_FIELD, 0, WEAPON_VIEW);
        link(&mut d, gun, OTHER, ammo, WEAPON_AMMO);
        set(&mut d, ammo, TOTAL_AMMO, &181u32.to_le_bytes());
        prop(&mut d, MAGAZINE_SIZE, &30u32.to_le_bytes());
        let mut hud = Hud::default();
        hud.read(&d, 0, None);
        frame(&mut hud, 3, ammo, &[(TOTAL_AMMO, 180)]);
        frame(&mut hud, 4, ammo, &[(TOTAL_AMMO, 151)]);

        let [primary, ..] = hud.slots(CONTROLLER).unwrap();
        let w = weapon(primary.as_ref(), &Gun::Unlinked(5)).unwrap();
        assert_eq!((w.id, w.asset, w.linked), (Some(2), Some(5), false));
        assert_eq!(
            w.ammo,
            Some(Ammo {
                magazine_size: Some(30),
                start: 181,
                end: 151,
                fired: 30
            })
        );
    }

    #[test]
    fn a_gadget_in_the_primary_slot_is_a_shield() {
        let slot = Slot {
            item: Some(419258819322),
            ..Slot::default()
        };
        let w = weapon(Some(&slot), &Gun::Empty).unwrap();
        assert!(w.shield && w.asset.is_none() && w.ammo.is_none());
        assert_eq!(w.name, Some("CCE Shield"));
        assert_eq!(weapon(None, &Gun::Empty), None);
    }

    fn loadout(operator: u64, primary: u64, sight: u64, gadget: u64) -> Loadout {
        let mut l = Loadout::new("a", Operator(operator));
        l.primary = Some(Weapon {
            id: Some(primary),
            asset: Some(primary + 1),
            sight: Named::attachment(sight),
            linked: true,
            ..Weapon::default()
        });
        l.gadget = Some(Counted {
            id: Some(gadget),
            ..Counted::default()
        });
        l
    }

    #[test]
    fn differences_name_the_fields_that_changed() {
        let fields = |a: &Loadout, b: &Loadout| -> Vec<&str> {
            differences(a, b).iter().map(|c| c.field).collect()
        };
        let base = loadout(1, 10, 100, 1000);
        assert!(differences(&base, &base.clone()).is_empty());
        assert_eq!(fields(&base, &loadout(1, 10, 101, 1000)), ["primary.sight"]);
        assert_eq!(
            fields(&base, &loadout(1, 10, 0, 1001)),
            ["primary.sight", "gadget"]
        );
        // Another gun's attachments are other ids: only the gun is listed.
        assert_eq!(
            fields(&base, &loadout(2, 11, 101, 1000)),
            ["operator", "primary"]
        );
        // Attachments of a gun whose entity was not found are unknown.
        let mut unlinked = loadout(1, 10, 0, 1000);
        unlinked.primary.as_mut().unwrap().linked = false;
        assert!(differences(&base, &unlinked).is_empty());
        let removed = &differences(&base, &loadout(1, 10, 0, 1000))[0];
        assert_eq!(removed.to, None);
        assert_eq!(
            serde_json::to_string(removed).unwrap(),
            r#"{"field":"primary.sight","from":{"id":100},"to":null}"#
        );
    }
}
