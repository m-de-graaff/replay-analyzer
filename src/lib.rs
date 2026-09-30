//! Parser for Rainbow Six Siege match replays (`.rec` files).
//!
//! ```no_run
//! use replay_analyzer::{ReadMode, Round};
//!
//! let round = Round::open("R01.rec", ReadMode::Full)?;
//! println!("{} on {}", round.header.game_mode, round.header.map);
//! # Ok::<(), replay_analyzer::Error>(())
//! ```

pub mod analytics;
pub mod census;
pub mod container;
mod cursor;
pub mod decoder;
mod decompress;
pub mod details;
pub mod entities;
pub mod error;
pub mod feedback;
pub mod file;
pub mod format;
pub mod header;
pub mod identity;
pub mod matches;
pub mod outcome;
pub mod report;
pub mod round;
pub mod stats;
pub mod summary;
pub mod timeline;
pub mod types;

pub use analytics::MatchAnalytics;
pub use census::Census;
pub use container::{Container, DirectoryState, StreamInfo};
pub use decoder::ParserInfo;
pub use details::{
    Ban, HealthUpdate, LifeEvent, LifeEventType, Loadout, ObservationSession, Phase,
};
pub use error::{Error, Result};
pub use feedback::{MatchUpdate, MatchUpdateType};
pub use file::FileInfo;
pub use format::{FormatInfo, GameVersion, Timing};
pub use header::{Header, PartyRole, Player, PlayerEntities, Relation, Team};
pub use identity::{KnownPlayer, PlayerDirectory};
pub use matches::Match;
pub use outcome::{ReasonSource, RoundInfo, RoundOutcome};
pub use report::{DecodeReport, Status};
pub use round::{ReadMode, ReadOptions, Round, decompressed_bytes};
pub use stats::{PlayerMatchStats, PlayerRoundStats};
pub use summary::MatchSummary;
pub use timeline::{PhaseSpan, Timeline};
pub use types::{GameMode, Map, MatchType, ObservationTool, Operator, TeamRole, WinCondition};
