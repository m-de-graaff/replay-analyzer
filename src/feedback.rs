use serde::{Serialize, Serializer};

use crate::details::Phase;
use crate::types::Operator;

/// What happened in a [`MatchUpdate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MatchUpdateType {
    Kill = 0,
    Death,
    DefuserPlantStart,
    DefuserPlantComplete,
    DefuserDisableStart,
    DefuserDisableComplete,
    LocateObjective,
    OperatorSwap,
    Battleye,
    PlayerLeave,
    Other,
}

impl MatchUpdateType {
    pub fn name(self) -> &'static str {
        use MatchUpdateType::*;
        match self {
            Kill => "Kill",
            Death => "Death",
            DefuserPlantStart => "DefuserPlantStart",
            DefuserPlantComplete => "DefuserPlantComplete",
            DefuserDisableStart => "DefuserDisableStart",
            DefuserDisableComplete => "DefuserDisableComplete",
            LocateObjective => "LocateObjective",
            OperatorSwap => "OperatorSwap",
            Battleye => "Battleye",
            PlayerLeave => "PlayerLeave",
            Other => "Other",
        }
    }
}

impl Serialize for MatchUpdateType {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("MatchUpdateType", 2)?;
        st.serialize_field("name", self.name())?;
        st.serialize_field("id", &(*self as u8))?;
        st.end()
    }
}

/// One entry of the round's event feed.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchUpdate {
    #[serde(rename = "type")]
    pub kind: MatchUpdateType,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub username: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub target: String,
    /// Profile id of `username`, when the replay has one.
    #[serde(rename = "profileID", skip_serializing_if = "String::is_empty")]
    pub profile_id: String,
    /// Profile id of `target`.
    #[serde(rename = "targetProfileID", skip_serializing_if = "String::is_empty")]
    pub target_profile_id: String,
    /// Y11S3+: the player the scoreboard credits with this kill, when not
    /// `username` (a teammate downed the victim, `username` finished them).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub credited_to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headshot: Option<bool>,
    /// Round clock as displayed in game, e.g. `2:41`.
    pub time: String,
    #[serde(serialize_with = "whole_number_as_int")]
    pub time_in_seconds: f64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub message: String,
    /// The operator swapped to, for `OperatorSwap`.
    #[serde(skip_serializing_if = "Operator::is_empty")]
    pub operator: Operator,
    /// The operator swapped from, for `OperatorSwap`.
    #[serde(skip_serializing_if = "Operator::is_empty")]
    pub previous_operator: Operator,
    /// The weapon or gadget id behind a kill.
    #[serde(skip_serializing_if = "is_zero")]
    pub weapon: u64,
    /// Team the event belongs to when no player is named: Y11S3 defuser
    /// events carry the side (plant: attackers, disable: defenders) but not
    /// the player.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<usize>,
    pub phase: Phase,
    /// Seconds since the prep phase started: one clock for the whole round,
    /// across the prep, action and defuser timers.
    #[serde(serialize_with = "whole_number_as_int")]
    pub elapsed: f64,
    /// Seconds since the recording started, to the frame (Y8S4+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_time: Option<f64>,
    /// Index of the clock tick the event was read at; resolved into `time`,
    /// `phase` and `elapsed` once the whole round is read.
    #[serde(skip)]
    pub tick: Option<usize>,
    /// Where in the decompressed data the event was read.
    #[serde(skip)]
    pub offset: Option<usize>,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

impl MatchUpdate {
    pub fn new(kind: MatchUpdateType, clock: &Clock) -> Self {
        Self {
            kind,
            username: String::new(),
            target: String::new(),
            profile_id: String::new(),
            target_profile_id: String::new(),
            credited_to: String::new(),
            headshot: None,
            time: clock.display.clone(),
            time_in_seconds: clock.seconds,
            message: String::new(),
            operator: Operator::default(),
            previous_operator: Operator::default(),
            weapon: 0,
            team: None,
            phase: Phase::default(),
            elapsed: 0.0,
            recording_time: None,
            tick: clock.tick,
            offset: None,
        }
    }

    pub fn is_kill_or_death(&self) -> bool {
        matches!(self.kind, MatchUpdateType::Kill | MatchUpdateType::Death)
    }

    /// The player who died in this update, if anyone did.
    pub fn victim(&self) -> Option<&str> {
        match self.kind {
            MatchUpdateType::Kill => Some(&self.target),
            MatchUpdateType::Death => Some(&self.username),
            _ => None,
        }
    }
}

/// Emits `28` rather than `28.0`.
pub(crate) fn whole_number_as_int<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        s.serialize_i64(*v as i64)
    } else {
        s.serialize_f64(*v)
    }
}

/// The round clock as last seen in the packet stream.
#[derive(Clone, Debug, Default)]
pub struct Clock {
    pub seconds: f64,
    pub display: String,
    /// Index of this reading in the round's clock track.
    pub tick: Option<usize>,
}

/// `m:ss` for a whole number of seconds.
pub fn display_clock(seconds: f64) -> String {
    let t = seconds.max(0.0) as u32;
    format!("{}:{:02}", t / 60, t % 60)
}
