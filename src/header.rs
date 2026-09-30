//! The key/value header at the start of every replay.

use std::collections::HashMap;
use std::str::FromStr;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Serialize, Serializer};

use crate::cursor::Cursor;
use crate::error::{Error, Result};
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
}

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

fn rfc3339<S: Serializer>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(&t.format("%Y-%m-%dT%H:%M:%SZ"))
}

const STRING_SEPARATOR: [u8; 7] = [0; 7];

/// Parses the header at the start of `data` (which begins with `dissect`).
/// Returns the header and the offset just past it.
pub fn parse(data: &[u8]) -> Result<(Header, usize)> {
    let mut c = Cursor::new(data, 0);
    if c.bytes(7).map_err(|_| Error::InvalidFile)? != b"dissect" {
        return Err(Error::InvalidFile);
    }
    skip_version_block(&mut c)?;
    let header = read_properties(&mut c)?;
    Ok((header, c.pos()))
}

/// The meaning of the bytes after the magic is unknown. The properties start
/// after the second run of seven zero bytes.
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

fn read_properties(c: &mut Cursor) -> Result<Header> {
    let mut props: HashMap<String, String> = HashMap::new();
    let mut gm_settings = Vec::new();
    let mut players = Vec::new();
    // `Some` while inside a run of player properties.
    let mut player: Option<Player> = None;

    // The last property in the header is always `teamscore1`.
    while !props.contains_key("teamscore1") {
        let key = read_string(c)?;
        let value = read_string(c)?;

        if key == "playerid" {
            players.extend(player.replace(Player::default()));
        } else if key == "playlistcategory" || key == "id" {
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
    })
}
