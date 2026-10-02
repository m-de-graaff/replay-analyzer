//! What happens to gadgets and what gadgets do to players (Y11S3): score
//! changes, gadget removals with who and how, statuses put on gadgets
//! (disabled, frozen, hacked, caught, captured) and traps set off.
//!
//! # What is read
//!
//! - **Score.** `MatchScore` (`ec da 4f 80`, an `i32`: a team killer goes
//!   below zero) on each player's scoreboard object. The game writes one
//!   total per frame: everything a player earned in that frame added up.
//!   There is no feed of what the points were for.
//! - **Removal signals**, from the [`World`]: an entity that was `live`
//!   going to 0 (`notLive`), the destroyed byte of a device component
//!   (`destroyedFlag`), flags `4000` on a placed gadget (`inert`), an `fe`
//!   entry in its damage list (`broken`), its delete message (`deleted`),
//!   and the owner component's released flag going back to 0 (`returned`).
//!   Signals within 0.3 s of the first are one removal.
//! - **Statuses**, from the [`Effects`]: an effect spawned with the
//!   gadget as its parent, until its stop. Which effect asset is which
//!   status is inferred (who is in every round it shows in, who scores).
//!   `captured` is the alliance of the owner component being rewritten.
//! - **Traps.** Razorbloom's state machine going from `Closed` to
//!   `Opening`; the HUD's list of Fenrir's mines (class `e238442b`) whose
//!   `State` becomes 3; an effect on a Proximity Alarm or a Banshee; a
//!   Welcome Mat going inert; a Gu mine, Entry Denial Device or Claymore
//!   deleted without first going out of use.
//!
//! # What is inferred
//!
//! Nothing in the file names who destroyed a gadget or with what.
//!
//! - `by` (`bySource: score`): the player whose score rises by the
//!   gadget's points within [`SCORE_WINDOW`] of the removal. A teammate's
//!   penalty (-10 each) is paired first, then the other team's points (10
//!   each; 20 for a Black Eye, Welcome Mat or Claymore, 5 for a T.R.I.P.
//!   Connector). One write pays for as many removals as its amount
//!   covers; when more removals were in the window than it pays for,
//!   those it took say `ambiguous`.
//! - `means` (`meansSource: shotRay`): a shot of that player passed
//!   within 0.6 m of the gadget in the 0.45 s before. Else
//!   (`meansSource: proximity`) an explosive of theirs ended within
//!   0.35 s and 8 m.
//! - `cause` without a scorer, by what the type is known to do: see
//!   [`Cause`]. `pickedUp` is the HUD's state of one of the owner's
//!   gadgets of that name going back to 0. `causeSource` says where a
//!   cause is from: `signals` (the destroyed flag of a device, an `fe`
//!   entry, a wire or a mat gone inert: read), `score`, `adsFired`,
//!   `type` (what that pattern of signals means for the type),
//!   `hudGadgetState`, or `time` (the recording ending).
//! - A trap's victims (`victimSource: time`): the hits and status
//!   effects of the type the trap deals, in the same frames. Its
//!   `nearestEnemy` (`nearestEnemySource: nearest`) is by the bodies.
//! - A score change's `reason` (`reasonSource: coincidence`): what else
//!   happened in the same frames.
//! - A gadget's `name` is that of the loadout slot of the body that
//!   carries its asset, else of a short table here. `username` is read
//!   from the entity (`usernameSource: read`), else the only player who
//!   carries the asset (`carrier`), else the only player of the operator
//!   the asset belongs to (`operator`).
//!
//! # Not in the file
//!
//! Who a Grzmot mine stunned and what an Airjab or Trax did to whom: no
//! marker was found. Barbed wire has none either: [`WireHit`] gives each
//! hit of its damage type the nearest wire in use (12 of 14 hits in the
//! test rounds have one within 1.5 m). A gadget lost to a cause that
//! gives no score (its owner's own explosive, fire) has `cause: unknown`
//! unless a device says `destroyedFlag`.
//!
//! [`Jam`] and [`WireHit`] are not written by themselves: they are for
//! the `effects[]` and `hits[]` entries they explain.
//!
//! # Where it is written
//!
//! What is about a gadget of `gadgets[]` is written on that gadget: its
//! removal as the gadget's `end`, its statuses and its triggers (see
//! [`crate::join`]). What is left is written as `deviceRemovals` (drones
//! and cameras, which are no gadgets there), `gadgetStatuses` and
//! `trapTriggers`. `scoreChanges` is written whole.

use std::cell::Cell;
use std::collections::HashMap;

use serde::{Serialize, Serializer};

use crate::combat::TimelineKind;
use crate::details::Loadout;
use crate::entities::{Hash, Record, for_each_record};
use crate::feedback::{MatchUpdate, MatchUpdateType};
use crate::fx::Effects;
use crate::loadout::{Input, When};
use crate::world::{self, Entity, World};

/// Name hash of the stream the HUD is in.
const STATE_STREAM: Hash = [0xA9, 0x8F, 0xDD, 0x0B];

/// Scoreboard: the player's score (`MatchScore`), an `i32`.
const MATCH_SCORE: Hash = [0xEC, 0xDA, 0x4F, 0x80];
/// `State` of a gadget state item or of a mine of Fenrir.
const STATE: Hash = [0xFF, 0xFD, 0x52, 0x62];
/// Gadget view model -> its list of state items (`GadgetStates`).
const GADGET_STATES: Hash = [0xB0, 0x10, 0xFA, 0x49];
/// Class of a gadget state item.
const GADGET_STATE_ITEM: Hash = [0x21, 0x65, 0xE7, 0x92];
/// Class of a gadget view model.
const GADGET_VIEW: Hash = [0x9F, 0x44, 0x69, 0x0A];
/// Controller -> the loadout view the gadget view models hang under.
const LOADOUT_FIELD: Hash = [0xE8, 0xD1, 0xE5, 0x39];
/// Loadout view -> the gadget and ability view models.
const GADGET_FIELD: Hash = [0xD8, 0x90, 0xB5, 0xF7];
const ABILITY_FIELD: Hash = [0x4C, 0xD6, 0xA0, 0xC7];
/// Class of one of Fenrir's mines in the HUD.
const FENRIR_MINE: Hash = [0xE2, 0x38, 0x44, 0x2B];
/// Fenrir mine: the room it lies in, as an array whose first element
/// holds the text.
const LOCATION: Hash = [0x9D, 0xEB, 0xE8, 0xA7];
/// Class of the object that counts a team's reinforcements, and its
/// count of those left.
const REINFORCEMENT_POOL: Hash = [0x80, 0xDE, 0xB4, 0xBC];
const POOL_LEFT: Hash = [0x67, 0xDE, 0x20, 0xF8];

/// Body slots: `(slot hash, what the loadout calls it)`.
const BODY_SLOTS: [(Hash, Slot); 4] = [
    ([0x08, 0x2C, 0xA3, 0x1D], Slot::Ability),
    ([0xD8, 0x55, 0xB4, 0xAF], Slot::Gadget),
    ([0x41, 0x20, 0x14, 0x8B], Slot::Gadget),
    ([0x60, 0x3E, 0x4A, 0x2F], Slot::Drone),
];

/// State machine blobs of the state component and where the state (the
/// CRC-32 of its name) is in the bytes after the blob's hash.
const MACHINE_A: Hash = [0xAD, 0x55, 0x0A, 0x19];
const MACHINE_B: Hash = [0x95, 0xB3, 0x88, 0xF2];
const CLOSED: Hash = [0xD2, 0x48, 0x6B, 0x03];
const OPENING: Hash = [0x5F, 0xED, 0x88, 0xC1];
const OPENED: Hash = [0x6D, 0x22, 0xFA, 0xAA];

/// A map prop's only class; such an entity is no gadget.
const PROP_CLASS: Hash = [0x6D, 0x47, 0x0B, 0xEB];

/// Flags of a placed gadget that is spent or broken and stays where it is.
const INERT: u16 = 0x4000;

/// Effect assets that are a status on the gadget they are spawned on.
/// Inferred from 183 rounds: who is in every round the asset shows in,
/// who scores at that moment and how long it lasts.
const STATUS_FX: [(u64, StatusKind); 12] = [
    (391431562118, StatusKind::EmpDisabled),
    (393296342898, StatusKind::EmpDisabled),
    (393296343343, StatusKind::EmpDisabled),
    (393296343037, StatusKind::EmpDisabled),
    (402687683592, StatusKind::Frozen),
    (393567073348, StatusKind::Hacked),
    (393567073349, StatusKind::Hacked),
    (393567073351, StatusKind::Hacked),
    (393567073352, StatusKind::Hacked),
    (395176464181, StatusKind::Hacking),
    (243110757477, StatusKind::Caught),
    (11379520028, StatusKind::AdsFired),
];

/// Effects spawned on a trap as it goes off: `(asset, trap, the HUD
/// effect type its victim gets in the same frame)`.
const TRAP_FX: [(u64, &str, u32); 2] = [
    (73593090661, PROXIMITY_ALARM, 13),
    (262855816735, "Banshee Sonic Defense", 14),
];

/// Assets no body slot carries: `(asset, name, operator)`. The names
/// are inferred from the rounds they show in.
const ASSETS: [(u64, &str, &str); 20] = [
    (375382544571, "Kiba Barrier (deployed)", "Azami"),
    (377423931277, "R.O.U. light screen", "Sens"),
    (385049618000, "Kawan swarm", "Grim"),
    (385049526209, "Kawan Hive", "Grim"),
    (378015312553, "Candela flash charge", "Ying"),
    (378015312549, "Cluster Charge puck", "Fuze"),
    (388352500293, "S.E.L.M.A. charge (stuck)", "Ace"),
    (391794748337, "X-KAIROS pellet", "Hibana"),
    (383322885052, "Airjab", "Nomad"),
    (391794754080, "Pest", "Mozzie"),
    (369569666222, "Bulletproof Camera mount", ""),
    (311807951223, "Defuser", ""),
    (446645529444, "Horus Lance", "Noor"),
    (370092345902, "Candela", "Ying"),
    (391794703535, "Breaching round", "Ash"),
    (392233337653, "LV lance", "Kali"),
    (391794756179, "Shumikha grenade", "Tachanka"),
    (311776456053, "KS79 grenade", "Zofia"),
    (392233332963, "KS79 grenade", "Zofia"),
    (325532530887, "Black Mirror pane", "Mira"),
];
/// The defuser and a bulletproof camera's mount (the camera is its own
/// entity) are no gadgets.
const NOT_GADGETS: [u64; 2] = [311807951223, 369569666222];

const RAZORBLOOM: &str = "Razorbloom Shell";
const FENRIR: &str = "F-NATT Dread Mine";
const WELCOME_MAT: &str = "Welcome Mat";
const EDD: &str = "Entry Denial Device";
const CLAYMORE: &str = "Claymore";
const GU: &str = "Gu Mine";
const BARBED_WIRE: &str = "Barbed Wire";
const PROXIMITY_ALARM: &str = "Proximity Alarm";
const JAMMER: &str = "Signal Disruptor";
/// Gadgets that end by going off: their delete alone is `detonated`.
const DETONATE: [&str; 11] = [
    "Frag Grenade",
    "Stun Grenade",
    "Smoke Grenade",
    "Impact Grenade",
    "Impact EMP Grenade",
    "Nitro Cell",
    "Remote Gas Grenade",
    "Breach Charge",
    "Hard Breach Charge",
    "Exothermic Charge",
    "Volcan Canister",
];

/// Points one destroyed gadget is worth to an opponent.
fn points(name: Option<&str>) -> i32 {
    match name {
        Some("Black Eye" | WELCOME_MAT | CLAYMORE) => 20,
        Some("T.R.I.P. Connector") => 5,
        _ => 10,
    }
}

/// Score time minus removal time. The score follows a frame or two
/// after; it precedes by about 0.15 s for a Signal Disruptor, Shock Wire
/// or Claymore, whose entity lingers.
const SCORE_WINDOW: (f64, f64) = (-0.25, 0.13);
/// Signals this close to the first belong to the same removal.
const SIGNAL_WINDOW: f64 = 0.3;
/// A jammed player is within this of the Signal Disruptor that jams them.
const JAM_RANGE: f32 = 3.5;
/// A player hurt by barbed wire is within this of its origin.
const WIRE_RANGE: f32 = 1.5;
/// `hits[].type.id` of barbed wire.
const WIRE_DAMAGE: u32 = 12;

/// The loadout slot a body carries a gadget in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, std::hash::Hash)]
enum Slot {
    Ability,
    Gadget,
    Drone,
}

/// What a deployed entity showed as it left play, see the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Signal {
    Broken,
    Deleted,
    DestroyedFlag,
    Inert,
    NotLive,
    Returned,
}

/// Why a gadget left play. Only `destroyed` with `destroyedFlag` or
/// `broken` among the signals is read; the rest is inferred from the
/// score or from what the type is known to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Cause {
    /// Nothing tells: used up, picked up, destroyed without a score, or
    /// tidied away.
    #[default]
    Unknown,
    Destroyed,
    /// A projectile shot down by an Active Defense System.
    Intercepted,
    /// A trap that went off.
    Triggered,
    Detonated,
    /// A Mag-NET that caught something.
    Used,
    PickedUp,
    /// Deleted in the last second of the recording.
    RoundEnd,
}

impl Cause {
    fn is_unknown(&self) -> bool {
        *self == Cause::Unknown
    }
}

/// How a name of a player was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OwnerSource {
    /// The entity names the player.
    Read,
    /// The only player whose body carries the asset.
    Carrier,
    /// The only player of the operator the asset belongs to.
    Operator,
}

fn hex<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(&format_args!("{v:x}"))
}

fn hex_opt<S: Serializer>(v: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(v) => hex(v, s),
        None => s.serialize_none(),
    }
}

/// The gadget an event is about: its entity, what it is and whose.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Subject {
    /// The entity of the movement stream, in hex.
    #[serde(serialize_with = "hex")]
    pub entity: u64,
    pub asset: u64,
    /// The type index its placed or owner component states.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_index: Option<u16>,
    /// Inferred, see the module docs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The owner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username_source: Option<OwnerSource>,
}

/// One change of a player's score.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreChange {
    pub username: String,
    pub delta: i32,
    pub total: i32,
    /// Inferred from what happened in the same frames; absent when
    /// nothing known did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    /// The gadget or the player the reason is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// `coincidence`, with every reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_source: Option<&'static str>,
    #[serde(flatten)]
    pub when: When,
}

/// Why a gadget left play, who did it and with what: everything of a
/// removal that is not the gadget itself. A gadget of `gadgets[]` has it
/// in its `end`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    /// The team that held the gadget then: that of the alliance its
    /// owner component states, else the owner's. Left out of a gadget's
    /// `end` when it is the owner's team.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_team: Option<usize>,
    #[serde(skip_serializing_if = "Cause::is_unknown")]
    pub cause: Cause,
    /// How the cause was found: `signals` (read), `score`, `adsFired`,
    /// `type` (what the type is known to do), `hudGadgetState` or `time`
    /// (the recording ending).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause_source: Option<&'static str>,
    /// Who destroyed or intercepted it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// `score`, or `adsFired` for a projectile an Active Defense System
    /// of `by` shot down.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_source: Option<&'static str>,
    /// The score change that paid for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<i32>,
    /// `by` is on the owner's team.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub friendly: bool,
    /// More removals were in the score's window than it paid for.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub ambiguous: bool,
    /// `bullet` or `explosion`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub means: Option<&'static str>,
    /// `shotRay` or `proximity`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub means_source: Option<&'static str>,
    /// The gun of the shot, or the explosive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weapon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weapon_slot: Option<&'static str>,
    /// The Active Defense System that shot it down.
    #[serde(serialize_with = "hex_opt", skip_serializing_if = "Option::is_none")]
    pub interceptor: Option<u64>,
}

/// A deployed gadget leaving play.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Removal {
    #[serde(flatten)]
    pub subject: Subject,
    /// Where it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<[f32; 3]>,
    /// What the entity showed, sorted.
    pub signals: Vec<Signal>,
    #[serde(flatten)]
    pub verdict: Verdict,
    #[serde(flatten)]
    pub when: When,
    /// The frame of its first signal, to join it with its gadget.
    #[serde(skip)]
    pub frame: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StatusKind {
    AdsFired,
    Captured,
    Caught,
    EmpDisabled,
    Frozen,
    Hacked,
    Hacking,
}

/// A status put on a gadget: what it is, by whom and for how long.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    pub kind: StatusKind,
    /// The effect asset that shows it; absent for `captured`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fx_asset: Option<u64>,
    /// `captured`: the team that took the gadget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<usize>,
    /// Who did it: the player who scored for it. For `adsFired`, the
    /// owner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_source: Option<&'static str>,
    /// `adsFired`: the projectile shot down.
    #[serde(serialize_with = "hex_opt", skip_serializing_if = "Option::is_none")]
    pub target: Option<u64>,
    /// When the effect was stopped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<When>,
    #[serde(flatten)]
    pub when: When,
}

/// A status with the gadget it is on. A gadget of `gadgets[]` has its
/// statuses in its own `statuses`, without the subject.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    #[serde(flatten)]
    pub subject: Subject,
    #[serde(flatten)]
    pub shown: Shown,
    /// The frame it started in, to join it with its gadget.
    #[serde(skip)]
    pub frame: u32,
}

/// Someone a trap hurt or marked, by a hit or a status effect in the
/// same frames.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Victim {
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damage: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<crate::combat::HitResult>,
    /// The `hits[].type.id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damage_type: Option<u32>,
    /// The `effects[].type`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    /// `hit` or `effect`.
    pub source: &'static str,
    /// `time`.
    pub victim_source: &'static str,
}

/// A trap going off: what says so, whom it hurt and what it paid.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trigger {
    /// What says it went off.
    pub marker: String,
    /// Razorbloom: seconds since the recording started when it burst.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detonated_at: Option<f64>,
    /// Fenrir: the room the HUD names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<[f32; 3]>,
    pub victims: Vec<Victim>,
    /// The opponent whose body was nearest, and how far.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nearest_enemy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nearest_enemy_distance: Option<f32>,
    /// `nearest`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nearest_enemy_source: Option<&'static str>,
    /// What the owner scored for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<i32>,
    #[serde(flatten)]
    pub when: When,
}

/// A trigger with the trap it is of. A gadget of `gadgets[]` has its
/// triggers in its own `triggers`, without the trap.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrapTrigger {
    pub trap: String,
    /// The entity; absent for a mine of Fenrir's HUD list that no entity
    /// was matched to.
    #[serde(serialize_with = "hex_opt", skip_serializing_if = "Option::is_none")]
    pub entity: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_index: Option<u16>,
    /// The owner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(flatten)]
    pub trigger: Trigger,
    /// The frame it went off in, to join it with its gadget; `None` in
    /// the snapshot.
    #[serde(skip)]
    pub frame: Option<u32>,
}

/// A jammed player (HUD effect 8) and the Signal Disruptor nearest to
/// them or to a device of theirs. Inferred: `jammerSource: nearest`.
#[derive(Clone, Debug, PartialEq)]
pub struct Jam {
    pub username: String,
    /// Seconds since the recording started, as the effect has it.
    pub recording_time: f64,
    pub jammer: u64,
    pub jammer_owner: Option<String>,
    /// The player's device in range; `None` when it is their body.
    pub device: Option<u64>,
    pub distance: f32,
}

/// A hit of barbed wire (`hits[].type.id` 12) and the wire it was:
/// the nearest one in use within [`WIRE_RANGE`] of the victim's body.
/// Inferred: nothing marks a wire as it hurts.
#[derive(Clone, Debug, PartialEq)]
pub struct WireHit {
    /// Index into the round's hits.
    pub hit: usize,
    pub entity: u64,
    /// The wire's owner.
    pub username: Option<String>,
    pub distance: f32,
}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GadgetEvents {
    pub score: Vec<ScoreChange>,
    pub removals: Vec<Removal>,
    pub statuses: Vec<Status>,
    pub traps: Vec<TrapTrigger>,
    /// For the `effects[]` entries of type 8.
    pub jams: Vec<Jam>,
    /// For the `hits[]` entries of type 12.
    pub wire_hits: Vec<WireHit>,
    /// Hits of type 12 with no wire in range.
    pub wire_misses: usize,
    pub warnings: Vec<String>,
}

/// What the round already has when this decoder runs.
pub(crate) struct Context<'a> {
    pub loadouts: &'a [Loadout],
    pub shots: &'a [crate::shots::Shot],
    pub hits: &'a [crate::combat::Hit],
    pub effects: &'a [crate::vitals::Effect],
    pub feedback: &'a [MatchUpdate],
    pub timeline: &'a [crate::combat::TimelineEvent],
}

// ---------------------------------------------------------------- HUD

#[derive(Clone, Copy, Debug)]
struct Link {
    field: Hash,
    child: u32,
    class: Hash,
    index: Option<u32>,
}

/// One written value: its frame and the number, `None` for a size that
/// is no number.
type Written = (Option<u32>, Option<u64>);

/// The objects of the HUD this decoder reads.
#[derive(Debug, Default)]
struct Hud {
    /// `(object, property)` -> every value written, for the three
    /// properties read here.
    values: HashMap<(u32, Hash), Vec<Written>>,
    /// Object -> the last text of its `LOCATION`.
    locations: HashMap<u32, String>,
    links: HashMap<u32, Vec<Link>>,
    class: HashMap<u32, Hash>,
    /// Child -> `(parent, field)`, the last link.
    parent: HashMap<u32, (u32, Hash)>,
    /// Objects in the order they were first linked.
    order: Vec<u32>,
}

fn number(value: &[u8]) -> Option<u64> {
    match *value {
        [v] => Some(u64::from(v)),
        [a, b, c, d] => Some(u64::from(u32::from_le_bytes([a, b, c, d]))),
        _ => <[u8; 8]>::try_from(value).ok().map(u64::from_le_bytes),
    }
}

impl Hud {
    fn read(&mut self, block: &[u8], frame: Option<u32>) {
        let hash_at = |at: usize| -> Option<Hash> { block.get(at..at + 4)?.try_into().ok() };
        // Each record block names its object before writing to it.
        let mut current: Option<u32> = None;
        for_each_record(block, |at, r| match r {
            Record::Set(obj, hash, from, to) => {
                current = Some(obj);
                self.property(obj, hash, block.get(from..to), frame);
            }
            Record::Prop(hash, from, to) => {
                let Some(obj) = current else { return };
                if block.get(at) == Some(&0x22) {
                    self.property(obj, hash, block.get(from..to), frame);
                } else if hash == LOCATION
                    && crate::entities::u32_at(block, at + 5) == Some(0)
                    && let Some(text) = block.get(from..to).filter(|t| !t.is_empty())
                {
                    let text = String::from_utf8_lossy(text).into_owned();
                    self.locations.insert(obj, text);
                }
            }
            Record::ParentChild(parent, field, child) => {
                current = Some(parent);
                if let Some(class) = hash_at(at + 21) {
                    self.link(parent, field, child, class, None);
                }
            }
            Record::Child(field, child) => {
                if let (Some(parent), Some(class)) = (current, hash_at(at + 13)) {
                    self.link(parent, field, child, class, None);
                }
            }
            Record::Element(field, index, child) => {
                if let (Some(parent), Some(class)) = (current, hash_at(at + 17)) {
                    self.link(parent, field, child, class, Some(index));
                }
            }
        });
    }

    fn property(&mut self, obj: u32, hash: Hash, value: Option<&[u8]>, frame: Option<u32>) {
        if ![MATCH_SCORE, STATE, POOL_LEFT].contains(&hash) {
            return;
        }
        let Some(value) = value else { return };
        let values = self.values.entry((obj, hash)).or_default();
        values.push((frame, number(value)));
    }

    fn link(&mut self, parent: u32, field: Hash, child: u32, class: Hash, index: Option<u32>) {
        if child == 0 {
            return;
        }
        let link = Link {
            field,
            child,
            class,
            index,
        };
        self.links.entry(parent).or_default().push(link);
        if self.class.insert(child, class).is_none() {
            self.order.push(child);
        }
        self.parent.insert(child, (parent, field));
    }

    /// The values of a property as they changed.
    fn changes(&self, obj: u32, hash: Hash) -> Vec<Written> {
        let mut out: Vec<Written> = Vec::new();
        for &(frame, value) in self.values.get(&(obj, hash)).into_iter().flatten() {
            if out.last().is_none_or(|l| l.1 != value) {
                out.push((frame, value));
            }
        }
        out
    }

    /// The fields from `obj` up to the controller it hangs under, and
    /// the player of that controller.
    fn path(&self, obj: u32, controllers: &HashMap<u32, usize>) -> (Vec<Hash>, Option<usize>) {
        let mut path = Vec::new();
        let mut at = obj;
        // A tree is a few levels deep; the bound ends a loop of links.
        for _ in 0..32 {
            let Some(&(parent, field)) = self.parent.get(&at) else {
                break;
            };
            path.push(field);
            at = parent;
            if let Some(&player) = controllers.get(&at) {
                return (path, Some(player));
            }
        }
        (path, None)
    }

    fn of_class(&self, class: Hash) -> impl Iterator<Item = u32> + '_ {
        let is = move |o: &&u32| self.class.get(o) == Some(&class);
        self.order.iter().filter(is).copied()
    }
}

// ---------------------------------------------------------------- entities

#[derive(Clone, Copy, Debug, PartialEq)]
enum What {
    Live(u8),
    Flags(u16),
    DestroyedFlag(bool),
    State(Hash),
    OwnerPlayer(u64),
    OwnerAlliance(u32),
    OwnerReleased(u8),
    /// The player who places it, as an index.
    Placer(usize),
    Broken,
    Deleted,
}

/// One thing an entity's messages said that was not so before.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Event {
    frame: Option<u32>,
    what: What,
    /// The last position written up to then.
    position: Option<[f32; 3]>,
}

/// A gadget entity with what its changes say, in stream order.
#[derive(Debug, Default)]
struct Gadget {
    id: u64,
    asset: u64,
    classes: Vec<Hash>,
    type_index: Option<u16>,
    events: Vec<Event>,
    /// Every position written.
    track: Track,
    /// `(player, slot)` of the bodies that carry the asset.
    carriers: Vec<(usize, Slot)>,
    name: Option<String>,
    owner: Option<usize>,
    owner_source: Option<OwnerSource>,
}

/// The state a blob of the state component is in.
fn machine_state(blob: &(Hash, Vec<u8>)) -> Option<(Hash, Hash)> {
    let (hash, bytes) = blob;
    let at = match (*hash, bytes.len()) {
        (MACHINE_A, 16) => 4,
        (MACHINE_B, 20) => 8,
        _ => return None,
    };
    Some((*hash, bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Every position an entity's messages wrote, with the frame.
type Track = Vec<(Option<u32>, [f32; 3])>;

fn events_of(e: &Entity, pids: &HashMap<u64, usize>) -> (Vec<Event>, Track) {
    let mut events = Vec::new();
    let mut track = Vec::new();
    let mut position = None;
    let (mut live, mut flags, mut dead) = (None, None, None);
    let mut machines: Vec<(Hash, Hash)> = Vec::new();
    let (mut player, mut alliance, mut released) = (None, None, None);
    let mut placer = None;
    for c in &e.changes {
        let frame = c.frame;
        if let Some(p) = c.position {
            position = Some(p);
            track.push((frame, p));
        }
        let mut push = |what: What| {
            events.push(Event {
                frame,
                what,
                position,
            })
        };
        if c.live.is_some() && c.live != live {
            live = c.live;
            push(What::Live(c.live.unwrap_or_default()));
        }
        if c.flags.is_some() && c.flags != flags {
            flags = c.flags;
            push(What::Flags(c.flags.unwrap_or_default()));
        }
        if let Some(d) = c.device.and_then(|d| d.destroyed)
            && dead != Some(d)
        {
            dead = Some(d);
            push(What::DestroyedFlag(d));
        }
        if let Some((machine, state)) = (c.state.as_ref())
            .and_then(|s| s.blob.as_ref())
            .and_then(machine_state)
        {
            match machines.iter_mut().find(|m| m.0 == machine) {
                Some(m) if m.1 == state => {}
                Some(m) => {
                    m.1 = state;
                    push(What::State(state));
                }
                None => {
                    machines.push((machine, state));
                    push(What::State(state));
                }
            }
        }
        if let Some(o) = &c.owner {
            if o.player.is_some() && o.player != player {
                player = o.player;
                push(What::OwnerPlayer(o.player.unwrap_or_default()));
            }
            if o.alliance.is_some() && o.alliance != alliance {
                alliance = o.alliance;
                push(What::OwnerAlliance(o.alliance.unwrap_or_default()));
            }
            if o.released.is_some() && o.released != released {
                released = o.released;
                push(What::OwnerReleased(o.released.unwrap_or_default()));
            }
        }
        if placer.is_none()
            && let Some(&p) = (c.placed.and_then(|p| p.owner)).and_then(|id| pids.get(&id))
        {
            placer = Some(p);
            push(What::Placer(p));
        }
        if c.destroyed {
            push(What::Broken);
        }
    }
    if let Some(frame) = e.deleted {
        events.push(Event {
            frame: Some(frame),
            what: What::Deleted,
            position,
        });
    }
    (events, track)
}

// ---------------------------------------------------------------- the round

/// A score change while it is being explained.
#[derive(Clone, Debug, Default, PartialEq)]
struct Score {
    frame: Option<u32>,
    /// Seconds, to the millisecond.
    time: f64,
    player: usize,
    delta: i32,
    total: i32,
    reason: Option<&'static str>,
    detail: Option<String>,
}

/// A removal while it is being explained.
#[derive(Clone, Debug, Default, PartialEq)]
struct Gone {
    /// Index into the gadgets.
    gadget: usize,
    entity: u64,
    asset: u64,
    name: Option<String>,
    /// A thrown thing that has an owner component and nothing else.
    projectile: bool,
    owner: Option<usize>,
    owner_team: Option<usize>,
    frame: u32,
    time: f64,
    position: Option<[f32; 3]>,
    signals: Vec<Signal>,
    /// The state its state machine was last in.
    state: Option<Hash>,
    cause: Cause,
    cause_source: Option<&'static str>,
    by: Option<usize>,
    by_source: Option<&'static str>,
    points: Option<i32>,
    friendly: bool,
    ambiguous: bool,
    means: Option<&'static str>,
    means_source: Option<&'static str>,
    weapon: Option<String>,
    weapon_slot: Option<&'static str>,
    interceptor: Option<u64>,
}

impl Gone {
    fn only(&self, signal: Signal) -> bool {
        self.signals == [signal]
    }

    /// A grenade or projectile that simply ends.
    fn ended_projectile(&self) -> bool {
        self.projectile && self.only(Signal::Deleted)
    }

    fn is(&self, name: &str) -> bool {
        self.name.as_deref() == Some(name)
    }

    /// A name to show: the gadget's, else its asset.
    fn label(&self) -> String {
        (self.name.clone()).unwrap_or_else(|| self.asset.to_string())
    }
}

/// A name to show: the gadget's, else its asset.
fn label(name: Option<&str>, gadget: Option<&Gadget>) -> String {
    match (name, gadget) {
        (Some(n), _) => n.to_owned(),
        (None, Some(g)) => g.asset.to_string(),
        (None, None) => String::new(),
    }
}

/// Pairs score changes with the removals they paid for. `friendly`
/// pairs the penalties for a teammate's gadget (-10 each), else the
/// points of the other team. `teams` is each player's team.
fn pay(scores: &mut [Score], removals: &mut [Gone], teams: &[usize], friendly: bool) {
    const PENALTIES: [i32; 4] = [-10, -20, -30, -40];
    const REWARDS: [i32; 9] = [5, 10, 15, 20, 25, 30, 40, 50, 60];
    for s in scores.iter_mut() {
        if s.reason.is_some() {
            continue;
        }
        let amounts: &[i32] = if friendly { &PENALTIES } else { &REWARDS };
        if !amounts.contains(&s.delta) {
            continue;
        }
        let team = teams.get(s.player).copied();
        let mut candidates: Vec<(f64, usize)> = Vec::new();
        for (i, r) in removals.iter().enumerate() {
            if r.by.is_some() || r.owner_team.is_none() {
                continue;
            }
            let dt = s.time - r.time;
            if !(SCORE_WINDOW.0..=SCORE_WINDOW.1).contains(&dt) {
                continue;
            }
            let same_team = r.owner_team == team;
            if friendly && (!same_team || r.owner == Some(s.player)) {
                continue;
            }
            if !friendly && same_team {
                continue;
            }
            // Only an interception (+20) is credited for a projectile.
            if r.ended_projectile() && (friendly || s.delta != 20) {
                continue;
            }
            candidates.push(((dt - 0.05).abs(), i));
        }
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut left = s.delta.abs();
        let mut paid = Vec::new();
        for &(_, i) in &candidates {
            let Some(r) = removals.get(i) else { continue };
            let worth = if friendly {
                10
            } else if r.ended_projectile() {
                20
            } else {
                points(r.name.as_deref())
            };
            if worth > left {
                continue;
            }
            left -= worth;
            paid.push(i);
            if left <= 0 {
                break;
            }
        }
        if paid.is_empty() {
            continue;
        }
        let mut names = Vec::new();
        for &i in &paid {
            let Some(r) = removals.get_mut(i) else {
                continue;
            };
            r.cause = if r.ended_projectile() {
                Cause::Intercepted
            } else {
                Cause::Destroyed
            };
            r.cause_source = Some("score");
            r.by = Some(s.player);
            r.by_source = Some("score");
            r.points = Some(s.delta);
            r.friendly = friendly;
            r.ambiguous = candidates.len() > paid.len();
            names.push(r.label());
        }
        s.reason = Some(if friendly {
            "friendlyGadgetDestroyed"
        } else {
            "gadgetDestroyed"
        });
        s.detail = Some(names.join(", "));
    }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
}

fn millis(t: f64) -> f64 {
    (t * 1000.0).round() / 1000.0
}

/// A trap trigger while its victims and points are being found.
struct Trap {
    name: String,
    gadget: Option<usize>,
    owner: Option<usize>,
    frame: Option<u32>,
    time: f64,
    marker: String,
    detonated_at: Option<f64>,
    location: Option<String>,
    position: Option<[f32; 3]>,
    victims: Vec<Victim>,
    nearest: Option<(f32, usize)>,
    points: Option<i32>,
}

/// One of Fenrir's mines as the HUD lists it.
struct Mine {
    player: Option<usize>,
    location: Option<String>,
    /// `(frame, seconds, State)`.
    states: Vec<(Option<u32>, f64, Option<u64>)>,
    gadget: Option<usize>,
}

struct Decoder<'a> {
    input: &'a Input<'a>,
    world: &'a World,
    ctx: &'a Context<'a>,
    /// Each player's team.
    teams: Vec<usize>,
    alliances: HashMap<u32, usize>,
    end: f64,
    /// Frames the recording has no time for.
    untimed: Cell<usize>,
}

impl Decoder<'_> {
    /// Seconds since the recording started for a frame; the snapshot is
    /// the start.
    fn time(&self, frame: Option<u32>) -> f64 {
        world::seconds(self.input, frame).unwrap_or_else(|| {
            self.untimed.set(self.untimed.get() + 1);
            self.end
        })
    }

    fn when(&self, frame: Option<u32>) -> When {
        self.world.when(self.input.clock, frame)
    }

    fn username(&self, player: Option<usize>) -> Option<String> {
        Some(self.input.players.get(player?)?.username.clone())
    }

    fn player(&self, username: &str) -> Option<usize> {
        let players = self.input.players;
        players.iter().position(|p| p.username == username)
    }

    fn team(&self, player: Option<usize>) -> Option<usize> {
        self.teams.get(player?).copied()
    }

    fn team_of(&self, username: &str) -> Option<usize> {
        self.team(self.player(username))
    }

    /// The team holding a gadget in `frame`: by the last alliance its
    /// owner component stated, else its owner's.
    fn owner_team(&self, g: &Gadget, frame: Option<u32>) -> Option<usize> {
        let mut alliance = None;
        for e in &g.events {
            if let What::OwnerAlliance(a) = e.what
                && (e.frame.is_none() || frame.is_none() || e.frame <= frame)
            {
                alliance = Some(a);
            }
        }
        let held = alliance.and_then(|a| self.alliances.get(&a).copied());
        held.or_else(|| self.team(g.owner))
    }

    fn subject(&self, g: &Gadget) -> Subject {
        Subject {
            entity: g.id,
            asset: g.asset,
            type_index: g.type_index,
            name: g.name.clone(),
            username: self.username(g.owner),
            username_source: g.owner_source,
        }
    }

    /// Where a body was at `time`.
    fn body_at(&self, player: usize, time: f64) -> Option<[f32; 3]> {
        let entities = self.input.players.get(player)?.entities.as_ref()?;
        let track = self.world.bodies.get(&u64::from(entities.movement?))?;
        let before = |p: &(u32, [f32; 3])| {
            world::seconds(self.input, Some(p.0)).is_some_and(|t| t <= time + 0.0005)
        };
        let i = track.partition_point(before).checked_sub(1)?;
        track.get(i).map(|p| p.1)
    }

    /// The hits and effects of the given types within `window` of `time`
    /// on players not of `team`.
    fn victims(
        &self,
        time: f64,
        window: (f64, f64),
        hit_types: &[u32],
        effect_types: &[u32],
        team: Option<usize>,
    ) -> Vec<Victim> {
        let within =
            |t: Option<f64>| t.is_some_and(|t| window.0 <= t - time && t - time <= window.1);
        let other = |username: &str| team.is_none() || self.team_of(username) != team;
        let mut out: Vec<Victim> = Vec::new();
        for h in self.ctx.hits {
            if within(h.recording_time) && hit_types.contains(&h.kind.id) && other(&h.username) {
                out.push(Victim {
                    username: h.username.clone(),
                    damage: h.damage,
                    result: Some(h.result),
                    damage_type: Some(h.kind.id),
                    effect: None,
                    seconds: None,
                    source: "hit",
                    victim_source: "time",
                });
            }
        }
        for e in self.ctx.effects {
            if !(within(e.start.recording_time) && effect_types.contains(&e.kind))
                || !other(&e.username)
            {
                continue;
            }
            let mut known = false;
            for v in out.iter_mut().filter(|v| v.username == e.username) {
                v.effect = Some(e.kind);
                known = true;
            }
            if !known {
                out.push(Victim {
                    username: e.username.clone(),
                    damage: None,
                    result: None,
                    damage_type: None,
                    effect: Some(e.kind),
                    seconds: Some(e.seconds),
                    source: "effect",
                    victim_source: "time",
                });
            }
        }
        out
    }
}

/// Where an entity was in `frame` (`None`: at the end), unless it was in
/// the pool.
fn placed_at(g: &Gadget, frame: Option<u32>) -> Option<[f32; 3]> {
    let upto = |p: &&(Option<u32>, [f32; 3])| frame.is_none() || p.0 <= frame;
    let last = g.track.iter().take_while(upto).last()?;
    (last.1[2] > -90.0).then_some(last.1)
}

/// Reads the score changes, gadget removals, statuses and trap triggers
/// of a round. `ctx` is what the round has by then: loadouts, shots,
/// hits, effects, kill feed and timeline.
pub(crate) fn decode(input: &Input, world: &World, fx: &Effects, ctx: &Context) -> GadgetEvents {
    let players = input.players;
    let d = Decoder {
        input,
        world,
        ctx,
        teams: players.iter().map(|p| p.team_index).collect(),
        alliances: (players.iter())
            .filter_map(|p| Some((u32::try_from(p.alliance).ok()?, p.team_index)))
            .collect(),
        end: input.clock.frame_times.last().copied().unwrap_or(0.0),
        untimed: Cell::new(0),
    };
    let pids: HashMap<u64, usize> = players.iter().enumerate().map(|(i, p)| (p.id, i)).collect();
    let controllers: HashMap<u32, usize> = (players.iter().enumerate())
        .filter_map(|(i, p)| Some((p.entities.as_ref()?.controller, i)))
        .collect();

    // ---- names: the loadout slot of the body that carries the asset
    let mut items: HashMap<(usize, Slot), &'static str> = HashMap::new();
    for l in ctx.loadouts {
        let Some(p) = d.player(&l.username) else {
            continue;
        };
        for (slot, counted) in [(Slot::Ability, &l.ability), (Slot::Gadget, &l.gadget)] {
            if let Some(name) = counted.as_ref().and_then(|c| c.name) {
                items.insert((p, slot), name);
            }
        }
    }
    let mut carried: HashMap<u64, Vec<(usize, Slot)>> = HashMap::new();
    let mut bodies: HashMap<u64, usize> = HashMap::new();
    for (i, p) in players.iter().enumerate() {
        let Some(body) = p.entities.as_ref().and_then(|e| e.movement) else {
            continue;
        };
        bodies.insert(u64::from(body), i);
        let Some(e) = world.get(u64::from(body)) else {
            continue;
        };
        for &(hash, item) in &e.slots {
            if let Some(&(_, slot)) = BODY_SLOTS.iter().find(|s| s.0 == hash) {
                carried.entry(item).or_default().push((i, slot));
            }
        }
    }

    // ---- the gadget entities
    let mut gadgets: Vec<Gadget> = Vec::new();
    for e in world.iter() {
        if e.is_map()
            || e.classes.is_empty()
            || e.has(world::CARRIED)
            || bodies.contains_key(&e.id)
            || world.players.contains_key(&e.id)
        {
            continue;
        }
        let (events, track) = events_of(e, &pids);
        let type_index = e.changes.iter().find_map(|c| {
            let placed = c.placed.and_then(|p| p.type_index).map(|t| t.0);
            placed.or(c.owner.and_then(|o| o.type_index))
        });
        let carriers = carried.get(&e.asset).cloned().unwrap_or_default();
        let name = if carriers.is_empty() {
            let known = ASSETS.iter().find(|a| a.0 == e.asset);
            known.map(|a| a.1.to_owned())
        } else {
            let mut names: Vec<&str> = (carriers.iter())
                .filter_map(|&(p, slot)| match slot {
                    Slot::Drone => Some("Drone"),
                    _ => items.get(&(p, slot)).copied(),
                })
                .collect();
            names.sort_unstable();
            names.dedup();
            (!names.is_empty()).then(|| names.join("/"))
        };
        // The owner: the last player the owner component names, else
        // who placed it.
        let mut owner = None;
        for ev in &events {
            match ev.what {
                What::OwnerPlayer(id) => owner = pids.get(&id).copied().or(owner),
                What::Placer(p) if owner.is_none() => owner = Some(p),
                _ => {}
            }
        }
        let mut owner_source = owner.map(|_| OwnerSource::Read);
        if owner.is_none() {
            let mut users: Vec<usize> = carriers.iter().map(|c| c.0).collect();
            users.sort_unstable();
            users.dedup();
            if let [only] = *users {
                (owner, owner_source) = (Some(only), Some(OwnerSource::Carrier));
            } else if let Some(a) = ASSETS.iter().find(|a| a.0 == e.asset && !a.2.is_empty()) {
                let of_operator = |p: &&crate::header::Player| p.operator.name() == Some(a.2);
                let mut of = players.iter().enumerate().filter(|(_, p)| of_operator(p));
                if let (Some((only, _)), None) = (of.next(), of.next()) {
                    (owner, owner_source) = (Some(only), Some(OwnerSource::Operator));
                }
            }
        }
        gadgets.push(Gadget {
            id: e.id,
            asset: e.asset,
            classes: e.classes.clone(),
            type_index,
            events,
            track,
            carriers,
            name,
            owner,
            owner_source,
        });
    }
    let by_id: HashMap<u64, usize> = gadgets.iter().enumerate().map(|(i, g)| (g.id, i)).collect();

    // ---- the HUD
    let mut hud = Hud::default();
    for (start, end, frame) in input.blocks(STATE_STREAM) {
        if let Some(block) = input.data.get(start..end) {
            hud.read(block, frame);
        }
    }

    // ---- 1. score changes (read)
    let mut scores: Vec<Score> = Vec::new();
    for (i, p) in players.iter().enumerate() {
        let Some(board) = p.entities.as_ref().and_then(|e| e.scoreboard) else {
            continue;
        };
        let mut before: Option<i32> = None;
        for &(frame, value) in hud.values.get(&(board, MATCH_SCORE)).into_iter().flatten() {
            // An `i32` in four bytes: a team killer goes below zero.
            let Some(total) = value.and_then(|v| u32::try_from(v).ok()) else {
                continue;
            };
            let total = total as i32;
            if let Some(b) = before.filter(|&b| b != total) {
                scores.push(Score {
                    frame,
                    time: millis(d.time(frame)),
                    player: i,
                    delta: total - b,
                    total,
                    ..Score::default()
                });
            }
            before = Some(total);
        }
    }
    let name_of = |s: &Score| players.get(s.player).map(|p| p.username.as_str());
    scores.sort_by(|a, b| (a.time.total_cmp(&b.time)).then_with(|| name_of(a).cmp(&name_of(b))));

    // ---- 2. removals of deployed gadgets (read)
    let mut removals: Vec<Gone> = Vec::new();
    for (gi, g) in gadgets.iter().enumerate() {
        let unnamed = g.name.is_none();
        if unnamed && (g.classes == [world::STATE] || g.classes == [PROP_CLASS]) {
            continue; // weapon parts, map props
        }
        if g.carriers.is_empty() && g.classes == [world::PLACED, world::DAMAGE] {
            continue; // barricades and reinforcements
        }
        if NOT_GADGETS.contains(&g.asset) {
            continue;
        }
        let mut live = false;
        // Index into `removals` of the removal signals still join.
        let mut current: Option<usize> = None;
        let mut state = None;
        for ev in &g.events {
            if let What::State(s) = ev.what {
                state = Some(s);
            }
            let Some(frame) = ev.frame else {
                if let What::Live(l) = ev.what {
                    live = l != 0;
                }
                continue;
            };
            let time = d.time(ev.frame);
            let open = current
                .and_then(|i| removals.get(i).map(|r| (i, r)))
                .filter(|(_, r)| time - d.time(Some(r.frame)) <= SIGNAL_WINDOW)
                .map(|(i, _)| i);
            let signal = match ev.what {
                What::Live(1) => {
                    live = true;
                    current = None;
                    None
                }
                What::Live(_) if live => {
                    live = false;
                    Some(Signal::NotLive)
                }
                What::DestroyedFlag(true) => Some(Signal::DestroyedFlag),
                What::Deleted => {
                    let counts = live || open.is_some();
                    live = false;
                    counts.then_some(Signal::Deleted)
                }
                What::Flags(INERT) if live && g.classes == [world::PLACED] => Some(Signal::Inert),
                What::Broken if live => Some(Signal::Broken),
                // Only ever joins a removal, never starts one.
                What::OwnerReleased(0) if open.is_some() => Some(Signal::Returned),
                _ => None,
            };
            let Some(signal) = signal else { continue };
            if let Some(r) = open.and_then(|i| removals.get_mut(i)) {
                if !r.signals.contains(&signal) {
                    r.signals.push(signal);
                }
                continue;
            }
            current = Some(removals.len());
            removals.push(Gone {
                gadget: gi,
                entity: g.id,
                asset: g.asset,
                name: g.name.clone(),
                projectile: g.classes == [world::OWNER],
                owner: g.owner,
                owner_team: d.owner_team(g, ev.frame),
                frame,
                time: millis(time),
                position: ev.position,
                signals: vec![signal],
                state,
                ..Gone::default()
            });
        }
    }
    removals.sort_by(|a, b| (a.time.total_cmp(&b.time)).then(a.entity.cmp(&b.entity)));

    // ---- 3. statuses: an effect spawned on the gadget (read)
    struct Mark {
        gadget: usize,
        kind: StatusKind,
        fx_asset: Option<u64>,
        frame: u32,
        time: f64,
        until: Option<u32>,
        owner_team: Option<usize>,
        team: Option<usize>,
        by: Option<usize>,
        target: Option<u64>,
    }
    let mut statuses: Vec<Mark> = Vec::new();
    for s in &fx.spawns {
        let Some(&(_, kind)) = STATUS_FX.iter().find(|k| k.0 == s.asset) else {
            continue;
        };
        let (Some(frame), Some(&gi)) = (s.frame, by_id.get(&s.parent)) else {
            continue;
        };
        let Some(g) = gadgets.get(gi) else { continue };
        statuses.push(Mark {
            gadget: gi,
            kind,
            fx_asset: Some(s.asset),
            frame,
            time: millis(d.time(s.frame)),
            until: s.stopped.filter(|&end| end > frame),
            owner_team: d.owner_team(g, s.frame),
            team: None,
            by: None,
            target: None,
        });
    }
    let entity_of = |gi: usize| gadgets.get(gi).map_or(0, |g| g.id);
    statuses.sort_by(|a, b| {
        (a.time.total_cmp(&b.time))
            .then(entity_of(a.gadget).cmp(&entity_of(b.gadget)))
            .then(a.kind.cmp(&b.kind))
    });

    // ---- 4. captures: the owner component's alliance rewritten (read)
    let mut captures: Vec<Mark> = Vec::new();
    for (gi, g) in gadgets.iter().enumerate() {
        let mut held = None;
        for ev in &g.events {
            let What::OwnerAlliance(a) = ev.what else {
                continue;
            };
            if let (Some(_), Some(frame)) = (held, ev.frame) {
                captures.push(Mark {
                    gadget: gi,
                    kind: StatusKind::Captured,
                    fx_asset: None,
                    frame,
                    time: millis(d.time(ev.frame)),
                    until: None,
                    owner_team: d.team(g.owner),
                    team: d.alliances.get(&a).copied(),
                    by: None,
                    target: None,
                });
            }
            held = Some(a);
        }
    }
    captures.sort_by(|a, b| a.time.total_cmp(&b.time));

    // ---- 5. the HUD: gadgets taken back, Fenrir's mines, reinforcements
    // `(player, name, seconds)` of a gadget state going back to 0: in
    // the inventory again.
    let mut taken_back: Vec<(usize, &str, f64)> = Vec::new();
    for view in hud.of_class(GADGET_VIEW) {
        let (path, Some(player)) = hud.path(view, &controllers) else {
            continue;
        };
        if path.last() != Some(&LOADOUT_FIELD) {
            continue;
        }
        let slot = match path.first() {
            Some(&GADGET_FIELD) => Slot::Gadget,
            Some(&ABILITY_FIELD) => Slot::Ability,
            _ => continue,
        };
        let Some(&name) = items.get(&(player, slot)) else {
            continue;
        };
        let mut items_of: Vec<(u32, u32)> = Vec::new();
        for l in hud.links.get(&view).into_iter().flatten() {
            let Some(index) = l.index else { continue };
            if l.field != GADGET_STATES || l.class != GADGET_STATE_ITEM {
                continue;
            }
            match items_of.iter_mut().find(|i| i.0 == index) {
                Some(i) => i.1 = l.child,
                None => items_of.push((index, l.child)),
            }
        }
        for (_, item) in items_of {
            for w in hud.changes(item, STATE).windows(2) {
                if let [(_, Some(1 | 2)), (frame, Some(0))] = *w {
                    taken_back.push((player, name, millis(d.time(frame))));
                }
            }
        }
    }
    let mut mines: Vec<(u32, Mine)> = Vec::new();
    for obj in hud.of_class(FENRIR_MINE) {
        let states = hud.changes(obj, STATE);
        let states = (states.into_iter())
            .map(|(frame, state)| (frame, millis(d.time(frame)), state))
            .collect();
        let mine = Mine {
            player: hud.path(obj, &controllers).1,
            location: hud.locations.get(&obj).cloned(),
            states,
            gadget: None,
        };
        mines.push((obj, mine));
    }
    mines.sort_by_key(|m| m.0);
    // A mine's HUD state becomes 2 when it lands, up to 3.5 s after its
    // entity went live (thrown).
    let mut thrown: Vec<(f64, usize, bool)> = Vec::new();
    for (gi, g) in gadgets.iter().enumerate() {
        if g.name.as_deref() != Some(FENRIR) {
            continue;
        }
        let first = (g.events.iter()).find(|e| e.what == What::Live(1) && e.frame.is_some());
        if let Some(ev) = first {
            thrown.push((d.time(ev.frame), gi, false));
        }
    }
    thrown.sort_by(|a, b| a.0.total_cmp(&b.0));
    let landed = |m: &Mine| m.states.iter().find(|s| s.2 == Some(2)).map(|s| s.1);
    let mut by_landing: Vec<usize> = (0..mines.len()).collect();
    by_landing.sort_by(|&a, &b| {
        let at = |i: usize| mines.get(i).and_then(|m| landed(&m.1)).unwrap_or(f64::MAX);
        at(a).total_cmp(&at(b))
    });
    for i in by_landing {
        let Some((_, mine)) = mines.get_mut(i) else {
            continue;
        };
        let Some(at) = landed(mine) else { continue };
        for t in thrown.iter_mut() {
            let owner = gadgets.get(t.1).and_then(|g| g.owner);
            if !t.2 && owner == mine.player && (-0.1..=3.5).contains(&(at - t.0)) {
                mine.gadget = Some(t.1);
                t.2 = true;
                break;
            }
        }
    }

    // ---- 6. who destroyed, disabled or captured: from the scoreboard
    let teams = d.teams.clone();
    let team = |player: usize| teams.get(player).copied();
    let mut used: Vec<usize> = Vec::new();
    for c in captures.iter_mut() {
        let found = scores.iter().enumerate().position(|(i, s)| {
            (-0.02..=0.15).contains(&(s.time - c.time))
                && !used.contains(&i)
                && team(s.player) == c.team
                && [15, 25].contains(&s.delta)
        });
        if let Some(s) = found.and_then(|i| scores.get_mut(i).map(|s| (i, s))) {
            c.by = Some(s.1.player);
            s.1.reason = Some("capture");
            let g = gadgets.get(c.gadget);
            s.1.detail = Some(label(g.and_then(|g| g.name.as_deref()), g));
            used.push(s.0);
        }
    }
    for st in statuses.iter_mut() {
        let amounts: &[i32] = match st.kind {
            StatusKind::Hacking | StatusKind::Captured => continue,
            StatusKind::EmpDisabled => &[5, 10, 15, 20, 30, 40, 50],
            StatusKind::Frozen => &[10, 20, 30, 40],
            StatusKind::Hacked => &[25],
            StatusKind::Caught => &[20],
            StatusKind::AdsFired => &[20, 40],
        };
        let owner = gadgets.get(st.gadget).and_then(|g| g.owner);
        let found = scores.iter().position(|s| {
            (-0.02..=0.13).contains(&(s.time - st.time))
                && amounts.contains(&s.delta)
                && match st.kind {
                    // The system's own team scores: its owner.
                    StatusKind::AdsFired => Some(s.player) == owner,
                    _ => team(s.player) != st.owner_team,
                }
        });
        if let Some(s) = found.and_then(|i| scores.get_mut(i)) {
            st.by = Some(s.player);
            if s.reason.is_none() {
                s.reason = Some(match st.kind {
                    StatusKind::EmpDisabled => "empDisabled",
                    StatusKind::Frozen => "frozen",
                    StatusKind::Hacked => "hacked",
                    StatusKind::Caught => "caught",
                    _ => "adsFired",
                });
                s.detail = Some(label(
                    gadgets.get(st.gadget).and_then(|g| g.name.as_deref()),
                    gadgets.get(st.gadget),
                ));
            }
        }
    }
    pay(&mut scores, &mut removals, &teams, true);
    pay(&mut scores, &mut removals, &teams, false);
    // What an Active Defense System shot down: the enemy projectile
    // deleted as it fires.
    for st in statuses.iter_mut() {
        if st.kind != StatusKind::AdsFired {
            continue;
        }
        let near = |r: &&mut Gone| {
            r.ended_projectile()
                && r.by.is_none()
                && r.owner_team.is_some()
                && r.owner_team != st.owner_team
                && (r.time - st.time).abs() <= 0.2
        };
        let closest = (removals.iter_mut().filter(near)).min_by(|a, b| {
            (a.time - st.time)
                .abs()
                .total_cmp(&(b.time - st.time).abs())
        });
        if let Some(r) = closest {
            r.cause = Cause::Intercepted;
            r.cause_source = Some("adsFired");
            r.by = gadgets.get(st.gadget).and_then(|g| g.owner);
            r.by_source = Some("adsFired");
            r.points = st.by.map(|_| 20);
            r.friendly = false;
            r.interceptor = Some(entity_of(st.gadget));
            st.target = Some(r.entity);
        }
    }
    // Causes that need no score.
    for r in removals.iter_mut() {
        let has = |s: Signal| r.signals.contains(&s);
        let only_deleted = r.only(Signal::Deleted);
        if r.cause != Cause::Unknown {
            if r.is(WELCOME_MAT) && has(Signal::Inert) {
                r.cause = Cause::Triggered;
                r.cause_source = Some("signals");
            }
            continue;
        }
        let name = r.name.as_deref().unwrap_or_default();
        let wire_cut = name == BARBED_WIRE && has(Signal::Inert);
        let went_off = [EDD, "Grzmot Mine", CLAYMORE].contains(&name) && only_deleted;
        let mat = name == WELCOME_MAT && has(Signal::Inert);
        // The device's flag, the `fe` entry and the flags of a wire or a
        // mat that stays where it is are read; what any other pattern of
        // signals means is known from the type.
        let destroyed = has(Signal::DestroyedFlag) || has(Signal::Broken) || wire_cut;
        let read = destroyed || mat;
        r.cause_source = Some(if read { "signals" } else { "type" });
        r.cause = if destroyed {
            Cause::Destroyed
        } else if mat {
            Cause::Triggered
        } else if name == RAZORBLOOM && matches!(r.state, Some(OPENING | OPENED)) {
            Cause::Detonated
        } else if went_off || (name == GU && !has(Signal::Returned)) {
            Cause::Triggered
        } else if name == "Mag-NET System" && only_deleted {
            Cause::Used
        } else if only_deleted && DETONATE.contains(&name) {
            Cause::Detonated
        } else {
            r.cause_source = None;
            Cause::Unknown
        };
    }
    // Picked up again: the HUD state of one of the owner's gadgets of
    // that name goes back to 0.
    for r in removals.iter_mut().filter(|r| r.cause == Cause::Unknown) {
        let back = |&(player, name, at): &(usize, &str, f64)| {
            Some(player) == r.owner && r.is(name) && (-0.5..=2.0).contains(&(at - r.time))
        };
        if taken_back.iter().any(back) {
            r.cause = Cause::PickedUp;
            r.cause_source = Some("hudGadgetState");
        }
    }
    for r in removals.iter_mut() {
        r.signals.sort_unstable();
    }
    // The means of a destruction: a shot of the scorer that passes the
    // gadget, else an explosive of theirs that just ended.
    for i in 0..removals.len() {
        let Some(r) = removals.get(i) else { continue };
        let (Cause::Destroyed, Some(by), Some(at)) = (r.cause, r.by, r.position) else {
            continue;
        };
        let by_name = d.username(Some(by));
        let mut best: Option<(f32, &crate::shots::Shot)> = None;
        for s in ctx.shots {
            let Some(t) = s.when.recording_time else {
                continue;
            };
            if t < r.time - 0.45 || t > r.time + 0.12 || s.username != by_name {
                continue;
            }
            let to: [f32; 3] = std::array::from_fn(|k| at[k] - s.origin[k]);
            let along: f32 = (0..3).map(|k| to[k] * s.direction[k]).sum();
            let foot: [f32; 3] = std::array::from_fn(|k| s.direction[k] * along);
            let off = distance(to, foot);
            if (0.0..=s.distance + 1.5).contains(&along)
                && off <= 0.6
                && best.is_none_or(|b| off < b.0)
            {
                best = Some((off, s));
            }
        }
        let explosive = || {
            removals.iter().enumerate().find(|&(j, q)| {
                j != i
                    && q.owner == Some(by)
                    && q.signals.contains(&Signal::Deleted)
                    && (r.time - q.time).abs() <= 0.35
                    && q.position.is_some_and(|p| distance(p, at) <= 8.0)
            })
        };
        let found = if let Some((_, s)) = best {
            let weapon = s.weapon.as_ref().and_then(|w| w.name).map(str::to_owned);
            Some(("bullet", "shotRay", weapon, s.slot))
        } else {
            explosive().map(|(_, q)| ("explosion", "proximity", Some(q.label()), None))
        };
        if let (Some((means, source, weapon, slot)), Some(r)) = (found, removals.get_mut(i)) {
            r.means = Some(means);
            r.means_source = Some(source);
            r.weapon = weapon;
            r.weapon_slot = slot;
        }
    }

    // ---- 7. traps set off
    let mut traps: Vec<Trap> = Vec::new();
    for (gi, g) in gadgets.iter().enumerate() {
        if g.name.as_deref() != Some(RAZORBLOOM) {
            continue;
        }
        let mut before = None;
        for (i, ev) in g.events.iter().enumerate() {
            let What::State(state) = ev.what else {
                continue;
            };
            if before == Some(CLOSED) && state == OPENING && ev.frame.is_some() {
                let time = d.time(ev.frame);
                let burst = (g.events.iter().skip(i))
                    .find(|w| w.what == What::Live(0) && w.frame > ev.frame)
                    .map(|w| d.time(w.frame))
                    .filter(|b| b - time < 2.0)
                    .map(millis);
                let victims = burst.map_or(Vec::new(), |b| {
                    d.victims(b, (-0.1, 0.25), &[2], &[39], d.team(g.owner))
                });
                traps.push(Trap {
                    name: RAZORBLOOM.to_owned(),
                    gadget: Some(gi),
                    owner: g.owner,
                    frame: ev.frame,
                    time: millis(time),
                    marker: "state Closed->Opening".to_owned(),
                    detonated_at: burst,
                    location: None,
                    position: ev.position,
                    victims,
                    nearest: None,
                    points: None,
                });
            }
            before = Some(state);
        }
    }
    for r in removals.iter_mut() {
        let (t, team) = (r.time, r.owner_team);
        let name = r.name.clone().unwrap_or_default();
        let victims = match name.as_str() {
            WELCOME_MAT => d.victims(t, (-0.15, 0.15), &[16], &[], team),
            EDD => d.victims(t, (-0.05, 0.45), &[2], &[], team),
            GU => d.victims(t, (-0.3, 0.3), &[22], &[1], team),
            CLAYMORE => d.victims(t, (-0.05, 0.3), &[2], &[], team),
            _ => Vec::new(),
        };
        if matches!(r.cause, Cause::Unknown | Cause::Triggered)
            && d.end - t < 1.0
            && !r.signals.contains(&Signal::DestroyedFlag)
        {
            // The round being torn down, unless somebody was hurt by it.
            let hurt = match name.as_str() {
                GU => !victims.is_empty(),
                EDD | CLAYMORE => !d.victims(t, (-0.05, 0.45), &[2], &[], team).is_empty(),
                _ => false,
            };
            if !hurt {
                r.cause = Cause::RoundEnd;
                r.cause_source = Some("time");
            }
        }
        if r.cause != Cause::Triggered {
            continue;
        }
        let signals: Vec<&str> = (r.signals.iter())
            .map(|s| match s {
                Signal::Broken => "broken",
                Signal::Deleted => "deleted",
                Signal::DestroyedFlag => "destroyedFlag",
                Signal::Inert => "inert",
                Signal::NotLive => "notLive",
                Signal::Returned => "returned",
            })
            .collect();
        traps.push(Trap {
            name,
            gadget: Some(r.gadget),
            owner: r.owner,
            frame: Some(r.frame),
            time: t,
            marker: signals.join("+"),
            detonated_at: None,
            location: None,
            position: r.position,
            victims,
            nearest: None,
            points: None,
        });
    }
    for s in &fx.spawns {
        let Some(&(_, name, effect)) = TRAP_FX.iter().find(|k| k.0 == s.asset) else {
            continue;
        };
        let (Some(_), Some(&gi)) = (s.frame, by_id.get(&s.parent)) else {
            continue;
        };
        let Some(g) = gadgets.get(gi) else { continue };
        let time = d.time(s.frame);
        let team = d.owner_team(g, s.frame);
        traps.push(Trap {
            name: name.to_owned(),
            gadget: Some(gi),
            owner: g.owner,
            frame: s.frame,
            time: millis(time),
            marker: format!("FX {}", s.asset),
            detonated_at: None,
            location: None,
            position: placed_at(g, s.frame),
            victims: d.victims(time, (-0.1, 0.1), &[], &[effect], team),
            nearest: None,
            points: None,
        });
    }
    for (_, mine) in &mines {
        let mut before = None;
        for &(frame, time, state) in &mine.states {
            if state == Some(3) && before != Some(3) {
                let g = mine.gadget.and_then(|gi| gadgets.get(gi));
                traps.push(Trap {
                    name: FENRIR.to_owned(),
                    gadget: mine.gadget,
                    owner: mine.player,
                    frame,
                    time,
                    marker: "HUD mine State 3".to_owned(),
                    detonated_at: None,
                    location: mine.location.clone(),
                    position: g.and_then(|g| placed_at(g, frame)),
                    victims: d.victims(time, (-0.15, 0.3), &[], &[26], d.team(mine.player)),
                    nearest: None,
                    points: None,
                });
            }
            before = state;
        }
    }
    traps.sort_by(|a, b| (a.time.total_cmp(&b.time)).then_with(|| a.name.cmp(&b.name)));
    for t in traps.iter_mut() {
        if let Some(at) = t.position {
            let owner_team = d.team(t.owner);
            t.nearest = (0..players.len())
                .filter(|&p| d.team(Some(p)) != owner_team)
                .filter_map(|p| Some((distance(d.body_at(p, t.time)?, at), p)))
                .min_by(|a, b| a.0.total_cmp(&b.0));
        }
        // The owner's score for it.
        let paid = scores.iter_mut().find(|s| {
            Some(s.player) == t.owner
                && (-0.05..=0.15).contains(&(s.time - t.time))
                && s.reason.is_none()
                && (1..=60).contains(&s.delta)
        });
        if let Some(s) = paid {
            t.points = Some(s.delta);
            s.reason = Some("trapTriggered");
            s.detail = Some(t.name.clone());
        }
    }
    // Fenrir arming a mine (its state goes to `Opening`) is worth 5.
    for g in gadgets.iter().filter(|g| g.name.as_deref() == Some(FENRIR)) {
        for ev in &g.events {
            if ev.what != What::State(OPENING) || ev.frame.is_none() {
                continue;
            }
            let at = d.time(ev.frame);
            let paid = scores.iter_mut().find(|s| {
                Some(s.player) == g.owner
                    && (-0.05..=0.12).contains(&(s.time - at))
                    && s.reason.is_none()
                    && s.delta == 5
            });
            if let Some(s) = paid {
                s.reason = Some("trapArmed");
                s.detail = Some(FENRIR.to_owned());
            }
        }
    }

    // ---- 8. the rest of the score changes
    let mut reinforced: Vec<f64> = Vec::new();
    for obj in hud.of_class(REINFORCEMENT_POOL) {
        for w in hud.changes(obj, POOL_LEFT).windows(2) {
            if let [(_, Some(a)), (Some(frame), Some(b))] = *w
                && b + 1 == a
            {
                reinforced.push(d.time(Some(frame)));
            }
        }
    }
    let mut deployed: Vec<(f64, Option<usize>, usize)> = Vec::new();
    for (gi, g) in gadgets.iter().enumerate() {
        for ev in &g.events {
            if ev.what == What::Live(1) && ev.frame.is_some() {
                deployed.push((d.time(ev.frame), g.owner, gi));
            }
        }
    }
    let kills: Vec<(f64, Option<usize>, &str)> = (ctx.feedback.iter())
        .filter(|k| k.kind == MatchUpdateType::Kill)
        .filter_map(|k| Some((k.recording_time?, d.player(&k.username), k.target.as_str())))
        .collect();
    let same: Vec<usize> = (scores.iter())
        .map(|s| {
            let alike = |x: &&Score| {
                x.frame == s.frame && x.delta == s.delta && team(x.player) == team(s.player)
            };
            scores.iter().filter(alike).count()
        })
        .collect();
    for (s, same) in scores.iter_mut().zip(same) {
        if s.reason.is_some() {
            continue;
        }
        let (t, u, dl) = (s.time, Some(s.player), s.delta);
        let kill = (kills.iter()).find(|k| k.1 == u && (-0.1..=0.4).contains(&(t - k.0)));
        let ended =
            (removals.iter()).find(|r| r.owner == u && (-0.12..=0.24).contains(&(t - r.time)));
        let placed = (deployed.iter()).find(|p| p.1 == u && (-0.35..=0.6).contains(&(t - p.0)));
        let revived = || {
            ctx.timeline.iter().any(|e| {
                e.kind == TimelineKind::Revive
                    && e.by.as_deref().and_then(|b| d.player(b)) == u
                    && e.recording_time.is_some_and(|r| (t - r).abs() <= 0.5)
            })
        };
        let (reason, detail) = if let (true, Some(k)) = (dl.abs() >= 100, kill) {
            let reason = if dl > 0 { "kill" } else { "teamKill" };
            (Some(reason), Some(k.2.to_owned()))
        } else if same >= 4 {
            (Some("team"), None)
        } else if [75, 85].contains(&dl)
            && kills
                .iter()
                .any(|k| (-0.1..=0.45).contains(&(t - k.0)) && d.team_of(k.2) != team(s.player))
        {
            (Some("assist"), None)
        } else if dl == 10 && reinforced.iter().any(|r| (t - r - 5.07).abs() <= 0.15) {
            (Some("reinforcement"), None)
        } else if let (true, Some(r)) = (dl > 0, ended) {
            (Some("ownGadgetEnded"), Some(r.label()))
        } else if let (true, Some(p)) = (dl > 0, placed) {
            let g = gadgets.get(p.2);
            let name = g.and_then(|g| g.name.as_deref());
            (Some("deployed"), Some(label(name, g)))
        } else if let (true, Some(r)) = (dl < 0, ended) {
            (Some("ownGadgetPenalty"), Some(r.label()))
        } else if dl == 50 && revived() {
            (Some("revive"), None)
        } else if d.end - t < 1.5 {
            (Some("roundEnd"), None)
        } else {
            (None, None)
        };
        s.reason = reason;
        s.detail = detail;
    }

    // ---- 9. jammed players and the jammer they are in (nearest)
    let mut jams = Vec::new();
    let frame_times = input.clock.frame_times;
    for ef in ctx.effects.iter().filter(|e| e.kind == 8) {
        let (Some(t), Some(u)) = (ef.start.recording_time, d.player(&ef.username)) else {
            continue;
        };
        let frame = frame_times.partition_point(|&f| f <= t + 0.0005);
        let frame = u32::try_from(frame.saturating_sub(1)).ok();
        let mut at: Vec<(Option<u64>, [f32; 3])> = Vec::new();
        at.extend(d.body_at(u, t).map(|p| (None, p)));
        for g in &gadgets {
            if g.classes.first() == Some(&world::DEVICE) && g.owner == Some(u) {
                at.extend(placed_at(g, frame).map(|p| (Some(g.id), p)));
            }
        }
        let mut best: Option<(f32, Option<u64>, &Gadget)> = None;
        for g in gadgets.iter().filter(|g| g.name.as_deref() == Some(JAMMER)) {
            // In use from its first `live` 1 to half a second after the
            // first sign of its end.
            let mut on = None;
            let mut off = None;
            for ev in g.events.iter().filter(|e| e.frame.is_some()) {
                match ev.what {
                    What::Live(1) => on = Some(d.time(ev.frame)),
                    What::Live(_) | What::Deleted if on.is_some() && off.is_none() => {
                        off = Some(d.time(ev.frame));
                    }
                    _ => {}
                }
            }
            let alive = on.is_some_and(|on| on <= t) && off.is_none_or(|off| t <= off + 0.5);
            let Some(place) = placed_at(g, frame).filter(|_| alive) else {
                continue;
            };
            for &(device, p) in &at {
                let apart = distance(p, place);
                if best.is_none_or(|b| apart < b.0) {
                    best = Some((apart, device, g));
                }
            }
        }
        if let Some((apart, device, g)) = best.filter(|b| b.0 <= JAM_RANGE) {
            jams.push(Jam {
                username: ef.username.clone(),
                recording_time: t,
                jammer: g.id,
                jammer_owner: d.username(g.owner),
                device,
                distance: apart,
            });
        }
    }

    // ---- 10. barbed wire hits and the wire they are from (nearest)
    let mut wire_hits = Vec::new();
    let mut wire_misses = 0;
    for (hi, h) in ctx.hits.iter().enumerate() {
        if h.kind.id != WIRE_DAMAGE {
            continue;
        }
        let Some(t) = h.recording_time else { continue };
        let frame = frame_times.partition_point(|&f| f <= t + 0.0005);
        let frame = u32::try_from(frame.saturating_sub(1)).ok();
        let body = d.player(&h.username).and_then(|p| d.body_at(p, t));
        let wires = gadgets
            .iter()
            .filter(|g| g.name.as_deref() == Some(BARBED_WIRE));
        let nearest = wires
            .filter(|g| {
                // In use from `live` 1 until it goes inert or is taken.
                let mut on = false;
                for ev in g
                    .events
                    .iter()
                    .filter(|e| e.frame.is_some() && e.frame <= frame)
                {
                    match ev.what {
                        What::Live(1) => on = true,
                        What::Live(_) | What::Deleted | What::Flags(INERT) => on = false,
                        _ => {}
                    }
                }
                on
            })
            .filter_map(|g| Some((distance(body?, placed_at(g, frame)?), g)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        match nearest.filter(|n| n.0 <= WIRE_RANGE) {
            Some((apart, g)) => wire_hits.push(WireHit {
                hit: hi,
                entity: g.id,
                username: d.username(g.owner),
                distance: apart,
            }),
            None => wire_misses += 1,
        }
    }

    // ---- what is written
    let mut out = GadgetEvents {
        jams,
        wire_hits,
        wire_misses,
        ..GadgetEvents::default()
    };
    if out.wire_misses > 0 {
        out.warnings.push(format!(
            "{} barbed wire hits have no wire within {WIRE_RANGE} m",
            out.wire_misses
        ));
    }
    for s in scores {
        let Some(username) = d.username(Some(s.player)) else {
            continue;
        };
        out.score.push(ScoreChange {
            username,
            delta: s.delta,
            total: s.total,
            reason: s.reason,
            detail: s.detail,
            reason_source: s.reason.map(|_| "coincidence"),
            when: d.when(s.frame),
        });
    }
    for r in removals {
        let Some(g) = gadgets.get(r.gadget) else {
            continue;
        };
        out.removals.push(Removal {
            subject: d.subject(g),
            position: r.position,
            signals: r.signals,
            verdict: Verdict {
                owner_team: r.owner_team,
                cause: r.cause,
                cause_source: r.cause_source,
                by: d.username(r.by),
                by_source: r.by_source,
                points: r.points,
                friendly: r.friendly,
                ambiguous: r.ambiguous,
                means: r.means,
                means_source: r.means_source,
                weapon: r.weapon,
                weapon_slot: r.weapon_slot,
                interceptor: r.interceptor,
            },
            when: d.when(Some(r.frame)),
            frame: r.frame,
        });
    }
    let mut shown: Vec<Mark> = statuses.into_iter().chain(captures).collect();
    shown.sort_by(|a, b| {
        (a.time.total_cmp(&b.time))
            .then(entity_of(a.gadget).cmp(&entity_of(b.gadget)))
            .then(a.kind.cmp(&b.kind))
    });
    for s in shown {
        let Some(g) = gadgets.get(s.gadget) else {
            continue;
        };
        out.statuses.push(Status {
            subject: d.subject(g),
            shown: Shown {
                kind: s.kind,
                fx_asset: s.fx_asset,
                team: s.team,
                by: d.username(s.by),
                by_source: s.by.map(|_| "score"),
                target: s.target,
                until: s.until.map(|f| d.when(Some(f))),
                when: d.when(Some(s.frame)),
            },
            frame: s.frame,
        });
    }
    for t in traps {
        let g = t.gadget.and_then(|gi| gadgets.get(gi));
        out.traps.push(TrapTrigger {
            trap: t.name,
            entity: g.map(|g| g.id),
            asset: g.map(|g| g.asset),
            type_index: g.and_then(|g| g.type_index),
            username: d.username(t.owner),
            trigger: Trigger {
                marker: t.marker,
                detonated_at: t.detonated_at,
                location: t.location,
                position: t.position,
                victims: t.victims,
                nearest_enemy: d.username(t.nearest.map(|n| n.1)),
                nearest_enemy_distance: t.nearest.map(|n| n.0),
                nearest_enemy_source: t.nearest.map(|_| "nearest"),
                points: t.points,
                when: d.when(t.frame),
            },
            frame: t.frame,
        });
    }
    if d.untimed.get() > 0 {
        out.warnings.push(format!(
            "{} events are in frames the recording has no time for",
            d.untimed.get()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(time: f64, player: usize, delta: i32) -> Score {
        Score {
            time,
            player,
            delta,
            ..Score::default()
        }
    }

    fn gone(time: f64, owner: usize, team: usize, name: &str) -> Gone {
        Gone {
            time,
            owner: Some(owner),
            owner_team: Some(team),
            name: Some(name.to_owned()),
            signals: vec![Signal::NotLive],
            ..Gone::default()
        }
    }

    /// Players 0 and 1 are team 0, players 2 and 3 team 1.
    const TEAMS: [usize; 4] = [0, 0, 1, 1];

    fn paired(scores: &mut [Score], removals: &mut [Gone]) {
        pay(scores, removals, &TEAMS, true);
        pay(scores, removals, &TEAMS, false);
    }

    #[test]
    fn one_write_pays_for_as_many_removals_as_it_covers() {
        let mut scores = [score(10.03, 2, 30)];
        let mut removals = [
            gone(10.0, 0, 0, "Barbed Wire"),
            gone(10.0, 0, 0, "Barbed Wire"),
            gone(10.0, 1, 0, "Shock Wire"),
        ];
        paired(&mut scores, &mut removals);
        for r in &removals {
            assert_eq!(
                (r.cause, r.by, r.points),
                (Cause::Destroyed, Some(2), Some(30))
            );
            assert_eq!(r.by_source, Some("score"));
            assert!(!r.friendly && !r.ambiguous);
        }
        assert_eq!(scores[0].reason, Some("gadgetDestroyed"));
        assert_eq!(
            scores[0].detail.as_deref(),
            Some("Barbed Wire, Barbed Wire, Shock Wire")
        );
    }

    #[test]
    fn a_teammates_penalty_is_paired_before_the_points() {
        // Player 1 breaks a teammate's gadget as player 2 breaks another
        // of that team.
        let mut scores = [score(5.05, 2, 10), score(5.05, 1, -10)];
        let mut removals = [
            gone(5.0, 0, 0, "Barbed Wire"),
            gone(5.04, 0, 0, "Barbed Wire"),
        ];
        paired(&mut scores, &mut removals);
        assert_eq!((removals[0].by, removals[0].friendly), (Some(1), true));
        assert_eq!((removals[1].by, removals[1].friendly), (Some(2), false));
        assert_eq!(scores[1].reason, Some("friendlyGadgetDestroyed"));
        assert_eq!(scores[0].reason, Some("gadgetDestroyed"));
    }

    #[test]
    fn a_penalty_is_not_for_ones_own_gadget() {
        let mut scores = [score(5.05, 0, -10)];
        let mut removals = [gone(5.0, 0, 0, "Barbed Wire")];
        paired(&mut scores, &mut removals);
        assert_eq!(removals[0].by, None);
        assert_eq!(scores[0].reason, None);
    }

    #[test]
    fn more_removals_than_points_is_ambiguous() {
        let mut scores = [score(10.05, 2, 10)];
        let mut removals = [
            gone(10.0, 0, 0, "Barbed Wire"),
            gone(9.96, 1, 0, "Shock Wire"),
        ];
        paired(&mut scores, &mut removals);
        // The removal nearest 0.05 s before the score takes it.
        assert_eq!(removals[0].by, Some(2));
        assert!(removals[0].ambiguous);
        assert_eq!(removals[1].by, None);
        assert_eq!(removals[1].cause, Cause::Unknown);
    }

    #[test]
    fn a_gadget_is_worth_its_points() {
        // A Black Eye is worth 20: a +10 does not pay for it.
        let mut scores = [score(1.05, 2, 10), score(2.05, 2, 20), score(3.05, 3, 5)];
        let mut removals = [
            gone(1.0, 0, 0, "Black Eye"),
            gone(2.0, 0, 0, "Black Eye"),
            gone(3.0, 0, 0, "T.R.I.P. Connector"),
        ];
        paired(&mut scores, &mut removals);
        assert_eq!(removals[0].by, None);
        assert_eq!(removals[1].by, Some(2));
        assert_eq!(removals[2].by, Some(3));
    }

    #[test]
    fn a_projectile_is_only_intercepted() {
        let thrown = |time| Gone {
            projectile: true,
            signals: vec![Signal::Deleted],
            ..gone(time, 0, 0, "Frag Grenade")
        };
        let mut scores = [score(1.05, 2, 10), score(2.05, 2, 20)];
        let mut removals = [thrown(1.0), thrown(2.0)];
        paired(&mut scores, &mut removals);
        assert_eq!(removals[0].by, None);
        assert_eq!(
            (removals[1].by, removals[1].cause),
            (Some(2), Cause::Intercepted)
        );
    }

    #[test]
    fn a_score_outside_the_window_pays_nothing() {
        let mut scores = [score(10.2, 2, 10), score(9.7, 3, 10)];
        let mut removals = [gone(10.0, 0, 0, "Barbed Wire")];
        paired(&mut scores, &mut removals);
        assert_eq!(removals[0].by, None);
    }

    #[test]
    fn a_state_machine_blob_gives_its_state() {
        let mut a = vec![8, 0, 0, 0];
        a.extend(OPENING);
        a.extend([0; 8]);
        assert_eq!(machine_state(&(MACHINE_A, a)), Some((MACHINE_A, OPENING)));
        let mut b = vec![12, 0, 0, 0, 1, 0, 0, 0];
        b.extend(CLOSED);
        b.extend([0; 8]);
        assert_eq!(machine_state(&(MACHINE_B, b)), Some((MACHINE_B, CLOSED)));
        assert_eq!(machine_state(&(MACHINE_A, vec![0; 3])), None);
        assert_eq!(machine_state(&([1, 2, 3, 4], vec![0; 16])), None);
    }

    #[test]
    fn events_are_what_changed() {
        use crate::world::{Change, Device, Owner};
        let pids = HashMap::from([(77u64, 3usize)]);
        let change = |frame, live, released| Change {
            frame: Some(frame),
            live,
            owner: Some(Owner {
                player: Some(77),
                released,
                ..Owner::default()
            }),
            ..Change::default()
        };
        let mut dead = change(9, Some(0), Some(0));
        dead.device = Some(Device {
            destroyed: Some(true),
            ..Device::default()
        });
        dead.position = Some([1.0, 2.0, 3.0]);
        let e = Entity {
            changes: vec![
                change(1, Some(1), Some(1)),
                change(2, Some(1), Some(1)),
                dead,
            ],
            deleted: Some(40),
            ..Entity::default()
        };
        let (events, track) = events_of(&e, &pids);
        let what: Vec<What> = events.iter().map(|e| e.what).collect();
        assert_eq!(
            what,
            [
                What::Live(1),
                What::OwnerPlayer(77),
                What::OwnerReleased(1),
                What::Live(0),
                What::DestroyedFlag(true),
                What::OwnerReleased(0),
                What::Deleted,
            ]
        );
        assert_eq!(track, [(Some(9), [1.0, 2.0, 3.0])]);
        assert_eq!(
            events.last().map(|e| e.position),
            Some(Some([1.0, 2.0, 3.0]))
        );
    }

    #[test]
    fn hud_values_and_paths() {
        let mut hud = Hud::default();
        hud.link(1, LOADOUT_FIELD, 2, [0; 4], None);
        hud.link(2, GADGET_FIELD, 3, GADGET_VIEW, None);
        hud.link(3, GADGET_STATES, 4, GADGET_STATE_ITEM, Some(0));
        hud.link(3, GADGET_STATES, 0, GADGET_STATE_ITEM, Some(1));
        for (frame, v) in [(1, 1u8), (2, 1), (3, 0)] {
            hud.property(4, STATE, Some(&[v]), Some(frame));
        }
        hud.property(4, [9; 4], Some(&[1]), Some(4));
        assert_eq!(
            hud.changes(4, STATE),
            [(Some(1), Some(1)), (Some(3), Some(0))]
        );
        let controllers = HashMap::from([(1u32, 6usize)]);
        assert_eq!(
            hud.path(3, &controllers),
            (vec![GADGET_FIELD, LOADOUT_FIELD], Some(6))
        );
        assert_eq!(hud.path(9, &controllers), (Vec::new(), None));
        assert_eq!(hud.of_class(GADGET_VIEW).collect::<Vec<_>>(), [3]);
        assert_eq!(number(&[1, 0, 0, 0]), Some(1));
        assert_eq!(number(&[1, 2]), None);
    }
}
