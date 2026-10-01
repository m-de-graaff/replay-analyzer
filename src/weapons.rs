//! What each player held, fired and reloaded (Y11S3), from the HUD objects
//! of the `state` stream.
//!
//! The `PlayerLoadoutViewModel` a controller links (see [`crate::loadout`])
//! says what is in the player's hands, and each gun slot's
//! `WeaponAmmoViewModel` splits the gun's ammunition:
//!
//! ```text
//! PlayerLoadoutViewModel
//!   6fd528eb ActiveReticleType   u32  what is in hand: 0 nothing, 1 the
//!                                     drone, else the slot's own number
//!   slot (WeaponViewModel or GadgetViewModel)
//!     66ac7724 EquippedWeaponType  u32  the slot's number: 2 primary,
//!                                       3 secondary, 4 ability, 5 gadget
//!     f8885fd5 IsReloading         u8   weapons only
//!     WeaponAmmoViewModel
//!       77ca96de AmmoInWeapon      u32  magazine plus the chambered round
//!       6d5b6d3e AmmoLeft          u32  reserve; written when a reload
//!                                       moves ammunition, not per shot
//!       400ac829 TotalAmmo         u32  the two added up
//!       8a836285 -> WeaponReticleViewModel
//!         4f6db478 IsAiming        u8
//! ```
//!
//! Nothing is in hand (0) between two items, while the player is on a
//! drone or camera, and once they are dead. The game has no flag for a
//! weapon swap or a cancelled reload; both are read off these values:
//!
//! - A swap is the hands going from one item to another, and it is in
//!   progress while they are empty in between.
//! - A reload is one pulse of `IsReloading`. It completed when the gun
//!   holds more than before. A reload of a gun that is not empty is two
//!   pulses a few frames apart: the magazine comes out (the gun keeps the
//!   chambered round and the reserve takes the rest), then the new one
//!   goes in. They are given as one reload. A pulse that changes nothing
//!   was cancelled; one shorter than [`NOISE`] is left out.
//!
//! Melee and fire mode are not in the HUD: the knife is no slot, and no
//! property changes with the fire mode.

use serde::Serialize;

use crate::details::{LifeEvent, LifeEventType};
use crate::entities::Hash;
use crate::feedback::{MatchUpdate, MatchUpdateType};
use crate::loadout::{Clock, Hud, SLOT_FIELDS, Sample, TOTAL_AMMO, When, distinct};
use crate::types::item_name;

const ACTIVE_RETICLE_TYPE: Hash = [0x6F, 0xD5, 0x28, 0xEB];
const EQUIPPED_WEAPON_TYPE: Hash = [0x66, 0xAC, 0x77, 0x24];
const AMMO_IN_WEAPON: Hash = [0x77, 0xCA, 0x96, 0xDE];
const AMMO_LEFT: Hash = [0x6D, 0x5B, 0x6D, 0x3E];
const IS_RELOADING: Hash = [0xF8, 0x88, 0x5F, 0xD5];
/// Ammunition object -> its `WeaponReticleViewModel`.
const RETICLE_FIELD: Hash = [0x8A, 0x83, 0x62, 0x85];
const IS_AIMING: Hash = [0x4F, 0x6D, 0xB4, 0x78];

/// The `u32` and `u8` properties the HUD reader keeps for this module.
pub(crate) const COUNTERS: [Hash; 4] = [
    ACTIVE_RETICLE_TYPE,
    EQUIPPED_WEAPON_TYPE,
    AMMO_IN_WEAPON,
    AMMO_LEFT,
];
pub(crate) const FLAGS: [Hash; 2] = [IS_RELOADING, IS_AIMING];

/// Hands left empty for longer than this between two items are not a swap
/// in progress: the player was on a drone or camera in between. Swaps
/// between guns take about a second.
const SWAP_LIMIT: f64 = 3.0;
/// The two pulses of one reload are at most this far apart.
const RELOAD_JOIN: f64 = 0.5;
/// A reload pulse shorter than this that moves no ammunition is not a
/// reload: the game raises the flag for a few frames on an empty gun.
const NOISE: f64 = 0.35;
/// The HUD empties a player's hands up to 0.07 s before the kill feed names
/// their death; what they were doing is read this long before they fell.
const DEATH_LEAD: f64 = 0.15;

/// What a player has in their hands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Item {
    /// Nothing: between two items, on a drone or camera, or dead.
    #[default]
    None,
    /// The drone, about to be thrown.
    Drone,
    /// The primary slot: a gun, or a shield.
    Primary,
    Secondary,
    /// The ability slot: the operator's gadget or launcher.
    Ability,
    Gadget,
}

impl Item {
    const SLOTS: [Item; 4] = [Item::Primary, Item::Secondary, Item::Ability, Item::Gadget];
}

/// A change of what a player holds.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Held {
    pub item: Item,
    /// The item's id and name, as in `loadouts`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
    #[serde(flatten)]
    pub when: When,
}

/// One item put away for another. `when` is the moment the old item left
/// the hands.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Swap {
    pub from: Item,
    pub to: Item,
    /// Seconds the hands were empty in between; 0 when the game went
    /// straight from one item to the other.
    pub duration: f64,
    #[serde(flatten)]
    pub when: When,
}

/// A drop of a gun's ammunition: one shot, or several fired between two
/// updates.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fired {
    pub slot: Item,
    pub rounds: u32,
    /// Rounds left in the gun after it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magazine: Option<u32>,
    /// Rounds left in reserve.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserve: Option<u32>,
    /// Whether the player was aiming down sights.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aiming: Option<bool>,
    #[serde(flatten)]
    pub when: When,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReloadOutcome {
    /// The gun holds more than before.
    Completed,
    /// The reload ended without the gun gaining a round. When the
    /// magazine was already out, `magazineAfter` is what stayed in the gun.
    Cancelled,
    /// The reload never ended: the player died, or the recording stopped.
    Unfinished,
}

/// One reload. `when` is its start.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reload {
    pub slot: Item,
    pub outcome: ReloadOutcome,
    /// Seconds from start to finish.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magazine_before: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magazine_after: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserve_after: Option<u32>,
    #[serde(flatten)]
    pub when: When,
}

/// What a player was doing with their weapons at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Handling {
    /// What they held. `none` in the middle of a swap takes the item being
    /// put away from `swappingFrom`.
    pub held: Item,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
    /// Rounds in the gun held.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magazine: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub reloading: bool,
    /// The item being put away, when a swap was in progress.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swapping_from: Option<Item>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swapping_to: Option<Item>,
}

/// A span of the recording, in seconds, with the frames it runs between.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Span {
    start: f64,
    end: f64,
}

/// One player's weapon handling over the round.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub username: String,
    pub held: Vec<Held>,
    pub swaps: Vec<Swap>,
    pub fired: Vec<Fired>,
    pub reloads: Vec<Reload>,
    /// What the player was doing when they were killed, or downed before
    /// being killed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_death: Option<Handling>,
    /// `(seconds, item)` of every change of the item in hand.
    #[serde(skip)]
    hands: Vec<(f64, Item)>,
    /// Swaps in progress: `(span, from, to)`.
    #[serde(skip)]
    swapping: Vec<(Span, Item, Item)>,
    /// Reload pulses: `(span, slot)`.
    #[serde(skip)]
    reloading: Vec<(Span, Item)>,
    /// `(seconds, slot, rounds in the gun)` of every change.
    #[serde(skip)]
    magazines: Vec<(f64, Item, u32)>,
    /// `(slot, item id)`.
    #[serde(skip)]
    items: Vec<(Item, u64)>,
}

impl Activity {
    /// The item id in `slot`.
    fn id(&self, slot: Item) -> Option<u64> {
        self.items.iter().find(|i| i.0 == slot).map(|i| i.1)
    }

    /// Fills `at_death` from the kill feed entry that names the player as
    /// its target. A player who was downed first has empty hands from
    /// then on, so what counts is the moment they went down.
    pub(crate) fn died(&mut self, feed: &[MatchUpdate], life: &[LifeEvent]) {
        let death = feed
            .iter()
            .filter(|u| u.kind == MatchUpdateType::Kill && u.target == self.username)
            .find_map(|u| u.recording_time);
        let Some(death) = death else { return };
        let fell = life
            .iter()
            .filter(|e| e.username == self.username)
            .filter_map(|e| Some((e.recording_time?, e.kind)))
            .rfind(|e| e.0 <= death)
            .filter(|e| e.1 == LifeEventType::Down)
            .map_or(death, |e| e.0);
        self.at_death = Some(self.at(fell - DEATH_LEAD));
    }

    /// What the player was doing `seconds` into the recording.
    pub fn at(&self, seconds: f64) -> Handling {
        let last = |times: &[(f64, Item)]| {
            let i = times.partition_point(|t| t.0 <= seconds);
            i.checked_sub(1).map(|i| times[i].1)
        };
        let swap = (self.swapping.iter()).find(|s| s.0.start <= seconds && seconds < s.0.end);
        let held = last(&self.hands).unwrap_or_default();
        let id = self.id(held);
        Handling {
            held,
            id,
            name: id.and_then(item_name),
            magazine: (self.magazines.iter())
                .rev()
                .find(|m| m.0 <= seconds && m.1 == held)
                .map(|m| m.2),
            reloading: (self.reloading.iter())
                .any(|r| r.0.start <= seconds && seconds < r.0.end && r.1 == held),
            swapping_from: swap.map(|s| s.1),
            swapping_to: swap.map(|s| s.2),
        }
    }
}

/// The value in force in `frame`: the last one written in it or before.
fn in_frame(series: &[Sample], frame: Option<u32>) -> Option<u32> {
    let i = series.partition_point(|s| s.frame <= frame);
    i.checked_sub(1).map(|i| series[i].value)
}

/// The value in force just before offset `at`.
fn before(series: &[Sample], at: usize) -> Option<u32> {
    let i = series.partition_point(|s| s.at < at);
    i.checked_sub(1).map(|i| series[i].value)
}

/// One pulse of `IsReloading`, with the gun's ammunition around it.
struct Pulse {
    start: Sample,
    end: Option<Sample>,
    magazine: (Option<u32>, Option<u32>),
    reserve: (Option<u32>, Option<u32>),
    /// What stayed in the gun once the magazine was out, when the pulse
    /// that put the new one in was joined to this one.
    emptied: Option<u32>,
}

/// Reads the weapon handling of the player whose loadout view is `view`.
pub(crate) fn activity(hud: &Hud, view: u32, username: &str, clock: &Clock) -> Activity {
    let mut out = Activity {
        username: username.to_owned(),
        ..Activity::default()
    };
    let seconds = |s: Sample| clock.seconds(s.frame).unwrap_or(0.0);
    let round = |t: f64| (t * 1000.0).round() / 1000.0;
    let slots = SLOT_FIELDS.map(|f| hud.slot_object(view, f));
    // A slot states its own number; the item in hand is given by it.
    let numbers: Vec<(u32, Item)> = (slots.iter().zip(Item::SLOTS).zip(2u32..))
        .map(|((slot, item), default)| {
            let own = slot.and_then(|(o, _)| hud.series(o, EQUIPPED_WEAPON_TYPE).next());
            (own.map_or(default, |s| s.value), item)
        })
        .collect();
    let item_of = |value: u32| match value {
        0 => Item::None,
        1 => Item::Drone,
        v => (numbers.iter().find(|n| n.0 == v)).map_or(Item::None, |n| n.1),
    };
    for (slot, item) in slots.iter().zip(Item::SLOTS) {
        out.items
            .extend(slot.and_then(|(o, _)| hud.item(o)).map(|id| (item, id)));
    }

    let mut hands: Vec<(Sample, Item)> = Vec::new();
    for s in hud.series(view, ACTIVE_RETICLE_TYPE) {
        let item = item_of(s.value);
        if hands.last().is_none_or(|l| l.1 != item) {
            hands.push((s, item));
        }
    }
    for &(s, item) in &hands {
        let id = out.id(item);
        out.hands.push((seconds(s), item));
        out.held.push(Held {
            item,
            id,
            name: id.and_then(item_name),
            when: clock.when(s.at, s.frame),
        });
    }
    for (i, w) in hands.windows(2).enumerate() {
        let ((_, from), (emptied, to)) = (w[0], w[1]);
        if from == Item::None {
            continue;
        }
        if to != Item::None {
            out.swaps.push(Swap {
                from,
                to,
                duration: 0.0,
                when: clock.when(emptied.at, emptied.frame),
            });
            continue;
        }
        // Hands empty: a swap when another item follows soon enough.
        let Some(&(next, to)) = hands.get(i + 2) else {
            continue;
        };
        let (start, end) = (seconds(emptied), seconds(next));
        if to != from && end - start <= SWAP_LIMIT {
            out.swapping.push((Span { start, end }, from, to));
            out.swaps.push(Swap {
                from,
                to,
                duration: round(end - start),
                when: clock.when(emptied.at, emptied.frame),
            });
        }
    }

    for (slot, item) in slots.iter().zip(Item::SLOTS) {
        let Some((object, true)) = *slot else {
            continue;
        };
        let Some(ammo) = hud.ammo_object(object) else {
            continue;
        };
        let magazine = distinct(hud.series(ammo, AMMO_IN_WEAPON));
        let reserve = distinct(hud.series(ammo, AMMO_LEFT));
        let aiming: Vec<Sample> = (hud.child(ammo, RETICLE_FIELD))
            .map(|r| distinct(hud.series(r, IS_AIMING)))
            .unwrap_or_default();
        (out.magazines).extend(magazine.iter().map(|s| (seconds(*s), item, s.value)));

        let total = distinct(hud.series(ammo, TOTAL_AMMO));
        for w in total.windows(2).filter(|w| w[1].value < w[0].value) {
            out.fired.push(Fired {
                slot: item,
                rounds: w[0].value - w[1].value,
                magazine: in_frame(&magazine, w[1].frame),
                reserve: in_frame(&reserve, w[1].frame),
                aiming: in_frame(&aiming, w[1].frame).map(|a| a != 0),
                when: clock.when(w[1].at, w[1].frame),
            });
        }

        let flag = distinct(hud.series(object, IS_RELOADING));
        let mut pulses: Vec<Pulse> = Vec::new();
        for (i, start) in flag.iter().enumerate().filter(|(_, s)| s.value != 0) {
            let end = flag.get(i + 1).copied();
            let after = |series: &[Sample]| match end {
                Some(e) => in_frame(series, e.frame),
                None => series.last().map(|s| s.value),
            };
            let pulse = Pulse {
                start: *start,
                end,
                magazine: (before(&magazine, start.at), after(&magazine)),
                reserve: (before(&reserve, start.at), after(&reserve)),
                emptied: None,
            };
            // The magazine came out in the pulse before: one reload.
            if let Some(last) = pulses.last_mut()
                && let Some(out_at) = last.end
                && last.magazine.1 < last.magazine.0
                && seconds(*start) - seconds(out_at) <= RELOAD_JOIN
            {
                last.emptied = last.magazine.1;
                last.end = pulse.end;
                last.magazine.1 = pulse.magazine.1;
                last.reserve.1 = pulse.reserve.1;
            } else {
                pulses.push(pulse);
            }
        }
        for p in pulses {
            let duration = p.end.map(|e| seconds(e) - seconds(p.start));
            let unchanged = p.magazine.0 == p.magazine.1 && p.reserve.0 == p.reserve.1;
            if unchanged && duration.is_some_and(|d| d < NOISE) {
                continue;
            }
            let span = Span {
                start: seconds(p.start),
                end: p.end.map_or(f64::MAX, seconds),
            };
            out.reloading.push((span, item));
            out.reloads.push(Reload {
                slot: item,
                outcome: match p.end {
                    None => ReloadOutcome::Unfinished,
                    Some(_) if p.magazine.1 > p.emptied.or(p.magazine.0).min(p.magazine.0) => {
                        ReloadOutcome::Completed
                    }
                    Some(_) => ReloadOutcome::Cancelled,
                },
                duration: duration.map(round),
                magazine_before: p.magazine.0,
                magazine_after: p.magazine.1,
                reserve_after: p.reserve.1,
                when: clock.when(p.start.at, p.start.frame),
            });
        }
    }
    let time = |w: &When| w.recording_time.unwrap_or(0.0);
    out.fired
        .sort_by(|a, b| time(&a.when).total_cmp(&time(&b.when)));
    out.reloads
        .sort_by(|a, b| time(&a.when).total_cmp(&time(&b.when)));
    out.magazines.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}
