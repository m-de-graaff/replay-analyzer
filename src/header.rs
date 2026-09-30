//! The key/value header at the start of every replay.

use std::collections::HashMap;
use std::str::FromStr;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Serialize, Serializer};

use crate::cursor::Cursor;
use crate::error::{Error, Result};
use crate::format::{self, FormatInfo};
use crate::types::{GameMode, Map, MatchType, Operator, TeamRole, WinCondition, version};

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub game_version: String,
    pub code_version: u32,
    #[serde(serialize_with = "rfc3339")]
    pub timestamp: DateTime<Utc>,
    pub match_type: MatchType,
    pub map: Map,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub site: String,
    #[serde(rename = "recordingPlayerID")]
    pub recording_player_id: u64,
    #[serde(
        rename = "recordingProfileID",
        skip_serializing_if = "String::is_empty"
    )]
    pub recording_profile_id: String,
    pub additional_tags: String,
    #[serde(rename = "gamemode")]
    pub game_mode: GameMode,
    pub rounds_per_match: u32,
    pub rounds_per_match_overtime: u32,
    pub round_number: u32,
    pub overtime_round_number: u32,
    pub teams: [Team; 2],
    pub players: Vec<Player>,
    pub gm_settings: Vec<i64>,
    #[serde(skip_serializing_if = "is_zero")]
    pub playlist_category: i64,
    #[serde(rename = "matchID")]
    pub match_id: String,
    /// When the recording started, UTC (Y11S3+ `starttime`). `timestamp` is
    /// the recording PC's local time.
    #[serde(
        serialize_with = "rfc3339_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub start_time: Option<DateTime<Utc>>,
    /// Whether a spectator recorded the match (Y11S3+ `isspectator`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_spectator: Option<bool>,
    /// Y11S3+ `maxnbplayersperteam`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_players_per_team: Option<u32>,
    /// Y11S3+ `matchresult`, written only on the round that decides the
    /// match. Its value matched the winning team's index in the one sample
    /// seen, so it is kept raw.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_result: Option<u32>,
    /// When the recording stopped, UTC (Y11S3+ `endtime`).
    #[serde(
        serialize_with = "rfc3339_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub end_time: Option<DateTime<Utc>>,
    /// Every header key read, with how often it appeared.
    #[serde(skip)]
    pub keys: Vec<(String, u32)>,
}

/// Header keys this parser uses. Others are kept only in the census.
pub const KNOWN_KEYS: &[&str] = &[
    "version",
    "code",
    "datetime",
    "matchtype",
    "worldid",
    "recordingplayerid",
    "recordingprofileid",
    "additionaltags",
    "gamemodeid",
    "roundspermatch",
    "roundspermatchovertime",
    "roundnumber",
    "overtimeroundnumber",
    "teamname0",
    "teamname1",
    "teamscore0",
    "teamscore1",
    "startingteamscore0",
    "startingteamscore1",
    "gmsetting",
    "playlistcategory",
    "id",
    "endtime",
    "starttime",
    "isspectator",
    "maxnbplayersperteam",
    "matchresult",
    "profileid",
    "playerid",
    "playername",
    "team",
    "heroname",
    "alliance",
    "roleimage",
    "rolename",
    "roleportrait",
];

/// Keys that describe one player; a run of them follows each `playerid`.
const PLAYER_KEYS: &[&str] = &[
    "playerid",
    "profileid",
    "playername",
    "team",
    "heroname",
    "alliance",
    "roleimage",
    "rolename",
    "roleportrait",
];

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub name: String,
    pub starting_score: u32,
    pub score: u32,
    pub won: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub win_condition: Option<WinCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<TeamRole>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Player {
    #[serde(skip_serializing_if = "is_zero")]
    pub id: u64,
    /// Ubisoft stats identifier.
    #[serde(rename = "profileID", skip_serializing_if = "String::is_empty")]
    pub profile_id: String,
    pub username: String,
    pub team_index: usize,
    pub operator: Operator,
    #[serde(skip_serializing_if = "is_zero")]
    pub hero_name: i64,
    pub alliance: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub role_image: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub role_name: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub role_portrait: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub spawn: String,
    /// Y11S3+, from the round's opening snapshot. Most likely the clearance
    /// level; see `decodeStatus.levels`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    /// The 4-byte id packets use to refer to this player.
    #[serde(skip)]
    pub dissect_id: Option<[u8; 4]>,
    /// Caster UI id (Y9S3+), used to attribute attacker operator swaps.
    #[serde(skip)]
    pub ui_id: u64,
    /// Id of the object holding the player's state (Y11S3 pick packets);
    /// attacker swaps are written to it.
    #[serde(skip)]
    pub state_id: Option<u32>,
}

impl Header {
    pub fn recording_player(&self) -> Option<&Player> {
        self.players
            .iter()
            .find(|p| p.id == self.recording_player_id)
    }
}

fn is_zero<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// A Unix time in milliseconds.
fn millis(value: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(value?.parse().ok()?)
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

const STRING_SEPARATOR: [u8; 7] = [0; 7];

/// Parses the header at the start of `data` (which begins with `dissect`).
/// Returns the header, the prelude, and the offset just past the header.
pub fn parse(data: &[u8]) -> Result<(Header, FormatInfo, usize)> {
    let mut c = Cursor::new(data, 0);
    if !data.starts_with(format::MAGIC) {
        return Err(Error::InvalidFile);
    }
    let prelude = format::read_prelude(&mut c);
    if prelude.is_none() {
        tracing::warn!("unrecognised prelude; scanning for the header properties");
        c.skip(7)?;
        skip_version_block(&mut c)?;
    }
    let count = prelude.as_ref().map(|p| p.property_count);
    let header = read_properties(&mut c, count)?;
    Ok((header, prelude.unwrap_or_default(), c.pos()))
}

/// Fallback for an unrecognised prelude: the properties start after the
/// second run of seven zero bytes.
fn skip_version_block(c: &mut Cursor) -> Result<()> {
    let (mut zeros, mut runs) = (0, 0);
    while runs < 2 {
        if c.u8()? == 0 {
            zeros += 1;
            if zeros == 7 {
                zeros = 0;
                runs += 1;
            }
        } else {
            zeros = 0;
        }
    }
    Ok(())
}

fn read_string(c: &mut Cursor) -> Result<String> {
    let len = c.u8()? as usize;
    if c.array::<7>()? != STRING_SEPARATOR {
        return Err(Error::InvalidStringSeparator(c.pos() - 7));
    }
    Ok(String::from_utf8_lossy(c.bytes(len)?).into_owned())
}

fn parse_num<T: FromStr>(key: &str, value: &str) -> Result<T> {
    value.parse().map_err(|_| Error::InvalidProperty {
        key: key.to_owned(),
        value: value.to_owned(),
    })
}

/// Reads `count` properties, or up to `teamscore1` (the last one before
/// Y11S3) when the count is unknown.
fn read_properties(c: &mut Cursor, count: Option<u32>) -> Result<Header> {
    let mut props: HashMap<String, String> = HashMap::new();
    let mut gm_settings = Vec::new();
    let mut players = Vec::new();
    // `Some` while inside a run of player properties.
    let mut player: Option<Player> = None;
    let mut keys: Vec<(String, u32)> = Vec::new();
    let mut read = 0;

    while match count {
        Some(n) => read < n,
        None => !props.contains_key("teamscore1"),
    } {
        let key = read_string(c)?;
        let value = read_string(c)?;
        read += 1;
        match keys.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += 1,
            None => keys.push((key.clone(), 1)),
        }

        if key == "playerid" {
            players.extend(player.replace(Player::default()));
        } else if !PLAYER_KEYS.contains(&key.as_str()) {
            // Y11S3 writes the players first, then the match properties;
            // older builds end the player run with `playlistcategory` or `id`.
            players.extend(player.take());
        }

        let Some(p) = player.as_mut() else {
            if key == "gmsetting" {
                gm_settings.push(parse_num(&key, &value)?);
            } else {
                props.insert(key, value);
            }
            continue;
        };
        match key.as_str() {
            "playerid" => p.id = parse_num(&key, &value)?,
            "playername" => p.username = value,
            "profileid" => p.profile_id = value,
            "team" => p.team_index = parse_num(&key, &value)?,
            "heroname" => p.hero_name = parse_num(&key, &value)?,
            "alliance" => p.alliance = parse_num(&key, &value)?,
            "roleimage" => p.role_image = parse_num(&key, &value)?,
            "rolename" => p.role_name = value,
            "roleportrait" => p.role_portrait = parse_num(&key, &value)?,
            _ => {
                props.insert(key, value);
            }
        }
    }

    let get = |key: &'static str| props.get(key).map(String::as_str);
    let req = |key: &'static str| get(key).ok_or(Error::MissingProperty(key));
    let num = |key: &'static str| -> Result<u64> { parse_num(key, req(key)?) };
    let small = |key: &'static str| -> Result<u32> { parse_num(key, req(key)?) };

    let datetime = req("datetime")?;
    let timestamp = NaiveDateTime::parse_from_str(datetime, "%Y-%m-%d-%H-%M-%S")
        .map_err(|_| Error::InvalidProperty {
            key: "datetime".into(),
            value: datetime.into(),
        })?
        .and_utc();

    let code_version = small("code")?;
    let mut teams: [Team; 2] = Default::default();
    for (i, team) in teams.iter_mut().enumerate() {
        let [name, score, starting] = [
            ["teamname0", "teamscore0", "startingteamscore0"],
            ["teamname1", "teamscore1", "startingteamscore1"],
        ][i];
        team.name = get(name).unwrap_or_default().to_owned();
        team.score = small(score)?;
        if code_version >= version::Y9S4 {
            team.starting_score = small(starting)?;
        }
    }

    // Sides from the operator icons in the header; full reads refine these
    // from the pick packets.
    let side = players.iter().filter(|p| p.team_index < 2).find_map(|p| {
        let op = Operator::from_role_image(u64::try_from(p.role_image).ok()?)?;
        Some((p.team_index, op.role()?))
    });
    if let Some((team, role)) = side {
        teams[team].role = Some(role);
        teams[team ^ 1].role = Some(role.opposite());
    }

    Ok(Header {
        game_version: get("version").unwrap_or_default().to_owned(),
        code_version,
        timestamp,
        match_type: MatchType(small("matchtype")?),
        map: Map(num("worldid")?),
        site: String::new(),
        recording_player_id: num("recordingplayerid")?,
        recording_profile_id: get("recordingprofileid").unwrap_or_default().to_owned(),
        additional_tags: get("additionaltags").unwrap_or_default().to_owned(),
        game_mode: GameMode(num("gamemodeid")?),
        rounds_per_match: small("roundspermatch")?,
        rounds_per_match_overtime: small("roundspermatchovertime")?,
        round_number: small("roundnumber")?,
        overtime_round_number: small("overtimeroundnumber")?,
        teams,
        players,
        gm_settings,
        // Optional and occasionally malformed; the original ignored bad values too.
        playlist_category: get("playlistcategory")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        match_id: get("id").unwrap_or_default().to_owned(),
        start_time: millis(get("starttime")),
        is_spectator: get("isspectator").map(|v| v == "1"),
        max_players_per_team: get("maxnbplayersperteam").and_then(|v| v.parse().ok()),
        match_result: get("matchresult").and_then(|v| v.parse().ok()),
        end_time: millis(get("endtime")),
        keys,
    })
}
