//! Round data beyond the scoreboard and kill feed: bans, damage, observation tools.

use serde::Serialize;

use crate::loadout::{Counted, Weapon};
use crate::types::{ObservationTool, Operator, TeamRole};

/// An operator banned for this round, or (Y11S3+) a ban slot whose vote
/// ended without a ban.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ban {
    /// `None` when the icon is not in the role image table (usually a replay
    /// from an older season, whose icons have other ids), or for `noBan`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<Operator>,
    /// The side the banned operator plays on.
    pub role: TeamRole,
    /// Index of the team that owns the ban slot, i.e. the team that banned
    /// the operator (Y11S3+). Bans are listed by team, then by slot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<usize>,
    /// The operator's role image id, which is all the replay records.
    /// `None` for `noBan`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<u64>,
    /// Y11S3+: the slot's place in its team's ban list, from 0: the order the
    /// team banned in. In ranked a team bans once before each round of a
    /// half, into the next slot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// Y11S3+: the team's vote ended without a ban, so the slot holds no
    /// operator.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub no_ban: bool,
    /// The slot's `TeamColor`, which `team` is derived from: the game numbers
    /// a player's own team 1 in their recording, whatever its header index.
    #[serde(skip)]
    pub color: Option<u32>,
}

/// Where in the round something happened. Inferred from the clock: prep
/// counts down from 45 seconds, action from its round length, and a plant
/// restarts the clock at the defuser timer. `End` covers what the replay
/// records after the round was decided (the clock resets to 0:00).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
pub enum Phase {
    #[default]
    Prep,
    Action,
    /// After the defuser was planted.
    Planted,
    /// After the round was decided.
    End,
}

impl Phase {
    /// Action or after the plant: the part of the round where fighting counts.
    pub fn is_live(self) -> bool {
        matches!(self, Phase::Action | Phase::Planted)
    }
}

/// A change to a player's health, from damage or healing.
///
/// Replays record the victim's health only: who dealt the damage is not in
/// the stream.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthUpdate {
    pub username: String,
    pub health: u32,
    /// Negative for damage, positive for healing.
    pub change: i32,
    /// Y11S3: the most health the player could have without overheal when
    /// this was written, a Rook plate included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_health: Option<u32>,
    /// Y11S3: how much of `health` is above `max_health`. Left out when none
    /// is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overheal: Option<u32>,
    /// Y11S3: why the health changed, when it was not damage: overheal
    /// wearing off, a heal, a plate. Inferred, see [`crate::vitals`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<crate::vitals::Cause>,
    pub time: String,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub time_in_seconds: f64,
    pub phase: Phase,
    /// Seconds since the prep phase started; see `Round::phases`.
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
    /// Seconds since the recording started, to the frame (Y8S4+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_time: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum LifeEventType {
    /// Health reached zero without dying outright (DBNO).
    Down,
    /// Picked back up from a down.
    Revive,
}

/// How a down ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum DownOutcome {
    /// Another player killed them while down.
    Finished,
    /// They got back up.
    Revived,
    /// They died with no killer named. A bleed-out would read like this;
    /// none has been seen in a replay.
    Died,
    /// Still down when the recording ended.
    DownAtEnd,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifeEvent {
    #[serde(rename = "type")]
    pub kind: LifeEventType,
    pub username: String,
    /// Y11S3, `Revive`: the health the player got up with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<u32>,
    /// Y11S3: who downed the player, or who revived them, as the round's
    /// timeline names them (see [`crate::combat`]). Absent when it names
    /// nobody: a down that ended the round, or one nobody dealt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// Y11S3, `Revive`: the player got themselves up.
    #[serde(rename = "self", skip_serializing_if = "std::ops::Not::not")]
    pub self_revive: bool,
    /// Y11S3, `Down`: how the down ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<DownOutcome>,
    /// Y11S3, `Down` that ended `Finished`: the killer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_by: Option<String>,
    pub time: String,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub time_in_seconds: f64,
    pub phase: Phase,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
    /// Seconds since the recording started, to the frame (Y8S4+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_time: Option<f64>,
}

/// A stretch of time a player spent looking through an observation device.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationSession {
    pub username: String,
    /// Whose device it is: the player's own, or a teammate's.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub owner: String,
    pub tool: ObservationTool,
    pub phase: Phase,
    /// Round clock when the session started.
    pub time: String,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub time_in_seconds: f64,
    /// Seconds since the prep phase started, when the session started.
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
    /// Seconds since the recording started, when the session started, to the
    /// frame (Y8S4+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_time: Option<f64>,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub seconds: f64,
}

/// The equipment a player spawned with on one operator. Replays carry ids
/// only; names come from the lookup table.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Loadout {
    pub username: String,
    pub operator: Operator,
    /// Guns, primary first. Kill feed `weapon` ids refer to these.
    pub weapons: Vec<u64>,
    /// The operator's ability, then their secondary gadget.
    pub gadgets: Vec<u64>,
    /// Y11S3, for the operator the player spawned with: the primary and
    /// secondary with their attachments and ammunition. A shield operator's
    /// primary is the shield.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<Weapon>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary: Option<Weapon>,
    /// Y11S3: the ability and the secondary gadget, with how many the
    /// player had and when the count dropped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ability: Option<Counted>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gadget: Option<Counted>,
}

impl Loadout {
    pub fn new(username: &str, operator: Operator) -> Self {
        Loadout {
            username: username.to_owned(),
            operator,
            ..Loadout::default()
        }
    }
}
