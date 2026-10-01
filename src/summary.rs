//! One record per match, for match history: who played what, where, under
//! which rules, and how it ended. Built from round headers only, so it works on
//! header-only reads (`ReadMode::Header`) as well as full ones.

use chrono::{DateTime, Utc};
use serde::{Serialize, Serializer};

use std::collections::HashMap;

use crate::details::Ban;
use crate::entities::Relation;
use crate::header::{PartyRole, Platform, Player};
use crate::outcome::{ReasonSource, scores};
use crate::round::Round;
use crate::types::{GameMode, Map, MatchType, Operator, TeamRole, WinCondition, playlist_name};

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
    /// The header's `playlistcategory`, when written (Y11S3+): an asset id
    /// with one value per playlist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_category: Option<i64>,
    /// The playlist's name, for the `playlistCategory` values seen:
    /// `Ranked`, `QuickMatch`, `Unranked`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist: Option<&'static str>,
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
    /// Each ban once, with the first round it applied to: what each team
    /// banned and when (full and partial reads).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bans: Vec<BanDecision>,
    /// One entry per round read, in play order.
    pub rounds: Vec<RoundSummary>,
}

/// One ban and the first round it applied to. In ranked each team bans once
/// before each round of a half, so `round` is the round the vote came before;
/// overtime rounds reuse earlier bans and add none.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BanDecision {
    /// The first round read, from 1, with this ban in force.
    pub round: u32,
    #[serde(flatten)]
    pub ban: Ban,
}

/// Every distinct ban of `rounds`, in the order they first apply.
fn ban_decisions(rounds: &[Round]) -> Vec<BanDecision> {
    let mut out: Vec<BanDecision> = Vec::new();
    for r in rounds {
        for b in &r.bans {
            match out.iter_mut().find(|d| ban_key(&d.ban) == ban_key(b)) {
                // A later round may know the team an earlier one could not.
                Some(d) => d.ban.team = d.ban.team.or(b.team),
                None => out.push(BanDecision {
                    round: r.header.round_number + 1,
                    ban: b.clone(),
                }),
            }
        }
    }
    out
}

/// What makes a ban the same one in another round. A slot's `TeamColor`
/// stays the same through a recording, while its team index needs that
/// round's team objects; bans without a color fall back to the index.
type BanKey = (
    Option<u32>,
    Option<usize>,
    TeamRole,
    Option<u32>,
    Option<u64>,
    bool,
);

fn ban_key(b: &Ban) -> BanKey {
    let team = if b.color.is_some() { None } else { b.team };
    (b.color, team, b.role, b.slot, b.icon, b.no_ban)
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
    /// Every player seen on this team, in the order first seen (header
    /// order within a round). A player who left stays listed, and one who
    /// joined after the first round read follows those who started it, so a
    /// team can list more players than it has seats.
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
    /// Y11S3+: the clearance level (full and partial reads). Replays hold no
    /// rank or reputation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    /// `you`, `teammate` or `opponent`; absent for spectator recordings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<Relation>,
    /// `leader` or `member` of the recording player's party (full and
    /// partial reads, Y8S1+, not in custom games).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<PartyRole>,
    /// `pc`, `playstation` or `xbox` (full and partial reads, Y11S3+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
    /// `username` is a nickname the game shows in place of the player's
    /// own name.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub uses_nickname: bool,
    /// The name the game gave the player as the match ended: for players
    /// behind a nickname, and for console players. Only in recordings that
    /// reach the end of the match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renamed_to: Option<String>,
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
    /// The game ended the match without a winner (`matchresult` 7): seen
    /// once, when the server stopped a ranked match before a round began.
    Cancelled,
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
    /// neither team reached the rounds needed to win: a forfeit, which has
    /// a `winner`, or a match the game ended without one (`cancelled`).
    /// `None` when the replay cannot tell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_early: Option<bool>,
    /// The raw Y11S3+ `matchresult` value of the deciding round: the result
    /// of the team the game numbers 1 (the recorder's, in a player's own
    /// recording): 2 won, 1 lost, 7 ended with no winner.
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

        // Every player of the match as first seen: the first round's header
        // lacks whoever joins later.
        let mut seen: Vec<&Player> = Vec::new();
        for p in rounds.iter().flat_map(|r| &r.header.players) {
            if !seen.iter().any(|q| same_player(q, p)) {
                seen.push(p);
            }
        }
        let teams = [0, 1].map(|t| TeamSummary {
            name: first.teams[t].name.clone(),
            score: 0,
            starting_side: first.teams[t].role,
            players: seen
                .iter()
                .filter(|p| p.team_index == t)
                .map(|p| PlayerSummary {
                    username: p.username.clone(),
                    key: p.key.clone(),
                    profile_id: p.profile_id.clone(),
                    level: rounds.iter().find_map(|r| player_in(r, p)?.level),
                    relation: p.relation,
                    party: rounds.iter().find_map(|r| player_in(r, p)?.party),
                    platform: rounds.iter().find_map(|r| player_in(r, p)?.platform),
                    uses_nickname: rounds
                        .iter()
                        .any(|r| player_in(r, p).is_some_and(|q| q.uses_nickname)),
                    renamed_to: (rounds.iter().rev())
                        .find_map(|r| player_in(r, p)?.renamed_to.clone()),
                })
                .collect(),
        });

        let round_summaries: Vec<RoundSummary> =
            rounds.iter().map(|r| round_summary(r, &rules)).collect();
        let final_score = round_summaries.last().map_or([0; 2], |r| r.score_after);

        let deciding = rounds.iter().find(|r| r.header.match_result.is_some());
        let raw_match_result = deciding.and_then(|r| r.header.match_result);
        let mut winner = rules.winner(final_score);
        let draw = winner.is_none() && rules.is_draw(final_score);
        let ended_early = match raw_match_result {
            None => None,
            Some(_) if winner.is_some() || draw => Some(false),
            // Neither side reached the target, yet the game ended the match:
            // `matchresult` is the result of the team it numbers 1.
            Some(value) => {
                // In a player's recording, the team numbered 1 is theirs.
                let first = deciding
                    .and_then(|r| r.header.team_of_color(1))
                    .or(your_team);
                winner = match value {
                    2 => first,
                    1 => first.map(|t| t ^ 1),
                    _ => None,
                };
                Some(true)
            }
        };
        let cancelled = ended_early == Some(true) && raw_match_result == Some(7);
        let complete = winner.is_some() || draw || cancelled;
        let outcome = match (winner, your_team) {
            _ if cancelled => Outcome::Cancelled,
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
            playlist: playlist_name(first.playlist_category),
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
            bans: ban_decisions(rounds),
            rounds: round_summaries,
        })
    }
}

/// Whether two rounds' entries are one player: by key (the profile id when
/// the replay has one), else by `playerid`. A player who reconnects comes
/// back under a new `playerid` and the same profile id.
fn same_player(a: &Player, b: &Player) -> bool {
    (!a.key.is_empty() && a.key == b.key) || (a.id != 0 && a.id == b.id)
}

/// `player` as `round` lists them.
fn player_in<'a>(round: &'a Round, player: &Player) -> Option<&'a Player> {
    let players = &round.header.players;
    players.iter().find(|q| same_player(q, player))
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
    match match_type.name() {
        Some("QuickMatch") => "quickMatch",
        Some("Ranked") => "ranked",
        Some("CustomGameLocal" | "CustomGameOnline") => "custom",
        Some("Standard") => "standard",
        Some("Unranked") => "unranked",
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

    /// A defending team's ban in `slot`, of the operator with this icon.
    fn ban(icon: u64, slot: u32) -> Ban {
        Ban {
            operator: Operator::from_role_image(icon),
            role: TeamRole::Defense,
            team: Some(0),
            icon: Some(icon),
            slot: Some(slot),
            no_ban: false,
            color: Some(1),
        }
    }

    /// Rounds 1, 2, ... with these bans in force.
    fn rounds_with_bans(bans: &[Vec<Ban>]) -> Vec<Round> {
        bans.iter()
            .enumerate()
            .map(|(i, bans)| {
                let mut r = Round::default();
                r.header.round_number = i as u32;
                r.bans = bans.clone();
                r
            })
            .collect()
    }

    /// Round `number` of a ranked match (6 rounds and 3 overtime), recorded
    /// by a player of `your_team`, going from `before` to `after`.
    fn ranked_round(
        number: u32,
        your_team: usize,
        before: [u32; 2],
        after: [u32; 2],
        match_result: Option<u32>,
    ) -> Round {
        let mut r = Round::default();
        let h = &mut r.header;
        h.code_version = crate::types::version::Y9S4;
        h.rounds_per_match = 6;
        h.rounds_per_match_overtime = 3;
        h.round_number = number - 1;
        h.recording_player_id = 7;
        h.players.push(Player {
            id: 7,
            username: "recorder".into(),
            team_index: your_team,
            ..Player::default()
        });
        for t in 0..2 {
            h.teams[t].starting_score = before[t];
            h.teams[t].score = after[t];
        }
        h.match_result = match_result;
        r
    }

    /// A player as a round's header lists them, with the key parsing gives.
    fn player(id: u64, profile: &str, team: usize) -> Player {
        Player {
            id,
            username: format!("name-{profile}"),
            profile_id: profile.into(),
            key: profile.into(),
            team_index: team,
            ..Player::default()
        }
    }

    /// Who the summary lists on `team`, by key.
    fn listed(rounds: &[Round], team: usize) -> Vec<String> {
        let s = MatchSummary::new(rounds).unwrap();
        s.teams[team]
            .players
            .iter()
            .map(|p| p.key.clone())
            .collect()
    }

    #[test]
    fn players_who_join_or_leave_between_rounds_are_all_listed() {
        let mut rounds = [
            ranked_round(1, 0, [0, 0], [1, 0], None),
            ranked_round(2, 0, [1, 0], [1, 1], None),
        ];
        // `left` plays round 1 only; `joined` takes the seat in round 2.
        rounds[0].header.players = vec![player(7, "you", 0), player(8, "left", 1)];
        rounds[1].header.players = vec![player(9, "joined", 1), player(7, "you", 0)];

        assert_eq!(listed(&rounds, 0), ["you"]);
        assert_eq!(listed(&rounds, 1), ["left", "joined"]);
    }

    #[test]
    fn a_player_who_reconnects_under_a_new_id_is_listed_once() {
        let mut rounds = [
            ranked_round(1, 0, [0, 0], [1, 0], None),
            ranked_round(2, 0, [1, 0], [1, 1], None),
        ];
        rounds[0].header.players = vec![player(7, "you", 0), player(8, "mate", 0)];
        rounds[1].header.players = vec![player(7, "you", 0), player(31, "mate", 0)];
        rounds[1].header.players[1].party = Some(PartyRole::Member);

        let s = MatchSummary::new(&rounds).unwrap();

        let keys: Vec<_> = s.teams[0].players.iter().map(|p| &p.key).collect();
        assert_eq!(keys, ["you", "mate"]);
        assert_eq!(s.teams[0].players[1].party, Some(PartyRole::Member));
    }

    #[test]
    fn players_without_an_id_are_told_apart_by_key() {
        // Header-only reads leave players the header gives no id at 0.
        let mut rounds = [ranked_round(1, 0, [0, 0], [1, 0], None)];
        rounds[0].header.players = vec![player(7, "you", 0), player(0, "a", 0), player(0, "b", 0)];
        rounds[0].header.players[2].level = Some(50);

        let s = MatchSummary::new(&rounds).unwrap();

        let levels: Vec<_> = s.teams[0].players.iter().map(|p| p.level).collect();
        assert_eq!(levels, [None, None, Some(50)]);
    }

    #[test]
    fn match_type_seven_queues_as_unranked() {
        assert_eq!(queue(MatchType::new(7, 9_901_603)), "unranked");
        assert_eq!(queue(MatchType::new(4, 9_901_603)), "custom");
    }

    #[test]
    fn a_match_ended_early_goes_to_the_team_matchresult_names() {
        // The recorder's team (1) trails 1-2, and the game says it won.
        let rounds = [ranked_round(3, 1, [1, 1], [1, 2], Some(2))];

        let r = MatchSummary::new(&rounds).unwrap().result;

        assert_eq!(
            (r.winner, r.outcome, r.ended_early),
            (Some(1), Outcome::Win, Some(true))
        );
    }

    #[test]
    fn an_early_end_goes_to_your_team_when_the_last_header_lacks_you() {
        let mut last = ranked_round(3, 1, [1, 1], [1, 2], Some(2));
        last.header.players.clear();
        let rounds = [ranked_round(2, 1, [1, 0], [1, 1], None), last];

        let r = MatchSummary::new(&rounds).unwrap().result;

        assert_eq!((r.winner, r.outcome), (Some(1), Outcome::Win));
    }

    #[test]
    fn matchresult_seven_is_a_match_ended_without_a_winner() {
        let rounds = [ranked_round(4, 0, [1, 2], [1, 2], Some(7))];

        let r = MatchSummary::new(&rounds).unwrap().result;

        assert_eq!(
            (r.winner, r.outcome, r.ended_early, r.complete),
            (None, Outcome::Cancelled, Some(true), true)
        );
    }

    #[test]
    fn each_ban_is_listed_once_with_the_first_round_it_applied_to() {
        let (mira, kaid) = (ban(39149215445, 0), ban(161289666176, 1));
        let rounds = rounds_with_bans(&[
            vec![mira.clone()],
            vec![mira.clone(), kaid.clone()],
            vec![mira, kaid],
        ]);

        let s = MatchSummary::new(&rounds).unwrap();

        let bans: Vec<_> = s.bans.iter().map(|d| (d.round, d.ban.slot)).collect();
        assert_eq!(bans, [(1, Some(0)), (2, Some(1))]);
    }

    #[test]
    fn a_ban_whose_team_a_later_round_cannot_tell_is_listed_once() {
        let mira = ban(39149215445, 0);
        let mut team_unknown = mira.clone();
        team_unknown.team = None;
        let rounds = rounds_with_bans(&[vec![team_unknown], vec![mira]]);

        let s = MatchSummary::new(&rounds).unwrap();

        let bans: Vec<_> = s.bans.iter().map(|d| (d.round, d.ban.team)).collect();
        assert_eq!(bans, [(1, Some(0))]);
    }

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
