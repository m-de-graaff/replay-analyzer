//! Melee hits and shield actions (Y11S3).

use serde::Serialize;

use crate::loadout::Input;

/// One melee hit on a barricade or map object.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeleeHit {}

/// One shield action.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShieldAction {}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    pub hits: Vec<MeleeHit>,
    pub shields: Vec<ShieldAction>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
}

pub(crate) fn decode(_input: &Input) -> Decoded {
    Decoded::default()
}
