//! Thrown and launched objects (Y11S3).

use serde::Serialize;

use crate::loadout::Input;

/// One throw or launch.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Throw {}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    pub throws: Vec<Throw>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
}

pub(crate) fn decode(_input: &Input) -> Decoded {
    Decoded::default()
}
