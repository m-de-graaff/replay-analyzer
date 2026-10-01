//! Which decoder handles which game build, and what revision of it ran.
//!
//! The packet layouts change at specific builds (see [`crate::types::version`]).
//! Each range between two changes is one decoder profile. When a decoding fix
//! lands, bump the `revision` of every profile it touches: stored results
//! whose `(decoder, decoderRevision)` differ from the current table are the
//! ones worth re-parsing. `replay-analyzer --decoders` prints the table.
//!
//! Revision bumps:
//! - every profile to its current revision: players short of a full team
//!   are decoded, not partial; from Y8S4 the container, recording times and
//!   timing rates; from Y11S3 the movement and defuser-player trust rules.
//! - Y8S1 onward: ban icons of current operators, and stray bytes no longer
//!   swallow object-tree records; from Y11S3 (profile Y9S4) bans from ban
//!   slots with their team, order and votes for none, team colors, match
//!   type 7 as Unranked, `isSpectator` false when absent, levels decoded
//!   and teams sized by `maxnbplayersperteam`.
//! - every profile: a skipped field no longer lowers `trusted`, overtime
//!   rounds are numbered from 1, a swap at 0:10 is not late, repeated
//!   `Death` entries are dropped. From Y9S4 a round whose score did not
//!   change has no winner. From Y11S3 (profile Y9S4): spawns follow changes
//!   in prep, sides come from the team objects, plants and disables from
//!   `IsDefuserStarted` with their player, the round's end from
//!   `TimerState`, players who left count as gone, and the end reason is
//!   checked against the game's round history.

use serde::Serialize;

use crate::types::version;

/// This crate's version.
pub const PARSER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PARSER_NAME: &str = env!("CARGO_PKG_NAME");

/// Builds whose replays the decoders were checked against, with the
/// `version` their header gives. Replays of all three decode alike.
pub const KNOWN_BUILDS: &[(u32, &str)] = &[
    (9_883_691, "Y11S3_Alpha04"),
    (9_901_603, "Y11S3_Alpha04"),
    (9_918_362, "Y11S3_Alpha04"),
];

/// The newest build the decoders were checked against.
pub const NEWEST_TESTED_BUILD: u32 = 9_918_362;

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
        revision: 4,
        changes: "text clock, legacy feedback and player layout",
    },
    Profile {
        name: "Y7S2",
        min_build: version::Y7S2,
        revision: 4,
        changes: "player id marker changed",
    },
    Profile {
        name: "Y7S4",
        min_build: version::Y7S4,
        revision: 4,
        changes: "player packet carries an operator block",
    },
    Profile {
        name: "Y8S1",
        min_build: version::Y8S1,
        revision: 6,
        changes: "numeric clock; bans, loadouts, health, observation decodable",
    },
    Profile {
        name: "Y8S2",
        min_build: version::Y8S2,
        revision: 6,
        changes: "players matched by packet id",
    },
    Profile {
        name: "Y9S1",
        min_build: version::Y9S1,
        revision: 6,
        changes: "new feedback layout, text messages not decoded",
    },
    Profile {
        name: "Y9S1.3",
        min_build: version::Y9S1_UPDATE3,
        revision: 6,
        changes: "feedback header grew",
    },
    Profile {
        name: "Y9S3",
        min_build: version::Y9S3,
        revision: 6,
        changes: "caster UI ids link attacker swaps",
    },
    Profile {
        name: "Y9S4",
        min_build: version::Y9S4,
        revision: 7,
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

/// The decoder profiles, for telling which stored results a new parser would
/// decode differently.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DecoderTable {
    pub parser: &'static str,
    pub parser_version: &'static str,
    pub newest_tested_build: u32,
    pub known_builds: Vec<KnownBuild>,
    /// Oldest first; every build falls in exactly one.
    pub profiles: Vec<ProfileRow>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KnownBuild {
    pub build: u32,
    pub version: &'static str,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRow {
    pub name: &'static str,
    pub min_build: u32,
    /// Last build the profile handles; absent for the newest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_build: Option<u32>,
    pub revision: u32,
    pub changes: &'static str,
}

pub fn table() -> DecoderTable {
    DecoderTable {
        parser: PARSER_NAME,
        parser_version: PARSER_VERSION,
        newest_tested_build: NEWEST_TESTED_BUILD,
        known_builds: KNOWN_BUILDS
            .iter()
            .map(|&(build, version)| KnownBuild { build, version })
            .collect(),
        profiles: PROFILES
            .iter()
            .enumerate()
            .map(|(i, p)| ProfileRow {
                name: p.name,
                min_build: p.min_build,
                max_build: PROFILES.get(i + 1).map(|n| n.min_build - 1),
                revision: p.revision,
                changes: p.changes,
            })
            .collect(),
    }
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
    fn every_build_seen_in_real_replays_is_tested() {
        for &(build, _) in KNOWN_BUILDS {
            assert!(!ParserInfo::for_build(build).untested_build, "{build}");
        }
        assert!(ParserInfo::for_build(9_918_363).untested_build);
    }

    #[test]
    fn the_table_gives_every_build_one_profile() {
        let t = table();
        assert_eq!(t.profiles[0].min_build, 0);
        for w in t.profiles.windows(2) {
            assert_eq!(w[0].max_build, Some(w[1].min_build - 1));
        }
        assert_eq!(t.profiles.last().unwrap().max_build, None);
        assert_eq!(t.newest_tested_build, NEWEST_TESTED_BUILD);
        assert_eq!(t.known_builds.len(), KNOWN_BUILDS.len());
    }

    #[test]
    fn profiles_are_sorted() {
        assert!(PROFILES.windows(2).all(|w| w[0].min_build < w[1].min_build));
    }
}
