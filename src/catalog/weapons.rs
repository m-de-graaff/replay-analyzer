//! A catalog of the guns of Y11S3: class, damage, fire rate, magazine,
//! fire modes and attachments, keyed by the item id loadouts and the kill
//! feed use, with time-to-kill helpers on top.
//!
//! A replay holds no weapon statistics. What it does hold is what each gun
//! did: the HUD's `MagazineSize` and the rounds in the gun, the attachment
//! ids on it, the time of every shot and the health every bullet took. The
//! catalog is grounded in that: each number says whether it was `observed`
//! in replays (with how many samples) or comes from a `reference`, and
//! where the two disagree it keeps both.
//!
//! **Observed**, by [`harvest`] over 188 rounds (the 10 test rounds and a
//! real folder of 178, builds 9883691, 9901603 and 9918362):
//!
//! - *Magazine*: `loadouts[].primary.ammo.magazineSize`, read. A round in
//!   the chamber shows as `AmmoInWeapon` one above it.
//! - *Fire rate*: the median rate of a gun's steady bursts of 8 shots or
//!   more. A shot's time is an update of the movement stream, 34 ms apart,
//!   so one burst of 20 rounds at 800 a minute is good to 2.4% and the
//!   median of many bursts to about 1%. It is the rate of fire for
//!   automatic guns only; for any other gun it measures the player.
//! - *Damage*: the damage most hits did that were not on a limb, left the
//!   victim standing and were not fired through an extended barrel. Hits
//!   that down or kill carry no damage, and head and torso are not told
//!   apart, so a headshot is never a sample. Range is not divided out:
//!   most fights are closer than any fall-off starts.
//! - *Attachments*: the ids on the guns players carried. Their names come
//!   from [`crate::types::attachment_info`], most of them inferred.
//!
//! **Reference**: the weapon table of r6data.com as of 2026-10-02 (class,
//! damage, fire rate, magazine and the attachment options a gun offers),
//! with the Y11S3 patch notes over it. Semi-automatic and pump guns have
//! no fire rate there, and none here. Fire modes are in neither: the file
//! holds no fire mode (see the README), so they follow from the class.
//!
//! The rules the helpers apply were checked on the same rounds:
//!
//! - Armor takes no damage off. A gun did its usual damage in 76%, 80%
//!   and 74% of its hits on targets of 100, 110 and 125 health; armor is
//!   the health itself.
//! - A limb takes three quarters, rounded down ([`LIMB_MULTIPLIER`]): so
//!   for 39 of 41 guns with limb hits enough to tell.
//! - An extended barrel adds 12%, rounded down
//!   ([`EXTENDED_BARREL_MULTIPLIER`]): so for 7 of 9 guns seen with one.
//!   The P90 (22 to 29) and the 9mm C1 (36 to 46) did more, on 4 and 2
//!   hits.
//! - A headshot kills, from the reference: the file marks a headshot on
//!   kills only.

use serde::Serialize;

use crate::types::ItemKind;

mod harvest;
mod table;

pub use harvest::{
    BURST_GAP, Damage, FireRate, Harvester, HealthDamage, MIN_BURST, MIN_BURSTS, MIN_DAMAGE_HITS,
    MIN_DAMAGE_SHARE, Observed, STEADY_SPREAD, SeenAttachment, Tally, UPDATE_INTERVAL, harvest,
};

/// The name the attachment table gives the barrel that adds damage.
pub const EXTENDED_BARREL: &str = "Extended Barrel";
/// What an extended barrel multiplies a bullet's damage by; the result is
/// rounded down. Observed: 32 becomes 35, 26 becomes 29, 40 becomes 44.
pub const EXTENDED_BARREL_MULTIPLIER: f64 = 1.12;
/// What a hit on an arm or leg multiplies a bullet's damage by; the result
/// is rounded down. Observed: 47 becomes 35, 37 becomes 27, 26 becomes 19.
pub const LIMB_MULTIPLIER: f64 = 0.75;
/// The maximum health of the three armor ratings, lightest first. A Rook
/// plate adds to it.
pub const TARGET_HEALTH: [u32; 3] = [100, 110, 125];

/// Where a number comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// Measured in replays.
    Observed,
    /// From the reference table; replays do not contradict it.
    Reference,
    /// Neither has it.
    Unknown,
}

/// One number of a gun: the value the catalog goes by, where it comes
/// from, and both sides when there are two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stat<T> {
    /// The observed value where there is one, else the reference's. A fire
    /// rate is the reference's when the measured one is within its
    /// precision of it, since the reference is exact and a measurement is
    /// not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<T>,
    pub source: Source,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed: Option<T>,
    /// What `observed` rests on: loadouts for a magazine, players who
    /// fired for the chamber, steady bursts for a fire rate, hits of
    /// exactly that damage for damage.
    #[serde(skip_serializing_if = "is_zero")]
    pub samples: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<T>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl<T: Copy> Stat<T> {
    pub const UNKNOWN: Self = Stat {
        value: None,
        source: Source::Unknown,
        observed: None,
        samples: 0,
        reference: None,
    };

    /// Observed, and the reference has nothing.
    pub const fn observed(value: T, samples: u32) -> Self {
        Stat {
            value: Some(value),
            source: Source::Observed,
            observed: Some(value),
            samples,
            reference: None,
        }
    }

    /// From the reference only.
    pub const fn reference(value: T) -> Self {
        Stat {
            value: Some(value),
            source: Source::Reference,
            observed: None,
            samples: 0,
            reference: Some(value),
        }
    }

    /// Observed, with what the reference says: the same, or not.
    pub const fn both(observed: T, samples: u32, reference: T) -> Self {
        Stat {
            value: Some(observed),
            source: Source::Observed,
            observed: Some(observed),
            samples,
            reference: Some(reference),
        }
    }

    /// The reference's value, which a measurement of `observed` confirms
    /// within its precision.
    pub const fn confirmed(reference: T, observed: T, samples: u32) -> Self {
        Stat {
            value: Some(reference),
            source: Source::Reference,
            observed: Some(observed),
            samples,
            reference: Some(reference),
        }
    }
}

impl<T: Copy + PartialEq> Stat<T> {
    /// Both sides have a value and they differ.
    pub fn disagrees(&self) -> bool {
        matches!((self.observed, self.reference), (Some(o), Some(r)) if o != r)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WeaponClass {
    AssaultRifle,
    SubmachineGun,
    LightMachineGun,
    MarksmanRifle,
    SniperRifle,
    /// Fires shells of pellets.
    Shotgun,
    /// A shotgun that fires one slug.
    SlugShotgun,
    MachinePistol,
    Handgun,
    Revolver,
    /// The Gonne-6.
    HandCannon,
    /// A held shield in the primary slot.
    Shield,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FireMode {
    FullAuto,
    SemiAuto,
    PumpAction,
    BoltAction,
}

/// A slot of a gun that takes an attachment. Magazines are a slot in the
/// file too, with one id per gun and no choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Slot {
    Sight,
    Barrel,
    Grip,
    Underbarrel,
}

/// An attachment seen on a gun in replays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentOption {
    /// The attachment's id on this gun: the same model on another gun has
    /// another id.
    pub id: u64,
    /// From the attachment table. Most sights have none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
    /// The name is inferred, not read.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub inferred: bool,
    /// Loadouts that carried it.
    pub samples: u32,
}

/// What can go in one slot of a gun.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentSlot {
    pub slot: Slot,
    /// Observed: the ids players had in the slot, most carried first. An
    /// id named `None` is the empty slot.
    pub seen: &'static [AttachmentOption],
    /// Reference: the names of the options the gun offers. They carry no
    /// ids, and a sight's name cannot be matched to an id.
    pub reference: &'static [&'static str],
}

/// A gun, or a shield carried as a primary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeaponInfo {
    /// The item id of loadouts and the kill feed. Absent for a gun of the
    /// reference that no replay has shown, so that has no id yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub name: &'static str,
    /// The loadout slot the item table puts it in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<ItemKind>,
    /// Reference. Absent for a gun the reference does not list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<WeaponClass>,
    /// Health a bullet takes off a torso, at close range, without an
    /// extended barrel. For a pellet shotgun the reference's figure for
    /// one pellet, which replays cannot check.
    pub damage: Stat<u32>,
    /// Rounds a minute. Absent for guns that are not automatic.
    pub rpm: Stat<u32>,
    /// Rounds in a full magazine, the chambered one not counted.
    pub magazine: Stat<u32>,
    /// A reload with a round left puts one more in the gun than the
    /// magazine holds.
    pub chambered: Stat<bool>,
    /// Reference, from the class: replays hold no fire mode. A selector's
    /// other settings (burst, single) are not listed.
    pub fire_modes: &'static [FireMode],
    pub attachments: &'static [AttachmentSlot],
    /// Loadouts that carried it in the rounds the catalog was built from.
    #[serde(skip_serializing_if = "is_zero")]
    pub carried: u32,
}

impl WeaponInfo {
    /// The most rounds the gun holds: a full magazine, and one in the
    /// chamber where the gun keeps one.
    pub fn capacity(&self) -> Option<u32> {
        let chambered = self.chambered.value.unwrap_or(false);
        self.magazine.value.map(|m| m + u32::from(chambered))
    }

    pub fn is_automatic(&self) -> bool {
        self.fire_modes.contains(&FireMode::FullAuto)
    }

    /// Whether the gun was seen with an extended barrel, or the reference
    /// lists one for it.
    pub fn takes_extended_barrel(&self) -> bool {
        self.attachments
            .iter()
            .filter(|a| a.slot == Slot::Barrel)
            .any(|a| {
                a.reference.contains(&EXTENDED_BARREL)
                    || a.seen.iter().any(|o| o.name == Some(EXTENDED_BARREL))
            })
    }

    /// Hits on the torso to bring a target of `health` to zero, and the
    /// seconds from the first to the last at the gun's fire rate. `None`
    /// without a damage; `seconds` is absent without a fire rate.
    pub fn time_to_kill(&self, health: u32, extended_barrel: bool) -> Option<TimeToKill> {
        let damage = hit_damage(
            self.damage.value?,
            extended_barrel,
            HitLocation::Body,
            health,
        );
        let shots = shots_to_kill(damage, health)?;
        Some(TimeToKill {
            health,
            shots,
            seconds: self.rpm.value.and_then(|rpm| time_to_kill(shots, rpm)),
        })
    }

    /// [`WeaponInfo::time_to_kill`] for each of [`TARGET_HEALTH`].
    pub fn times_to_kill(&self, extended_barrel: bool) -> Vec<TimeToKill> {
        TARGET_HEALTH
            .iter()
            .filter_map(|h| self.time_to_kill(*h, extended_barrel))
            .collect()
    }
}

/// Every entry of the catalog: the guns of the item table in its order,
/// then the guns only the reference knows.
pub fn all() -> &'static [WeaponInfo] {
    table::WEAPONS
}

/// The gun with this item id.
pub fn weapon_info(id: u64) -> Option<&'static WeaponInfo> {
    all().iter().find(|w| w.id == Some(id))
}

/// The gun with this name, as the item table spells it; case is ignored.
pub fn weapon_by_name(name: &str) -> Option<&'static WeaponInfo> {
    all().iter().find(|w| w.name.eq_ignore_ascii_case(name))
}

/// Primaries and secondaries of the item table that no reference lists:
/// they have no class, and only what replays show of them.
pub fn unknown() -> &'static [u64] {
    table::UNKNOWN
}

/// Where a bullet strikes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HitLocation {
    /// Torso.
    Body,
    /// An arm or a leg.
    Limb,
    Head,
}

/// What armor multiplies bullet damage by: nothing. A heavier operator has
/// more health (100, 110 or 125) and takes the same damage: in the rounds
/// the catalog was built from, a gun did its usual damage in 76%, 80% and
/// 74% of its hits on the three.
pub const fn armor_multiplier(_max_health: u32) -> f64 {
    1.0
}

/// The health one bullet takes. `base` is the gun's damage; `health` is
/// what the target has left, which a headshot takes whole.
pub fn hit_damage(base: u32, extended_barrel: bool, location: HitLocation, health: u32) -> u32 {
    let scale = |damage: u32, by: f64| (f64::from(damage) * by + 1e-9).floor() as u32;
    let barrel = match extended_barrel {
        true => scale(base, EXTENDED_BARREL_MULTIPLIER),
        false => base,
    };
    match location {
        HitLocation::Body => barrel,
        HitLocation::Limb => scale(barrel, LIMB_MULTIPLIER),
        HitLocation::Head => health.max(barrel),
    }
}

/// Hits of `damage` each to bring `health` to zero. `None` when a hit does
/// no damage.
pub fn shots_to_kill(damage: u32, health: u32) -> Option<u32> {
    (damage > 0).then(|| health.div_ceil(damage).max(1))
}

/// Seconds from the first of `shots` to the last at `rpm` rounds a minute:
/// the first bullet leaves at once, so one shot takes no time. Travel time
/// and the time to aim are not in it.
pub fn time_to_kill(shots: u32, rpm: u32) -> Option<f64> {
    (shots > 0 && rpm > 0).then(|| f64::from(shots - 1) * 60.0 / f64::from(rpm))
}

/// Shots and time to bring one target down.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeToKill {
    /// The target's health.
    pub health: u32,
    pub shots: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
}

/// The share of hits that land on the torso, on a limb and on the head.
/// They need not add up to 1: they are scaled so that they do.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HitMix {
    pub body: f64,
    pub limb: f64,
    pub head: f64,
}

impl HitMix {
    /// Every hit on the torso.
    pub const BODY: HitMix = HitMix {
        body: 1.0,
        limb: 0.0,
        head: 0.0,
    };

    /// Torso hits with a share of headshots.
    pub fn headshots(share: f64) -> HitMix {
        let head = share.clamp(0.0, 1.0);
        HitMix {
            body: 1.0 - head,
            limb: 0.0,
            head,
        }
    }
}

/// The hits it takes on average to bring `health` to zero with a gun of
/// `base` damage, when each hit lands by `mix`: a headshot ends it, any
/// other hit takes its damage off. `None` when the mix is empty or its
/// hits do no damage.
pub fn expected_shots_to_kill(
    base: u32,
    extended_barrel: bool,
    health: u32,
    mix: HitMix,
) -> Option<f64> {
    let shares = [mix.body.max(0.0), mix.limb.max(0.0), mix.head.max(0.0)];
    let total: f64 = shares.iter().sum();
    if !(total > 0.0 && total.is_finite()) {
        return None;
    }
    let [body, limb, _] = shares.map(|s| s / total);
    let damage = |location| hit_damage(base, extended_barrel, location, health);
    let (on_body, on_limb) = (damage(HitLocation::Body), damage(HitLocation::Limb));
    if (body > 0.0 && on_body == 0) || (limb > 0.0 && on_limb == 0) {
        return None;
    }
    // expected[h]: hits still needed with h health left.
    let mut expected = vec![0.0; health as usize + 1];
    for left in 1..=health as usize {
        let after = |damage: u32| expected[left.saturating_sub(damage as usize)];
        expected[left] = 1.0 + body * after(on_body) + limb * after(on_limb);
    }
    Some(expected[health as usize])
}

/// [`expected_shots_to_kill`] as seconds at `rpm` rounds a minute: every
/// hit but the first waits for the gun to cycle. Misses are not counted;
/// divide by the share of shots that hit to allow for them.
pub fn expected_time_to_kill(
    base: u32,
    extended_barrel: bool,
    rpm: u32,
    health: u32,
    mix: HitMix,
) -> Option<f64> {
    let shots = expected_shots_to_kill(base, extended_barrel, health, mix)?;
    (rpm > 0).then(|| (shots - 1.0).max(0.0) * 60.0 / f64::from(rpm))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// `(id, name)` of every primary and secondary of the item table. The
    /// table is private to `types`, so its source is read.
    fn guns_of_the_item_table() -> Vec<(u64, String)> {
        include_str!("../types/tables.rs")
            .lines()
            .filter_map(|line| {
                let line = line.trim().strip_prefix('(')?;
                let line = line
                    .strip_suffix(", Primary),")
                    .or_else(|| line.strip_suffix(", Secondary),"))?;
                let (id, name) = line.split_once(", ")?;
                Some((id.parse().ok()?, name.trim_matches('"').to_owned()))
            })
            .collect()
    }

    #[test]
    fn ids_and_names_are_unique() {
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for weapon in all() {
            if let Some(id) = weapon.id {
                assert!(ids.insert(id), "{} twice", weapon.name);
            }
            assert!(
                names.insert(weapon.name.to_ascii_lowercase()),
                "{} twice",
                weapon.name
            );
        }
        assert!(unknown().iter().all(|id| ids.contains(id)));
    }

    #[test]
    fn every_gun_of_the_item_table_has_an_entry() {
        let guns = guns_of_the_item_table();
        assert!(guns.len() > 100, "{} guns parsed", guns.len());
        for (id, name) in &guns {
            let weapon = weapon_info(*id).unwrap_or_else(|| panic!("{name} ({id}) has no entry"));
            assert_eq!(weapon.name, name);
            assert_eq!(crate::types::item_name(*id), Some(weapon.name));
            assert_eq!(crate::types::item_kind(*id), weapon.slot);
            assert_eq!(weapon_by_name(name).and_then(|w| w.id), Some(*id));
            // An entry is either listed as unknown or says something.
            let known = weapon.class.is_some();
            assert_eq!(unknown().contains(id), !known, "{name}");
        }
        // Every entry with an id is a gun of the item table.
        let listed: HashSet<u64> = guns.iter().map(|g| g.0).collect();
        for weapon in all() {
            assert!(
                weapon.id.is_none_or(|id| listed.contains(&id)),
                "{}",
                weapon.name
            );
            assert!(
                weapon.id.is_some() || weapon.slot.is_none(),
                "{}",
                weapon.name
            );
        }
    }

    #[test]
    fn stats_say_where_they_come_from() {
        for weapon in all() {
            let name = weapon.name;
            for stat in [&weapon.damage, &weapon.rpm, &weapon.magazine] {
                match stat.source {
                    Source::Observed => {
                        assert_eq!(stat.value, stat.observed, "{name}");
                        assert!(stat.samples > 0, "{name}");
                    }
                    Source::Reference => assert_eq!(stat.value, stat.reference, "{name}"),
                    Source::Unknown => assert_eq!(*stat, Stat::UNKNOWN, "{name}"),
                }
                assert_eq!(stat.observed.is_some(), stat.samples > 0, "{name}");
                assert!(stat.value.is_none_or(|v| v > 0), "{name}");
            }
            // Only automatic guns have a fire rate, and a sane one.
            if let Some(rpm) = weapon.rpm.value {
                assert!(weapon.is_automatic(), "{name}");
                assert!((250..=1400).contains(&rpm), "{name}: {rpm}");
            }
            if weapon.class == Some(WeaponClass::Shield) {
                assert_eq!(weapon.damage, Stat::UNKNOWN, "{name}");
                assert!(weapon.fire_modes.is_empty(), "{name}");
            } else if weapon.class.is_some() {
                assert!(!weapon.fire_modes.is_empty(), "{name}");
            }
            // A seen attachment is seen once per slot, most carried first.
            for slot in weapon.attachments {
                let ids: HashSet<u64> = slot.seen.iter().map(|o| o.id).collect();
                assert_eq!(ids.len(), slot.seen.len(), "{name}");
                assert!(
                    slot.seen.is_sorted_by(|a, b| a.samples >= b.samples),
                    "{name}"
                );
                assert!(
                    !slot.seen.is_empty() || !slot.reference.is_empty(),
                    "{name}"
                );
            }
            let slots: HashSet<Slot> = weapon.attachments.iter().map(|a| a.slot).collect();
            assert_eq!(slots.len(), weapon.attachments.len(), "{name}");
        }
    }

    #[test]
    fn looks_guns_up() {
        let mp7 = weapon_info(1366019616).unwrap();
        assert_eq!(mp7.name, "MP7");
        assert_eq!(mp7.class, Some(WeaponClass::SubmachineGun));
        assert_eq!(mp7.slot, Some(ItemKind::Primary));
        assert_eq!(
            (mp7.damage.value, mp7.damage.source),
            (Some(32), Source::Observed)
        );
        assert_eq!(
            (mp7.rpm.value, mp7.rpm.source),
            (Some(900), Source::Reference)
        );
        assert!(mp7.rpm.observed.is_some_and(|rpm| rpm.abs_diff(900) < 15));
        assert_eq!(mp7.magazine.value, Some(30));
        assert_eq!(mp7.capacity(), Some(31));
        assert!(mp7.is_automatic() && mp7.takes_extended_barrel());
        assert_eq!(weapon_by_name("mp7"), Some(mp7));
        assert_eq!(weapon_info(1), None);
        assert_eq!(weapon_by_name("Knife"), None);
        // An open bolt: nothing in the chamber.
        assert_eq!(weapon_by_name("DP27").unwrap().capacity(), Some(70));
        // Only the reference has the G36C: no replay has shown its id.
        let g36c = weapon_by_name("G36C").unwrap();
        assert_eq!((g36c.id, g36c.damage.source), (None, Source::Reference));
    }

    #[test]
    fn serializes_camel_case() {
        let json = serde_json::to_value(weapon_info(1366019616).unwrap()).unwrap();
        assert_eq!(json["class"], "submachineGun");
        assert_eq!(json["fireModes"][0], "fullAuto");
        assert_eq!(json["magazine"]["source"], "observed");
        assert_eq!(json["rpm"]["reference"], 900);
        assert_eq!(json["attachments"][0]["slot"], "sight");
        assert!(json["attachments"][1]["seen"][0]["id"].is_u64());
    }

    #[test]
    fn a_hit_takes_what_its_place_allows() {
        use HitLocation::{Body, Head, Limb};
        assert_eq!(hit_damage(32, false, Body, 100), 32);
        assert_eq!(hit_damage(32, true, Body, 100), 35);
        assert_eq!(hit_damage(47, false, Limb, 100), 35);
        assert_eq!(hit_damage(37, false, Limb, 100), 27);
        // Barrel first, then the limb: 26 to 29 to 21, as observed.
        assert_eq!(hit_damage(26, true, Limb, 100), 21);
        assert_eq!(hit_damage(25, true, Body, 100), 28);
        assert_eq!(hit_damage(32, false, Head, 125), 125);
        assert_eq!(armor_multiplier(125), 1.0);
    }

    #[test]
    fn counts_shots_and_time() {
        assert_eq!(shots_to_kill(32, 100), Some(4));
        assert_eq!(shots_to_kill(32, 125), Some(4));
        assert_eq!(shots_to_kill(25, 100), Some(4));
        assert_eq!(shots_to_kill(25, 110), Some(5));
        assert_eq!(shots_to_kill(135, 100), Some(1));
        assert_eq!(shots_to_kill(0, 100), None);
        assert_eq!(time_to_kill(1, 900), Some(0.0));
        assert_eq!(time_to_kill(4, 900), Some(0.2));
        assert_eq!(time_to_kill(4, 0), None);

        let mp7 = weapon_by_name("MP7").unwrap();
        let ttk = mp7.times_to_kill(false);
        assert_eq!(ttk.iter().map(|t| t.shots).collect::<Vec<_>>(), [4, 4, 4]);
        assert_eq!(ttk[0].seconds, Some(0.2));
        // 35 a bullet with the extended barrel: one fewer on 100 health.
        assert_eq!(mp7.time_to_kill(100, true).unwrap().shots, 3);
        // A semi-automatic gun has shots and no time.
        let d50 = weapon_by_name("D-50")
            .unwrap()
            .time_to_kill(100, false)
            .unwrap();
        assert_eq!(d50.seconds, None);
    }

    #[test]
    fn a_mix_of_hits_averages_out() {
        let shots = |mix| expected_shots_to_kill(32, false, 100, mix).unwrap();
        assert_eq!(shots(HitMix::BODY), 4.0);
        assert_eq!(shots(HitMix::headshots(1.0)), 1.0);
        // Limbs only: 24 a hit, five hits.
        let limbs = HitMix {
            body: 0.0,
            limb: 1.0,
            head: 0.0,
        };
        assert_eq!(shots(limbs), 5.0);
        // One hit in four on the head: 1 + 0.75 + 0.75^2 + 0.75^3.
        let quarter = shots(HitMix::headshots(0.25));
        assert!((quarter - 2.734375).abs() < 1e-12, "{quarter}");
        // Shares are scaled to add up to 1.
        let scaled = HitMix {
            body: 3.0,
            limb: 0.0,
            head: 1.0,
        };
        assert_eq!(shots(scaled), quarter);
        let seconds = expected_time_to_kill(32, false, 900, 100, HitMix::headshots(0.25));
        assert!((seconds.unwrap() - 1.734375 / 15.0).abs() < 1e-12);
        let none = HitMix {
            body: 0.0,
            limb: 0.0,
            head: 0.0,
        };
        assert_eq!(expected_shots_to_kill(32, false, 100, none), None);
        assert_eq!(expected_shots_to_kill(1, false, 100, limbs), None);
        assert_eq!(expected_time_to_kill(32, false, 0, 100, HitMix::BODY), None);
    }
}
