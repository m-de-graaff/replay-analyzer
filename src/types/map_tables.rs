//! Assets of what players and the map put on walls, hatches, doors and
//! windows (Y11S3), for [`crate::panels`].

/// What a panel asset is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PanelAsset {
    /// A wall reinforcement. The game has one asset per width of wall;
    /// the width in metres is not in the file. It is inferred from the
    /// order of the asset ids (ten of them, 623 apart but for the first)
    /// and the walls they were seen on.
    Wall { width: Option<f32> },
    /// A hatch reinforcement.
    Hatch,
    /// A barricade: Castle's when `castle`, on a door when `door`, else on
    /// a window. Which asset is which opening is inferred from where they
    /// were seen.
    Barricade {
        castle: bool,
        door: bool,
        wide: bool,
    },
}

const fn wall(width: f32) -> PanelAsset {
    PanelAsset::Wall { width: Some(width) }
}

const fn barricade(castle: bool, door: bool, wide: bool) -> PanelAsset {
    PanelAsset::Barricade { castle, door, wide }
}

/// Seen in the ten test rounds (Bank) and about 175 real rounds on
/// fifteen maps.
const PANELS: &[(u64, PanelAsset)] = &[
    (406076330074, PanelAsset::Hatch),
    (406076330089, barricade(false, true, true)),
    (406076330090, barricade(false, true, false)),
    (406076330091, barricade(false, false, true)),
    (406076330092, barricade(false, false, false)),
    (361321226204, barricade(true, true, true)),
    (361321226207, barricade(true, true, false)),
    (361321226210, barricade(true, false, false)),
    (361321226213, barricade(true, false, true)),
    // Seen only as placed by the map in Quick Match (Kanal).
    (417911026321, PanelAsset::Wall { width: None }),
    // Not seen measured: one step below the narrowest that was.
    (417911059814, wall(1.5)),
    (417911060317, wall(1.6)),
    (417911060940, wall(1.7)),
    (417911061563, wall(1.8)),
    (417911062186, wall(1.9)),
    (417911062809, wall(2.0)),
    (417911063432, wall(2.1)),
    (417911064055, wall(2.2)),
    (417911064678, wall(2.3)),
    (417911065301, wall(2.4)),
];

/// What the table knows of a panel's asset.
pub(crate) fn panel_asset(asset: u64) -> Option<PanelAsset> {
    PANELS.iter().find(|p| p.0 == asset).map(|p| p.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_are_listed_once() {
        for (i, a) in PANELS.iter().enumerate() {
            assert!(PANELS.iter().skip(i + 1).all(|b| b.0 != a.0), "{}", a.0);
        }
    }

    #[test]
    fn wall_widths_follow_the_asset_ids() {
        let width = |asset| match panel_asset(asset) {
            Some(PanelAsset::Wall { width }) => width,
            _ => None,
        };
        for i in 0..9u8 {
            let asset = 417911060317 + 623 * u64::from(i);
            let expected = 1.6 + 0.1 * f32::from(i);
            let found = width(asset).unwrap_or(f32::NAN);
            assert!((found - expected).abs() < 1e-4, "{asset}: {found}");
        }
        assert_eq!(panel_asset(1), None);
    }
}
