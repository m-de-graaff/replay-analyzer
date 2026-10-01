//! The key/value header at the start of every replay.

use std::collections::HashMap;
use std::str::FromStr;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Serialize, Serializer};

use crate::cosmetics::Cosmetics;
use crate::cursor::Cursor;
pub use crate::entities::Relation;
use crate::error::{Error, Result};
use crate::format::{self, FormatInfo, read_string};
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
    /// Whether a spectator recorded the match (Y11S3+ `isspectator`, which
    /// the game writes only when true). `None` before Y11S3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_spectator: Option<bool>,
    /// Y11S3+ `maxnbplayersperteam`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_players_per_team: Option<u32>,
    /// Y11S3+ `matchresult`, written only on the round that ends the match:
    /// the result of the team the game numbers 1 (see `Team::color`), which
    /// is the recorder's team in a player's recording. 2 won, 1 lost (all 26
    /// finished matches of a real folder agree); 7 the game ended the match
    /// with no winner (seen once).
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
    /// The game's number for this team (`TeamColor`, 1 or 2), from the team
    /// object (full and partial reads, Y11S3+). Ban slots and `matchresult`
    /// name teams by it; in a player's recording 1 is the recorder's team.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
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
    /// Y11S3+: the clearance level, from the round's opening snapshot (the
    /// game's property is `ClearanceLevelText`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    /// Y11S3+ full reads: the operator's maximum health without a Rook
    /// plate (the HUD's `MaxHealth`): 100, 110 or 125.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_health: Option<u32>,
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
    /// Stable key across matches: the profile id, or `name:<username>` when
    /// the replay has none (usernames can change, so prefer the profile id).
    pub key: String,
    /// How the player relates to whoever recorded: `you`, `teammate` or
    /// `opponent`. Absent for spectator recordings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<Relation>,
    /// The player queued with the recorder: `leader` or `member` of their
    /// party (Y8S1+, not in custom games).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<PartyRole>,
    /// Ids of the objects that carry this player in the packet stream
    /// (Y8S1+ full and partial reads).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entities: Option<PlayerEntities>,
    /// Where the player's body was created, in map coordinates (Y11S3+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spawn_position: Option<[f32; 3]>,
    /// The platform the player is on, for the values whose meaning is known
    /// (Y11S3+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
    /// The controller's raw `PlayerPlatform`.
    #[serde(skip)]
    pub platform_raw: Option<u32>,
    /// `username` is a nickname the game shows in place of the player's own
    /// name (the profile's `UsesNickname`). `renamedTo` has the name the
    /// game shows once the match is over, when the recording reaches it.
    #[serde(skip_serializing_if = "is_zero")]
    pub uses_nickname: bool,
    /// The name the game gave the player later in the round, which it does
    /// when the match ends: for players behind a nickname, and for console
    /// players.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renamed_to: Option<String>,
    /// What the player wears and carries: uniform, headgear, weapon skins,
    /// charms (Y11S3+, once their body exists).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cosmetics: Option<Cosmetics>,
}

/// The platform a player is on. PC is confirmed by the recording players
/// seen; the consoles are told apart by the account id the controller holds
/// beside it (`PlatformPlayerID`: empty on PC, in the range Xbox ids are
/// numbered in for Xbox).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Pc,
    PlayStation,
    Xbox,
}

impl Platform {
    /// From the controller's `PlayerPlatform`. `None` for values not seen
    /// on a player.
    pub fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::Pc),
            5 => Some(Self::PlayStation),
            7 => Some(Self::Xbox),
            _ => None,
        }
    }
}

/// A player's role in the recorder's party.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PartyRole {
    Leader,
    Member,
}

/// Object ids (hex) that link packets to a player.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerEntities {
    /// Holds the name, operator, team and per-player state such as the
    /// weapon-ready flag; pick and swap packets write to it.
    #[serde(serialize_with = "hex_id")]
    pub controller: u32,
    /// Receives the player's scoreboard updates.
    #[serde(serialize_with = "hex_id_opt", skip_serializing_if = "Option::is_none")]
    pub scoreboard: Option<u32>,
    /// Receives health and down/dead state.
    #[serde(serialize_with = "hex_id_opt", skip_serializing_if = "Option::is_none")]
    pub health: Option<u32>,
    /// The operator's body the movement stream moves, linked through the
    /// replay's player table rather than by player order (Y11S3+). The last
    /// one when the player got a new body during the round.
    #[serde(serialize_with = "hex_id_opt", skip_serializing_if = "Option::is_none")]
    pub movement: Option<u32>,
}

fn hex_id<S: Serializer>(v: &u32, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(&format_args!("{v:08x}"))
}

fn hex_id_opt<S: Serializer>(v: &Option<u32>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(v) => hex_id(v, s),
        None => s.serialize_none(),
    }
}

impl Player {
    /// The profile id, or `name:<username>` without one.
    pub fn stable_key(&self) -> String {
        if self.profile_id.is_empty() {
            format!("name:{}", self.username)
        } else {
            self.profile_id.clone()
        }
    }
}

impl Header {
    /// The player who recorded: by the header's `recordingplayerid`, or by
    /// `recordingprofileid` when the ids do not match.
    pub fn recording_player(&self) -> Option<&Player> {
        self.players
            .iter()
            .find(|p| p.id != 0 && p.id == self.recording_player_id)
            .or_else(|| {
                let profile = &self.recording_profile_id;
                (!profile.is_empty())
                    .then(|| self.players.iter().find(|p| p.profile_id == *profile))
                    .flatten()
            })
            .or_else(|| {
                self.players
                    .iter()
                    .find(|p| p.relation == Some(Relation::You))
            })
    }

    /// Index of the team the game numbers `color` (`TeamColor`, 1 or 2).
    /// Colors decoded from the team objects decide. Without them, 1 is the
    /// recorder's team in a player's recording, and team 0 in a spectator's
    /// (as in the one spectator match seen).
    pub fn team_of_color(&self, color: u32) -> Option<usize> {
        if !matches!(color, 1 | 2) {
            return None;
        }
        if let Some(i) = self.teams.iter().position(|t| t.color == Some(color)) {
            return Some(i);
        }
        // One team's decoded color gives the other team the other one.
        if let Some(i) = self.teams.iter().position(|t| t.color.is_some()) {
            return Some(i ^ 1);
        }
        let first = if self.is_spectator == Some(true) {
            0
        } else {
            self.recording_player()
                .map(|p| p.team_index)
                .filter(|&t| t < 2)?
        };
        Some(if color == 1 { first } else { first ^ 1 })
    }

    /// Fills each player's `key` and `relation`. A relation already set to
    /// `you` (decoded from the stream) wins over the header's recording ids.
    pub fn assign_relations(&mut self) {
        for p in &mut self.players {
            p.key = p.stable_key();
        }
        let spectator = self.is_spectator == Some(true);
        let recorder = self
            .recording_player()
            .map(|p| (p.id, p.username.clone(), p.team_index));
        for p in &mut self.players {
            p.relation = match &recorder {
                _ if spectator => None,
                Some((id, name, team)) => Some(if p.id == *id && p.username == *name {
                    Relation::You
                } else if p.team_index == *team {
                    Relation::Teammate
                } else {
                    Relation::Opponent
                }),
                None => None,
            };
        }
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

    let mut header = Header {
        game_version: get("version").unwrap_or_default().to_owned(),
        code_version,
        timestamp,
        match_type: MatchType::new(small("matchtype")?, code_version),
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
        // Y11S3 headers, the first with `starttime`, write `isspectator` only
        // for spectators.
        is_spectator: get("isspectator")
            .map(|v| v == "1")
            .or(get("starttime").map(|_| false)),
        max_players_per_team: get("maxnbplayersperteam").and_then(|v| v.parse().ok()),
        match_result: get("matchresult").and_then(|v| v.parse().ok()),
        end_time: millis(get("endtime")),
        keys,
    };
    header.assign_relations();
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A header whose recording player (`recordingplayerid` 7) plays for
    /// `team`.
    fn recorded_by_team(team: usize) -> Header {
        Header {
            recording_player_id: 7,
            players: vec![Player {
                id: 7,
                username: "recorder".into(),
                team_index: team,
                ..Player::default()
            }],
            ..Header::default()
        }
    }

    /// The header properties of a ranked recording with `extra` added, as
    /// the file stores them (`u64` length, then the bytes), and their count.
    fn properties(extra: &[(&str, &str)]) -> (Vec<u8>, u32) {
        let mut pairs = vec![
            ("version", "Y11S3_Alpha04"),
            ("code", "9901603"),
            ("datetime", "2026-09-29-01-41-49"),
            ("matchtype", "2"),
            ("worldid", "413779563590"),
            ("recordingplayerid", "7"),
            ("gamemodeid", "327933806"),
            ("roundspermatch", "6"),
            ("roundspermatchovertime", "3"),
            ("roundnumber", "0"),
            ("overtimeroundnumber", "0"),
            ("teamname0", "YOUR TEAM"),
            ("startingteamscore0", "0"),
            ("teamname1", "ENEMY TEAM"),
            ("startingteamscore1", "0"),
            ("teamscore0", "0"),
            ("teamscore1", "0"),
        ];
        pairs.extend_from_slice(extra);
        let mut d = Vec::new();
        for s in pairs.iter().flat_map(|(k, v)| [k, v]) {
            d.extend((s.len() as u64).to_le_bytes());
            d.extend(s.as_bytes());
        }
        (d, pairs.len() as u32)
    }

    fn read(extra: &[(&str, &str)]) -> Header {
        let (d, count) = properties(extra);
        read_properties(&mut Cursor::new(&d, 0), Some(count)).unwrap()
    }

    #[test]
    fn a_y11s3_header_without_isspectator_is_a_players_recording() {
        let h = read(&[("starttime", "1790638909220")]);
        assert_eq!(h.is_spectator, Some(false));
    }

    #[test]
    fn isspectator_marks_a_spectators_recording() {
        let h = read(&[("starttime", "1790638909220"), ("isspectator", "1")]);
        assert_eq!(h.is_spectator, Some(true));
    }

    #[test]
    fn an_older_header_leaves_spectator_unknown() {
        assert_eq!(read(&[]).is_spectator, None);
    }

    #[test]
    fn team_color_one_is_the_recorders_team() {
        let h = recorded_by_team(1);
        assert_eq!((h.team_of_color(1), h.team_of_color(2)), (Some(1), Some(0)));
    }

    #[test]
    fn team_color_one_is_team_zero_in_a_spectators_recording() {
        let mut h = recorded_by_team(1);
        h.is_spectator = Some(true);
        assert_eq!((h.team_of_color(1), h.team_of_color(2)), (Some(0), Some(1)));
    }

    #[test]
    fn decoded_team_colors_override_the_recorders_team() {
        let mut h = recorded_by_team(1);
        h.teams[0].color = Some(1);
        h.teams[1].color = Some(2);
        assert_eq!((h.team_of_color(1), h.team_of_color(2)), (Some(0), Some(1)));
    }

    #[test]
    fn one_decoded_team_color_gives_the_other_team_the_other_color() {
        let mut h = recorded_by_team(0);
        h.teams[1].color = Some(1);
        assert_eq!(h.team_of_color(2), Some(0));
    }

    #[test]
    fn team_color_without_a_recorder_or_out_of_range_is_no_team() {
        assert_eq!(Header::default().team_of_color(1), None);
        assert_eq!(recorded_by_team(0).team_of_color(3), None);
    }
}
