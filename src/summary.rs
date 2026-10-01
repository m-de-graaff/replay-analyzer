//! One record per match, for match history: who played what, where, under
//! which rules, and how it ended. Built from round headers only, so it works on
//! header-only reads (`ReadMode::Header`) as well as full ones.

use chrono::{DateTime, Utc};
use serde::{Serialize, Serializer};

use std::collections::HashMap;

use crate::details::Ban;
use crate::entities::Relation;
use crate::header::{PartyRole, Player};
use crate::outcome::{ReasonSource, scores};
use crate::round::Round;
use crate::types::{GameMode, Map, MatchType, Operator, TeamRole, WinCondition};

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchSummary {
    /// Same for every round and every recording player, so teammates'
    /// imports of one match share it. Empty in replays that predate it.
    #[serde(rename = "matchID")]
    pub match_id: String,
    /// When the first round read started, UTC. From Y11S3 `starttime`; older
    /// replays only carry the recording PC's local time, see
    /// `start_time_is_local`.
    #[serde(serialize_with = "rfc3339")]
    pub start_time: DateTime<Utc>,
    /// True when `start_time` is the header's local `datetime`, not UTC.
    pub start_time_is_local: bool,
    /// When the last round read stopped, UTC (Y11S3+).
    #[serde(
        serialize_with = "rfc3339_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub end_time: Option<DateTime<Utc>>,
    pub match_type: MatchType,
    /// Queue family derived from `match_type`: `ranked`, `unranked`,
    /// `quickMatch`, `custom`, `standard` or `unknown`.
    pub queue: &'static str,
    /// The header's `playlistcategory`, when written. Raw: not yet named.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_category: Option<i64>,
    pub game_mode: GameMode,
    pub map: MapInfo,
    pub rules: Rules,
    pub teams: [TeamSummary; 2],
    pub recording: Recording,
    /// Index into `teams` of the recording player's team. `None` for
    /// spectators and when the recording player is not in the player list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub your_team: Option<usize>,
    pub result: MatchResult,
    /// One entry per round read, in play order.
    pub rounds: Vec<RoundSummary>,
}

/// A map, keyed by id: reworked maps get new ids (`Bank` vs `BankY10`), so
/// floor plans and callouts should be looked up by `id`, not by `base`. A
/// new id marks a new world build; its floor plan can still be the old one.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapInfo {
    pub id: u64,
    /// Full name, `Map(<id>)` when unknown.
    pub name: String,
    /// The name without its rework suffix: `Bank` for both `Bank` and
    /// `BankY10`. `None` when the id is unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// The year suffix of the world build (`Y10`), `None` for the original
    /// build.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl MapInfo {
    pub fn new(map: Map) -> Self {
        let name = map.to_string();
        let (base, version) = match map.name() {
            Some(n) => {
                let (base, version) = split_rework(n);
                (Some(base.to_owned()), version.map(str::to_owned))
            }
            None => (None, None),
        };
        Self {
            id: map.0,
            name,
            base,
            version,
        }
    }
}

/// Splits `BankY10` into `("Bank", Some("Y10"))`.
fn split_rework(name: &str) -> (&str, Option<&str>) {
    if let Some(i) = name.rfind('Y') {
        let digits = &name[i + 1..];
        if i > 0 && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            return (&name[..i], Some(&name[i..]));
        }
    }
    (name, None)
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rules {
    /// Regulation rounds (`roundspermatch`), e.g. 12.
    pub rounds_per_match: u32,
    /// Rounds a team needs to win in regulation, e.g. 7 of 12.
    pub rounds_to_win: u32,
    /// Overtime rounds (`roundspermatchovertime`); 0 means a tie at the end
    /// of regulation is a draw.
    pub overtime_rounds: u32,
    /// Total rounds a team needs to win once overtime starts, e.g. 8 when
    /// 12 regulation rounds are followed by 3 overtime rounds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overtime_rounds_to_win: Option<u32>,
    /// Y11S3+ `maxnbplayersperteam`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_players_per_team: Option<u32>,
    /// Raw `gmsetting` values, in header order. Only some are understood,
    /// so they are kept as-is to be named over time.
    pub game_mode_settings: Vec<i64>,
}

impl Rules {
    pub fn new(rounds_per_match: u32, overtime_rounds: u32) -> Self {
        let half = rounds_per_match / 2;
        Self {
            rounds_per_match,
            rounds_to_win: half + 1,
            overtime_rounds,
            overtime_rounds_to_win: (overtime_rounds > 0).then(|| half + overtime_rounds / 2 + 1),
            ..Self::default()
        }
    }

    /// Whether a round with this score going into it is played in overtime.
    pub fn is_overtime(&self, score: [u32; 2]) -> bool {
        let half = self.rounds_per_match / 2;
        self.overtime_rounds > 0 && score[0] >= half && score[1] >= half
    }

    /// Rounds needed to win at this score.
    fn target(&self, score: [u32; 2]) -> u32 {
        match self.overtime_rounds_to_win {
            Some(ot) if self.is_overtime(score) => ot,
            _ => self.rounds_to_win,
        }
    }

    /// The team that has won at this score, if any.
    pub fn winner(&self, score: [u32; 2]) -> Option<usize> {
        if self.rounds_per_match == 0 {
            return None;
        }
        let target = self.target(score);
        (0..2).find(|&t| score[t] >= target)
    }

    /// Whether this score ends the match without a winner.
    fn is_draw(&self, score: [u32; 2]) -> bool {
        self.rounds_per_match > 0
            && self.overtime_rounds == 0
            && score[0] + score[1] >= self.rounds_per_match
            && score[0] == score[1]
    }

    /// Chance each team wins the match from this score if every remaining
    /// round were a coin flip. `None` when the rules are unknown.
    pub fn win_probability(&self, score: [u32; 2]) -> Option<[f64; 2]> {
        if self.rounds_per_match == 0 {
            return None;
        }
        let mut memo = HashMap::new();
        let p = self.team0_wins(score, &mut memo);
        Some([p, 1.0 - p])
    }

    /// Chance team 0 wins from `score`; a draw counts half.
    fn team0_wins(&self, score: [u32; 2], memo: &mut HashMap<[u32; 2], f64>) -> f64 {
        if let Some(w) = self.winner(score) {
            return if w == 0 { 1.0 } else { 0.0 };
        }
        // Rules that never end (or that this model cannot follow) are a tie.
        let cap = self.rounds_per_match + self.overtime_rounds.max(1) * 4;
        if self.is_draw(score) || score[0] + score[1] >= cap {
            return 0.5;
        }
        if let Some(&p) = memo.get(&score) {
            return p;
        }
        let p = 0.5 * self.team0_wins([score[0] + 1, score[1]], memo)
            + 0.5 * self.team0_wins([score[0], score[1] + 1], memo);
        memo.insert(score, p);
        p
    }

    /// Teams one round away from winning at this score.
    pub fn match_point(&self, score: [u32; 2]) -> [bool; 2] {
        if self.rounds_per_match == 0 || self.winner(score).is_some() {
            return [false; 2];
        }
        let target = self.target(score);
        [score[0] + 1 >= target, score[1] + 1 >= target]
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamSummary {
    pub name: String,
    /// Rounds won, after the last round read.
    pub score: u32,
    /// Side played in the first round read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starting_side: Option<TeamRole>,
    /// Players on this team in the first round read, in header order.
    pub players: Vec<PlayerSummary>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSummary {
    pub username: String,
    /// Stable across matches: the profile id, else `name:<username>`.
    pub key: String,
    /// Ubisoft profile id: the key for ranks and other stats from Ubisoft's
    /// services, which replays do not record.
    #[serde(rename = "profileID", skip_serializing_if = "String::is_empty")]
    pub profile_id: String,
    /// Y11S3+, most likely the clearance level (full and partial reads).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    /// `you`, `teammate` or `opponent`; absent for spectator recordings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<Relation>,
    /// `leader` or `member` of the recording player's party (full and
    /// partial reads, Y8S1+, not in custom games).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<PartyRole>,
}

/// The operators one player played in a round, in order. More than one when
/// they swapped (attackers in prep, or modes that allow changes mid-round).
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pick {
    pub username: String,
    pub team: usize,
    pub operators: Vec<Operator>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(rename = "profileID", skip_serializing_if = "String::is_empty")]
    pub profile_id: String,
    /// Y11S3+ `isspectator`; before that, inferred from the recording player
    /// being absent from the player list.
    pub spectator: bool,
    /// The other players who queued with the recording player (full and
    /// partial reads, Y8S1+).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub party: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Win,
    Loss,
    Draw,
    /// The match was decided but the recording player was not on a team.
    Decided,
    /// Not decided in the rounds read.
    #[default]
    Unfinished,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchResult {
    /// Scores after the last round read, indexed like `teams`.
    pub final_score: [u32; 2],
    /// Index into `teams` of the winner, when the match was decided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner: Option<usize>,
    /// From the recording player's side; `decided` for spectators.
    pub outcome: Outcome,
    /// Whether the match went to overtime.
    pub overtime: bool,
    /// Whether the rounds read end the match. `false` usually means later
    /// rounds are missing from the folder.
    pub complete: bool,
    /// The game says the match is over (Y11S3+ `matchresult`) although
    /// neither team reached the rounds needed to win: a forfeit or an
    /// abandoned match. Inferred; `None` when the replay cannot tell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_early: Option<bool>,
    /// The raw Y11S3+ `matchresult` value of the deciding round.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_match_result: Option<u32>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundSummary {
    /// Counting from 1, as the file names do.
    pub number: u32,
    /// Score going into the round.
    pub score_before: [u32; 2],
    /// Score after the round.
    pub score_after: [u32; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner: Option<usize>,
    /// Side of each team this round, when known (full reads).
    pub sides: [Option<TeamRole>; 2],
    /// Teams on match point going into the round.
    pub match_point: [bool; 2],
    pub overtime: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub site: String,
    /// Operators banned for the round; `team` says who banned each (Y11S3+).
    /// Empty on header-only reads.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bans: Vec<Ban>,
    /// Operators each player played. Empty on header-only reads.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub picks: Vec<Pick>,
    /// Chance each team wins the match going into the round, if every
    /// remaining round were a coin flip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub win_probability: Option<[f64; 2]>,
    /// How the round ended (full reads).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_reason: Option<WinCondition>,
    #[serde(skip_serializing_if = "is_unknown")]
    pub end_reason_source: ReasonSource,
    /// Players per team alive when action started (full reads).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub players_at_start: Option<[usize; 2]>,
}

fn is_unknown(s: &ReasonSource) -> bool {
    *s == ReasonSource::Unknown
}

impl MatchSummary {
    /// Summarises `rounds`, which must be in play order. `None` when empty.
    pub fn new(rounds: &[Round]) -> Option<Self> {
        let first = &rounds.first()?.header;
        let last = &rounds.last()?.header;

        let mut rules = Rules::new(first.rounds_per_match, first.rounds_per_match_overtime);
        rules.max_players_per_team = first.max_players_per_team;
        rules.game_mode_settings = first.gm_settings.clone();

        let recorder = first.recording_player();
        let spectator = first.is_spectator.unwrap_or(recorder.is_none());
        let your_team = recorder
            .filter(|_| !spectator)
            .map(|p| p.team_index)
            .filter(|&t| t < 2);

        let teams = [0, 1].map(|t| TeamSummary {
            name: first.teams[t].name.clone(),
            score: 0,
            starting_side: first.teams[t].role,
            players: first
                .players
                .iter()
                .filter(|p| p.team_index == t)
                .map(|p| {
                    let same = |r: &'_ Round| -> Option<Player> {
                        r.header
                            .players
                            .iter()
                            .find(|q| q.key == p.key || (q.id != 0 && q.id == p.id))
                            .cloned()
                    };
                    PlayerSummary {
                        username: p.username.clone(),
                        key: p.key.clone(),
                        profile_id: p.profile_id.clone(),
                        level: rounds.iter().find_map(|r| same(r)?.level),
                        relation: p.relation,
                        party: rounds.iter().find_map(|r| same(r)?.party),
                    }
                })
                .collect(),
        });

        let round_summaries: Vec<RoundSummary> =
            rounds.iter().map(|r| round_summary(r, &rules)).collect();
        let final_score = round_summaries.last().map_or([0; 2], |r| r.score_after);

        let raw_match_result = rounds.iter().find_map(|r| r.header.match_result);
        let decided_by_game = raw_match_result.is_some();
        let mut winner = rules.winner(final_score);
        let draw = winner.is_none() && rules.is_draw(final_score);
        let ended_early = if winner.is_none() && !draw && decided_by_game {
            // Neither side reached the target, yet the game ended the match.
            // `matchresult` matched the winner's index in the one sample seen.
            winner = raw_match_result.map(|v| v as usize).filter(|&t| t < 2);
            Some(true)
        } else if decided_by_game {
            Some(false)
        } else {
            None
        };
        let complete = winner.is_some() || draw;
        let outcome = match (winner, your_team) {
            (Some(w), Some(y)) if w == y => Outcome::Win,
            (Some(_), Some(_)) => Outcome::Loss,
            (Some(_), None) => Outcome::Decided,
            (None, _) if draw => Outcome::Draw,
            (None, _) => Outcome::Unfinished,
        };

        let (start_time, start_time_is_local) = match first.start_time {
            Some(t) => (t, false),
            None => (first.timestamp, true),
        };

        let party = teams
            .iter()
            .flat_map(|t| &t.players)
            .filter(|p| p.party.is_some() && p.relation != Some(Relation::You))
            .map(|p| p.username.clone())
            .collect();
        let mut teams = teams;
        for (t, team) in teams.iter_mut().enumerate() {
            team.score = final_score[t];
        }

        Some(Self {
            match_id: first.match_id.clone(),
            start_time,
            start_time_is_local,
            end_time: last.end_time,
            match_type: first.match_type,
            queue: queue(first.match_type),
            playlist_category: (first.playlist_category != 0).then_some(first.playlist_category),
            game_mode: first.game_mode,
            map: MapInfo::new(first.map),
            teams,
            recording: Recording {
                username: recorder.map(|p| p.username.clone()),
                profile_id: first.recording_profile_id.clone(),
                spectator,
                party,
            },
            your_team,
            result: MatchResult {
                final_score,
                winner,
                outcome,
                overtime: round_summaries.iter().any(|r| r.overtime),
                complete,
                ended_early,
                raw_match_result,
            },
            rules,
            rounds: round_summaries,
        })
    }
}

fn round_summary(round: &Round, rules: &Rules) -> RoundSummary {
    let h = &round.header;
    let t = &h.teams;
    let (before, after) = scores(round);
    let winner = (0..2).find(|&i| after[i] > before[i]);
    let info = round.info();
    RoundSummary {
        number: h.round_number + 1,
        score_before: before,
        score_after: after,
        winner,
        sides: [t[0].role, t[1].role],
        match_point: rules.match_point(before),
        overtime: rules.is_overtime(before),
        site: h.site.clone(),
        bans: round.bans.clone(),
        picks: picks(round),
        win_probability: rules.win_probability(before),
        end_reason: info.end_reason,
        end_reason_source: info.end_reason_source,
        players_at_start: info.players_at_start,
    }
}

/// Operators per player in play order: one per loadout (a loadout is written
/// for every operator played), else the operator the round ended on.
fn picks(round: &Round) -> Vec<Pick> {
    if round.match_feedback.is_empty() && round.loadouts.is_empty() {
        // Header-only read: the header's operator is not a confirmed pick.
        return Vec::new();
    }
    round
        .header
        .players
        .iter()
        .filter(|p| p.team_index < 2)
        .map(|p| {
            let mut operators: Vec<Operator> = round
                .loadouts
                .iter()
                .filter(|l| l.username == p.username)
                .map(|l| l.operator)
                .collect();
            operators.dedup();
            if operators.is_empty() && !p.operator.is_empty() {
                operators.push(p.operator);
            }
            Pick {
                username: p.username.clone(),
                team: p.team_index,
                operators,
            }
        })
        .collect()
}

/// The queue family a match type belongs to.
pub fn queue(match_type: MatchType) -> &'static str {
    match match_type.0 {
        1 => "quickMatch",
        2 => "ranked",
        3 | 4 => "custom",
        8 => "standard",
        9 => "unranked",
        _ => "unknown",
    }
}

fn rfc3339_opt<S: Serializer>(t: &Option<DateTime<Utc>>, s: S) -> Result<S::Ok, S::Error> {
    match t {
        Some(t) => rfc3339(t, s),
        None => s.serialize_none(),
    }
}

fn rfc3339<S: Serializer>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(&t.format("%Y-%m-%dT%H:%M:%SZ"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_reworked_map_names() {
        assert_eq!(split_rework("BankY10"), ("Bank", Some("Y10")));
        assert_eq!(split_rework("ConsulateY7"), ("Consulate", Some("Y7")));
        assert_eq!(split_rework("Bank"), ("Bank", None));
        assert_eq!(split_rework("Yacht"), ("Yacht", None));
        let m = MapInfo::new(Map(413779563590));
        assert_eq!(
            (m.base.as_deref(), m.version.as_deref()),
            (Some("Bank"), Some("Y10"))
        );
    }

    #[test]
    fn ranked_thresholds() {
        let r = Rules::new(12, 3);
        assert_eq!((r.rounds_to_win, r.overtime_rounds_to_win), (7, Some(8)));
        assert_eq!(r.winner([7, 3]), Some(0));
        assert_eq!(r.winner([6, 6]), None);
        assert!(r.is_overtime([6, 6]));
        assert_eq!(r.winner([7, 6]), None, "overtime needs 8");
        assert_eq!(r.winner([6, 8]), Some(1));
        assert_eq!(r.match_point([6, 3]), [true, false]);
        assert_eq!(r.match_point([6, 6]), [false, false]);
        assert_eq!(r.match_point([7, 6]), [true, false]);
        assert_eq!(r.match_point([7, 7]), [true, true]);
    }

    #[test]
    fn no_overtime_allows_draws() {
        let r = Rules::new(4, 0);
        assert_eq!(r.rounds_to_win, 3);
        assert!(r.is_draw([2, 2]));
        assert!(!r.is_draw([1, 1]));
        assert_eq!(r.winner([2, 2]), None);
    }
}
