//! What each player wears and carries (Y11S3): uniform, headgear, weapon
//! skins, charms and attachments.
//!
//! The movement stream creates every body and every item with a message
//! listing named slots (see [`crate::entities::Spawn`]). A body's slots hold
//! its uniform, headgear and operator card, and the asset of each weapon and
//! gadget it carries; each of those is created by a message of its own whose
//! slots hold its skin, charm and attachments.
//!
//! Everything is an asset id. Replays hold no names for them, so the same
//! id is the same item, and telling which item needs a table from elsewhere.

use serde::Serialize;

use crate::entities::{Spawn, lists_together};

type Hash = [u8; 4];

// Slot names, as the CRC-32 the stream writes.
const UNIFORM: Hash = [0x2B, 0x40, 0x91, 0x39];
const HEADGEAR: Hash = [0x86, 0xE4, 0xCE, 0xC3];
const CARD_BACKGROUND: Hash = [0x3C, 0x95, 0xEE, 0x4E];
const CARD_PORTRAIT: Hash = [0x77, 0x6C, 0xAD, 0xBA];
/// `PrimaryOperatorCardBadge`, `Secondary...` and `Tertiary...`.
const CARD_BADGES: [Hash; 3] = [
    [0x3C, 0x9E, 0xF1, 0x50],
    [0xB7, 0x26, 0x80, 0x09],
    [0xFA, 0x49, 0x22, 0x22],
];
/// `MVPData`: the animation shown when the player is the match's MVP.
const MVP_DATA: Hash = [0xA7, 0x6C, 0xF1, 0x76];
const WEAPONS: [(Hash, ItemSlot); 2] = [
    ([0x64, 0xDC, 0xCF, 0xC2], ItemSlot::Primary),
    ([0xB4, 0xA5, 0xD8, 0x70], ItemSlot::Secondary),
];
const GADGETS: [(Hash, ItemSlot); 4] = [
    ([0x08, 0x2C, 0xA3, 0x1D], ItemSlot::PrimaryGadget),
    ([0xD8, 0x55, 0xB4, 0xAF], ItemSlot::SecondaryGadget),
    ([0x41, 0x20, 0x14, 0x8B], ItemSlot::TertiaryGadget),
    ([0x60, 0x3E, 0x4A, 0x2F], ItemSlot::Drone),
];
const WEAPON_SKIN: Hash = [0x3D, 0x2D, 0x30, 0x02];
const CHARM: Hash = [0xEE, 0xAD, 0xFC, 0x40];
const ATTACHMENT_SKIN: Hash = [0xEF, 0xDD, 0xE7, 0x33];
const BARREL: Hash = [0x81, 0x32, 0xCD, 0x9D];
const GRIP: Hash = [0x74, 0xA2, 0xC8, 0x94];
const SIGHT: Hash = [0xA0, 0x73, 0x6B, 0x48];
const UNDERBARREL: Hash = [0x28, 0xDD, 0x90, 0x64];
/// A gadget's or drone's `Skin`.
const SKIN: Hash = [0x20, 0xC7, 0x4B, 0xA2];

/// The charm slot's value on an item without a charm. Gadgets, which take
/// no charm, hold it.
const NO_CHARM: u64 = 234_390_650_796;

/// A player's look in one round. Every value is an asset id; a slot left
/// out is empty.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cosmetics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uniform: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headgear: Option<u64>,
    #[serde(skip_serializing_if = "OperatorCard::is_empty")]
    pub operator_card: OperatorCard,
    /// The animation shown when the player is the match's MVP (`MVPData`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mvp_animation: Option<u64>,
    /// The guns carried, primary first. A weapon whose creation message
    /// could not be told from another player's has only its `item`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub weapons: Vec<Weapon>,
    /// Gadgets and drones that carry a skin.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gadgets: Vec<Gadget>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatorCard {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portrait: Option<u64>,
    /// Up to three, in the card's order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<u64>,
}

impl OperatorCard {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemSlot {
    Primary,
    Secondary,
    PrimaryGadget,
    SecondaryGadget,
    TertiaryGadget,
    Drone,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Weapon {
    pub slot: ItemSlot,
    /// The weapon itself, as the body's slot names it.
    pub item: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skin: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charm: Option<u64>,
    /// `WeaponAttachmentSkinSet`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment_skin: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sight: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub barrel: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grip: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underbarrel: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gadget {
    pub slot: ItemSlot,
    pub item: u64,
    pub skin: u64,
}

/// The look of the body created by `body_spawn`. `spawns` are the round's
/// creation messages and `data` the bytes they came from, which settle
/// which of two players holding the same weapon an item belongs to.
pub fn of(body_spawn: &Spawn, spawns: &[Spawn], data: &[u8]) -> Cosmetics {
    // The item a slot of the body names: the one carried item with that
    // asset, or among several the one listed with this body.
    let carried = |asset: u64| -> Option<&Spawn> {
        let mut with = spawns.iter().filter(|s| s.carried && s.asset == asset);
        let first = with.next()?;
        if with.next().is_none() {
            return Some(first);
        }
        spawns
            .iter()
            .filter(|s| s.carried && s.asset == asset)
            .find(|s| lists_together(data, body_spawn.object, s.object))
    };
    let weapons = WEAPONS
        .iter()
        .filter_map(|&(hash, slot)| {
            let item = body_spawn.slot(hash)?;
            let w = carried(item);
            let get = |hash| w.and_then(|w| w.slot(hash));
            Some(Weapon {
                slot,
                item,
                skin: get(WEAPON_SKIN),
                charm: get(CHARM).filter(|&c| c != NO_CHARM),
                attachment_skin: get(ATTACHMENT_SKIN),
                sight: get(SIGHT),
                barrel: get(BARREL),
                grip: get(GRIP),
                underbarrel: get(UNDERBARREL),
            })
        })
        .collect();
    let gadgets = GADGETS
        .iter()
        .filter_map(|&(hash, slot)| {
            let item = body_spawn.slot(hash)?;
            let skin = carried(item)?.slot(SKIN)?;
            Some(Gadget { slot, item, skin })
        })
        .collect();
    Cosmetics {
        uniform: body_spawn.slot(UNIFORM),
        headgear: body_spawn.slot(HEADGEAR),
        operator_card: OperatorCard {
            background: body_spawn.slot(CARD_BACKGROUND),
            portrait: body_spawn.slot(CARD_PORTRAIT),
            badges: (CARD_BADGES.iter())
                .filter_map(|&b| body_spawn.slot(b))
                .collect(),
        },
        mvp_animation: body_spawn.slot(MVP_DATA),
        weapons,
        gadgets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn(object: u32, carried: bool, asset: u64, slots: &[(Hash, u64)]) -> Spawn {
        Spawn {
            object,
            carried,
            asset,
            slots: slots.to_vec(),
        }
    }

    #[test]
    fn a_body_names_its_weapons_and_the_weapons_their_skins() {
        let body = spawn(
            0xF000_0001,
            false,
            10,
            &[
                (UNIFORM, 100),
                (HEADGEAR, 101),
                (CARD_BADGES[0], 0),
                (CARD_BADGES[1], 7),
                (WEAPONS[0].0, 500),
                (WEAPONS[1].0, 0),
                (GADGETS[0].0, 600),
            ],
        );
        let all = [
            body.clone(),
            spawn(
                0xF000_0002,
                true,
                500,
                &[(WEAPON_SKIN, 501), (CHARM, NO_CHARM), (SIGHT, 502)],
            ),
            // A copy that is not carried is not the body's.
            spawn(0xF000_0003, false, 500, &[(WEAPON_SKIN, 999)]),
            spawn(0xF000_0004, true, 600, &[(CHARM, NO_CHARM), (SKIN, 601)]),
        ];
        let c = of(&body, &all, &[]);
        assert_eq!((c.uniform, c.headgear), (Some(100), Some(101)));
        assert_eq!(c.operator_card.badges, [7]);
        assert_eq!(c.weapons.len(), 1, "an empty weapon slot is left out");
        let w = &c.weapons[0];
        assert_eq!(
            (w.slot, w.item, w.skin),
            (ItemSlot::Primary, 500, Some(501))
        );
        assert_eq!(w.charm, None, "the placeholder is no charm");
        assert_eq!(w.sight, Some(502));
        assert_eq!(
            c.gadgets,
            [Gadget {
                slot: ItemSlot::PrimaryGadget,
                item: 600,
                skin: 601
            }]
        );
    }

    #[test]
    fn two_players_with_the_same_weapon_keep_their_own_skins() {
        let body = spawn(0xF000_0001, false, 10, &[(UNIFORM, 1), (WEAPONS[0].0, 500)]);
        let all = [
            spawn(0xF000_0010, true, 500, &[(WEAPON_SKIN, 1)]),
            spawn(0xF000_0011, true, 500, &[(WEAPON_SKIN, 2)]),
        ];
        // The body's list of what it carries names the second.
        let mut data = vec![];
        for id in [0xF000_0001u32, 0xF000_0011] {
            data.extend(u64::from(id).to_le_bytes());
        }
        assert_eq!(of(&body, &all, &data).weapons[0].skin, Some(2));
        // Without it the skin is left out rather than guessed.
        let w = &of(&body, &all, &[]).weapons[0];
        assert_eq!((w.item, w.skin), (500, None));
    }
}
