//! Which decoder handles which game build, and what revision of it ran.
//!
//! The packet layouts change at specific builds (see [`crate::types::version`]).
//! Each range between two changes is one decoder profile. When a decoding fix
//! lands, bump the `revision` of every profile it touches: stored results
//! whose `(decoder, decoderRevision)` differ from the current table are the
//! ones worth re-parsing.

use serde::Serialize;

use crate::types::version;

/// This crate's version.
pub const PARSER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PARSER_NAME: &str = env!("CARGO_PKG_NAME");

/// The newest build the decoders were checked against.
pub const NEWEST_TESTED_BUILD: u32 = 9_883_691;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    /// Stable name, stored with results.
    pub name: &'static str,
    /// First build this profile handles.
    pub min_build: u32,
    /// Bumped whenever decoding for this profile changes.
    pub revision: u32,
    /// What is different from the previous profile.
    pub changes: &'static str,
}

/// Oldest first.
pub const PROFILES: &[Profile] = &[
    Profile {
        name: "pre-Y7S2",
        min_build: 0,
        revision: 2,
        changes: "text clock, legacy feedback and player layout",
    },
    Profile {
        name: "Y7S2",
        min_build: version::Y7S2,
        revision: 2,
        changes: "player id marker changed",
    },
    Profile {
        name: "Y7S4",
        min_build: version::Y7S4,
        revision: 2,
        changes: "player packet carries an operator block",
    },
    Profile {
        name: "Y8S1",
        min_build: version::Y8S1,
        revision: 2,
        changes: "numeric clock; bans, loadouts, health, observation decodable",
    },
    Profile {
        name: "Y8S2",
        min_build: version::Y8S2,
        revision: 2,
        changes: "players matched by packet id",
    },
    Profile {
        name: "Y9S1",
        min_build: version::Y9S1,
        revision: 2,
        changes: "new feedback layout, text messages not decoded",
    },
    Profile {
        name: "Y9S1.3",
        min_build: version::Y9S1_UPDATE3,
        revision: 2,
        changes: "feedback header grew",
    },
    Profile {
        name: "Y9S3",
        min_build: version::Y9S3,
        revision: 2,
        changes: "caster UI ids link attacker swaps",
    },
    Profile {
        name: "Y9S4",
        min_build: version::Y9S4,
        revision: 3,
        changes: "starting scores in header; Y11S3 state-object swaps and defuser objects; endtime and property count",
    },
];

/// The profile that decodes `build`.
pub fn select(build: u32) -> &'static Profile {
    PROFILES
        .iter()
        .rev()
        .find(|p| build >= p.min_build)
        .unwrap_or(&PROFILES[0])
}

/// Which parser and decoder produced a result.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ParserInfo {
    pub parser: &'static str,
    pub parser_version: &'static str,
    pub decoder: &'static str,
    pub decoder_revision: u32,
    /// The build is newer than any the decoders were checked against.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub untested_build: bool,
}

impl Default for ParserInfo {
    fn default() -> Self {
        ParserInfo::for_build(0)
    }
}

impl ParserInfo {
    pub fn for_build(build: u32) -> Self {
        let p = select(build);
        ParserInfo {
            parser: PARSER_NAME,
            parser_version: PARSER_VERSION,
            decoder: p.name,
            decoder_revision: p.revision,
            untested_build: build > NEWEST_TESTED_BUILD,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_the_newest_profile_at_or_below_the_build() {
        assert_eq!(select(0).name, "pre-Y7S2");
        assert_eq!(select(version::Y8S1 - 1).name, "Y7S4");
        assert_eq!(select(version::Y8S1).name, "Y8S1");
        assert_eq!(select(9_883_691).name, "Y9S4");
        assert!(ParserInfo::for_build(NEWEST_TESTED_BUILD + 1).untested_build);
    }

    #[test]
    fn profiles_are_sorted() {
        assert!(PROFILES.windows(2).all(|w| w[0].min_build < w[1].min_build));
    }
}
