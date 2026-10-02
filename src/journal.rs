//! The app's own data: play sessions and breaks cut from the matches of a
//! replay folder, a journal of tags, notes and goals the player writes, and
//! what the two say together about form and tilt.
//!
//! Three kinds of data, kept apart:
//!
//! - **Derived from replays**: [`MatchRecord`], [`PlaySession`], [`Break`],
//!   [`Insights`], [`TiltSignal`], [`GoalProgress`]. None of it is stored as
//!   truth: the same replays and the same rules give the same values again.
//!   The game keeps only the latest matches, so an app that wants history
//!   stores the [`MatchRecord`]s (they deserialize) and derives the rest.
//! - **Owned by the player**: [`Journal`], one JSON file whose path the
//!   caller chooses. It refers to matches by `matchID` and never copies
//!   replay data.
//! - **Rules**: [`SessionRules`], [`InsightRules`], [`TiltRules`]: the
//!   thresholds, all configurable.
//!
//! The insights are descriptive statistics of one player's matches. They are
//! correlations on small samples, not causes: "worse after a loss" can as
//! well be a stronger lobby, a late hour or chance. Every group carries its
//! sample size and the counts behind it, and holds no rate or effect below
//! the minimum sample of [`InsightRules`].
//!
//! Nothing here reads the clock: functions that stamp a time take `now`.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use chrono::{DateTime, Duration, NaiveDateTime, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::library::Library;
use crate::matches::{Match, find_match_folders};
use crate::round::{ReadMode, Round};
use crate::summary::{MatchSummary, Outcome};
use crate::types::TeamRole;

/// Timestamps as `2026-01-31T20:15:00.000Z`: chrono's own serde support is
/// a feature this crate does not enable.
mod ts {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    pub const FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";

    pub fn serialize<S: Serializer>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&t.format(FORMAT))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        let text = String::deserialize(d)?;
        DateTime::parse_from_rfc3339(&text)
            .map(|t| t.with_timezone(&Utc))
            .map_err(serde::de::Error::custom)
    }
}

mod ts_opt {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(t: &Option<DateTime<Utc>>, s: S) -> Result<S::Ok, S::Error> {
        match t {
            Some(t) => super::ts::serialize(t, s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DateTime<Utc>>, D::Error> {
        let Some(text) = Option::<String>::deserialize(d)? else {
            return Ok(None);
        };
        DateTime::parse_from_rfc3339(&text)
            .map(|t| Some(t.with_timezone(&Utc)))
            .map_err(serde::de::Error::custom)
    }
}

/// `now` as the file stores it, so a saved and reloaded item compares equal.
fn stamp(now: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(now.timestamp_millis()).unwrap_or(now)
}

fn ratio(a: u32, b: u32) -> Option<f64> {
    (b > 0).then(|| f64::from(a) / f64::from(b))
}

// ---------------------------------------------------------------------------
// Matches as the journal sees them
// ---------------------------------------------------------------------------

/// A match's result from the recording player's side: [`Outcome`], in a form
/// that can be read back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchOutcome {
    Win,
    Loss,
    Draw,
    /// Decided, but the recording player was on neither team.
    Decided,
    /// The game ended the match without a winner.
    Cancelled,
    /// Not decided in the rounds read.
    #[default]
    Unfinished,
}

impl From<Outcome> for MatchOutcome {
    fn from(o: Outcome) -> Self {
        match o {
            Outcome::Win => Self::Win,
            Outcome::Loss => Self::Loss,
            Outcome::Draw => Self::Draw,
            Outcome::Decided => Self::Decided,
            Outcome::Cancelled => Self::Cancelled,
            Outcome::Unfinished => Self::Unfinished,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    Attack,
    Defense,
}

impl From<TeamRole> for Side {
    fn from(r: TeamRole) -> Self {
        match r {
            TeamRole::Attack => Self::Attack,
            TeamRole::Defense => Self::Defense,
        }
    }
}

/// The run of the game that recorded a match, from the folder name and the
/// recording counter (see [`crate::library::Session`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameLaunch {
    /// Windows process id of the game.
    pub process_id: u32,
    /// `recordingId` of the match's first round read. The counter starts at
    /// 0 with the game, so within one run it only goes up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_recording_id: Option<u32>,
}

impl GameLaunch {
    /// Whether `next`, a later match, was recorded by another run of the
    /// game: another process, or the same process id with the counter
    /// started over (Windows reuses process ids).
    fn differs(&self, next: &GameLaunch) -> bool {
        self.process_id != next.process_id
            || matches!(
                (self.first_recording_id, next.first_recording_id),
                (Some(a), Some(b)) if b < a
            )
    }
}

/// One player's numbers over a match.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerLine {
    pub username: String,
    /// Rounds read with this player in them.
    pub rounds: u32,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub headshots: u32,
    /// Y11S3+, an estimate: see [`crate::stats::PlayerRoundStats`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage_dealt: Option<u32>,
    pub damage_taken: u32,
    /// Rounds in which the player made the first kill.
    pub opening_kills: u32,
    /// Rounds in which the player was the first to die.
    pub opening_deaths: u32,
}

impl PlayerLine {
    /// Kills per death; with no deaths, the kills.
    pub fn kill_death_ratio(&self) -> f64 {
        f64::from(self.kills) / f64::from(self.deaths.max(1))
    }

    pub fn kills_per_round(&self) -> Option<f64> {
        ratio(self.kills, self.rounds)
    }
}

/// One player's numbers in a round.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundPlayerLine {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub operator: String,
    pub kills: u32,
    pub died: bool,
    pub assists: u32,
    pub headshots: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage_dealt: Option<u32>,
    pub damage_taken: u32,
    /// The player made the round's first kill.
    pub opening_kill: bool,
    /// The player was the first to die in the round.
    pub opening_death: bool,
}

/// One round of a match, from the followed player's side.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundLine {
    /// Counting from 1, as the file names do.
    pub number: u32,
    /// Whether the player's team won the round. `None` without a team or a
    /// known winner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub won: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    /// Absent on header-only reads and when the player was not in the round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<RoundPlayerLine>,
}

/// What sessions, goals and insights need of one match. Built from replays
/// and nothing else, small, and readable back from JSON: the unit an app
/// keeps once the game has deleted the replays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchRecord {
    /// Empty in replays that predate match ids; see [`MatchRecord::key`].
    #[serde(rename = "matchID")]
    pub match_id: String,
    /// When the first round read started recording, UTC. That is after the
    /// queue, the map load and the ban and pick phases.
    #[serde(with = "ts")]
    pub started: DateTime<Utc>,
    /// When the last round read stopped, UTC (Y11S3+).
    #[serde(default, with = "ts_opt", skip_serializing_if = "Option::is_none")]
    pub ended: Option<DateTime<Utc>>,
    /// `started` is the recording PC's local time, not UTC (before Y11S3).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub started_is_local: bool,
    /// The recording PC's offset from UTC in minutes (Y11S3+).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utc_offset_minutes: Option<i32>,
    pub queue: String,
    pub map: String,
    pub outcome: MatchOutcome,
    /// Rounds won by the player's team and by the other; in team order
    /// without a team.
    pub score: [u32; 2],
    /// The match folder's name, when the game named it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<GameLaunch>,
    /// The followed player's match line: the recording player, or the one
    /// named to [`MatchRecord::from_match_as`]. Absent on header-only reads
    /// and for spectator recordings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<PlayerLine>,
    /// One per round read, in play order.
    #[serde(default)]
    pub rounds: Vec<RoundLine>,
}

impl MatchRecord {
    /// What tags, sessions and goals call this match: the `matchID`, or
    /// `start:<time>` for a replay without one.
    pub fn key(&self) -> String {
        if self.match_id.is_empty() {
            format!("start:{}", self.started.format(ts::FORMAT))
        } else {
            self.match_id.clone()
        }
    }

    /// The record of a header-only summary: times, queue, map and result,
    /// and which rounds were won. No player line.
    pub fn from_summary(summary: &MatchSummary) -> Self {
        let you = summary.your_team;
        let score = summary.result.final_score;
        Self {
            match_id: summary.match_id.clone(),
            started: summary.start_time,
            ended: summary.end_time,
            started_is_local: summary.start_time_is_local,
            utc_offset_minutes: None,
            queue: summary.queue.to_owned(),
            map: summary.map.name.clone(),
            outcome: summary.result.outcome.into(),
            score: match you {
                Some(1) => [score[1], score[0]],
                _ => score,
            },
            folder: None,
            launch: None,
            player: None,
            rounds: summary
                .rounds
                .iter()
                .map(|r| RoundLine {
                    number: r.number,
                    won: you.and_then(|y| r.winner.map(|w| w == y)),
                    side: you.and_then(|y| r.sides[y]).map(Side::from),
                    player: None,
                })
                .collect(),
        }
    }

    /// The record of a match folder, following the recording player. With
    /// `stats`, which needs rounds read in full ([`ReadMode::Full`]), it
    /// holds that player's lines. `None` when the match has no rounds.
    pub fn from_match(m: &Match, stats: bool) -> Option<Self> {
        let mut record = Self::base(m)?;
        if stats {
            let names: Vec<Option<String>> = (m.rounds.iter())
                .map(|r| r.header.recording_player().map(|p| p.username.clone()))
                .collect();
            let spectator = m.rounds.first()?.header.is_spectator == Some(true);
            if !spectator {
                record.fill(&m.rounds, &names);
            }
        }
        Some(record)
    }

    /// The record of a match seen from `username`'s side, whoever recorded
    /// it: for spectator recordings and teammates' files. Result, score and
    /// rounds won are those of that player's team. Needs rounds read in
    /// full. `None` when the player is in no round.
    pub fn from_match_as(m: &Match, username: &str) -> Option<Self> {
        let mut record = Self::base(m)?;
        let summary = m.summary()?;
        let team = (m.rounds.iter())
            .find_map(|r| r.header.players.iter().find(|p| p.username == username))
            .map(|p| p.team_index)
            .filter(|&t| t < 2)?;
        let result = &summary.result;
        record.outcome = match (result.outcome, result.winner) {
            (Outcome::Cancelled, _) => MatchOutcome::Cancelled,
            (Outcome::Draw, _) => MatchOutcome::Draw,
            (_, Some(w)) if w == team => MatchOutcome::Win,
            (_, Some(_)) => MatchOutcome::Loss,
            (_, None) => MatchOutcome::Unfinished,
        };
        let score = result.final_score;
        record.score = [score[team], score[team ^ 1]];
        for (line, r) in record.rounds.iter_mut().zip(&summary.rounds) {
            line.won = r.winner.map(|w| w == team);
            line.side = r.sides[team].map(Side::from);
        }
        let names = vec![Some(username.to_owned()); m.rounds.len()];
        record.fill(&m.rounds, &names);
        Some(record)
    }

    /// Everything but the player's lines.
    fn base(m: &Match) -> Option<Self> {
        let mut record = Self::from_summary(&m.summary()?);
        let first = m.rounds.first()?;
        record.utc_offset_minutes = first.header.start_time.map(|start| {
            let minutes = (first.header.timestamp - start).num_seconds() as f64 / 60.0;
            // Time zones are whole quarter hours.
            (minutes / 15.0).round() as i32 * 15
        });
        if let Some(folder) = &m.folder {
            let name = Path::new(&folder.path).file_name();
            record.folder = (folder.name.as_ref())
                .and(name)
                .map(|n| n.to_string_lossy().into_owned());
            record.launch = folder.name.as_ref().map(|n| GameLaunch {
                process_id: n.process_id,
                first_recording_id: (m.rounds.iter())
                    .filter_map(|r| r.container.as_ref()?.recording_id)
                    .min(),
            });
        }
        Some(record)
    }

    /// Adds the lines of the player `names` gives for each round.
    fn fill(&mut self, rounds: &[Round], names: &[Option<String>]) {
        let mut total = PlayerLine::default();
        for ((line, round), name) in self.rounds.iter_mut().zip(rounds).zip(names) {
            let Some(name) = name.as_deref() else {
                continue;
            };
            let stats = round.player_stats();
            let Some(s) = stats.iter().find(|s| s.username == name) else {
                continue;
            };
            let opening_kill = (round.opening_kill()).is_some_and(|k| {
                let by = if k.credited_to.is_empty() {
                    &k.username
                } else {
                    &k.credited_to
                };
                by == name
            });
            let opening_death = round.opening_death().and_then(|d| d.victim()) == Some(name);
            total.username = name.to_owned();
            total.rounds += 1;
            total.kills += s.kills;
            total.deaths += u32::from(s.died);
            total.assists += s.assists;
            total.headshots += s.headshots;
            total.damage_taken += s.damage_taken;
            if let Some(d) = s.damage_dealt {
                *total.damage_dealt.get_or_insert(0) += d;
            }
            total.opening_kills += u32::from(opening_kill);
            total.opening_deaths += u32::from(opening_death);
            line.player = Some(RoundPlayerLine {
                operator: s.operator.clone(),
                kills: s.kills,
                died: s.died,
                assists: s.assists,
                headshots: s.headshots,
                damage_dealt: s.damage_dealt,
                damage_taken: s.damage_taken,
                opening_kill,
                opening_death,
            });
        }
        self.player = (total.rounds > 0).then_some(total);
    }

    /// When the match started on the recording PC's clock. `None` when the
    /// replay holds no UTC offset.
    pub fn local_start(&self) -> Option<NaiveDateTime> {
        if self.started_is_local {
            return Some(self.started.naive_utc());
        }
        let offset = Duration::minutes(i64::from(self.utc_offset_minutes?));
        Some((self.started + offset).naive_utc())
    }
}

/// The records of a scanned library: no player lines, since a library keeps
/// summaries only. Folders that could not be read are left out.
pub fn records_from_library(library: &Library) -> Vec<MatchRecord> {
    let mut out = Vec::new();
    for f in &library.folders {
        let Some(summary) = &f.summary else { continue };
        let mut record = MatchRecord::from_summary(summary);
        let first = f.round_list.first();
        record.utc_offset_minutes = first.and_then(|r| {
            let start = DateTime::parse_from_rfc3339(r.start_time.as_deref()?).ok()?;
            let local = NaiveDateTime::parse_from_str(&r.local_time, "%Y-%m-%dT%H:%M:%S").ok()?;
            let minutes = (local - start.naive_utc()).num_seconds() as f64 / 60.0;
            Some((minutes / 15.0).round() as i32 * 15)
        });
        if let Some(name) = &f.folder.name {
            record.folder =
                (Path::new(&f.folder.path).file_name()).map(|n| n.to_string_lossy().into_owned());
            record.launch = Some(GameLaunch {
                process_id: name.process_id,
                first_recording_id: f.round_list.iter().filter_map(|r| r.recording_id).min(),
            });
        }
        out.push(record);
    }
    out
}

/// Reads every match folder under `root` into records, oldest first. With
/// [`ReadMode::Full`] the records hold the recording player's lines; with
/// [`ReadMode::Header`] they hold times and results only, read in a fraction
/// of the time. Folders that cannot be read are skipped. Fails only when
/// there is no match folder.
pub fn read_folder(root: &Path, mode: ReadMode) -> crate::error::Result<Vec<MatchRecord>> {
    let dirs = find_match_folders(root)?;
    if dirs.is_empty() {
        return Err(crate::error::Error::InvalidFolder);
    }
    let mut out: Vec<MatchRecord> = dirs
        .iter()
        .filter_map(|dir| Match::open_with(dir, mode).ok())
        .filter_map(|m| MatchRecord::from_match(&m, mode == ReadMode::Full))
        .collect();
    out.sort_by_key(|a| (a.started, a.key()));
    Ok(out)
}

/// One record per match, the fuller one of two of the same match, oldest
/// first.
fn ordered(records: &[MatchRecord]) -> Vec<&MatchRecord> {
    let detail = |r: &MatchRecord| (r.player.is_some(), r.rounds.len(), r.ended.is_some());
    let mut best: Vec<&MatchRecord> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for r in records {
        match index.get(&r.key()) {
            Some(&i) => {
                if detail(r) > detail(best[i]) {
                    best[i] = r;
                }
            }
            None => {
                index.insert(r.key(), best.len());
                best.push(r);
            }
        }
    }
    best.sort_by_key(|a| (a.started, a.key()));
    best
}

// ---------------------------------------------------------------------------
// Totals
// ---------------------------------------------------------------------------

/// A number a goal or an insight can be about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stat {
    /// Sums over the matches in scope.
    Kills,
    Deaths,
    Assists,
    Headshots,
    /// Kills per death; with no deaths, the kills.
    KillDeathRatio,
    KillsPerRound,
    DeathsPerRound,
    /// Share of rounds survived, 0 to 1.
    SurvivalRate,
    /// Headshot kills per kill, 0 to 100 as elsewhere in the crate.
    HeadshotPercentage,
    /// Y11S3+, an estimate.
    DamagePerRound,
    /// Wins per decided match, 0 to 1. A draw is decided and not a win.
    WinRate,
    /// Rounds won per round with a winner, 0 to 1.
    RoundWinRate,
    /// Share of rounds with the first kill, 0 to 1.
    OpeningKillRate,
    /// Share of rounds as the first to die, 0 to 1.
    OpeningDeathRate,
}

/// Counts over a set of matches: the numbers behind every rate here.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub matches: u32,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
    /// Matches with a player line.
    pub matches_with_stats: u32,
    /// Rounds of those matches the player was in.
    pub rounds: u32,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub headshots: u32,
    pub opening_kills: u32,
    pub opening_deaths: u32,
    pub damage_dealt: u64,
    /// Rounds `damage_dealt` covers.
    pub damage_rounds: u32,
    /// Rounds with a known winner, and those the player's team won.
    pub rounds_decided: u32,
    pub rounds_won: u32,
}

impl Totals {
    pub fn of<'a>(records: impl IntoIterator<Item = &'a MatchRecord>) -> Self {
        let mut t = Self::default();
        for r in records {
            t.add(r);
        }
        t
    }

    pub fn add(&mut self, r: &MatchRecord) {
        self.matches += 1;
        self.add_outcome(r.outcome);
        if let Some(p) = &r.player {
            self.add_line(p);
        }
        for round in &r.rounds {
            if let Some(won) = round.won {
                self.rounds_decided += 1;
                self.rounds_won += u32::from(won);
            }
        }
    }

    fn add_outcome(&mut self, outcome: MatchOutcome) {
        match outcome {
            MatchOutcome::Win => self.wins += 1,
            MatchOutcome::Loss => self.losses += 1,
            MatchOutcome::Draw => self.draws += 1,
            _ => {}
        }
    }

    fn add_line(&mut self, p: &PlayerLine) {
        self.matches_with_stats += 1;
        self.rounds += p.rounds;
        self.kills += p.kills;
        self.deaths += p.deaths;
        self.assists += p.assists;
        self.headshots += p.headshots;
        self.opening_kills += p.opening_kills;
        self.opening_deaths += p.opening_deaths;
        if let Some(d) = p.damage_dealt {
            self.damage_dealt += u64::from(d);
            self.damage_rounds += p.rounds;
        }
    }

    /// Matches won, lost or drawn.
    pub fn decided(&self) -> u32 {
        self.wins + self.losses + self.draws
    }

    /// The stat over these matches. `None` when they hold nothing to work
    /// it out from.
    pub fn value(&self, stat: Stat) -> Option<f64> {
        let line = |v: u32| (self.matches_with_stats > 0).then_some(f64::from(v));
        match stat {
            Stat::Kills => line(self.kills),
            Stat::Deaths => line(self.deaths),
            Stat::Assists => line(self.assists),
            Stat::Headshots => line(self.headshots),
            Stat::KillDeathRatio => {
                (self.rounds > 0).then(|| f64::from(self.kills) / f64::from(self.deaths.max(1)))
            }
            Stat::KillsPerRound => ratio(self.kills, self.rounds),
            Stat::DeathsPerRound => ratio(self.deaths, self.rounds),
            Stat::SurvivalRate => ratio(self.deaths, self.rounds).map(|d| 1.0 - d),
            Stat::HeadshotPercentage => ratio(self.headshots, self.kills).map(|h| h * 100.0),
            Stat::DamagePerRound => (self.damage_rounds > 0)
                .then(|| self.damage_dealt as f64 / f64::from(self.damage_rounds)),
            Stat::WinRate => ratio(self.wins, self.decided()),
            Stat::RoundWinRate => ratio(self.rounds_won, self.rounds_decided),
            Stat::OpeningKillRate => ratio(self.opening_kills, self.rounds),
            Stat::OpeningDeathRate => ratio(self.opening_deaths, self.rounds),
        }
    }
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// Where to cut play sessions and what counts as a break. The gap measured
/// is from the end of one match's last round to the start of the next
/// match's first round, so it always holds the end screens, the queue, the
/// map load and the ban and pick phases.
///
/// The defaults come from a real folder of 30 matches over 8 evenings: the
/// 22 gaps between matches of one evening ran from 2.3 to 14.3 minutes
/// (median 3.6, 20 of them at most 6.0, then 7.8 and 14.3), and the 7 gaps
/// between evenings were all over 21 hours, with nothing in between.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionRules {
    /// A gap of at least this long inside a session is a break; a shorter
    /// one is queue time. Default 600: well clear of the 6 minutes a normal
    /// requeue takes at most.
    pub break_seconds: i64,
    /// A gap of at least this long starts a new session. Default 3600. The
    /// data seen has no gap between 15 minutes and 21 hours, so this is a
    /// convention: a pause for a meal is a break, an hour away is a new
    /// sitting.
    pub session_gap_seconds: i64,
    /// Start a new session whenever the game was restarted, however short
    /// the gap. Default false: a crash or an update restart costs minutes
    /// and the player has not left. Restarts are listed in
    /// [`PlaySession::launches`] either way.
    pub split_on_launch: bool,
}

impl Default for SessionRules {
    fn default() -> Self {
        Self {
            break_seconds: 600,
            session_gap_seconds: 3600,
            split_on_launch: false,
        }
    }
}

/// A pause between two matches of a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Break {
    /// Key of the match before the break and of the one after.
    pub after: String,
    pub before: String,
    #[serde(with = "ts")]
    pub from: DateTime<Utc>,
    #[serde(with = "ts")]
    pub to: DateTime<Utc>,
    pub seconds: i64,
}

/// One match of a session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMatch {
    /// [`MatchRecord::key`].
    #[serde(rename = "matchID")]
    pub match_id: String,
    /// Place in the session, from 1.
    pub position: u32,
    #[serde(with = "ts")]
    pub started: DateTime<Utc>,
    #[serde(default, with = "ts_opt", skip_serializing_if = "Option::is_none")]
    pub ended: Option<DateTime<Utc>>,
    /// Seconds since the matches before it ended; 0 when they overlap.
    /// Absent for the first match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_seconds: Option<i64>,
    /// That gap was a break.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub after_break: bool,
    pub queue: String,
    pub map: String,
    pub outcome: MatchOutcome,
    pub score: [u32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<PlayerLine>,
}

/// One run of the game inside a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionLaunch {
    pub process_id: u32,
    /// Matches of the session this run recorded.
    pub matches: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WinLoss {
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
    /// Unfinished, cancelled, or watched as a spectator.
    pub undecided: u32,
}

/// A sitting: matches played with no gap of
/// [`SessionRules::session_gap_seconds`] between them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaySession {
    /// `session:` and the key of the first match. It changes when that
    /// match is no longer among the records, so resolve stored ids with
    /// [`find_session`], which also finds a session by any match in it.
    pub id: String,
    #[serde(with = "ts")]
    pub started: DateTime<Utc>,
    /// End of the last match; its start when the replay holds no end.
    #[serde(with = "ts")]
    pub ended: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utc_offset_minutes: Option<i32>,
    pub matches: Vec<SessionMatch>,
    pub breaks: Vec<Break>,
    /// Runs of the game, in order. Matches without a game-named folder are
    /// in none.
    pub launches: Vec<SessionLaunch>,
    pub record: WinLoss,
    /// The followed player's numbers over the session.
    pub totals: Totals,
}

impl PlaySession {
    /// Seconds from the session's start, or the end of its last break, to
    /// its end.
    pub fn seconds_since_break(&self) -> i64 {
        let from = self.breaks.last().map_or(self.started, |b| b.to);
        (self.ended - from).num_seconds().max(0)
    }
}

/// Cuts matches into sessions, oldest first. Records may come in any order;
/// two of the same match count once. Every match lands in exactly one
/// session, and a break never overlaps a match: it runs from the latest end
/// of the matches before it to the start of the next.
pub fn sessions(records: &[MatchRecord], rules: &SessionRules) -> Vec<PlaySession> {
    let mut out: Vec<PlaySession> = Vec::new();
    // The launch of the last match that had one.
    let mut launch: Option<GameLaunch> = None;
    for r in ordered(records) {
        let end = r.ended.unwrap_or(r.started).max(r.started);
        let relaunched = match (&launch, &r.launch) {
            (Some(a), Some(b)) => a.differs(b),
            _ => false,
        };
        let gap = (out.last()).map(|s| (r.started - s.ended).num_seconds().max(0));
        let split = gap.is_none_or(|g| {
            g >= rules.session_gap_seconds || (rules.split_on_launch && relaunched)
        });
        if split {
            out.push(PlaySession {
                id: format!("session:{}", r.key()),
                started: r.started,
                ended: end,
                utc_offset_minutes: r.utc_offset_minutes,
                matches: Vec::new(),
                breaks: Vec::new(),
                launches: Vec::new(),
                record: WinLoss::default(),
                totals: Totals::default(),
            });
        }
        let s = out.last_mut().expect("pushed above");
        let gap = gap.filter(|_| !split);
        let after_break = gap.is_some_and(|g| g >= rules.break_seconds);
        if let (true, Some(seconds), Some(prev)) = (after_break, gap, s.matches.last()) {
            s.breaks.push(Break {
                after: prev.match_id.clone(),
                before: r.key(),
                from: s.ended,
                to: r.started,
                seconds,
            });
        }
        if let Some(l) = &r.launch {
            match s.launches.last_mut() {
                Some(last) if !relaunched && last.process_id == l.process_id => last.matches += 1,
                _ => s.launches.push(SessionLaunch {
                    process_id: l.process_id,
                    matches: 1,
                }),
            }
            launch = Some(*l);
        }
        match r.outcome {
            MatchOutcome::Win => s.record.wins += 1,
            MatchOutcome::Loss => s.record.losses += 1,
            MatchOutcome::Draw => s.record.draws += 1,
            _ => s.record.undecided += 1,
        }
        s.totals.add(r);
        s.utc_offset_minutes = s.utc_offset_minutes.or(r.utc_offset_minutes);
        s.ended = s.ended.max(end);
        s.matches.push(SessionMatch {
            match_id: r.key(),
            position: s.matches.len() as u32 + 1,
            started: r.started,
            ended: r.ended,
            gap_seconds: gap,
            after_break,
            queue: r.queue.clone(),
            map: r.map.clone(),
            outcome: r.outcome,
            score: r.score,
            player: r.player.clone(),
        });
    }
    out
}

/// The session a stored id names: the one with that id, else the one that
/// holds the match the id was made from.
pub fn find_session<'a>(sessions: &'a [PlaySession], id: &str) -> Option<&'a PlaySession> {
    let key = id.strip_prefix("session:").unwrap_or(id);
    (sessions.iter().find(|s| s.id == id)).or_else(|| {
        sessions
            .iter()
            .find(|s| s.matches.iter().any(|m| m.match_id == key))
    })
}

// ---------------------------------------------------------------------------
// Journal
// ---------------------------------------------------------------------------

/// The file format version this code writes.
pub const JOURNAL_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("journal is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// What a tag or a note is attached to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Target {
    Match {
        #[serde(rename = "matchID")]
        match_id: String,
    },
    /// `round` counts from 1, as the file names do.
    Round {
        #[serde(rename = "matchID")]
        match_id: String,
        round: u32,
    },
    /// A [`PlaySession::id`]; resolve it with [`find_session`].
    Session {
        #[serde(rename = "sessionID")]
        session_id: String,
    },
    /// A kill: its place among the round's kills and deaths, from 0, and
    /// the round clock in seconds. The time is the one to trust should a
    /// later decoder find more events in the round.
    Kill {
        #[serde(rename = "matchID")]
        match_id: String,
        round: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        time: Option<f64>,
    },
}

impl Target {
    /// The match this is about; `None` for a session.
    pub fn match_id(&self) -> Option<&str> {
        match self {
            Target::Match { match_id }
            | Target::Round { match_id, .. }
            | Target::Kill { match_id, .. } => Some(match_id),
            Target::Session { .. } => None,
        }
    }
}

type Extra = BTreeMap<String, Value>;

/// A label on a match, round, session or kill. One tag per target and
/// label: its id is made from both, so the same tag set on two devices is
/// one tag after a merge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    pub target: Target,
    /// Empty once deleted.
    pub label: String,
    #[serde(with = "ts")]
    pub created: DateTime<Utc>,
    #[serde(with = "ts")]
    pub edited: DateTime<Utc>,
    #[serde(default, with = "ts_opt", skip_serializing_if = "Option::is_none")]
    pub deleted: Option<DateTime<Utc>>,
    /// Fields a newer version wrote, kept as they are.
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub target: Target,
    /// Empty once deleted.
    pub text: String,
    #[serde(with = "ts")]
    pub created: DateTime<Utc>,
    #[serde(with = "ts")]
    pub edited: DateTime<Utc>,
    #[serde(default, with = "ts_opt", skip_serializing_if = "Option::is_none")]
    pub deleted: Option<DateTime<Utc>>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Comparison {
    AtLeast,
    AtMost,
}

/// What a goal's number is worked out over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Scope {
    /// Each match on its own.
    PerMatch,
    /// Each session's matches together.
    PerSession,
    /// The last `matches` matches together.
    Rolling { matches: u32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalMetric {
    pub stat: Stat,
    pub comparison: Comparison,
    pub target: f64,
    pub scope: Scope,
}

impl GoalMetric {
    pub fn is_met(&self, value: f64) -> bool {
        match self.comparison {
            Comparison::AtLeast => value >= self.target,
            Comparison::AtMost => value <= self.target,
        }
    }
}

/// Set by the player; [`goal_progress`] reports and never changes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoalStatus {
    #[default]
    Active,
    Achieved,
    Abandoned,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    pub id: String,
    /// Empty once deleted.
    pub text: String,
    /// Absent for a goal no number measures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<GoalMetric>,
    #[serde(with = "ts")]
    pub created: DateTime<Utc>,
    #[serde(default, with = "ts_opt", skip_serializing_if = "Option::is_none")]
    pub due: Option<DateTime<Utc>>,
    #[serde(default)]
    pub status: GoalStatus,
    #[serde(with = "ts")]
    pub edited: DateTime<Utc>,
    #[serde(default, with = "ts_opt", skip_serializing_if = "Option::is_none")]
    pub deleted: Option<DateTime<Utc>>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// What merging needs of an item.
trait Item: Clone + Serialize {
    fn id(&self) -> &str;
    fn created(&self) -> DateTime<Utc>;
    fn edited(&self) -> DateTime<Utc>;
    fn deleted(&self) -> Option<DateTime<Utc>>;

    /// When the item last changed, deletion included.
    fn changed(&self) -> DateTime<Utc> {
        self.deleted()
            .map_or(self.edited(), |d| d.max(self.edited()))
    }
}

macro_rules! item {
    ($t:ty) => {
        impl Item for $t {
            fn id(&self) -> &str {
                &self.id
            }
            fn created(&self) -> DateTime<Utc> {
                self.created
            }
            fn edited(&self) -> DateTime<Utc> {
                self.edited
            }
            fn deleted(&self) -> Option<DateTime<Utc>> {
                self.deleted
            }
        }
    };
}

item!(Tag);
item!(Note);
item!(Goal);

/// Whether `theirs` replaces `mine`: the later change wins; at the same
/// instant a deletion wins, and then the larger JSON text, so that merging
/// either way round ends the same.
fn replaces<T: Item>(theirs: &T, mine: &T) -> bool {
    let key = |i: &T| {
        let json = serde_json::to_string(i).unwrap_or_default();
        (i.changed(), i.deleted().is_some(), json)
    };
    key(theirs) > key(mine)
}

fn merge_items<T: Item>(mine: &mut Vec<T>, theirs: &[T]) {
    let mut index: HashMap<String, usize> = (mine.iter().enumerate())
        .map(|(i, item)| (item.id().to_owned(), i))
        .collect();
    for item in theirs {
        match index.get(item.id()) {
            Some(&i) => {
                if replaces(item, &mine[i]) {
                    mine[i] = item.clone();
                }
            }
            None => {
                index.insert(item.id().to_owned(), mine.len());
                mine.push(item.clone());
            }
        }
    }
    mine.sort_by(|a, b| (a.created(), a.id()).cmp(&(b.created(), b.id())));
}

/// Items this version cannot read, as a newer version wrote them: kept,
/// written back, and merged by `id` like the rest.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Unread {
    pub tags: Vec<Value>,
    pub notes: Vec<Value>,
    pub goals: Vec<Value>,
}

/// When an unread item last changed, as far as its fields say.
fn raw_changed(v: &Value) -> String {
    let field = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    field("edited").max(field("deleted"))
}

fn merge_unread(mine: &mut Vec<Value>, theirs: &[Value]) {
    for item in theirs {
        let id = item.get("id").and_then(Value::as_str);
        let same = mine.iter().position(|m| match id {
            Some(id) => m.get("id").and_then(Value::as_str) == Some(id),
            None => m == item,
        });
        match same {
            Some(i) => {
                let key = |v: &Value| (raw_changed(v), v.to_string());
                if key(item) > key(&mine[i]) {
                    mine[i] = item.clone();
                }
            }
            None => mine.push(item.clone()),
        }
    }
    mine.sort_by_key(|v| v.to_string());
}

/// The file as written: items stay raw until each is read on its own, so
/// one item of a newer version does not make the file unreadable.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct RawJournal {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    tags: Vec<Value>,
    #[serde(default)]
    notes: Vec<Value>,
    #[serde(default)]
    goals: Vec<Value>,
    #[serde(flatten)]
    extra: Extra,
}

/// Brings a file of an older version up to [`JOURNAL_VERSION`]. Version 1
/// is the first, and a file without a version is read as version 1; a later
/// format change adds its step here.
fn migrate(raw: &mut RawJournal) {
    if raw.version < 1 {
        raw.version = 1;
    }
}

fn read_items<T: for<'de> Deserialize<'de>>(raw: Vec<Value>) -> (Vec<T>, Vec<Value>) {
    let mut items = Vec::new();
    let mut unread = Vec::new();
    for v in raw {
        match T::deserialize(&v) {
            Ok(item) => items.push(item),
            Err(_) => unread.push(v),
        }
    }
    (items, unread)
}

fn write_items<T: Serialize>(items: &[T], unread: &[Value]) -> Vec<Value> {
    (items.iter())
        .filter_map(|i| serde_json::to_value(i).ok())
        .chain(unread.iter().cloned())
        .collect()
}

/// The player's own tags, notes and goals: one JSON file. Deleted items
/// stay as tombstones (`deleted` set, text emptied) so that a merge with
/// another copy does not bring them back; [`Journal::purge_deleted`] drops
/// old ones.
///
/// Reading is forward compatible: unknown fields, of the file and of every
/// item, are kept and written back, items this version cannot read are kept
/// in [`Journal::unread`], and a newer file's `version` is left as it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawJournal", into = "RawJournal")]
pub struct Journal {
    pub version: u32,
    pub tags: Vec<Tag>,
    pub notes: Vec<Note>,
    pub goals: Vec<Goal>,
    pub unread: Unread,
    /// Top-level fields a newer version wrote.
    pub extra: Extra,
}

impl Default for Journal {
    fn default() -> Self {
        Self {
            version: JOURNAL_VERSION,
            tags: Vec::new(),
            notes: Vec::new(),
            goals: Vec::new(),
            unread: Unread::default(),
            extra: Extra::new(),
        }
    }
}

impl From<RawJournal> for Journal {
    fn from(mut raw: RawJournal) -> Self {
        migrate(&mut raw);
        let (tags, unread_tags) = read_items(raw.tags);
        let (notes, unread_notes) = read_items(raw.notes);
        let (goals, unread_goals) = read_items(raw.goals);
        Self {
            version: raw.version.max(JOURNAL_VERSION),
            tags,
            notes,
            goals,
            unread: Unread {
                tags: unread_tags,
                notes: unread_notes,
                goals: unread_goals,
            },
            extra: raw.extra,
        }
    }
}

impl From<Journal> for RawJournal {
    fn from(j: Journal) -> Self {
        Self {
            version: j.version,
            tags: write_items(&j.tags, &j.unread.tags),
            notes: write_items(&j.notes, &j.unread.notes),
            goals: write_items(&j.goals, &j.unread.goals),
            extra: j.extra,
        }
    }
}

/// An id for a new item: 16 hex digits of a hash of what it is.
fn new_id(parts: &[&str]) -> String {
    let mut hash = crate::file::sha256(parts.join("\u{1f}").as_bytes());
    hash.truncate(16);
    hash
}

fn target_text(target: &Target) -> String {
    serde_json::to_string(target).unwrap_or_default()
}

impl Journal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_json(json: &str) -> Result<Self, JournalError> {
        Ok(serde_json::from_str(json)?)
    }

    pub fn to_json(&self) -> Result<String, JournalError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Reads the journal at `path`; an empty one when there is no file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, JournalError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_json(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes the journal to `path` in one step: to a temporary file next
    /// to it, flushed to disk, then renamed over it, so a crash leaves the
    /// old file or the new one and never half of either.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), JournalError> {
        use std::io::Write;
        let path = path.as_ref();
        let json = self.to_json()?;
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".{}.tmp", std::process::id()));
        let temp = path.with_file_name(name);
        let write = || -> std::io::Result<()> {
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temp, path)
        };
        write().map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            e.into()
        })
    }

    /// Takes in another copy of the journal, for sync: per item the later
    /// change wins, and a deletion is a change. Merging is the same either
    /// way round and can be repeated.
    pub fn merge(&mut self, other: &Journal) {
        self.version = self.version.max(other.version);
        merge_items(&mut self.tags, &other.tags);
        merge_items(&mut self.notes, &other.notes);
        merge_items(&mut self.goals, &other.goals);
        merge_unread(&mut self.unread.tags, &other.unread.tags);
        merge_unread(&mut self.unread.notes, &other.unread.notes);
        merge_unread(&mut self.unread.goals, &other.unread.goals);
        for (k, v) in &other.extra {
            let ours = self.extra.entry(k.clone()).or_insert_with(|| v.clone());
            let (theirs, mine) = (v.to_string(), ours.to_string());
            if theirs > mine {
                *ours = v.clone();
            }
        }
    }

    /// Drops tombstones of items deleted before `before`. Only safe once
    /// every copy that may still hold the item has merged since.
    pub fn purge_deleted(&mut self, before: DateTime<Utc>) -> usize {
        let count = self.tags.len() + self.notes.len() + self.goals.len();
        let keep = |d: Option<DateTime<Utc>>| d.is_none_or(|d| d >= before);
        self.tags.retain(|t| keep(t.deleted));
        self.notes.retain(|n| keep(n.deleted));
        self.goals.retain(|g| keep(g.deleted));
        count - (self.tags.len() + self.notes.len() + self.goals.len())
    }

    /// Tags that are not deleted.
    pub fn live_tags(&self) -> impl Iterator<Item = &Tag> {
        self.tags.iter().filter(|t| t.deleted.is_none())
    }

    pub fn live_notes(&self) -> impl Iterator<Item = &Note> {
        self.notes.iter().filter(|n| n.deleted.is_none())
    }

    pub fn live_goals(&self) -> impl Iterator<Item = &Goal> {
        self.goals.iter().filter(|g| g.deleted.is_none())
    }

    pub fn tags_on<'a>(&'a self, target: &'a Target) -> impl Iterator<Item = &'a Tag> {
        self.live_tags().filter(move |t| t.target == *target)
    }

    pub fn notes_on<'a>(&'a self, target: &'a Target) -> impl Iterator<Item = &'a Note> {
        self.live_notes().filter(move |n| n.target == *target)
    }

    /// Every label in use with the number of tags that carry it, most used
    /// first.
    pub fn labels(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for t in self.live_tags() {
            *counts.entry(&t.label).or_default() += 1;
        }
        let mut out: Vec<(String, usize)> =
            counts.into_iter().map(|(l, n)| (l.to_owned(), n)).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    /// Tags `target` with `label` and returns the tag's id. Labels are
    /// trimmed, and compared without case: tagging twice gives one tag, and
    /// tagging again after a removal brings it back.
    pub fn add_tag(&mut self, target: Target, label: &str, now: DateTime<Utc>) -> String {
        let now = stamp(now);
        let label = label.trim();
        let id = new_id(&["tag", &target_text(&target), &label.to_lowercase()]);
        match self.tags.iter_mut().find(|t| t.id == id) {
            Some(tag) if tag.deleted.is_some() => {
                tag.deleted = None;
                tag.label = label.to_owned();
                tag.edited = now.max(tag.edited);
            }
            Some(_) => {}
            None => self.tags.push(Tag {
                id: id.clone(),
                target,
                label: label.to_owned(),
                created: now,
                edited: now,
                deleted: None,
                extra: Extra::new(),
            }),
        }
        id
    }

    /// False when there is no such live tag.
    pub fn remove_tag(&mut self, id: &str, now: DateTime<Utc>) -> bool {
        let Some(tag) = self
            .tags
            .iter_mut()
            .find(|t| t.id == id && t.deleted.is_none())
        else {
            return false;
        };
        tag.deleted = Some(stamp(now).max(tag.edited));
        tag.label.clear();
        true
    }

    /// Adds a note and returns its id.
    pub fn add_note(&mut self, target: Target, text: &str, now: DateTime<Utc>) -> String {
        let now = stamp(now);
        let time = now.format(ts::FORMAT).to_string();
        let count = self.notes.len().to_string();
        let id = new_id(&["note", &target_text(&target), text, &time, &count]);
        self.notes.push(Note {
            id: id.clone(),
            target,
            text: text.to_owned(),
            created: now,
            edited: now,
            deleted: None,
            extra: Extra::new(),
        });
        id
    }

    /// False when there is no such live note.
    pub fn edit_note(&mut self, id: &str, text: &str, now: DateTime<Utc>) -> bool {
        let Some(note) = self
            .notes
            .iter_mut()
            .find(|n| n.id == id && n.deleted.is_none())
        else {
            return false;
        };
        note.text = text.to_owned();
        note.edited = stamp(now).max(note.edited);
        true
    }

    pub fn remove_note(&mut self, id: &str, now: DateTime<Utc>) -> bool {
        let Some(note) = self
            .notes
            .iter_mut()
            .find(|n| n.id == id && n.deleted.is_none())
        else {
            return false;
        };
        note.deleted = Some(stamp(now).max(note.edited));
        note.text.clear();
        true
    }

    /// Adds an active goal and returns its id.
    pub fn add_goal(
        &mut self,
        text: &str,
        metric: Option<GoalMetric>,
        due: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> String {
        let now = stamp(now);
        let time = now.format(ts::FORMAT).to_string();
        let count = self.goals.len().to_string();
        let id = new_id(&["goal", text, &time, &count]);
        self.goals.push(Goal {
            id: id.clone(),
            text: text.to_owned(),
            metric,
            created: now,
            due: due.map(stamp),
            status: GoalStatus::Active,
            edited: now,
            deleted: None,
            extra: Extra::new(),
        });
        id
    }

    /// Changes a live goal through `change` and stamps the edit. False when
    /// there is no such live goal.
    pub fn edit_goal(
        &mut self,
        id: &str,
        now: DateTime<Utc>,
        change: impl FnOnce(&mut Goal),
    ) -> bool {
        let Some(goal) = self
            .goals
            .iter_mut()
            .find(|g| g.id == id && g.deleted.is_none())
        else {
            return false;
        };
        let (id, created, edited) = (goal.id.clone(), goal.created, goal.edited);
        change(goal);
        goal.id = id;
        goal.created = created;
        goal.deleted = None;
        goal.due = goal.due.map(stamp);
        goal.edited = stamp(now).max(edited);
        true
    }

    pub fn set_goal_status(&mut self, id: &str, status: GoalStatus, now: DateTime<Utc>) -> bool {
        self.edit_goal(id, now, |g| g.status = status)
    }

    pub fn remove_goal(&mut self, id: &str, now: DateTime<Utc>) -> bool {
        let Some(goal) = self
            .goals
            .iter_mut()
            .find(|g| g.id == id && g.deleted.is_none())
        else {
            return false;
        };
        goal.deleted = Some(stamp(now).max(goal.edited));
        goal.text.clear();
        goal.metric = None;
        true
    }
}

// ---------------------------------------------------------------------------
// Goal progress
// ---------------------------------------------------------------------------

/// A goal's number over one match, one session or one rolling window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalPoint {
    /// The match's key, the session's id, or for a rolling window the key
    /// of its last match.
    pub key: String,
    /// Start of the match, or of the last match counted.
    #[serde(with = "ts")]
    pub at: DateTime<Utc>,
    pub matches: u32,
    pub value: f64,
    pub met: bool,
}

/// How a goal with a metric stands against the matches played since it was
/// set. Derived: nothing here is stored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalProgress {
    #[serde(rename = "goalID")]
    pub goal_id: String,
    /// Matches started from the goal's creation up to its due time.
    pub sample: u32,
    /// One per match, session or full window, oldest first. Matches that
    /// hold nothing to work the stat out from give no point.
    pub points: Vec<GoalPoint>,
    /// Points that met the target.
    pub met: u32,
    /// The latest value: of the last point, or, for a rolling goal still
    /// short of its window, of the matches so far.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<f64>,
    /// Whether the latest point met the target. `None` without a point, so
    /// a rolling goal claims nothing before its window is full.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_met: Option<bool>,
}

/// Evaluates `goal` against the matches started from its creation up to its
/// due time. `None` for a goal without a metric.
pub fn goal_progress(
    goal: &Goal,
    records: &[MatchRecord],
    rules: &SessionRules,
) -> Option<GoalProgress> {
    let metric = goal.metric.as_ref()?;
    let counts =
        |started: DateTime<Utc>| started >= goal.created && goal.due.is_none_or(|d| started <= d);
    let all = ordered(records);
    let eligible: Vec<&MatchRecord> = all.iter().copied().filter(|r| counts(r.started)).collect();
    let point = |key: String, of: &[&MatchRecord]| {
        let value = Totals::of(of.iter().copied()).value(metric.stat)?;
        Some(GoalPoint {
            key,
            at: of.last()?.started,
            matches: of.len() as u32,
            value,
            met: metric.is_met(value),
        })
    };
    let mut current = None;
    let points: Vec<GoalPoint> = match metric.scope {
        Scope::PerMatch => (eligible.iter())
            .filter_map(|r| point(r.key(), &[r]))
            .collect(),
        Scope::PerSession => {
            let by_key: HashMap<String, &MatchRecord> =
                eligible.iter().map(|r| (r.key(), *r)).collect();
            (sessions(records, rules).iter())
                .filter_map(|s| {
                    let of: Vec<&MatchRecord> = (s.matches.iter())
                        .filter_map(|m| by_key.get(&m.match_id).copied())
                        .collect();
                    point(s.id.clone(), &of)
                })
                .collect()
        }
        Scope::Rolling { matches } => {
            let n = (matches as usize).max(1);
            if eligible.len() < n {
                current = Totals::of(eligible.iter().copied()).value(metric.stat);
            }
            (eligible.windows(n))
                .filter_map(|w| point(w.last()?.key(), w))
                .collect()
        }
    };
    Some(GoalProgress {
        goal_id: goal.id.clone(),
        sample: eligible.len() as u32,
        met: points.iter().filter(|p| p.met).count() as u32,
        current: points.last().map(|p| p.value).or(current),
        current_met: points.last().map(|p| p.met),
        points,
    })
}

// ---------------------------------------------------------------------------
// Insights
// ---------------------------------------------------------------------------

/// What every insight says about itself.
pub const INSIGHT_CAVEAT: &str = "Descriptive statistics of your own matches: correlations on small \
     samples, not causes. Lobby strength, map, queue and time of day are not held equal.";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InsightRules {
    /// Matches a group needs before it gives a rate, and both groups of a
    /// comparison before it gives an effect. Default 10: at ten matches a
    /// win rate still has a standard error of about 16 points.
    pub min_matches: u32,
    /// The same for groups of rounds. Default 30.
    pub min_rounds: u32,
    /// Losses in a row, inside a session, that count as a loss streak.
    /// Default 2.
    pub loss_streak: u32,
    /// Rounds lost in a row, inside a match, for the round-level
    /// comparison. Default 2.
    pub round_loss_streak: u32,
    /// Positions in a session reported one by one; later ones share a
    /// group (`5+`). Default 5.
    pub positions: u32,
}

impl Default for InsightRules {
    fn default() -> Self {
        Self {
            min_matches: 10,
            min_rounds: 30,
            loss_streak: 2,
            round_loss_streak: 2,
            positions: 5,
        }
    }
}

/// Rates of a group of matches. Each is absent when fewer than
/// [`InsightRules::min_matches`] matches hold what it is made from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rates {
    /// Wins per decided match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_rate: Option<f64>,
    /// 95% Wilson interval of `win_rate`: how wide the uncertainty is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_rate_interval: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kill_death_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kills_per_round: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deaths_per_round: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headshot_percentage: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round_win_rate: Option<f64>,
}

/// A group of matches: its sample, the counts, and its rates when the
/// sample is large enough.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub label: String,
    /// Matches in the group: the sample size.
    pub sample: u32,
    /// `sample` reaches [`InsightRules::min_matches`].
    pub enough: bool,
    pub totals: Totals,
    pub rates: Rates,
    /// Kills per round of each match with a player line, for effect sizes.
    #[serde(skip)]
    per_match: Vec<f64>,
}

/// 95% Wilson score interval of `wins` in `n`.
fn wilson(wins: u32, n: u32) -> Option<[f64; 2]> {
    if n == 0 {
        return None;
    }
    let (z, n) = (1.96_f64, f64::from(n));
    let p = f64::from(wins) / n;
    let centre = p + z * z / (2.0 * n);
    let spread = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt();
    let denominator = 1.0 + z * z / n;
    Some([
        ((centre - spread) / denominator).max(0.0),
        ((centre + spread) / denominator).min(1.0),
    ])
}

impl Group {
    fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
        }
    }

    fn add(&mut self, r: &MatchRecord) {
        self.totals.add(r);
        if let Some(k) = r.player.as_ref().and_then(PlayerLine::kills_per_round) {
            self.per_match.push(k);
        }
    }

    fn finish(mut self, rules: &InsightRules) -> Self {
        self.per_match.clear();
        let t = &self.totals;
        self.sample = t.matches;
        self.enough = t.matches >= rules.min_matches;
        let results = t.decided() >= rules.min_matches;
        let stats = t.matches_with_stats >= rules.min_matches;
        let when = |ok: bool, stat: Stat| t.value(stat).filter(|_| ok);
        self.rates = Rates {
            win_rate: when(results, Stat::WinRate),
            win_rate_interval: wilson(t.wins, t.decided()).filter(|_| results),
            kill_death_ratio: when(stats, Stat::KillDeathRatio),
            kills_per_round: when(stats, Stat::KillsPerRound),
            deaths_per_round: when(stats, Stat::DeathsPerRound),
            headshot_percentage: when(stats, Stat::HeadshotPercentage),
            round_win_rate: when(self.enough, Stat::RoundWinRate),
        };
        self
    }
}

/// How far apart the two groups of a comparison are, first minus second.
/// A field is absent when either group lacks the rate.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Effect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_rate_difference: Option<f64>,
    /// Cohen's h of the two win rates: about 0.2 is small, 0.5 medium, 0.8
    /// large.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_rate_h: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kill_death_ratio_difference: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kills_per_round_difference: Option<f64>,
    /// Cohen's d of kills per round, match by match, on the same scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kills_per_round_d: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round_win_rate_difference: Option<f64>,
}

fn cohen_h(a: f64, b: f64) -> f64 {
    2.0 * a.sqrt().asin() - 2.0 * b.sqrt().asin()
}

/// Cohen's d with the pooled standard deviation. `None` under two values a
/// side or without spread.
fn cohen_d(a: &[f64], b: &[f64]) -> Option<f64> {
    if a.len() < 2 || b.len() < 2 {
        return None;
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let squares = |v: &[f64], m: f64| v.iter().map(|x| (x - m).powi(2)).sum::<f64>();
    let (ma, mb) = (mean(a), mean(b));
    let pooled = ((squares(a, ma) + squares(b, mb)) / (a.len() + b.len() - 2) as f64).sqrt();
    // Rounding alone leaves a spread of about 1e-16 between equal values.
    (pooled > 1e-9).then(|| (ma - mb) / pooled)
}

/// Two groups of matches side by side.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Compared {
    pub first: Group,
    pub second: Group,
    /// Absent unless both groups reach the minimum sample.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<Effect>,
}

impl Compared {
    fn new(first: Group, second: Group, rules: &InsightRules) -> Self {
        let d = cohen_d(&first.per_match, &second.per_match);
        let (first, second) = (first.finish(rules), second.finish(rules));
        let difference = |f: fn(&Rates) -> Option<f64>| Some(f(&first.rates)? - f(&second.rates)?);
        let effect = (first.enough && second.enough).then(|| Effect {
            win_rate_difference: difference(|r| r.win_rate),
            win_rate_h: (first.rates.win_rate)
                .zip(second.rates.win_rate)
                .map(|(a, b)| cohen_h(a, b)),
            kill_death_ratio_difference: difference(|r| r.kill_death_ratio),
            kills_per_round_difference: difference(|r| r.kills_per_round),
            kills_per_round_d: (first.rates.kills_per_round)
                .zip(second.rates.kills_per_round)
                .and(d),
            round_win_rate_difference: difference(|r| r.round_win_rate),
        });
        Self {
            first,
            second,
            effect,
        }
    }
}

/// A group of rounds: its sample, the counts, and its rates when the sample
/// reaches [`InsightRules::min_rounds`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundGroup {
    pub label: String,
    /// Rounds in the group: the sample size.
    pub sample: u32,
    pub enough: bool,
    /// Rounds with a known winner, and those won.
    pub decided: u32,
    pub won: u32,
    /// Rounds with a player line, and the kills and deaths in them.
    pub with_stats: u32,
    pub kills: u32,
    pub deaths: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_rate_interval: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kills_per_round: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub death_rate: Option<f64>,
}

impl RoundGroup {
    fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
        }
    }

    fn add(&mut self, r: &RoundLine) {
        self.sample += 1;
        if let Some(won) = r.won {
            self.decided += 1;
            self.won += u32::from(won);
        }
        if let Some(p) = &r.player {
            self.with_stats += 1;
            self.kills += p.kills;
            self.deaths += u32::from(p.died);
        }
    }

    fn finish(mut self, rules: &InsightRules) -> Self {
        self.enough = self.sample >= rules.min_rounds;
        if self.decided >= rules.min_rounds {
            self.win_rate = ratio(self.won, self.decided);
            self.win_rate_interval = wilson(self.won, self.decided);
        }
        if self.with_stats >= rules.min_rounds {
            self.kills_per_round = ratio(self.kills, self.with_stats);
            self.death_rate = ratio(self.deaths, self.with_stats);
        }
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundEffect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_rate_difference: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_rate_h: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kills_per_round_difference: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub death_rate_difference: Option<f64>,
}

/// Two groups of rounds side by side.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundsCompared {
    pub first: RoundGroup,
    pub second: RoundGroup,
    /// Absent unless both groups reach the minimum sample.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<RoundEffect>,
}

impl RoundsCompared {
    fn new(first: RoundGroup, second: RoundGroup, rules: &InsightRules) -> Self {
        let (first, second) = (first.finish(rules), second.finish(rules));
        let both = |a: Option<f64>, b: Option<f64>| a.zip(b);
        let effect = (first.enough && second.enough).then(|| RoundEffect {
            win_rate_difference: both(first.win_rate, second.win_rate).map(|(a, b)| a - b),
            win_rate_h: both(first.win_rate, second.win_rate).map(|(a, b)| cohen_h(a, b)),
            kills_per_round_difference: both(first.kills_per_round, second.kills_per_round)
                .map(|(a, b)| a - b),
            death_rate_difference: both(first.death_rate, second.death_rate).map(|(a, b)| a - b),
        });
        Self {
            first,
            second,
            effect,
        }
    }
}

/// How the followed player's results move with where a match sits in a
/// session and what came before it. See [`INSIGHT_CAVEAT`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Insights {
    pub caveat: String,
    pub rules: InsightRules,
    pub sessions: u32,
    /// Every match.
    pub overall: Group,
    /// By place in the session: `1`, `2`, ... and a last group such as `5+`.
    pub by_position: Vec<Group>,
    /// Matches right after a loss in the same session against those right
    /// after a win. A session's first match is in neither.
    pub after_loss_vs_after_win: Compared,
    /// Matches that follow [`InsightRules::loss_streak`] or more straight
    /// losses in the session against the session's other matches that have
    /// one before them.
    pub on_loss_streak: Compared,
    /// Matches that follow a break against those that follow a normal
    /// requeue.
    pub after_break_vs_without: Compared,
    /// By how many matches the whole session had: `1-2`, `3-4`, `5+`.
    pub by_session_length: Vec<Group>,
    /// By local start time: `night` (0-6), `morning` (6-12), `afternoon`
    /// (12-18), `evening` (18-24). Matches without a UTC offset are left
    /// out.
    pub by_time_of_day: Vec<Group>,
    /// Rounds right after one in which the player died first, against
    /// rounds after one in which they did not. Needs player lines.
    pub after_dying_first: RoundsCompared,
    /// Rounds that follow [`InsightRules::round_loss_streak`] or more
    /// straight lost rounds of the match, against the other rounds that
    /// have one before them.
    pub after_round_losses: RoundsCompared,
}

/// Builds every insight from `records`, cutting sessions by `sessions`.
pub fn insights(
    records: &[MatchRecord],
    sessions: &SessionRules,
    rules: &InsightRules,
) -> Insights {
    let by_key: HashMap<String, &MatchRecord> =
        ordered(records).into_iter().map(|r| (r.key(), r)).collect();
    let cut = self::sessions(records, sessions);

    let mut overall = Group::new("all");
    let positions = rules.positions.max(1);
    let mut by_position: Vec<Group> = (1..=positions)
        .map(|p| match p {
            p if p == positions => Group::new(format!("{p}+")),
            p => Group::new(p.to_string()),
        })
        .collect();
    let mut after = [Group::new("afterLoss"), Group::new("afterWin")];
    let mut streak = [Group::new("onLossStreak"), Group::new("notOnLossStreak")];
    let mut breaks = [Group::new("afterBreak"), Group::new("withoutBreak")];
    let mut by_length = ["1-2", "3-4", "5+"].map(Group::new);
    let mut by_time = ["night", "morning", "afternoon", "evening"].map(Group::new);
    let mut died_first = [
        RoundGroup::new("afterDyingFirst"),
        RoundGroup::new("afterNotDyingFirst"),
    ];
    let mut round_streak = [
        RoundGroup::new("afterRoundLosses"),
        RoundGroup::new("notAfterRoundLosses"),
    ];

    for s in &cut {
        let length = match s.matches.len() {
            0..=2 => 0,
            3..=4 => 1,
            _ => 2,
        };
        // Straight losses going into the match.
        let mut losses = 0;
        let mut previous: Option<MatchOutcome> = None;
        for m in &s.matches {
            let Some(r) = by_key.get(&m.match_id).copied() else {
                continue;
            };
            overall.add(r);
            by_position[(m.position.min(positions) - 1) as usize].add(r);
            by_length[length].add(r);
            if let Some(t) = r.local_start() {
                by_time[(t.hour() / 6) as usize].add(r);
            }
            match previous {
                Some(MatchOutcome::Loss) => after[0].add(r),
                Some(MatchOutcome::Win) => after[1].add(r),
                _ => {}
            }
            if previous.is_some() {
                let on = rules.loss_streak > 0 && losses >= rules.loss_streak;
                streak[usize::from(!on)].add(r);
                breaks[usize::from(!m.after_break)].add(r);
            }
            // An undecided match neither extends nor ends a streak.
            match m.outcome {
                MatchOutcome::Loss => losses += 1,
                MatchOutcome::Win | MatchOutcome::Draw => losses = 0,
                _ => {}
            }
            previous = Some(m.outcome);
            add_rounds(r, rules, &mut died_first, &mut round_streak);
        }
    }

    let finish = |groups: Vec<Group>| groups.into_iter().map(|g| g.finish(rules)).collect();
    let [a, b] = after;
    let [c, d] = streak;
    let [e, f] = breaks;
    let [g, h] = died_first;
    let [i, j] = round_streak;
    Insights {
        caveat: INSIGHT_CAVEAT.to_owned(),
        rules: rules.clone(),
        sessions: cut.len() as u32,
        overall: overall.finish(rules),
        by_position: finish(by_position),
        after_loss_vs_after_win: Compared::new(a, b, rules),
        on_loss_streak: Compared::new(c, d, rules),
        after_break_vs_without: Compared::new(e, f, rules),
        by_session_length: finish(by_length.into()),
        by_time_of_day: finish(by_time.into()),
        after_dying_first: RoundsCompared::new(g, h, rules),
        after_round_losses: RoundsCompared::new(i, j, rules),
    }
}

/// Sorts the rounds of one match into the round-level groups. A round
/// counts only when the round before it was read too.
fn add_rounds(
    r: &MatchRecord,
    rules: &InsightRules,
    died_first: &mut [RoundGroup; 2],
    round_streak: &mut [RoundGroup; 2],
) {
    let mut rounds: Vec<&RoundLine> = r.rounds.iter().collect();
    rounds.sort_by_key(|l| l.number);
    // Straight lost rounds going into the round.
    let mut losses = 0;
    for (i, round) in rounds.iter().enumerate() {
        let previous = (i.checked_sub(1))
            .map(|p| rounds[p])
            .filter(|p| p.number + 1 == round.number);
        let Some(previous) = previous else {
            losses = u32::from(round.won == Some(false));
            continue;
        };
        if let Some(p) = &previous.player {
            died_first[usize::from(!p.opening_death)].add(round);
        }
        if previous.won.is_some() {
            let on = rules.round_loss_streak > 0 && losses >= rules.round_loss_streak;
            round_streak[usize::from(!on)].add(round);
        }
        match round.won {
            Some(false) => losses += 1,
            _ => losses = 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Tilt
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TiltRules {
    /// Straight losses that raise the signal to `watch`. Default 2.
    pub watch_losses: u32,
    /// Straight losses that raise it to `tilted` whatever the K/D does.
    /// Default 3.
    pub tilted_losses: u32,
    /// Share by which the K/D must be lower in the streak than before it to
    /// count as falling. Default 0.2.
    pub kd_drop: f64,
    /// Seconds of play without a break after which one is suggested
    /// whatever the results. Default 7200.
    pub max_seconds_without_break: i64,
}

impl Default for TiltRules {
    fn default() -> Self {
        Self {
            watch_losses: 2,
            tilted_losses: 3,
            kd_drop: 0.2,
            max_seconds_without_break: 7200,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TiltLevel {
    #[default]
    None,
    Watch,
    Tilted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TiltReason {
    /// At least [`TiltRules::watch_losses`] straight losses.
    LossStreak,
    /// The K/D in the streak is lower than before it by
    /// [`TiltRules::kd_drop`].
    KillDeathRatioFalling,
    /// The win rate before the streak was above zero: it fell to none.
    WinRateFalling,
    /// [`TiltRules::max_seconds_without_break`] of play without a break.
    LongWithoutBreak,
}

/// Where a session in progress stands: the current run of losses since the
/// last break, the numbers before and during it, and whether a break is
/// suggested. A heuristic on a handful of matches; see [`INSIGHT_CAVEAT`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TiltSignal {
    pub level: TiltLevel,
    pub reasons: Vec<TiltReason>,
    /// Matches in the session: the sample.
    pub sample: u32,
    /// Straight losses at the end of the session, counted back to the last
    /// break. Unfinished and cancelled matches are stepped over.
    pub loss_streak: u32,
    /// The session's matches before the streak, back to the last break.
    pub before: Totals,
    /// The matches of the streak.
    pub during: Totals,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kill_death_ratio_before: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kill_death_ratio_during: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_rate_before: Option<f64>,
    pub matches_since_break: u32,
    pub seconds_since_break: i64,
    pub suggest_break: bool,
    /// How long the suggested break is: [`SessionRules::break_seconds`],
    /// the shortest gap the next match would count as coming after a break.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_break_seconds: Option<i64>,
}

/// The tilt signal of `session`, usually the latest one while it is still
/// being played.
pub fn tilt(session: &PlaySession, sessions: &SessionRules, rules: &TiltRules) -> TiltSignal {
    // The matches since the last break.
    let from = (session.matches.iter())
        .rposition(|m| m.after_break)
        .unwrap_or(0);
    let stretch = &session.matches[from..];
    let mut loss_streak = 0;
    // Index in `stretch` of the first match of the streak.
    let mut start = stretch.len();
    for (i, m) in stretch.iter().enumerate().rev() {
        match m.outcome {
            MatchOutcome::Loss => {
                loss_streak += 1;
                start = i;
            }
            MatchOutcome::Win | MatchOutcome::Draw => break,
            _ => {}
        }
    }
    let totals = |matches: &[SessionMatch]| {
        let mut t = Totals::default();
        for m in matches {
            t.matches += 1;
            t.add_outcome(m.outcome);
            if let Some(p) = &m.player {
                t.add_line(p);
            }
        }
        t
    };
    let (before, during) = (totals(&stretch[..start]), totals(&stretch[start..]));
    let kd_before = before.value(Stat::KillDeathRatio);
    let kd_during = during.value(Stat::KillDeathRatio);
    let win_rate_before = before.value(Stat::WinRate);
    let seconds_since_break = session.seconds_since_break();

    let mut reasons = Vec::new();
    let on_streak = rules.watch_losses > 0 && loss_streak >= rules.watch_losses;
    if on_streak {
        reasons.push(TiltReason::LossStreak);
        if let (Some(b), Some(d)) = (kd_before, kd_during)
            && d < b * (1.0 - rules.kd_drop)
        {
            reasons.push(TiltReason::KillDeathRatioFalling);
        }
        if win_rate_before.is_some_and(|w| w > 0.0) {
            reasons.push(TiltReason::WinRateFalling);
        }
    }
    let long = seconds_since_break >= rules.max_seconds_without_break;
    if long {
        reasons.push(TiltReason::LongWithoutBreak);
    }
    let falling = reasons.contains(&TiltReason::KillDeathRatioFalling);
    let level = match on_streak {
        true if falling || loss_streak >= rules.tilted_losses => TiltLevel::Tilted,
        true => TiltLevel::Watch,
        false => TiltLevel::None,
    };
    let suggest_break = level == TiltLevel::Tilted || long;
    TiltSignal {
        level,
        reasons,
        sample: session.matches.len() as u32,
        loss_streak,
        before,
        during,
        kill_death_ratio_before: kd_before,
        kill_death_ratio_during: kd_during,
        win_rate_before,
        matches_since_break: stretch.len() as u32,
        seconds_since_break,
        suggest_break,
        suggested_break_seconds: suggest_break.then_some(sessions.break_seconds),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap() + Duration::minutes(minutes)
    }

    /// A match `id` that starts `start` minutes in and lasts `length`.
    fn record(id: &str, start: i64, length: i64, outcome: MatchOutcome) -> MatchRecord {
        MatchRecord {
            match_id: id.to_owned(),
            started: at(start),
            ended: Some(at(start + length)),
            started_is_local: false,
            utc_offset_minutes: Some(120),
            queue: "ranked".to_owned(),
            map: "Bank".to_owned(),
            outcome,
            score: [0, 0],
            folder: None,
            launch: Some(GameLaunch {
                process_id: 100,
                first_recording_id: Some((start + 100_000) as u32),
            }),
            player: None,
            rounds: Vec::new(),
        }
    }

    /// The same with a player line of `rounds` rounds.
    fn played(mut r: MatchRecord, rounds: u32, kills: u32, deaths: u32) -> MatchRecord {
        r.player = Some(PlayerLine {
            username: "you".to_owned(),
            rounds,
            kills,
            deaths,
            ..PlayerLine::default()
        });
        r
    }

    use MatchOutcome::{Loss, Unfinished, Win};

    fn keys(s: &PlaySession) -> Vec<&str> {
        s.matches.iter().map(|m| m.match_id.as_str()).collect()
    }

    #[test]
    fn gaps_are_queue_time_a_break_or_a_new_session_at_the_thresholds() {
        let rules = SessionRules::default();
        // a ends at 20. b starts 9:59 later, c 10:00 after b, d 59:59 after
        // c, e 60:00 after d.
        let mut b = record("b", 30, 20, Loss);
        b.started = at(30) - Duration::seconds(1);
        let mut d = record("d", 140, 20, Win);
        d.started = at(140) - Duration::seconds(1);
        let records = [
            record("a", 0, 20, Win),
            b,
            record("c", 60, 20, Win),
            d,
            record("e", 220, 20, Loss),
        ];
        let cut = sessions(&records, &rules);
        assert_eq!(cut.len(), 2);
        assert_eq!(keys(&cut[0]), ["a", "b", "c", "d"]);
        assert_eq!(keys(&cut[1]), ["e"]);
        assert_eq!(cut[0].id, "session:a");
        let gaps: Vec<_> = cut[0].matches.iter().map(|m| m.gap_seconds).collect();
        assert_eq!(gaps, [None, Some(599), Some(600), Some(3599)]);
        let breaks: Vec<_> = (cut[0].breaks.iter())
            .map(|b| (b.after.as_str(), b.before.as_str(), b.seconds))
            .collect();
        assert_eq!(breaks, [("b", "c", 600), ("c", "d", 3599)]);
        assert_eq!(cut[0].breaks[0].from, at(50));
        assert_eq!(cut[0].breaks[0].to, at(60));
        assert_eq!(
            cut[0].record,
            WinLoss {
                wins: 3,
                losses: 1,
                ..WinLoss::default()
            }
        );
        assert_eq!(cut[0].matches[2].position, 3);
        assert!(cut[0].matches[2].after_break && !cut[0].matches[1].after_break);
        assert_eq!(cut[1].matches[0].gap_seconds, None);
        // Stricter rules cut the same matches finer.
        let strict = SessionRules {
            session_gap_seconds: 600,
            ..rules
        };
        assert_eq!(sessions(&records, &strict).len(), 4);
    }

    #[test]
    fn a_game_restart_is_listed_and_splits_only_when_asked() {
        let launch = |process_id, id| {
            Some(GameLaunch {
                process_id,
                first_recording_id: Some(id),
            })
        };
        let mut b = record("b", 25, 20, Win);
        b.launch = launch(200, 0);
        // The same process id with the counter started over is a new run.
        let mut c = record("c", 50, 20, Win);
        c.launch = launch(200, 22);
        let mut d = record("d", 75, 20, Win);
        d.launch = launch(200, 0);
        // A copied folder without a game name belongs to no run.
        let mut e = record("e", 100, 20, Win);
        e.launch = None;
        let mut f = record("f", 125, 20, Win);
        f.launch = launch(200, 11);
        let records = [record("a", 0, 20, Win), b, c, d, e, f];
        let cut = sessions(&records, &SessionRules::default());
        assert_eq!(cut.len(), 1);
        let launches: Vec<_> = (cut[0].launches.iter())
            .map(|l| (l.process_id, l.matches))
            .collect();
        assert_eq!(launches, [(100, 1), (200, 2), (200, 2)]);
        let split = SessionRules {
            split_on_launch: true,
            ..SessionRules::default()
        };
        let cut = sessions(&records, &split);
        let all: Vec<_> = cut.iter().map(keys).collect();
        assert_eq!(all, [vec!["a"], vec!["b", "c"], vec!["d", "e", "f"]]);
    }

    #[test]
    fn odd_timestamps_still_give_one_session_per_match() {
        // b starts before a ends and outlasts c; d has no end; a is listed
        // twice, once with a player line; records come out of order.
        let mut d = record("d", 200, 0, Unfinished);
        d.ended = None;
        let records = [
            record("c", 30, 5, Win),
            d,
            record("a", 0, 20, Win),
            record("b", 10, 60, Loss),
            played(record("a", 0, 20, Win), 5, 4, 2),
            record("e", 201, 10, Win),
        ];
        let cut = sessions(&records, &SessionRules::default());
        assert_eq!(cut.len(), 2);
        assert_eq!(keys(&cut[0]), ["a", "b", "c"]);
        assert!(cut[0].matches[0].player.is_some());
        let gaps: Vec<_> = cut[0].matches.iter().map(|m| m.gap_seconds).collect();
        assert_eq!(gaps, [None, Some(0), Some(0)]);
        assert!(cut[0].breaks.is_empty());
        // The session ends with its longest match, not its last.
        assert_eq!(cut[0].ended, at(70));
        // d starts 130 minutes after b ended: a new session. With no end it
        // counts as over when it began, so e follows a minute later.
        assert_eq!(keys(&cut[1]), ["d", "e"]);
        assert_eq!(cut[1].matches[0].ended, None);
        assert_eq!(cut[1].matches[1].gap_seconds, Some(60));
        assert_eq!(cut[1].record.undecided, 1);
        // An end before the start is not believed.
        let mut bad = record("z", 0, 0, Win);
        bad.ended = Some(at(-30));
        let cut = sessions(&[bad], &SessionRules::default());
        assert_eq!((cut[0].started, cut[0].ended), (at(0), at(0)));
        assert!(sessions(&[], &SessionRules::default()).is_empty());
    }

    #[test]
    fn a_match_without_an_id_is_keyed_by_its_start() {
        let mut old = record("", 0, 20, Win);
        old.started_is_local = true;
        old.utc_offset_minutes = None;
        assert!(old.key().starts_with("start:"));
        let records = [old.clone(), record("", 30, 20, Win)];
        let cut = sessions(&records, &SessionRules::default());
        assert_eq!(cut[0].matches.len(), 2);
        assert_eq!(old.local_start(), Some(at(0).naive_utc()));
        let utc = record("a", 0, 20, Win);
        assert_eq!(utc.local_start(), Some(at(120).naive_utc()));
        let mut unknown = record("a", 0, 20, Win);
        unknown.utc_offset_minutes = None;
        assert_eq!(unknown.local_start(), None);
    }

    #[test]
    fn a_session_is_found_by_its_id_or_by_a_match_in_it() {
        let records = [record("a", 0, 20, Win), record("b", 25, 20, Win)];
        let cut = sessions(&records, &SessionRules::default());
        assert_eq!(find_session(&cut, "session:a").unwrap().id, "session:a");
        // The first match is gone from the folder: the session is now b's.
        let later = sessions(&records[1..], &SessionRules::default());
        assert_eq!(later[0].id, "session:b");
        assert!(find_session(&later, "session:a").is_none());
        assert_eq!(find_session(&cut, "session:b").unwrap().id, "session:a");
    }

    #[test]
    fn records_and_sessions_survive_json() {
        let mut r = played(record("a", 0, 20, Win), 5, 4, 2);
        r.rounds = vec![RoundLine {
            number: 1,
            won: Some(true),
            side: Some(Side::Attack),
            player: Some(RoundPlayerLine {
                kills: 2,
                opening_kill: true,
                ..RoundPlayerLine::default()
            }),
        }];
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"matchID\":\"a\"") && json.contains("\"utcOffsetMinutes\":120"));
        assert_eq!(serde_json::from_str::<MatchRecord>(&json).unwrap(), r);
        let cut = sessions(&[r], &SessionRules::default());
        let json = serde_json::to_string(&cut).unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<PlaySession>>(&json).unwrap(),
            cut
        );
        // Rules read from a partial file keep the other defaults.
        let rules: SessionRules = serde_json::from_str(r#"{"breakSeconds":300}"#).unwrap();
        assert_eq!(
            (rules.break_seconds, rules.session_gap_seconds),
            (300, 3600)
        );
    }

    fn on_match(id: &str) -> Target {
        Target::Match {
            match_id: id.to_owned(),
        }
    }

    fn journal() -> Journal {
        let mut j = Journal::new();
        j.add_tag(on_match("a"), "Clutch", at(0));
        let round = Target::Round {
            match_id: "a".to_owned(),
            round: 3,
        };
        j.add_tag(round, "throw", at(1));
        let kill = Target::Kill {
            match_id: "a".to_owned(),
            round: 3,
            index: Some(2),
            time: Some(101.5),
        };
        j.add_note(kill, "peeked twice", at(2));
        let session = Target::Session {
            session_id: "session:a".to_owned(),
        };
        j.add_note(session, "tired", at(3));
        let metric = GoalMetric {
            stat: Stat::KillDeathRatio,
            comparison: Comparison::AtLeast,
            target: 1.0,
            scope: Scope::Rolling { matches: 10 },
        };
        j.add_goal("stay positive", Some(metric), Some(at(10_000)), at(4));
        j.add_goal("call more", None, None, at(5));
        j
    }

    #[test]
    fn a_journal_survives_json() {
        let j = journal();
        let json = j.to_json().unwrap();
        assert_eq!(Journal::from_json(&json).unwrap(), j);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["tags"][1]["target"]["kind"], "round");
        assert_eq!(v["tags"][1]["target"]["matchID"], "a");
        assert_eq!(v["notes"][0]["target"]["time"], 101.5);
        assert_eq!(v["notes"][1]["target"]["sessionID"], "session:a");
        assert_eq!(v["goals"][0]["metric"]["stat"], "killDeathRatio");
        assert_eq!(v["goals"][0]["metric"]["scope"]["kind"], "rolling");
        assert_eq!(v["goals"][0]["status"], "active");
        assert!(v.get("unread").is_none() && v.get("extra").is_none());
        // An empty object is an empty journal.
        assert_eq!(Journal::from_json("{}").unwrap(), Journal::new());
        assert!(Journal::from_json("[").is_err());
    }

    #[test]
    fn tags_are_one_per_target_and_label() {
        let mut j = Journal::new();
        let id = j.add_tag(on_match("a"), " Clutch ", at(0));
        assert_eq!(j.add_tag(on_match("a"), "clutch", at(1)), id);
        assert_ne!(j.add_tag(on_match("b"), "clutch", at(1)), id);
        j.add_tag(on_match("b"), "throw", at(1));
        assert_eq!(j.tags.len(), 3);
        assert_eq!(j.tags[0].label, "Clutch");
        assert_eq!(j.tags_on(&on_match("a")).count(), 1);
        // Labels are listed as written.
        assert_eq!(j.labels().len(), 3);
        assert!(j.remove_tag(&id, at(2)));
        assert!(!j.remove_tag(&id, at(3)));
        assert_eq!(j.tags_on(&on_match("a")).count(), 0);
        // Tagging again brings the same tag back, newer than its deletion.
        assert_eq!(j.add_tag(on_match("a"), "clutch", at(4)), id);
        let target = on_match("a");
        let tag = j.tags_on(&target).next().unwrap();
        assert_eq!(
            (tag.label.as_str(), tag.edited, tag.deleted),
            ("clutch", at(4), None)
        );
    }

    #[test]
    fn notes_and_goals_keep_created_and_stamp_edits() {
        let mut j = journal();
        let note = j.notes[0].id.clone();
        assert!(j.edit_note(&note, "peeked three times", at(9)));
        assert_eq!((j.notes[0].created, j.notes[0].edited), (at(2), at(9)));
        assert!(!j.edit_note("nope", "x", at(9)));
        let goal = j.goals[1].id.clone();
        assert!(j.set_goal_status(&goal, GoalStatus::Achieved, at(9)));
        assert!(j.edit_goal(&goal, at(10), |g| {
            g.text = "call every drone".to_owned();
            g.id = "changed".to_owned();
        }));
        let g = &j.goals[1];
        assert_eq!(
            (g.id.as_str(), g.status, g.created),
            (goal.as_str(), GoalStatus::Achieved, at(5))
        );
        assert_eq!((g.text.as_str(), g.edited), ("call every drone", at(10)));
        assert!(j.remove_goal(&goal, at(11)));
        assert!(!j.set_goal_status(&goal, GoalStatus::Active, at(12)));
        assert_eq!(j.live_goals().count(), 1);
        // Two notes with the same text at the same instant are two notes.
        let a = j.add_note(on_match("a"), "same", at(20));
        assert_ne!(j.add_note(on_match("a"), "same", at(20)), a);
    }

    #[test]
    fn the_later_change_wins_a_merge_and_deletions_stay_deleted() {
        let base = journal();
        let (note, tag) = (base.notes[0].id.clone(), base.tags[0].id.clone());
        let mut phone = base.clone();
        let mut pc = base.clone();
        phone.edit_note(&note, "from the phone", at(10));
        pc.edit_note(&note, "from the pc", at(11));
        phone.remove_tag(&tag, at(12));
        pc.add_note(on_match("b"), "only on the pc", at(13));
        phone.add_tag(on_match("b"), "new", at(14));
        pc.add_tag(on_match("b"), "New", at(15));

        let mut one = phone.clone();
        one.merge(&pc);
        let mut two = pc.clone();
        two.merge(&phone);
        assert_eq!(one, two);
        // Merging again changes nothing.
        let again = one.clone();
        one.merge(&pc);
        one.merge(&phone);
        assert_eq!(one, again);

        let text = |j: &Journal, id: &str| {
            let note = j.notes.iter().find(|n| n.id == id).unwrap();
            note.text.clone()
        };
        assert_eq!(text(&one, &note), "from the pc");
        assert_eq!(one.live_notes().count(), 3);
        // The tag stays deleted though the pc still had it, and the text of
        // a deleted item is gone.
        let dead = one.tags.iter().find(|t| t.id == tag).unwrap();
        assert_eq!((dead.deleted, dead.label.as_str()), (Some(at(12)), ""));
        // The same tag made on both sides is one tag.
        assert_eq!(one.tags_on(&on_match("b")).count(), 1);
        assert_eq!(one.tags.len(), base.tags.len() + 1);

        // A copy that never saw the deletion cannot bring the tag back,
        // while a tag set again after it does come back.
        let mut stale = base.clone();
        stale.merge(&one);
        assert!(
            stale
                .tags
                .iter()
                .any(|t| t.id == tag && t.deleted.is_some())
        );
        let mut fresh = base.clone();
        fresh.remove_tag(&tag, at(1));
        fresh.add_tag(on_match("a"), "Clutch", at(30));
        one.merge(&fresh);
        assert_eq!(one.tags_on(&on_match("a")).count(), 1);

        // Tombstones go once they are old enough.
        let mut j = phone.clone();
        assert_eq!(j.purge_deleted(at(12)), 0);
        assert_eq!(j.purge_deleted(at(13)), 1);
        assert!(j.tags.iter().all(|t| t.id != tag));
    }

    #[test]
    fn fields_and_items_of_a_newer_version_are_kept() {
        let json = r#"{
            "version": 7,
            "theme": {"dark": true},
            "tags": [
                {"id": "t1", "target": {"kind": "match", "matchID": "a"}, "label": "x",
                 "created": "2027-01-01T10:00:00.000Z", "edited": "2027-01-01T10:00:00.000Z",
                 "colour": "red"},
                {"id": "t2", "target": {"kind": "clip", "clipID": "c"}, "label": "y",
                 "created": "2027-01-01T10:00:00.000Z", "edited": "2027-01-01T10:00:00.000Z"}
            ],
            "goals": [{"id": "g1", "text": "t", "created": "2027-01-01T10:00:00Z",
                       "edited": "2027-01-01T10:00:00+02:00",
                       "metric": {"stat": "plants", "comparison": "atLeast", "target": 1,
                                  "scope": {"kind": "perMatch"}}}]
        }"#;
        let mut j = Journal::from_json(json).unwrap();
        assert_eq!((j.version, j.tags.len(), j.goals.len()), (7, 1, 0));
        assert_eq!(j.tags[0].extra["colour"], "red");
        assert_eq!((j.unread.tags.len(), j.unread.goals.len()), (1, 1));
        j.add_tag(on_match("b"), "mine", at(0));
        let out: Value = serde_json::from_str(&j.to_json().unwrap()).unwrap();
        assert_eq!(out["version"], 7);
        assert_eq!(out["theme"]["dark"], true);
        assert_eq!(out["tags"].as_array().unwrap().len(), 3);
        assert_eq!(out["tags"][0]["colour"], "red");
        assert_eq!(out["tags"][2]["target"]["clipID"], "c");
        assert_eq!(out["goals"][0]["metric"]["stat"], "plants");
        // Merging keeps the other side's unread items once, and the later
        // of two.
        let mut newer = Journal::from_json(json).unwrap();
        newer.unread.tags[0]["edited"] = "2027-02-01T10:00:00.000Z".into();
        newer.unread.tags[0]["label"] = "z".into();
        j.merge(&newer);
        assert_eq!(j.unread.tags.len(), 1);
        assert_eq!(j.unread.tags[0]["label"], "z");
        let mut empty = Journal::new();
        empty.merge(&j);
        assert_eq!((empty.version, empty.unread.goals.len()), (7, 1));
        assert_eq!(empty.extra["theme"]["dark"], true);
    }

    #[test]
    fn saving_replaces_the_file_in_one_step() {
        let name = format!("replay-analyzer-journal-{}", std::process::id());
        let dir = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("journal.json");
        // No file yet: an empty journal.
        assert_eq!(Journal::load(&path).unwrap(), Journal::new());
        let mut j = journal();
        j.save(&path).unwrap();
        assert_eq!(Journal::load(&path).unwrap(), j);
        // Saving over an existing file replaces it and leaves nothing else.
        j.add_note(on_match("c"), "second save", at(50));
        j.save(&path).unwrap();
        assert_eq!(Journal::load(&path).unwrap(), j);
        let files: Vec<_> = (std::fs::read_dir(&dir).unwrap())
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files, ["journal.json"]);
        // A file that is not a journal is an error, not an empty journal.
        std::fs::write(&path, "not json").unwrap();
        assert!(matches!(Journal::load(&path), Err(JournalError::Json(_))));
        // A save that cannot be written fails and leaves no temporary file.
        let missing = dir.join("no-such-folder").join("journal.json");
        assert!(matches!(j.save(&missing), Err(JournalError::Io(_))));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn goal(stat: Stat, comparison: Comparison, target: f64, scope: Scope) -> Goal {
        let mut j = Journal::new();
        let metric = GoalMetric {
            stat,
            comparison,
            target,
            scope,
        };
        j.add_goal("goal", Some(metric), Some(at(1000)), at(0));
        j.goals.remove(0)
    }

    #[test]
    fn goal_progress_follows_the_scope() {
        let rules = SessionRules::default();
        let records = [
            // Before the goal was set: never counted.
            played(record("old", -30, 20, Loss), 5, 0, 5),
            played(record("a", 0, 20, Win), 5, 6, 3),
            played(record("b", 25, 20, Loss), 5, 2, 4),
            // Header-only: no kills to count.
            record("c", 50, 20, Win),
            // A new session.
            played(record("d", 500, 20, Win), 6, 6, 6),
            // After the due time.
            played(record("late", 2000, 20, Win), 5, 50, 1),
        ];
        let kd = |scope| goal(Stat::KillDeathRatio, Comparison::AtLeast, 1.0, scope);

        let p = goal_progress(&kd(Scope::PerMatch), &records, &rules).unwrap();
        assert_eq!(p.sample, 4);
        let points: Vec<_> = (p.points.iter())
            .map(|p| (p.key.as_str(), p.value, p.met))
            .collect();
        assert_eq!(
            points,
            [("a", 2.0, true), ("b", 0.5, false), ("d", 1.0, true)]
        );
        assert_eq!(
            (p.met, p.current, p.current_met),
            (2, Some(1.0), Some(true))
        );

        // The first session began with `old`, which does not count: 8
        // kills and 7 deaths over a, b and c.
        let p = goal_progress(&kd(Scope::PerSession), &records, &rules).unwrap();
        assert_eq!(p.points.len(), 2);
        let first = &p.points[0];
        assert_eq!((first.key.as_str(), first.matches), ("session:old", 3));
        assert!((first.value - 8.0 / 7.0).abs() < 1e-9 && first.met);
        let second = &p.points[1];
        assert_eq!(
            (second.key.as_str(), second.matches, second.value),
            ("session:d", 1, 1.0)
        );

        let rolling = kd(Scope::Rolling { matches: 3 });
        let p = goal_progress(&rolling, &records, &rules).unwrap();
        assert_eq!(p.points.len(), 2);
        assert_eq!((p.points[1].key.as_str(), p.points[1].matches), ("d", 3));
        assert!((p.points[1].value - 8.0 / 10.0).abs() < 1e-9);
        assert_eq!((p.met, p.current_met), (1, Some(false)));
        // A window that is not full yet reports a value and claims nothing.
        let rolling = kd(Scope::Rolling { matches: 10 });
        let p = goal_progress(&rolling, &records, &rules).unwrap();
        assert!(p.points.is_empty());
        assert_eq!(p.current_met, None);
        assert!((p.current.unwrap() - 14.0 / 13.0).abs() < 1e-9);

        let wins = goal(Stat::WinRate, Comparison::AtLeast, 0.75, Scope::PerSession);
        let p = goal_progress(&wins, &records, &rules).unwrap();
        assert!((p.points[0].value - 2.0 / 3.0).abs() < 1e-9 && !p.points[0].met);
        let deaths = goal(Stat::Deaths, Comparison::AtMost, 4.0, Scope::PerMatch);
        let p = goal_progress(&deaths, &records, &rules).unwrap();
        let met: Vec<_> = p.points.iter().map(|p| p.met).collect();
        assert_eq!(met, [true, true, false]);

        // No metric, nothing to measure; no matches, nothing claimed.
        let mut plain = kd(Scope::PerMatch);
        plain.metric = None;
        assert!(goal_progress(&plain, &records, &rules).is_none());
        let p = goal_progress(&kd(Scope::PerMatch), &[], &rules).unwrap();
        assert_eq!((p.sample, p.current, p.current_met), (0, None, None));
    }

    #[test]
    fn totals_give_no_value_without_the_data() {
        let t = Totals::of(&[record("a", 0, 20, Unfinished)]);
        for stat in [
            Stat::Kills,
            Stat::KillDeathRatio,
            Stat::WinRate,
            Stat::RoundWinRate,
        ] {
            assert_eq!(t.value(stat), None, "{stat:?}");
        }
        let mut r = played(record("a", 0, 20, Win), 4, 3, 0);
        r.player.as_mut().unwrap().headshots = 1;
        r.player.as_mut().unwrap().damage_dealt = Some(400);
        let t = Totals::of(&[r]);
        // No deaths: the K/D is the kills.
        assert_eq!(t.value(Stat::KillDeathRatio), Some(3.0));
        assert_eq!(t.value(Stat::SurvivalRate), Some(1.0));
        assert_eq!(t.value(Stat::DamagePerRound), Some(100.0));
        assert!((t.value(Stat::HeadshotPercentage).unwrap() - 100.0 / 3.0).abs() < 1e-9);
        assert_eq!(t.value(Stat::WinRate), Some(1.0));
    }

    /// `n` sessions of three matches each: a win, a loss, then a loss
    /// after a 15 minute gap.
    fn evenings(n: i64) -> Vec<MatchRecord> {
        (0..n)
            .flat_map(|day| {
                let start = day * 1440;
                [
                    played(record(&format!("{day}a"), start, 20, Win), 5, 5, 3),
                    played(record(&format!("{day}b"), start + 25, 20, Loss), 5, 4, 4),
                    played(record(&format!("{day}c"), start + 60, 20, Loss), 5, 2, 5),
                ]
            })
            .collect()
    }

    #[test]
    fn insights_claim_nothing_below_the_minimum_sample() {
        let rules = InsightRules::default();
        let i = insights(&evenings(9), &SessionRules::default(), &rules);
        assert_eq!((i.sessions, i.overall.sample), (9, 27));
        assert!(i.overall.enough && i.overall.rates.win_rate.is_some());
        // Nine matches a group: the counts are there, the rates are not.
        for g in &i.by_position[..3] {
            assert_eq!((g.sample, g.enough), (9, false), "{}", g.label);
            assert_eq!(g.rates, Rates::default(), "{}", g.label);
        }
        assert_eq!(i.by_position[0].totals.wins, 9);
        let c = &i.after_loss_vs_after_win;
        assert_eq!((c.first.sample, c.second.sample), (9, 9));
        assert!(c.effect.is_none());
        assert!(i.after_break_vs_without.effect.is_none());

        // One more evening and the same groups speak.
        let i = insights(&evenings(10), &SessionRules::default(), &rules);
        let first = &i.by_position[0];
        assert_eq!((first.sample, first.enough), (10, true));
        assert_eq!(first.rates.win_rate, Some(1.0));
        assert_eq!(first.rates.kills_per_round, Some(1.0));
        let [low, high] = first.rates.win_rate_interval.unwrap();
        assert!(low > 0.6 && low < 0.8 && high == 1.0);
        assert_eq!(i.by_position[3].sample, 0);
        // After a loss (the third matches) against after a win (the
        // second): both all lost, the third with fewer kills.
        let c = &i.after_loss_vs_after_win;
        let e = c.effect.as_ref().unwrap();
        assert_eq!(e.win_rate_difference, Some(0.0));
        assert!((e.kills_per_round_difference.unwrap() + 0.4).abs() < 1e-9);
        // Every match of a group is the same, so there is no spread to
        // scale by.
        assert_eq!(e.kills_per_round_d, None);
        // The third match follows a 15 minute gap: a break.
        let b = &i.after_break_vs_without;
        assert_eq!((b.first.sample, b.second.sample), (10, 10));
        assert!(b.effect.is_some());
        // No match follows two straight losses.
        assert_eq!(i.on_loss_streak.first.sample, 0);
        assert_eq!(i.on_loss_streak.second.sample, 20);
        assert!(i.on_loss_streak.effect.is_none());
        assert_eq!(i.by_session_length[1].sample, 30);
        // The records start at 08:00 UTC, 10:00 at UTC+2, and the third
        // of an evening at 11:00: all in the morning.
        assert_eq!(i.by_time_of_day[1].sample, 30);
        // A lower bar is the caller's to set.
        let lax = InsightRules {
            min_matches: 3,
            ..rules
        };
        let i = insights(&evenings(3), &SessionRules::default(), &lax);
        assert!(i.by_position[0].enough && i.after_loss_vs_after_win.effect.is_some());
        // Insights read back from JSON, to the last digit or the one
        // before: serde_json does not parse every float exactly.
        let json = serde_json::to_string(&i).unwrap();
        let back: Insights = serde_json::from_str(&json).unwrap();
        assert_eq!(back.by_position[0].totals, i.by_position[0].totals);
        assert_eq!(back.after_loss_vs_after_win.second.sample, 3);
    }

    #[test]
    fn effect_sizes() {
        assert!((cohen_h(0.75, 0.25) - std::f64::consts::FRAC_PI_3).abs() < 1e-9);
        assert_eq!(cohen_h(0.5, 0.5), 0.0);
        let d = cohen_d(&[1.0, 2.0, 3.0], &[2.0, 3.0, 4.0]).unwrap();
        assert!((d + 1.0).abs() < 1e-9);
        assert_eq!(cohen_d(&[1.0], &[2.0, 3.0]), None);
        assert_eq!(cohen_d(&[1.0, 1.0], &[2.0, 2.0]), None);
        let [low, high] = wilson(5, 10).unwrap();
        assert!((low - 0.2366).abs() < 1e-3 && (high - 0.7634).abs() < 1e-3);
        assert_eq!(wilson(0, 0), None);
    }

    /// A round the player's team won or lost, in which the player died
    /// first or not.
    fn round(number: u32, won: bool, opening_death: bool, kills: u32) -> RoundLine {
        RoundLine {
            number,
            won: Some(won),
            side: None,
            player: Some(RoundPlayerLine {
                kills,
                died: opening_death,
                opening_death,
                ..RoundPlayerLine::default()
            }),
        }
    }

    #[test]
    fn round_level_groups_follow_the_round_before() {
        let mut r = record("a", 0, 20, Loss);
        r.rounds = vec![
            round(1, false, true, 0),
            round(2, false, false, 1),
            round(3, true, false, 2),
            round(4, false, true, 0),
            // Round 5 is missing: round 6 has no round before it.
            round(6, false, false, 3),
            round(7, false, false, 0),
            round(8, true, false, 1),
        ];
        let rules = InsightRules {
            min_rounds: 2,
            ..InsightRules::default()
        };
        let i = insights(&[r], &SessionRules::default(), &rules);
        // Rounds 2, 3, 4, 7, 8 have a round before them. Round 2 follows a
        // first death.
        let d = &i.after_dying_first;
        assert_eq!((d.first.sample, d.second.sample), (1, 4));
        assert_eq!((d.first.kills, d.second.kills), (1, 3));
        assert!(!d.first.enough && d.effect.is_none() && d.first.win_rate.is_none());
        assert_eq!(d.second.win_rate, Some(0.5));
        // Rounds 3 and 8 follow two straight lost rounds; both were won.
        let s = &i.after_round_losses;
        assert_eq!((s.first.sample, s.first.won), (2, 2));
        assert_eq!((s.second.sample, s.second.won), (3, 0));
        let e = s.effect.as_ref().unwrap();
        assert_eq!(e.win_rate_difference, Some(1.0));
        assert!((e.win_rate_h.unwrap() - std::f64::consts::PI).abs() < 1e-9);
        assert!((e.kills_per_round_difference.unwrap() - (1.5 - 1.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn tilt_rises_with_losses_and_a_falling_kd() {
        let rules = SessionRules::default();
        let tilt_rules = TiltRules::default();
        let signal = |records: &[MatchRecord]| {
            let cut = sessions(records, &rules);
            tilt(cut.last().unwrap(), &rules, &tilt_rules)
        };
        let mut records = vec![
            played(record("a", 0, 20, Win), 5, 6, 3),
            played(record("b", 25, 20, Loss), 5, 5, 3),
        ];
        let s = signal(&records);
        assert_eq!(
            (s.level, s.loss_streak, s.suggest_break),
            (TiltLevel::None, 1, false)
        );
        assert!(s.reasons.is_empty() && s.suggested_break_seconds.is_none());

        // A second loss with the K/D holding: worth watching.
        records.push(played(record("c", 50, 20, Loss), 5, 5, 3));
        let s = signal(&records);
        assert_eq!(
            (s.level, s.loss_streak, s.suggest_break),
            (TiltLevel::Watch, 2, false)
        );
        assert_eq!(
            s.reasons,
            [TiltReason::LossStreak, TiltReason::WinRateFalling]
        );
        assert_eq!((s.before.matches, s.during.matches, s.sample), (1, 2, 3));
        assert_eq!(s.kill_death_ratio_before, Some(2.0));

        // The same two losses with the K/D more than a fifth lower: tilted.
        records[2] = played(record("c", 50, 20, Loss), 5, 1, 5);
        let s = signal(&records);
        assert_eq!(s.level, TiltLevel::Tilted);
        assert!(s.reasons.contains(&TiltReason::KillDeathRatioFalling));
        assert_eq!(s.kill_death_ratio_during, Some(0.75));
        assert_eq!(
            (s.suggest_break, s.suggested_break_seconds),
            (true, Some(600))
        );

        // Three losses are enough on their own, and an unfinished match in
        // between does not end the streak.
        let records = [
            record("a", 0, 20, Loss),
            record("b", 25, 20, Unfinished),
            record("c", 50, 20, Loss),
            record("d", 75, 20, Loss),
        ];
        let s = signal(&records);
        assert_eq!((s.level, s.loss_streak), (TiltLevel::Tilted, 3));
        assert_eq!(s.reasons, [TiltReason::LossStreak]);
        assert_eq!(s.kill_death_ratio_during, None);

        // A break resets the count: one loss since.
        let mut records = records.to_vec();
        records.push(record("e", 110, 20, Loss));
        let s = signal(&records);
        assert_eq!(
            (s.level, s.loss_streak, s.matches_since_break),
            (TiltLevel::None, 1, 1)
        );
        assert_eq!(s.seconds_since_break, 1200);

        // Two hours without a break suggest one, whatever the results.
        let long: Vec<_> = (0..5)
            .map(|i| record(&format!("m{i}"), i * 25, 22, Win))
            .collect();
        let s = signal(&long);
        assert_eq!((s.level, s.suggest_break), (TiltLevel::None, true));
        assert_eq!(s.reasons, [TiltReason::LongWithoutBreak]);
        assert_eq!(s.seconds_since_break, 122 * 60);
    }
}
