//! Rank, rank points and level from outside the replay, joined to the
//! players a replay names.
//!
//! Replays hold no rank, rank points or reputation; they do hold each
//! player's Ubisoft profile id, name, platform and clearance level. This
//! module joins profile stats fetched elsewhere onto those players and
//! derives lobby strength and rank progress from them.
//!
//! Nothing here touches the network and no key or URL lives in the crate.
//! The caller implements [`ProfileSource`] against its own proxy,
//! [`requests_for`] says who needs fetching, [`ProfileCache`] keeps what
//! came back in a JSON file the caller names, and [`from_r6data_json`]
//! turns one provider response into [`ProfileStats`]: the only function
//! that knows the provider's field names.
//!
//! What is the provider's and what is modelled here is said on each item;
//! `docs/outside/profiles.md` has the whole account.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::header::Platform;
use crate::summary::{MatchSummary, PlayerSummary};

/// What can go wrong fetching, reading or storing profiles.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("profile data is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// The provider answered for another account: lookups go by name, and a
    /// name can belong to someone else by the time it is looked up.
    #[error("asked for profile {wanted}, the provider answered for {got}")]
    Mismatch { wanted: String, got: String },
    #[error("profile cache has version {0}, newer than this build reads")]
    CacheVersion(u32),
    /// Whatever the caller's source reports: transport, auth, rate limit.
    #[error("profile source: {0}")]
    Source(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T, E = ProfileError> = std::result::Result<T, E>;

// ---------------------------------------------------------------------------
// Ranks
// ---------------------------------------------------------------------------

/// A rank: the id counts divisions from 1 (Copper V); 0 is unranked.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rank {
    pub id: u32,
    /// `Gold II`, `Champion`, `Unranked`, or `Rank(<id>)` for an id the
    /// table does not have.
    pub name: String,
}

const TIERS: [&str; 7] = [
    "Copper", "Bronze", "Silver", "Gold", "Platinum", "Emerald", "Diamond",
];
const DIVISIONS: [&str; 5] = ["V", "IV", "III", "II", "I"];

/// How rank points map to ranks. A model of the game's ladder: see the
/// constants for what each is based on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RankSystem {
    pub name: &'static str,
    /// Rank points at which Copper V starts.
    pub base: i32,
    /// Rank points per division.
    pub division: i32,
    /// Champion divisions: 1 (a single rank) or 5 (Champion V to I).
    pub champion_divisions: u32,
}

impl RankSystem {
    /// Y7S4 to Y11S1: Copper V at 1000 RP, 100 RP a division, 35 divisions,
    /// Champion from 4500 RP. Ids 1 to 36.
    pub const RANKED_2: Self = Self {
        name: "Ranked 2.0",
        base: 1000,
        division: 100,
        champion_divisions: 1,
    };

    /// Y11S2 on: Champion has five divisions too, 40 ranks. Ubisoft says
    /// that much; the thresholds are assumed to carry on from Ranked 2.0
    /// (Champion V at 4500, Champion I from 4900).
    pub const RANKED_3: Self = Self {
        name: "Ranked 3.0",
        base: 1000,
        division: 100,
        champion_divisions: 5,
    };

    /// First season id (Ubisoft counts seasons from Y1S1 = 1) of Ranked 3.0:
    /// Y11S2.
    pub const RANKED_3_FIRST_SEASON: u32 = 42;

    /// The system of a season id; the current one when the season is not
    /// known.
    pub fn for_season(season: Option<u32>) -> &'static Self {
        match season {
            Some(s) if s < Self::RANKED_3_FIRST_SEASON => &Self::RANKED_2,
            _ => &Self::RANKED_3,
        }
    }

    /// The highest rank id.
    pub fn top(&self) -> u32 {
        35 + self.champion_divisions
    }

    /// `Gold II` for 19. `None` for ids past the top; 0 is `Unranked`.
    pub fn name_of(&self, id: u32) -> Option<String> {
        match id {
            0 => Some("Unranked".to_owned()),
            1..=35 => {
                let i = (id - 1) as usize;
                Some(format!("{} {}", TIERS[i / 5], DIVISIONS[i % 5]))
            }
            _ if id > self.top() => None,
            _ if self.champion_divisions == 1 => Some("Champion".to_owned()),
            _ => Some(format!("Champion {}", DIVISIONS[(id - 36) as usize])),
        }
    }

    /// The rank with this id, named `Rank(<id>)` when the table lacks it.
    pub fn rank(&self, id: u32) -> Rank {
        let name = self.name_of(id).unwrap_or_else(|| format!("Rank({id})"));
        Rank { id, name }
    }

    /// The rank a ranked player with these points holds. Points below the
    /// base are Copper V.
    pub fn rank_of_points(&self, points: i32) -> Rank {
        let steps = (points - self.base).max(0) / self.division;
        self.rank((steps as u32 + 1).min(self.top()))
    }

    /// Rank points at which a rank starts. `None` for 0 and unknown ids.
    pub fn floor(&self, id: u32) -> Option<i32> {
        (1..=self.top())
            .contains(&id)
            .then(|| self.base + (id as i32 - 1) * self.division)
    }

    /// The next division up and the points still needed. `None` at the top.
    pub fn next_division(&self, points: i32) -> Option<Target> {
        self.target(self.rank_of_points(points).id + 1, points)
    }

    /// The first division of the next tier (`Gold V` from any Silver) and
    /// the points still needed. `None` in Champion.
    pub fn next_tier(&self, points: i32) -> Option<Target> {
        let id = self.rank_of_points(points).id;
        if id > 35 {
            return None;
        }
        self.target((id - 1) / 5 * 5 + 6, points)
    }

    fn target(&self, id: u32, points: i32) -> Option<Target> {
        let floor = self.floor(id)?;
        Some(Target {
            rank: self.rank(id),
            at: floor,
            points: floor - points,
        })
    }
}

/// A rank ahead and how far it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub rank: Rank,
    /// Rank points at which it starts.
    pub at: i32,
    /// Rank points still needed.
    pub points: i32,
}

// ---------------------------------------------------------------------------
// Profile stats
// ---------------------------------------------------------------------------

/// Ubisoft keeps separate ranked profiles for PC and for the consoles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    Pc,
    Console,
}

impl Family {
    pub fn of(platform: Platform) -> Self {
        match platform {
            Platform::Pc => Self::Pc,
            Platform::PlayStation | Platform::Xbox => Self::Console,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Pc => "pc",
            Self::Console => "console",
        }
    }
}

/// One playlist's standing in one season, as the provider gave it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BoardStats {
    /// Ubisoft's season number, counted from Y1S1 = 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    /// The rank, named by this crate's table: from `rank_points` when the
    /// player is ranked, else from the provider's rank id. `Unranked` (id 0)
    /// for a player without a rank this season.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<Rank>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_points: Option<i32>,
    /// The season's peak.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_rank: Option<Rank>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_rank_points: Option<i32>,
    /// The rank ids exactly as the provider sent them, which `rank` and
    /// `max_rank` need not repeat: their ids follow this crate's table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_rank: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_max_rank: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wins: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub losses: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abandons: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kills: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deaths: Option<u32>,
    /// Place on the Champions leaderboard, when on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_position: Option<u32>,
    /// When the provider says the board last changed, as it wrote it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl BoardStats {
    /// Rank points, for a player who holds a rank this season.
    pub fn rated_points(&self) -> Option<i32> {
        match &self.rank {
            Some(r) if r.id > 0 => self.rank_points,
            _ => None,
        }
    }

    /// Matches the season counts: wins, losses and abandons.
    pub fn matches(&self) -> Option<u32> {
        Some(self.wins? + self.losses? + self.abandons.unwrap_or(0))
    }

    /// Kills per death; `None` without deaths.
    pub fn kd(&self) -> Option<f64> {
        match (self.kills, self.deaths) {
            (Some(k), Some(d)) if d > 0 => Some(f64::from(k) / f64::from(d)),
            _ => None,
        }
    }

    /// Share of won matches among those won or lost.
    pub fn win_rate(&self) -> Option<f64> {
        match (self.wins, self.losses) {
            (Some(w), Some(l)) if w + l > 0 => Some(f64::from(w) / f64::from(w + l)),
            _ => None,
        }
    }
}

/// One player's stats at one moment: this crate's own shape, which stays
/// put when the provider's changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileStats {
    /// Ubisoft profile id: what joins this to `players[].profileID`.
    #[serde(rename = "profileID")]
    pub profile_id: String,
    /// The provider's answer named this profile id. `false` when it named
    /// none, so the join rests on the name looked up.
    #[serde(default)]
    pub id_confirmed: bool,
    #[serde(
        default,
        deserialize_with = "de_platform",
        skip_serializing_if = "Option::is_none"
    )]
    pub platform: Option<Platform>,
    /// Which of the account's ranked profiles this is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<Family>,
    /// The name the provider knows the account by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// When the caller fetched it. Stats are as of then, not as of any
    /// match.
    #[serde(with = "ts")]
    pub fetched_at: DateTime<Utc>,
    /// The ranked board's season.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    /// Clearance level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranked: Option<BoardStats>,
    /// The other boards by the provider's id: `standard`, `casual`,
    /// `event`, `warmup`, ...
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub boards: BTreeMap<String, BoardStats>,
}

impl ProfileStats {
    /// A profile with nothing known yet.
    pub fn new(profile_id: impl Into<String>, fetched_at: DateTime<Utc>) -> Self {
        Self {
            profile_id: profile_id.into(),
            id_confirmed: false,
            platform: None,
            family: None,
            username: None,
            fetched_at,
            season: None,
            level: None,
            ranked: None,
            boards: BTreeMap::new(),
        }
    }

    /// Ranked rank points, for a player who holds a rank.
    pub fn rank_points(&self) -> Option<i32> {
        self.ranked.as_ref()?.rated_points()
    }

    fn is_for(&self, player: &PlayerSummary) -> bool {
        !player.profile_id.is_empty() && self.profile_id.eq_ignore_ascii_case(&player.profile_id)
    }
}

/// The stats of `player` among `profiles`: by profile id, and of the
/// player's platform family when the account has more than one.
pub fn profile_of<'a>(
    player: &PlayerSummary,
    profiles: &'a [ProfileStats],
) -> Option<&'a ProfileStats> {
    let family = player.platform.map(Family::of);
    let mut found = profiles.iter().filter(|p| p.is_for(player));
    let first = found.next()?;
    if family.is_none() || first.family.is_none() || first.family == family {
        return Some(first);
    }
    Some(
        found
            .find(|p| p.family.is_none() || p.family == family)
            .unwrap_or(first),
    )
}

// ---------------------------------------------------------------------------
// Provider adapter
// ---------------------------------------------------------------------------

/// Reads one R6 Data API player response (`/r6/api/v2/profile`, or the
/// older `stats` shape) fetched for `key` at `fetched_at`.
///
/// One entry per platform family the account has a profile in; one entry
/// with whatever is known (the level, perhaps) when it has none. Every
/// field is optional and numbers may come as strings. Fails with
/// [`ProfileError::Mismatch`] when the response names profile ids and none
/// is `key`'s.
pub fn from_r6data_json(
    json: &str,
    key: &ProfileKey,
    fetched_at: DateTime<Utc>,
) -> Result<Vec<ProfileStats>> {
    from_r6data_value(&serde_json::from_str(json)?, key, fetched_at)
}

/// [`from_r6data_json`] on parsed JSON.
///
/// The shape read, with Ubisoft's own names (the provider passes its
/// `full_profiles` through):
///
/// ```text
/// player.nameOnPlatform, player.platformType
/// account.level
/// stats.platform_families_full_profiles[]
///   .profile_id, .platform_family
///   .board_ids_full_profiles[]
///     .board_id                      ranked, standard, casual, event, ...
///     .full_profiles[0]
///       .profile.{season_id, rank, rank_points, max_rank, max_rank_points,
///                 top_rank_position, update_time}
///       .season_statistics.{kills, deaths,
///                           match_outcomes.{wins, losses, abandons}}
/// ```
///
/// The provider's documented example flattens some of it (`season_id`
/// beside `profile`; `kills`, `wins`, `abandon` inside it); both are read.
pub fn from_r6data_value(
    value: &Value,
    key: &ProfileKey,
    fetched_at: DateTime<Utc>,
) -> Result<Vec<ProfileStats>> {
    let root = match value.get("data") {
        Some(d) if d.is_object() && value.get("stats").is_none() => d,
        _ => value,
    };
    let player = root.get("player");
    let account = root.get("account");

    let mut base = ProfileStats::new(key.profile_id.clone(), fetched_at);
    base.platform = first(&[player, Some(root)], &["platformType", "platform"])
        .and_then(Value::as_str)
        .and_then(platform_of)
        .or(key.platform);
    base.username = first(&[player, Some(root)], &["nameOnPlatform", "username"])
        .and_then(Value::as_str)
        .map(str::to_owned);
    base.level = first(&[account, Some(root)], &["level", "clearance_level"]).and_then(uint);

    let families = find(root, "platform_families_full_profiles", 3)
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);

    let mut out = Vec::new();
    let mut other = None;
    for entry in families {
        let id = first(&[Some(entry)], &["profile_id", "profileId", "id"]).and_then(Value::as_str);
        if let Some(id) = id
            && !key.profile_id.is_empty()
            && !id.eq_ignore_ascii_case(&key.profile_id)
        {
            other = Some(id.to_owned());
            continue;
        }
        let mut stats = base.clone();
        if let Some(id) = id {
            stats.profile_id = id.to_ascii_lowercase();
            stats.id_confirmed = true;
        }
        stats.family = match entry.get("platform_family").and_then(Value::as_str) {
            Some("pc") => Some(Family::Pc),
            Some("console") => Some(Family::Console),
            _ => key.family(),
        };
        let boards = entry
            .get("board_ids_full_profiles")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        for b in boards {
            let Some((id, board)) = board(b) else {
                continue;
            };
            if id == "ranked" {
                stats.season = board.season;
                stats.ranked = Some(board);
            } else {
                stats.boards.insert(id, board);
            }
        }
        out.push(stats);
    }

    if out.is_empty() {
        if let Some(got) = other {
            return Err(ProfileError::Mismatch {
                wanted: key.profile_id.clone(),
                got,
            });
        }
        base.family = key.family();
        out.push(base);
    }
    Ok(out)
}

/// One `board_ids_full_profiles` entry.
fn board(entry: &Value) -> Option<(String, BoardStats)> {
    let full = match entry.get("full_profiles") {
        Some(Value::Array(a)) => a.first()?,
        Some(v) if v.is_object() => v,
        _ => entry,
    };
    let profile = full.get("profile").unwrap_or(full);
    let season_stats = full.get("season_statistics");
    let outcomes = season_stats.and_then(|s| s.get("match_outcomes"));
    let places = [Some(profile), Some(full), season_stats, outcomes];
    let get = |names: &[&str]| first(&places, names);

    let id = first(&[Some(entry), Some(profile)], &["board_id"])
        .and_then(Value::as_str)?
        .to_ascii_lowercase();

    let season = get(&["season_id"]).and_then(uint);
    let system = RankSystem::for_season(season);
    let provider_rank = get(&["rank"]).and_then(uint);
    let provider_max_rank = get(&["max_rank"]).and_then(uint);
    let rank_points = get(&["rank_points"]).and_then(int);
    let max_rank_points = get(&["max_rank_points"]).and_then(int);

    let stats = BoardStats {
        season,
        rank: named(system, provider_rank, rank_points),
        rank_points,
        max_rank: named(system, provider_max_rank, max_rank_points),
        max_rank_points,
        provider_rank,
        provider_max_rank,
        wins: get(&["wins"]).and_then(uint),
        losses: get(&["losses"]).and_then(uint),
        abandons: get(&["abandons", "abandon"]).and_then(uint),
        kills: get(&["kills"]).and_then(uint),
        deaths: get(&["deaths"]).and_then(uint),
        top_position: get(&["top_rank_position"])
            .and_then(uint)
            .filter(|&p| p > 0),
        updated_at: get(&["update_time"])
            .and_then(Value::as_str)
            .map(str::to_owned),
    };
    Some((id, stats))
}

/// The rank to show for a provider rank id and rank points. Id 0 is
/// unranked whatever the points say (an unplayed season still carries
/// points); a ranked player's rank is named from the points, which the
/// table is surer of than of the provider's numbering.
fn named(system: &RankSystem, id: Option<u32>, points: Option<i32>) -> Option<Rank> {
    match (id, points) {
        (Some(0), _) => Some(system.rank(0)),
        (_, Some(p)) if p > 0 => Some(system.rank_of_points(p)),
        (Some(id), _) => Some(system.rank(id)),
        (None, Some(_)) => Some(system.rank(0)),
        (None, None) => None,
    }
}

/// The first of `names` any of `places` has, ignoring nulls.
fn first<'a>(places: &[Option<&'a Value>], names: &[&str]) -> Option<&'a Value> {
    places
        .iter()
        .flatten()
        .find_map(|p| names.iter().find_map(|n| p.get(n).filter(|v| !v.is_null())))
}

/// `name` in `value` or in an object up to `depth` levels below it.
fn find<'a>(value: &'a Value, name: &str, depth: u32) -> Option<&'a Value> {
    let map = value.as_object()?;
    if let Some(v) = map.get(name) {
        return Some(v);
    }
    if depth == 0 {
        return None;
    }
    map.values().find_map(|v| find(v, name, depth - 1))
}

/// A number, or a string holding one.
fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|n| n.is_finite())
}

fn int(v: &Value) -> Option<i32> {
    number(v).map(|n| n.round() as i32)
}

fn uint(v: &Value) -> Option<u32> {
    number(v).filter(|n| *n >= 0.0).map(|n| n.round() as u32)
}

/// The platform of a provider `platformType` or of this crate's own name.
fn platform_of(name: &str) -> Option<Platform> {
    match name.to_ascii_lowercase().as_str() {
        "uplay" | "pc" | "steam" => Some(Platform::Pc),
        "psn" | "playstation" => Some(Platform::PlayStation),
        "xbl" | "xbox" => Some(Platform::Xbox),
        _ => None,
    }
}

fn de_platform<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Platform>, D::Error> {
    let name: Option<String> = Option::deserialize(d)?;
    Ok(name.as_deref().and_then(platform_of))
}

mod ts {
    use super::*;

    pub fn serialize<S: Serializer>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        // Whole seconds as the other timestamps of the crate; a fraction
        // only when there is one, so a snapshot reads back as it was.
        s.serialize_str(&t.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        let text = String::deserialize(d)?;
        DateTime::parse_from_rfc3339(&text)
            .map(|t| t.with_timezone(&Utc))
            .map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------
// Fetching: keys, source, cache
// ---------------------------------------------------------------------------

/// A player to fetch: everything a replay gives to look one up by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileKey {
    #[serde(rename = "profileID")]
    pub profile_id: String,
    /// `None` before Y11S3 and on header-only reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
    /// The name to look up: the one the game gave as the match ended when
    /// the recording has it, else the one shown during the match.
    pub name: String,
    /// `name` is a nickname shown in place of the player's own name, so a
    /// lookup by name will not find the account; only one by profile id
    /// can.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub name_is_nickname: bool,
}

impl ProfileKey {
    pub fn of(player: &PlayerSummary) -> Self {
        Self {
            profile_id: player.profile_id.to_ascii_lowercase(),
            platform: player.platform,
            name: player
                .renamed_to
                .as_ref()
                .unwrap_or(&player.username)
                .clone(),
            name_is_nickname: player.uses_nickname && player.renamed_to.is_none(),
        }
    }

    pub fn family(&self) -> Option<Family> {
        self.platform.map(Family::of)
    }

    /// The provider's `platformType`: `uplay`, `psn` or `xbl`. PC when the
    /// replay does not say.
    pub fn platform_type(&self) -> &'static str {
        match self.platform {
            Some(Platform::PlayStation) => "psn",
            Some(Platform::Xbox) => "xbl",
            Some(Platform::Pc) | None => "uplay",
        }
    }

    /// The provider's `platform_families`: `pc` or `console`.
    pub fn platform_families(&self) -> &'static str {
        self.family().unwrap_or(Family::Pc).as_str()
    }

    fn cache_key(&self) -> String {
        cache_key(&self.profile_id, self.family())
    }
}

fn cache_key(profile_id: &str, family: Option<Family>) -> String {
    let family = family.unwrap_or(Family::Pc).as_str();
    format!("{}/{family}", profile_id.to_ascii_lowercase())
}

/// Where profiles come from. The crate has no implementation: the app
/// implements it over its own proxy, which holds the key and the URL.
pub trait ProfileSource {
    /// The stats of `keys`, in any order. A player the provider does not
    /// know is left out; an error is for the call as a whole.
    fn fetch(&self, keys: &[ProfileKey]) -> Result<Vec<ProfileStats>>;
}

/// The current time, for `fetched_at` and for cache ages.
pub fn now() -> DateTime<Utc> {
    std::time::SystemTime::now().into()
}

const CACHE_VERSION: u32 = 1;

/// One player in the cache.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedProfile {
    pub latest: ProfileStats,
    /// Earlier and current snapshots whose ranked board differs from the
    /// one before, oldest first: the input of [`rank_progress`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<ProfileStats>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Miss {
    #[serde(with = "ts")]
    at: DateTime<Utc>,
}

/// Fetched profiles, kept in a JSON file the caller names.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileCache {
    version: u32,
    /// By `<profile id>/<family>`.
    #[serde(default)]
    profiles: BTreeMap<String, CachedProfile>,
    /// Players asked for and not found, with when: not asked again until
    /// that is older than the max age.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    missing: BTreeMap<String, Miss>,
}

impl ProfileCache {
    pub fn new() -> Self {
        Self {
            version: CACHE_VERSION,
            ..Self::default()
        }
    }

    /// Reads the cache at `path`; an empty one when there is no such file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(e) => return Err(e.into()),
        };
        let cache: Self = serde_json::from_str(&text)?;
        if cache.version > CACHE_VERSION {
            return Err(ProfileError::CacheVersion(cache.version));
        }
        Ok(cache)
    }

    /// Writes the cache to `path` through a temporary file beside it, so a
    /// crash leaves the old file or the new one, never half of either.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let mut name = path.file_name().unwrap_or_default().to_owned();
        name.push(".tmp");
        let tmp = path.with_file_name(name);
        let mut file = std::fs::File::create(&tmp)?;
        let mut cache = self.clone();
        cache.version = CACHE_VERSION;
        file.write_all(&serde_json::to_vec_pretty(&cache)?)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// The latest stats of `key`'s player.
    pub fn get(&self, key: &ProfileKey) -> Option<&ProfileStats> {
        self.profiles.get(&key.cache_key()).map(|c| &c.latest)
    }

    /// Every snapshot kept of `key`'s player, oldest first.
    pub fn history(&self, key: &ProfileKey) -> &[ProfileStats] {
        self.profiles
            .get(&key.cache_key())
            .map_or(&[], |c| &c.history)
    }

    /// The latest stats of everyone: what [`lobby_strength`] takes.
    pub fn profiles(&self) -> Vec<ProfileStats> {
        self.profiles.values().map(|c| c.latest.clone()).collect()
    }

    /// Stores `stats` as its player's latest, and in the history when its
    /// ranked board differs from the last snapshot kept. Older stats than
    /// the latest held only go into the history.
    pub fn insert(&mut self, stats: ProfileStats) {
        let key = cache_key(&stats.profile_id, stats.family);
        self.missing.remove(&key);
        let entry = self.profiles.entry(key).or_insert_with(|| CachedProfile {
            latest: stats.clone(),
            history: Vec::new(),
        });
        if stats.ranked.is_some() {
            let at = entry
                .history
                .partition_point(|h| h.fetched_at <= stats.fetched_at);
            let before = at.checked_sub(1).map(|i| &entry.history[i]);
            if before.is_none_or(|b| b.ranked != stats.ranked) {
                entry.history.insert(at, stats.clone());
            }
        }
        if stats.fetched_at >= entry.latest.fetched_at {
            entry.latest = stats;
        }
    }

    /// Notes that the provider did not know `key`'s player at `at`.
    pub fn note_missing(&mut self, key: &ProfileKey, at: DateTime<Utc>) {
        self.missing.insert(key.cache_key(), Miss { at });
    }

    /// Whether `key`'s player was fetched, or found missing, less than
    /// `max_age` before `now`.
    pub fn is_fresh(&self, key: &ProfileKey, now: DateTime<Utc>, max_age: Duration) -> bool {
        let k = key.cache_key();
        let fetched = self.profiles.get(&k).map(|c| c.latest.fetched_at);
        let missed = self.missing.get(&k).map(|m| m.at);
        fetched.max(missed).is_some_and(|at| now - at < max_age)
    }

    /// Fetches `keys` from `source` and stores what comes back; players
    /// left out of the answer are noted as missing at `now`. Returns how
    /// many profiles were stored.
    pub fn refresh(
        &mut self,
        source: &dyn ProfileSource,
        keys: &[ProfileKey],
        now: DateTime<Utc>,
    ) -> Result<usize> {
        if keys.is_empty() {
            return Ok(0);
        }
        let fetched = source.fetch(keys)?;
        let count = fetched.len();
        for stats in fetched {
            self.insert(stats);
        }
        for key in keys {
            if !self.profiles.contains_key(&key.cache_key()) {
                self.note_missing(key, now);
            }
        }
        Ok(count)
    }
}

/// Every player of a match that has a profile id, each once.
pub fn keys_of(summary: &MatchSummary) -> Vec<ProfileKey> {
    let mut out: Vec<ProfileKey> = Vec::new();
    for p in summary.teams.iter().flat_map(|t| &t.players) {
        if p.profile_id.is_empty() {
            continue;
        }
        let key = ProfileKey::of(p);
        if !out.iter().any(|k| k.cache_key() == key.cache_key()) {
            out.push(key);
        }
    }
    out
}

/// The players of `summary` to fetch: those `cache` has nothing on, or
/// nothing younger than `max_age` at `now`. The recording player comes
/// first, then teammates, then opponents, so a caller short on calls
/// spends them in that order.
pub fn requests_for(
    summary: &MatchSummary,
    cache: &ProfileCache,
    now: DateTime<Utc>,
    max_age: Duration,
) -> Vec<ProfileKey> {
    let you = summary.recording.profile_id.to_ascii_lowercase();
    let mut keys = keys_of(summary);
    keys.retain(|k| !cache.is_fresh(k, now, max_age));
    let teams = &summary.teams;
    let team_of = |k: &ProfileKey| {
        (teams.iter()).position(|t| t.players.iter().any(|p| ProfileKey::of(p) == *k))
    };
    keys.sort_by_key(|k| {
        if k.profile_id == you {
            0
        } else if summary.your_team.is_some() && team_of(k) == summary.your_team {
            1
        } else {
            2
        }
    });
    keys
}

// ---------------------------------------------------------------------------
// Lobby strength
// ---------------------------------------------------------------------------

/// What a lobby's numbers are based on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Basis {
    /// Ranked rank points: both teams have at least one ranked player
    /// known.
    RankPoints,
    /// A fallback: clearance levels, because ranks are missing for a team.
    /// Level counts time played, not skill.
    Level,
    /// Neither is known for both teams.
    #[default]
    None,
}

/// Mean, median and spread of one set of values.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spread {
    /// How many values it is over.
    pub known: usize,
    pub mean: f64,
    pub median: f64,
    pub min: f64,
    pub max: f64,
    /// Population standard deviation.
    pub deviation: f64,
}

impl Spread {
    fn of(values: &[f64]) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let n = sorted.len();
        let mean = sorted.iter().sum::<f64>() / n as f64;
        let median = if n % 2 == 1 {
            sorted[n / 2]
        } else {
            (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
        };
        let variance = sorted.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64;
        Some(Self {
            known: n,
            mean,
            median,
            min: sorted[0],
            max: sorted[n - 1],
            deviation: variance.sqrt(),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamStrength {
    /// Players listed on the team: more than five when someone left and
    /// another joined.
    pub players: usize,
    /// Ranked rank points of the players who hold a rank.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_points: Option<Spread>,
    /// The ranks the mean and median rank points fall in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean_rank: Option<Rank>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_rank: Option<Rank>,
    /// Clearance levels: the replay's, else the profile's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<Spread>,
}

/// One player of the lobby with what was joined to them.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LobbyPlayer {
    pub username: String,
    #[serde(rename = "profileID", skip_serializing_if = "String::is_empty")]
    pub profile_id: String,
    /// Index into the summary's `teams`.
    pub team: usize,
    /// A profile was found for the player, ranked or not.
    pub profile: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<Rank>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_points: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    /// Where the player stands among the others of the lobby on the
    /// lobby's basis, 0 to 100: the share of them below, ties counting
    /// half.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percentile: Option<f64>,
}

/// How strong a lobby was, from stats fetched outside the replay.
///
/// The stats are as of when they were fetched, not as of the match: a
/// lobby looked up weeks later is measured by where its players are now.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LobbyStrength {
    /// What `difference` and the percentiles are measured in.
    pub basis: Basis,
    /// Players in the lobby, and how many of them have a profile, ranked
    /// rank points, a level.
    pub players: usize,
    pub profiles_known: usize,
    pub ranks_known: usize,
    pub levels_known: usize,
    /// Indexed like the summary's `teams`.
    pub teams: [TeamStrength; 2],
    /// Rank points over the whole lobby.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_points: Option<Spread>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean_rank: Option<Rank>,
    /// The team the difference is seen from: the recording player's, else
    /// team 0.
    pub from_team: usize,
    /// Mean of `from_team` minus mean of the other, in the basis's unit
    /// (rank points, or levels on the fallback).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub difference: Option<f64>,
    /// Chance `from_team` wins, from the rank point difference alone:
    /// `1 / (1 + 10^(-difference / 400))`, the Elo curve with its usual
    /// scale. A model, not the game's own figure, and not fitted to
    /// matches; only on the rank point basis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_win: Option<f64>,
    /// The recording player's percentile in the lobby.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub your_percentile: Option<f64>,
    /// The longest time between the match and a profile used, in seconds:
    /// how stale the picture can be.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fetch_gap_seconds: Option<i64>,
    pub lobby: Vec<LobbyPlayer>,
}

/// Rank points per factor of ten in the odds, in [`expected_win`].
pub const ELO_SCALE: f64 = 400.0;

/// The Elo expectation for a side `difference` rank points ahead.
pub fn expected_win(difference: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf(-difference / ELO_SCALE))
}

/// Joins `profiles` onto the players of `summary` by profile id and
/// measures the lobby.
pub fn lobby_strength(summary: &MatchSummary, profiles: &[ProfileStats]) -> LobbyStrength {
    let mut lobby = Vec::new();
    let mut gap: Option<i64> = None;
    for (t, team) in summary.teams.iter().enumerate() {
        for p in &team.players {
            let profile = profile_of(p, profiles);
            if let Some(s) = profile {
                let g = (s.fetched_at - summary.start_time).num_seconds().abs();
                gap = gap.max(Some(g));
            }
            let ranked = profile.and_then(|s| s.ranked.as_ref());
            let rank_points = ranked.and_then(BoardStats::rated_points);
            lobby.push(LobbyPlayer {
                username: p.username.clone(),
                profile_id: p.profile_id.clone(),
                team: t,
                profile: profile.is_some(),
                rank: ranked.and_then(|b| b.rank.clone()),
                rank_points,
                level: p.level.or(profile.and_then(|s| s.level)),
                percentile: None,
            });
        }
    }

    let points_of = |t: Option<usize>| -> Vec<f64> {
        (lobby.iter())
            .filter(|p| t.is_none_or(|t| p.team == t))
            .filter_map(|p| p.rank_points.map(f64::from))
            .collect()
    };
    let levels_of = |t: usize| -> Vec<f64> {
        (lobby.iter())
            .filter(|p| p.team == t)
            .filter_map(|p| p.level.map(f64::from))
            .collect()
    };
    let system = RankSystem::for_season(
        profiles
            .iter()
            .filter(|s| {
                lobby
                    .iter()
                    .any(|p| s.profile_id.eq_ignore_ascii_case(&p.profile_id))
            })
            .find_map(|s| s.season),
    );
    let rank_at = |points: f64| system.rank_of_points(points.round() as i32);

    let teams = [0, 1].map(|t| {
        let rank_points = Spread::of(&points_of(Some(t)));
        TeamStrength {
            players: lobby.iter().filter(|p| p.team == t).count(),
            mean_rank: rank_points.map(|s| rank_at(s.mean)),
            median_rank: rank_points.map(|s| rank_at(s.median)),
            rank_points,
            level: Spread::of(&levels_of(t)),
        }
    });

    let basis = if teams.iter().all(|t| t.rank_points.is_some()) {
        Basis::RankPoints
    } else if teams.iter().all(|t| t.level.is_some()) {
        Basis::Level
    } else {
        Basis::None
    };
    let value = |p: &LobbyPlayer| match basis {
        Basis::RankPoints => p.rank_points.map(f64::from),
        Basis::Level => p.level.map(f64::from),
        Basis::None => None,
    };
    let percentiles: Vec<Option<f64>> = lobby
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let own = value(p)?;
            let others: Vec<f64> = (lobby.iter().enumerate())
                .filter(|(j, _)| *j != i)
                .filter_map(|(_, q)| value(q))
                .collect();
            if others.is_empty() {
                return None;
            }
            let below = others.iter().filter(|&&v| v < own).count() as f64;
            let equal = others.iter().filter(|&&v| v == own).count() as f64;
            Some(100.0 * (below + equal / 2.0) / others.len() as f64)
        })
        .collect();

    let from_team = summary.your_team.unwrap_or(0);
    let mean = |t: &TeamStrength| match basis {
        Basis::RankPoints => t.rank_points.map(|s| s.mean),
        Basis::Level => t.level.map(|s| s.mean),
        Basis::None => None,
    };
    let difference = mean(&teams[from_team])
        .zip(mean(&teams[from_team ^ 1]))
        .map(|(a, b)| a - b);
    let you = &summary.recording.profile_id;
    let rank_points = Spread::of(&points_of(None));
    let mean_rank = rank_points.map(|s| rank_at(s.mean));
    for (p, pct) in lobby.iter_mut().zip(percentiles) {
        p.percentile = pct;
    }

    LobbyStrength {
        basis,
        players: lobby.len(),
        profiles_known: lobby.iter().filter(|p| p.profile).count(),
        ranks_known: lobby.iter().filter(|p| p.rank_points.is_some()).count(),
        levels_known: lobby.iter().filter(|p| p.level.is_some()).count(),
        teams,
        mean_rank,
        rank_points,
        from_team,
        difference,
        expected_win: difference
            .filter(|_| basis == Basis::RankPoints)
            .map(expected_win),
        your_percentile: (lobby.iter())
            .find(|p| !you.is_empty() && p.profile_id.eq_ignore_ascii_case(you))
            .and_then(|p| p.percentile),
        max_fetch_gap_seconds: gap,
        lobby,
    }
}

// ---------------------------------------------------------------------------
// Rank progress
// ---------------------------------------------------------------------------

/// What a rank point change between two snapshots can be pinned on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Attribution {
    /// The provider counted exactly one more ranked match, and exactly one
    /// ranked match of the replays ended between the snapshots: the change
    /// is that match's.
    Match,
    /// Exactly one ranked match of the replays ended between the
    /// snapshots, but the provider gave no match counts, so a match
    /// without a replay may share the change.
    MatchUnconfirmed,
    /// Anything else: several matches, none, or a count that disagrees
    /// with the replays. The change belongs to the span as a whole.
    Span,
}

/// The stretch between two consecutive snapshots.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressSpan {
    #[serde(with = "ts")]
    pub from: DateTime<Utc>,
    #[serde(with = "ts")]
    pub to: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_points_before: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_points_after: Option<i32>,
    /// `None` when either end has no rank, and across a season change,
    /// where the reset is not a result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change: Option<i32>,
    /// The snapshots are of different seasons.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub season_changed: bool,
    /// Ranked matches the provider counted in the span: the growth of
    /// wins, losses and abandons.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_matches: Option<u32>,
    /// `matchID`s of the ranked matches of the replays the player was in
    /// that ended in the span, oldest first.
    pub replay_matches: Vec<String>,
    pub attribution: Attribution,
    /// The match the change is attributed to, unless `attribution` is
    /// `span`.
    #[serde(rename = "matchID", skip_serializing_if = "Option::is_none")]
    pub match_id: Option<String>,
    /// Whether the player's team won that match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub won: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PeakSource {
    /// The provider's `max_rank_points`: the season's true peak.
    Provider,
    /// The highest snapshot: a lower bound, peaks between snapshots are
    /// not seen.
    Snapshots,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Peak {
    pub rank_points: i32,
    pub rank: Rank,
    pub source: PeakSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Rising,
    Falling,
    Flat,
}

/// The change over the latest spans of the current season.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trend {
    /// Spans it is over: up to [`TREND_SPANS`], those with a change.
    pub spans: usize,
    pub change: i32,
    /// Ranked matches the provider counted over them, when it counted in
    /// every one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matches: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_match: Option<f64>,
    /// The sign of `change`.
    pub direction: Direction,
}

/// Spans [`Trend`] looks back over.
pub const TREND_SPANS: usize = 5;

/// One player's rank over time, from snapshots of their profile.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankProgress {
    #[serde(rename = "profileID", skip_serializing_if = "String::is_empty")]
    pub profile_id: String,
    /// Snapshots used: those of the player with a ranked board.
    pub snapshots: usize,
    /// The latest snapshot's season.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    /// As of the latest snapshot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_points: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<Rank>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_division: Option<Target>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_tier: Option<Target>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub season_peak: Option<Peak>,
    /// One per pair of consecutive snapshots, oldest first.
    pub spans: Vec<ProgressSpan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trend: Option<Trend>,
}

/// Rank progress of one player from `history`, snapshots of their profile
/// in any order (those of the latest snapshot's profile id and family are
/// used), set against `matches`.
///
/// A snapshot says where the player stood when it was fetched and nothing
/// of how they got there. The change between two snapshots is one match's
/// only when exactly one ranked match lies between them, which takes a
/// fetch before and after every match; otherwise it is the span's, and no
/// split between its matches is made up.
pub fn rank_progress(history: &[ProfileStats], matches: &[MatchSummary]) -> RankProgress {
    let Some(latest) = history.iter().max_by_key(|s| s.fetched_at) else {
        return RankProgress::default();
    };
    let id = &latest.profile_id;
    let mut snaps: Vec<(&ProfileStats, &BoardStats)> = history
        .iter()
        .filter(|s| s.profile_id.eq_ignore_ascii_case(id) && s.family == latest.family)
        .filter_map(|s| Some((s, s.ranked.as_ref()?)))
        .collect();
    snaps.sort_by_key(|(s, _)| s.fetched_at);

    // The player's ranked matches by when they ended, each match once.
    let mut played: Vec<(DateTime<Utc>, &MatchSummary, usize)> = Vec::new();
    for m in matches.iter().filter(|m| m.queue == "ranked") {
        let team = (m.teams.iter()).position(|t| {
            t.players
                .iter()
                .any(|p| p.profile_id.eq_ignore_ascii_case(id))
        });
        let Some(team) = team else { continue };
        if m.match_id.is_empty() || !played.iter().any(|(_, o, _)| o.match_id == m.match_id) {
            played.push((m.end_time.unwrap_or(m.start_time), m, team));
        }
    }
    played.sort_by_key(|(t, _, _)| *t);

    let spans: Vec<ProgressSpan> = snaps
        .windows(2)
        .map(|w| {
            let ((a, before), (b, after)) = (w[0], w[1]);
            let season_changed = before.season != after.season;
            let (rp_before, rp_after) = (before.rated_points(), after.rated_points());
            let between: Vec<_> = played
                .iter()
                .filter(|(t, _, _)| *t > a.fetched_at && *t <= b.fetched_at)
                .collect();
            let provider_matches = match (before.matches(), after.matches()) {
                (Some(x), Some(y)) if !season_changed => y.checked_sub(x),
                _ => None,
            };
            let change = rp_before
                .zip(rp_after)
                .filter(|_| !season_changed)
                .map(|(x, y)| y - x);
            let attribution = match (between.len(), provider_matches) {
                _ if change.is_none() => Attribution::Span,
                (1, Some(1)) => Attribution::Match,
                (1, None) => Attribution::MatchUnconfirmed,
                _ => Attribution::Span,
            };
            let one = (attribution != Attribution::Span).then(|| between[0]);
            ProgressSpan {
                from: a.fetched_at,
                to: b.fetched_at,
                rank_points_before: rp_before,
                rank_points_after: rp_after,
                change,
                season_changed,
                provider_matches,
                replay_matches: between.iter().map(|(_, m, _)| m.match_id.clone()).collect(),
                attribution,
                match_id: one.map(|(_, m, _)| m.match_id.clone()),
                won: one.and_then(|(_, m, team)| Some(m.result.winner? == *team)),
            }
        })
        .collect();

    let board = snaps.last().map(|(_, b)| *b);
    let season = board.and_then(|b| b.season);
    let system = RankSystem::for_season(season);
    let rank_points = board.and_then(BoardStats::rated_points);

    let seen = (snaps.iter())
        .filter(|(_, b)| b.season == season)
        .filter_map(|(_, b)| b.rated_points())
        .max();
    let provided = board.and_then(|b| b.max_rank_points).filter(|&p| p > 0);
    let season_peak = match (provided, seen) {
        (Some(p), _) => Some((p.max(seen.unwrap_or(p)), PeakSource::Provider)),
        (None, Some(s)) => Some((s, PeakSource::Snapshots)),
        (None, None) => None,
    }
    .map(|(points, source)| Peak {
        rank_points: points,
        rank: system.rank_of_points(points),
        source,
    });

    let recent: Vec<&ProgressSpan> = (spans.iter().rev())
        .take_while(|s| !s.season_changed)
        .filter(|s| s.change.is_some())
        .take(TREND_SPANS)
        .collect();
    let trend = (!recent.is_empty()).then(|| {
        let change: i32 = recent.iter().filter_map(|s| s.change).sum();
        let matches = (recent.iter())
            .map(|s| s.provider_matches)
            .sum::<Option<u32>>()
            .filter(|&n| n > 0);
        Trend {
            spans: recent.len(),
            change,
            matches,
            per_match: matches.map(|n| f64::from(change) / f64::from(n)),
            direction: match change {
                1.. => Direction::Rising,
                0 => Direction::Flat,
                _ => Direction::Falling,
            },
        }
    });

    RankProgress {
        profile_id: id.clone(),
        snapshots: snaps.len(),
        season,
        rank_points,
        rank: board.and_then(|b| b.rank.clone()),
        next_division: rank_points.and_then(|p| system.next_division(p)),
        next_tier: rank_points.and_then(|p| system.next_tier(p)),
        season_peak,
        spans,
        trend,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary::TeamSummary;
    use serde_json::json;

    fn at(hours: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + Duration::hours(hours)
    }

    fn id(n: u32) -> String {
        format!("00000000-0000-4000-8000-{n:012}")
    }

    fn key(n: u32, platform: Option<Platform>) -> ProfileKey {
        ProfileKey {
            profile_id: id(n),
            platform,
            name: format!("Player{n}"),
            name_is_nickname: false,
        }
    }

    /// A provider response in Ubisoft's shape for player `n`.
    fn response(n: u32, family: &str, rank: u32, points: i32) -> Value {
        json!({
            "player": {"nameOnPlatform": format!("Player{n}"), "platformType": "uplay"},
            "account": {"level": 120 + n},
            "stats": {"platform_families_full_profiles": [{
                "profile_id": id(n),
                "platform_family": family,
                "board_ids_full_profiles": [
                    {"board_id": "ranked", "full_profiles": [{
                        "profile": {
                            "board_id": "ranked", "id": id(n), "season_id": 43,
                            "rank": rank, "rank_points": points,
                            "max_rank": rank, "max_rank_points": points + 40,
                            "top_rank_position": 0
                        },
                        "season_statistics": {
                            "kills": 300, "deaths": 200,
                            "match_outcomes": {"wins": 30, "losses": 20, "abandons": 1}
                        }
                    }]},
                    {"board_id": "standard", "full_profiles": [{
                        "profile": {"season_id": 43, "rank": 0, "rank_points": 0},
                        "season_statistics": {
                            "kills": 10, "deaths": 12,
                            "match_outcomes": {"wins": 2, "losses": 1, "abandons": 0}
                        }
                    }]}
                ]
            }]}
        })
    }

    fn stats(n: u32, points: Option<i32>, when: DateTime<Utc>) -> ProfileStats {
        let mut s = ProfileStats::new(id(n), when);
        s.family = Some(Family::Pc);
        s.season = Some(43);
        s.ranked = points.map(|p| BoardStats {
            season: Some(43),
            rank: Some(RankSystem::RANKED_3.rank_of_points(p)),
            rank_points: Some(p),
            ..BoardStats::default()
        });
        s
    }

    fn player(n: u32, level: Option<u32>) -> PlayerSummary {
        PlayerSummary {
            username: format!("Player{n}"),
            key: id(n),
            profile_id: id(n),
            level,
            platform: Some(Platform::Pc),
            ..PlayerSummary::default()
        }
    }

    /// Players 1 to 5 against 6 to 10, recorded by player 1, starting at
    /// hour `hour` and lasting half an hour.
    fn lobby(match_id: &str, queue: &'static str, hour: i64) -> MatchSummary {
        let team = |from: u32| TeamSummary {
            players: (from..from + 5).map(|n| player(n, Some(100 + n))).collect(),
            ..TeamSummary::default()
        };
        let mut m = MatchSummary {
            match_id: match_id.to_owned(),
            start_time: at(hour),
            end_time: Some(at(hour) + Duration::minutes(30)),
            queue,
            teams: [team(1), team(6)],
            your_team: Some(0),
            ..MatchSummary::default()
        };
        m.recording.profile_id = id(1);
        m.result.winner = Some(0);
        m
    }

    #[test]
    fn rank_table_round_trips() {
        for system in [RankSystem::RANKED_2, RankSystem::RANKED_3] {
            for rank in 1..=system.top() {
                let floor = system.floor(rank).unwrap();
                assert_eq!(system.rank_of_points(floor).id, rank);
                assert_eq!(system.rank_of_points(floor + 99).id, rank);
                if rank > 1 {
                    assert_eq!(system.rank_of_points(floor - 1).id, rank - 1);
                }
                assert!(system.name_of(rank).is_some());
            }
            assert_eq!(system.name_of(system.top() + 1), None);
            assert_eq!(system.floor(0), None);
            assert_eq!(system.rank_of_points(0).name, "Copper V");
            assert_eq!(system.rank_of_points(1000).name, "Copper V");
            assert_eq!(system.rank_of_points(2850).name, "Gold II");
            assert_eq!(system.rank_of_points(3300).name, "Platinum II");
            assert_eq!(system.rank_of_points(4499).name, "Diamond I");
        }
        assert_eq!(RankSystem::RANKED_2.rank_of_points(9000).name, "Champion");
        assert_eq!(RankSystem::RANKED_2.top(), 36);
        assert_eq!(RankSystem::RANKED_3.rank_of_points(4500).name, "Champion V");
        assert_eq!(RankSystem::RANKED_3.rank_of_points(9000).name, "Champion I");
        assert_eq!(RankSystem::RANKED_3.top(), 40);
        assert_eq!(RankSystem::RANKED_3.rank(77).name, "Rank(77)");
        assert_eq!(RankSystem::for_season(Some(41)), &RankSystem::RANKED_2);
        assert_eq!(RankSystem::for_season(Some(42)), &RankSystem::RANKED_3);
        assert_eq!(RankSystem::for_season(None), &RankSystem::RANKED_3);
    }

    #[test]
    fn next_division_and_tier() {
        let s = RankSystem::RANKED_3;
        let next = s.next_division(2850).unwrap();
        assert_eq!(
            (next.rank.name.as_str(), next.at, next.points),
            ("Gold I", 2900, 50)
        );
        let tier = s.next_tier(2850).unwrap();
        assert_eq!((tier.rank.name.as_str(), tier.points), ("Platinum V", 150));
        let tier = s.next_tier(4450).unwrap();
        assert_eq!((tier.rank.name.as_str(), tier.points), ("Champion V", 50));
        assert_eq!(s.next_tier(4600), None);
        assert_eq!(s.next_division(4600).unwrap().rank.name, "Champion III");
        assert_eq!(s.next_division(4950), None);
        assert_eq!(RankSystem::RANKED_2.next_division(4700), None);
    }

    #[test]
    fn a_full_response_is_read() {
        let v = response(1, "pc", 19, 2850);
        let got = from_r6data_json(&v.to_string(), &key(1, Some(Platform::Pc)), at(0)).unwrap();
        assert_eq!(got.len(), 1);
        let s = &got[0];
        assert_eq!(s.profile_id, id(1));
        assert!(s.id_confirmed);
        assert_eq!(s.platform, Some(Platform::Pc));
        assert_eq!(s.family, Some(Family::Pc));
        assert_eq!(s.username.as_deref(), Some("Player1"));
        assert_eq!((s.level, s.season), (Some(121), Some(43)));
        let r = s.ranked.as_ref().unwrap();
        assert_eq!(
            r.rank,
            Some(Rank {
                id: 19,
                name: "Gold II".into()
            })
        );
        assert_eq!((r.rank_points, r.max_rank_points), (Some(2850), Some(2890)));
        assert_eq!(r.max_rank.as_ref().unwrap().name, "Gold II");
        assert_eq!(
            (r.wins, r.losses, r.abandons),
            (Some(30), Some(20), Some(1))
        );
        assert_eq!(
            (r.kills, r.deaths, r.top_position),
            (Some(300), Some(200), None)
        );
        assert_eq!(
            (r.matches(), r.kd(), r.win_rate()),
            (Some(51), Some(1.5), Some(0.6))
        );
        assert_eq!(s.rank_points(), Some(2850));
        let standard = &s.boards["standard"];
        assert_eq!(standard.rank.as_ref().unwrap().name, "Unranked");
        assert_eq!(standard.rated_points(), None);
        assert_eq!(standard.wins, Some(2));
    }

    #[test]
    fn the_documented_flat_example_is_read() {
        // The provider's documented board example: `season_id` beside
        // `profile`, the season's counts inside it, `abandon` singular,
        // and rank ids that do not follow its own rank points.
        let v = json!({"stats": {"platform_families_full_profiles": [{
            "profile_id": id(2),
            "board_ids_full_profiles": [{"board_id": "ranked", "full_profiles": [{
                "season_id": 40,
                "profile": {
                    "rank": 18, "rank_points": 3300, "max_rank": 19,
                    "max_rank_points": 3450, "kills": 1240, "deaths": 1180,
                    "wins": 64, "losses": 51, "abandon": 2,
                    "update_time": "2025-10-14T21:43:27.315Z"
                }
            }]}]
        }]}});
        let s = &from_r6data_value(&v, &key(2, None), at(0)).unwrap()[0];
        let r = s.ranked.as_ref().unwrap();
        assert_eq!(s.season, Some(40));
        assert_eq!(r.rank.as_ref().unwrap().name, "Platinum II");
        assert_eq!(r.max_rank.as_ref().unwrap().name, "Platinum I");
        assert_eq!((r.provider_rank, r.provider_max_rank), (Some(18), Some(19)));
        assert_eq!(
            (r.wins, r.losses, r.abandons),
            (Some(64), Some(51), Some(2))
        );
        assert_eq!(r.updated_at.as_deref(), Some("2025-10-14T21:43:27.315Z"));
        assert_eq!(s.family, None);
        assert_eq!(s.platform, None);
    }

    #[test]
    fn missing_fields_are_tolerated() {
        // Nothing but a level: an account with no ranked profile.
        let v =
            json!({"account": {"level": "57"}, "stats": {"platform_families_full_profiles": []}});
        let got = from_r6data_value(&v, &key(3, Some(Platform::Xbox)), at(0)).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].profile_id, id(3));
        assert!(!got[0].id_confirmed);
        assert_eq!(got[0].level, Some(57));
        assert_eq!(got[0].family, Some(Family::Console));
        assert_eq!(got[0].ranked, None);
        assert_eq!(got[0].rank_points(), None);

        // An empty object, nulls, wrong types, a board without an id.
        for v in [
            json!({}),
            json!({"account": null, "player": {"nameOnPlatform": null}, "stats": null}),
            json!({"account": {"level": [1]}, "stats": {"platform_families_full_profiles": "no"}}),
            json!({"stats": {"platform_families_full_profiles": [{
                "board_ids_full_profiles": [{"full_profiles": []}, {"board_id": "ranked"}, 7]
            }]}}),
        ] {
            let got = from_r6data_value(&v, &key(3, None), at(0)).unwrap();
            assert_eq!(got.len(), 1, "{v}");
            assert_eq!(got[0].level, None);
            assert_eq!(got[0].rank_points(), None);
        }
        assert!(matches!(
            from_r6data_json("not json", &key(3, None), at(0)),
            Err(ProfileError::Json(_))
        ));
    }

    #[test]
    fn unranked_and_unknown_ranks() {
        // Unranked: rank 0, though the board carries points.
        let v = response(4, "pc", 0, 1000);
        let s = &from_r6data_value(&v, &key(4, None), at(0)).unwrap()[0];
        let r = s.ranked.as_ref().unwrap();
        assert_eq!(
            r.rank,
            Some(Rank {
                id: 0,
                name: "Unranked".into()
            })
        );
        assert_eq!(r.rank_points, Some(1000));
        assert_eq!(s.rank_points(), None);

        // An id the table lacks is named from the points...
        let v = response(4, "pc", 99, 3120);
        let s = &from_r6data_value(&v, &key(4, None), at(0)).unwrap()[0];
        let r = s.ranked.as_ref().unwrap();
        assert_eq!(r.rank.as_ref().unwrap().name, "Platinum IV");
        assert_eq!(r.provider_rank, Some(99));

        // ...and kept as it is without them.
        let v = json!({"stats": {"platform_families_full_profiles": [{
            "board_ids_full_profiles": [{"board_id": "ranked", "full_profiles": [
                {"profile": {"rank": 99}}
            ]}]
        }]}});
        let s = &from_r6data_value(&v, &key(4, None), at(0)).unwrap()[0];
        let r = s.ranked.as_ref().unwrap();
        assert_eq!(
            r.rank,
            Some(Rank {
                id: 99,
                name: "Rank(99)".into()
            })
        );
        assert_eq!(r.rated_points(), None);
    }

    #[test]
    fn console_platforms_and_families() {
        // An account with a PC and a console profile, looked up for its
        // PlayStation player.
        let mut v = response(5, "pc", 12, 2150);
        let console =
            response(5, "console", 22, 3180)["stats"]["platform_families_full_profiles"][0].clone();
        v["stats"]["platform_families_full_profiles"]
            .as_array_mut()
            .unwrap()
            .push(console);
        v["player"]["platformType"] = json!("psn");
        let k = key(5, Some(Platform::PlayStation));
        assert_eq!(
            (k.platform_type(), k.platform_families()),
            ("psn", "console")
        );
        assert_eq!(key(5, Some(Platform::Xbox)).platform_type(), "xbl");
        assert_eq!(key(5, None).platform_type(), "uplay");
        assert_eq!(key(5, None).platform_families(), "pc");

        let got = from_r6data_value(&v, &k, at(0)).unwrap();
        assert_eq!(got.len(), 2);
        assert!(
            got.iter()
                .all(|s| s.platform == Some(Platform::PlayStation))
        );
        let mut p = player(5, None);
        p.platform = Some(Platform::PlayStation);
        assert_eq!(profile_of(&p, &got).unwrap().rank_points(), Some(3180));
        p.platform = Some(Platform::Pc);
        assert_eq!(profile_of(&p, &got).unwrap().rank_points(), Some(2150));
        p.platform = None;
        assert_eq!(profile_of(&p, &got).unwrap().rank_points(), Some(2150));
        assert!(profile_of(&player(6, None), &got).is_none());
    }

    #[test]
    fn another_account_under_the_name_is_refused() {
        let v = response(7, "pc", 19, 2850);
        let err = from_r6data_value(&v, &key(8, None), at(0)).unwrap_err();
        assert!(matches!(err, ProfileError::Mismatch { got, .. } if got == id(7)));
        // Ids compare without case.
        let mut k = key(7, None);
        k.profile_id = k.profile_id.to_ascii_uppercase();
        assert_eq!(
            from_r6data_value(&v, &k, at(0)).unwrap()[0].profile_id,
            id(7)
        );
    }

    #[test]
    fn stats_round_trip_as_json() {
        let v = response(1, "console", 19, 2850);
        let s = from_r6data_value(&v, &key(1, Some(Platform::Xbox)), at(0)).unwrap();
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains("\"profileID\""));
        assert!(text.contains("\"fetchedAt\":\"2026-09-01T12:00:00Z\""));
        assert!(text.contains("\"rankPoints\":2850"));
        let back: Vec<ProfileStats> = serde_json::from_str(&text).unwrap();
        assert_eq!(back, s);
        // Our own type reads with everything optional left out.
        let bare: ProfileStats =
            serde_json::from_str(r#"{"profileID":"x","fetchedAt":"2026-09-01T14:00:00+02:00"}"#)
                .unwrap();
        assert_eq!(bare.fetched_at, at(0));
        assert_eq!(bare.ranked, None);
    }

    struct Fixed(Vec<ProfileStats>);

    impl ProfileSource for Fixed {
        fn fetch(&self, keys: &[ProfileKey]) -> Result<Vec<ProfileStats>> {
            let wanted = |s: &&ProfileStats| keys.iter().any(|k| k.profile_id == s.profile_id);
            Ok(self.0.iter().filter(wanted).cloned().collect())
        }
    }

    #[test]
    fn the_cache_expires_and_lists_who_to_fetch() {
        let m = lobby("m1", "ranked", 0);
        let day = Duration::hours(24);
        let mut cache = ProfileCache::new();

        // Nothing cached: all ten, you first, then your team.
        let all = requests_for(&m, &cache, at(1), day);
        assert_eq!(all.len(), 10);
        assert_eq!(all[0].profile_id, id(1));
        let ids: Vec<_> = all.iter().map(|k| k.profile_id.clone()).collect();
        assert_eq!(ids, (1..=10).map(id).collect::<Vec<_>>());

        // The source knows players 1 to 8; 9 and 10 are noted as missing.
        let source = Fixed(
            (1..=8)
                .map(|n| stats(n, Some(2000 + 100 * n as i32), at(1)))
                .collect(),
        );
        assert_eq!(cache.refresh(&source, &all, at(1)).unwrap(), 8);
        assert_eq!(cache.len(), 8);
        assert!(requests_for(&m, &cache, at(2), day).is_empty());
        assert!(cache.is_fresh(&all[9], at(2), day));

        // A day on, everything is due again.
        assert_eq!(requests_for(&m, &cache, at(25), day).len(), 10);
        // A fresher entry for one player takes them off the list.
        cache.insert(stats(3, Some(2400), at(20)));
        let due = requests_for(&m, &cache, at(25), day);
        assert_eq!(due.len(), 9);
        assert!(due.iter().all(|k| k.profile_id != id(3)));
        assert_eq!(cache.get(&all[2]).unwrap().rank_points(), Some(2400));

        // Players without a profile id cannot be fetched; a recorder on
        // the other team still comes first.
        let mut other = lobby("m2", "ranked", 0);
        other.teams[0].players[4].profile_id.clear();
        other.recording.profile_id = id(7);
        other.your_team = Some(1);
        let keys = requests_for(&other, &ProfileCache::new(), at(1), day);
        assert_eq!(keys.len(), 9);
        assert_eq!(keys[0].profile_id, id(7));
        assert_eq!(keys[1].profile_id, id(6));
        assert_eq!(keys[5].profile_id, id(1));
        assert!(cache.refresh(&source, &[], at(1)).is_ok());
    }

    #[test]
    fn a_nickname_is_not_a_name_to_look_up() {
        let mut p = player(1, None);
        p.uses_nickname = true;
        let k = ProfileKey::of(&p);
        assert!(k.name_is_nickname);
        p.renamed_to = Some("RealName".into());
        let k = ProfileKey::of(&p);
        assert_eq!((k.name.as_str(), k.name_is_nickname), ("RealName", false));
    }

    #[test]
    fn the_cache_keeps_a_history_of_changes_and_saves_atomically() {
        let k = key(1, Some(Platform::Pc));
        let mut cache = ProfileCache::new();
        cache.insert(stats(1, Some(2800), at(0)));
        cache.insert(stats(1, Some(2800), at(1)));
        cache.insert(stats(1, Some(2825), at(2)));
        // An older fetch arriving late lands in order, not as the latest.
        cache.insert(stats(1, Some(2780), at(-5)));
        let points: Vec<_> = cache.history(&k).iter().map(|s| s.rank_points()).collect();
        assert_eq!(points, [Some(2780), Some(2800), Some(2825)]);
        assert_eq!(cache.get(&k).unwrap().fetched_at, at(2));
        cache.note_missing(&key(2, None), at(2));

        let dir = std::env::temp_dir().join(format!("profiles-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cache.json");
        assert_eq!(ProfileCache::load(&path).unwrap(), ProfileCache::new());
        cache.save(&path).unwrap();
        cache.save(&path).unwrap();
        assert!(!dir.join("cache.json.tmp").exists());
        assert_eq!(ProfileCache::load(&path).unwrap(), cache);

        std::fs::write(&path, r#"{"version": 99}"#).unwrap();
        assert!(matches!(
            ProfileCache::load(&path),
            Err(ProfileError::CacheVersion(99))
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lobby_strength_with_everyone_known() {
        let m = lobby("m1", "ranked", 0);
        // Team 0: 2600..3000, team 1: 2500..2900.
        let profiles: Vec<_> = (1..=10u32)
            .map(|n| {
                let p = if n <= 5 {
                    2500 + 100 * n
                } else {
                    2400 + 100 * (n - 5)
                };
                stats(n, Some(p as i32), at(2))
            })
            .collect();
        let s = lobby_strength(&m, &profiles);
        assert_eq!(s.basis, Basis::RankPoints);
        assert_eq!(
            (s.players, s.profiles_known, s.ranks_known, s.levels_known),
            (10, 10, 10, 10)
        );
        let t0 = s.teams[0].rank_points.unwrap();
        assert_eq!(
            (t0.mean, t0.median, t0.min, t0.max),
            (2800.0, 2800.0, 2600.0, 3000.0)
        );
        assert!((t0.deviation - 20000f64.sqrt()).abs() < 1e-9);
        assert_eq!(s.teams[0].mean_rank.as_ref().unwrap().name, "Gold II");
        assert_eq!(s.teams[1].median_rank.as_ref().unwrap().name, "Gold III");
        assert_eq!(s.mean_rank.as_ref().unwrap().name, "Gold III");
        assert_eq!(s.difference, Some(100.0));
        let p = s.expected_win.unwrap();
        assert!((p - 1.0 / (1.0 + 10f64.powf(-0.25))).abs() < 1e-12);
        assert!(p > 0.63 && p < 0.65);
        // You are the lowest of your team: 2600, above 2500 and level
        // with the other 2600.
        assert_eq!(s.your_percentile, Some(100.0 * 1.5 / 9.0));
        assert_eq!(s.lobby[4].percentile, Some(100.0));
        assert_eq!(s.max_fetch_gap_seconds, Some(7200));
        assert_eq!(expected_win(0.0), 0.5);

        // Seen from the other team the difference turns round.
        let mut other = m.clone();
        other.your_team = Some(1);
        let o = lobby_strength(&other, &profiles);
        assert_eq!(o.difference, Some(-100.0));
        assert!((o.expected_win.unwrap() + p - 1.0).abs() < 1e-12);
    }

    #[test]
    fn lobby_strength_with_partial_knowledge() {
        let m = lobby("m1", "ranked", 0);
        // Three ranked, one unranked, one without a ranked board; the
        // rest unknown.
        let mut unranked = stats(3, Some(1000), at(1));
        unranked.ranked.as_mut().unwrap().rank = Some(RankSystem::RANKED_3.rank(0));
        let profiles = vec![
            stats(1, Some(3000), at(1)),
            stats(2, Some(3200), at(1)),
            unranked,
            stats(4, None, at(1)),
            stats(7, Some(2500), at(1)),
        ];
        let s = lobby_strength(&m, &profiles);
        assert_eq!(s.basis, Basis::RankPoints);
        assert_eq!((s.players, s.profiles_known, s.ranks_known), (10, 5, 3));
        assert_eq!(s.teams[0].rank_points.unwrap().known, 2);
        assert_eq!(s.teams[1].rank_points.unwrap().known, 1);
        assert_eq!(s.teams[1].rank_points.unwrap().deviation, 0.0);
        assert_eq!(s.difference, Some(600.0));
        assert_eq!(s.your_percentile, Some(50.0));
        assert_eq!(s.lobby[2].rank.as_ref().unwrap().name, "Unranked");
        assert_eq!(s.lobby[2].percentile, None);

        // No ranks on one team: levels stand in, and no win chance is
        // made of them.
        let s = lobby_strength(&m, &profiles[..2]);
        assert_eq!(s.basis, Basis::Level);
        assert_eq!(s.ranks_known, 2);
        assert_eq!(s.difference, Some(-5.0));
        assert_eq!(s.expected_win, None);
        assert_eq!(s.your_percentile, Some(0.0));
        assert_eq!(s.teams[0].level.unwrap().mean, 103.0);

        // Levels come from the profile when the replay has none.
        let mut bare = m.clone();
        for p in bare.teams.iter_mut().flat_map(|t| &mut t.players) {
            p.level = None;
        }
        let none = lobby_strength(&bare, &[]);
        assert_eq!(none.basis, Basis::None);
        assert_eq!((none.difference, none.your_percentile), (None, None));
        let mut a = stats(1, None, at(1));
        a.level = Some(200);
        let mut b = stats(6, None, at(1));
        b.level = Some(150);
        let s = lobby_strength(&bare, &[a, b]);
        assert_eq!((s.basis, s.difference), (Basis::Level, Some(50.0)));
        assert_eq!(s.levels_known, 2);
    }

    /// A snapshot of player 1 with `points` after `played` matches.
    fn snap(points: i32, played: u32, hour: i64) -> ProfileStats {
        let mut s = stats(1, Some(points), at(hour));
        let b = s.ranked.as_mut().unwrap();
        b.wins = Some(played);
        b.losses = Some(0);
        b.abandons = Some(0);
        s
    }

    #[test]
    fn progress_is_attributed_only_where_one_match_lies_between() {
        let matches = [
            lobby("m1", "ranked", 1),
            // The same match imported from a teammate counts once.
            lobby("m1", "ranked", 1),
            lobby("m2", "ranked", 5),
            lobby("m3", "ranked", 7),
            lobby("q1", "quickMatch", 11),
            lobby("m4", "ranked", 13),
        ];
        let mut lost = lobby("m5", "ranked", 16);
        lost.result.winner = Some(1);
        let mut matches = matches.to_vec();
        matches.push(lost);

        let mut no_counts = stats(1, Some(2960), at(15));
        no_counts.ranked.as_mut().unwrap().max_rank_points = Some(2990);
        let mut after_loss = stats(1, Some(2935), at(18));
        after_loss.ranked.as_mut().unwrap().max_rank_points = Some(2990);
        let history = vec![
            snap(2800, 10, 0),
            snap(2826, 11, 3),           // m1 alone
            snap(2870, 13, 9),           // m2 and m3
            snap(2870, 13, 12),          // only a quick match
            snap(2890, 15, 14),          // m4, and one more match without a replay
            no_counts,                   // a match nobody recorded, counts unknown
            after_loss,                  // m5, counts unknown
            stats(2, Some(4000), at(4)), // another player's snapshot
        ];
        let p = rank_progress(&history, &matches);
        assert_eq!(p.profile_id, id(1));
        assert_eq!((p.snapshots, p.spans.len()), (7, 6));

        let s = &p.spans[0];
        assert_eq!((s.change, s.provider_matches), (Some(26), Some(1)));
        assert_eq!(s.attribution, Attribution::Match);
        assert_eq!((s.match_id.as_deref(), s.won), (Some("m1"), Some(true)));
        assert_eq!(s.replay_matches, ["m1"]);

        let s = &p.spans[1];
        assert_eq!((s.change, s.provider_matches), (Some(44), Some(2)));
        assert_eq!(
            (s.attribution, s.match_id.as_deref()),
            (Attribution::Span, None)
        );
        assert_eq!(s.replay_matches, ["m2", "m3"]);

        let s = &p.spans[2];
        assert_eq!((s.change, s.provider_matches), (Some(0), Some(0)));
        assert!(s.replay_matches.is_empty());
        assert_eq!(s.attribution, Attribution::Span);

        // One replay, but the provider counted two matches.
        let s = &p.spans[3];
        assert_eq!((s.change, s.provider_matches), (Some(20), Some(2)));
        assert_eq!(s.replay_matches, ["m4"]);
        assert_eq!(
            (s.attribution, s.match_id.as_deref()),
            (Attribution::Span, None)
        );

        let s = &p.spans[4];
        assert_eq!((s.change, s.provider_matches), (Some(70), None));
        assert_eq!(s.attribution, Attribution::Span);

        let s = &p.spans[5];
        assert_eq!(s.change, Some(-25));
        assert_eq!(s.attribution, Attribution::MatchUnconfirmed);
        assert_eq!((s.match_id.as_deref(), s.won), (Some("m5"), Some(false)));

        assert_eq!((p.season, p.rank_points), (Some(43), Some(2935)));
        assert_eq!(p.rank.as_ref().unwrap().name, "Gold I");
        let next = p.next_division.as_ref().unwrap();
        assert_eq!((next.rank.name.as_str(), next.points), ("Platinum V", 65));
        assert_eq!(p.next_tier, p.next_division);
        let peak = p.season_peak.as_ref().unwrap();
        assert_eq!(
            (peak.rank_points, peak.source),
            (2990, PeakSource::Provider)
        );
        let t = p.trend.as_ref().unwrap();
        assert_eq!(
            (t.spans, t.change, t.direction),
            (5, 109, Direction::Rising)
        );
        assert_eq!((t.matches, t.per_match), (None, None));
    }

    #[test]
    fn progress_across_seasons_and_without_ranks() {
        assert_eq!(rank_progress(&[], &[]), RankProgress::default());

        let mut old = snap(3900, 80, 0);
        old.ranked.as_mut().unwrap().season = Some(42);
        let mut placing = snap(1000, 2, 50);
        placing.ranked.as_mut().unwrap().rank = Some(RankSystem::RANKED_3.rank(0));
        // Out of order on purpose.
        let history = vec![
            snap(3500, 7, 90),
            old,
            placing,
            snap(3460, 6, 80),
            stats(1, None, at(95)),
        ];
        let p = rank_progress(&history, &[]);
        assert_eq!((p.snapshots, p.spans.len()), (4, 3));
        assert!(p.spans[0].season_changed);
        assert_eq!(
            (p.spans[0].change, p.spans[0].provider_matches),
            (None, None)
        );
        // Unranked at one end: no change, though matches were counted.
        assert_eq!(
            (p.spans[1].change, p.spans[1].provider_matches),
            (None, Some(4))
        );
        assert_eq!(p.spans[1].attribution, Attribution::Span);
        assert_eq!(
            (p.spans[2].change, p.spans[2].provider_matches),
            (Some(40), Some(1))
        );
        // The provider counted one match, no replay holds it.
        assert_eq!(p.spans[2].attribution, Attribution::Span);

        // The peak is this season's, from snapshots when the provider
        // gives none: last season's 3900 does not count.
        let peak = p.season_peak.as_ref().unwrap();
        assert_eq!(
            (peak.rank_points, peak.source),
            (3500, PeakSource::Snapshots)
        );
        assert_eq!(peak.rank.name, "Emerald V");
        let t = p.trend.as_ref().unwrap();
        assert_eq!(
            (t.spans, t.change, t.matches, t.per_match),
            (1, 40, Some(1), Some(40.0))
        );

        // Falling and flat.
        let down = rank_progress(&[snap(3000, 1, 0), snap(2950, 3, 5)], &[]);
        let t = down.trend.unwrap();
        assert_eq!(
            (t.direction, t.per_match),
            (Direction::Falling, Some(-25.0))
        );
        let flat = rank_progress(&[snap(3000, 1, 0), snap(3000, 3, 5)], &[]);
        assert_eq!(flat.trend.unwrap().direction, Direction::Flat);
        // One snapshot: a standing and no spans.
        let one = rank_progress(&[snap(3000, 1, 0)], &[]);
        assert_eq!(
            (one.spans.len(), one.trend, one.rank_points),
            (0, None, Some(3000))
        );
    }
}
