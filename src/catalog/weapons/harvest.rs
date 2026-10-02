//! What parsed rounds say about each gun: the observed side of the catalog.
//!
//! Everything here is measured from [`Round`]s and nothing is looked up, so
//! the result can be held against the static table (see
//! [`Observed::disagreements`]) or used to rebuild it after a patch.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use super::{EXTENDED_BARREL, Slot, WeaponClass, WeaponInfo};
use crate::loadout::Weapon;
use crate::round::Round;
use crate::shots::HitResult;
use crate::weapons::Item;

/// Seconds between two updates of the movement stream, which is as fine as
/// a shot's time gets: shot times of the test rounds and of real ones step
/// by this much.
pub const UPDATE_INTERVAL: f64 = 0.034;
/// Consecutive shots of one player at most this far apart belong to one
/// burst. The slowest automatic gun (the ACS12, 300 rounds a minute) fires
/// every 0.2 s.
pub const BURST_GAP: f64 = 0.25;
/// The fewest shots a burst needs to be measured: over fewer, the one
/// update a burst's length can be off by is more than 6% of it.
pub const MIN_BURST: usize = 8;
/// A burst is steady when its longest and shortest gap differ by no more
/// than one update (and a millisecond of slack per end): the trigger was
/// held, or pulled as fast as the gun allows.
pub const STEADY_SPREAD: f64 = UPDATE_INTERVAL + 0.011;
/// The fewest steady bursts for a gun to get a fire rate.
pub const MIN_BURSTS: usize = 3;
/// The fewest hits that must share one damage for it to count as the gun's
/// damage, and the share of the gun's hits they must be.
pub const MIN_DAMAGE_HITS: u32 = 3;
pub const MIN_DAMAGE_SHARE: f64 = 0.5;

/// A value with how often it was seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Tally<T> {
    pub value: T,
    pub count: u32,
}

/// An attachment id seen on a gun, with the name the attachment table has
/// for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeenAttachment {
    pub slot: Slot,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
    /// The name is inferred, not read (see [`crate::types::attachment_info`]).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub inferred: bool,
    /// Loadouts that carried it.
    pub count: u32,
}

/// A gun's fire rate, from the spacing of its shots.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FireRate {
    /// Steady bursts of at least [`MIN_BURST`] shots.
    pub bursts: u32,
    /// The median of those bursts' rates, in rounds a minute. Absent with
    /// fewer than [`MIN_BURSTS`] bursts. For an automatic gun this is its
    /// rate of fire; for any other it is how fast players pulled the
    /// trigger, a lower bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpm: Option<f64>,
    /// The slowest and the fastest steady burst.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slowest: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fastest: Option<f64>,
    /// How far one burst's rate can be off, as a share of it: one update
    /// over the length of the median burst.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<f64>,
    /// The shortest time between two shots of one player, in seconds. Never
    /// finer than an update.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shortest_gap: Option<f64>,
}

/// The damage one target health took from a gun.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthDamage {
    /// The victim's maximum health: 100, 110 or 125.
    pub max_health: u32,
    /// The damage most hits did, with how many.
    pub value: u32,
    pub samples: u32,
    /// All hits on victims of this health, and how many of them did the
    /// gun's damage ([`Damage::value`]; 0 when the gun has none).
    pub hits: u32,
    pub full: u32,
}

/// A gun's damage per bullet, from hits that left the victim standing.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Damage {
    /// Hits counted: not on a limb, and fired through a barrel that is
    /// known not to be an extended one.
    pub hits: u32,
    /// The damage at least [`MIN_DAMAGE_HITS`] and half of those hits
    /// share: the gun's damage. Absent when no value has both.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<u32>,
    /// How many hits did exactly `value`.
    pub samples: u32,
    /// The most common values of those hits, most common first.
    pub body: Vec<Tally<u32>>,
    /// The same hits by the victim's maximum health.
    pub by_health: Vec<HealthDamage>,
    /// Limb hits through the same barrels.
    pub limb: Vec<Tally<u32>>,
    /// Hits not on a limb through an extended barrel.
    pub extended_barrel: Vec<Tally<u32>>,
}

/// What rounds show of one gun.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observed {
    /// The item id of loadouts and the kill feed.
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
    /// Loadouts that carried it.
    pub carried: u32,
    /// The operators that carried it.
    pub operators: Vec<Tally<String>>,
    /// Each `MagazineSize` the HUD gave it.
    pub magazine: Vec<Tally<u32>>,
    /// The most rounds a player had in the gun above its magazine size,
    /// per player who fired it: 1 for a round in the chamber.
    pub chambered: Vec<Tally<u32>>,
    pub attachments: Vec<SeenAttachment>,
    pub shots: u32,
    pub fire: FireRate,
    pub damage: Damage,
}

#[derive(Default)]
struct Gun {
    carried: u32,
    operators: BTreeMap<String, u32>,
    magazine: BTreeMap<u32, u32>,
    chambered: BTreeMap<u32, u32>,
    attachments: BTreeMap<(Slot, u64), u32>,
    shots: u32,
    /// `(shots, seconds)` of each burst worth measuring.
    bursts: Vec<(usize, f64)>,
    shortest_gap: Option<f64>,
    body: Vec<(u32, u32)>,
    limb: BTreeMap<u32, u32>,
    extended: BTreeMap<u32, u32>,
}

/// Collects what rounds show of each gun, one round at a time, so a folder
/// of matches need not be held in memory.
#[derive(Default)]
pub struct Harvester {
    guns: BTreeMap<u64, Gun>,
    rounds: u32,
}

/// What `rounds` show of each gun they hold, by item id.
pub fn harvest(rounds: &[Round]) -> Vec<Observed> {
    let mut harvester = Harvester::default();
    for round in rounds {
        harvester.add(round);
    }
    harvester.finish()
}

impl Harvester {
    /// Rounds added so far.
    pub fn rounds(&self) -> u32 {
        self.rounds
    }

    pub fn add(&mut self, round: &Round) {
        self.rounds += 1;
        // The barrel on each player's guns, for the damage of their hits.
        let mut barrels: HashMap<(&str, u64), Option<&'static str>> = HashMap::new();
        for loadout in &round.loadouts {
            let slots = [
                (Item::Primary, &loadout.primary),
                (Item::Secondary, &loadout.secondary),
            ];
            for (slot, weapon) in slots {
                let Some(weapon) = weapon else { continue };
                let Some(id) = weapon.id.filter(|_| !weapon.shield) else {
                    continue;
                };
                let gun = self.guns.entry(id).or_default();
                gun.carried += 1;
                let operator = match loadout.operator.name() {
                    Some(name) => name.to_owned(),
                    None => loadout.operator.0.to_string(),
                };
                *gun.operators.entry(operator).or_default() += 1;
                for (slot, attachment) in attachments(weapon) {
                    *gun.attachments.entry((slot, attachment)).or_default() += 1;
                }
                barrels.insert(
                    (loadout.username.as_str(), id),
                    weapon.barrel.and_then(|b| b.name),
                );
                let Some(size) = weapon.ammo.and_then(|a| a.magazine_size) else {
                    continue;
                };
                *gun.magazine.entry(size).or_default() += 1;
                let most = round
                    .weapon_activity
                    .iter()
                    .filter(|a| a.username == loadout.username)
                    .flat_map(|a| &a.fired)
                    .filter(|f| f.slot == slot)
                    .filter_map(|f| Some(f.magazine? + f.rounds))
                    .max();
                if let Some(most) = most {
                    *gun.chambered.entry(most.saturating_sub(size)).or_default() += 1;
                }
            }
        }

        let mut times: HashMap<(&str, u64), Vec<f64>> = HashMap::new();
        for shot in &round.shots {
            let Some((user, id)) = shooter(shot) else {
                continue;
            };
            self.guns.entry(id).or_default().shots += 1;
            if let Some(at) = shot.when.recording_time {
                times.entry((user, id)).or_default().push(at);
            }
        }
        for ((_, id), mut times) in times {
            times.sort_by(f64::total_cmp);
            let gun = self.guns.entry(id).or_default();
            let mut start = 0;
            for i in 1..=times.len() {
                if i < times.len() {
                    let gap = times[i] - times[i - 1];
                    if gap > 0.0 && gun.shortest_gap.is_none_or(|g| gap < g) {
                        gun.shortest_gap = Some(gap);
                    }
                    if gap <= BURST_GAP {
                        continue;
                    }
                }
                if let Some(burst) = steady(&times[start..i]) {
                    gun.bursts.push(burst);
                }
                start = i;
            }
        }

        let health: HashMap<&str, u32> = round
            .header
            .players
            .iter()
            .filter_map(|p| Some((p.username.as_str(), p.max_health?)))
            .collect();
        for hit in &round.bullet_hits {
            let (Some(damage), Some(limb), Some(HitResult::Alive)) =
                (hit.damage, hit.limb, hit.result)
            else {
                continue;
            };
            let Some(shot) = hit.shot.and_then(|i| round.shots.get(i)) else {
                continue;
            };
            let Some((user, id)) = shooter(shot) else {
                continue;
            };
            // A barrel the table has no name for may be an extended one.
            let Some(barrel) = barrels.get(&(user, id)).copied().flatten() else {
                continue;
            };
            let gun = self.guns.entry(id).or_default();
            if barrel == EXTENDED_BARREL {
                if !limb {
                    *gun.extended.entry(damage).or_default() += 1;
                }
            } else if limb {
                *gun.limb.entry(damage).or_default() += 1;
            } else {
                let max = health.get(hit.victim.as_str()).copied().unwrap_or(0);
                gun.body.push((damage, max));
            }
        }
    }

    pub fn finish(self) -> Vec<Observed> {
        self.guns
            .into_iter()
            .map(|(id, gun)| Observed {
                id,
                name: crate::types::item_name(id),
                carried: gun.carried,
                operators: tallies(gun.operators),
                magazine: tallies(gun.magazine),
                chambered: tallies(gun.chambered),
                attachments: gun
                    .attachments
                    .into_iter()
                    .map(|((slot, id), count)| {
                        let info = crate::types::attachment_info(id);
                        SeenAttachment {
                            slot,
                            id,
                            name: info.name,
                            inferred: info.inferred,
                            count,
                        }
                    })
                    .collect(),
                shots: gun.shots,
                fire: fire_rate(gun.bursts, gun.shortest_gap),
                damage: damage(&gun.body, gun.limb, gun.extended),
            })
            .collect()
    }
}

/// The player and the gun of a shot from a primary or secondary.
fn shooter(shot: &crate::shots::Shot) -> Option<(&str, u64)> {
    matches!(shot.slot, Some("primary" | "secondary"))
        .then_some(())
        .and(shot.username.as_deref())
        .zip(shot.weapon.map(|w| w.id))
}

fn attachments(weapon: &Weapon) -> impl Iterator<Item = (Slot, u64)> {
    [
        (Slot::Sight, weapon.sight),
        (Slot::Barrel, weapon.barrel),
        (Slot::Grip, weapon.grip),
        (Slot::Underbarrel, weapon.underbarrel),
    ]
    .into_iter()
    .filter_map(|(slot, named)| Some((slot, named?.id)))
}

/// `(shots, seconds)` of a burst long and even enough to give a rate.
fn steady(times: &[f64]) -> Option<(usize, f64)> {
    if times.len() < MIN_BURST {
        return None;
    }
    let gaps = times.windows(2).map(|w| w[1] - w[0]);
    let shortest = gaps.clone().fold(f64::INFINITY, f64::min);
    let longest = gaps.fold(0.0, f64::max);
    // Two shots in one update are one shot read twice, or two players'
    // shots under one name.
    (shortest > UPDATE_INTERVAL / 2.0 && longest - shortest <= STEADY_SPREAD)
        .then(|| (times.len(), times[times.len() - 1] - times[0]))
}

fn rpm(burst: (usize, f64)) -> f64 {
    60.0 * (burst.0 - 1) as f64 / burst.1
}

fn fire_rate(mut bursts: Vec<(usize, f64)>, shortest_gap: Option<f64>) -> FireRate {
    bursts.sort_by(|a, b| rpm(*a).total_cmp(&rpm(*b)));
    let enough = bursts.len() >= MIN_BURSTS;
    let median = bursts.get(bursts.len() / 2).copied().filter(|_| enough);
    FireRate {
        bursts: bursts.len() as u32,
        rpm: median.map(|m| match bursts.len() % 2 {
            0 => (rpm(m) + rpm(bursts[bursts.len() / 2 - 1])) / 2.0,
            _ => rpm(m),
        }),
        slowest: bursts.first().map(|b| rpm(*b)),
        fastest: bursts.last().map(|b| rpm(*b)),
        precision: median.map(|m| UPDATE_INTERVAL / m.1),
        shortest_gap,
    }
}

fn damage(body: &[(u32, u32)], limb: BTreeMap<u32, u32>, extended: BTreeMap<u32, u32>) -> Damage {
    let count = |hits: &mut dyn Iterator<Item = u32>| {
        let mut counts = BTreeMap::new();
        for damage in hits {
            *counts.entry(damage).or_default() += 1;
        }
        tallies(counts)
    };
    let all = count(&mut body.iter().map(|h| h.0));
    let top = all.first().copied().filter(|t| {
        t.count >= MIN_DAMAGE_HITS && f64::from(t.count) >= MIN_DAMAGE_SHARE * body.len() as f64
    });
    let gun = top.map(|t| t.value);
    let by_health = [100, 110, 125]
        .into_iter()
        .filter_map(|max| {
            let of = body.iter().filter(|h| h.1 == max);
            let top = count(&mut of.clone().map(|h| h.0)).first().copied()?;
            Some(HealthDamage {
                max_health: max,
                value: top.value,
                samples: top.count,
                hits: of.clone().count() as u32,
                full: of.filter(|h| Some(h.0) == gun).count() as u32,
            })
        })
        .collect();
    Damage {
        hits: body.len() as u32,
        value: gun,
        samples: top.map_or(0, |t| t.count),
        body: all,
        by_health,
        limb: tallies(limb),
        extended_barrel: tallies(extended),
    }
}

/// Most seen first; equal counts by value.
fn tallies<T: Ord>(counts: BTreeMap<T, u32>) -> Vec<Tally<T>> {
    let mut out: Vec<Tally<T>> = counts
        .into_iter()
        .map(|(value, count)| Tally { value, count })
        .collect();
    out.sort_by_key(|t| std::cmp::Reverse(t.count));
    out
}

impl Observed {
    /// The magazine size most loadouts had.
    pub fn magazine_size(&self) -> Option<u32> {
        self.magazine.first().map(|t| t.value)
    }

    /// Whether players had a round in the chamber on top of a full
    /// magazine, when anyone fired the gun from full.
    pub fn chamber(&self) -> Option<bool> {
        self.chambered.iter().map(|t| t.value).max().map(|m| m > 0)
    }

    /// Where these observations contradict a catalog entry. `tolerance` is
    /// the share a measured fire rate may exceed the catalog's by (see
    /// [`FireRate::precision`]).
    ///
    /// - The magazine size is read, so it must be equal.
    /// - A fire rate is checked for guns the catalog calls automatic: a
    ///   steady burst of any other gun is the player's trigger finger.
    /// - The damage most hits did may not be above the catalog's. Pellet
    ///   shotguns are left out: a shell's hit carries what its pellets did
    ///   together.
    pub fn disagreements(&self, info: &WeaponInfo, tolerance: f64) -> Vec<String> {
        let mut out = Vec::new();
        let name = info.name;
        if let (Some(seen), Some(listed)) = (self.magazine_size(), info.magazine.value)
            && seen != listed
        {
            out.push(format!("{name}: magazine of {seen}, catalog {listed}"));
        }
        if let (Some(seen), Some(listed)) = (self.fire.rpm, info.rpm.value)
            && info.is_automatic()
            && seen > f64::from(listed) * (1.0 + tolerance)
        {
            out.push(format!(
                "{name}: {seen:.0} rounds a minute over {} bursts, catalog {listed}",
                self.fire.bursts
            ));
        }
        if let (Some(seen), Some(listed)) = (self.damage.value, info.damage.value)
            && info.class != Some(WeaponClass::Shotgun)
            && seen > listed
        {
            out.push(format!(
                "{name}: {seen} damage on {} of {} hits, catalog {listed}",
                self.damage.samples, self.damage.hits
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_needs_length_and_even_gaps() {
        let even: Vec<f64> = (0..10).map(|i| f64::from(i) * 0.075).collect();
        let (shots, seconds) = steady(&even).unwrap();
        assert_eq!(shots, 10);
        assert!((rpm((shots, seconds)) - 800.0).abs() < 0.01);
        assert_eq!(steady(&even[..7]), None);
        let mut uneven = even.clone();
        uneven[9] += 0.1;
        assert_eq!(steady(&uneven), None);
        let mut doubled = even;
        doubled[5] = doubled[4];
        assert_eq!(steady(&doubled), None);
    }

    #[test]
    fn fire_rate_is_the_median_burst() {
        let rate = fire_rate(vec![(11, 1.0), (11, 0.75), (11, 0.8)], Some(0.068));
        assert_eq!(rate.bursts, 3);
        assert_eq!(rate.rpm, Some(750.0));
        assert_eq!(rate.slowest, Some(600.0));
        assert_eq!(rate.fastest, Some(800.0));
        assert_eq!(rate.precision, Some(UPDATE_INTERVAL / 0.8));
        assert_eq!(fire_rate(vec![(11, 1.0), (11, 0.8)], None).rpm, None);
    }

    #[test]
    fn damage_is_what_most_hits_did() {
        let body = [(32, 100), (32, 110), (32, 125), (64, 110), (22, 110)];
        let found = damage(&body, BTreeMap::from([(24, 2)]), BTreeMap::from([(35, 1)]));
        assert_eq!((found.value, found.samples, found.hits), (Some(32), 3, 5));
        assert_eq!(found.by_health.len(), 3);
        assert_eq!((found.by_health[1].hits, found.by_health[1].full), (3, 1));
        assert_eq!(
            found.limb[0],
            Tally {
                value: 24,
                count: 2
            }
        );
        // Two of five agreeing is no majority.
        let split = [(32, 100), (32, 110), (30, 125), (64, 110), (22, 110)];
        assert_eq!(damage(&split, BTreeMap::new(), BTreeMap::new()).value, None);
    }
}
