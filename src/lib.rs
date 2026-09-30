//! Parser for Rainbow Six Siege match replays (`.rec` files).
//!
//! A Rust port of [r6-dissect](https://github.com/redraskal/r6-dissect).
//!
//! ```no_run
//! use replay_analyzer::{ReadMode, Round};
//!
//! let round = Round::open("R01.rec", ReadMode::Full)?;
//! println!("{} on {}", round.header.game_mode, round.header.map);
//! # Ok::<(), replay_analyzer::Error>(())
//! ```

mod cursor;
mod decompress;
pub mod details;
pub mod error;
pub mod feedback;
pub mod header;
pub mod matches;
pub mod round;
pub mod stats;
pub mod types;

pub use details::{
    Ban, HealthUpdate, LifeEvent, LifeEventType, Loadout, ObservationSession, Phase,
};
pub use error::{Error, Result};
pub use feedback::{MatchUpdate, MatchUpdateType};
pub use header::{Header, Player, Team};
pub use matches::Match;
pub use round::{ReadMode, Round, decompressed_bytes};
pub use stats::{PlayerMatchStats, PlayerRoundStats};
pub use types::{GameMode, Map, MatchType, ObservationTool, Operator, TeamRole, WinCondition};
