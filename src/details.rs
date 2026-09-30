//! Round data beyond the scoreboard and kill feed: bans, damage, observation tools.

use serde::Serialize;

use crate::types::{ObservationTool, Operator, TeamRole};

/// An operator banned for this round.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ban {
    /// `None` when the icon is not in the role image table (usually a replay
    /// from an older season, whose icons have other ids).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<Operator>,
    /// The side the banned operator plays on.
    pub role: TeamRole,
    /// Index of the team that owns the ban slot, i.e. the team that banned
    /// the operator (Y11S3+). Slots are listed in the game's order, so bans
    /// of one team keep their slot order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<usize>,
    /// The operator's role image id, which is all the replay records.
    pub icon: u64,
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
    pub time: String,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub time_in_seconds: f64,
    pub phase: Phase,
    /// Seconds since the prep phase started; see `Round::phases`.
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum LifeEventType {
    /// Health reached zero without dying outright (DBNO).
    Down,
    /// Picked back up from a down.
    Revive,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifeEvent {
    #[serde(rename = "type")]
    pub kind: LifeEventType,
    pub username: String,
    pub time: String,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub time_in_seconds: f64,
    pub phase: Phase,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
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
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub seconds: f64,
}

/// The equipment a player spawned with on one operator. Ids only: replays
/// carry no item names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Loadout {
    pub username: String,
    pub operator: Operator,
    /// Guns, primary first. Kill feed `weapon` ids refer to these.
    pub weapons: Vec<u64>,
    /// The operator's ability, then their secondary gadget.
    pub gadgets: Vec<u64>,
}
