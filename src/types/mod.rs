//! Game identifiers found in replays.
//!
//! Ubisoft identifies operators, maps and modes by opaque numeric ids. They are
//! modelled as newtypes over the raw id so unknown values (new seasons) survive
//! a round trip instead of failing to parse.

mod tables;

use std::fmt;

use serde::{Serialize, Serializer, ser::SerializeStruct};

pub use tables::{AttachmentInfo, attachment_info, item_kind, item_name};
use tables::{MAPS, OPERATORS, PLAYLISTS, ROLE_IMAGES};

/// Serializes as `{"name": ..., "id": ...}`.
fn serialize_named<S: Serializer>(s: S, name: &str, id: u64) -> Result<S::Ok, S::Error> {
    let mut st = s.serialize_struct("Named", 2)?;
    st.serialize_field("name", name)?;
    st.serialize_field("id", &id)?;
    st.end()
}

macro_rules! named_id {
    ($(#[$meta:meta])* $name:ident($repr:ty), $lookup:expr) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub $repr);

        impl $name {
            /// The known name for this id, if any.
            pub fn name(self) -> Option<&'static str> {
                #[allow(clippy::redundant_closure_call)]
                ($lookup)(self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self.name() {
                    Some(n) => f.write_str(n),
                    None => write!(f, concat!(stringify!($name), "({})"), self.0),
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                serialize_named(s, &self.to_string(), self.0 as u64)
            }
        }
    };
}

named_id!(
    /// An operator id.
    Operator(u64),
    |id| OPERATORS.iter().find(|op| op.1 == id).map(|op| op.0)
);

named_id!(
    /// A map (world) id.
    Map(u64),
    |id| MAPS.iter().find(|m| m.1 == id).map(|m| m.0)
);

named_id!(
    /// A game mode id.
    GameMode(u64),
    |id| match id {
        327933806 => Some("Bomb"),
        1983085217 => Some("SecureArea"),
        2838806006 => Some("Hostage"),
        400168582901 => Some("QuickMatchBomb"),
        _ => None,
    }
);

/// The kind of match (`matchtype`). The game has renumbered these over the
/// years, so a name holds only for the builds it was seen in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MatchType {
    pub id: u32,
    /// The build (`code`) of the replay the id came from.
    pub build: u32,
}

impl MatchType {
    pub fn new(id: u32, build: u32) -> Self {
        Self { id, build }
    }

    /// The known name for this id in this build, if any.
    pub fn name(self) -> Option<&'static str> {
        match self.id {
            1 => Some("QuickMatch"),
            2 => Some("Ranked"),
            3 => Some("CustomGameLocal"),
            4 => Some("CustomGameOnline"),
            // Inferred from four Y11S3 matches: ranked rules and bans, but
            // players below the level Ranked requires, and Tower, which only
            // Quick Match and Unranked play.
            7 if self.build >= version::Y11S3 => Some("Unranked"),
            8 => Some("Standard"),
            9 => Some("Unranked"),
            _ => None,
        }
    }

    /// A custom game, local or online.
    pub fn is_custom(self) -> bool {
        matches!(self.id, 3 | 4)
    }
}

impl fmt::Display for MatchType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "MatchType({})", self.id),
        }
    }
}

impl Serialize for MatchType {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serialize_named(s, &self.to_string(), u64::from(self.id))
    }
}

/// The playlist a match was queued in, from the header's
/// `playlistcategory` (Y11S3+).
pub fn playlist_name(category: i64) -> Option<&'static str> {
    PLAYLISTS.iter().find(|p| p.0 == category).map(|p| p.1)
}

named_id!(
    /// The kind of observation device a player is looking through.
    ObservationTool(u32),
    |id| match id {
        1 => Some("Drone"),
        2 => Some("Camera"),
        // Seen for defenders whose loadout has a second camera gadget.
        3 => Some("GadgetCamera"),
        6 => Some("BlackEye"),
        8 => Some("FloresDrone"),
        9 => Some("ShockDrone"),
        _ => None,
    }
);

impl ObservationTool {
    /// Drones and other devices attackers drive around.
    pub fn is_drone(self) -> bool {
        matches!(self.0, 1 | 8 | 9)
    }

    /// Fixed or placed cameras.
    pub fn is_camera(self) -> bool {
        matches!(self.0, 2 | 3 | 6)
    }
}

impl Operator {
    pub const RECRUIT: Operator = Operator(359656345734);

    /// The side this operator plays on. `None` for recruits and unknown ids.
    pub fn role(self) -> Option<TeamRole> {
        OPERATORS
            .iter()
            .find(|op| op.1 == self.0)
            .and_then(|op| op.2)
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// The operator whose icon has this role image id, if known.
    pub fn from_role_image(image: u64) -> Option<Operator> {
        let name = ROLE_IMAGES.iter().find(|r| r.0 == image)?.1;
        OPERATORS
            .iter()
            .find(|op| op.0 == name)
            .map(|op| Operator(op.1))
    }
}

impl GameMode {
    pub const BOMB: GameMode = GameMode(327933806);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum TeamRole {
    Attack,
    Defense,
}

impl TeamRole {
    pub fn opposite(self) -> Self {
        match self {
            TeamRole::Attack => TeamRole::Defense,
            TeamRole::Defense => TeamRole::Attack,
        }
    }
}

/// The loadout slot an item id belongs in (Y11S3 HUD item ids).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    Primary,
    Secondary,
    Ability,
    Gadget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum WinCondition {
    KilledOpponents,
    SecuredArea,
    DisabledDefuser,
    DefusedBomb,
    ExtractedHostage,
    Time,
}

/// Game build numbers (`code` header property) at which the format changed.
pub mod version {
    pub const Y7S2: u32 = 7040830;
    pub const Y7S4: u32 = 7338571;
    pub const Y8S1: u32 = 7408213;
    pub const Y8S2: u32 = 7601998;
    pub const Y9S1: u32 = 8111697;
    pub const Y9S1_UPDATE3: u32 = 8211379;
    pub const Y9S3: u32 = 8506016;
    pub const Y9S4: u32 = 8673114;
    /// The oldest Y11S3 build seen; the season may have started earlier.
    pub const Y11S3: u32 = 9883691;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_lookup() {
        assert_eq!(Operator(92270642500).name(), Some("Frost"));
        assert_eq!(Operator(92270642500).role(), Some(TeamRole::Defense));
        assert_eq!(Operator::RECRUIT.role(), None);
        assert_eq!(Operator(1).to_string(), "Operator(1)");
    }

    #[test]
    fn every_role_image_names_a_known_operator() {
        for (image, name) in ROLE_IMAGES {
            let op = Operator::from_role_image(*image);
            assert_eq!(op.and_then(Operator::name), Some(*name), "{image}");
        }
    }

    #[test]
    fn match_type_seven_is_unranked_from_y11s3() {
        assert_eq!(MatchType::new(7, 9_901_603).name(), Some("Unranked"));
        assert_eq!(MatchType::new(7, version::Y9S4).name(), None);
        assert_eq!(MatchType::new(9, 0).name(), Some("Unranked"));
    }

    #[test]
    fn match_types_serialize_as_name_and_id() {
        let json = serde_json::to_string(&MatchType::new(2, 1)).unwrap();
        assert_eq!(json, r#"{"name":"Ranked","id":2}"#);
    }

    #[test]
    fn names_playlists_by_category() {
        assert_eq!(playlist_name(416350367764), Some("Unranked"));
        assert_eq!(playlist_name(1), None);
    }

    #[test]
    fn names_current_maps_and_ban_icons() {
        assert_eq!(Map(419965653950).name(), Some("CalypsoCasino"));
        assert_eq!(Map(398899676157).name(), Some("FortressY10"));
        let icon = |id| Operator::from_role_image(id).and_then(Operator::name);
        assert_eq!(icon(445433447900), Some("Dokkaebi"));
        assert_eq!(icon(104189663973), Some("Iana"));
    }

    #[test]
    fn names_items_and_attachments() {
        assert_eq!(item_name(1366019616), Some("MP7"));
        assert_eq!(item_kind(1366019616), Some(ItemKind::Primary));
        assert_eq!(item_name(1), None);
        assert_eq!(attachment_info(238373620035).name, Some("Laser"));
        assert!(attachment_info(238373621282).name.is_some());
        assert_eq!(attachment_info(1), AttachmentInfo::default());
    }

    #[test]
    fn serializes_name_and_id() {
        let json = serde_json::to_string(&Map(259816839773)).unwrap();
        assert_eq!(json, r#"{"name":"Chalet","id":259816839773}"#);
    }
}
