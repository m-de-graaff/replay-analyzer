//! Parsing a single round (one `.rec` file).

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use aho_corasick::AhoCorasick;
use rayon::prelude::*;
use serde::Serialize;

use crate::cursor::Cursor;
use crate::decompress::{self, Decompressed};
use crate::details::{
    Ban, HealthUpdate, LifeEvent, LifeEventType, Loadout, ObservationSession, Phase,
};
use crate::error::{Error, Result};
use crate::feedback::{Clock, MatchUpdate, MatchUpdateType};
use crate::header::{Header, Player};
use crate::stats::PlayerRoundStats;
use crate::types::{ObservationTool, Operator, TeamRole, WinCondition, version};

/// A fully parsed round.
#[derive(Clone, Debug, Default)]
pub struct Round {
    pub header: Header,
    pub match_feedback: Vec<MatchUpdate>,
    /// Scoreboard values keyed by the player's packet id.
    pub scoreboard: HashMap<[u8; 4], ScoreboardEntry>,
    /// Operators banned for this round, in the order the game lists them.
    pub bans: Vec<Ban>,
    /// Every change to a player's health after the round starts.
    pub health: Vec<HealthUpdate>,
    /// Downs and revives.
    pub life_events: Vec<LifeEvent>,
    /// Time players spent on drones and cameras.
    pub observation: Vec<ObservationSession>,
    /// What each player carried, once per operator they played.
    pub loadouts: Vec<Loadout>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScoreboardEntry {
    pub score: u32,
    /// Cumulative match assists as shown on the scoreboard.
    pub assists: u32,
    /// Number of assist updates seen during this round.
    pub assists_from_round: u32,
}

/// How much of the replay to read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReadMode {
    /// Every packet, including the round result.
    #[default]
    Full,
    /// Only the first third of the replay: enough for the header and player
    /// list, much faster. Attacker operator swaps and the result are missing.
    Partial,
}

impl Round {
    pub fn open(path: impl AsRef<Path>, mode: ReadMode) -> Result<Self> {
        Self::from_bytes(&std::fs::read(path)?, mode)
    }

    pub fn from_bytes(raw: &[u8], mode: ReadMode) -> Result<Self> {
        let Decompressed {
            data,
            header,
            body_start,
        } = decompress::decompress(raw)?;
        Ok(Parser::new(&data, header).run(body_start, mode))
    }

    /// Parses only the header, without scanning packets.
    pub fn header_only(raw: &[u8]) -> Result<Header> {
        Ok(decompress::decompress(raw)?.header)
    }

    pub fn player_index_by_id(&self, id: [u8; 4]) -> Option<usize> {
        self.header
            .players
            .iter()
            .position(|p| p.dissect_id == Some(id))
    }

    pub fn player_index_by_username(&self, username: &str) -> Option<usize> {
        self.header
            .players
            .iter()
            .position(|p| p.username == username)
    }

    pub fn scoreboard_for(&self, player: &Player) -> ScoreboardEntry {
        player
            .dissect_id
            .and_then(|id| self.scoreboard.get(&id).copied())
            .unwrap_or_default()
    }
}

/// The decompressed dissect stream, for debugging the format.
pub fn decompressed_bytes(raw: &[u8]) -> Result<Vec<u8>> {
    let d = decompress::decompress(raw)?;
    Ok(d.data)
}

impl Serialize for Round {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Output<'a> {
            #[serde(flatten)]
            header: &'a Header,
            match_feedback: &'a [MatchUpdate],
            stats: Vec<PlayerRoundStats>,
            #[serde(skip_serializing_if = "<[_]>::is_empty")]
            bans: &'a [Ban],
            #[serde(skip_serializing_if = "<[_]>::is_empty")]
            health: &'a [HealthUpdate],
            #[serde(skip_serializing_if = "<[_]>::is_empty")]
            life_events: &'a [LifeEvent],
            #[serde(skip_serializing_if = "<[_]>::is_empty")]
            observation: &'a [ObservationSession],
            #[serde(skip_serializing_if = "<[_]>::is_empty")]
            loadouts: &'a [Loadout],
        }
        Output {
            header: &self.header,
            match_feedback: &self.match_feedback,
            stats: self.player_stats(),
            bans: &self.bans,
            health: &self.health,
            life_events: &self.life_events,
            observation: &self.observation,
            loadouts: &self.loadouts,
        }
        .serialize(s)
    }
}

#[derive(Clone, Copy, Debug)]
enum Packet {
    Player,
    AttackerSwap,
    Spawn,
    Time,
    LegacyTime,
    Feedback,
    DefuserTimer,
    ScoreboardScore,
    ScoreboardAssists,
    RoleImage,
    Health,
    LifeState,
    Observer,
    ObservedOwner,
    ObservationTool,
    Item,
}

const PACKETS: [(Packet, &[u8]); 15] = [
    (Packet::Player, &[0x22, 0x07, 0x94, 0x9B, 0xDC]),
    (Packet::AttackerSwap, &[0x22, 0xA9, 0x26, 0x0B, 0xE4]),
    (Packet::Spawn, &[0xAF, 0x98, 0x99, 0xCA]),
    (Packet::Feedback, &[0x59, 0x34, 0xE5, 0x8B, 0x04]),
    (Packet::DefuserTimer, &[0x22, 0xA9, 0xC8, 0x58, 0xD9]),
    (Packet::ScoreboardScore, &[0xEC, 0xDA, 0x4F, 0x80]),
    (Packet::ScoreboardAssists, &[0x4D, 0x73, 0x7F, 0x9E]),
    (Packet::RoleImage, &[0xDA, 0x69, 0x14, 0xD5]),
    (Packet::Health, &[0x25, 0x26, 0x76, 0xC9]),
    (Packet::LifeState, &[0xE7, 0x88, 0xF6, 0xA5]),
    (Packet::Observer, &[0xDE, 0xAE, 0xCA, 0x09]),
    (Packet::ObservedOwner, &[0x5B, 0xE8, 0x47, 0x28]),
    (Packet::ObservationTool, &[0x06, 0x6B, 0xC0, 0xA1]),
    (Packet::Item, &[0x0E, 0x9E, 0xBE, 0x88]),
    (Packet::Time, &[0x1F, 0x07, 0xEF, 0xC9]),
];
const LEGACY_TIME: &[u8] = &[0x1E, 0xF1, 0x11, 0xAB];

/// One multi-pattern automaton per clock format, built once.
static SCANNERS: LazyLock<[(Vec<Packet>, AhoCorasick); 2]> = LazyLock::new(|| {
    let build = |time: (Packet, &'static [u8])| {
        let (_, rest) = PACKETS.split_last().expect("packets");
        let set: Vec<_> = rest.iter().copied().chain([time]).collect();
        let ac = AhoCorasick::new(set.iter().map(|p| p.1)).expect("valid patterns");
        (set.into_iter().map(|p| p.0).collect(), ac)
    };
    [
        build(PACKETS[PACKETS.len() - 1]),
        build((Packet::LegacyTime, LEGACY_TIME)),
    ]
});

/// Finds every packet marker in `body`, returning `(end offset, pattern)` in
/// stream order.
///
/// No marker's suffix is a prefix of another, so occurrences never overlap and
/// a non-overlapping search (which can use SIMD prefilters) finds all of them.
/// The body is split into chunks searched in parallel; each chunk's window
/// extends far enough to finish any marker that starts inside it.
fn scan(scanner: &AhoCorasick, body: &[u8]) -> Vec<(usize, usize)> {
    const CHUNK: usize = 4 << 20;
    let overlap = scanner.max_pattern_len() - 1;
    (0..body.len().div_ceil(CHUNK))
        .into_par_iter()
        .flat_map_iter(|i| {
            let from = i * CHUNK;
            let window = &body[from..(from + CHUNK + overlap).min(body.len())];
            scanner
                .find_iter(window)
                .take_while(|m| m.start() < CHUNK)
                .map(move |m| (from + m.end(), m.pattern().as_usize()))
        })
        .collect()
}

const SPAWN_INDICATOR: &[u8] = &[0xAF, 0x98, 0x99, 0xCA];
const PROFILE_ID_INDICATOR: &[u8] = &[0x8A, 0x50, 0x9B, 0xD0];
const UI_ID_INDICATOR: &[u8] = &[0x38, 0xDF, 0xEE, 0x88];
const CURRENT_SITE: [u8; 5] = [0xFC, 0xC6, 0xA8, 0x60, 0x01];
const LEGACY_FEEDBACK: &[u8] = &[0x00, 0x00, 0x00, 0x22, 0xE3, 0x09, 0x00, 0x79];
const KILL_INDICATOR: [u8; 5] = [0x22, 0xD9, 0x13, 0x3C, 0xBA];
/// Property that follows the operator in a pick or swap, on the same object.
const STATE_PROPERTY: [u8; 4] = [0x63, 0xCC, 0x18, 0x8F];
const ITEM_ICON: [u8; 5] = [0x22, 0xB2, 0x97, 0xEF, 0x0C];
/// Items every attacker (the drone) or, before Y11, every defender (likely
/// reinforcements) carries; they are not part of the chosen loadout.
const STANDARD_ITEMS: [u64; 2] = [249219679725, 133651070300];
/// A player's items come within this many bytes before their pick or swap.
const ITEM_WINDOW: usize = 2500;
const KILL_WEAPON: [u8; 5] = [0x22, 0xF8, 0x6C, 0xDD, 0x65];
/// The side of a ban slot, written into an object right after the banned
/// operator's icon.
const BAN_ROLE: &[u8] = &[0x18, 0xFF, 0xCA, 0x5E];
/// How far after an icon to look for a ban slot's side. Older replays put one
/// more object in between; player icons are thousands of bytes from a ban.
const BAN_WINDOW: usize = 160;

struct Parser<'a> {
    data: &'a [u8],
    round: Round,
    clock: Clock,
    last_defuser: Option<usize>,
    planted: bool,
    players_read: u32,
    timeline: Timeline,
    /// `players_read` -> username of the player picked at that count.
    pick_slots: HashMap<u32, String>,
    /// Object id -> `players_read` when the object first appeared. Players'
    /// objects are sent next to their pick packet, which links the two.
    health_objects: HashMap<u32, u32>,
    observers: HashMap<u32, u32>,
    /// Latest device owner shown on each observer object.
    observed_owner: HashMap<u32, String>,
    samples: Vec<Sample>,
    /// Items sent since the last pick or swap, which they belong to.
    pending_items: Vec<Item>,
    seen_items: std::collections::HashSet<u32>,
}

/// An equipment slot as sent before a pick or swap packet.
struct Item {
    offset: usize,
    id: u64,
    /// Gadgets carry an icon; guns do not.
    icon: u64,
}

/// Round time that keeps counting across the prep -> action clock reset.
#[derive(Default)]
struct Timeline {
    last: Option<f64>,
    elapsed: f64,
    phase: Phase,
}

impl Timeline {
    /// Action phases start well above the 45 second prep phase.
    const ACTION_START: f64 = 50.0;

    fn tick(&mut self, seconds: f64) {
        match self.last {
            Some(last) if seconds < last => self.elapsed += last - seconds,
            Some(last) if seconds > last + 1.0 => {
                self.phase = if seconds > Self::ACTION_START {
                    Phase::Action
                } else {
                    Phase::Prep
                };
            }
            None if seconds > Self::ACTION_START => self.phase = Phase::Action,
            _ => {}
        }
        self.last = Some(seconds);
    }
}

/// A per-player object property, resolved to a player after the stream is read.
struct Sample {
    object: u32,
    value: SampleValue,
    clock: Clock,
    elapsed: f64,
    phase: Phase,
}

enum SampleValue {
    Health(u32),
    LifeState(u32),
    Tool(ObservationTool, String),
}

impl<'a> Parser<'a> {
    fn new(data: &'a [u8], header: Header) -> Self {
        Self {
            data,
            round: Round {
                header,
                ..Round::default()
            },
            clock: Clock::default(),
            last_defuser: None,
            planted: false,
            players_read: 0,
            timeline: Timeline::default(),
            pick_slots: HashMap::new(),
            health_objects: HashMap::new(),
            observers: HashMap::new(),
            observed_owner: HashMap::new(),
            samples: Vec::new(),
            pending_items: Vec::new(),
            seen_items: Default::default(),
        }
    }

    fn code(&self) -> u32 {
        self.round.header.code_version
    }

    fn players(&mut self) -> &mut Vec<Player> {
        &mut self.round.header.players
    }

    fn run(mut self, start: usize, mode: ReadMode) -> Round {
        let end = match mode {
            ReadMode::Full => self.data.len(),
            ReadMode::Partial => (self.data.len() / 3).max(start),
        };
        let (packets, scanner) = &SCANNERS[usize::from(self.code() < version::Y8S1)];
        // Handlers run in stream order but never depend on each other's cursor.
        for (offset, pattern) in scan(scanner, &self.data[start..end]) {
            let packet = packets[pattern];
            let mut c = Cursor::new(self.data, start + offset);
            if let Err(e) = self.dispatch(packet, &mut c) {
                tracing::debug!(?packet, offset = start + offset, error = %e, "skipping packet");
            }
        }
        if self.players_read < 10 {
            self.derive_team_roles();
        }
        if mode == ReadMode::Full {
            self.round_end();
        }
        self.resolve_samples();
        self.round
    }

    fn dispatch(&mut self, packet: Packet, c: &mut Cursor) -> Result<()> {
        match packet {
            Packet::Player => {
                self.players_read += 1;
                let result = self.read_player(c);
                if self.players_read == 10 {
                    self.derive_team_roles();
                }
                result
            }
            Packet::AttackerSwap => self.read_attacker_swap(c),
            Packet::Spawn => self.read_spawn(c),
            Packet::Time => self.read_time(c),
            Packet::LegacyTime => self.read_legacy_time(c),
            Packet::Feedback => self.read_feedback(c),
            Packet::DefuserTimer => self.read_defuser_timer(c),
            Packet::ScoreboardScore => self.read_scoreboard_score(c),
            Packet::ScoreboardAssists => self.read_scoreboard_assists(c),
            Packet::RoleImage => self.read_role_image(c),
            Packet::Health => self.read_health(c),
            Packet::LifeState => self.read_life_state(c),
            Packet::Observer => self.read_observer(c),
            Packet::ObservedOwner => self.read_observed_owner(c),
            Packet::ObservationTool => self.read_observation_tool(c),
            Packet::Item => self.read_item(c),
        }
    }

    fn push(&mut self, update: MatchUpdate) {
        tracing::debug!(?update, "match update");
        self.round.match_feedback.push(update);
    }

    fn update(&self, kind: MatchUpdateType, username: &str) -> MatchUpdate {
        let mut u = MatchUpdate::new(kind, &self.clock);
        u.username = username.to_owned();
        u
    }

    fn read_player(&mut self, c: &mut Cursor) -> Result<()> {
        let code = self.code();
        let username = c.string()?;
        if code >= version::Y7S4 {
            c.seek(&[0x40, 0xF2, 0x15, 0x04])?;
            c.skip(8)?;
            // The marker is sometimes sent twice; the repeat is not a player.
            if c.u8()? == 0x9D {
                return Ok(());
            }
        } else {
            c.seek(&[0x22, 0xA9, 0x26, 0x0B, 0xE4])?;
        }
        // Operator before any attacker swaps.
        let operator = Operator(c.u64()?);
        if operator.is_empty() {
            return Ok(()); // empty slot
        }
        let state_id = state_object_after(c.peek(200));
        if c.u8()? != 0x22 {
            tracing::warn!(%operator, "invalid player packet");
            return Ok(());
        }
        c.seek(if code <= version::Y7S2 {
            &[0xE6, 0xF9, 0x7D, 0x86]
        } else {
            &[0x33, 0xD8, 0x3D, 0x4F, 0x23]
        })?;
        let dissect_id = c.array::<4>()?;
        c.seek(SPAWN_INDICATOR)?;
        let mut spawn = c.string()?;
        if spawn.is_empty() {
            c.skip(10)?;
            if c.u8()? != 0x1B {
                return Ok(());
            }
        }
        self.take_loadout(c.pos(), &username, operator);
        let team_index = usize::from(self.players_read > 5);
        self.pick_slots.insert(self.players_read, username.clone());

        // Caster UI id; links attacker swaps to players from Y9S3.
        let mut ui_id = 0;
        if code >= version::Y9S3 {
            c.seek(UI_ID_INDICATOR)?;
            c.skip(13)?;
            ui_id = c.u64()?;
        }

        // Older replays have no profile ids.
        let (mut profile_id, mut id) = (String::new(), 0);
        if !self.round.header.recording_profile_id.is_empty() {
            c.seek(PROFILE_ID_INDICATOR)?;
            profile_id = c.string()?;
            c.skip(5)?;
            id = c.u64()?;
        }

        // Defender spawns come from the site packet instead.
        if operator != Operator::RECRUIT && operator.role() == Some(TeamRole::Defense) {
            spawn = self.round.header.site.clone();
        }
        tracing::debug!(%username, team_index, %operator, %profile_id, id, ui_id, %spawn, "player");

        let existing = self.round.header.players.iter_mut().find(|e| {
            e.username == username
                || (code < version::Y8S2 && id != 0 && e.id == id)
                || (code >= version::Y8S2 && e.dissect_id == Some(dissect_id))
                || (code <= version::Y7S2 && username.starts_with(&e.username))
        });
        match existing {
            Some(e) => {
                e.profile_id = profile_id;
                e.username = username;
                e.operator = operator;
                e.spawn = spawn;
                e.dissect_id = Some(dissect_id);
                e.ui_id = ui_id;
                e.state_id = state_id;
            }
            None if !username.is_empty() => self.players().push(Player {
                id,
                profile_id,
                username,
                team_index,
                operator,
                spawn,
                dissect_id: Some(dissect_id),
                ui_id,
                state_id,
                ..Player::default()
            }),
            None => {}
        }
        Ok(())
    }

    /// Drops empty player slots and infers which team attacks from the
    /// operators picked.
    fn derive_team_roles(&mut self) {
        let header = &mut self.round.header;
        header.players.retain(|p| {
            if p.operator.is_empty() {
                tracing::warn!(username = %p.username, "operator id was 0, removing player");
            }
            !p.operator.is_empty()
        });
        if header.players.len() > 10 {
            tracing::warn!(players = header.players.len(), "more than 10 players");
        }
        let known = header
            .players
            .iter()
            .find_map(|p| Some((p.team_index, p.operator.role()?)));
        if let Some((team, role)) = known
            && team < 2
        {
            let other = match role {
                TeamRole::Attack => TeamRole::Defense,
                TeamRole::Defense => TeamRole::Attack,
            };
            header.teams[team].role = Some(role);
            header.teams[team ^ 1].role = Some(other);
        }
    }

    fn read_attacker_swap(&mut self, c: &mut Cursor) -> Result<()> {
        // Owner of the property chain the swap belongs to, if it is a state object.
        let owner = owning_object(c.behind(PACKETS[1].1.len() + 128), PACKETS[1].1.len());
        let operator = Operator(c.u64()?);
        let by_state = owner.and_then(|id| {
            self.round
                .header
                .players
                .iter()
                .position(|p| p.state_id == Some(id))
        });
        let index = if let Some(i) = by_state {
            Some(i)
        } else if self.code() < version::Y9S3 {
            c.skip(5)?;
            let id = c.array::<4>()?;
            self.round.player_index_by_id(id)
        } else {
            // Layout changed with the Y9S3 caster view overhaul.
            c.skip(402)?;
            let ui_id = c.u64()?;
            self.round
                .header
                .players
                .iter()
                .position(|p| ui_id != 0 && p.ui_id == ui_id)
        };
        if let Some(i) = index {
            let username = self.round.header.players[i].username.clone();
            self.take_loadout(c.pos(), &username, operator);
            self.players()[i].operator = operator;
            let mut u = self.update(
                MatchUpdateType::OperatorSwap,
                &self.round.header.players[i].username,
            );
            u.operator = operator;
            self.push(u);
        }
        Ok(())
    }

    fn read_spawn(&mut self, c: &mut Cursor) -> Result<()> {
        let location = c.string()?;
        c.skip(150)?;
        let marker = c.array::<5>()?;
        if !location.contains("<br/>") {
            return Ok(());
        }
        let header = &mut self.round.header;
        if !header.site.is_empty() && marker != CURRENT_SITE {
            return Ok(());
        }
        let site = location.replacen("<br/>", ", ", 1);
        let teams = header.teams.clone();
        for p in &mut header.players {
            let defense_team =
                teams.get(p.team_index).and_then(|t| t.role) == Some(TeamRole::Defense);
            let defense_op =
                p.operator != Operator::RECRUIT && p.operator.role() == Some(TeamRole::Defense);
            if defense_team || defense_op {
                p.spawn = site.clone();
            }
        }
        tracing::debug!(%site, "defense site");
        header.site = site;
        Ok(())
    }

    fn read_time(&mut self, c: &mut Cursor) -> Result<()> {
        let t = c.u32()?;
        self.timeline.tick(f64::from(t));
        self.clock = Clock {
            seconds: f64::from(t),
            display: format!("{}:{:02}", t / 60, t % 60),
        };
        Ok(())
    }

    /// Pre-Y8S1 replays store the clock as text: `m:ss` or fractional seconds.
    fn read_legacy_time(&mut self, c: &mut Cursor) -> Result<()> {
        let text = c.string()?;
        let invalid = || Error::InvalidProperty {
            key: "time".into(),
            value: text.clone(),
        };
        let seconds = match text.split_once(':') {
            None => text.parse::<f64>().map_err(|_| invalid())?,
            Some((m, s)) => {
                let s = s.split(':').next().unwrap_or(s);
                let (m, s): (u32, u32) = (
                    m.parse().map_err(|_| invalid())?,
                    s.parse().map_err(|_| invalid())?,
                );
                f64::from(m * 60 + s)
            }
        };
        self.timeline.tick(seconds);
        self.clock = Clock {
            seconds,
            display: text,
        };
        Ok(())
    }

    fn read_feedback(&mut self, c: &mut Cursor) -> Result<()> {
        let code = self.code();
        if code >= version::Y9S1_UPDATE3 {
            c.skip(38)?;
        } else if code >= version::Y9S1 {
            c.skip(9)?;
            if c.u8()? != 4 {
                return Err(Error::InvalidProperty {
                    key: "feedback".into(),
                    value: "validity byte".into(),
                });
            }
            c.skip(24)?;
        } else {
            c.skip(1)?;
            c.seek(LEGACY_FEEDBACK)?;
        }
        let size = c.u8()? as usize;
        if size == 0 {
            return self.read_kill(c);
        }
        // Y9S1 changed or removed the text messages; not decoded yet.
        if code >= version::Y9S1 {
            return Ok(());
        }
        let msg = String::from_utf8_lossy(c.bytes(size)?).into_owned();
        let kind = if msg.contains("left") {
            MatchUpdateType::PlayerLeave
        } else if msg.contains("BattlEye") {
            MatchUpdateType::Battleye
        } else if msg.contains("bombs") || msg.contains("objective") {
            MatchUpdateType::LocateObjective
        } else {
            MatchUpdateType::Other
        };
        let mut u = MatchUpdate::new(kind, &self.clock);
        if kind == MatchUpdateType::Other {
            u.message = msg;
        } else {
            u.username = msg.split(' ').next().unwrap_or_default().to_owned();
        }
        self.push(u);
        Ok(())
    }

    fn read_kill(&mut self, c: &mut Cursor) -> Result<()> {
        if c.array::<5>()? != KILL_INDICATOR {
            return Ok(());
        }
        let username = c.string()?;
        c.skip(15)?; // unknown, possibly kill type
        let target = c.string()?;
        if username.is_empty() {
            // No killer: the target died on their own (fall damage, etc.).
            if !target.is_empty() {
                let u = self.update(MatchUpdateType::Death, &target);
                self.push(u);
            }
            return Ok(());
        }
        c.skip(56)?;
        let headshot = c.u8()? == 1;
        let weapon = if c.array::<5>()? == KILL_WEAPON {
            c.u64()?
        } else {
            0
        };
        let duplicate = self.round.match_feedback.iter().any(|v| {
            v.kind == MatchUpdateType::Kill && v.username == username && v.target == target
        });
        if !duplicate {
            let mut u = self.update(MatchUpdateType::Kill, &username);
            u.target = target;
            u.headshot = Some(headshot);
            u.weapon = weapon;
            self.push(u);
        }
        Ok(())
    }

    fn read_defuser_timer(&mut self, c: &mut Cursor) -> Result<()> {
        let timer = c.string()?;
        c.skip(34)?;
        let id = c.array::<4>()?;
        if let Some(i) = self.round.player_index_by_id(id) {
            let kind = if self.planted {
                MatchUpdateType::DefuserDisableStart
            } else {
                MatchUpdateType::DefuserPlantStart
            };
            let u = self.update(kind, &self.round.header.players[i].username);
            self.push(u);
            self.last_defuser = Some(i);
        }
        // TODO: 0.00 can appear without the defuser being disabled.
        if !timer.starts_with("0.00") {
            return Ok(());
        }
        let kind = if self.planted {
            MatchUpdateType::DefuserDisableComplete
        } else {
            self.planted = true;
            MatchUpdateType::DefuserPlantComplete
        };
        let username = self
            .last_defuser
            .and_then(|i| self.round.header.players.get(i))
            .map(|p| p.username.clone())
            .unwrap_or_default();
        let u = self.update(kind, &username);
        self.push(u);
        Ok(())
    }

    /// An operator icon. Most belong to players; the ones followed by a side
    /// are ban slots.
    fn read_role_image(&mut self, c: &mut Cursor) -> Result<()> {
        let icon = c.u64()?;
        let window = c.peek(BAN_WINDOW);
        let Some(at) = memchr::memmem::find(window, BAN_ROLE) else {
            return Ok(());
        };
        // Another icon first means the side belongs to that one.
        if memchr::memmem::find(&window[..at], PACKETS[7].1).is_some() {
            return Ok(());
        }
        c.skip(at + BAN_ROLE.len())?;
        let role = match c.u32()? {
            1 => TeamRole::Attack,
            2 => TeamRole::Defense,
            _ => return Ok(()),
        };
        if !self.round.bans.iter().any(|b| b.icon == icon) {
            let operator = Operator::from_role_image(icon);
            tracing::debug!(icon, ?operator, ?role, "ban");
            self.round.bans.push(Ban {
                operator,
                role,
                icon,
            });
        }
        Ok(())
    }

    fn read_item(&mut self, c: &mut Cursor) -> Result<()> {
        let head = c.behind(13);
        if head.len() < 13 || head[0] != 0x23 {
            return Ok(());
        }
        let object = u32::from_le_bytes(head[1..5].try_into().expect("4 bytes"));
        let offset = c.pos();
        let id = c.u64()?;
        if id == 0 || STANDARD_ITEMS.contains(&id) || !self.seen_items.insert(object) {
            return Ok(());
        }
        let icon = if c.array::<5>()? == ITEM_ICON {
            c.u64()?
        } else {
            0
        };
        self.pending_items.push(Item { offset, id, icon });
        Ok(())
    }

    /// Hands the items sent just before a pick or swap at `offset` to that
    /// player.
    fn take_loadout(&mut self, offset: usize, username: &str, operator: Operator) {
        let items = std::mem::take(&mut self.pending_items);
        let (weapons, gadgets): (Vec<_>, Vec<_>) = items
            .into_iter()
            .filter(|i| offset.saturating_sub(i.offset) <= ITEM_WINDOW)
            .partition(|i| i.icon == 0);
        if weapons.is_empty() && gadgets.is_empty() {
            return;
        }
        self.round.loadouts.push(Loadout {
            username: username.to_owned(),
            operator,
            weapons: weapons.iter().map(|i| i.id).collect(),
            gadgets: gadgets.iter().map(|i| i.id).collect(),
        });
    }

    fn sample(&mut self, object: u32, value: SampleValue) {
        self.samples.push(Sample {
            object,
            value,
            clock: self.clock.clone(),
            elapsed: self.timeline.elapsed,
            phase: self.timeline.phase,
        });
    }

    fn read_health(&mut self, c: &mut Cursor) -> Result<()> {
        let Some(object) = property_object(c) else {
            return Ok(());
        };
        let health = c.u32()?;
        self.health_objects
            .entry(object)
            .or_insert(self.players_read);
        self.sample(object, SampleValue::Health(health));
        Ok(())
    }

    fn read_life_state(&mut self, c: &mut Cursor) -> Result<()> {
        let Some(object) = property_object(c) else {
            return Ok(());
        };
        let state = c.u32()?;
        self.sample(object, SampleValue::LifeState(state));
        Ok(())
    }

    fn read_observer(&mut self, c: &mut Cursor) -> Result<()> {
        if let Some(object) = property_object(c) {
            self.observers.entry(object).or_insert(self.players_read);
        }
        Ok(())
    }

    fn read_observed_owner(&mut self, c: &mut Cursor) -> Result<()> {
        if let Some(object) = property_object(c) {
            let owner = c.string()?;
            self.observed_owner.insert(object, owner);
        }
        Ok(())
    }

    fn read_observation_tool(&mut self, c: &mut Cursor) -> Result<()> {
        let Some(object) = property_object(c) else {
            return Ok(());
        };
        let tool = ObservationTool(c.u32()?);
        let owner = self
            .observed_owner
            .get(&object)
            .cloned()
            .unwrap_or_default();
        self.sample(object, SampleValue::Tool(tool, owner));
        Ok(())
    }

    /// Turns the per-object samples into per-player health changes, downs and
    /// observation sessions.
    fn resolve_samples(&mut self) {
        // A player's health object is sent just before their pick packet and
        // the observer object just after it.
        let slots = &self.pick_slots;
        let owner = |objects: &HashMap<u32, u32>, object: u32, offset: u32| {
            slots.get(&(objects.get(&object)? + offset)).cloned()
        };
        let mut health: HashMap<u32, u32> = HashMap::new();
        let mut life: HashMap<u32, u32> = HashMap::new();
        // Observer object -> (index of its open session, elapsed at start).
        let mut open: HashMap<u32, (usize, f64)> = HashMap::new();
        let round = &mut self.round;
        for s in &self.samples {
            match &s.value {
                SampleValue::Health(value) => {
                    let Some(username) = owner(&self.health_objects, s.object, 1) else {
                        continue;
                    };
                    let prev = health.insert(s.object, *value);
                    // Prep phase sets up (or resets leftover) health; rising
                    // from zero is a spawn or a revive, not healing.
                    let live = s.phase == Phase::Action;
                    if let Some(prev) = prev.filter(|&p| live && p > 0 && p != *value) {
                        round.health.push(HealthUpdate {
                            username,
                            health: *value,
                            change: *value as i32 - prev as i32,
                            time: s.clock.display.clone(),
                            time_in_seconds: s.clock.seconds,
                        });
                    }
                }
                SampleValue::LifeState(state) => {
                    let Some(username) = owner(&self.health_objects, s.object, 1) else {
                        continue;
                    };
                    // 0 alive, 2 wounded, 3 downed, 4 dead.
                    let prev = life.insert(s.object, *state).unwrap_or(0);
                    let kind = match (prev, *state) {
                        (p, 3) if p != 3 => LifeEventType::Down,
                        (3, 0 | 2) => LifeEventType::Revive,
                        _ => continue,
                    };
                    round.life_events.push(LifeEvent {
                        kind,
                        username,
                        time: s.clock.display.clone(),
                        time_in_seconds: s.clock.seconds,
                    });
                }
                SampleValue::Tool(tool, device_owner) => {
                    let Some(username) = owner(&self.observers, s.object, 0) else {
                        continue;
                    };
                    let current = open.get(&s.object).map(|&(i, _)| round.observation[i].tool);
                    if current == Some(*tool) {
                        continue;
                    }
                    if let Some((i, start)) = open.remove(&s.object) {
                        round.observation[i].seconds = s.elapsed - start;
                    }
                    if tool.0 != 0 {
                        open.insert(s.object, (round.observation.len(), s.elapsed));
                        round.observation.push(ObservationSession {
                            username,
                            owner: device_owner.clone(),
                            tool: *tool,
                            phase: s.phase,
                            time: s.clock.display.clone(),
                            time_in_seconds: s.clock.seconds,
                            seconds: 0.0,
                        });
                    }
                }
            }
        }
        for (i, start) in open.into_values() {
            round.observation[i].seconds = self.timeline.elapsed - start;
        }
    }

    fn read_scoreboard_score(&mut self, c: &mut Cursor) -> Result<()> {
        let score = c.u32()?;
        if score == 0 {
            return Ok(());
        }
        c.skip(13)?;
        let id = c.array::<4>()?;
        if self.round.player_index_by_id(id).is_some() {
            self.round.scoreboard.entry(id).or_default().score = score;
        }
        Ok(())
    }

    fn read_scoreboard_assists(&mut self, c: &mut Cursor) -> Result<()> {
        let assists = c.u32()?;
        if assists == 0 {
            return Ok(());
        }
        c.skip(30)?;
        let id = c.array::<4>()?;
        if self.round.player_index_by_id(id).is_some() {
            let e = self.round.scoreboard.entry(id).or_default();
            e.assists = assists;
            e.assists_from_round += 1;
        }
        Ok(())
    }

    /// Decides which team won and how.
    fn round_end(&mut self) {
        let round = &mut self.round;
        let header = &mut round.header;
        let team_of = |name: &str| {
            header
                .players
                .iter()
                .find(|p| p.username == name)
                .map(|p| p.team_index)
                .filter(|&t| t < 2)
        };

        let mut sizes = [0usize; 2];
        for p in header.players.iter().filter(|p| p.team_index < 2) {
            sizes[p.team_index] += 1;
        }
        let roles = [header.teams[0].role, header.teams[1].role];
        let mut deaths = [0usize; 2];
        let mut planter_team = None;

        let explicit_scores = header.code_version >= version::Y9S4;
        if explicit_scores {
            let team0_won = header.teams[0].starting_score < header.teams[0].score;
            header.teams[0].won = team0_won;
            header.teams[1].won = !team0_won;
        }

        for u in &round.match_feedback {
            match u.kind {
                MatchUpdateType::Kill | MatchUpdateType::Death => {
                    if let Some(t) = u.victim().and_then(team_of) {
                        deaths[t] += 1;
                    }
                }
                MatchUpdateType::DefuserPlantComplete => planter_team = team_of(&u.username),
                MatchUpdateType::DefuserDisableComplete => {
                    if let Some(t) = team_of(&u.username) {
                        header.teams[t].won = true;
                        header.teams[t].win_condition = Some(WinCondition::DisabledDefuser);
                    }
                    return;
                }
                _ => {}
            }
        }

        if let Some(t) = planter_team {
            header.teams[t].won = true;
            header.teams[t].win_condition = Some(WinCondition::DefusedBomb);
            return;
        }
        // Y9S4+ headers state the winner; the condition is not yet reliable.
        if explicit_scores {
            return;
        }
        for (dead, winner) in [(0, 1), (1, 0)] {
            if deaths[dead] == sizes[dead] {
                header.teams[winner].won = true;
                header.teams[winner].win_condition = Some(WinCondition::KilledOpponents);
                return;
            }
        }
        // Nobody was wiped out: defenders win on time.
        let defenders = usize::from(roles[1] == Some(TeamRole::Defense));
        header.teams[defenders].won = true;
        header.teams[defenders].win_condition = Some(WinCondition::Time);
    }
}

/// The object a property belongs to, with the cursor just past its 4-byte
/// hash: either the object header right before it, or the header that starts
/// the property chain it continues.
fn property_object(c: &Cursor) -> Option<u32> {
    let head = c.behind(13);
    if head.len() == 13 && head[0] == 0x23 && head[5..9] == [0; 4] {
        return Some(u32::from_le_bytes(head[1..5].try_into().expect("4 bytes")));
    }
    if c.behind(5).first() != Some(&0x22) {
        return None;
    }
    owning_object(c.behind(5 + 256), 5)
}

/// The state object id in `23 <id> 00000000 63CC188F`, which follows the
/// operator in a Y11S3 pick packet.
fn state_object_after(window: &[u8]) -> Option<u32> {
    let at = memchr::memmem::find(window, &STATE_PROPERTY)?;
    let head = window.get(at.checked_sub(9)?..at)?;
    (head[0] == 0x23 && head[5..] == [0; 4])
        .then(|| u32::from_le_bytes(head[1..5].try_into().expect("4 bytes")))
}

/// Finds the object a property chain belongs to. `before` ends at a property
/// marker of `marker_len` bytes that is part of the chain; the chain starts
/// with `23 <id u32> 00000000 <hash> <size> <value>` and continues with
/// `22 <hash> <size> <value>` properties up to the marker.
fn owning_object(before: &[u8], marker_len: usize) -> Option<u32> {
    let end = before.len().checked_sub(marker_len)?;
    let chain = &before[..end];
    (0..chain.len().saturating_sub(13)).rev().find_map(|start| {
        if chain[start] != 0x23 || chain[start + 5..start + 9] != [0; 4] {
            return None;
        }
        let mut at = start + 13; // tag, id, padding, hash
        loop {
            at += 1 + usize::from(*chain.get(at)?);
            match chain.get(at) {
                None if at == chain.len() => {
                    let id = chain[start + 1..start + 5].try_into().expect("4 bytes");
                    return Some(u32::from_le_bytes(id));
                }
                Some(0x22) => at += 5,
                _ => return None,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `scan` relies on markers never overlapping: no marker may contain
    /// another, or end with the start of another.
    #[test]
    fn markers_never_overlap() {
        let all: Vec<&[u8]> = PACKETS.iter().map(|p| p.1).chain([LEGACY_TIME]).collect();
        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate() {
                if i == j {
                    continue;
                }
                assert!(
                    memchr::memmem::find(a, b).is_none(),
                    "{a:x?} contains {b:x?}"
                );
                for k in 1..a.len().min(b.len()) {
                    assert_ne!(a[a.len() - k..], b[..k], "{a:x?} overlaps {b:x?}");
                }
            }
        }
    }

    #[test]
    fn owning_object_walks_the_property_chain() {
        let mut bytes = vec![
            0x23, 0x86, 0xF3, 0x2A, 0xF0, 0, 0, 0, 0, 0xA2, 0xBD, 0xF5, 0x7A,
        ];
        bytes.extend([8, 1, 2, 3, 4, 5, 6, 7, 8]);
        bytes.extend([0x22, 0xF9, 0x39, 0x11, 0xF2, 1, 9]);
        let marker = [0x22, 0xA9, 0x26, 0x0B, 0xE4];
        bytes.extend(marker);
        assert_eq!(owning_object(&bytes, 5), Some(0xF02A_F386));
        // A chain broken by a non-property byte has no owner.
        bytes[22] = 0x1A;
        assert_eq!(owning_object(&bytes, 5), None);
    }

    #[test]
    fn scan_finds_markers_across_chunk_boundaries() {
        let (packets, scanner) = &SCANNERS[0];
        let marker = PACKETS[0].1;
        let mut body = vec![0u8; (4 << 20) * 2 + 100];
        // One marker straddling the first chunk boundary, one at the very end.
        let straddle = (4 << 20) - 2;
        body[straddle..straddle + marker.len()].copy_from_slice(marker);
        let tail = body.len() - marker.len();
        body[tail..].copy_from_slice(marker);

        let found = scan(scanner, &body);
        assert_eq!(found, vec![(straddle + marker.len(), 0), (body.len(), 0)]);
        assert!(matches!(packets[0], Packet::Player));
    }
}
