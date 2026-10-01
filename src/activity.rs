//! What each player does over a round (Y11S3): who carries the defuser,
//! plants and disables with how they ended, what is in a player's hands,
//! operator abilities, reloads, and the defenders' reinforcements.
//!
//! All of it is HUD state in the `state` stream, which sends a value when
//! it changes and once when its object is created. The objects hang off
//! each player's controller (class `2759e897`, `PlayerViewModel`); names in
//! quotes are the game's property names, matched by CRC-32:
//!
//! ```text
//! controller
//!   27c08dca "GameModeInteractionVM" -> b2216bf3 "DefuserInteractionViewModel"
//!       8d68f855 "HasDefuser" u8                   1 while the player carries the defuser
//!       e58c06e9 "DefuserInteractionType" u32      0 planting, 1 disabling, 2 idle
//!       e9a37feb "DefuserInteractionProgress" f32  1.0 -> 0.0 (not read)
//!   e8d1e539 "PlayerLoadoutVM" -> 579fb57b "PlayerLoadoutViewModel"
//!       66ac7724 "EquippedWeaponType" u32          0 nothing, 1 drone, 2 primary,
//!                                                  3 secondary, 4 ability, 5 gadget
//!       9327a12b "PrimaryWeaponVM", 03994083 "SecondaryWeaponVM"
//!                                    -> 0eaa2be0 "WeaponViewModel"
//!           f8885fd5 "IsReloading" u8
//!       4cd6a0c7 "OperatorAbilityVM", d890b5f7 "SecondaryGadgetVM"
//!                                    -> 9f44690a "GadgetViewModel"
//!           3bd73fc1 "IsEquipped" u8
//!           8e940073 "IsActive" u8
//!           cb4f819b f32                           gauge fill 0..1 (not read)
//!           5c8e4b10 u32                           gauge state
//!           c7d74c2e "CooldownStatus" u32
//!           a57aaa29 -> a view model of the operator's own, by class:
//!               03fd3cca Montagne    4c87e2f1 "IsExtended" u8
//!               f7699f0e Blackbeard  53d01dd0 "IsShieldEquipped" u8
//!               5e27ebdf Solis       09d7d230 "DeviceState" u32
//!               08610b90 Deimos      2f64237c "IsTracking" u8
//!                                    c8fda761 "IsActivating" u8
//!               de28a56c Thatcher    474903e1 "IsScreenActive" u8
//!               682dfbcc Dokkaebi    1b68078a "CallState" u32
//! ```
//!
//! The ability field can also hold a `WeaponViewModel` (launchers). Objects
//! are linked again during a round: an attacker who swaps operator in prep
//! gets new slots, and every player gets a new defuser interaction object
//! when action starts. A value counts for a player only while its object is
//! the last one linked to its field, all the way up to the controller.
//!
//! Two things are not per player. The game-mode object's `ff39f408`
//! "IsDefuserStarted" (u8) turns 1 when a plant completes and 0 when a
//! disable does: in the record that puts the player back to idle, or in
//! the stream's next one, a frame later. And each team's view model (class `d2b1c612`,
//! with its players' controllers in the array `a87a2c30` "PlayersVM") links
//! through field `478b9617` a pool object: class `80deb4bc` for defenders,
//! whose `67de20f8` (u32) is how many reinforcements the team has left. It
//! starts at 10 (6 or 7 in one game mode of players' own recordings), drops
//! when a player starts reinforcing and rises again when they cancel. Who
//! did it is not in this stream.
//!
//! Times are seconds since the recording started, to the frame and rounded
//! to the millisecond. The opening snapshot counts as frame 0.

use std::collections::HashMap;

use serde::Serialize;

use crate::container::StreamInfo;
use crate::entities::{Hash, Record, for_each_record};
use crate::header::Player;
use crate::loadout::{STATE_STREAM, blocks};
use crate::records::RecordMap;

/// Controller -> the player's `DefuserInteractionViewModel`, and its class.
const INTERACTION_FIELD: Hash = [0x27, 0xC0, 0x8D, 0xCA];
const INTERACTION_VIEW: Hash = [0xB2, 0x21, 0x6B, 0xF3];
/// Defuser interaction: `HasDefuser` and `DefuserInteractionType`.
const HAS_DEFUSER: Hash = [0x8D, 0x68, 0xF8, 0x55];
const INTERACTION_TYPE: Hash = [0xE5, 0x8C, 0x06, 0xE9];
/// `DefuserInteractionType` values; 2 is idle.
const PLANTING: u32 = 0;
const DISABLING: u32 = 1;
/// Game-mode object: `IsDefuserStarted`.
const DEFUSER_STARTED: Hash = [0xFF, 0x39, 0xF4, 0x08];

/// Controller -> the player's `PlayerLoadoutViewModel`.
const LOADOUT_FIELD: Hash = [0xE8, 0xD1, 0xE5, 0x39];
/// Loadout view: `EquippedWeaponType`.
const EQUIPPED_TYPE: Hash = [0x66, 0xAC, 0x77, 0x24];
/// Loadout view -> its slots.
const PRIMARY_FIELD: Hash = [0x93, 0x27, 0xA1, 0x2B];
const SECONDARY_FIELD: Hash = [0x03, 0x99, 0x40, 0x83];
const ABILITY_FIELD: Hash = [0x4C, 0xD6, 0xA0, 0xC7];
const GADGET_FIELD: Hash = [0xD8, 0x90, 0xB5, 0xF7];
/// Weapon slot: `IsReloading`.
const IS_RELOADING: Hash = [0xF8, 0x88, 0x5F, 0xD5];
/// Ability or gadget slot: `IsEquipped`, `IsActive`, the gauge state and
/// `CooldownStatus`.
const IS_EQUIPPED: Hash = [0x3B, 0xD7, 0x3F, 0xC1];
const IS_ACTIVE: Hash = [0x8E, 0x94, 0x00, 0x73];
const GAUGE_STATE: Hash = [0x5C, 0x8E, 0x4B, 0x10];
const COOLDOWN_STATUS: Hash = [0xC7, 0xD7, 0x4C, 0x2E];
/// Ability or gadget slot -> the operator's own view model.
const OPERATOR_VIEW_FIELD: Hash = [0xA5, 0x7A, 0xAA, 0x29];

/// `(class, property, signal, value size)` of what the operators' own view
/// models say.
const OPERATOR_SIGNALS: [(Hash, Hash, Signal, usize); 7] = [
    // Montagne: `IsExtended`.
    (
        [0x03, 0xFD, 0x3C, 0xCA],
        [0x4C, 0x87, 0xE2, 0xF1],
        Signal::Extended,
        1,
    ),
    // Blackbeard: `IsShieldEquipped`.
    (
        [0xF7, 0x69, 0x9F, 0x0E],
        [0x53, 0xD0, 0x1D, 0xD0],
        Signal::ShieldEquipped,
        1,
    ),
    // Solis: `DeviceState`.
    (
        [0x5E, 0x27, 0xEB, 0xDF],
        [0x09, 0xD7, 0xD2, 0x30],
        Signal::DeviceState,
        4,
    ),
    // Deimos: `IsTracking` and `IsActivating`.
    (
        [0x08, 0x61, 0x0B, 0x90],
        [0x2F, 0x64, 0x23, 0x7C],
        Signal::Tracking,
        1,
    ),
    (
        [0x08, 0x61, 0x0B, 0x90],
        [0xC8, 0xFD, 0xA7, 0x61],
        Signal::Activating,
        1,
    ),
    // Thatcher: `IsScreenActive`.
    (
        [0xDE, 0x28, 0xA5, 0x6C],
        [0x47, 0x49, 0x03, 0xE1],
        Signal::ScreenActive,
        1,
    ),
    // Dokkaebi: `CallState`.
    (
        [0x68, 0x2D, 0xFB, 0xCC],
        [0x1B, 0x68, 0x07, 0x8A],
        Signal::CallState,
        4,
    ),
];

/// Team view model -> its players' controllers (`PlayersVM`).
const TEAM_PLAYERS_FIELD: Hash = [0xA8, 0x7A, 0x2C, 0x30];
/// Team view model -> its pool object, and the class of the defenders'.
const POOL_FIELD: Hash = [0x47, 0x8B, 0x96, 0x17];
const REINFORCEMENT_POOL: Hash = [0x80, 0xDE, 0xB4, 0xBC];
/// Reinforcement pool: how many are left.
const REINFORCEMENTS_LEFT: Hash = [0x67, 0xDE, 0x20, 0xF8];

/// One stretch a player carried the defuser. Only attackers carry it, and
/// one at a time. How a stretch ended (planted, dropped, the carrier died)
/// is not said here.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefuserCarry {
    pub username: String,
    /// Seconds since the recording started.
    pub start: f64,
    /// Absent while the player still carried it when the recording ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum InteractionKind {
    Plant,
    Disable,
}

/// How a plant or disable ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum InteractionOutcome {
    /// `IsDefuserStarted` turned as the player went back to idle: in that
    /// record of the stream or the next.
    Completed,
    /// The player went back to idle and `IsDefuserStarted` stayed as it was.
    Aborted,
    /// Still going when the recording ended: the round was decided first.
    Unfinished,
}

/// A player planting or disabling the defuser, from start to finish.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefuserInteraction {
    pub username: String,
    pub kind: InteractionKind,
    /// Seconds since the recording started.
    pub start: f64,
    /// Absent for an unfinished one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
    pub outcome: InteractionOutcome,
}

/// What a player holds (`EquippedWeaponType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum EquippedItem {
    /// Hands busy: placing, reinforcing, on a drone, or between two items.
    Nothing,
    Drone,
    Primary,
    Secondary,
    Ability,
    /// The secondary gadget.
    Gadget,
    /// A value not seen before, written as its number.
    #[serde(untagged)]
    Other(u32),
}

impl EquippedItem {
    fn from_raw(raw: u32) -> Self {
        match raw {
            0 => EquippedItem::Nothing,
            1 => EquippedItem::Drone,
            2 => EquippedItem::Primary,
            3 => EquippedItem::Secondary,
            4 => EquippedItem::Ability,
            5 => EquippedItem::Gadget,
            other => EquippedItem::Other(other),
        }
    }
}

/// A change of what a player holds. A player's first entry is what they
/// held when the recording started.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EquippedChange {
    pub username: String,
    /// Seconds since the recording started.
    pub time: f64,
    pub item: EquippedItem,
}

/// The loadout slot a signal comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum AbilitySlot {
    Ability,
    /// The secondary gadget.
    Gadget,
}

/// Which property of an ability or gadget changed. The properties are read
/// exactly; what their values mean is known to different degrees.
///
/// Confirmed against play:
/// - `Equipped` (`IsEquipped`): 1 while the item is out, or while an
///   ability that is switched on is running (Vigil: 1 exactly while
///   cloaked). It agrees with what the player holds.
/// - `Cooldown` (`CooldownStatus`): 2 while cooling down, else 0.
///
/// Inferred from how the values move, not confirmed:
/// - `Active` (`IsActive`): 1 while a placed device is armed and waiting.
/// - `GaugeState` (name unknown): for abilities that drain, 0 idle,
///   1 draining, 2 locked after use, 3 refilling.
///
/// Named by the game, with the values seen and no meaning checked beyond
/// the name: `Extended` (Montagne's `IsExtended`), `ShieldEquipped`
/// (Blackbeard's `IsShieldEquipped`), `DeviceState` (Solis, 0 or 1),
/// `Tracking` and `Activating` (Deimos's `IsTracking` and `IsActivating`),
/// `ScreenActive` (Thatcher's `IsScreenActive`), and `CallState` (Dokkaebi,
/// 0 to 3 in the test match, up to 6 in players' own recordings).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Signal {
    Equipped,
    Active,
    GaugeState,
    Cooldown,
    Extended,
    ShieldEquipped,
    DeviceState,
    Tracking,
    Activating,
    ScreenActive,
    CallState,
}

/// A change of one ability or gadget property. A signal starts at 0: its
/// first entry is the first value that is not.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbilitySignal {
    pub username: String,
    /// Seconds since the recording started.
    pub time: f64,
    pub slot: AbilitySlot,
    pub signal: Signal,
    /// The property's value; flags are 0 or 1.
    pub value: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum WeaponSlot {
    Primary,
    Secondary,
}

/// A reload starting or ending (`IsReloading`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reload {
    pub username: String,
    /// Seconds since the recording started.
    pub time: f64,
    pub weapon: WeaponSlot,
    pub reloading: bool,
}

/// A change of how many reinforcements a team has left. A team's first
/// entry is what it had when the recording started.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolChange {
    /// Index into the header's teams.
    pub team: usize,
    /// Seconds since the recording started.
    pub time: f64,
    pub left: u32,
}

/// What the players did over the round, each list in the order it happened.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub defuser: Vec<DefuserCarry>,
    pub interactions: Vec<DefuserInteraction>,
    pub equipped: Vec<EquippedChange>,
    pub ability: Vec<AbilitySignal>,
    pub reloads: Vec<Reload>,
    /// Pools whose team could not be told from its players are left out.
    pub reinforcement_pool: Vec<PoolChange>,
}

/// What an object is to a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Interaction,
    Loadout,
    Weapon(WeaponSlot),
    Item(AbilitySlot),
    /// The operator's own view model under a slot, with its class.
    Operator(AbilitySlot, Hash),
}

/// A link to a child: `(parent, field, class)`.
type Link = (u32, Hash, Hash);

/// The state stream as read so far.
#[derive(Debug, Default)]
struct Reader<'a> {
    /// Controller -> index into the players.
    controllers: HashMap<u32, usize>,
    usernames: Vec<&'a str>,
    /// `(parent, field)` -> the child last linked there.
    held: HashMap<(u32, Hash), u32>,
    /// Child -> the link that last named it.
    owners: HashMap<u32, Link>,
    /// Team view model -> the players whose controllers it lists.
    members: HashMap<u32, Vec<usize>>,

    /// Per player: `HasDefuser`, `DefuserInteractionType`, and where in the
    /// output their carry and interaction still going are.
    has_defuser: Vec<bool>,
    interaction: Vec<Option<u32>>,
    carrying: Vec<Option<usize>>,
    interacting: Vec<Option<usize>>,
    /// The latest `IsDefuserStarted`, and what it turned to in this frame.
    started: Option<bool>,
    turned: Option<bool>,
    /// Records read so far, and the interactions that ended without
    /// `IsDefuserStarted` turning, with the record they ended in.
    records: usize,
    ended: Vec<(usize, usize)>,
    /// A defuser property was written in this frame.
    defuser_written: bool,

    /// The latest value of everything else, to tell changes from resends.
    equipped: HashMap<usize, u32>,
    signals: HashMap<(usize, AbilitySlot, Signal), u32>,
    reloading: HashMap<(usize, WeaponSlot), bool>,
    /// Pool object -> reinforcements left.
    pools: HashMap<u32, u32>,
    /// `(pool object, time, left)` of every pool change.
    pool_changes: Vec<(u32, f64, u32)>,

    out: Activity,
}

impl<'a> Reader<'a> {
    fn new(players: &'a [Player]) -> Self {
        let controllers = players
            .iter()
            .enumerate()
            .filter_map(|(i, p)| Some((p.entities.as_ref()?.controller, i)))
            .collect();
        Reader {
            controllers,
            usernames: players.iter().map(|p| p.username.as_str()).collect(),
            has_defuser: vec![false; players.len()],
            interaction: vec![None; players.len()],
            carrying: vec![None; players.len()],
            interacting: vec![None; players.len()],
            ..Reader::default()
        }
    }

    /// Reads one snapshot or frame record written at `time`. A frame's links
    /// are taken before its values, so a value written to an object in the
    /// frame that links it counts.
    fn read(&mut self, block: &[u8], time: f64) {
        let hash_at = |at: usize| -> Option<Hash> { block.get(at..at + 4)?.try_into().ok() };
        let mut links: Vec<(u32, Hash, u32, Hash)> = Vec::new();
        let mut values: Vec<(u32, Hash, usize, usize)> = Vec::new();
        // Each record block names its object before writing to it.
        let mut current: Option<u32> = None;
        for_each_record(block, |at, r| match r {
            Record::Set(obj, hash, from, to) => {
                current = Some(obj);
                values.push((obj, hash, from, to));
            }
            // `26` array elements are none of the values read here.
            Record::Prop(hash, from, to) if block[at] == 0x22 => {
                if let Some(obj) = current {
                    values.push((obj, hash, from, to));
                }
            }
            Record::Prop(..) => {}
            Record::ParentChild(parent, field, child) => {
                current = Some(parent);
                if let Some(class) = hash_at(at + 21) {
                    links.push((parent, field, child, class));
                }
            }
            Record::Child(field, child) => {
                if let (Some(parent), Some(class)) = (current, hash_at(at + 13)) {
                    links.push((parent, field, child, class));
                }
            }
            Record::Element(field, _, child) => {
                if let (Some(parent), Some(class)) = (current, hash_at(at + 17)) {
                    links.push((parent, field, child, class));
                }
            }
        });
        for (parent, field, child, class) in links {
            self.link(parent, field, child, class);
        }
        for (obj, hash, from, to) in values {
            self.value(obj, hash, &block[from..to], time);
        }
        self.end_frame(time);
    }

    fn link(&mut self, parent: u32, field: Hash, child: u32, class: Hash) {
        // An empty link does not undo the one before: a slot is emptied
        // when its player leaves or dies, and nothing is written after.
        if child == 0 {
            return;
        }
        if field == TEAM_PLAYERS_FIELD {
            if let Some(&player) = self.controllers.get(&child) {
                let members = self.members.entry(parent).or_default();
                if !members.contains(&player) {
                    members.push(player);
                }
            }
            return;
        }
        let followed = [
            INTERACTION_FIELD,
            LOADOUT_FIELD,
            PRIMARY_FIELD,
            SECONDARY_FIELD,
            ABILITY_FIELD,
            GADGET_FIELD,
            OPERATOR_VIEW_FIELD,
            POOL_FIELD,
        ];
        if followed.contains(&field) {
            self.held.insert((parent, field), child);
            self.owners.insert(child, (parent, field, class));
        }
    }

    /// The link that names `obj`, while `obj` is still the last child
    /// linked to that field.
    fn owner(&self, obj: u32) -> Option<Link> {
        let link = *self.owners.get(&obj)?;
        (self.held.get(&(link.0, link.1)) == Some(&obj)).then_some(link)
    }

    /// The player `obj` belongs to and what it is to them.
    fn part(&self, obj: u32) -> Option<(usize, Part)> {
        let (parent, field, class) = self.owner(obj)?;
        if let Some(&player) = self.controllers.get(&parent) {
            let part = match field {
                INTERACTION_FIELD if class == INTERACTION_VIEW => Part::Interaction,
                LOADOUT_FIELD => Part::Loadout,
                _ => return None,
            };
            return Some((player, part));
        }
        match (self.part(parent)?, field) {
            ((player, Part::Loadout), PRIMARY_FIELD) => {
                Some((player, Part::Weapon(WeaponSlot::Primary)))
            }
            ((player, Part::Loadout), SECONDARY_FIELD) => {
                Some((player, Part::Weapon(WeaponSlot::Secondary)))
            }
            ((player, Part::Loadout), ABILITY_FIELD) => {
                Some((player, Part::Item(AbilitySlot::Ability)))
            }
            ((player, Part::Loadout), GADGET_FIELD) => {
                Some((player, Part::Item(AbilitySlot::Gadget)))
            }
            ((player, Part::Item(slot)), OPERATOR_VIEW_FIELD) => {
                Some((player, Part::Operator(slot, class)))
            }
            _ => None,
        }
    }

    fn value(&mut self, obj: u32, hash: Hash, value: &[u8], time: f64) {
        let number = match *value {
            [v] => u32::from(v),
            [a, b, c, d] => u32::from_le_bytes([a, b, c, d]),
            _ => return,
        };
        let (flag, wide) = (value.len() == 1, value.len() == 4);
        match hash {
            DEFUSER_STARTED if flag && number <= 1 => {
                let started = number == 1;
                if self.started.replace(started).unwrap_or(false) != started {
                    self.turned = Some(started);
                }
                self.defuser_written = true;
            }
            REINFORCEMENTS_LEFT if wide => {
                if self.owner(obj).is_some_and(|l| l.2 == REINFORCEMENT_POOL)
                    && self.pools.insert(obj, number) != Some(number)
                {
                    self.pool_changes.push((obj, time, number));
                }
            }
            HAS_DEFUSER if flag => {
                if let Some((player, Part::Interaction)) = self.part(obj) {
                    self.has_defuser[player] = number == 1;
                    self.defuser_written = true;
                }
            }
            INTERACTION_TYPE if wide => {
                if let Some((player, Part::Interaction)) = self.part(obj) {
                    self.interaction[player] = Some(number);
                    self.defuser_written = true;
                }
            }
            EQUIPPED_TYPE if wide => {
                if let Some((player, Part::Loadout)) = self.part(obj)
                    && self.equipped.insert(player, number) != Some(number)
                {
                    self.out.equipped.push(EquippedChange {
                        username: self.usernames[player].to_owned(),
                        time,
                        item: EquippedItem::from_raw(number),
                    });
                }
            }
            IS_RELOADING if flag => {
                if let Some((player, Part::Weapon(weapon))) = self.part(obj) {
                    let reloading = number == 1;
                    let before = self.reloading.insert((player, weapon), reloading);
                    if before.unwrap_or(false) != reloading {
                        self.out.reloads.push(Reload {
                            username: self.usernames[player].to_owned(),
                            time,
                            weapon,
                            reloading,
                        });
                    }
                }
            }
            IS_EQUIPPED | IS_ACTIVE if flag => {
                let signal = if hash == IS_EQUIPPED {
                    Signal::Equipped
                } else {
                    Signal::Active
                };
                if let Some((player, Part::Item(slot))) = self.part(obj) {
                    self.signal(player, slot, signal, number, time);
                }
            }
            GAUGE_STATE | COOLDOWN_STATUS if wide => {
                let signal = if hash == GAUGE_STATE {
                    Signal::GaugeState
                } else {
                    Signal::Cooldown
                };
                if let Some((player, Part::Item(slot))) = self.part(obj) {
                    self.signal(player, slot, signal, number, time);
                }
            }
            _ => {
                let Some(&(class, _, signal, _)) = OPERATOR_SIGNALS
                    .iter()
                    .find(|s| s.1 == hash && s.3 == value.len())
                else {
                    return;
                };
                if let Some((player, Part::Operator(slot, found))) = self.part(obj)
                    && found == class
                {
                    self.signal(player, slot, signal, number, time);
                }
            }
        }
    }

    fn signal(&mut self, player: usize, slot: AbilitySlot, signal: Signal, value: u32, time: f64) {
        let before = self.signals.insert((player, slot, signal), value);
        if before.unwrap_or(0) != value {
            self.out.ability.push(AbilitySignal {
                username: self.usernames[player].to_owned(),
                time,
                slot,
                signal,
                value,
            });
        }
    }

    /// Settles the defuser once all of a frame's values are in, so a
    /// hand-over within one frame ends one carry and then starts the next.
    fn end_frame(&mut self, time: f64) {
        let turned = self.turned.take();
        self.records += 1;
        if !std::mem::take(&mut self.defuser_written) {
            return;
        }
        // `IsDefuserStarted` can turn one record after the player went back
        // to idle (20 of 32 completions in players' own recordings).
        let record = self.records;
        for (at, ended) in std::mem::take(&mut self.ended) {
            let i = &mut self.out.interactions[at];
            if ended + 1 == record && turned == Some(i.kind == InteractionKind::Plant) {
                i.outcome = InteractionOutcome::Completed;
            }
        }
        for player in 0..self.usernames.len() {
            if let Some(at) = self.carrying[player].filter(|_| !self.has_defuser[player]) {
                self.out.defuser[at].end = Some(time);
                self.carrying[player] = None;
            }
        }
        for player in 0..self.usernames.len() {
            if self.has_defuser[player] && self.carrying[player].is_none() {
                self.carrying[player] = Some(self.out.defuser.len());
                self.out.defuser.push(DefuserCarry {
                    username: self.usernames[player].to_owned(),
                    start: time,
                    end: None,
                });
            }
        }
        for player in 0..self.usernames.len() {
            let wanted = match self.interaction[player] {
                Some(PLANTING) => Some(InteractionKind::Plant),
                Some(DISABLING) => Some(InteractionKind::Disable),
                _ => None,
            };
            let going = self.interacting[player];
            if going.map(|at| self.out.interactions[at].kind) == wanted {
                continue;
            }
            if let Some(at) = going {
                let i = &mut self.out.interactions[at];
                // A plant completes as `IsDefuserStarted` turns 1, a
                // disable as it turns 0 again.
                let completed =
                    wanted.is_none() && turned == Some(i.kind == InteractionKind::Plant);
                i.end = Some(time);
                i.outcome = if completed {
                    InteractionOutcome::Completed
                } else {
                    InteractionOutcome::Aborted
                };
                if wanted.is_none() && turned.is_none() {
                    self.ended.push((at, record));
                }
            }
            self.interacting[player] = wanted.map(|kind| {
                self.out.interactions.push(DefuserInteraction {
                    username: self.usernames[player].to_owned(),
                    kind,
                    start: time,
                    end: None,
                    outcome: InteractionOutcome::Unfinished,
                });
                self.out.interactions.len() - 1
            });
        }
    }

    /// The team a pool belongs to: the one all the players listed by the
    /// team view model it hangs off are in.
    fn pool_team(&self, pool: u32, players: &[Player]) -> Option<usize> {
        let (team_view, ..) = *self.owners.get(&pool)?;
        let mut teams = self
            .members
            .get(&team_view)?
            .iter()
            .map(|&p| players[p].team_index);
        let first = teams.next()?;
        teams.all(|t| t == first).then_some(first)
    }

    fn finish(mut self, players: &[Player]) -> Activity {
        let changes = std::mem::take(&mut self.pool_changes);
        self.out.reinforcement_pool = changes
            .into_iter()
            .filter_map(|(pool, time, left)| {
                Some(PoolChange {
                    team: self.pool_team(pool, players)?,
                    time,
                    left,
                })
            })
            .collect();
        self.out
    }
}

/// Reads the players' activity from the state stream. `frame_times` are
/// the seconds since the recording started, per frame.
pub(crate) fn decode(
    data: &[u8],
    map: &RecordMap,
    streams: &[StreamInfo],
    players: &[Player],
    frame_times: &[f64],
) -> Activity {
    let mut reader = Reader::new(players);
    for (start, end, frame) in blocks(map, streams, STATE_STREAM) {
        // The snapshot is the state at frame 0.
        let seconds = frame_times
            .get(frame.unwrap_or(0) as usize)
            .or(frame_times.last())
            .copied()
            .unwrap_or(0.0);
        if let Some(block) = data.get(start..end) {
            reader.read(block, (seconds * 1000.0).round() / 1000.0);
        }
    }
    reader.finish(players)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::PlayerEntities;

    /// `23 <obj> 00000000 <hash> <size> <value>`.
    fn set(d: &mut Vec<u8>, obj: u32, hash: Hash, value: &[u8]) {
        d.push(0x23);
        d.extend(obj.to_le_bytes());
        d.extend([0; 4]);
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

    const CONTROLLERS: [u32; 2] = [0xF000_0001, 0xF000_0002];
    const VIEWS: [u32; 2] = [0xF000_0011, 0xF000_0012];
    const GAME_MODE: u32 = 0xF000_0020;
    const OTHER: Hash = [9, 9, 9, 9];

    fn players() -> Vec<Player> {
        ["a", "b"]
            .iter()
            .zip(CONTROLLERS)
            .map(|(name, controller)| Player {
                username: (*name).to_owned(),
                entities: Some(PlayerEntities {
                    controller,
                    scoreboard: None,
                    health: None,
                    movement: None,
                }),
                ..Player::default()
            })
            .collect()
    }

    /// A reader whose two players have their interaction objects linked.
    fn reader(players: &[Player]) -> Reader<'_> {
        let mut d = vec![];
        for (controller, view) in CONTROLLERS.into_iter().zip(VIEWS) {
            link(
                &mut d,
                controller,
                INTERACTION_FIELD,
                view,
                INTERACTION_VIEW,
            );
            set(&mut d, view, HAS_DEFUSER, &[0]);
            set(&mut d, view, INTERACTION_TYPE, &2u32.to_le_bytes());
        }
        let mut r = Reader::new(players);
        r.read(&d, 0.0);
        r
    }

    #[test]
    fn a_hand_over_within_one_frame_ends_one_carry_before_the_next_starts() {
        let players = players();
        let mut r = reader(&players);
        let mut d = vec![];
        set(&mut d, VIEWS[0], HAS_DEFUSER, &[1]);
        r.read(&d, 1.0);
        // The game sends the value again, then both change in one frame,
        // the new carrier first.
        r.read(&d, 2.0);
        let mut d = vec![];
        set(&mut d, VIEWS[1], HAS_DEFUSER, &[1]);
        set(&mut d, VIEWS[0], HAS_DEFUSER, &[0]);
        r.read(&d, 3.0);
        let carry = |username: &str, start, end| DefuserCarry {
            username: username.to_owned(),
            start,
            end,
        };
        assert_eq!(
            r.finish(&players).defuser,
            [carry("a", 1.0, Some(3.0)), carry("b", 3.0, None)]
        );
    }

    #[test]
    fn an_interaction_completes_only_when_the_defuser_turns_as_it_ends() {
        let players = players();
        let mut r = reader(&players);
        let kind = |r: &mut Reader, view: u32, kind: u32, started: Option<u8>, time: f64| {
            let mut d = vec![];
            set(&mut d, view, INTERACTION_TYPE, &kind.to_le_bytes());
            if let Some(started) = started {
                set(&mut d, GAME_MODE, DEFUSER_STARTED, &[started]);
            }
            r.read(&d, time);
        };
        let started = |value: u8| {
            let mut d = vec![];
            set(&mut d, GAME_MODE, DEFUSER_STARTED, &[value]);
            d
        };
        // An aborted plant, a completed one, a disable cut short by a 1
        // that turns nothing, and one still going at the end.
        kind(&mut r, VIEWS[0], PLANTING, None, 1.0);
        kind(&mut r, VIEWS[0], 2, None, 2.0);
        kind(&mut r, VIEWS[0], PLANTING, None, 3.0);
        kind(&mut r, VIEWS[0], 2, Some(1), 4.0);
        kind(&mut r, VIEWS[1], DISABLING, None, 5.0);
        kind(&mut r, VIEWS[1], 2, Some(1), 6.0);
        kind(&mut r, VIEWS[1], DISABLING, None, 7.0);
        // The defuser turning in the next record still completes; a record
        // later it does not.
        kind(&mut r, VIEWS[1], 2, None, 8.0);
        r.read(&started(0), 8.1);
        r.read(&started(1), 8.2);
        kind(&mut r, VIEWS[1], DISABLING, None, 9.0);
        kind(&mut r, VIEWS[1], 2, None, 10.0);
        r.read(&[], 10.1);
        r.read(&started(0), 10.2);
        kind(&mut r, VIEWS[1], DISABLING, None, 11.0);
        let found: Vec<_> = r
            .finish(&players)
            .interactions
            .into_iter()
            .map(|i| (i.username, i.kind, i.start, i.end, i.outcome))
            .collect();
        use InteractionKind::{Disable, Plant};
        use InteractionOutcome::{Aborted, Completed, Unfinished};
        assert_eq!(
            found,
            [
                ("a".to_owned(), Plant, 1.0, Some(2.0), Aborted),
                ("a".to_owned(), Plant, 3.0, Some(4.0), Completed),
                ("b".to_owned(), Disable, 5.0, Some(6.0), Aborted),
                ("b".to_owned(), Disable, 7.0, Some(8.0), Completed),
                ("b".to_owned(), Disable, 9.0, Some(10.0), Aborted),
                ("b".to_owned(), Disable, 11.0, None, Unfinished),
            ]
        );
    }

    #[test]
    fn an_object_counts_only_while_it_is_the_last_one_linked() {
        let players = players();
        let (loadout, old, new, operator) = (0xF000_0030, 0xF000_0031, 0xF000_0032, 0xF000_0033);
        let montagne = OPERATOR_SIGNALS[0];
        let mut d = vec![];
        link(&mut d, CONTROLLERS[0], LOADOUT_FIELD, loadout, OTHER);
        link(&mut d, loadout, ABILITY_FIELD, old, OTHER);
        set(&mut d, loadout, EQUIPPED_TYPE, &2u32.to_le_bytes());
        // Values of an object nobody links are nobody's.
        set(&mut d, new, IS_EQUIPPED, &[1]);
        let mut r = Reader::new(&players);
        r.read(&d, 0.0);
        let mut d = vec![];
        set(&mut d, old, IS_EQUIPPED, &[1]);
        r.read(&d, 1.0);
        // The swap links a new slot, in the frame that writes to it, and an
        // empty link after it changes nothing.
        let mut d = vec![];
        set(&mut d, new, IS_EQUIPPED, &[0]);
        set(&mut d, operator, montagne.1, &[1]);
        link(&mut d, loadout, ABILITY_FIELD, new, OTHER);
        link(&mut d, new, OPERATOR_VIEW_FIELD, operator, montagne.0);
        link(&mut d, loadout, GADGET_FIELD, 0, OTHER);
        r.read(&d, 2.0);
        let mut d = vec![];
        set(&mut d, old, IS_EQUIPPED, &[1]);
        set(&mut d, loadout, EQUIPPED_TYPE, &2u32.to_le_bytes());
        set(&mut d, loadout, EQUIPPED_TYPE, &7u32.to_le_bytes());
        r.read(&d, 3.0);

        let out = r.finish(&players);
        let signals: Vec<_> = out
            .ability
            .iter()
            .map(|s| (s.time, s.signal, s.value))
            .collect();
        assert_eq!(
            signals,
            [
                (1.0, Signal::Equipped, 1),
                (2.0, Signal::Equipped, 0),
                (2.0, Signal::Extended, 1)
            ]
        );
        let items: Vec<_> = out.equipped.iter().map(|e| (e.time, e.item)).collect();
        assert_eq!(
            items,
            [(0.0, EquippedItem::Primary), (3.0, EquippedItem::Other(7))]
        );
        assert_eq!(
            serde_json::to_string(&out.equipped).unwrap(),
            r#"[{"username":"a","time":0.0,"item":"Primary"},{"username":"a","time":3.0,"item":7}]"#
        );
    }

    #[test]
    fn a_pool_belongs_to_the_team_whose_players_its_parent_lists() {
        let mut players = players();
        players[1].team_index = 1;
        let (team_view, pool, stray) = (0xF000_0040, 0xF000_0041, 0xF000_0042);
        let mut d = vec![];
        // `1e <field> <index> <child> 00000000 <class>` after the object is
        // named.
        link(&mut d, team_view, POOL_FIELD, pool, REINFORCEMENT_POOL);
        d.push(0x1E);
        d.extend(TEAM_PLAYERS_FIELD);
        d.extend(0u32.to_le_bytes());
        d.extend(CONTROLLERS[1].to_le_bytes());
        d.extend([0; 4]);
        d.extend(OTHER);
        set(&mut d, pool, REINFORCEMENTS_LEFT, &10u32.to_le_bytes());
        // A pool under a parent that lists no player has no team.
        link(&mut d, stray, POOL_FIELD, stray + 1, REINFORCEMENT_POOL);
        set(&mut d, stray + 1, REINFORCEMENTS_LEFT, &10u32.to_le_bytes());
        let mut r = Reader::new(&players);
        r.read(&d, 0.0);
        for (time, left) in [(1.0, 9u32), (2.0, 9), (3.0, 10)] {
            let mut d = vec![];
            set(&mut d, pool, REINFORCEMENTS_LEFT, &left.to_le_bytes());
            r.read(&d, time);
        }
        let pool: Vec<_> = r
            .finish(&players)
            .reinforcement_pool
            .into_iter()
            .map(|c| (c.team, c.time, c.left))
            .collect();
        assert_eq!(pool, [(1, 0.0, 10), (1, 1.0, 9), (1, 3.0, 10)]);
    }
}
