//! Parsing a single round (one `.rec` file).

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use aho_corasick::AhoCorasick;
use rayon::prelude::*;
use serde::Serialize;

use crate::census::{self, Census, PacketCount};
use crate::container::{self, Container, DirectoryState};
use crate::cursor::Cursor;
use crate::decoder::ParserInfo;
use crate::decompress::{self, Decompressed};
use crate::details::{
    Ban, HealthUpdate, LifeEvent, LifeEventType, Loadout, ObservationSession, Phase,
};
use crate::error::{Error, Result};
use crate::feedback::{Clock, MatchUpdate, MatchUpdateType, display_clock};
use crate::file::{self, FileInfo};
use crate::format::{self, ClockGap, FormatInfo, GameVersion, Hole, Layout, Timing};
use crate::header::{Header, Player};
use crate::outcome::{ReasonSource, RoundInfo, RoundOutcome};
use crate::records::RecordMap;
use crate::report::{DecodeReport, Status};
use crate::stats::PlayerRoundStats;
use crate::timeline::Timeline;
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
    /// The file the round was read from, when read from disk.
    pub file: Option<FileInfo>,
    /// Container layout and prelude.
    pub format: FormatInfo,
    /// Season and build, from the header.
    pub version: GameVersion,
    /// Parser and decoder that produced this round.
    pub parser: ParserInfo,
    /// Number of zstd frames in the file.
    pub zstd_frames: usize,
    /// Streams and compressed blocks (Y8S4+), and whether the file is whole.
    pub container: Option<Container>,
    /// Recording rate and holes, from the frame index and the clock.
    pub timing: Option<Timing>,
    /// Trust level of each output field.
    pub decode: DecodeReport,
    /// Counts of every packet and field seen (only with `ReadOptions::census`).
    pub census: Option<Census>,
    /// The round clock resolved into phases and seconds since prep started.
    pub timeline: Timeline,
    /// Who won, how, and who was alive when action started (full reads).
    pub outcome: RoundOutcome,
    /// Y8S1+: each change of a player's weapon-ready flag, by username.
    pub weapon_ready: Vec<WeaponReady>,
}

/// A player's weapon going up (`ready`) or down.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeaponReady {
    pub username: String,
    pub ready: bool,
    pub phase: Phase,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub elapsed: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScoreboardEntry {
    pub score: u32,
    /// Cumulative match assists as shown on the scoreboard.
    pub assists: u32,
    /// Number of assist updates seen during this round.
    pub assists_from_round: u32,
    /// Y11S3+: cumulative match kills as shown on the scoreboard.
    pub kills: Option<u32>,
    /// Y11S3+: cumulative match deaths as shown on the scoreboard.
    pub deaths: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
enum ScoreField {
    Score,
    Assists,
    Kills,
    Deaths,
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
    /// Only the header and frame index. Y8S4+ replays are not decompressed,
    /// so this is the fast way to list and group many files. Players have no
    /// operators and no packet data is read.
    Header,
}

/// What to read, beyond the round itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadOptions {
    pub mode: ReadMode,
    /// Also count every packet marker and property hash in the stream.
    pub census: bool,
}

impl From<ReadMode> for ReadOptions {
    fn from(mode: ReadMode) -> Self {
        ReadOptions {
            mode,
            census: false,
        }
    }
}

impl Round {
    /// Reads a finished round file. In-progress `.tmprec` recordings are
    /// refused.
    pub fn open(path: impl AsRef<Path>, options: impl Into<ReadOptions>) -> Result<Self> {
        let path = path.as_ref();
        if file::is_temporary(path) {
            return Err(Error::TemporaryFile(path.display().to_string()));
        }
        let raw = std::fs::read(path)?;
        let mut round = Self::from_bytes(&raw, options)?;
        round.file = Some(FileInfo::new(path, &raw));
        Ok(round)
    }

    pub fn from_bytes(raw: &[u8], options: impl Into<ReadOptions>) -> Result<Self> {
        let options = options.into();
        let read = if options.mode == ReadMode::Header {
            decompress::header_only
        } else {
            decompress::decompress
        };
        let Decompressed {
            data,
            header,
            body_start,
            format,
            zstd_frames,
            frame_index,
            container,
        } = read(raw)?;
        let mut parser = Parser::new(&data, header);
        parser.round.format = format;
        parser.round.zstd_frames = zstd_frames;
        parser.round.container = container;
        parser.round.timing = frame_index.as_ref().map(Timing::from_index);
        if let Some(index) = frame_index {
            if index.out_of_order > 0 {
                parser.round.decode.warnings.push(format!(
                    "{} frame index entries out of order",
                    index.out_of_order
                ));
            }
            parser.frame_times = index.times;
        }
        Ok(parser.run(body_start, options))
    }

    /// Parses only the header, without scanning packets. Y8S4+ replays are
    /// not decompressed at all.
    pub fn header_only(raw: &[u8]) -> Result<Header> {
        Ok(decompress::header_only(raw)?.header)
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
            round: RoundInfo,
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
            #[serde(skip_serializing_if = "<[_]>::is_empty")]
            weapon_ready: &'a [WeaponReady],
            replay: ReplayInfo<'a>,
            decode_status: &'a DecodeReport,
            #[serde(skip_serializing_if = "Option::is_none")]
            timing: Option<&'a Timing>,
            #[serde(skip_serializing_if = "Option::is_none")]
            census: Option<&'a Census>,
        }
        Output {
            header: &self.header,
            round: self.info(),
            match_feedback: &self.match_feedback,
            stats: self.player_stats(),
            bans: &self.bans,
            health: &self.health,
            life_events: &self.life_events,
            observation: &self.observation,
            loadouts: &self.loadouts,
            weapon_ready: &self.weapon_ready,
            replay: self.replay_info(),
            decode_status: &self.decode,
            timing: self.timing.as_ref(),
            census: self.census.as_ref(),
        }
        .serialize(s)
    }
}

/// Where a round came from and what read it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayInfo<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<&'a FileInfo>,
    pub format: &'a FormatInfo,
    pub version: &'a GameVersion,
    pub parser: &'a ParserInfo,
    pub zstd_frames: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<&'a Container>,
}

impl Round {
    pub fn replay_info(&self) -> ReplayInfo<'_> {
        ReplayInfo {
            file: self.file.as_ref(),
            format: &self.format,
            version: &self.version,
            parser: &self.parser,
            zstd_frames: self.zstd_frames,
            container: self.container.as_ref(),
        }
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
    DefuserAction,
    WeaponReady,
    ScoreboardKills,
    ScoreboardDeaths,
}

impl Packet {
    const COUNT: usize = 20;

    fn name(self) -> &'static str {
        match self {
            Packet::Player => "player",
            Packet::AttackerSwap => "attackerSwap",
            Packet::Spawn => "spawn",
            Packet::Time => "time",
            Packet::LegacyTime => "legacyTime",
            Packet::Feedback => "feedback",
            Packet::DefuserTimer => "defuserTimer",
            Packet::ScoreboardScore => "scoreboardScore",
            Packet::ScoreboardAssists => "scoreboardAssists",
            Packet::RoleImage => "roleImage",
            Packet::Health => "health",
            Packet::LifeState => "lifeState",
            Packet::Observer => "observer",
            Packet::ObservedOwner => "observedOwner",
            Packet::ObservationTool => "observationTool",
            Packet::Item => "item",
            Packet::DefuserAction => "defuserAction",
            Packet::WeaponReady => "weaponReady",
            Packet::ScoreboardKills => "scoreboardKills",
            Packet::ScoreboardDeaths => "scoreboardDeaths",
        }
    }
}

const PACKETS: [(Packet, &[u8]); 19] = [
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
    (Packet::DefuserAction, &DEFUSER_ACTION),
    (Packet::WeaponReady, &crate::entities::WEAPON_READY),
    (Packet::ScoreboardKills, &[0x1C, 0xD2, 0xB1, 0x9D]),
    (Packet::ScoreboardDeaths, &[0xCD, 0x9C, 0x5D, 0x72]),
    (Packet::Time, &[0x1F, 0x07, 0xEF, 0xC9]),
];
const LEGACY_TIME: &[u8] = &[0x1E, 0xF1, 0x11, 0xAB];

/// Every property hash the parser reads in a replay of this age, for the
/// census.
fn known_fields(legacy: bool) -> Vec<(&'static str, [u8; 4])> {
    let hash = |m: &[u8]| -> [u8; 4] {
        let m = if m.len() == 5 && m[0] == 0x22 {
            &m[1..]
        } else {
            &m[..4]
        };
        m.try_into().expect("4 bytes")
    };
    let time = if legacy {
        (Packet::LegacyTime, LEGACY_TIME)
    } else {
        PACKETS[PACKETS.len() - 1]
    };
    let mut out: Vec<_> = PACKETS[..PACKETS.len() - 1]
        .iter()
        .chain([&time])
        .map(|(p, m)| (p.name(), hash(m)))
        .collect();
    out.extend([
        ("profileId", hash(PROFILE_ID_INDICATOR)),
        ("uiId", hash(UI_ID_INDICATOR)),
        ("currentSite", hash(&CURRENT_SITE)),
        ("killer", hash(&KILL_INDICATOR)),
        ("killWeapon", hash(&KILL_WEAPON)),
        ("playerState", STATE_PROPERTY),
        ("itemIcon", hash(&ITEM_ICON)),
        ("banRole", hash(BAN_ROLE)),
    ]);
    out
}

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
/// Y11S3+: the team that owns a ban slot, 1-based, written right after its
/// side.
const BAN_TEAM: [u8; 5] = [0x22, 0x2E, 0x61, 0xA2, 0xA9];
/// Y11S3+: a player's level as decimal text, on the object carrying their
/// name. Written once in the round's opening snapshot.
const PLAYER_LEVEL: [u8; 5] = [0x22, 0x3F, 0x0F, 0xDC, 0x1F];
/// Name property on the same object, just before the level.
const PLAYER_NAME: [u8; 8] = [0x75, 0x6D, 0x39, 0xD4, 0x00, 0x00, 0x00, 0x00];
/// The opening snapshot, where levels are written, fits well within this.
const SNAPSHOT_BYTES: usize = 4 << 20;
/// A scoreboard kill credit belongs to the feed entry written within this
/// many bytes after it.
const CREDIT_WINDOW: usize = 4096;
/// A countdown step longer than this many seconds skipped time.
const CLOCK_GAP: f64 = 2.0;
/// Y11S3+: what a defuser interaction object is doing: 0 planting,
/// 1 disabling, 2 idle. Written on the object that also carries the
/// countdown text (`DefuserTimer`), which runs from 7.000 down to 0.
const DEFUSER_ACTION: [u8; 4] = [0xE5, 0x8C, 0x06, 0xE9];
/// A countdown that stopped at or below this many seconds finished: at ~30
/// samples a second the last sample written is 0.000 to 0.07.
const DEFUSER_DONE: f64 = 0.1;

struct Parser<'a> {
    data: &'a [u8],
    round: Round,
    clock: Clock,
    last_defuser: Option<usize>,
    planted: bool,
    players_read: u32,
    /// Distinct clock readings in stream order; events keep an index into it.
    readings: Vec<f64>,
    /// Tick at which the plant completed.
    plant_tick: Option<usize>,
    /// Y11S3+ defuser interaction objects: what each is doing and the last
    /// countdown value it showed.
    interactions: HashMap<u32, Interaction>,
    /// Last health each player showed before action started.
    health_before_action: HashMap<String, u32>,
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
    /// `(seen, failed)` per packet kind.
    packet_counts: [(u32, u32); Packet::COUNT],
    /// First error per packet kind.
    packet_errors: [Option<String>; Packet::COUNT],
    warnings: Vec<String>,
    clock_gaps: Vec<ClockGap>,
    /// Whether the clock has switched to the defuser timer since the plant.
    defuser_clock: bool,
    /// Controller object -> username.
    controllers: HashMap<u32, String>,
    /// Scoreboard object -> username.
    scoreboards: HashMap<u32, String>,
    /// Weapon-ready flag changes: username, value, tick.
    weapon_samples: Vec<(String, bool, Option<usize>)>,
    /// Players' objects from the opening snapshot of the object tree.
    entity_players: Vec<crate::entities::PlayerObjects>,
    /// Y11S3 scoreboard values by username, and the first assists value
    /// seen (the total going into the round). Players are only known once
    /// their pick packet is read, often after their scoreboard's first
    /// values, so these are handed over at the end.
    scoreboard_by_name: HashMap<String, (ScoreboardEntry, Option<u32>)>,
    /// Scoreboard kills just credited, with where: the kill-feed entries
    /// written right after them are the kills they count.
    pending_credits: Vec<(String, usize)>,
    /// Offset in `data` of the packet being read. Events keep it, so they can
    /// be placed on the recording's clock once the round is read.
    packet_at: usize,
    /// Offset of the packet that first showed each clock reading.
    reading_offsets: Vec<usize>,
    /// Snapshots and frame records (Y8S4+).
    records: Option<RecordMap>,
    /// Seconds since the recording started, per frame.
    frame_times: Vec<f64>,
}

/// An equipment slot as sent before a pick or swap packet.
struct Item {
    offset: usize,
    id: u64,
    /// Gadgets carry an icon; guns do not.
    icon: u64,
}

/// A Y11S3+ defuser interaction in progress.
#[derive(Clone, Copy, Default)]
struct Interaction {
    /// `DefuserPlantStart` or `DefuserDisableStart` while one runs.
    active: Option<MatchUpdateType>,
    remaining: f64,
    /// Clock tick of the last countdown value: when a finished plant or
    /// disable actually finished. The object goes idle only after the game
    /// has moved on (for a plant, after the clock switched to the defuser
    /// timer).
    last_tick: Option<usize>,
    /// Offset of that countdown value's packet.
    last_offset: Option<usize>,
}

/// A per-player object property, resolved to a player after the stream is read.
struct Sample {
    object: u32,
    value: SampleValue,
    tick: Option<usize>,
    offset: usize,
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
            readings: Vec::new(),
            plant_tick: None,
            interactions: HashMap::new(),
            health_before_action: HashMap::new(),
            pick_slots: HashMap::new(),
            health_objects: HashMap::new(),
            observers: HashMap::new(),
            observed_owner: HashMap::new(),
            samples: Vec::new(),
            pending_items: Vec::new(),
            seen_items: Default::default(),
            packet_counts: [(0, 0); Packet::COUNT],
            packet_errors: Default::default(),
            warnings: Vec::new(),
            clock_gaps: Vec::new(),
            defuser_clock: false,
            controllers: HashMap::new(),
            scoreboards: HashMap::new(),
            weapon_samples: Vec::new(),
            entity_players: Vec::new(),
            scoreboard_by_name: HashMap::new(),
            pending_credits: Vec::new(),
            packet_at: 0,
            reading_offsets: Vec::new(),
            records: None,
            frame_times: Vec::new(),
        }
    }

    fn warn(&mut self, message: String) {
        tracing::warn!("{message}");
        self.warnings.push(message);
    }

    fn code(&self) -> u32 {
        self.round.header.code_version
    }

    fn players(&mut self) -> &mut Vec<Player> {
        &mut self.round.header.players
    }

    fn run(mut self, start: usize, options: ReadOptions) -> Round {
        let mode = options.mode;
        if mode == ReadMode::Header {
            self.finish_report(mode);
            return self.round;
        }
        let end = match mode {
            ReadMode::Full => self.data.len(),
            ReadMode::Partial => (self.data.len() / 3).max(start),
            ReadMode::Header => start,
        };
        self.records = self
            .round
            .container
            .as_ref()
            .and_then(|c| RecordMap::parse(self.data, c.streams.len()));
        self.read_entities(start, end);
        let (packets, scanner) = &SCANNERS[usize::from(self.code() < version::Y8S1)];
        // Handlers run in stream order but never depend on each other's cursor.
        for (offset, pattern) in scan(scanner, &self.data[start..end]) {
            let packet = packets[pattern];
            let mut c = Cursor::new(self.data, start + offset);
            self.packet_at = start + offset;
            self.packet_counts[packet as usize].0 += 1;
            if let Err(e) = self.dispatch(packet, &mut c) {
                tracing::debug!(?packet, offset = start + offset, error = %e, "skipping packet");
                self.packet_counts[packet as usize].1 += 1;
                self.packet_errors[packet as usize]
                    .get_or_insert_with(|| format!("at {}: {e}", start + offset));
            }
        }
        if self.players_read < 10 {
            self.derive_team_roles();
        }
        self.read_levels(start, end);
        self.apply_entities(start, end);
        self.finish_scoreboard();
        self.finish_interactions();
        self.round.timeline = Timeline::resolve(&self.readings, self.plant_tick);
        self.round.timeline.recording = self
            .reading_offsets
            .iter()
            .map(|&o| self.recording_time(o))
            .collect();
        self.place_feedback();
        self.resolve_samples();
        self.resolve_weapon_ready();
        self.name_defuser_players();
        self.link_feed_profiles();
        if mode == ReadMode::Full {
            self.round_end();
        }
        self.measure_records();
        if options.census {
            self.round.census = Some(census::build(
                &self.data[start..],
                &self.round.header,
                self.packet_census(),
                &known_fields(self.code() < version::Y8S1),
            ));
        }
        self.finish_report(mode);
        self.round
    }

    fn packet_census(&self) -> Vec<PacketCount> {
        let legacy = self.code() < version::Y8S1;
        let all = PACKETS
            .iter()
            .copied()
            .chain([(Packet::LegacyTime, LEGACY_TIME)]);
        all.filter(|(p, _)| match p {
            Packet::Time => !legacy,
            Packet::LegacyTime => legacy,
            _ => true,
        })
        .map(|(p, marker)| {
            let (seen, failed) = self.packet_counts[p as usize];
            PacketCount {
                name: p.name(),
                marker: census::hex(marker),
                seen,
                failed,
            }
        })
        .collect()
    }

    /// "3 of 40 health packets failed to decode (first ...)" for each of
    /// `packets` that had failures.
    fn failures(&self, packets: &[Packet]) -> Vec<String> {
        packets
            .iter()
            .filter(|p| self.packet_counts[**p as usize].1 > 0)
            .map(|p| {
                let (seen, failed) = self.packet_counts[*p as usize];
                let first = self.packet_errors[*p as usize].as_deref().unwrap_or("?");
                format!(
                    "{failed} of {seen} {} packets failed to decode (first {first})",
                    p.name()
                )
            })
            .collect()
    }

    /// Fills in version, parser and how far each output field can be trusted.
    fn finish_report(&mut self, mode: ReadMode) {
        let code = self.code();
        self.round.version = GameVersion::parse(&self.round.header.game_version, code);
        self.round.parser = ParserInfo::for_build(code);
        if let Some(t) = self.round.timing.as_mut() {
            t.clock_gaps = std::mem::take(&mut self.clock_gaps);
            let h = &self.round.header;
            if let Some(w) = t.calibrate(h.timestamp, h.start_time, h.end_time) {
                self.round.decode.warnings.push(w);
            }
        }
        let mut r = DecodeReport {
            warnings: std::mem::take(&mut self.round.decode.warnings),
            ..DecodeReport::default()
        };
        let skipped = mode != ReadMode::Full;
        let header_only = mode == ReadMode::Header;
        let modern = code >= version::Y8S1;
        let round = &self.round;
        let h = &round.header;

        let f = r.field("header", Status::Decoded, h.keys.len());
        if !round.format.prelude_decoded {
            f.at_most(Status::Partial)
                .warn("prelude not recognised; header found by scanning");
        }
        if round.version.season.is_none() {
            f.warn(format!("version {:?} is not in YxSy form", h.game_version));
        }
        if round.parser.untested_build {
            f.warn(format!(
                "build {code} is newer than any tested build ({})",
                crate::decoder::NEWEST_TESTED_BUILD
            ));
        }
        let fmt = &round.format;
        if fmt.prelude_decoded {
            if !format::KNOWN_FORMAT_VERSIONS.contains(&fmt.format_version) {
                f.at_most(Status::Partial).warn(format!(
                    "format version {} is not one seen so far ({:?})",
                    fmt.format_version,
                    format::KNOWN_FORMAT_VERSIONS
                ));
            }
            if fmt.label != format::KNOWN_LABEL {
                f.warn(format!(
                    "prelude label {:?} instead of {:?}",
                    fmt.label,
                    format::KNOWN_LABEL
                ));
            }
            let expected = if fmt.format_version >= 8 {
                Layout::Chunked
            } else {
                Layout::Stream
            };
            if fmt.layout != expected {
                f.warn(format!(
                    "format version {} in the {:?} layout",
                    fmt.format_version, fmt.layout
                ));
            }
        }

        match &round.container {
            Some(c) => {
                let clean =
                    c.complete && c.directory == DirectoryState::Valid && c.warnings.is_empty();
                let f = r.field(
                    "container",
                    if clean {
                        Status::Decoded
                    } else {
                        Status::Partial
                    },
                    c.streams.len(),
                );
                f.warnings.extend(c.warnings.iter().cloned());
                if c.complete && !header_only && self.records.is_none() {
                    f.at_most(Status::Partial).warn(
                        "the decompressed data does not split into snapshots and frame records: \
                         events have no recordingTime",
                    );
                }
            }
            None if fmt.layout == Layout::Stream => {
                r.field("container", Status::NotInVersion, 0)
                    .warn("one zstd stream (before Y8S4): streams are not mapped");
            }
            None => {
                r.field("container", Status::Missing, 0)
                    .warn("no frame index, so the stream list could not be found");
            }
        }

        let with_op = h.players.iter().filter(|p| !p.operator.is_empty()).count();
        let f = r.field("players", Status::Decoded, h.players.len());
        if header_only {
            f.at_most(Status::Skipped)
                .warn("header only: names from the header, no operators");
        } else if h.players.is_empty() {
            f.at_most(Status::Missing);
        } else if with_op < h.players.len() || h.players.len() != 10 {
            f.at_most(Status::Partial).warn(format!(
                "{} players, {with_op} with an operator",
                h.players.len()
            ));
        }
        for w in self.failures(&[Packet::Player, Packet::AttackerSwap]) {
            f.at_most(Status::Partial).warn(w);
        }
        f.warnings.extend(self.warnings.iter().cloned());
        if mode == ReadMode::Partial {
            f.warn("partial read: attacker swaps not read");
        }

        let roles = h.teams.iter().filter(|t| t.role.is_some()).count();
        let f = r.field("teamRoles", Status::Inferred, roles);
        if header_only {
            f.at_most(Status::Skipped);
        } else if roles < 2 {
            f.at_most(Status::Missing)
                .warn("no player with an operator of known side");
        }

        let is_bomb = h.game_mode.name().is_some_and(|n| n.contains("Bomb"));
        let f = r.field("site", Status::Decoded, usize::from(!h.site.is_empty()));
        if header_only {
            f.at_most(Status::Skipped);
        } else if h.site.is_empty() {
            f.at_most(if is_bomb {
                Status::Missing
            } else {
                Status::NotInVersion
            });
        }

        // Fields filled from packets. `expected`: absent data is a problem
        // rather than just an uneventful round.
        let mut from_packets = |name: &'static str,
                                count: usize,
                                since_y8s1: bool,
                                expected: bool,
                                packets: &[Packet]| {
            let status = if skipped {
                Status::Skipped
            } else if since_y8s1 && !modern {
                Status::NotInVersion
            } else if count == 0 && expected {
                Status::Missing
            } else {
                Status::Decoded
            };
            let f = r.field(name, status, count);
            if matches!(status, Status::Decoded | Status::Missing) {
                for w in self.failures(packets) {
                    f.at_most(Status::Partial).warn(w);
                }
            }
        };
        let fb = &round.match_feedback;
        let count_kinds =
            |kinds: &[MatchUpdateType]| fb.iter().filter(|u| kinds.contains(&u.kind)).count();
        from_packets(
            "kills",
            count_kinds(&[MatchUpdateType::Kill, MatchUpdateType::Death]),
            false,
            true,
            &[Packet::Feedback],
        );
        from_packets(
            "defuser",
            count_kinds(&[
                MatchUpdateType::DefuserPlantStart,
                MatchUpdateType::DefuserPlantComplete,
                MatchUpdateType::DefuserDisableStart,
                MatchUpdateType::DefuserDisableComplete,
            ]),
            false,
            false,
            &[Packet::DefuserTimer],
        );
        from_packets(
            "operatorSwaps",
            count_kinds(&[MatchUpdateType::OperatorSwap]),
            false,
            false,
            &[Packet::AttackerSwap],
        );
        from_packets(
            "scoreboard",
            round.scoreboard.len(),
            false,
            true,
            &[Packet::ScoreboardScore, Packet::ScoreboardAssists],
        );
        from_packets("bans", round.bans.len(), true, false, &[Packet::RoleImage]);
        from_packets(
            "loadouts",
            round.loadouts.len(),
            true,
            true,
            &[Packet::Item],
        );
        from_packets("health", round.health.len(), true, true, &[Packet::Health]);
        from_packets(
            "lifeEvents",
            round.life_events.len(),
            true,
            false,
            &[Packet::LifeState],
        );
        from_packets(
            "observation",
            round.observation.len(),
            true,
            false,
            &[
                Packet::Observer,
                Packet::ObservedOwner,
                Packet::ObservationTool,
            ],
        );
        // Who the players are and which objects carry them.
        let players = &round.header.players;
        let with_profile = players.iter().filter(|p| !p.profile_id.is_empty()).count();
        let f = r.field("profileIds", Status::Decoded, with_profile);
        if with_profile < players.len() {
            f.at_most(if with_profile == 0 && !modern {
                Status::NotInVersion
            } else {
                Status::Partial
            })
            .warn(format!(
                "{} of {} players have no profile id; their `key` falls back to the username",
                players.len() - with_profile,
                players.len()
            ));
        }
        let you = players
            .iter()
            .filter(|p| p.relation == Some(crate::entities::Relation::You))
            .count();
        if h.is_spectator == Some(true) {
            r.field("recorder", Status::Decoded, 0)
                .warn("spectator recording: no player is `you`, relations are left out");
        } else {
            let f = r.field(
                "recorder",
                if you == 1 {
                    Status::Decoded
                } else {
                    Status::Missing
                },
                you,
            );
            if you == 0 {
                f.warn("the recording player is not among the players (a spectator before Y11S3?)");
            }
        }
        if modern && !header_only {
            let with = players.iter().filter(|p| p.entities.is_some()).count();
            let f = r.field("entities", Status::Decoded, with);
            if with < players.len() {
                f.at_most(if with == 0 {
                    Status::Missing
                } else {
                    Status::Partial
                })
                .warn(format!(
                    "{} players without a controller object",
                    players.len() - with
                ));
            }
            let moving = players
                .iter()
                .filter(|p| p.entities.as_ref().is_some_and(|e| e.movement.is_some()))
                .count();
            let f = r.field(
                "movement",
                if moving == 0 {
                    Status::NotInVersion
                } else if moving < with {
                    Status::Partial
                } else {
                    Status::Decoded
                },
                moving,
            );
            if moving == 0 {
                f.warn("no player table linking bodies to players (seen from Y11S3)");
            }
            let party = players.iter().filter(|p| p.party.is_some()).count();
            let f = r.field("party", Status::Decoded, party);
            if matches!(h.match_type.0, 3 | 4) {
                f.at_most(Status::Skipped).warn(
                    "custom game: the whole lobby counts as one party, so no roles are given",
                );
            } else if h.is_spectator == Some(true) || you == 0 {
                f.at_most(Status::Skipped)
                    .warn("parties are only known relative to the recording player");
            } else {
                f.warn("only the recording player's party is recorded; other parties are not");
            }
            if !skipped || mode == ReadMode::Partial {
                let n = round.weapon_ready.len();
                let f = r.field(
                    "weaponReady",
                    if n == 0 {
                        Status::Missing
                    } else {
                        Status::Inferred
                    },
                    n,
                );
                f.warn(
                    "the controller flag reads as weapon-ready: attackers hold 0 through prep until their body spawns, and it drops briefly (median about 1 s) during action, as on reloads and swaps",
                );
                if mode == ReadMode::Partial {
                    f.warn("partial read: later changes not read");
                }
            }
        }

        let levels = round
            .header
            .players
            .iter()
            .filter(|p| p.level.is_some())
            .count();
        if levels > 0 {
            r.field("levels", Status::Inferred, levels).warn(
                "probably the clearance level: stable across rounds and distinct per player, but not checked against Ubisoft's stats",
            );
        }
        if code >= version::Y9S1 && !skipped {
            r.field("feedbackMessages", Status::NotInVersion, 0)
                .warn("text feed messages (leaves, objective found) are not decoded from Y9S1");
        }

        let won = h.teams.iter().filter(|t| t.won).count();
        let status = match () {
            _ if skipped => Status::Skipped,
            _ if won != 1 => Status::Missing,
            _ if code >= version::Y9S4 => Status::Decoded,
            _ => Status::Inferred,
        };
        let f = r.field("result", status, won);
        if won > 1 {
            f.at_most(Status::Partial)
                .warn("more than one team marked as winner");
        }
        let has_condition = h.teams.iter().any(|t| t.win_condition.is_some());
        let f = r.field(
            "winCondition",
            if skipped {
                Status::Skipped
            } else {
                Status::Inferred
            },
            usize::from(has_condition),
        );
        if !skipped {
            if !has_condition {
                f.at_most(Status::Missing)
                    .warn("no plant, disable, wipe or time-out fits the result");
            }
            match round.outcome.reason_source {
                crate::outcome::ReasonSource::Confirmed => f.warn(
                    "from the kill feed and defuser events; winner agrees with the header score",
                ),
                crate::outcome::ReasonSource::Header => f
                    .at_most(Status::Partial)
                    .warn("the header's winner disagrees with the events; see round.warnings"),
                _ => f.warn("from the kill feed and defuser events"),
            };
            for w in &round.outcome.warnings {
                f.warn(w.clone());
            }
        }

        let spans = round.timeline.spans();
        let f = r.field(
            "phases",
            if skipped {
                Status::Skipped
            } else if spans.is_empty() {
                Status::Missing
            } else {
                Status::Inferred
            },
            spans.len(),
        );
        if !skipped && round.timeline.action_start.is_none() {
            f.at_most(Status::Partial)
                .warn("the clock never switched to the action phase");
        }
        let f = r.field(
            "playersAtStart",
            if skipped {
                Status::Skipped
            } else {
                Status::Inferred
            },
            round.outcome.players_at_start.iter().sum(),
        );
        if !skipped {
            f.warn("alive when action started: no death in prep and health above zero");
        }
        let unnamed = fb
            .iter()
            .filter(|u| {
                u.team.is_some() && u.username.is_empty() && u.kind != MatchUpdateType::OperatorSwap
            })
            .count();
        if unnamed > 0 {
            r.field("defuserPlayers", Status::Partial, unnamed).warn(
                "Y11S3+ defuser events record the side, not the player; named only when one player of that side was alive",
            );
        }

        let f = r.field(
            "timing",
            if round.timing.is_some() {
                Status::Decoded
            } else {
                Status::Missing
            },
            round.timing.as_ref().map_or(0, |t| t.frames),
        );
        if let Some(t) = &round.timing {
            if round.format.prelude_decoded && t.frames != round.format.declared_frames as usize {
                f.at_most(Status::Partial).warn(format!(
                    "prelude declares {} frames, index has {}",
                    round.format.declared_frames, t.frames
                ));
            }
            if !t.gaps.is_empty() {
                f.at_most(Status::Partial)
                    .warn(format!("{} gaps in the frame index", t.gaps.len()));
            }
            if t.backwards > 0 {
                f.at_most(Status::Partial)
                    .warn(format!("{} frames go back in time", t.backwards));
            }
            if !t.clock_gaps.is_empty() {
                f.at_most(Status::Partial)
                    .warn(format!("{} jumps in the in-game clock", t.clock_gaps.len()));
            }
            if !t.holes.is_empty() {
                let longest = t.holes.iter().map(|h| h.seconds).fold(0.0, f64::max);
                f.at_most(Status::Partial).warn(format!(
                    "{} holes in the movement stream, the longest {longest:.1} s",
                    t.holes.len()
                ));
            }
            let movement = self.records.as_ref().is_some_and(|m| {
                m.streams
                    .iter()
                    .any(|s| container::stream_name(s.name_hash) == Some("movement"))
            });
            if self.records.is_some() && !movement {
                f.warn("no movement stream, so holes were not looked for");
            }
        }
        let clock = if modern {
            Packet::Time
        } else {
            Packet::LegacyTime
        };
        if !skipped && self.packet_counts[clock as usize].0 == 0 {
            f.at_most(Status::Partial).warn("no clock packets found");
        }
        if let Some(c) = round.container.as_ref().filter(|c| !c.complete) {
            let at = c.truncated_at.unwrap_or_default();
            let why = if c.main.is_none() {
                format!(
                    "the file stops at byte {at}, before the main stream that holds every \
                     frame record: only the opening snapshots were written"
                )
            } else {
                format!(
                    "the file stops at byte {at}, inside the main stream: later frames are missing"
                )
            };
            for f in &mut r.fields {
                if matches!(f.status, Status::Missing | Status::Partial)
                    && !matches!(f.field, "header" | "container" | "timing")
                {
                    f.warn("the file is incomplete (see warnings)");
                }
            }
            r.warnings.push(why);
        }
        r.finish();
        self.round.decode = r;
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
            Packet::DefuserAction => self.read_defuser_action(c),
            Packet::WeaponReady => self.read_weapon_ready(c),
            Packet::ScoreboardKills => self.read_scoreboard_object(c, ScoreField::Kills),
            Packet::ScoreboardDeaths => self.read_scoreboard_object(c, ScoreField::Deaths),
        }
    }

    fn push(&mut self, update: MatchUpdate) {
        tracing::debug!(?update, "match update");
        self.round.match_feedback.push(update);
    }

    fn update(&self, kind: MatchUpdateType, username: &str) -> MatchUpdate {
        let mut u = MatchUpdate::new(kind, &self.clock);
        u.username = username.to_owned();
        u.offset = Some(self.packet_at);
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
            self.warn(format!("invalid player packet for {username} ({operator})"));
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
        let warnings = &mut self.warnings;
        header.players.retain(|p| {
            if p.operator.is_empty() {
                tracing::warn!(username = %p.username, "operator id was 0, removing player");
                warnings.push(format!("{}: operator id was 0, player removed", p.username));
            }
            !p.operator.is_empty()
        });
        // 5v5 unless the header says otherwise (Y11S3+ `maxnbplayersperteam`).
        let max = 2 * header.max_players_per_team.unwrap_or(5) as usize;
        if header.players.len() > max {
            tracing::warn!(players = header.players.len(), max, "too many players");
            warnings.push(format!("{} players, more than {max}", header.players.len()));
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
            let previous = self.round.header.players[i].operator;
            // The same pick can be written again; that is no swap.
            if previous == operator {
                return Ok(());
            }
            self.take_loadout(c.pos(), &username, operator);
            self.players()[i].operator = operator;
            let mut u = self.update(MatchUpdateType::OperatorSwap, &username);
            u.operator = operator;
            u.previous_operator = previous;
            u.team = Some(self.round.header.players[i].team_index);
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
        let t = f64::from(c.u32()?);
        self.set_clock(Clock {
            seconds: t,
            display: display_clock(t),
            tick: None,
        });
        Ok(())
    }

    /// Moves the clock, noting any seconds it skipped while counting down.
    fn set_clock(&mut self, mut clock: Clock) {
        let prev = self.clock.seconds;
        let started = !self.readings.is_empty();
        // The opening snapshot can hold a stale 0:00 before prep starts.
        if !started && clock.seconds == 0.0 {
            self.clock = clock;
            return;
        }
        if self.readings.last() != Some(&clock.seconds) {
            self.readings.push(clock.seconds);
            self.reading_offsets.push(self.packet_at);
        }
        clock.tick = Some(self.readings.len() - 1);
        // The countdown drops one second at a time. Expected jumps: up at a
        // new phase, down to 0:00 when the round ends, and down to the
        // defuser timer once after a plant.
        // Y11S3+ plants are confirmed only once the object goes idle, after
        // the clock has switched; a countdown at zero is as good.
        let planting_done = self.interactions.values().any(|i| {
            i.active == Some(MatchUpdateType::DefuserPlantStart) && i.remaining <= DEFUSER_DONE
        });
        let defuser_reset = (self.planted || planting_done) && !self.defuser_clock;
        if defuser_reset && clock.seconds < prev {
            self.defuser_clock = true;
        } else if started && clock.seconds > 0.0 && clock.seconds < prev - CLOCK_GAP {
            self.clock_gaps.push(ClockGap {
                from: self.clock.display.clone(),
                to: clock.display.clone(),
                missing: prev - clock.seconds - 1.0,
            });
        }
        self.clock = clock;
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
        self.set_clock(Clock {
            seconds,
            display: text,
            tick: None,
        });
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
        u.offset = Some(self.packet_at);
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
            // Y11S3: the scoreboard credits the kill right before the feed
            // entry, sometimes to a teammate (who downed the victim) rather
            // than the player the feed names.
            if let Some(credited) = self.take_credit(&username, c.pos()) {
                u.credited_to = credited;
            }
            u.target = target;
            u.headshot = Some(headshot);
            u.weapon = weapon;
            self.push(u);
        }
        Ok(())
    }

    fn read_defuser_timer(&mut self, c: &mut Cursor) -> Result<()> {
        // Y11S3+: the countdown of an interaction object.
        if let Some(object) = property_object(c)
            && self.interactions.contains_key(&object)
        {
            let timer = c.string()?;
            if let Ok(remaining) = timer.parse::<f64>()
                && let Some(i) = self.interactions.get_mut(&object)
            {
                i.remaining = remaining;
                i.last_tick = self.clock.tick;
                i.last_offset = Some(self.packet_at);
            }
            return Ok(());
        }
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
            self.plant_tick = self.clock.tick;
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

    /// Y11S3+: a defuser interaction object starts or stops planting or
    /// disabling. The replay does not say who holds it, only what it does.
    fn read_defuser_action(&mut self, c: &mut Cursor) -> Result<()> {
        let Some(object) = property_object(c) else {
            return Ok(());
        };
        let action = c.u32()?;
        let entry = self.interactions.entry(object).or_default();
        let kind = match action {
            0 => MatchUpdateType::DefuserPlantStart,
            1 => MatchUpdateType::DefuserDisableStart,
            2 => {
                let done = std::mem::take(entry);
                self.finish_interaction(done);
                return Ok(());
            }
            _ => return Ok(()),
        };
        if entry.active == Some(kind) {
            return Ok(());
        }
        let previous = std::mem::replace(
            entry,
            Interaction {
                active: Some(kind),
                remaining: f64::INFINITY,
                last_tick: None,
                last_offset: None,
            },
        );
        self.finish_interaction(previous);
        let mut u = self.update(kind, "");
        u.team = self.side_team(kind);
        self.push(u);
        Ok(())
    }

    /// Reports a plant or disable that ran down to zero as completed.
    fn finish_interaction(&mut self, i: Interaction) {
        let Some(started) = i.active else { return };
        if i.remaining > DEFUSER_DONE {
            return;
        }
        let kind = if started == MatchUpdateType::DefuserPlantStart {
            if self.planted {
                return;
            }
            self.planted = true;
            self.plant_tick = i.last_tick.or(self.clock.tick);
            MatchUpdateType::DefuserPlantComplete
        } else {
            MatchUpdateType::DefuserDisableComplete
        };
        let mut u = self.update(kind, "");
        u.team = self.side_team(kind);
        u.tick = i.last_tick.or(u.tick);
        u.offset = i.last_offset.or(u.offset);
        self.push(u);
    }

    /// Interactions still running when the recording stops.
    fn finish_interactions(&mut self) {
        let open: Vec<Interaction> = self.interactions.drain().map(|(_, i)| i).collect();
        for i in open {
            self.finish_interaction(i);
        }
    }

    /// The team that plants (attack) or disables (defense).
    fn side_team(&self, kind: MatchUpdateType) -> Option<usize> {
        let side = match kind {
            MatchUpdateType::DefuserPlantStart | MatchUpdateType::DefuserPlantComplete => {
                TeamRole::Attack
            }
            _ => TeamRole::Defense,
        };
        self.round
            .header
            .teams
            .iter()
            .position(|t| t.role == Some(side))
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
        let team = if c.peek(BAN_TEAM.len()) == BAN_TEAM {
            c.skip(BAN_TEAM.len())?;
            match c.u32()? {
                t @ 1..=2 => Some(t as usize - 1),
                _ => None,
            }
        } else {
            None
        };
        if !self.round.bans.iter().any(|b| b.icon == icon) {
            let operator = Operator::from_role_image(icon);
            tracing::debug!(icon, ?operator, ?role, ?team, "ban");
            self.round.bans.push(Ban {
                operator,
                role,
                team,
                icon,
            });
        }
        Ok(())
    }

    /// Y11S3+ player levels from the opening snapshot: each follows the
    /// player's name on the same object.
    fn read_levels(&mut self, start: usize, end: usize) {
        let data = &self.data[start..end.min(start + SNAPSHOT_BYTES)];
        for at in memchr::memmem::find_iter(data, &PLAYER_LEVEL) {
            let mut c = Cursor::new(data, at + PLAYER_LEVEL.len());
            let Some(level) = c.string().ok().and_then(|v| v.parse::<u32>().ok()) else {
                continue;
            };
            let before = &data[at.saturating_sub(96)..at];
            let Some(name_at) = memchr::memmem::rfind(before, &PLAYER_NAME) else {
                continue;
            };
            let mut c = Cursor::new(before, name_at + PLAYER_NAME.len());
            let Ok(name) = c.string() else { continue };
            if let Some(p) = self.players().iter_mut().find(|p| p.username == name) {
                p.level.get_or_insert(level);
            }
        }
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
            tick: self.clock.tick,
            offset: self.packet_at,
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

    fn recording_time(&self, offset: usize) -> Option<f64> {
        recording_time(self.records.as_ref(), &self.frame_times, offset)
    }

    /// Record counts per stream, the rate the game sent updates at, and holes
    /// in the movement stream, which has a record at every update.
    fn measure_records(&mut self) {
        let Some(map) = &self.records else { return };
        if let Some(c) = self.round.container.as_mut() {
            for s in &mut c.streams {
                if let Some(sub) = map.streams.iter().find(|x| x.name_hash == s.name_hash) {
                    s.records = Some(sub.frames.len() as u32);
                    s.record_bytes = Some(sub.bytes);
                }
            }
        }
        let times = &self.frame_times;
        let Some(timing) = self.round.timing.as_mut() else {
            return;
        };
        for sub in &map.streams {
            // Frame 0 holds the record every stream starts with.
            let t: Vec<f64> = sub
                .frames
                .iter()
                .filter(|&&f| f > 0)
                .filter_map(|&f| times.get(f as usize).copied())
                .collect();
            if t.len() < MIN_RECORDS {
                continue;
            }
            let span = t[t.len() - 1] - t[0];
            match container::stream_name(sub.name_hash) {
                Some("state") if span > 0.0 => {
                    timing.data_rate = Some(((t.len() - 1) as f64 / span * 100.0).round() / 100.0);
                }
                Some("movement") => {
                    let mut intervals: Vec<f64> = t.windows(2).map(|w| w[1] - w[0]).collect();
                    intervals.sort_by(f64::total_cmp);
                    let limit = (intervals[intervals.len() / 2] * HOLE_FACTOR).max(MIN_HOLE);
                    timing.holes = t
                        .windows(2)
                        .filter(|w| w[1] - w[0] > limit)
                        .map(|w| Hole {
                            at: (w[0] * 1000.0).round() / 1000.0,
                            seconds: ((w[1] - w[0]) * 1000.0).round() / 1000.0,
                        })
                        .collect();
                }
                _ => {}
            }
        }
    }

    /// Puts every feed entry on the round's timeline: phase, seconds since
    /// prep, and the clock it happened at (the last live second for events
    /// the game logged after resetting the clock at round end).
    fn place_feedback(&mut self) {
        let times: Vec<Option<f64>> = self
            .round
            .match_feedback
            .iter()
            .map(|u| u.offset.and_then(|o| self.recording_time(o)))
            .collect();
        for (u, t) in self.round.match_feedback.iter_mut().zip(times) {
            u.recording_time = t;
        }
        let Round {
            timeline,
            match_feedback,
            ..
        } = &mut self.round;
        // Events stamped with an earlier tick (a finished plant) move back
        // into place; the sort is stable, so stream order holds otherwise.
        match_feedback.sort_by_key(|u| u.tick);
        for u in match_feedback {
            let at = timeline.at(u.tick);
            u.phase = at.phase;
            u.elapsed = at.elapsed;
            if u.tick.is_some() && at.seconds != u.time_in_seconds {
                u.time_in_seconds = at.seconds;
                u.time = display_clock(at.seconds);
            }
        }
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
        let before_action = &mut self.health_before_action;
        let (records, frame_times) = (self.records.as_ref(), &self.frame_times);
        let recorded = |offset: usize| recording_time(records, frame_times, offset);
        let round = &mut self.round;
        let timeline = &round.timeline;
        for s in &self.samples {
            let at = timeline.at(s.tick);
            let time = display_clock(at.seconds);
            match &s.value {
                SampleValue::Health(value) => {
                    let Some(username) = owner(&self.health_objects, s.object, 1) else {
                        continue;
                    };
                    let prev = health.insert(s.object, *value);
                    if at.phase == Phase::Prep {
                        before_action.insert(username.clone(), *value);
                    }
                    // Prep phase sets up (or resets leftover) health; rising
                    // from zero is a spawn or a revive, not healing.
                    let live = at.phase.is_live();
                    if let Some(prev) = prev.filter(|&p| live && p > 0 && p != *value) {
                        round.health.push(HealthUpdate {
                            username,
                            health: *value,
                            change: *value as i32 - prev as i32,
                            time,
                            time_in_seconds: at.seconds,
                            phase: at.phase,
                            elapsed: at.elapsed,
                            recording_time: recorded(s.offset),
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
                        time,
                        time_in_seconds: at.seconds,
                        phase: at.phase,
                        elapsed: at.elapsed,
                        recording_time: recorded(s.offset),
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
                        round.observation[i].seconds = at.elapsed - start;
                    }
                    if tool.0 != 0 {
                        open.insert(s.object, (round.observation.len(), at.elapsed));
                        round.observation.push(ObservationSession {
                            username,
                            owner: device_owner.clone(),
                            tool: *tool,
                            phase: at.phase,
                            time,
                            time_in_seconds: at.seconds,
                            elapsed: at.elapsed,
                            recording_time: recorded(s.offset),
                            seconds: 0.0,
                        });
                    }
                }
            }
        }
        let end = round.timeline.duration();
        for (i, start) in open.into_values() {
            round.observation[i].seconds = end - start;
        }
    }

    /// Y11S3 defuser events carry only the side. Names the player when just
    /// one player of that side was alive at the time.
    fn name_defuser_players(&mut self) {
        let round = &mut self.round;
        let deaths: Vec<(String, f64)> = round
            .match_feedback
            .iter()
            .filter_map(|u| Some((u.victim()?.to_owned(), u.elapsed)))
            .collect();
        let players = &round.header.players;
        for u in &mut round.match_feedback {
            let defuser = matches!(
                u.kind,
                MatchUpdateType::DefuserPlantStart
                    | MatchUpdateType::DefuserPlantComplete
                    | MatchUpdateType::DefuserDisableStart
                    | MatchUpdateType::DefuserDisableComplete
            );
            let Some(team) = u.team.filter(|_| defuser && u.username.is_empty()) else {
                continue;
            };
            let mut alive = players.iter().filter(|p| {
                p.team_index == team
                    && !deaths
                        .iter()
                        .any(|(name, at)| *name == p.username && *at < u.elapsed)
            });
            if let (Some(only), None) = (alive.next(), alive.next()) {
                u.username = only.username.clone();
            }
        }
    }

    /// Y8S1+: finds every player's objects in the opening snapshot, so
    /// packets written to them (scoreboard, weapon-ready flag) can be
    /// attributed while the stream is read.
    fn read_entities(&mut self, start: usize, end: usize) {
        if self.code() < version::Y8S1 {
            return;
        }
        self.entity_players = crate::entities::players(&self.data[start..end]);
        for o in &self.entity_players {
            self.controllers.insert(o.controller, o.username.clone());
            if let Some(sb) = o.scoreboard {
                self.scoreboards.insert(sb, o.username.clone());
            }
        }
    }

    /// Hands each player their objects, who they are to the recorder, their
    /// party role, and (Y11S3+) the body the movement stream moves.
    fn apply_entities(&mut self, start: usize, end: usize) {
        use crate::entities::Relation;
        let objects = std::mem::take(&mut self.entity_players);
        let header = &mut self.round.header;
        let custom = matches!(header.match_type.0, 3 | 4);
        let spectator = header.is_spectator == Some(true);
        let find = |players: &[Player], o: &crate::entities::PlayerObjects| {
            players
                .iter()
                .position(|p| o.player_id != 0 && p.id == o.player_id)
                .or_else(|| players.iter().position(|p| p.username == o.username))
        };
        for o in &objects {
            let Some(i) = find(&header.players, o) else {
                continue;
            };
            let p = &mut header.players[i];
            if p.profile_id.is_empty() {
                p.profile_id = o.profile_id.clone();
            }
            p.entities = Some(crate::header::PlayerEntities {
                controller: o.controller,
                scoreboard: o.scoreboard,
                health: o.health,
                movement: None,
            });
            p.relation = (!spectator && o.relation == Some(5)).then_some(Relation::You);
        }
        header.assign_relations();
        // Party roles are those of the recorder's party: only players the
        // profile marks as the recorder or a teammate in their party.
        if !spectator && !custom {
            for o in &objects {
                let Some(i) = find(&header.players, o) else {
                    continue;
                };
                let p = &mut header.players[i];
                let with_you = matches!(o.relation, Some(3 | 5))
                    && matches!(p.relation, Some(Relation::You | Relation::Teammate));
                p.party = match o.party_role {
                    Some(2) if with_you => Some(crate::header::PartyRole::Leader),
                    Some(1) if with_you => Some(crate::header::PartyRole::Member),
                    _ => None,
                };
            }
        }
        if objects.is_empty() {
            return;
        }
        // Bodies from the player table, in the order each player got them.
        let body = &self.data[start..end];
        let ids: Vec<u64> = header.players.iter().map(|p| p.id).collect();
        let mut bodies: HashMap<u64, Vec<u32>> = HashMap::new();
        for change in crate::entities::possessions(body, &ids) {
            if let Some(b) = change.body {
                let list = bodies.entry(change.player_id).or_default();
                if list.last() != Some(&b) {
                    list.push(b);
                }
            }
        }
        let first: Vec<u32> = bodies.values().filter_map(|l| l.first().copied()).collect();
        let spawns = crate::entities::spawn_positions(body, &first);
        for p in &mut header.players {
            let Some(list) = bodies.get(&p.id) else {
                continue;
            };
            if let Some(e) = p.entities.as_mut() {
                e.movement = list.last().copied();
            }
            p.spawn_position = list.first().and_then(|b| spawns.get(b).copied());
        }
    }

    fn read_weapon_ready(&mut self, c: &mut Cursor) -> Result<()> {
        let Some(object) = property_object(c) else {
            return Ok(());
        };
        let Some(name) = self.controllers.get(&object).cloned() else {
            return Ok(());
        };
        if c.u8()? != 1 {
            return Ok(());
        }
        let ready = match c.u8()? {
            0 => false,
            1 => true,
            _ => return Ok(()),
        };
        self.weapon_samples.push((name, ready, self.clock.tick));
        Ok(())
    }

    /// Places weapon-ready changes on the timeline, dropping repeats.
    fn resolve_weapon_ready(&mut self) {
        let mut last: HashMap<&str, bool> = HashMap::new();
        let timeline = &self.round.timeline;
        for (name, ready, tick) in &self.weapon_samples {
            if last.insert(name, *ready) == Some(*ready) {
                continue;
            }
            let at = timeline.at(*tick);
            self.round.weapon_ready.push(WeaponReady {
                username: name.clone(),
                ready: *ready,
                phase: at.phase,
                elapsed: at.elapsed,
            });
        }
    }

    /// Adds the profile ids of the players a feed entry names.
    fn link_feed_profiles(&mut self) {
        let players = &self.round.header.players;
        let profile = |name: &str| {
            players
                .iter()
                .find(|p| !name.is_empty() && p.username == name)
                .map(|p| p.profile_id.clone())
                .unwrap_or_default()
        };
        for u in &mut self.round.match_feedback {
            u.profile_id = profile(&u.username);
            u.target_profile_id = profile(&u.target);
        }
    }

    /// The player whose scoreboard object the property at `c` belongs to
    /// (Y11S3+).
    fn scoreboard_owner(&self, c: &Cursor) -> Option<String> {
        self.scoreboards.get(&property_object(c)?).cloned()
    }

    /// Y11S3+: a value written to a player's scoreboard object. Values are
    /// match totals; the first assists value seen is the total going into
    /// the round.
    fn read_scoreboard_object(&mut self, c: &mut Cursor, field: ScoreField) -> Result<()> {
        let Some(name) = self.scoreboard_owner(c) else {
            return Ok(());
        };
        let v = c.u32()?;
        let (e, base) = self.scoreboard_by_name.entry(name.clone()).or_default();
        match field {
            ScoreField::Score => e.score = v,
            ScoreField::Assists => {
                let base = *base.get_or_insert(v);
                e.assists = v;
                e.assists_from_round = v.saturating_sub(base);
            }
            ScoreField::Kills => {
                if e.kills.is_some_and(|k| v > k) {
                    self.pending_credits.push((name, c.pos()));
                }
                e.kills = Some(v);
            }
            ScoreField::Deaths => e.deaths = Some(v),
        }
        Ok(())
    }

    /// The teammate the scoreboard credited with the kill `killer` is named
    /// for in the feed, when that is not `killer`. Credits older than
    /// `CREDIT_WINDOW` bytes are dropped; a credit to the killer is used up
    /// first.
    fn take_credit(&mut self, killer: &str, at: usize) -> Option<String> {
        self.pending_credits
            .retain(|(_, from)| at.saturating_sub(*from) < CREDIT_WINDOW);
        if let Some(i) = self.pending_credits.iter().position(|(n, _)| n == killer) {
            self.pending_credits.remove(i);
            return None;
        }
        let team = |name: &str| {
            let p = self
                .round
                .header
                .players
                .iter()
                .find(|p| p.username == name)?;
            Some(p.team_index)
        };
        let killer_team = team(killer)?;
        let i = self
            .pending_credits
            .iter()
            .position(|(n, _)| team(n) == Some(killer_team))?;
        Some(self.pending_credits.remove(i).0)
    }

    /// Files the Y11S3 scoreboard values under each player's packet id.
    fn finish_scoreboard(&mut self) {
        for (name, (entry, _)) in std::mem::take(&mut self.scoreboard_by_name) {
            if let Some(id) = self
                .round
                .header
                .players
                .iter()
                .find(|p| p.username == name)
                .and_then(|p| p.dissect_id)
            {
                self.round.scoreboard.insert(id, entry);
            }
        }
    }

    fn read_scoreboard_score(&mut self, c: &mut Cursor) -> Result<()> {
        if self.scoreboard_owner(c).is_some() {
            return self.read_scoreboard_object(c, ScoreField::Score);
        }
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
        if self.scoreboard_owner(c).is_some() {
            return self.read_scoreboard_object(c, ScoreField::Assists);
        }
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

    /// Decides which team won and how, cross-checking the header's score
    /// (Y9S4+) against the kill feed and defuser events, and counts who was
    /// alive when action started.
    fn round_end(&mut self) {
        let round = &mut self.round;
        let header = &mut round.header;
        let feed = &round.match_feedback;
        let mut outcome = RoundOutcome::default();

        let team_of = |name: &str| {
            header
                .players
                .iter()
                .find(|p| p.username == name)
                .map(|p| p.team_index)
                .filter(|&t| t < 2)
        };
        let prep_deaths: Vec<&str> = feed
            .iter()
            .filter(|u| u.phase == Phase::Prep)
            .filter_map(MatchUpdate::victim)
            .collect();
        let mut roster = [0usize; 2];
        for p in header.players.iter().filter(|p| p.team_index < 2) {
            roster[p.team_index] += 1;
            let dead_in_prep = prep_deaths.contains(&p.username.as_str());
            let no_health = self.health_before_action.get(&p.username) == Some(&0);
            if dead_in_prep || no_health {
                outcome.down_at_start.push(p.username.clone());
            } else {
                outcome.players_at_start[p.team_index] += 1;
            }
        }

        // Deaths of players alive at the start, with when the last one fell.
        let mut dead: Vec<&str> = Vec::new();
        let mut last_death = [f64::NEG_INFINITY; 2];
        for u in feed.iter().filter(|u| u.phase != Phase::Prep) {
            let Some(victim) = u.victim() else { continue };
            let Some(t) = team_of(victim) else { continue };
            if dead.contains(&victim) || outcome.down_at_start.iter().any(|d| d == victim) {
                continue;
            }
            dead.push(victim);
            outcome.deaths[t] += 1;
            last_death[t] = last_death[t].max(u.elapsed);
        }
        let kinds = |k: MatchUpdateType| feed.iter().filter(move |u| u.kind == k);
        outcome.planted = kinds(MatchUpdateType::DefuserPlantComplete)
            .next()
            .is_some();
        outcome.disabled = kinds(MatchUpdateType::DefuserDisableComplete)
            .next()
            .is_some();

        let role_team = |role| header.teams.iter().position(|t| t.role == Some(role));
        let (attack, defense) = match (role_team(TeamRole::Attack), role_team(TeamRole::Defense)) {
            (Some(a), Some(d)) => (a, d),
            _ => {
                // Without sides, fall back to who planted or disabled.
                let planter = kinds(MatchUpdateType::DefuserPlantComplete)
                    .find_map(|u| u.team.or_else(|| team_of(&u.username)));
                match planter {
                    Some(a) => (a, a ^ 1),
                    None => {
                        outcome
                            .warnings
                            .push("sides unknown: no result from events".into());
                        round.outcome = outcome;
                        return;
                    }
                }
            }
        };
        let wiped = |t: usize| {
            outcome.players_at_start[t] > 0 && outcome.deaths[t] >= outcome.players_at_start[t]
        };

        // Who the events say won.
        let from_events = if outcome.disabled {
            defense
        } else if outcome.planted {
            attack
        } else {
            match (wiped(attack), wiped(defense)) {
                // Both wiped (a trade, or a player left): the later wipe won.
                (true, true) if last_death[attack] > last_death[defense] => attack,
                (true, _) => defense,
                (false, true) => attack,
                // Nobody wiped and nothing planted: defenders win on time.
                (false, false) => defense,
            }
        };

        // Y9S4+ headers state the score after the round.
        let explicit = header.code_version >= version::Y9S4;
        let from_header = explicit
            .then(|| (0..2).find(|&t| header.teams[t].score > header.teams[t].starting_score))
            .flatten();
        let winner = match from_header {
            Some(h) => {
                outcome.reason_source = if h == from_events {
                    ReasonSource::Confirmed
                } else {
                    outcome.warnings.push(format!(
                        "header says team {h} won, the kill feed and defuser events suggest team {from_events}"
                    ));
                    ReasonSource::Header
                };
                h
            }
            None => {
                if explicit {
                    outcome
                        .warnings
                        .push("header score did not change; winner taken from events".into());
                }
                outcome.reason_source = ReasonSource::Events;
                from_events
            }
        };

        // How the winner won, given what happened.
        let reason = if winner == defense {
            if outcome.planted {
                if !outcome.disabled {
                    outcome.warnings.push(
                        "defenders won after a plant, but no completed disable was seen".into(),
                    );
                }
                Some(WinCondition::DisabledDefuser)
            } else if wiped(attack) {
                Some(WinCondition::KilledOpponents)
            } else {
                // Time ran out: the clock should have counted down to 0:00.
                let end = round.timeline.end_start.map(|e| round.timeline.ticks[e]);
                if end.is_some_and(|t| t.seconds > 1.0) {
                    outcome.warnings.push(format!(
                        "defenders won with attackers alive and {} left on the clock",
                        crate::feedback::display_clock(end.map_or(0.0, |t| t.seconds))
                    ));
                }
                Some(WinCondition::Time)
            }
        } else if wiped(defense) {
            Some(WinCondition::KilledOpponents)
        } else if outcome.planted {
            Some(WinCondition::DefusedBomb)
        } else {
            outcome.warnings.push(
                "attackers won without a plant or eliminating the defenders (a player left?)"
                    .into(),
            );
            None
        };
        if outcome.disabled && winner != defense {
            outcome
                .warnings
                .push("a completed disable was seen, but attackers won".into());
        }

        for (t, team) in header.teams.iter_mut().enumerate() {
            team.won = t == winner;
            team.win_condition = if t == winner { reason } else { None };
        }
        outcome.winner = Some(winner);
        outcome.reason = reason;
        round.outcome = outcome;
    }
}

/// The object a property belongs to, with the cursor just past its 4-byte
/// hash: either the object header right before it, or the header that starts
/// the property chain it continues.
/// Seconds since the recording started for the packet at `offset`, to the
/// frame, in milliseconds. `None` for packets in a snapshot, or without frame
/// records.
fn recording_time(records: Option<&RecordMap>, frame_times: &[f64], offset: usize) -> Option<f64> {
    let frame = records?.frame_at(offset)?;
    let t = *frame_times.get(frame as usize)?;
    Some((t * 1000.0).round() / 1000.0)
}

/// Fewer records than this say nothing about a stream's rate.
const MIN_RECORDS: usize = 100;
/// Movement records further apart than this, and than `HOLE_FACTOR` times
/// their median distance (35 ms), leave a hole. The movement stream has a
/// record at every update; the most seen between two is 69 ms. Other streams
/// go quiet for seconds when nothing changes.
const MIN_HOLE: f64 = 0.5;
const HOLE_FACTOR: f64 = 10.0;

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
