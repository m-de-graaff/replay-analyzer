//! What each operator can bring, per season: side, role tags, armor and
//! speed, the unique ability with its count, and the gadgets, primaries and
//! secondaries a player chooses from.
//!
//! A replay holds one player's choice per round, never the menu. The
//! catalog is the menu, and every value in it says where it comes from:
//!
//! - [`Source::Observed`]: read from replays, with the number of loadouts
//!   (one player in one round) it was seen in. Item ids, the counts the HUD
//!   starts with, `MaxHealth` and the side a team played are of this kind.
//! - [`Source::Reference`]: not in any replay read so far. Role tags are
//!   never in a replay; an item no player picked and an operator no player
//!   played come from the game's own operator pages. A reference item has
//!   an id when another operator was seen carrying the same item.
//!
//! Replays hold no armor or speed rating. `MaxHealth` stands in for armor
//! (100, 110 or 125 for 1, 2 or 3), and speed is what is left of 4, so both
//! follow from an observed maximum and are absent without one.
//!
//! A count is what the HUD shows when the player spawns, which is not
//! always a number of devices: Buck's is the Skeleton Key's 36 shells,
//! Maverick's the torch's 320 units of fuel, Sledge's the hammer's 25
//! blows, and an ability that is on or off counts 1. An ability that
//! refills (`regenerates`) starts below what it can hold.
//!
//! [`observe`] harvests the observed side from parsed rounds; the season
//! tables are that harvest with the reference values added.

mod y11s3;

use std::collections::BTreeMap;

use serde::Serialize;

use crate::header::Player;
use crate::loadout::{Counted, Weapon};
use crate::round::Round;
use crate::types::{Operator, TeamRole};

/// Where a catalog value comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// Read from replays.
    Observed,
    /// From the game's operator pages; no replay read so far shows it.
    Reference,
}

/// A value's source and, when observed, how many loadouts showed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub source: Source,
    #[serde(skip_serializing_if = "is_zero")]
    pub rounds: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Evidence {
    pub const REFERENCE: Evidence = Evidence {
        source: Source::Reference,
        rounds: 0,
    };

    pub const fn observed(rounds: u32) -> Self {
        Evidence {
            source: Source::Observed,
            rounds,
        }
    }

    pub fn is_observed(self) -> bool {
        self.source == Source::Observed
    }
}

/// A gun, shield or gadget an operator can pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    /// The HUD item id `loadouts` and the kill feed use, when a replay
    /// has shown it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub name: &'static str,
    /// A gadget's count at spawn. Absent for guns, and for a gadget this
    /// operator was never seen with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    pub evidence: Evidence,
}

/// An operator's unique ability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub name: &'static str,
    /// What the HUD counts at spawn. Absent when the HUD shows no count
    /// (Skopos) or the operator was never seen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// The most the slot holds (the HUD's `MaxAmmo`), when it states one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
    /// The count refills over time, so it can rise above `count`.
    pub regenerates: bool,
    pub evidence: Evidence,
}

/// One operator in one season.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatorInfo {
    pub operator: Operator,
    pub side: TeamRole,
    pub side_evidence: Evidence,
    /// The game's role tags (`Breach`, `Anti-Gadget`, ...). Never in a
    /// replay, so always reference.
    pub roles: &'static [&'static str],
    /// 1 to 3, from `max_health`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub armor: Option<u8>,
    /// 1 to 3: `4 - armor`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<u8>,
    /// 100, 110 or 125, as `players[].maxHealth`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_health: Option<u32>,
    /// Evidence for `max_health`, and with it `armor` and `speed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_evidence: Option<Evidence>,
    /// Absent for Striker and Sentry, whose ability slot holds a second
    /// gadget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ability: Option<Ability>,
    /// How many of `gadgets` the operator brings: 1, or 2 for Striker and
    /// Sentry.
    pub gadget_slots: u8,
    pub gadgets: &'static [Item],
    /// A shield in the primary slot is listed here, under the id the
    /// loadout gives it.
    pub primaries: &'static [Item],
    pub secondaries: &'static [Item],
}

impl OperatorInfo {
    /// The gadget with this HUD item id, if the operator can pick it.
    pub fn gadget(&self, id: u64) -> Option<&'static Item> {
        find(self.gadgets, id)
    }

    pub fn primary(&self, id: u64) -> Option<&'static Item> {
        find(self.primaries, id)
    }

    pub fn secondary(&self, id: u64) -> Option<&'static Item> {
        find(self.secondaries, id)
    }
}

fn find(items: &'static [Item], id: u64) -> Option<&'static Item> {
    items.iter().find(|i| i.id == Some(id))
}

/// The armor rating a maximum health stands for.
pub fn armor_for_health(max_health: u32) -> Option<u8> {
    match max_health {
        100 => Some(1),
        110 => Some(2),
        125 => Some(3),
        _ => None,
    }
}

/// The operators of one season.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    /// `Y11S3`, as [`crate::GameVersion::season`].
    pub season: &'static str,
    /// Rounds the observed values were harvested from.
    pub rounds: u32,
    /// Loadouts in those rounds: one player in one round.
    pub loadouts: u32,
    pub operators: &'static [OperatorInfo],
}

impl Catalog {
    pub fn operator(&self, operator: Operator) -> Option<&'static OperatorInfo> {
        self.operators.iter().find(|o| o.operator == operator)
    }
}

/// Every season with a catalog, oldest first.
pub const SEASONS: &[&Catalog] = &[&y11s3::CATALOG];

/// The catalog of exactly this season (`Y11S3`).
pub fn catalog(season: &str) -> Option<&'static Catalog> {
    SEASONS.iter().copied().find(|c| c.season == season)
}

/// The newest season with a catalog.
pub fn latest() -> &'static Catalog {
    SEASONS[SEASONS.len() - 1]
}

/// A catalog picked for a season that may have none of its own.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolved {
    pub catalog: &'static Catalog,
    /// The season has no catalog, so this is the latest one: its values
    /// may not hold for the season asked for.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub fallback: bool,
}

/// The season's catalog, else the latest one with `fallback` set.
pub fn catalog_or_latest(season: Option<&str>) -> Resolved {
    match season.and_then(catalog) {
        Some(catalog) => Resolved {
            catalog,
            fallback: false,
        },
        None => Resolved {
            catalog: latest(),
            fallback: true,
        },
    }
}

/// The catalog for a round, by the season in its header.
pub fn catalog_for(round: &Round) -> Resolved {
    catalog_or_latest(round.version.season.as_deref())
}

/// What is known about an operator in exactly this season.
pub fn operator_info(season: &str, operator: Operator) -> Option<&'static OperatorInfo> {
    catalog(season)?.operator(operator)
}

/// As [`operator_info`], falling back to the latest season; the flag says
/// it did.
pub fn operator_info_or_latest(
    season: Option<&str>,
    operator: Operator,
) -> Option<(&'static OperatorInfo, bool)> {
    let resolved = catalog_or_latest(season);
    Some((resolved.catalog.operator(operator)?, resolved.fallback))
}

/// A value and the number of loadouts it was seen in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tally<T> {
    pub value: T,
    pub rounds: u32,
}

/// An item seen in one slot of an operator's loadouts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedItem {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
    pub rounds: u32,
    /// Abilities and gadgets: each count at spawn, most seen first. A
    /// player who never spawned has none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub starts: Vec<Tally<u32>>,
    /// The largest `MaxAmmo` the HUD stated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub regenerates: bool,
}

/// What a set of rounds shows of one operator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedOperator {
    pub operator: Operator,
    /// Loadouts of players who spawned with this operator.
    pub rounds: u32,
    /// The side of each of those players' teams, most seen first.
    pub sides: Vec<Tally<TeamRole>>,
    /// `players[].maxHealth`, most seen first.
    pub max_health: Vec<Tally<u32>>,
    pub abilities: Vec<ObservedItem>,
    pub gadgets: Vec<ObservedItem>,
    pub primaries: Vec<ObservedItem>,
    pub secondaries: Vec<ObservedItem>,
}

#[derive(Default)]
struct SeenItem {
    rounds: u32,
    starts: BTreeMap<u32, u32>,
    max: Option<u32>,
    regenerates: bool,
}

#[derive(Default)]
struct Seen {
    rounds: u32,
    sides: [u32; 2],
    max_health: BTreeMap<u32, u32>,
    /// Ability, gadget, primary, secondary.
    slots: [BTreeMap<u64, SeenItem>; 4],
}

/// Collects what rounds show of each operator, one round at a time, so a
/// folder can be harvested without keeping its rounds.
#[derive(Default)]
pub struct Observer {
    operators: BTreeMap<Operator, Seen>,
}

impl Observer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a round's loadouts. Only the entry of the operator a player
    /// spawned with counts: one an attacker swapped away from has no slots.
    pub fn add(&mut self, round: &Round) {
        for l in &round.loadouts {
            if l.primary.is_none() && l.ability.is_none() {
                continue;
            }
            let seen = self.operators.entry(l.operator).or_default();
            seen.rounds += 1;
            let player = round
                .header
                .players
                .iter()
                .find(|p| p.username == l.username && p.operator == l.operator);
            if let Some(role) = player.and_then(|p| side(round, p)) {
                seen.sides[role as usize] += 1;
            }
            if let Some(health) = player.and_then(|p| p.max_health) {
                *seen.max_health.entry(health).or_default() += 1;
            }
            let [abilities, gadgets, primaries, secondaries] = &mut seen.slots;
            counted(abilities, l.ability.as_ref());
            counted(gadgets, l.gadget.as_ref());
            weapon(primaries, l.primary.as_ref());
            weapon(secondaries, l.secondary.as_ref());
        }
    }

    /// Operators by id, each slot's items most seen first.
    pub fn finish(self) -> Vec<ObservedOperator> {
        self.operators
            .into_iter()
            .map(|(operator, seen)| {
                let [abilities, gadgets, primaries, secondaries] = seen.slots;
                let sides = [TeamRole::Attack, TeamRole::Defense]
                    .into_iter()
                    .map(|role| (role, seen.sides[role as usize]))
                    .filter(|s| s.1 > 0);
                ObservedOperator {
                    operator,
                    rounds: seen.rounds,
                    sides: tallies(sides),
                    max_health: tallies(seen.max_health),
                    abilities: items(abilities),
                    gadgets: items(gadgets),
                    primaries: items(primaries),
                    secondaries: items(secondaries),
                }
            })
            .collect()
    }
}

fn side(round: &Round, player: &Player) -> Option<TeamRole> {
    round.header.teams.get(player.team_index)?.role
}

fn counted(slot: &mut BTreeMap<u64, SeenItem>, item: Option<&Counted>) {
    let Some((id, item)) = item.and_then(|i| Some((i.id?, i))) else {
        return;
    };
    let seen = slot.entry(id).or_default();
    seen.rounds += 1;
    if let Some(counts) = &item.counts {
        *seen.starts.entry(counts.start).or_default() += 1;
        seen.max = seen.max.max(counts.max);
        seen.regenerates |= counts.regenerates;
    }
}

fn weapon(slot: &mut BTreeMap<u64, SeenItem>, item: Option<&Weapon>) {
    if let Some(id) = item.and_then(|w| w.id) {
        slot.entry(id).or_default().rounds += 1;
    }
}

/// Most seen first; ties keep the order of the values.
fn tallies<T>(counts: impl IntoIterator<Item = (T, u32)>) -> Vec<Tally<T>> {
    let mut out: Vec<Tally<T>> = counts
        .into_iter()
        .map(|(value, rounds)| Tally { value, rounds })
        .collect();
    out.sort_by_key(|t| std::cmp::Reverse(t.rounds));
    out
}

fn items(slot: BTreeMap<u64, SeenItem>) -> Vec<ObservedItem> {
    let mut out: Vec<ObservedItem> = slot
        .into_iter()
        .map(|(id, seen)| ObservedItem {
            id,
            name: crate::types::item_name(id),
            rounds: seen.rounds,
            starts: tallies(seen.starts),
            max: seen.max,
            regenerates: seen.regenerates,
        })
        .collect();
    out.sort_by_key(|i| std::cmp::Reverse(i.rounds));
    out
}

/// What the rounds show of each operator played in them: the observed
/// side of the catalog.
pub fn observe(rounds: &[Round]) -> Vec<ObservedOperator> {
    let mut observer = Observer::new();
    for round in rounds {
        observer.add(round);
    }
    observer.finish()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::details::Loadout;
    use crate::header::Team;
    use crate::loadout::Counts;
    use crate::types::{ItemKind, item_kind, item_name};

    const ACE: Operator = Operator(104189664390);
    const IANA: Operator = Operator(104189664038);

    #[test]
    fn looks_up_by_season_and_operator() {
        let ace = operator_info("Y11S3", ACE).unwrap();
        assert_eq!(ace.side, TeamRole::Attack);
        assert_eq!(
            (ace.armor, ace.speed, ace.max_health),
            (Some(3), Some(1), Some(125))
        );
        let ability = ace.ability.unwrap();
        assert_eq!(
            (ability.name, ability.count),
            ("S.E.L.M.A. Aqua Breacher", Some(3))
        );
        assert!(ability.evidence.is_observed());
        assert_eq!(
            ace.gadget(133651070240).map(|g| (g.name, g.count)),
            Some(("Claymore", Some(2)))
        );
        assert_eq!(ace.primary(1366019220).map(|g| g.name), Some("AK-12"));
        assert!(operator_info("Y11S3", Operator::RECRUIT).is_none());
        assert!(operator_info("Y9S1", ACE).is_none());
    }

    #[test]
    fn an_unknown_season_falls_back_to_the_latest_flagged() {
        assert!(!catalog_or_latest(Some("Y11S3")).fallback);
        for season in [Some("Y12S1"), None] {
            let resolved = catalog_or_latest(season);
            assert!(resolved.fallback);
            assert_eq!(resolved.catalog.season, latest().season);
        }
        assert_eq!(
            operator_info_or_latest(Some("Y12S1"), ACE).map(|o| o.1),
            Some(true)
        );
        assert_eq!(
            operator_info_or_latest(Some("Y11S3"), ACE).map(|o| o.1),
            Some(false)
        );
    }

    #[test]
    fn every_operator_with_a_side_has_an_entry_on_that_side() {
        for catalog in SEASONS {
            let mut ids = HashSet::new();
            for info in catalog.operators {
                assert!(ids.insert(info.operator), "{} twice", info.operator);
                assert_eq!(info.operator.role(), Some(info.side), "{}", info.operator);
            }
            for (name, id, role) in crate::types::OPERATORS.iter().copied() {
                let entry = catalog.operator(Operator(id));
                assert_eq!(
                    entry.is_some(),
                    role.is_some(),
                    "{name} in {}",
                    catalog.season
                );
            }
        }
    }

    #[test]
    fn armor_and_speed_follow_from_max_health() {
        for info in SEASONS.iter().flat_map(|c| c.operators) {
            let name = info.operator;
            match (info.armor, info.speed, info.max_health) {
                (Some(armor), Some(speed), Some(health)) => {
                    assert_eq!(armor + speed, 4, "{name}");
                    assert_eq!(armor_for_health(health), Some(armor), "{name}");
                    assert!(info.health_evidence.is_some(), "{name}");
                }
                (None, None, None) => assert!(info.health_evidence.is_none(), "{name}"),
                other => panic!("{name}: partial rating {other:?}"),
            }
        }
    }

    /// Ids are unique within a list, and each is the item table's id for
    /// that name and slot.
    #[test]
    fn item_ids_agree_with_the_item_table() {
        for info in SEASONS.iter().flat_map(|c| c.operators) {
            let name = info.operator;
            let slots = [
                (info.gadgets, ItemKind::Gadget),
                (info.primaries, ItemKind::Primary),
                (info.secondaries, ItemKind::Secondary),
            ];
            for (items, kind) in slots {
                assert!(!items.is_empty(), "{name}: no {kind:?}");
                let mut names = HashSet::new();
                let mut ids = HashSet::new();
                for item in items {
                    assert!(names.insert(item.name), "{name}: {} twice", item.name);
                    // Only what was seen has a count or must have an id.
                    if item.evidence.is_observed() {
                        assert!(
                            item.id.is_some() && item.evidence.rounds > 0,
                            "{name}: {}",
                            item.name
                        );
                    } else {
                        assert_eq!(item.count, None, "{name}: {}", item.name);
                    }
                    assert_eq!(
                        item.count.is_some(),
                        kind == ItemKind::Gadget && item.evidence.is_observed()
                    );
                    let Some(id) = item.id else { continue };
                    assert!(ids.insert(id), "{name}: {id} twice");
                    assert_eq!(item_name(id), Some(item.name), "{name}");
                    // A shield is an ability in the primary slot, and behind
                    // Blackbeard's the rifle sits in the secondary slot.
                    let shield =
                        kind == ItemKind::Primary && item_kind(id) == Some(ItemKind::Ability);
                    let rifle = kind == ItemKind::Secondary
                        && item_kind(id) == Some(ItemKind::Primary)
                        && info
                            .primaries
                            .iter()
                            .all(|p| p.id.and_then(item_kind) == Some(ItemKind::Ability));
                    assert!(
                        shield || rifle || item_kind(id) == Some(kind),
                        "{name}: {}",
                        item.name
                    );
                }
            }
            match info.ability {
                Some(ability) => {
                    assert_eq!(info.gadget_slots, 1, "{name}");
                    if let Some(id) = ability.id {
                        assert_eq!(item_name(id), Some(ability.name), "{name}");
                        assert_eq!(item_kind(id), Some(ItemKind::Ability), "{name}");
                    }
                    if let (Some(count), Some(max)) = (ability.count, ability.max) {
                        assert!(count <= max, "{name}");
                    }
                }
                None => assert_eq!(info.gadget_slots, 2, "{name}"),
            }
        }
    }

    #[test]
    fn the_one_operator_never_seen_is_all_reference() {
        let iana = operator_info("Y11S3", IANA).unwrap();
        assert_eq!(iana.side_evidence, Evidence::REFERENCE);
        assert_eq!((iana.armor, iana.max_health), (None, None));
        assert_eq!(iana.ability.map(|a| (a.id, a.count)), Some((None, None)));
        let unseen = y11s3::CATALOG
            .operators
            .iter()
            .filter(|o| !o.side_evidence.is_observed())
            .count();
        assert_eq!(unseen, 1);
    }

    #[test]
    fn serializes_in_camel_case() {
        let json = serde_json::to_value(operator_info("Y11S3", ACE).unwrap()).unwrap();
        assert_eq!(json["operator"]["name"], "Ace");
        assert_eq!(json["side"], "Attack");
        assert_eq!(json["maxHealth"], 125);
        assert_eq!(json["healthEvidence"]["source"], "observed");
        assert_eq!(json["gadgetSlots"], 1);
        assert_eq!(json["ability"]["regenerates"], false);
        let json = serde_json::to_value(operator_info("Y11S3", IANA).unwrap()).unwrap();
        assert_eq!(
            json["sideEvidence"],
            serde_json::json!({"source": "reference"})
        );
        assert!(json.get("maxHealth").is_none());
        let json = serde_json::to_value(catalog_or_latest(Some("Y1S1"))).unwrap();
        assert_eq!(json["fallback"], true);
        assert_eq!(json["catalog"]["season"], "Y11S3");
    }

    fn round(operator: Operator, start: u32) -> Round {
        let mut round = Round::default();
        round.header.teams = [Team::default(), Team::default()];
        round.header.teams[1].role = Some(TeamRole::Attack);
        round.header.players.push(Player {
            username: "a".into(),
            team_index: 1,
            operator,
            max_health: Some(125),
            ..Player::default()
        });
        let counts = Counts {
            start,
            max: Some(3),
            ..Counts::default()
        };
        let mut loadout = Loadout::new("a", operator);
        loadout.primary = Some(Weapon {
            id: Some(1366019220),
            ..Weapon::default()
        });
        loadout.ability = Some(Counted {
            id: Some(201523838482),
            name: None,
            counts: Some(counts),
        });
        // An operator the player swapped away from: no slots, not counted.
        round.loadouts = vec![Loadout::new("a", IANA), loadout];
        round
    }

    #[test]
    fn observes_what_players_spawned_with() {
        let seen = observe(&[round(ACE, 3), round(ACE, 3), round(ACE, 2)]);
        assert_eq!(seen.len(), 1);
        let ace = &seen[0];
        assert_eq!((ace.operator, ace.rounds), (ACE, 3));
        assert_eq!(
            ace.sides,
            [Tally {
                value: TeamRole::Attack,
                rounds: 3
            }]
        );
        assert_eq!(
            ace.max_health,
            [Tally {
                value: 125,
                rounds: 3
            }]
        );
        assert_eq!(ace.primaries[0].name, Some("AK-12"));
        assert!(ace.secondaries.is_empty() && ace.gadgets.is_empty());
        let ability = &ace.abilities[0];
        assert_eq!((ability.rounds, ability.max), (3, Some(3)));
        let starts = [
            Tally {
                value: 3,
                rounds: 2,
            },
            Tally {
                value: 2,
                rounds: 1,
            },
        ];
        assert_eq!(ability.starts, starts);
    }
}
