//! Parser for Rainbow Six Siege match replays (`.rec` files).
//!
//! ```no_run
//! use replay_analyzer::{ReadMode, Round};
//!
//! let round = Round::open("R01.rec", ReadMode::Full)?;
//! println!("{} on {}", round.header.game_mode, round.header.map);
//! # Ok::<(), replay_analyzer::Error>(())
//! ```

pub mod activity;
pub mod analytics;
pub mod areas;
pub mod catalog;
pub mod census;
pub mod combat;
pub mod container;
pub mod cosmetics;
mod cursor;
pub mod decoder;
mod decompress;
pub mod destruction;
pub mod details;
pub mod devices;
pub mod entities;
pub mod error;
pub mod feedback;
pub mod file;
pub mod format;
pub mod fx;
pub mod gadgets;
pub mod gadget_events;
pub mod header;
pub mod identity;
pub mod intel;
pub mod joins;
pub mod join;
pub mod library;
pub mod loadout;
pub mod markers;
pub mod matches;
pub mod melee;
pub mod messages;
pub mod movement;
pub mod objective;
pub mod outcome;
pub mod panels;
pub mod pauses;
pub mod presence;
pub mod records;
pub mod report;
pub mod round;
pub mod settings;
pub mod shots;
pub mod sound;
pub mod stats;
pub mod summary;
pub mod throws;
pub mod timeline;
pub mod types;
pub mod weapons;
pub mod vitals;
pub mod world;

pub use analytics::MatchAnalytics;
pub use census::Census;
pub use container::{Container, DirectoryState, StreamInfo};
pub use cosmetics::Cosmetics;
pub use decoder::ParserInfo;
pub use details::{
    Ban, DownOutcome, HealthUpdate, LifeEvent, LifeEventType, Loadout, ObservationSession, Phase,
};
pub use error::{Error, Result};
pub use feedback::{MatchUpdate, MatchUpdateType};
pub use file::FileInfo;
pub use format::{FormatInfo, GameVersion, Timing};
pub use header::{Header, PartyRole, Platform, Player, PlayerEntities, Relation, Team};
pub use identity::{KnownPlayer, PlayerDirectory};
pub use library::Library;
pub use loadout::LoadoutChange;
pub use matches::Match;
pub use outcome::{ReasonSource, RoundInfo, RoundOutcome};
pub use report::{DecodeReport, Status};
pub use round::{
    PlayerScoreboard, ReadMode, ReadOptions, Round, ScoreTotals, ScoreboardEntry,
    decompressed_bytes,
};
pub use stats::{PlayerMatchStats, PlayerRoundStats};
pub use summary::MatchSummary;
pub use timeline::{PhaseSpan, Timeline};
pub use types::{GameMode, Map, MatchType, ObservationTool, Operator, TeamRole, WinCondition};
pub use vitals::Vitals;
