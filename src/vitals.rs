//! Health over a round (Y11S3): maximum health and overheal, downs and
//! revives, heals and armor plates, status effects, reverse friendly fire
//! and flashes.
//!
//! All of it is HUD state in the `state` stream, read the way
//! [`crate::loadout`] reads slots. Each controller links three view models:
//!
//! ```text
//! 4154dcc4 PlayerLifeVM            -> PlayerLifeViewModel: Health, MaxHealth,
//!                                     OverhealedMaxHealth, PlayerLifeState,
//!                                     DBNOProgress
//! 4c37235c EffectsVM               -> EffectsViewModel: a list of the status
//!                                     effects shown next to the health bar
//! 5ef7d99d FriendlyFireFeedbackVM  -> IsReverseFriendlyFireActive
//! ```
//!
//! and holds `IsAffectedByFlashbang` itself. Every hash is the CRC-32 of the
//! game's own name for the property, which is how the names here were
//! confirmed; the meaning of each value was checked on 174 rounds (10 test
//! rounds, 164 from ranked play).
//!
//! The effects list is written whole each time it changes: the list
//! property itself (`23 <list> 00000000 b504bf0f 01 01`, the same bytes for
//! an empty list), then one `1e b504bf0f <index> <item>` per effect
//! showing. An effect starts at the first write that lists its item and
//! ends at the first that leaves it out.
//!
//! The file says whose health rose and by how much, never why or thanks to
//! whom. The cause of a rise ([`HealKind`], a [`Plate`]) and its giver
//! (`by`) are worked out from what else the HUD wrote in the same moment:
//! they are inferred, as are the names of the effect types.

use std::collections::HashMap;

use serde::Serialize;

use crate::container::StreamInfo;
use crate::details::{Loadout, Phase};
use crate::entities::{Hash, Record, for_each_record};
use crate::header::{Player, Relation};
use crate::loadout::{Clock, Sample, blocks};
use crate::records::RecordMap;

/// Name hash of the stream read here.
const STATE_STREAM: Hash = [0xA9, 0x8F, 0xDD, 0x0B];

/// Controller -> the player's `PlayerLifeViewModel` (`PlayerLifeVM`).
const LIFE_FIELD: Hash = [0x41, 0x54, 0xDC, 0xC4];
/// Life: hit points, overheal included (`Health`). 0 while down or dead.
const HEALTH: Hash = [0x25, 0x26, 0x76, 0xC9];
/// Life: the most health without overheal (`MaxHealth`): 100, 110 or 125 by
/// operator (one value per operator over 174 rounds), 25 more with a plate.
const MAX_HEALTH: Hash = [0x11, 0x49, 0xA6, 0x72];
/// Life: the most health with overheal (`OverhealedMaxHealth`), always
/// `MaxHealth + 20`.
const OVERHEALED_MAX_HEALTH: Hash = [0x01, 0x3F, 0xD2, 0xDA];
/// Life: 0 above 20 health, 2 at or under 20, 1 overhealed, 3 down, 4 dead
/// (`PlayerLifeState`). 1 held exactly while `Health > MaxHealth`.
const LIFE_STATE: Hash = [0xE7, 0x88, 0xF6, 0xA5];
/// Life: how far a downed player has bled out, 0 towards 1, as an `f32`
/// (`DBNOProgress`).
const DBNO_PROGRESS: Hash = [0x72, 0x5E, 0x99, 0xF9];

/// Controller -> the player's `EffectsViewModel` (`EffectsVM`).
const EFFECTS_FIELD: Hash = [0x4C, 0x37, 0x23, 0x5C];
/// Effects view model: its list of `EffectItemViewModel`s. The one hash
/// here whose name was not found.
const EFFECT_LIST: Hash = [0xB5, 0x04, 0xBF, 0x0F];
/// Effect item: what the effect is (`Type`), see [`effect_name`].
const EFFECT_TYPE: Hash = [0x17, 0xF8, 0xEC, 0x2C];
/// Effect item: 2 for an effect that helps the player, 3 for one that
/// harms (`State`). One value per type over 2415 effects.
const EFFECT_STATE: Hash = [0xFF, 0xFD, 0x52, 0x62];
const BUFF: u32 = 2;

/// Controller -> `FriendlyFireFeedbackViewModel` (`FriendlyFireFeedbackVM`).
const FRIENDLY_FIRE_FIELD: Hash = [0x5E, 0xF7, 0xD9, 0x9D];
/// Friendly fire feedback: 1 while the damage the player deals to
/// teammates comes back to them (`IsReverseFriendlyFireActive`). It turned
/// on within 0.1 s of a team kill for the killer, and stays on into the
/// next rounds.
const REVERSE_FRIENDLY_FIRE: Hash = [0x4A, 0xCD, 0x2D, 0x46];
/// Controller: 1 while the screen is flashed (`IsAffectedByFlashbang`). The
/// game writes the same value to every controller in the same frame: it is
/// what the recording player sees, whoever the controller belongs to.
const FLASHED: Hash = [0x56, 0x78, 0x29, 0x44];
/// Scoreboard: the player's score (`MatchScore`). A Kona station's healing
/// burst gives its Thunderbird 5 points in the same moment.
const MATCH_SCORE: Hash = [0xEC, 0xDA, 0x4F, 0x80];

/// Effect types the heal rules look for.
const SURGE: u32 = 2;
const KONA: u32 = 35;
/// A plate adds this much to `MaxHealth` and to `Health`.
const PLATE: u32 = 25;
/// What a surge adds.
const SURGE_HEAL: u32 = 20;
/// Frame times are compared after rounding to the millisecond.
const EPS: f64 = 0.0005;
/// A stim's shot leaves Doc's pistol up to this long before the health
/// arrives (0.31 s at most over 12 stims), or shortly after.
const STIM_BEFORE: f64 = 0.8;
/// Other causes are written within this long of the rise: Finka's count
/// (0.11 s at most over 115 surges), the plate's `MaxHealth`.
const NEAR: f64 = 0.16;
const PLATE_BEFORE: f64 = 0.11;
/// Thunderbird's points follow a burst within this long.
const SCORE_AFTER: f64 = 0.11;
/// Health is set back to the maximum within this long of action starting.
const ACTION_RESET: f64 = 3.0;

/// The name of an effect type. Inferred, not read: the file holds the
/// number only. Each name is the operator or gadget that was in the round
/// every time the type showed (over 174 rounds, 2415 effects) and whose
/// use preceded it. Type 15 showed on defenders only, for at most 1.05 s,
/// with the body outside the area the defenders were in during prep in 17
/// of 24 cases checked: the warning before a defender outside is detected.
/// Type 51 started 7.03 to 7.07 s after a Logic Bomb, as a 40-damage
/// explosion hit that defender, and lasted until death or the end of the
/// round (6 of 6): the phone Dokkaebi called, overloaded. Types seen
/// without a telling operator (21, 25, 29, 37) have no name.
pub fn effect_name(kind: u32) -> Option<&'static str> {
    Some(match kind {
        0 => "JackalTracked",
        1 => "LesionPoison",
        2 => "FinkaSurge",
        3 => "DokkaebiCall",
        4 => "RookArmor",
        5 => "AlibiTracked",
        6 => "ClashShock",
        8 => "EnemyJammer",
        11 => "LionScan",
        13 => "ProximityAlarm",
        14 => "MelusiBanshee",
        15 => "OutsideWarning",
        22 => "GrimSwarm",
        23 => "GrimTracked",
        26 => "FenrirMine",
        27 => "FenrirFear",
        28 => "TubaraoZoto",
        30 => "DeimosMarked",
        31 => "DeimosTracking",
        33 => "JackalTracking",
        34 => "FriendlyJammer",
        35 => "ThunderbirdHeal",
        39 => "ThornRazorbloom",
        43 => "SnakeRadar",
        46 => "NoorLance",
        51 => "DokkaebiOverload",
        52 => "Burning",
        _ => return None,
    })
}

/// When something was written: on the round's clock and on the recording's.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct At {
    pub time: String,
    pub phase: Phase,
    /// Seconds since the prep phase started.
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
    /// Seconds since the recording started, to the frame. Absent for what
    /// the opening snapshot already held.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_time: Option<f64>,
}

/// Why a player's health changed, when it was not damage. Inferred from
/// what the HUD wrote in the same moment, see [`Heal`], [`Plate`] and
/// [`Vitals::decay`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Cause {
    /// Overheal wearing off, a point at a time.
    Decay,
    FinkaSurge,
    DocStim,
    KonaBurst,
    KonaTick,
    /// A Rook plate picked up.
    Plate,
    /// Picked up from a down by a teammate.
    Revive,
}

/// A player's health in one frame that changed it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VitalSample {
    /// Overheal included.
    pub health: u32,
    /// The maximum in force, plate included.
    pub max_health: u32,
    /// `PlayerLifeState`: 0 alive, 1 overhealed, 2 at or under 20 health,
    /// 3 down, 4 dead.
    pub state: u32,
    /// Why the health changed, when a cause other than damage was found.
    pub cause: Option<Cause>,
    /// Seconds since the recording started, to the millisecond.
    pub recording_time: Option<f64>,
}

/// One player's health over the round.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerVitals {
    pub username: String,
    /// The operator's maximum health without a plate: 100, 110 or 125.
    /// `None` for a player whose life object held no maximum (they never
    /// spawned).
    pub max_health: Option<u32>,
    /// One per frame that changed health, maximum or life state, from the
    /// first frame record on.
    pub samples: Vec<VitalSample>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum LifeKind {
    Down,
    Revive,
}

/// A player going down, or getting back up.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifeChange {
    #[serde(rename = "type")]
    pub kind: LifeKind,
    pub username: String,
    /// `Revive`: the health the player got up with: 20, or more when a
    /// heal picked them up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<u32>,
    /// `Down`: how far the bleed-out had got (0 towards 1) when the down
    /// ended, or the recording did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bleed_out: Option<f32>,
    pub time: String,
    pub phase: Phase,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_time: Option<f64>,
}

/// What healed a player. Inferred:
///
/// - `FinkaSurge`: a `FinkaSurge` effect appeared on the player in the same
///   frame, or the effect was showing already and a Finka's ability count
///   dropped as the health rose by 20.
/// - `DocStim`: the health went to `OverhealedMaxHealth`, or up from a
///   down, as a Doc's stim pistol lost a round.
/// - `KonaBurst`, `KonaTick`: the `ThunderbirdHeal` effect was showing (or
///   a Thunderbird scored 5 points as the health rose by 20 or 21); a burst
///   is a rise of 20 or more, a tick a smaller one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HealKind {
    FinkaSurge,
    DocStim,
    KonaBurst,
    KonaTick,
}

impl HealKind {
    fn cause(self) -> Cause {
        match self {
            HealKind::FinkaSurge => Cause::FinkaSurge,
            HealKind::DocStim => Cause::DocStim,
            HealKind::KonaBurst => Cause::KonaBurst,
            HealKind::KonaTick => Cause::KonaTick,
        }
    }

    /// The operator whose ability it is.
    fn operator(self) -> &'static str {
        match self {
            HealKind::FinkaSurge => "Finka",
            HealKind::DocStim => "Doc",
            HealKind::KonaBurst | HealKind::KonaTick => "Thunderbird",
        }
    }
}

/// A rise of a player's health by an ability.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Heal {
    pub username: String,
    /// Who gave it. The file links no giver: this is the Doc whose pistol
    /// fired, or the Finka or Thunderbird on the player's team.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    pub amount: u32,
    /// Health after it, overheal included.
    pub health: u32,
    /// How much of `health` is above the maximum.
    pub overheal: u32,
    pub kind: HealKind,
    /// The heal picked the player up from a down.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub revive: bool,
    pub time: String,
    pub phase: Phase,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_time: Option<f64>,
}

/// A Rook plate picked up: `MaxHealth` and `Health` both rose by 25.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plate {
    pub username: String,
    /// The Rook on the player's team; the file links no giver.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    pub time: String,
    pub phase: Phase,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_time: Option<f64>,
}

/// A status effect the HUD showed on a player.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Effect {
    pub username: String,
    /// The item's `Type`.
    #[serde(rename = "type")]
    pub kind: u32,
    /// Inferred, see [`effect_name`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
    /// The effect helps the player (the item's `State` is 2).
    pub buff: bool,
    /// When the list first held it.
    #[serde(flatten)]
    pub start: At,
    /// How long it stayed listed.
    pub seconds: f64,
    /// Still listed when the recording ended.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub open: bool,
    /// Type 8 (jammed): the owner of the Signal Disruptor nearest to the
    /// player or to a device of theirs, with `jammerSource: nearest` (see
    /// [`crate::gadget_events`]). The file does not say which jammer it
    /// is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jammer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jammer_source: Option<&'static str>,
}

/// A stretch in which the damage a player deals to teammates is turned
/// back on them.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReverseFriendlyFire {
    pub username: String,
    /// On when the recording started: carried over from an earlier round.
    pub active_at_start: bool,
    /// When it turned on; absent with `active_at_start`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<At>,
    /// When it turned off again, if it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub off: Option<At>,
}

/// A stretch the recording player was flashed for.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Flash {
    /// The recording player.
    pub username: String,
    #[serde(flatten)]
    pub start: At,
    pub seconds: f64,
    /// Still flashed when the recording ended.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub open: bool,
}

/// A rise of health no rule explains.
#[derive(Clone, Debug, PartialEq)]
pub struct Rise {
    pub username: String,
    pub amount: u32,
    /// Health after it.
    pub health: u32,
    pub recording_time: Option<f64>,
}

/// What the HUD recorded of the players' health.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vitals {
    /// One per player of the round, in the header's order.
    pub players: Vec<PlayerVitals>,
    /// Downs and revives, in stream order.
    pub life: Vec<LifeChange>,
    pub heals: Vec<Heal>,
    pub plates: Vec<Plate>,
    /// In the order they started.
    pub effects: Vec<Effect>,
    pub friendly_fire: Vec<ReverseFriendlyFire>,
    /// The recording player's own; empty for a spectator's recording.
    pub flashes: Vec<Flash>,
    /// Health drops that are overheal wearing off, not damage: username and
    /// recording time.
    pub decay: Vec<(String, Option<f64>)>,
    /// Rises that are neither a spawn, a revive, a plate nor a known heal.
    pub unexplained: Vec<Rise>,
    /// What was skipped: values of a size the property never has, effects
    /// whose item states no type.
    pub warnings: Vec<String>,
}

impl Vitals {
    /// The sample of `username` written at `recording_time`.
    pub fn sample(&self, username: &str, recording_time: Option<f64>) -> Option<&VitalSample> {
        let time = recording_time?;
        let p = self.players.iter().find(|p| p.username == username)?;
        // Samples are in frame order, so in time order.
        let i = p
            .samples
            .partition_point(|s| s.recording_time.is_none_or(|t| t < time - EPS));
        p.samples
            .get(i)
            .filter(|s| s.recording_time.is_some_and(|t| (t - time).abs() <= EPS))
    }
}

/// What a record wrote, of the things read here. Players are indices into
/// the round's players.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Raw {
    Health(usize, u32),
    MaxHealth(usize, u32),
    OverhealedMax(usize, u32),
    LifeState(usize, u32),
    Bleed(usize, f32),
    /// The list property of an effects view model: a new enumeration of
    /// the list starts.
    List(u32),
    /// `(list, item)`: the list holds this item; 0 for an empty slot.
    Item(u32, u32),
    /// `(object, value)` of a `Type` or `State` property. Whether the
    /// object is an effect item is known once its list names it, which can
    /// be later in the frame.
    Type(u32, u32),
    State(u32, u32),
    ReverseFriendlyFire(usize, bool),
    Flashed(bool),
    Score(usize, u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Event {
    /// `None` in the opening snapshot.
    frame: Option<u32>,
    /// Offset in the data.
    at: usize,
    raw: Raw,
}

/// The health, effects and friendly-fire objects of the state stream.
#[derive(Debug, Default)]
struct Hud {
    controllers: HashMap<u32, usize>,
    life: HashMap<u32, usize>,
    scoreboards: HashMap<u32, usize>,
    /// Effects view model -> player.
    lists: HashMap<u32, usize>,
    /// Friendly fire feedback view model -> player.
    feedback: HashMap<u32, usize>,
    /// The recording player's controller.
    recorder: Option<u32>,
    events: Vec<Event>,
    /// Values whose size the property never has.
    malformed: usize,
}

fn u32_of(value: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(value.try_into().ok()?))
}

/// A flag is a `u8`, or the same in a `u32`.
fn flag_of(value: &[u8]) -> Option<bool> {
    match value {
        [v] => Some(*v != 0),
        _ => u32_of(value).map(|v| v != 0),
    }
}

impl Hud {
    fn new(players: &[Player]) -> Self {
        let mut hud = Hud::default();
        for (i, p) in players.iter().enumerate() {
            let Some(e) = &p.entities else { continue };
            hud.controllers.insert(e.controller, i);
            if let Some(health) = e.health {
                hud.life.insert(health, i);
            }
            if let Some(scoreboard) = e.scoreboard {
                hud.scoreboards.insert(scoreboard, i);
            }
            if p.relation == Some(Relation::You) {
                hud.recorder = Some(e.controller);
            }
        }
        hud
    }

    /// Reads the records of one snapshot or frame record. `base` is where
    /// `block` starts in the data.
    fn read(&mut self, block: &[u8], base: usize, frame: Option<u32>) {
        // Each record block names its object before writing to it.
        let mut current: Option<u32> = None;
        for_each_record(block, |at, r| match r {
            Record::Set(obj, hash, from, to) => {
                current = Some(obj);
                if let Some(value) = block.get(from..to) {
                    self.property(obj, hash, value, frame, base + at);
                }
            }
            // `26` array elements are none of the properties read here.
            Record::Prop(hash, from, to) if block.get(at) == Some(&0x22) => {
                if let (Some(obj), Some(value)) = (current, block.get(from..to)) {
                    self.property(obj, hash, value, frame, base + at);
                }
            }
            Record::ParentChild(parent, field, child) => {
                current = Some(parent);
                self.link(parent, field, child);
            }
            Record::Child(field, child) => {
                if let Some(parent) = current {
                    self.link(parent, field, child);
                }
            }
            Record::Element(field, _, child) => {
                if let Some(list) =
                    current.filter(|l| field == EFFECT_LIST && self.lists.contains_key(l))
                {
                    let raw = Raw::Item(list, child);
                    self.events.push(Event {
                        frame,
                        at: base + at,
                        raw,
                    });
                }
            }
            Record::Prop(..) => {}
        });
    }

    fn link(&mut self, parent: u32, field: Hash, child: u32) {
        let Some(&player) = self.controllers.get(&parent).filter(|_| child != 0) else {
            return;
        };
        match field {
            EFFECTS_FIELD => self.lists.insert(child, player),
            FRIENDLY_FIRE_FIELD => self.feedback.insert(child, player),
            // A seat's life object is linked anew for whoever takes it.
            LIFE_FIELD => self.life.insert(child, player),
            _ => None,
        };
    }

    fn property(&mut self, obj: u32, hash: Hash, value: &[u8], frame: Option<u32>, at: usize) {
        let number = u32_of(value);
        let raw = if let Some(&p) = self.life.get(&obj) {
            match hash {
                HEALTH => number.map(|v| Raw::Health(p, v)),
                MAX_HEALTH => number.map(|v| Raw::MaxHealth(p, v)),
                OVERHEALED_MAX_HEALTH => number.map(|v| Raw::OverhealedMax(p, v)),
                LIFE_STATE => number.map(|v| Raw::LifeState(p, v)),
                // Zero is written as the integer, which is the same bits.
                DBNO_PROGRESS => number.map(|v| Raw::Bleed(p, f32::from_bits(v))),
                _ => return,
            }
        } else if hash == EFFECT_LIST {
            if !self.lists.contains_key(&obj) {
                return;
            }
            // The value says nothing (always 1): the items follow.
            Some(Raw::List(obj))
        } else if hash == EFFECT_TYPE || hash == EFFECT_STATE {
            // Other objects have a `Type` or `State` of their own, of
            // other sizes too: only a number can be an effect's.
            let Some(v) = number else { return };
            Some(if hash == EFFECT_TYPE {
                Raw::Type(obj, v)
            } else {
                Raw::State(obj, v)
            })
        } else if hash == REVERSE_FRIENDLY_FIRE {
            let Some(&p) = self.feedback.get(&obj) else {
                return;
            };
            flag_of(value).map(|v| Raw::ReverseFriendlyFire(p, v))
        } else if hash == FLASHED {
            if self.recorder != Some(obj) {
                return;
            }
            flag_of(value).map(Raw::Flashed)
        } else if hash == MATCH_SCORE {
            let Some(&p) = self.scoreboards.get(&obj) else {
                return;
            };
            number.map(|v| Raw::Score(p, v))
        } else {
            return;
        };
        match raw {
            Some(raw) => self.events.push(Event { frame, at, raw }),
            None => self.malformed += 1,
        }
    }
}

/// A healing ability used: the count of the player's ability slot dropped.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Drop {
    player: usize,
    /// Seconds since the recording started.
    time: f64,
}

/// One player's life object as the stream has left it so far.
#[derive(Clone, Debug, Default)]
struct Life {
    health: Option<u32>,
    max: Option<u32>,
    overhealed_max: Option<u32>,
    state: Option<u32>,
    /// The state left down for one that is alive, with no health yet: a
    /// revive if health follows, a death if state 4 does.
    getting_up: bool,
    /// Index into `Vitals::life` of the down the player is in.
    down: Option<usize>,
    /// Frame and time `MaxHealth` last rose by a plate's worth.
    plated: Option<(Option<u32>, Option<f64>)>,
    /// `MaxHealth` when action started, and how many plates it included.
    at_action: Option<(u32, u32)>,
    /// Plates taken so far.
    plates: u32,
    /// The first `MaxHealth` above 0.
    first_max: Option<u32>,
    reverse: Option<bool>,
    /// Index into `Vitals::friendly_fire` of the stretch that is on.
    reverse_open: Option<usize>,
    score: Option<u32>,
}

/// An effect that is listed.
#[derive(Clone, Copy, Debug)]
struct Listed {
    list: u32,
    item: u32,
    player: usize,
    frame: Option<u32>,
    at: usize,
    /// Index into `Tracker::effects`.
    index: usize,
}

/// Walks the events frame by frame.
struct Tracker<'a> {
    players: &'a [Player],
    clock: &'a Clock<'a>,
    lists: &'a HashMap<u32, usize>,
    /// Ability counts that dropped, by Doc and by Finka.
    stims: Vec<Drop>,
    surges: Vec<Drop>,
    /// When a Thunderbird scored 5 points.
    kona_scores: Vec<f64>,
    /// When action started on the recording's clock.
    action: Option<f64>,
    life: Vec<Life>,
    /// Item -> the player whose list named it.
    item_owner: HashMap<u32, usize>,
    item_type: HashMap<u32, u32>,
    item_state: HashMap<u32, u32>,
    open: Vec<Listed>,
    /// Effects in the order they started; `None` until one ends.
    effects: Vec<Option<Effect>>,
    untyped: usize,
    flashed: Option<bool>,
    flash: Option<(Option<u32>, usize)>,
    out: Vitals,
}

fn round_ms(t: f64) -> f64 {
    (t * 1000.0).round() / 1000.0
}

impl<'a> Tracker<'a> {
    fn new(
        players: &'a [Player],
        loadouts: &[Loadout],
        clock: &'a Clock<'a>,
        lists: &'a HashMap<u32, usize>,
    ) -> Self {
        // The uses of the loadout a player spawned with.
        let drops = |operator: &str| -> Vec<Drop> {
            let mut out = Vec::new();
            for (player, p) in players.iter().enumerate() {
                if p.operator.name() != Some(operator) {
                    continue;
                }
                let loadout = loadouts
                    .iter()
                    .rfind(|l| l.username == p.username && l.operator == p.operator);
                let uses = loadout
                    .and_then(|l| l.ability.as_ref())
                    .and_then(|a| a.counts.as_ref())
                    .map_or(&[][..], |c| &c.uses);
                let times = uses.iter().filter_map(|u| u.recording_time);
                out.extend(times.map(|time| Drop { player, time }));
            }
            out
        };
        let timeline = clock.timeline;
        let action = timeline
            .action_start
            .and_then(|t| timeline.recording.get(t).copied().flatten());
        let out = Vitals {
            players: players
                .iter()
                .map(|p| PlayerVitals {
                    username: p.username.clone(),
                    ..PlayerVitals::default()
                })
                .collect(),
            ..Vitals::default()
        };
        Tracker {
            players,
            clock,
            lists,
            stims: drops("Doc"),
            surges: drops("Finka"),
            kona_scores: Vec::new(),
            action,
            life: vec![Life::default(); players.len()],
            item_owner: HashMap::new(),
            item_type: HashMap::new(),
            item_state: HashMap::new(),
            open: Vec::new(),
            effects: Vec::new(),
            untyped: 0,
            flashed: None,
            flash: None,
            out,
        }
    }

    fn seconds(&self, frame: Option<u32>) -> Option<f64> {
        self.clock.frame_times.get(frame? as usize).copied()
    }

    fn stamp(&self, frame: Option<u32>, at: usize) -> At {
        let placed = self.clock.place(
            0,
            Sample {
                value: 0,
                frame,
                at,
            },
        );
        At {
            time: placed.time,
            phase: placed.phase,
            elapsed: placed.elapsed,
            recording_time: placed.recording_time,
        }
    }

    fn name(&self, player: usize) -> String {
        self.players
            .get(player)
            .map(|p| p.username.clone())
            .unwrap_or_default()
    }

    fn plays(&self, player: usize, operator: &str) -> bool {
        self.players
            .get(player)
            .is_some_and(|p| p.operator.name() == Some(operator))
    }

    /// The player on `player`'s team who plays `operator`.
    fn teammate(&self, player: usize, operator: &str) -> Option<String> {
        let team = self.players.get(player)?.team_index;
        let found = self
            .players
            .iter()
            .find(|p| p.team_index == team && p.operator.name() == Some(operator))?;
        Some(found.username.clone())
    }

    /// The types of the effects listed on `player`.
    fn active(&self, player: usize) -> Vec<u32> {
        self.open
            .iter()
            .filter(|l| l.player == player)
            .filter_map(|l| self.item_type.get(&l.item).copied())
            .collect()
    }

    /// When each Thunderbird scored 5 points: read ahead of the frames,
    /// since the points follow the burst they are for.
    fn read_scores(&mut self, events: &[Event]) {
        for e in events {
            let Raw::Score(p, v) = e.raw else { continue };
            let Some(life) = self.life.get_mut(p) else {
                continue;
            };
            let before = life.score.replace(v);
            let gained = before.is_some_and(|b| b.checked_add(5) == Some(v));
            if let (true, true, Some(t)) =
                (gained, self.plays(p, "Thunderbird"), self.seconds(e.frame))
            {
                self.kona_scores.push(round_ms(t));
            }
        }
    }

    fn push_life(&mut self, kind: LifeKind, player: usize, health: Option<u32>, e: &Event) {
        let at = self.stamp(e.frame, e.at);
        self.out.life.push(LifeChange {
            kind,
            username: self.name(player),
            health,
            bleed_out: None,
            time: at.time,
            phase: at.phase,
            elapsed: at.elapsed,
            recording_time: at.recording_time,
        });
    }

    /// One snapshot's or frame's events.
    fn frame(&mut self, events: &[Event]) {
        let Some(frame) = events.first().map(|e| e.frame) else {
            return;
        };
        // The snapshot is the state the recording opens on: nothing in it
        // happened.
        let live = frame.is_some();
        let now = self.seconds(frame).map(round_ms);
        let players = self.life.len();
        let states: Vec<Option<u32>> = self.life.iter().map(|l| l.state).collect();
        let healed = events.iter().any(|e| matches!(e.raw, Raw::Health(..)));
        let kona: Vec<bool> = (0..players)
            .map(|p| healed && self.active(p).contains(&KONA))
            .collect();
        let mut typed: Vec<(u32, u32)> = Vec::new();
        let mut written: Vec<(u32, Vec<u32>)> = Vec::new();
        let mut healths: Vec<&Event> = Vec::new();
        let mut touched = vec![false; players];
        let mut revived = vec![false; players];
        let mut causes: Vec<Option<Cause>> = vec![None; players];

        for e in events {
            match e.raw {
                Raw::Health(p, _) => {
                    if p < players {
                        healths.push(e);
                        touched[p] = true;
                    }
                }
                Raw::MaxHealth(p, v) => {
                    let Some(l) = self.life.get_mut(p) else {
                        continue;
                    };
                    let before = l.max.replace(v);
                    touched[p] = true;
                    if v > 0 && l.first_max.is_none() {
                        l.first_max = Some(v);
                    }
                    if live && before.is_some_and(|b| b > 0 && v == b + PLATE) {
                        l.plated = Some((frame, now));
                        l.plates += 1;
                    }
                }
                Raw::OverhealedMax(p, v) => {
                    if let Some(l) = self.life.get_mut(p) {
                        l.overhealed_max = Some(v);
                    }
                }
                Raw::LifeState(p, v) => {
                    let Some(l) = self.life.get_mut(p) else {
                        continue;
                    };
                    let before = l.state.replace(v).unwrap_or(0);
                    touched[p] = true;
                    l.getting_up = false;
                    if !live {
                        continue;
                    }
                    if before != 3 && v == 3 {
                        l.down = Some(self.out.life.len());
                        self.push_life(LifeKind::Down, p, None, e);
                        continue;
                    }
                    if before != 3 {
                        continue;
                    }
                    l.down = None;
                    // Up again only with health: a downed player who dies
                    // passes through 2 on the way to 4. The health can come
                    // later in the frame, or in the next one.
                    let health = l.health.filter(|&h| h > 0);
                    l.getting_up = v <= 2 && health.is_none();
                    if let (true, Some(health)) = (v <= 2, health) {
                        revived[p] = true;
                        self.push_life(LifeKind::Revive, p, Some(health), e);
                    }
                }
                Raw::Bleed(p, v) => {
                    let down = self.life.get(p).and_then(|l| l.down);
                    if let Some(d) = down.and_then(|i| self.out.life.get_mut(i))
                        && v > 0.0
                        && v.is_finite()
                    {
                        d.bleed_out = Some(v);
                    }
                }
                Raw::List(list) => match written.iter_mut().find(|w| w.0 == list) {
                    Some(w) => w.1.clear(),
                    None => written.push((list, Vec::new())),
                },
                Raw::Item(list, item) => {
                    let at = written.iter().position(|w| w.0 == list);
                    let at = at.unwrap_or_else(|| {
                        written.push((list, Vec::new()));
                        written.len() - 1
                    });
                    if let (Some(w), true) = (written.get_mut(at), item != 0)
                        && !w.1.contains(&item)
                    {
                        w.1.push(item);
                        if let Some(&player) = self.lists.get(&list) {
                            self.item_owner.insert(item, player);
                        }
                    }
                }
                Raw::Type(obj, v) => {
                    self.item_type.insert(obj, v);
                    typed.push((obj, v));
                }
                Raw::State(obj, v) => {
                    self.item_state.insert(obj, v);
                }
                Raw::ReverseFriendlyFire(p, on) => self.reverse(p, on, e),
                Raw::Flashed(on) => self.flashed(on, e),
                Raw::Score(..) => {}
            }
        }

        for (list, items) in written {
            self.list(list, items, events.first());
        }

        for e in healths {
            let Raw::Health(p, v) = e.raw else { continue };
            let Some(l) = self.life.get_mut(p) else {
                continue;
            };
            let before = l.health.replace(v);
            if !live {
                continue;
            }
            if l.getting_up && v > 0 {
                l.getting_up = false;
                revived[p] = true;
                self.push_life(LifeKind::Revive, p, Some(v), e);
            }
            let max = self.life.get(p).and_then(|l| l.max);
            let Some(before) = before else { continue };
            if v > before {
                let rise = Rise {
                    username: self.name(p),
                    amount: v - before,
                    health: v,
                    recording_time: now,
                };
                let was_down = states.get(p).copied().flatten() == Some(3);
                let new_types: Vec<u32> = typed
                    .iter()
                    .filter(|t| self.item_owner.get(&t.0) == Some(&p))
                    .map(|t| t.1)
                    .collect();
                let context = Context {
                    frame,
                    was_down,
                    revived: revived[p],
                    kona: kona[p] || new_types.contains(&KONA),
                    surged: new_types.contains(&SURGE),
                };
                if let Some(cause) = self.rise(p, rise, &context, e) {
                    causes[p] = Some(cause);
                }
            } else if before - v == 1 && max.is_some_and(|m| before > m) {
                // Overheal wears off a point at a time.
                self.out.decay.push((self.name(p), now));
                causes[p] = Some(Cause::Decay);
            }
        }

        if !live {
            return;
        }
        let in_action = self
            .action
            .zip(now)
            .is_some_and(|(action, now)| now >= action - EPS);
        for (p, l) in self.life.iter_mut().enumerate() {
            if in_action && l.at_action.is_none() {
                // The maximum in force in the first frame of action, or
                // the first one above 0 after it for a late spawn.
                l.at_action = l.max.filter(|&m| m > 0).map(|m| (m, l.plates));
            }
            if !touched[p] {
                continue;
            }
            let (Some(health), Some(max_health)) = (l.health, l.max) else {
                continue;
            };
            if let Some(v) = self.out.players.get_mut(p) {
                v.samples.push(VitalSample {
                    health,
                    max_health,
                    state: l.state.unwrap_or(0),
                    cause: causes[p],
                    recording_time: now,
                });
            }
        }
    }

    /// A list was written whole: what it no longer holds has ended, what it
    /// holds for the first time has started.
    fn list(&mut self, list: u32, items: Vec<u32>, first: Option<&Event>) {
        let Some(&player) = self.lists.get(&list) else {
            return;
        };
        let (frame, at) = first.map_or((None, 0), |e| (e.frame, e.at));
        let (kept, ended): (Vec<Listed>, Vec<Listed>) = std::mem::take(&mut self.open)
            .into_iter()
            .partition(|l| l.list != list || items.contains(&l.item));
        self.open = kept;
        for l in ended {
            self.end(l, frame, false);
        }
        for &item in &items {
            if self.open.iter().any(|l| l.list == list && l.item == item) {
                continue;
            }
            self.open.push(Listed {
                list,
                item,
                player,
                frame,
                at,
                index: self.effects.len(),
            });
            self.effects.push(None);
        }
    }

    /// Files the effect `l` as ended at `frame`.
    fn end(&mut self, l: Listed, frame: Option<u32>, open: bool) {
        let Some(&kind) = self.item_type.get(&l.item) else {
            self.untyped += 1;
            return;
        };
        // The snapshot is the recording's first moment.
        let first = self.clock.frame_times.first().copied().unwrap_or(0.0);
        let start = self.seconds(l.frame).unwrap_or(first);
        let end = self.seconds(frame).unwrap_or(start);
        let effect = Effect {
            username: self.name(l.player),
            kind,
            name: effect_name(kind),
            buff: self.item_state.get(&l.item) == Some(&BUFF),
            start: self.stamp(l.frame, l.at),
            seconds: round_ms((end - start).max(0.0)),
            open,
            jammer: None,
            jammer_source: None,
        };
        if let Some(slot) = self.effects.get_mut(l.index) {
            *slot = Some(effect);
        }
    }

    fn reverse(&mut self, p: usize, on: bool, e: &Event) {
        let stamp = self.stamp(e.frame, e.at);
        let username = self.name(p);
        let Some(l) = self.life.get_mut(p) else {
            return;
        };
        let before = l.reverse.replace(on);
        // The game sends the value again without it having changed.
        if on && before != Some(true) {
            l.reverse_open = Some(self.out.friendly_fire.len());
            let live = e.frame.is_some();
            self.out.friendly_fire.push(ReverseFriendlyFire {
                username,
                active_at_start: !live,
                on: live.then_some(stamp),
                off: None,
            });
        } else if !on
            && before == Some(true)
            && let Some(open) = l.reverse_open.take()
            && let Some(f) = self.out.friendly_fire.get_mut(open)
        {
            f.off = Some(stamp);
        }
    }

    fn flashed(&mut self, on: bool, e: &Event) {
        let before = self.flashed.replace(on);
        if on && before != Some(true) {
            self.flash = Some((e.frame, e.at));
        } else if !on && before == Some(true) {
            self.end_flash(e.frame, false);
        }
    }

    fn end_flash(&mut self, frame: Option<u32>, open: bool) {
        let Some((from, at)) = self.flash.take() else {
            return;
        };
        let recorder = self
            .players
            .iter()
            .find(|p| p.relation == Some(Relation::You));
        let Some(recorder) = recorder else { return };
        let first = self.clock.frame_times.first().copied().unwrap_or(0.0);
        let start = self.seconds(from).unwrap_or(first);
        let end = self.seconds(frame).unwrap_or(start);
        self.out.flashes.push(Flash {
            username: recorder.username.clone(),
            start: self.stamp(from, at),
            seconds: round_ms((end - start).max(0.0)),
            open,
        });
    }

    /// Finds what raised `p`'s health and files it. Returns the cause for
    /// the frame's sample.
    fn rise(&mut self, p: usize, rise: Rise, c: &Context, e: &Event) -> Option<Cause> {
        let l = self.life.get(p)?.clone();
        let (new, amount) = (rise.health, rise.amount);
        let old = new - amount;
        let now = rise.recording_time;
        let within = |time: f64, before: f64, after: f64| {
            now.is_some_and(|t| time - t >= -before - EPS && time - t <= after + EPS)
        };
        // The stim fired closest to the rise.
        let gap = |d: &Drop| now.map_or(f64::MAX, |t| (d.time - t).abs());
        let stim = self
            .stims
            .iter()
            .filter(|d| within(d.time, STIM_BEFORE, NEAR))
            .min_by(|a, b| gap(a).total_cmp(&gap(b)))
            .copied();
        let surge_used = self.surges.iter().any(|d| within(d.time, NEAR, NEAR));
        let plated = l.plated.is_some_and(|(frame, time)| {
            frame == c.frame || time.is_some_and(|t| within(t, PLATE_BEFORE, 0.0))
        });
        let kona_scored = self
            .kona_scores
            .iter()
            .any(|&t| within(t, 0.0, SCORE_AFTER));
        let at_action = self
            .action
            .zip(now)
            .is_some_and(|(action, t)| (t - action).abs() < ACTION_RESET);
        let full = Some(new) == l.max;
        let overhealed = Some(new) == l.overhealed_max;
        let overheal = l.max.map_or(0, |m| new.saturating_sub(m));
        // A spawn sets the health of an operator just picked.
        if old == 0 && l.state == Some(0) && full && !c.was_down && !c.revived {
            return None;
        }
        let kind = if old == 0 {
            // Up from a down: by a stim, by a surge, or by a teammate's
            // hands, which gives 20 and is no heal.
            if stim.is_some() {
                HealKind::DocStim
            } else if c.surged {
                HealKind::FinkaSurge
            } else {
                return c.revived.then_some(Cause::Revive);
            }
        } else if c.surged {
            HealKind::FinkaSurge
        } else if plated && amount == PLATE {
            let at = self.stamp(e.frame, e.at);
            let by = self.teammate(p, "Rook");
            self.out.plates.push(Plate {
                username: rise.username,
                by,
                time: at.time,
                phase: at.phase,
                elapsed: at.elapsed,
                recording_time: at.recording_time,
            });
            return Some(Cause::Plate);
        } else if stim.is_some() && overhealed {
            HealKind::DocStim
        } else if c.kona || (kona_scored && (amount == 20 || amount == 21)) {
            if amount >= 20 {
                HealKind::KonaBurst
            } else {
                HealKind::KonaTick
            }
        } else if amount == SURGE_HEAL && surge_used && self.active(p).contains(&SURGE) {
            // A second surge while the first still shows adds no item.
            HealKind::FinkaSurge
        } else if at_action && full {
            // Damage taken in prep is undone when action starts.
            return None;
        } else {
            self.out.unexplained.push(rise);
            return None;
        };
        let by = match (kind, stim) {
            (HealKind::DocStim, Some(stim)) => Some(self.name(stim.player)),
            _ => self.teammate(p, kind.operator()),
        };
        let at = self.stamp(e.frame, e.at);
        self.out.heals.push(Heal {
            username: rise.username,
            by,
            amount,
            health: new,
            overheal,
            kind,
            revive: c.revived,
            time: at.time,
            phase: at.phase,
            elapsed: at.elapsed,
            recording_time: at.recording_time,
        });
        Some(kind.cause())
    }

    fn finish(mut self, malformed: usize) -> Vitals {
        let last = self
            .clock
            .frame_times
            .len()
            .checked_sub(1)
            .and_then(|f| u32::try_from(f).ok());
        for l in std::mem::take(&mut self.open) {
            self.end(l, last, true);
        }
        self.end_flash(last, true);
        self.out.effects = std::mem::take(&mut self.effects)
            .into_iter()
            .flatten()
            .collect();
        for (l, v) in self.life.iter().zip(&mut self.out.players) {
            // Attackers change operator in prep, so the maximum that counts
            // is the one action starts with, less the plates it includes.
            // Without an action phase, the first one the player was given.
            v.max_health = match l.at_action {
                Some((max, plates)) => Some(max.saturating_sub(PLATE * plates)),
                None => l.first_max,
            };
        }
        if malformed > 0 {
            self.out.warnings.push(format!(
                "{malformed} health or status values of a size their property never has were skipped"
            ));
        }
        if self.untyped > 0 {
            self.out.warnings.push(format!(
                "{} listed effects whose item states no type were skipped",
                self.untyped
            ));
        }
        self.out
    }
}

/// What the frame of a rise says about it.
struct Context {
    frame: Option<u32>,
    /// The player was down before the frame.
    was_down: bool,
    /// The frame revived the player.
    revived: bool,
    /// A `ThunderbirdHeal` effect was listed before the frame, or its item
    /// was written in it.
    kona: bool,
    /// A `FinkaSurge` item was written for the player in the frame.
    surged: bool,
}

fn resolve(hud: Hud, players: &[Player], loadouts: &[Loadout], clock: &Clock) -> Vitals {
    let mut tracker = Tracker::new(players, loadouts, clock, &hud.lists);
    tracker.read_scores(&hud.events);
    // A frame has one record, so its events sit together.
    for frame in hud.events.chunk_by(|a, b| a.frame == b.frame) {
        tracker.frame(frame);
    }
    tracker.finish(hud.malformed)
}

/// Reads the players' health, heals, plates, effects, reverse friendly fire
/// and the recorder's flashes from the state stream. `loadouts` are the
/// round's, for when the healing abilities were used.
pub(crate) fn decode(
    data: &[u8],
    map: &RecordMap,
    streams: &[StreamInfo],
    players: &[Player],
    loadouts: &[Loadout],
    clock: &Clock,
) -> Vitals {
    let mut hud = Hud::new(players);
    for (start, end, frame) in blocks(map, streams, STATE_STREAM) {
        if let Some(block) = data.get(start..end) {
            hud.read(block, start, frame);
        }
    }
    resolve(hud, players, loadouts, clock)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::PlayerEntities;
    use crate::loadout::{Counted, Counts, Use};
    use crate::timeline::Timeline;
    use crate::types::Operator;

    const FINKA: Operator = Operator(104189661965);
    const DOC: Operator = Operator(92270644007);
    const ROOK: Operator = Operator(92270644059);

    const CONTROLLER: u32 = 0xF000_0100;
    const LIFE: u32 = 0xF000_0200;
    const LIST: u32 = 0xF000_0300;
    const FEEDBACK: u32 = 0xF000_0400;
    const OTHER: Hash = [9, 9, 9, 9];

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
    fn link(d: &mut Vec<u8>, parent: u32, field: Hash, child: u32) {
        d.push(0x1B);
        d.extend(parent.to_le_bytes());
        d.extend([0; 4]);
        d.extend(field);
        d.extend(child.to_le_bytes());
        d.extend([0; 4]);
        d.extend(OTHER);
    }

    /// `1e <field> <index> <child> 00000000 <class>`.
    fn element(d: &mut Vec<u8>, field: Hash, index: u32, child: u32) {
        d.push(0x1E);
        d.extend(field);
        d.extend(index.to_le_bytes());
        d.extend(child.to_le_bytes());
        d.extend([0; 4]);
        d.extend(OTHER);
    }

    /// Player `i` with its objects: every id is the constant plus `i`.
    fn player(i: u32, operator: Operator, team: usize) -> Player {
        Player {
            id: u64::from(i) + 1,
            username: format!("p{i}"),
            team_index: team,
            operator,
            entities: Some(PlayerEntities {
                controller: CONTROLLER + i,
                health: Some(LIFE + i),
                ..PlayerEntities::default()
            }),
            ..Player::default()
        }
    }

    /// A round under construction: players, and the HUD fed frame by frame.
    /// Frame `n` is `n / 10` seconds into the recording.
    struct Fixture {
        players: Vec<Player>,
        loadouts: Vec<Loadout>,
        hud: Hud,
    }

    impl Fixture {
        fn new(players: Vec<Player>) -> Self {
            let mut hud = Hud::new(&players);
            // The snapshot links each player's view models.
            let mut d = vec![];
            for i in 0..players.len() as u32 {
                link(&mut d, CONTROLLER + i, EFFECTS_FIELD, LIST + i);
                link(&mut d, CONTROLLER + i, FRIENDLY_FIRE_FIELD, FEEDBACK + i);
            }
            hud.read(&d, 0, None);
            Fixture {
                players,
                loadouts: Vec::new(),
                hud,
            }
        }

        fn snapshot(&mut self, block: &[u8]) {
            self.hud.read(block, 10, None);
        }

        fn frame(&mut self, frame: u32, block: &[u8]) {
            self.hud.read(block, 1000 * frame as usize, Some(frame));
        }

        /// Life properties of player `i` in one frame.
        fn life(&mut self, frame: u32, i: u32, values: &[(Hash, u32)]) {
            let mut d = vec![];
            for (n, (hash, v)) in values.iter().enumerate() {
                if n == 0 {
                    set(&mut d, LIFE + i, *hash, &v.to_le_bytes());
                } else {
                    prop(&mut d, *hash, &v.to_le_bytes());
                }
            }
            self.frame(frame, &d);
        }

        /// Player `i`'s ability count dropped at these recording times.
        fn uses(&mut self, i: usize, times: &[f64]) {
            let p = &self.players[i];
            let mut l = Loadout::new(&p.username, p.operator);
            let uses = times.iter().map(|&t| Use {
                count: 0,
                time: String::new(),
                phase: Phase::Action,
                elapsed: 0.0,
                recording_time: Some(t),
            });
            l.ability = Some(Counted {
                counts: Some(Counts {
                    uses: uses.collect(),
                    ..Counts::default()
                }),
                ..Counted::default()
            });
            self.loadouts.push(l);
        }

        fn resolve(self) -> Vitals {
            let times: Vec<f64> = (0..1000).map(|f| f64::from(f) / 10.0).collect();
            let timeline = Timeline::default();
            let clock = Clock {
                timeline: &timeline,
                reading_offsets: &[],
                frame_times: &times,
            };
            resolve(self.hud, &self.players, &self.loadouts, &clock)
        }
    }

    fn one_player() -> Fixture {
        let mut f = Fixture::new(vec![player(0, DOC, 0)]);
        let mut d = vec![];
        set(&mut d, LIFE, HEALTH, &100u32.to_le_bytes());
        prop(&mut d, MAX_HEALTH, &100u32.to_le_bytes());
        prop(&mut d, OVERHEALED_MAX_HEALTH, &120u32.to_le_bytes());
        prop(&mut d, LIFE_STATE, &0u32.to_le_bytes());
        f.snapshot(&d);
        f
    }

    fn kinds(v: &Vitals) -> Vec<(LifeKind, Option<u32>)> {
        v.life.iter().map(|l| (l.kind, l.health)).collect()
    }

    #[test]
    fn a_death_through_state_two_is_not_a_revive() {
        let mut f = one_player();
        f.life(10, 0, &[(LIFE_STATE, 3)]);
        f.life(11, 0, &[(HEALTH, 0)]);
        // Finished while down: 3, 2, 4 with no health.
        f.life(50, 0, &[(LIFE_STATE, 2)]);
        f.life(51, 0, &[(LIFE_STATE, 4)]);
        let v = f.resolve();
        assert_eq!(kinds(&v), [(LifeKind::Down, None)]);
        assert_eq!(v.life[0].recording_time, Some(1.0));
        assert!(v.heals.is_empty() && v.unexplained.is_empty());
    }

    #[test]
    fn a_revive_needs_health() {
        // In the frame of the state, whichever is written first.
        for health_first in [true, false] {
            let mut f = one_player();
            f.life(10, 0, &[(LIFE_STATE, 3), (HEALTH, 0)]);
            if health_first {
                f.life(40, 0, &[(HEALTH, 20), (LIFE_STATE, 2)]);
            } else {
                f.life(40, 0, &[(LIFE_STATE, 2), (HEALTH, 20)]);
            }
            let v = f.resolve();
            assert_eq!(
                kinds(&v),
                [(LifeKind::Down, None), (LifeKind::Revive, Some(20))]
            );
            assert_eq!(v.life[1].recording_time, Some(4.0));
            // Hands give 20: a revive, not a heal.
            assert!(v.heals.is_empty() && v.unexplained.is_empty());
            let sample = v.sample("p0", Some(4.0)).unwrap();
            assert_eq!((sample.health, sample.state), (20, 2));
            assert_eq!(sample.cause, Some(Cause::Revive));
        }
    }

    #[test]
    fn a_revive_can_get_its_health_a_frame_later() {
        let mut f = one_player();
        f.life(10, 0, &[(LIFE_STATE, 3), (HEALTH, 0)]);
        f.life(40, 0, &[(LIFE_STATE, 2)]);
        f.life(41, 0, &[(HEALTH, 20)]);
        let v = f.resolve();
        assert_eq!(
            kinds(&v),
            [(LifeKind::Down, None), (LifeKind::Revive, Some(20))]
        );
        assert_eq!(v.life[1].recording_time, Some(4.1));
    }

    #[test]
    fn a_stim_revives_to_overheal() {
        let mut f = one_player();
        f.uses(0, &[3.8]);
        f.life(10, 0, &[(LIFE_STATE, 3), (HEALTH, 0)]);
        let mut d = vec![];
        set(&mut d, LIFE, DBNO_PROGRESS, &0.25f32.to_le_bytes());
        f.frame(20, &d);
        // 3 to 1: overhealed straight from the down.
        f.life(40, 0, &[(HEALTH, 120), (LIFE_STATE, 1)]);
        f.life(50, 0, &[(HEALTH, 119)]);
        let v = f.resolve();
        assert_eq!(
            kinds(&v),
            [(LifeKind::Down, None), (LifeKind::Revive, Some(120))]
        );
        assert_eq!(v.life[0].bleed_out, Some(0.25));
        let [heal] = &v.heals[..] else {
            panic!("{:?}", v.heals)
        };
        assert_eq!(heal.kind, HealKind::DocStim);
        assert_eq!((heal.amount, heal.health, heal.overheal), (120, 120, 20));
        assert!(heal.revive);
        assert_eq!(heal.by.as_deref(), Some("p0"));
        assert_eq!(v.decay, [("p0".to_owned(), Some(5.0))]);
        let cause = |t| v.sample("p0", Some(t)).unwrap().cause;
        assert_eq!(cause(4.0), Some(Cause::DocStim));
        assert_eq!(cause(5.0), Some(Cause::Decay));
    }

    /// One item listed on player `i`.
    fn list_write(i: u32, items: &[u32]) -> Vec<u8> {
        let mut d = vec![];
        set(&mut d, LIST + i, EFFECT_LIST, &[items.len() as u8]);
        for (n, item) in items.iter().enumerate() {
            element(&mut d, EFFECT_LIST, n as u32, *item);
        }
        d
    }

    fn item(d: &mut Vec<u8>, obj: u32, kind: u32, state: u32) {
        set(d, obj, EFFECT_TYPE, &kind.to_le_bytes());
        prop(d, EFFECT_STATE, &state.to_le_bytes());
    }

    #[test]
    fn effects_start_and_end_with_the_list_writes() {
        let mut f = one_player();
        let (a, b, c) = (0xF000_0501, 0xF000_0502, 0xF000_0503);
        // Start: one item.
        let mut d = list_write(0, &[a]);
        item(&mut d, a, 34, 2);
        f.frame(10, &d);
        // Replace: another item takes its place, and a third joins.
        let mut d = vec![];
        item(&mut d, b, 8, 3);
        item(&mut d, c, 777, 3);
        d.extend(list_write(0, &[b, c]));
        f.frame(25, &d);
        // The same list sent again changes nothing.
        f.frame(30, &list_write(0, &[b, c]));
        // Empty: the first of the two ends, then the list is cleared in a
        // frame that also holds an empty slot.
        f.frame(40, &list_write(0, &[c]));
        let mut d = list_write(0, &[]);
        element(&mut d, EFFECT_LIST, 0, 0);
        f.frame(45, &d);
        // One still listed at the end.
        f.frame(990, &list_write(0, &[a]));
        let v = f.resolve();

        let seen: Vec<_> = v
            .effects
            .iter()
            .map(|e| {
                (
                    e.kind,
                    e.name,
                    e.buff,
                    e.start.recording_time,
                    e.seconds,
                    e.open,
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                (34, Some("FriendlyJammer"), true, Some(1.0), 1.5, false),
                (8, Some("EnemyJammer"), false, Some(2.5), 1.5, false),
                (777, None, false, Some(2.5), 2.0, false),
                (34, Some("FriendlyJammer"), true, Some(99.0), 0.9, true),
            ]
        );
        assert!(v.effects.iter().all(|e| e.username == "p0"));
        assert!(v.warnings.is_empty());
    }

    #[test]
    fn an_effect_listed_in_the_snapshot_starts_with_the_recording() {
        let mut f = one_player();
        let a = 0xF000_0501;
        let mut d = list_write(0, &[a]);
        item(&mut d, a, 4, 2);
        f.snapshot(&d);
        f.frame(20, &list_write(0, &[]));
        // An item that never states its type is skipped and counted.
        f.frame(30, &list_write(0, &[0xF000_0599]));
        let v = f.resolve();
        let [e] = &v.effects[..] else {
            panic!("{:?}", v.effects)
        };
        assert_eq!((e.kind, e.name, e.buff), (4, Some("RookArmor"), true));
        assert_eq!((e.start.recording_time, e.seconds), (None, 2.0));
        assert_eq!(v.warnings.len(), 1, "{:?}", v.warnings);
    }

    #[test]
    fn a_surge_heals_whoever_gets_its_effect() {
        let mut f = Fixture::new(vec![player(0, FINKA, 0), player(1, ROOK, 0)]);
        f.uses(0, &[5.0, 9.0]);
        for i in 0..2 {
            let mut d = vec![];
            set(&mut d, LIFE + i, HEALTH, &90u32.to_le_bytes());
            prop(&mut d, MAX_HEALTH, &100u32.to_le_bytes());
            prop(&mut d, OVERHEALED_MAX_HEALTH, &120u32.to_le_bytes());
            prop(&mut d, LIFE_STATE, &0u32.to_le_bytes());
            f.snapshot(&d);
        }
        let surge = 0xF000_0501;
        let mut d = list_write(1, &[surge]);
        item(&mut d, surge, SURGE, 2);
        set(&mut d, LIFE + 1, HEALTH, &110u32.to_le_bytes());
        prop(&mut d, LIFE_STATE, &1u32.to_le_bytes());
        f.frame(50, &d);
        // A second surge while the effect still shows writes no new item:
        // Finka's count dropping stands in for it.
        f.life(60, 1, &[(HEALTH, 95), (LIFE_STATE, 0)]);
        f.life(90, 1, &[(HEALTH, 115), (LIFE_STATE, 1)]);
        // A rise nothing explains.
        f.life(200, 1, &[(HEALTH, 118)]);
        let v = f.resolve();

        let heals: Vec<_> = v
            .heals
            .iter()
            .map(|h| (h.kind, h.amount, h.health, h.overheal, h.by.as_deref()))
            .collect();
        assert_eq!(
            heals,
            [
                (HealKind::FinkaSurge, 20, 110, 10, Some("p0")),
                (HealKind::FinkaSurge, 20, 115, 15, Some("p0")),
            ]
        );
        assert!(v.heals.iter().all(|h| h.username == "p1" && !h.revive));
        let [rise] = &v.unexplained[..] else {
            panic!("{:?}", v.unexplained)
        };
        assert_eq!((rise.amount, rise.health), (3, 118));
        // Damage is not decay: the drop of 15 has no cause.
        assert_eq!(v.sample("p1", Some(6.0)).unwrap().cause, None);
        assert!(v.decay.is_empty());
    }

    #[test]
    fn a_plate_raises_health_and_its_maximum() {
        let mut f = Fixture::new(vec![player(0, ROOK, 1), player(1, DOC, 1)]);
        for i in 0..2 {
            let mut d = vec![];
            set(&mut d, LIFE + i, HEALTH, &0u32.to_le_bytes());
            prop(&mut d, MAX_HEALTH, &0u32.to_le_bytes());
            prop(&mut d, LIFE_STATE, &0u32.to_le_bytes());
            f.snapshot(&d);
        }
        // The spawn is no heal.
        f.life(5, 1, &[(MAX_HEALTH, 110), (HEALTH, 110)]);
        f.life(30, 1, &[(MAX_HEALTH, 135), (HEALTH, 135)]);
        let v = f.resolve();
        let [plate] = &v.plates[..] else {
            panic!("{:?}", v.plates)
        };
        assert_eq!(
            (plate.username.as_str(), plate.by.as_deref()),
            ("p1", Some("p0"))
        );
        assert_eq!(plate.recording_time, Some(3.0));
        assert!(v.heals.is_empty() && v.unexplained.is_empty());
        let sample = v.sample("p1", Some(3.0)).unwrap();
        assert_eq!((sample.max_health, sample.cause), (135, Some(Cause::Plate)));
        // The base is the maximum without the plate.
        assert_eq!(v.players[1].max_health, Some(110));
        assert_eq!(v.players[0].max_health, None);
    }

    #[test]
    fn reverse_friendly_fire_turns_on_and_off() {
        let mut f = Fixture::new(vec![player(0, DOC, 0), player(1, ROOK, 0)]);
        let mut d = vec![];
        set(&mut d, FEEDBACK, REVERSE_FRIENDLY_FIRE, &[0]);
        set(&mut d, FEEDBACK + 1, REVERSE_FRIENDLY_FIRE, &[1]);
        f.snapshot(&d);
        let write = |i: u32, v: u8| {
            let mut d = vec![];
            set(&mut d, FEEDBACK + i, REVERSE_FRIENDLY_FIRE, &[v]);
            d
        };
        f.frame(20, &write(0, 1));
        // Sent again, unchanged.
        f.frame(25, &write(0, 1));
        f.frame(70, &write(0, 0));
        let v = f.resolve();
        let seen: Vec<_> = v
            .friendly_fire
            .iter()
            .map(|f| {
                let time = |at: &Option<At>| at.as_ref().and_then(|a| a.recording_time);
                (
                    f.username.as_str(),
                    f.active_at_start,
                    time(&f.on),
                    time(&f.off),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                ("p1", true, None, None),
                ("p0", false, Some(2.0), Some(7.0))
            ]
        );
    }

    #[test]
    fn flashes_belong_to_the_recorder() {
        let flashes = |recorder: bool| {
            let mut me = player(0, DOC, 0);
            me.relation = recorder.then_some(Relation::You);
            let mut f = Fixture::new(vec![me, player(1, ROOK, 1)]);
            // The game writes the recorder's state to every controller.
            for (frame, v) in [(30, 1u8), (52, 0), (80, 1)] {
                let mut d = vec![];
                set(&mut d, CONTROLLER, FLASHED, &[v]);
                set(&mut d, CONTROLLER + 1, FLASHED, &[v]);
                f.frame(frame, &d);
            }
            f.resolve().flashes
        };
        let seen: Vec<_> = flashes(true)
            .iter()
            .map(|f| {
                (
                    f.username.clone(),
                    f.start.recording_time,
                    f.seconds,
                    f.open,
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                ("p0".to_owned(), Some(3.0), 2.2, false),
                ("p0".to_owned(), Some(8.0), 91.9, true)
            ]
        );
        assert!(flashes(false).is_empty(), "a spectator is not flashed");
    }

    #[test]
    fn malformed_bytes_are_skipped_and_counted() {
        let mut f = one_player();
        let mut d = vec![];
        // A health of three bytes, a life state of one.
        set(&mut d, LIFE, HEALTH, &[1, 2, 3]);
        prop(&mut d, LIFE_STATE, &[3]);
        set(&mut d, FEEDBACK, REVERSE_FRIENDLY_FIRE, &[1, 2]);
        f.frame(10, &d);
        // Records cut anywhere do not panic.
        let mut whole = list_write(0, &[0xF000_0501]);
        item(&mut whole, 0xF000_0501, 2, 2);
        set(&mut whole, LIFE, HEALTH, &120u32.to_le_bytes());
        for cut in 0..whole.len() {
            let mut hud = Hud::new(&f.players);
            hud.lists.insert(LIST, 0);
            hud.read(&whole[..cut], 0, Some(1));
            hud.read(&whole[cut..], 0, Some(2));
        }
        let v = f.resolve();
        assert!(v.life.is_empty() && v.friendly_fire.is_empty());
        assert_eq!(v.warnings.len(), 1);
        assert!(v.warnings[0].starts_with("3 "), "{:?}", v.warnings);
    }

    #[test]
    fn effect_names_are_those_of_the_table() {
        assert_eq!(effect_name(2), Some("FinkaSurge"));
        assert_eq!(effect_name(35), Some("ThunderbirdHeal"));
        assert_eq!(effect_name(15), Some("OutsideWarning"));
        assert_eq!(effect_name(51), Some("DokkaebiOverload"));
        assert_eq!(effect_name(29), None);
    }
}
