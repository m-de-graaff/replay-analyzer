//! Per-round facts in one place: order and overtime, score, sides, site,
//! who won and how, who was alive at the start, what each player played and
//! where they spawned, prep swaps, and the phases of the round.

use serde::Serialize;

use crate::details::Phase;
use crate::feedback::MatchUpdateType;
use crate::round::Round;
use crate::summary::Rules;
use crate::timeline::PhaseSpan;
use crate::types::{Operator, TeamRole, WinCondition, version};

/// How the round's end reason was established.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReasonSource {
    /// Not a full read, or sides unknown.
    #[default]
    Unknown,
    /// The header's score and the events agree on the winner.
    Confirmed,
    /// No winner in the header (before Y9S4): from events alone.
    Events,
    /// The header's winner disagrees with the events; the header wins and
    /// the reason is the one that fits it. See `warnings`.
    Header,
    /// The header's score did not change (Y9S4+): the round was not played
    /// out, so it has no winner and no end reason.
    Unfinished,
}

/// Who won the round, how, and who was there at the start. Filled on full
/// reads.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundOutcome {
    pub winner: Option<usize>,
    pub reason: Option<WinCondition>,
    pub reason_source: ReasonSource,
    /// A plant completed.
    pub planted: bool,
    /// A disable completed.
    pub disabled: bool,
    /// Players per team alive when action started.
    pub players_at_start: [usize; 2],
    /// Players who were dead or had no health when action started.
    pub down_at_start: Vec<String>,
    /// Deaths per team among players alive at the start.
    pub deaths: [usize; 2],
    /// Players alive at the start who left before the round was decided
    /// (Y11S3+). They count as gone when deciding how the round ended.
    pub left: Vec<String>,
    /// The game's round history states this kind of win (Y11S3+), and the
    /// events agree.
    pub reason_stated: bool,
    pub warnings: Vec<String>,
}

/// A clock reading on the round's timeline.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Moment {
    pub time: String,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub time_in_seconds: f64,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
}

/// One player's round: side, final operator and spawn.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineupEntry {
    pub username: String,
    pub team: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<TeamRole>,
    /// The operator played once prep ended.
    pub operator: Operator,
    /// Attackers: the spawn picked. Defenders: the site defended.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub spawn: String,
    /// Operators played before `operator`, in order (attacker prep swaps).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub swapped_from: Vec<Operator>,
}

/// An operator change during the round (attackers during prep).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Swap {
    pub username: String,
    pub team: usize,
    pub from: Operator,
    pub to: Operator,
    pub time: String,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub time_in_seconds: f64,
    pub phase: Phase,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
    /// Made in the last 10 seconds of prep.
    pub late: bool,
}

/// Swaps in the last this-many seconds of prep are late. The clock shows
/// whole seconds rounded down, so `0:09` is the first late reading.
pub const LATE_SWAP_SECONDS: f64 = 10.0;

/// The `round` block of a round's JSON.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundInfo {
    /// Counting from 1, as the file names do.
    pub number: u32,
    pub overtime: bool,
    /// Counting from 1 within overtime.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overtime_number: Option<u32>,
    /// Score going into the round and after it.
    pub score_before: [u32; 2],
    pub score_after: [u32; 2],
    /// Teams one round from winning the match, going into the round.
    pub match_point: [bool; 2],
    /// Chance each team wins the match, going into the round, if every
    /// remaining round were a coin flip. A score-only baseline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub win_probability: Option<[f64; 2]>,
    pub sides: [Option<TeamRole>; 2],
    /// The defended site.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub site: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner_side: Option<TeamRole>,
    /// Elimination, defuser detonated, defuser disabled or time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_reason: Option<WinCondition>,
    pub end_reason_source: ReasonSource,
    pub planted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plant: Option<Moment>,
    /// When the round was decided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<Moment>,
    /// Players per team alive when action started (full reads).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub players_at_start: Option<[usize; 2]>,
    /// Teams that started action with fewer players than a full team.
    pub started_down: [bool; 2],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub down_at_start: Vec<String>,
    /// Players who left or lost connection during the round (Y11S3+).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub left: Vec<String>,
    pub lineup: Vec<LineupEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub swaps: Vec<Swap>,
    /// Prep, action, planted and end, with when each started and ended.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<PhaseSpan>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Score going into a round and after it. From Y9S4 the header holds the
/// score after the round; before, the score going into it.
pub fn scores(round: &Round) -> ([u32; 2], [u32; 2]) {
    let h = &round.header;
    let t = &h.teams;
    if h.code_version >= version::Y9S4 {
        (
            [t[0].starting_score, t[1].starting_score],
            [t[0].score, t[1].score],
        )
    } else {
        let before = [t[0].score, t[1].score];
        (before, [0, 1].map(|i| before[i] + u32::from(t[i].won)))
    }
}

impl Round {
    /// The rules this round was played under, from its header.
    pub fn rules(&self) -> Rules {
        Rules::new(
            self.header.rounds_per_match,
            self.header.rounds_per_match_overtime,
        )
    }

    /// Everything about the round itself, in one block.
    pub fn info(&self) -> RoundInfo {
        let h = &self.header;
        let rules = self.rules();
        let (before, after) = scores(self);
        let number = h.round_number + 1;
        let overtime = rules.is_overtime(before);
        let overtime_number = overtime.then(|| {
            // The header counts overtime rounds from 0.
            if h.overtime_round_number > 0 {
                h.overtime_round_number + 1
            } else {
                number.saturating_sub(h.rounds_per_match)
            }
        });
        let sides = [h.teams[0].role, h.teams[1].role];
        let winner = self
            .outcome
            .winner
            .or_else(|| (0..2).find(|&t| after[t] > before[t]));
        let full =
            !self.timeline.ticks.is_empty() && self.outcome.reason_source != ReasonSource::Unknown;
        let moment = |kind: MatchUpdateType| {
            self.match_feedback
                .iter()
                .find(|u| u.kind == kind)
                .map(|u| Moment {
                    time: u.time.clone(),
                    time_in_seconds: u.time_in_seconds,
                    elapsed: u.elapsed,
                })
        };
        let ended = self.timeline.end_start.map(|e| {
            let t = self.timeline.ticks[e];
            Moment {
                time: crate::feedback::display_clock(t.seconds),
                time_in_seconds: t.seconds,
                elapsed: t.elapsed,
            }
        });
        let full_team = h.max_players_per_team.unwrap_or(5) as usize;
        // Before action starts attackers are on drones, with no body to count.
        let full = full && self.timeline.action_start.is_some();
        let players_at_start = full.then_some(self.outcome.players_at_start);
        let started_down =
            players_at_start.map_or([false; 2], |p| [p[0] < full_team, p[1] < full_team]);

        let swaps: Vec<Swap> = self
            .match_feedback
            .iter()
            .filter(|u| u.kind == MatchUpdateType::OperatorSwap)
            .map(|u| Swap {
                username: u.username.clone(),
                team: u
                    .team
                    .unwrap_or_else(|| self.team_of(&u.username).unwrap_or(0)),
                from: u.previous_operator,
                to: u.operator,
                time: u.time.clone(),
                time_in_seconds: u.time_in_seconds,
                phase: u.phase,
                elapsed: u.elapsed,
                late: u.phase == Phase::Prep && u.time_in_seconds < LATE_SWAP_SECONDS,
            })
            .collect();
        let lineup = h
            .players
            .iter()
            .filter(|p| p.team_index < 2)
            .map(|p| LineupEntry {
                username: p.username.clone(),
                team: p.team_index,
                side: sides[p.team_index],
                operator: p.operator,
                spawn: p.spawn.clone(),
                swapped_from: swaps
                    .iter()
                    .filter(|s| s.username == p.username)
                    .map(|s| s.from)
                    .collect(),
            })
            .collect();

        RoundInfo {
            number,
            overtime,
            overtime_number,
            score_before: before,
            score_after: after,
            match_point: rules.match_point(before),
            win_probability: rules.win_probability(before),
            sides,
            site: h.site.clone(),
            winner,
            winner_side: winner.and_then(|w| sides[w]),
            end_reason: self.outcome.reason,
            end_reason_source: self.outcome.reason_source,
            planted: self.outcome.planted,
            plant: moment(MatchUpdateType::DefuserPlantComplete),
            ended,
            players_at_start,
            started_down,
            down_at_start: if full {
                self.outcome.down_at_start.clone()
            } else {
                Vec::new()
            },
            left: self.outcome.left.clone(),
            lineup,
            swaps,
            phases: self.timeline.spans(),
            warnings: self.outcome.warnings.clone(),
        }
    }

    fn team_of(&self, username: &str) -> Option<usize> {
        self.header
            .players
            .iter()
            .find(|p| p.username == username)
            .map(|p| p.team_index)
    }
}
