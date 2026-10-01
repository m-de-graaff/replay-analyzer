//! Shots and bullet hits (Y11S3).

use serde::Serialize;

use crate::loadout::Input;

/// One shot.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shot {}

/// One bullet hit on a player.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    pub shots: Vec<Shot>,
    pub hits: Vec<Hit>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
}

pub(crate) fn decode(_input: &Input) -> Decoded {
    Decoded::default()
}
