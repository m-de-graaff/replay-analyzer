//! The table behind [`super::all`]. Built from [`super::harvest`] over
//! 188 rounds (the observed side: `Stat::observed`, `Stat::both`,
//! `Stat::confirmed` and every `seen` attachment) and from the reference
//! table of r6data.com as of 2026-10-02 with the Y11S3 patch notes over it
//! (`Stat::reference`, the classes and every `reference` list). The module
//! documentation of [`super`] says what each side rests on.
//!
//! The reference's own slips are kept as it gives them, beside what was
//! measured: 1800 rounds a minute for the Scorpion EVO 3 A1, 900 for the
//! MP5K.

use super::FireMode::{self, BoltAction, FullAuto, PumpAction, SemiAuto};
use super::{AttachmentOption, AttachmentSlot, Slot, Stat, WeaponClass, WeaponInfo};
use crate::types::ItemKind;

const FULL: &[FireMode] = &[FullAuto];
const SEMI: &[FireMode] = &[SemiAuto];
const PUMP: &[FireMode] = &[PumpAction];
const BOLT: &[FireMode] = &[BoltAction];
const NONE: &[FireMode] = &[];

/// A name the attachment table read from the file.
const fn named(id: u64, name: &'static str, samples: u32) -> AttachmentOption {
    AttachmentOption {
        id,
        name: Some(name),
        inferred: false,
        samples,
    }
}

/// A name the attachment table inferred.
const fn inferred(id: u64, name: &'static str, samples: u32) -> AttachmentOption {
    AttachmentOption {
        id,
        name: Some(name),
        inferred: true,
        samples,
    }
}

const fn unnamed(id: u64, samples: u32) -> AttachmentOption {
    AttachmentOption {
        id,
        name: None,
        inferred: false,
        samples,
    }
}

/// A set of sights the reference lists for several guns.
const SIGHTS_1X: &[&str] = &[
    "Red Dot A",
    "Red Dot B",
    "Red Dot C",
    "Holo A",
    "Holo B",
    "Holo C",
    "Holo D",
    "Reflex A",
    "Reflex B",
    "Reflex C",
    "Iron Sight",
];

/// A set of sights the reference lists for several guns.
const SIGHTS_MAGNIFIED: &[&str] = &[
    "Magnified A",
    "Magnified B",
    "Magnified C",
    "Red Dot A",
    "Red Dot B",
    "Red Dot C",
    "Holo A",
    "Holo B",
    "Holo C",
    "Holo D",
    "Reflex A",
    "Reflex B",
    "Reflex C",
    "Iron Sight",
];

/// A set of sights the reference lists for several guns.
const SIGHTS_TELESCOPIC: &[&str] = &[
    "Telescopic A",
    "Telescopic B",
    "Magnified A",
    "Magnified B",
    "Magnified C",
    "Red Dot A",
    "Red Dot B",
    "Red Dot C",
    "Holo A",
    "Holo B",
    "Holo C",
    "Holo D",
    "Reflex A",
    "Reflex B",
    "Reflex C",
    "Iron Sight",
];

/// A set of sights the reference lists for several guns.
const SIGHTS_DP27: &[&str] = &[
    "Red Dot A",
    "Red Dot B",
    "Red Dot C",
    "Holo A",
    "Holo B",
    "Holo C",
    "Holo D",
    "Reflex A",
    "Reflex B",
    "Reflex C",
    "Reflex D",
    "Iron Sight",
];

#[rustfmt::skip]
pub(super) const WEAPONS: &[WeaponInfo] = &[
    WeaponInfo {
        id: Some(1366019208),
        name: "416-C Carbine",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(38, 27, 38),
        rpm: Stat::confirmed(740, 733, 45),
        magazine: Stat::both(25, 29, 25),
        chambered: Stat::observed(true, 28),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373619980, 20),
                    unnamed(367839568301, 4),
                    unnamed(367839568336, 4),
                    unnamed(367839568311, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373619984, "Flash Hider", 20),
                    inferred(238373619985, "Compensator", 6),
                    inferred(238373619986, "Muzzle Brake", 2),
                    inferred(238373619987, "Extended Barrel", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373619989, "Vertical Grip", 28),
                    unnamed(238373619988, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373619990, "None", 19),
                    named(238373619991, "Laser", 10),
                ],
                reference: &[],
            },
        ],
        carried: 29,
    },
    WeaponInfo {
        id: Some(1366019220),
        name: "AK-12",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(40, 27, 40),
        rpm: Stat::confirmed(850, 848, 33),
        magazine: Stat::both(30, 51, 30),
        chambered: Stat::observed(true, 48),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(367839609818, "Magnified 2.5x", 47),
                    unnamed(384304323768, 4),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620187, "Flash Hider", 43),
                    inferred(238373620188, "Compensator", 8),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373620191, "Vertical Grip", 49),
                    unnamed(238373620190, 2),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620193, "None", 29),
                    named(238373620194, "Laser", 22),
                ],
                reference: &[],
            },
        ],
        carried: 51,
    },
    WeaponInfo {
        id: Some(1366019232),
        name: "AUG A2",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(42, 4, 42),
        rpm: Stat::confirmed(720, 716, 8),
        magazine: Stat::both(30, 3, 30),
        chambered: Stat::observed(true, 3),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373620331, 2),
                    unnamed(367839576407, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620336, "Flash Hider", 2),
                    inferred(238373620335, "Suppressor", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373620338, "None", 3),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620339, "None", 2),
                    named(238373620340, "Laser", 1),
                ],
                reference: &[],
            },
        ],
        carried: 3,
    },
    WeaponInfo {
        id: Some(1366019244),
        name: "F2",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(37, 71, 37),
        rpm: Stat::confirmed(980, 978, 130),
        magazine: Stat::both(25, 74, 25),
        chambered: Stat::observed(true, 67),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373620607, "Magnified 2.5x", 62),
                    unnamed(238373620605, 10),
                    unnamed(287652917825, 1),
                    unnamed(367839576893, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620610, "Flash Hider", 68),
                    inferred(238373620611, "Compensator", 3),
                    inferred(238373620612, "Muzzle Brake", 2),
                    inferred(238373620609, "Suppressor", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(441481271391, 73),
                    unnamed(441481337697, 1),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620616, "Laser", 43),
                    named(238373620615, "None", 31),
                ],
                reference: &[],
            },
        ],
        carried: 74,
    },
    WeaponInfo {
        id: Some(1366019268),
        name: "AR33",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::reference(41),
        rpm: Stat::confirmed(749, 744, 13),
        magazine: Stat::both(25, 16, 25),
        chambered: Stat::observed(true, 14),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373620284, "Magnified 2.5x", 11),
                    unnamed(287652917795, 3),
                    unnamed(238373620282, 1),
                    unnamed(367839576284, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620287, "Flash Hider", 11),
                    inferred(238373620288, "Compensator", 3),
                    inferred(238373620286, "Suppressor", 2),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373620291, "Vertical Grip", 15),
                    unnamed(238373620292, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620293, "None", 11),
                    named(238373620294, "Laser", 5),
                ],
                reference: &[],
            },
        ],
        carried: 16,
    },
    WeaponInfo {
        id: Some(1366019292),
        name: "417",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::MarksmanRifle),
        damage: Stat::both(69, 23, 69),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(20, 35, 20),
        chambered: Stat::observed(true, 35),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(359470844725, "Magnified 2.5x", 25),
                    unnamed(359470844719, 6),
                    unnamed(287652916030, 3),
                    unnamed(359470844731, 1),
                ],
                reference: SIGHTS_TELESCOPIC,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620012, "Muzzle Brake", 35),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373620014, "Vertical Grip", 32),
                    unnamed(386201549676, 3),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620015, "None", 20),
                    named(238373620016, "Laser", 15),
                ],
                reference: &[],
            },
        ],
        carried: 35,
    },
    WeaponInfo {
        id: Some(1366019304),
        name: "L85A2",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(47, 19, 47),
        rpm: Stat::confirmed(670, 667, 34),
        magazine: Stat::both(30, 31, 30),
        chambered: Stat::observed(true, 31),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373620875, "Magnified 2.5x", 27),
                    unnamed(287652917849, 3),
                    unnamed(238373620873, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620878, "Flash Hider", 17),
                    inferred(238373620879, "Compensator", 11),
                    inferred(238373620877, "Suppressor", 3),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373620882, "Vertical Grip", 30),
                    unnamed(238373620881, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620883, "None", 26),
                    named(238373620884, "Laser", 5),
                ],
                reference: &[],
            },
        ],
        carried: 31,
    },
    WeaponInfo {
        id: Some(1366019316),
        name: "R4-C",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(39, 49, 39),
        rpm: Stat::confirmed(860, 858, 43),
        magazine: Stat::both(25, 54, 25),
        chambered: Stat::observed(true, 51),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(403332439097, "Magnified 2.5x", 48),
                    unnamed(238373621653, 4),
                    unnamed(367839578690, 1),
                    unnamed(396066531378, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621657, "Flash Hider", 40),
                    inferred(238373621658, "Compensator", 8),
                    inferred(238373621659, "Muzzle Brake", 3),
                    inferred(238373621656, "Suppressor", 2),
                    inferred(238373621660, "Extended Barrel", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373621662, "Vertical Grip", 54),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621664, "Laser", 30),
                    named(238373621663, "None", 24),
                ],
                reference: &[],
            },
        ],
        carried: 55,
    },
    WeaponInfo {
        id: Some(1366019328),
        name: "552 Commando",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(43, 39, 43),
        rpm: Stat::confirmed(690, 689, 82),
        magazine: Stat::both(30, 60, 30),
        chambered: Stat::observed(true, 59),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373620052, "Magnified 2.5x", 56),
                    unnamed(238373620050, 3),
                    unnamed(293876186512, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620055, "Flash Hider", 48),
                    inferred(381580271012, "Extended Barrel", 4),
                    inferred(238373620054, "Suppressor", 3),
                    inferred(238373620056, "Compensator", 3),
                    inferred(238373620057, "Muzzle Brake", 2),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373620059, "Vertical Grip", 60),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620061, "None", 43),
                    named(238373620062, "Laser", 17),
                ],
                reference: &[],
            },
        ],
        carried: 60,
    },
    WeaponInfo {
        id: Some(1366019340),
        name: "556xi",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(47, 46, 47),
        rpm: Stat::confirmed(690, 689, 99),
        magazine: Stat::both(30, 69, 30),
        chambered: Stat::observed(true, 69),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(258614296872, "Magnified 2.5x", 62),
                    unnamed(384304323145, 4),
                    unnamed(258614296876, 3),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(258614296896, "Flash Hider", 42),
                    inferred(258614296892, "Compensator", 21),
                    inferred(258614296900, "Muzzle Brake", 6),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(258614296920, "Vertical Grip", 65),
                    inferred(258614296912, "Angled Grip", 3),
                    unnamed(258614296916, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(258614296868, "None", 53),
                    named(258614296864, "Laser", 16),
                ],
                reference: &[],
            },
        ],
        carried: 69,
    },
    WeaponInfo {
        id: Some(1366019352),
        name: "G8A1",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::LightMachineGun),
        damage: Stat::both(37, 5, 37),
        rpm: Stat::confirmed(850, 847, 4),
        magazine: Stat::both(50, 4, 50),
        chambered: Stat::observed(true, 4),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(287652917843, 2),
                    unnamed(238373620738, 1),
                    inferred(238373620740, "Magnified 2.5x", 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620742, "Suppressor", 2),
                    inferred(238373620743, "Flash Hider", 2),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(238373620746, 4),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620748, "None", 3),
                    named(381724020404, "Laser", 1),
                ],
                reference: &[],
            },
        ],
        carried: 4,
    },
    WeaponInfo {
        id: Some(1366019364),
        name: "6P41",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::LightMachineGun),
        damage: Stat::both(46, 5, 46),
        rpm: Stat::confirmed(680, 673, 23),
        magazine: Stat::both(100, 8, 100),
        chambered: Stat::observed(false, 8),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(287652917771, 5),
                    inferred(367839609805, "Magnified 2.5x", 3),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620094, "Flash Hider", 8),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373620096, "Vertical Grip", 8),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620098, "Laser", 5),
                    named(238373620097, "None", 3),
                ],
                reference: &[],
            },
        ],
        carried: 8,
    },
    WeaponInfo {
        id: Some(1366019508),
        name: "M590A1",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(48),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(7, 99, 7),
        chambered: Stat::observed(false, 93),
        fire_modes: PUMP,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373621098, "Iron Sights", 57),
                    unnamed(238373621100, 21),
                    unnamed(238373621101, 17),
                    unnamed(367839577790, 3),
                    unnamed(238373621099, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621102, "None", 99),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621103, "None", 99),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621105, "Laser", 60),
                    named(238373621104, "None", 39),
                ],
                reference: &[],
            },
        ],
        carried: 99,
    },
    WeaponInfo {
        id: Some(1366019520),
        name: "M1014",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(28),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(8, 5, 8),
        chambered: Stat::observed(false, 5),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373620968, 2),
                    unnamed(384951456354, 2),
                    unnamed(238373620966, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620969, "None", 5),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373620970, "None", 5),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620972, "Laser", 3),
                    named(238373620971, "None", 2),
                ],
                reference: &[],
            },
        ],
        carried: 5,
    },
    WeaponInfo {
        id: Some(1366019532),
        name: "M870",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(42),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(7, 4, 7),
        chambered: Stat::observed(false, 4),
        fire_modes: PUMP,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621146, 2),
                    unnamed(238373621147, 2),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621148, "None", 4),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621149, "None", 4),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621151, "Laser", 3),
                    unnamed(238373621150, 1),
                ],
                reference: &[],
            },
        ],
        carried: 4,
    },
    WeaponInfo {
        id: Some(1366019556),
        name: "SG-CQB",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(44),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(7, 1, 7),
        chambered: Stat::observed(false, 1),
        fire_modes: PUMP,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621758, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621762, "None", 1),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(238373621764, 1),
                ],
                reference: &["Vertical Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621766, "Laser", 1),
                ],
                reference: &[],
            },
        ],
        carried: 1,
    },
    WeaponInfo {
        id: Some(1366019592),
        name: "MP5K",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(30, 55, 30),
        rpm: Stat::both(797, 54, 900),
        magazine: Stat::both(30, 42, 30),
        chambered: Stat::observed(true, 38),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621235, 41),
                    unnamed(367839578145, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621239, "Flash Hider", 36),
                    inferred(238373621240, "Compensator", 6),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621242, "None", 42),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621243, "None", 28),
                    named(238373621244, "Laser", 14),
                ],
                reference: &[],
            },
        ],
        carried: 42,
    },
    WeaponInfo {
        id: Some(1366019604),
        name: "MP5",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(27, 37, 27),
        rpm: Stat::confirmed(800, 809, 27),
        magazine: Stat::both(30, 37, 30),
        chambered: Stat::observed(true, 33),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621211, 32),
                    unnamed(367839578090, 2),
                    unnamed(367839578094, 2),
                    unnamed(238373621212, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621216, "Flash Hider", 31),
                    inferred(238373621215, "Suppressor", 3),
                    inferred(383859036508, "Extended Barrel", 3),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373621219, "Vertical Grip", 36),
                    unnamed(238373621218, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621220, "None", 24),
                    named(238373621221, "Laser", 13),
                ],
                reference: &[],
            },
        ],
        carried: 37,
    },
    WeaponInfo {
        id: Some(1366019616),
        name: "MP7",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(32, 74, 32),
        rpm: Stat::confirmed(900, 899, 122),
        magazine: Stat::both(30, 93, 30),
        chambered: Stat::observed(true, 85),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(258614298894, 40),
                    inferred(238373621279, "Iron Sights", 24),
                    unnamed(367839578265, 13),
                    unnamed(258614298896, 6),
                    unnamed(367839578266, 3),
                    inferred(403332451571, "Magnified 2.5x", 3),
                    unnamed(367839578273, 2),
                    unnamed(258614298895, 1),
                    unnamed(367839578267, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(258614298875, "Flash Hider", 57),
                    inferred(258614298874, "Compensator", 15),
                    inferred(383859036493, "Extended Barrel", 14),
                    inferred(258614298877, "Suppressor", 6),
                    inferred(258614298876, "Muzzle Brake", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621281, "None", 93),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621282, "None", 68),
                    named(258614298890, "Laser", 25),
                ],
                reference: &[],
            },
        ],
        carried: 93,
    },
    WeaponInfo {
        id: Some(1366019628),
        name: "P90",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(22, 16, 22),
        rpm: Stat::confirmed(970, 969, 37),
        magazine: Stat::both(50, 26, 50),
        chambered: Stat::observed(true, 25),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621506, 25),
                    unnamed(367839578510, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621510, "Suppressor", 10),
                    inferred(238373621511, "Flash Hider", 9),
                    inferred(384304379591, "Compensator", 4),
                    inferred(238373621513, "Extended Barrel", 3),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621514, "None", 26),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621516, "Laser", 18),
                    named(238373621515, "None", 8),
                ],
                reference: &[],
            },
        ],
        carried: 26,
    },
    WeaponInfo {
        id: Some(1366019640),
        name: "UMP45",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(42, 17, 42),
        rpm: Stat::confirmed(600, 599, 34),
        magazine: Stat::both(25, 27, 25),
        chambered: Stat::observed(true, 24),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373622140, 21),
                    unnamed(238373622139, 2),
                    unnamed(238373622141, 2),
                    unnamed(367839579704, 2),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373622144, "Flash Hider", 9),
                    inferred(238373622145, "Compensator", 8),
                    inferred(238373622143, "Suppressor", 5),
                    inferred(238373622147, "Extended Barrel", 5),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373622149, "Vertical Grip", 22),
                    unnamed(238373622148, 4),
                    inferred(238373622150, "Angled Grip", 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373622151, "None", 15),
                    named(238373622152, "Laser", 12),
                ],
                reference: &[],
            },
        ],
        carried: 27,
    },
    WeaponInfo {
        id: Some(1366019652),
        name: "9x19VSN",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(34, 56, 34),
        rpm: Stat::confirmed(750, 745, 103),
        magazine: Stat::both(30, 63, 30),
        chambered: Stat::observed(true, 61),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(378015302692, 60),
                    unnamed(238373620134, 1),
                    unnamed(367839575925, 1),
                    unnamed(367839575927, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620139, "Flash Hider", 51),
                    inferred(238373620140, "Compensator", 6),
                    inferred(381580270192, "Extended Barrel", 5),
                    inferred(238373620138, "Suppressor", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373620143, "Vertical Grip", 62),
                    unnamed(238373620142, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620145, "None", 33),
                    named(238373620146, "Laser", 30),
                ],
                reference: &[],
            },
        ],
        carried: 63,
    },
    WeaponInfo {
        id: Some(1366019988),
        name: "FMG-9",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(34, 4, 34),
        rpm: Stat::confirmed(800, 780, 3),
        magazine: Stat::both(30, 7, 30),
        chambered: Stat::observed(true, 7),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(287652917831, 2),
                    unnamed(367839577014, 2),
                    inferred(403332439478, "Magnified 2.5x", 2),
                    unnamed(367839577003, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620659, "Flash Hider", 5),
                    inferred(238373620658, "Suppressor", 2),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373620661, "None", 7),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620662, "None", 4),
                    named(238373620663, "Laser", 3),
                ],
                reference: &[],
            },
        ],
        carried: 7,
    },
    WeaponInfo {
        id: Some(9817293379),
        name: "OTs-03",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SniperRifle),
        damage: Stat::both(71, 5, 71),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(15, 14, 15),
        chambered: Stat::observed(true, 12),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(379128554145, 7),
                    unnamed(379128554149, 6),
                    unnamed(379128554148, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621380, "Muzzle Brake", 10),
                    inferred(238373621378, "Suppressor", 4),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(387791442381, 13),
                    unnamed(387791442386, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621382, "None", 12),
                    named(381724020413, "Laser", 2),
                ],
                reference: &[],
            },
        ],
        carried: 14,
    },
    WeaponInfo {
        id: Some(10929932395),
        name: "MK17 CQB",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(44, 6, 44),
        rpm: Stat::confirmed(585, 585, 11),
        magazine: Stat::both(20, 16, 25),
        chambered: Stat::observed(true, 15),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373621181, "Iron Sights", 6),
                    inferred(238373621185, "Magnified 2.5x", 6),
                    unnamed(238373621183, 4),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621188, "Flash Hider", 9),
                    inferred(238373621191, "Extended Barrel", 5),
                    inferred(238373621189, "Compensator", 2),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373621193, "Vertical Grip", 6),
                    unnamed(238373621192, 5),
                    inferred(238373621194, "Angled Grip", 5),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621196, "Laser", 9),
                    named(238373621195, "None", 7),
                ],
                reference: &[],
            },
        ],
        carried: 16,
    },
    WeaponInfo {
        id: Some(13333481354),
        name: "9mm C1",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(36, 11, 36),
        rpm: Stat::confirmed(575, 574, 6),
        magazine: Stat::both(34, 14, 34),
        chambered: Stat::observed(true, 13),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(367839575863, 13),
                    unnamed(271767745117, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620115, "Suppressor", 4),
                    inferred(384304379596, "Compensator", 4),
                    inferred(238373620116, "Extended Barrel", 3),
                    inferred(384304385248, "Flash Hider", 3),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373620118, "Angled Grip", 10),
                    unnamed(386201549741, 4),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620119, "None", 10),
                    named(238373620120, "Laser", 4),
                ],
                reference: &[],
            },
        ],
        carried: 14,
    },
    WeaponInfo {
        id: Some(13333481726),
        name: "C8-SFW",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(40, 23, 40),
        rpm: Stat::confirmed(837, 833, 20),
        magazine: Stat::both(30, 27, 30),
        chambered: Stat::observed(true, 25),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373620502, "Magnified 2.5x", 24),
                    unnamed(287652917819, 3),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620505, "Flash Hider", 15),
                    inferred(238373620506, "Compensator", 7),
                    inferred(238373620508, "Extended Barrel", 3),
                    inferred(238373620504, "Suppressor", 2),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(238373620509, 27),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620510, "None", 17),
                    named(238373620511, "Laser", 10),
                ],
                reference: &[],
            },
        ],
        carried: 28,
    },
    WeaponInfo {
        id: Some(13333481738),
        name: "SPAS-12",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(31),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(7, 1, 7),
        chambered: Stat::observed(false, 1),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621868, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621872, "None", 1),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621873, "None", 1),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    unnamed(238373621875, 1),
                ],
                reference: &[],
            },
        ],
        carried: 1,
    },
    WeaponInfo {
        id: Some(13333481762),
        name: "SR-25",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::MarksmanRifle),
        damage: Stat::both(61, 3, 61),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(20, 6, 20),
        chambered: Stat::observed(true, 6),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373621953, "Magnified 2.5x", 4),
                    unnamed(238373621951, 1),
                    unnamed(359470845360, 1),
                ],
                reference: SIGHTS_TELESCOPIC,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621958, "Muzzle Brake", 6),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373621960, "Vertical Grip", 6),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621961, "None", 3),
                    named(238373621962, "Laser", 3),
                ],
                reference: &[],
            },
        ],
        carried: 6,
    },
    WeaponInfo {
        id: Some(34160268747),
        name: "MPX",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(26, 56, 26),
        rpm: Stat::confirmed(830, 821, 41),
        magazine: Stat::both(30, 67, 30),
        chambered: Stat::observed(true, 63),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621297, 52),
                    unnamed(367839578325, 11),
                    unnamed(367839578334, 2),
                    unnamed(238373621296, 1),
                    unnamed(367839578330, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621301, "Flash Hider", 43),
                    inferred(381580276224, "Extended Barrel", 17),
                    inferred(238373621300, "Suppressor", 3),
                    inferred(238373621302, "Compensator", 3),
                    inferred(238373621303, "Muzzle Brake", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373621305, "Vertical Grip", 66),
                    unnamed(238373621304, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621307, "None", 44),
                    named(238373621308, "Laser", 23),
                ],
                reference: &[],
            },
        ],
        carried: 67,
    },
    WeaponInfo {
        id: Some(38581443640),
        name: "PARA-308",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(47, 20, 47),
        rpm: Stat::confirmed(650, 645, 67),
        magazine: Stat::both(30, 46, 30),
        chambered: Stat::observed(true, 45),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373621542, "Magnified 2.5x", 40),
                    unnamed(287652917915, 5),
                    unnamed(238373621540, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621544, "Suppressor", 24),
                    inferred(238373621545, "Flash Hider", 16),
                    inferred(238373621546, "Compensator", 4),
                    inferred(238373621548, "Extended Barrel", 2),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373621550, "Vertical Grip", 46),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621552, "None", 36),
                    named(238373621553, "Laser", 10),
                ],
                reference: &[],
            },
        ],
        carried: 46,
    },
    WeaponInfo {
        id: Some(38581443658),
        name: "M249",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::LightMachineGun),
        damage: Stat::reference(48),
        rpm: Stat::reference(650),
        magazine: Stat::both(100, 4, 100),
        chambered: Stat::observed(false, 3),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(319015688001, "Magnified 2.5x", 2),
                    unnamed(238373621032, 1),
                    unnamed(287652917861, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621036, "Flash Hider", 2),
                    inferred(238373621037, "Compensator", 2),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373621039, "Vertical Grip", 4),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621040, "None", 3),
                    named(238373621041, "Laser", 1),
                ],
                reference: &[],
            },
        ],
        carried: 4,
    },
    WeaponInfo {
        id: Some(38581443694),
        name: "SPAS-15",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(24),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(6, 6, 6),
        chambered: Stat::observed(true, 6),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621887, 3),
                    unnamed(238373621890, 1),
                    unnamed(379128554176, 1),
                    unnamed(379128554181, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621891, "None", 6),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621892, "None", 6),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621893, "None", 3),
                    named(238373621894, "Laser", 3),
                ],
                reference: &[],
            },
        ],
        carried: 6,
    },
    WeaponInfo {
        id: Some(38581443712),
        name: "M12",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::reference(40),
        rpm: Stat::reference(550),
        magazine: Stat::both(30, 6, 30),
        chambered: Stat::observed(true, 3),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(367839577543, 4),
                    unnamed(367839577550, 1),
                    unnamed(367839577554, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620988, "Suppressor", 6),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373620992, "None", 6),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620993, "None", 4),
                    named(238373620994, "Laser", 2),
                ],
                reference: &[],
            },
        ],
        carried: 6,
    },
    WeaponInfo {
        id: Some(39149214650),
        name: "MP5SD",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::reference(30),
        rpm: Stat::reference(800),
        magazine: Stat::both(30, 2, 30),
        chambered: Stat::observed(true, 2),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621259, 1),
                    unnamed(367839578207, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621261, "None", 2),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373621263, "Vertical Grip", 2),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621265, "None", 1),
                    named(238373621266, "Laser", 1),
                ],
                reference: &[],
            },
        ],
        carried: 2,
    },
    WeaponInfo {
        id: Some(39149214686),
        name: "Supernova",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(48),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(7, 9, 7),
        chambered: Stat::observed(false, 8),
        fire_modes: PUMP,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373622015, "Iron Sights", 8),
                    unnamed(238373622018, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373622020, "Suppressor", 8),
                    unnamed(238373622019, 1),
                ],
                reference: &["Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373622021, "None", 9),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373622023, "Laser", 7),
                    named(238373622022, "None", 2),
                ],
                reference: &[],
            },
        ],
        carried: 9,
    },
    WeaponInfo {
        id: Some(39149214704),
        name: "Type-89",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(40, 11, 40),
        rpm: Stat::confirmed(850, 853, 15),
        magazine: Stat::both(20, 18, 20),
        chambered: Stat::observed(true, 16),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373622116, "Magnified 2.5x", 16),
                    unnamed(287652917939, 2),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373622119, "Flash Hider", 14),
                    inferred(238373622120, "Compensator", 2),
                    inferred(238373622118, "Suppressor", 1),
                    unnamed(238373622121, 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373622123, "Vertical Grip", 17),
                    inferred(238373622124, "Angled Grip", 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373622125, "None", 10),
                    named(238373622126, "Laser", 8),
                ],
                reference: &[],
            },
        ],
        carried: 18,
    },
    WeaponInfo {
        id: Some(39149216458),
        name: "C7E",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(42, 4, 42),
        rpm: Stat::reference(800),
        magazine: Stat::both(30, 9, 30),
        chambered: Stat::observed(true, 8),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(403332439342, "Magnified 2.5x", 7),
                    unnamed(384304323107, 2),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620478, "Flash Hider", 9),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373620482, "Vertical Grip", 7),
                    unnamed(238373620481, 2),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620484, "None", 8),
                    inferred(238373620485, "Laser", 1),
                ],
                reference: &[],
            },
        ],
        carried: 9,
    },
    WeaponInfo {
        id: Some(44316587774),
        name: "ITA12L",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(41),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(8, 5, 8),
        chambered: Stat::observed(false, 5),
        fire_modes: PUMP,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373620782, 3),
                    unnamed(238373620784, 1),
                    unnamed(238373620785, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620786, "None", 5),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373620787, "None", 5),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620788, "None", 3),
                    named(238373620789, "Laser", 2),
                ],
                reference: &[],
            },
        ],
        carried: 5,
    },
    WeaponInfo {
        id: Some(44316587819),
        name: "Vector .45 ACP",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(23, 12, 23),
        rpm: Stat::confirmed(1200, 1202, 24),
        magazine: Stat::both(25, 17, 25),
        chambered: Stat::observed(true, 15),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373622210, 16),
                    unnamed(367839579884, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373622214, "Flash Hider", 10),
                    inferred(238373622215, "Compensator", 4),
                    inferred(238373622217, "Extended Barrel", 2),
                    inferred(238373622213, "Suppressor", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373622219, "Vertical Grip", 17),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373622220, "None", 9),
                    named(238373622221, "Laser", 8),
                ],
                reference: &[],
            },
        ],
        carried: 17,
    },
    WeaponInfo {
        id: Some(44316587837),
        name: "PDW9",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(34, 4, 34),
        rpm: Stat::confirmed(800, 797, 7),
        magazine: Stat::both(50, 5, 50),
        chambered: Stat::observed(true, 5),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(287652917921, 4),
                    inferred(238373621569, "Magnified 2.5x", 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621572, "Flash Hider", 4),
                    inferred(238373621573, "Compensator", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373621576, "Vertical Grip", 3),
                    inferred(238373621577, "Angled Grip", 2),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621578, "None", 5),
                ],
                reference: &[],
            },
        ],
        carried: 5,
    },
    WeaponInfo {
        id: Some(53995319813),
        name: "T-95 LSW",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::LightMachineGun),
        damage: Stat::both(46, 21, 46),
        rpm: Stat::confirmed(650, 648, 51),
        magazine: Stat::both(80, 28, 80),
        chambered: Stat::observed(true, 28),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373622062, "Magnified 2.5x", 27),
                    unnamed(287652917933, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373622065, "Flash Hider", 18),
                    inferred(238373622066, "Compensator", 9),
                    inferred(238373622064, "Suppressor", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373622069, "Vertical Grip", 27),
                    unnamed(238373622068, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373622071, "None", 22),
                    named(238373622072, "Laser", 6),
                ],
                reference: &[],
            },
        ],
        carried: 28,
    },
    WeaponInfo {
        id: Some(53995319846),
        name: "SIX12",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(46),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(6, 1, 6),
        chambered: Stat::observed(false, 1),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621801, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621802, "None", 1),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(238373621803, 1),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621805, "Laser", 1),
                ],
                reference: &[],
            },
        ],
        carried: 1,
    },
    WeaponInfo {
        id: Some(53995319912),
        name: "T-5 SMG",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(28, 40, 28),
        rpm: Stat::confirmed(900, 896, 52),
        magazine: Stat::both(30, 45, 30),
        chambered: Stat::observed(true, 41),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373622037, 42),
                    unnamed(238373622038, 1),
                    unnamed(367839579473, 1),
                    unnamed(367839579474, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373622041, "Flash Hider", 26),
                    inferred(238373622042, "Compensator", 15),
                    inferred(238373622043, "Muzzle Brake", 2),
                    unnamed(381580276276, 2),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(386201549746, 39),
                    unnamed(386201549731, 4),
                    unnamed(238373622044, 2),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373622045, "None", 34),
                    named(238373622046, "Laser", 11),
                ],
                reference: &[],
            },
        ],
        carried: 45,
    },
    WeaponInfo {
        id: Some(55586802030),
        name: "V308",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(44, 7, 44),
        rpm: Stat::confirmed(700, 692, 16),
        magazine: Stat::both(50, 12, 50),
        chambered: Stat::observed(true, 12),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373622186, "Magnified 2.5x", 8),
                    unnamed(238373622184, 2),
                    unnamed(367839579826, 1),
                    unnamed(384304323120, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373622189, "Flash Hider", 11),
                    inferred(238373622188, "Suppressor", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373622193, "Vertical Grip", 12),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373622195, "None", 8),
                    named(238373622196, "Laser", 4),
                ],
                reference: &[],
            },
        ],
        carried: 12,
    },
    WeaponInfo {
        id: Some(55586802095),
        name: "ACS12",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SlugShotgun),
        damage: Stat::reference(69),
        rpm: Stat::reference(300),
        magazine: Stat::both(30, 9, 30),
        chambered: Stat::observed(true, 7),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(403332439808, "Magnified 2.5x", 9),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620163, "None", 9),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373620165, "Vertical Grip", 8),
                    unnamed(238373620164, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620167, "None", 6),
                    named(238373620168, "Laser", 3),
                ],
                reference: &[],
            },
        ],
        carried: 9,
    },
    WeaponInfo {
        id: Some(55586802503),
        name: "Spear .308",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(42, 10, 42),
        rpm: Stat::confirmed(700, 698, 32),
        magazine: Stat::both(30, 22, 30),
        chambered: Stat::observed(true, 21),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(403332439192, "Magnified 2.5x", 11),
                    unnamed(357640400722, 7),
                    unnamed(385609920224, 2),
                    unnamed(238373621909, 1),
                    unnamed(367839579224, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621913, "Flash Hider", 14),
                    inferred(238373621912, "Suppressor", 4),
                    inferred(238373621914, "Compensator", 4),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373621917, "Vertical Grip", 22),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621918, "None", 20),
                    named(238373621919, "Laser", 2),
                ],
                reference: &[],
            },
        ],
        carried: 22,
    },
    WeaponInfo {
        id: Some(68967567352),
        name: "M762",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(45, 40, 45),
        rpm: Stat::confirmed(730, 727, 75),
        magazine: Stat::both(30, 56, 30),
        chambered: Stat::observed(true, 54),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373621121, "Magnified 2.5x", 49),
                    unnamed(385609918710, 5),
                    unnamed(238373621119, 2),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621124, "Flash Hider", 40),
                    inferred(238373621125, "Compensator", 15),
                    inferred(238373621123, "Suppressor", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373621128, "Vertical Grip", 55),
                    unnamed(238373621127, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621130, "None", 32),
                    named(238373621131, "Laser", 24),
                ],
                reference: &[],
            },
        ],
        carried: 57,
    },
    WeaponInfo {
        id: Some(68967567391),
        name: "LMG-E",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::LightMachineGun),
        damage: Stat::reference(41),
        rpm: Stat::reference(720),
        magazine: Stat::reference(150),
        chambered: Stat::UNKNOWN,
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
        ],
        carried: 0,
    },
    WeaponInfo {
        id: Some(68967567469),
        name: "FO-12",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(26),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(10, 3, 10),
        chambered: Stat::observed(true, 3),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373620678, 3),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620682, "Extended Barrel", 3),
                ],
                reference: &["Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(238373620683, 3),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620686, "None", 3),
                ],
                reference: &[],
            },
        ],
        carried: 3,
    },
    WeaponInfo {
        id: Some(68967567508),
        name: "Scorpion EVO 3 A1",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(23, 58, 23),
        rpm: Stat::both(1076, 99, 1800),
        magazine: Stat::both(40, 52, 40),
        chambered: Stat::observed(true, 48),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621719, 38),
                    unnamed(367839578864, 7),
                    unnamed(238373621718, 3),
                    unnamed(367839578867, 2),
                    unnamed(238373621720, 1),
                    unnamed(367839578866, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621723, "Flash Hider", 36),
                    inferred(238373621724, "Compensator", 14),
                    inferred(238373621722, "Suppressor", 1),
                    inferred(238373621725, "Muzzle Brake", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373621727, "Vertical Grip", 52),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621729, "None", 33),
                    named(238373621730, "Laser", 19),
                ],
                reference: &[],
            },
        ],
        carried: 52,
    },
    WeaponInfo {
        id: Some(78526932091),
        name: "Mk 14 EBR",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::MarksmanRifle),
        damage: Stat::both(56, 5, 60),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(20, 11, 20),
        chambered: Stat::observed(true, 9),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373621324, "Magnified 2.5x", 6),
                    unnamed(293876252584, 3),
                    unnamed(238373621321, 1),
                    unnamed(287652916048, 1),
                ],
                reference: SIGHTS_TELESCOPIC,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621329, "Muzzle Brake", 6),
                    inferred(238373621325, "None", 3),
                    inferred(238373621326, "Suppressor", 2),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373621331, "Vertical Grip", 11),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621334, "Laser", 6),
                    named(238373621333, "None", 5),
                ],
                reference: &[],
            },
        ],
        carried: 11,
    },
    WeaponInfo {
        id: Some(78526932208),
        name: "BOSG.12.2",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SlugShotgun),
        damage: Stat::reference(125),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(2, 19, 2),
        chambered: Stat::observed(false, 18),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(265004201063, "Magnified 2.5x", 19),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620412, "None", 19),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(238373620413, 10),
                    inferred(238373620415, "Angled Grip", 5),
                    inferred(238373620414, "Vertical Grip", 4),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620417, "Laser", 10),
                    named(238373620416, "None", 9),
                ],
                reference: &[],
            },
        ],
        carried: 20,
    },
    WeaponInfo {
        id: Some(78526932247),
        name: "K1A",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::reference(36),
        rpm: Stat::confirmed(720, 719, 6),
        magazine: Stat::both(30, 5, 30),
        chambered: Stat::observed(true, 5),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373620823, 5),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620827, "Flash Hider", 4),
                    unnamed(383859036488, 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373620831, "Vertical Grip", 4),
                    unnamed(238373620830, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620833, "None", 4),
                    named(238373620834, "Laser", 1),
                ],
                reference: &[],
            },
        ],
        carried: 5,
    },
    WeaponInfo {
        id: Some(127174503954),
        name: "ALDA 5.56",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::LightMachineGun),
        damage: Stat::both(35, 10, 35),
        rpm: Stat::confirmed(900, 895, 14),
        magazine: Stat::both(80, 12, 80),
        chambered: Stat::observed(false, 10),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373620232, 8),
                    unnamed(367839576167, 2),
                    unnamed(238373620231, 1),
                    unnamed(367839576165, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620237, "Flash Hider", 10),
                    inferred(238373620236, "Suppressor", 1),
                    inferred(238373620238, "Compensator", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(238373620241, 11),
                    unnamed(238373620240, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620242, "None", 8),
                    named(238373620243, "Laser", 4),
                ],
                reference: &[],
            },
        ],
        carried: 12,
    },
    WeaponInfo {
        id: Some(127174504008),
        name: "Mx4 Storm",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(26, 8, 26),
        rpm: Stat::confirmed(950, 952, 7),
        magazine: Stat::both(30, 13, 30),
        chambered: Stat::observed(true, 10),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621348, 9),
                    unnamed(367839578393, 2),
                    unnamed(238373621347, 1),
                    unnamed(367839578394, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621352, "Flash Hider", 7),
                    inferred(238373621355, "Extended Barrel", 4),
                    inferred(238373621353, "Compensator", 2),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373621357, "Vertical Grip", 12),
                    unnamed(238373621356, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621359, "None", 13),
                ],
                reference: &[],
            },
        ],
        carried: 13,
    },
    WeaponInfo {
        id: Some(139558931653),
        name: "AR-15.50",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::MarksmanRifle),
        damage: Stat::reference(59),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(10, 4, 10),
        chambered: Stat::observed(true, 4),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(359470845236, "Magnified 2.5x", 4),
                ],
                reference: SIGHTS_TELESCOPIC,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620260, "None", 3),
                    inferred(238373620261, "Suppressor", 1),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(238373620264, 3),
                    unnamed(238373620263, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620266, "None", 4),
                ],
                reference: &[],
            },
        ],
        carried: 4,
    },
    WeaponInfo {
        id: Some(139558931671),
        name: "M4",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(44, 31, 44),
        rpm: Stat::confirmed(750, 749, 43),
        magazine: Stat::both(30, 36, 30),
        chambered: Stat::observed(true, 35),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373621056, "Magnified 2.5x", 34),
                    unnamed(238373621054, 1),
                    unnamed(287652917873, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621059, "Flash Hider", 25),
                    inferred(238373621060, "Compensator", 5),
                    inferred(238373621058, "Suppressor", 4),
                    inferred(238373621061, "Muzzle Brake", 2),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(238373621064, 36),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621067, "Laser", 21),
                    named(238373621066, "None", 15),
                ],
                reference: &[],
            },
        ],
        carried: 36,
    },
    WeaponInfo {
        id: Some(161289761740),
        name: "AK-74M",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(44, 16, 44),
        rpm: Stat::confirmed(650, 649, 45),
        magazine: Stat::both(40, 36, 40),
        chambered: Stat::observed(true, 30),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(319015678260, "Magnified 2.5x", 25),
                    unnamed(319015678254, 6),
                    unnamed(367839576114, 3),
                    unnamed(238373620209, 2),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620214, "Flash Hider", 28),
                    inferred(238373620213, "Suppressor", 5),
                    unnamed(238373620215, 2),
                    unnamed(238373620216, 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(410720037348, 34),
                    unnamed(409245310500, 1),
                    unnamed(409245312471, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620218, "None", 19),
                    named(238373620219, "Laser", 17),
                ],
                reference: &[],
            },
        ],
        carried: 36,
    },
    WeaponInfo {
        id: Some(161289761758),
        name: "ARX200",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(47, 13, 47),
        rpm: Stat::confirmed(700, 697, 28),
        magazine: Stat::both(20, 23, 20),
        chambered: Stat::observed(true, 22),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373620310, "Magnified 2.5x", 13),
                    unnamed(238373620308, 4),
                    unnamed(287652917801, 4),
                    unnamed(367839576345, 1),
                    unnamed(367839609862, 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620313, "Flash Hider", 19),
                    inferred(238373620314, "Compensator", 3),
                    inferred(238373620312, "Suppressor", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(266047519808, "Vertical Grip", 19),
                    unnamed(238373620316, 2),
                    unnamed(386201549706, 2),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620317, "None", 17),
                    named(238373620318, "Laser", 6),
                ],
                reference: &[],
            },
        ],
        carried: 23,
    },
    WeaponInfo {
        id: Some(161289761794),
        name: "TCSG12",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SlugShotgun),
        damage: Stat::reference(75),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(10, 98, 10),
        chambered: Stat::observed(true, 94),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373622094, "Magnified 2.5x", 88),
                    unnamed(386201546577, 10),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373622096, "Suppressor", 60),
                    inferred(238373622095, "None", 38),
                ],
                reference: &["Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373622098, "Vertical Grip", 96),
                    inferred(238373622099, "Angled Grip", 2),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373622100, "None", 64),
                    named(238373622101, "Laser", 34),
                ],
                reference: &[],
            },
        ],
        carried: 98,
    },
    WeaponInfo {
        id: Some(161289761812),
        name: "AUG A3",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(36, 8, 40),
        rpm: Stat::confirmed(700, 703, 11),
        magazine: Stat::both(31, 4, 31),
        chambered: Stat::observed(true, 4),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373620354, 2),
                    unnamed(367839576465, 1),
                    unnamed(367839576468, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620358, "Flash Hider", 2),
                    inferred(238373620357, "Suppressor", 1),
                    inferred(238373620359, "Compensator", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373620362, "Vertical Grip", 4),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620364, "None", 3),
                    named(238373620365, "Laser", 1),
                ],
                reference: &[],
            },
        ],
        carried: 4,
    },
    WeaponInfo {
        id: Some(168013421923),
        name: "F90",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::reference(38),
        rpm: Stat::confirmed(780, 774, 14),
        magazine: Stat::both(30, 7, 30),
        chambered: Stat::observed(true, 7),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(238373620633, "Magnified 2.5x", 4),
                    unnamed(238373620631, 3),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620636, "Flash Hider", 6),
                    unnamed(386266743835, 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373620640, "Vertical Grip", 7),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620641, "None", 6),
                    unnamed(238373620642, 1),
                ],
                reference: &[],
            },
        ],
        carried: 7,
    },
    WeaponInfo {
        id: Some(168013421941),
        name: "Commando 9",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(36, 7, 40),
        rpm: Stat::confirmed(780, 778, 5),
        magazine: Stat::both(25, 10, 25),
        chambered: Stat::observed(true, 9),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373620560, 9),
                    unnamed(367839576765, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620564, "Flash Hider", 8),
                    inferred(238373620565, "Compensator", 1),
                    inferred(238373620566, "Muzzle Brake", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373620568, "Vertical Grip", 9),
                    inferred(238373620569, "Angled Grip", 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620570, "None", 9),
                    named(238373620571, "Laser", 1),
                ],
                reference: &[],
            },
        ],
        carried: 10,
    },
    WeaponInfo {
        id: Some(168013421959),
        name: "M249 SAW",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::LightMachineGun),
        damage: Stat::reference(48),
        rpm: Stat::reference(650),
        magazine: Stat::reference(60),
        chambered: Stat::UNKNOWN,
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
        ],
        carried: 0,
    },
    WeaponInfo {
        id: Some(168013421995),
        name: "P10 RONI",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::reference(26),
        rpm: Stat::confirmed(980, 993, 8),
        magazine: Stat::both(15, 7, 15),
        chambered: Stat::observed(true, 7),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(367839578443, 4),
                    unnamed(238373621415, 3),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621419, "Flash Hider", 6),
                    unnamed(238373621422, 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(238373621424, 7),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621426, "None", 4),
                    named(238373621427, "Laser", 3),
                ],
                reference: &[],
            },
        ],
        carried: 7,
    },
    WeaponInfo {
        id: Some(202492079847),
        name: "CSRX 300",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SniperRifle),
        damage: Stat::reference(135),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(5, 19, 5),
        chambered: Stat::observed(true, 18),
        fire_modes: BOLT,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373619928, "Iron Sights", 19),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373619929, "None", 19),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(238373619930, 19),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373619931, "None", 19),
                ],
                reference: &[],
            },
        ],
        carried: 19,
    },
    WeaponInfo {
        id: Some(282404990418),
        name: "DP27",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::LightMachineGun),
        damage: Stat::both(60, 3, 60),
        rpm: Stat::confirmed(550, 548, 18),
        magazine: Stat::both(70, 7, 70),
        chambered: Stat::observed(false, 7),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(278209069112, 4),
                    unnamed(379128554108, 2),
                    unnamed(379128554100, 1),
                ],
                reference: SIGHTS_DP27,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    named(261359123091, "None", 7),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(295787778372, "None", 7),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(261359123111, "None", 7),
                ],
                reference: &[],
            },
        ],
        carried: 7,
    },
    WeaponInfo {
        id: Some(291191166795),
        name: "SC3000K",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::reference(45),
        rpm: Stat::reference(800),
        magazine: Stat::both(25, 1, 25),
        chambered: Stat::observed(true, 1),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(403332439621, "Magnified 2.5x", 1),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(282357282443, "Extended Barrel", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(276763215709, 1),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(276763175323, "None", 1),
                ],
                reference: &[],
            },
        ],
        carried: 1,
    },
    WeaponInfo {
        id: Some(372271879863),
        name: "UZK50GI",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::SubmachineGun),
        damage: Stat::both(36, 21, 40),
        rpm: Stat::confirmed(700, 698, 75),
        magazine: Stat::both(22, 63, 22),
        chambered: Stat::observed(true, 61),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(361321322598, 49),
                    unnamed(367839579764, 4),
                    unnamed(367839579766, 4),
                    unnamed(367839579767, 3),
                    unnamed(361321322604, 2),
                    unnamed(361321322592, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(361321322504, "Flash Hider", 36),
                    unnamed(386266743840, 17),
                    inferred(361321322498, "Compensator", 6),
                    inferred(361321322510, "Muzzle Brake", 2),
                    unnamed(361321322516, 2),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(361321322550, "Vertical Grip", 52),
                    unnamed(361321322544, 9),
                    unnamed(382651836038, 2),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(360069650642, "None", 42),
                    named(367839527873, "Laser", 21),
                ],
                reference: &[],
            },
        ],
        carried: 63,
    },
    WeaponInfo {
        id: Some(374708016933),
        name: "Ballistic Shield",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::Shield),
        damage: Stat::UNKNOWN,
        rpm: Stat::UNKNOWN,
        magazine: Stat::UNKNOWN,
        chambered: Stat::UNKNOWN,
        fire_modes: NONE,
        attachments: &[],
        carried: 0,
    },
    WeaponInfo {
        id: Some(385168104894),
        name: "POF-9",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::reference(35),
        rpm: Stat::reference(740),
        magazine: Stat::both(50, 2, 50),
        chambered: Stat::observed(true, 2),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(382578737497, 2),
                ],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(382578737408, "Flash Hider", 2),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(382578737398, "Vertical Grip", 2),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(382578737428, "Laser", 2),
                ],
                reference: &[],
            },
        ],
        carried: 2,
    },
    WeaponInfo {
        id: Some(387336878940),
        name: "CAMRS",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::MarksmanRifle),
        damage: Stat::reference(69),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(20, 2, 20),
        chambered: Stat::observed(true, 2),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373620526, 1),
                    inferred(238373620527, "Magnified 2.5x", 1),
                ],
                reference: SIGHTS_TELESCOPIC,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620532, "Muzzle Brake", 2),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(395378995951, 1),
                    unnamed(395378998811, 1),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620535, "Laser", 2),
                ],
                reference: &[],
            },
        ],
        carried: 2,
    },
    WeaponInfo {
        id: Some(409707105529),
        name: "PCX-33",
        slot: Some(ItemKind::Primary),
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::both(36, 9, 36),
        rpm: Stat::confirmed(745, 744, 8),
        magazine: Stat::both(31, 5, 31),
        chambered: Stat::observed(true, 5),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(410271291554, 4),
                    unnamed(410271291572, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(410271289872, "Flash Hider", 2),
                    inferred(410271289987, "Suppressor", 2),
                    inferred(410271289945, "Compensator", 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(410271291313, "Vertical Grip", 5),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(410271291495, "Laser", 3),
                    named(410271291435, "None", 2),
                ],
                reference: &[],
            },
        ],
        carried: 5,
    },
    WeaponInfo {
        id: Some(431308949290),
        name: "PMR90A2",
        slot: Some(ItemKind::Primary),
        class: None,
        damage: Stat::observed(62, 16),
        rpm: Stat::UNKNOWN,
        magazine: Stat::observed(20, 22),
        chambered: Stat::observed(true, 21),
        fire_modes: NONE,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(432227685183, "Magnified 2.5x", 15),
                    unnamed(433965912215, 3),
                    unnamed(433965912220, 2),
                    unnamed(432227685182, 1),
                    unnamed(432227685184, 1),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(432227685079, "Muzzle Brake", 20),
                    inferred(432227685077, "Suppressor", 2),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(432227685157, "Vertical Grip", 20),
                    unnamed(432317663915, 2),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(432227685277, "None", 17),
                    named(433965908877, "Laser", 5),
                ],
                reference: &[],
            },
        ],
        carried: 22,
    },
    WeaponInfo {
        id: Some(437464101745),
        name: "XK23",
        slot: Some(ItemKind::Primary),
        class: None,
        damage: Stat::observed(49, 18),
        rpm: Stat::observed(676, 64),
        magazine: Stat::observed(35, 31),
        chambered: Stat::observed(true, 28),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    inferred(437464101714, "Magnified 2.5x", 30),
                    unnamed(437464101713, 1),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(437464101718, "Flash Hider", 16),
                    inferred(437464101716, "Compensator", 11),
                    inferred(437464101717, "Extended Barrel", 4),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(437464101724, "Vertical Grip", 30),
                    inferred(437464101722, "Angled Grip", 1),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(437464101726, "None", 20),
                    named(437464101725, "Laser", 11),
                ],
                reference: &[],
            },
        ],
        carried: 31,
    },
    WeaponInfo {
        id: Some(1366019400),
        name: "SMG-11",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::MachinePistol),
        damage: Stat::both(32, 35, 32),
        rpm: Stat::confirmed(1270, 1261, 96),
        magazine: Stat::both(16, 123, 16),
        chambered: Stat::observed(true, 95),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621825, 110),
                    unnamed(367839580004, 4),
                    unnamed(367839580014, 4),
                    unnamed(367839580007, 3),
                    unnamed(238373621826, 2),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621829, "Flash Hider", 90),
                    inferred(238373621830, "Compensator", 26),
                    inferred(384304376806, "Suppressor", 3),
                    inferred(238373621828, "Suppressor", 2),
                    inferred(238373621831, "Extended Barrel", 2),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    unnamed(386201549751, 123),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621834, "None", 68),
                    named(238373621835, "Laser", 55),
                ],
                reference: &[],
            },
        ],
        carried: 123,
    },
    WeaponInfo {
        id: Some(1366019412),
        name: "5.7 USG",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(42),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(20, 57, 20),
        chambered: Stat::observed(true, 11),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373620029, "Iron Sights", 57),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620032, "Muzzle Brake", 52),
                    inferred(238373620031, "Suppressor", 5),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373620033, "None", 57),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620034, "None", 38),
                    named(238373620035, "Laser", 19),
                ],
                reference: &[],
            },
        ],
        carried: 57,
    },
    WeaponInfo {
        id: Some(1366019424),
        name: "LFP586",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Revolver),
        damage: Stat::reference(78),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(6, 21, 6),
        chambered: Stat::observed(false, 6),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373620897, "Iron Sights", 21),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620898, "None", 21),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373620899, "None", 21),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620900, "None", 14),
                    named(258614298870, "Laser", 7),
                ],
                reference: &[],
            },
        ],
        carried: 21,
    },
    WeaponInfo {
        id: Some(1366019436),
        name: "GSH-18",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(44),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(18, 23, 18),
        chambered: Stat::observed(true, 1),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373620765, "Iron Sights", 23),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620768, "Muzzle Brake", 19),
                    inferred(238373620767, "Suppressor", 4),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373620769, "None", 23),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(381724020437, "None", 14),
                    named(381724020432, "Laser", 9),
                ],
                reference: &[],
            },
        ],
        carried: 23,
    },
    WeaponInfo {
        id: Some(1366019448),
        name: "M45 MEUSOC",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(58),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(7, 28, 7),
        chambered: Stat::observed(true, 3),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373621079, "Iron Sights", 28),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621082, "Muzzle Brake", 22),
                    inferred(238373621081, "Suppressor", 6),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621083, "None", 28),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621084, "None", 18),
                    named(238373621085, "Laser", 10),
                ],
                reference: &[],
            },
        ],
        carried: 29,
    },
    WeaponInfo {
        id: Some(1366019460),
        name: "PMM",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(61),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(8, 72, 8),
        chambered: Stat::observed(true, 11),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373621596, "Iron Sights", 72),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621599, "Muzzle Brake", 66),
                    inferred(238373621598, "Suppressor", 6),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621600, "None", 72),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621601, "None", 36),
                    named(238373621602, "Laser", 36),
                ],
                reference: &[],
            },
        ],
        carried: 72,
    },
    WeaponInfo {
        id: Some(1366019472),
        name: "P9",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::both(45, 22, 45),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(16, 140, 16),
        chambered: Stat::observed(true, 32),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373621489, "Iron Sights", 140),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(258614298918, "Muzzle Brake", 118),
                    inferred(258614298919, "Suppressor", 22),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621491, "None", 140),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(258614298926, "Laser", 74),
                    named(238373621492, "None", 66),
                ],
                reference: &[],
            },
        ],
        carried: 140,
    },
    WeaponInfo {
        id: Some(1366019484),
        name: "P226 MK 25",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(50),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(15, 68, 15),
        chambered: Stat::observed(true, 17),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373621454, "Iron Sights", 68),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621457, "Muzzle Brake", 57),
                    inferred(238373621456, "Suppressor", 9),
                    inferred(238373621455, "None", 2),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621458, "None", 68),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621459, "None", 54),
                    named(238373621460, "Laser", 14),
                ],
                reference: &[],
            },
        ],
        carried: 68,
    },
    WeaponInfo {
        id: Some(1366019496),
        name: "P12",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::both(44, 14, 44),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(15, 45, 15),
        chambered: Stat::observed(true, 26),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373621439, "Iron Sights", 45),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(258614298907, "Muzzle Brake", 33),
                    inferred(258614298906, "Suppressor", 10),
                    inferred(238373621440, "None", 2),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621441, "None", 45),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(258614298914, "Laser", 25),
                    named(238373621442, "None", 20),
                ],
                reference: &[],
            },
        ],
        carried: 45,
    },
    WeaponInfo {
        id: Some(13333481774),
        name: "MK1 9mm",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(48),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(13, 44, 13),
        chambered: Stat::observed(true, 6),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373621163, "Iron Sights", 44),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621166, "Muzzle Brake", 31),
                    inferred(238373621165, "Suppressor", 12),
                    inferred(238373621164, "None", 1),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621167, "None", 44),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621169, "Laser", 28),
                    named(238373621168, "None", 16),
                ],
                reference: &[],
            },
        ],
        carried: 45,
    },
    WeaponInfo {
        id: Some(34160268720),
        name: "D-50",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(71),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(7, 94, 7),
        chambered: Stat::observed(true, 66),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373620584, "Iron Sights", 94),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620587, "Muzzle Brake", 83),
                    inferred(238373620586, "Suppressor", 10),
                    inferred(238373620585, "None", 1),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373620588, "None", 94),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620589, "None", 52),
                    named(238373620590, "Laser", 42),
                ],
                reference: &[],
            },
        ],
        carried: 94,
    },
    WeaponInfo {
        id: Some(38581443676),
        name: "PRB92",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(42),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(15, 52, 15),
        chambered: Stat::observed(true, 6),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373621614, "Iron Sights", 52),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621617, "Muzzle Brake", 45),
                    inferred(261952020333, "Suppressor", 7),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621618, "None", 52),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621619, "None", 33),
                    named(238373621620, "Laser", 19),
                ],
                reference: &[],
            },
        ],
        carried: 52,
    },
    WeaponInfo {
        id: Some(39149214668),
        name: "Bearing 9",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::MachinePistol),
        damage: Stat::reference(33),
        rpm: Stat::confirmed(1100, 1106, 14),
        magazine: Stat::both(25, 49, 25),
        chambered: Stat::observed(true, 17),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(379793656710, 38),
                    unnamed(379793656714, 4),
                    unnamed(379793656718, 4),
                    named(238373620385, "Iron Sights", 1),
                    unnamed(379793656712, 1),
                    unnamed(379793656719, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620391, "Flash Hider", 38),
                    inferred(238373620392, "Compensator", 7),
                    unnamed(381580290053, 2),
                    unnamed(384304375552, 2),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373620393, "None", 49),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620394, "None", 28),
                    named(238373620395, "Laser", 21),
                ],
                reference: &[],
            },
        ],
        carried: 49,
    },
    WeaponInfo {
        id: Some(39149214734),
        name: "P229",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(51),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(12, 49, 12),
        chambered: Stat::observed(true, 22),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373621472, "Iron Sights", 49),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621475, "Muzzle Brake", 42),
                    inferred(238373621474, "Suppressor", 7),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621476, "None", 49),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621477, "None", 32),
                    named(238373621478, "Laser", 17),
                ],
                reference: &[],
            },
        ],
        carried: 49,
    },
    WeaponInfo {
        id: Some(39149216440),
        name: "ITA12S",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(29),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(5, 124, 5),
        chambered: Stat::observed(false, 64),
        fire_modes: PUMP,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373620804, 71),
                    unnamed(238373620803, 38),
                    inferred(238373620801, "Iron Sights", 13),
                    unnamed(367839582799, 1),
                    unnamed(367839582808, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620805, "None", 124),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373620806, "None", 124),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620808, "Laser", 102),
                    named(238373620807, "None", 22),
                ],
                reference: &[],
            },
        ],
        carried: 124,
    },
    WeaponInfo {
        id: Some(39149216488),
        name: "USP40",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(48),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(12, 6, 12),
        chambered: Stat::UNKNOWN,
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373622164, "Iron Sights", 6),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373622167, "Muzzle Brake", 5),
                    inferred(238373622166, "Suppressor", 1),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373622168, "None", 6),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373622169, "None", 5),
                    named(238373622170, "Laser", 1),
                ],
                reference: &[],
            },
        ],
        carried: 6,
    },
    WeaponInfo {
        id: Some(39404979943),
        name: "Luison",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(65),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(12, 10, 12),
        chambered: Stat::observed(true, 10),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373621614, "Iron Sights", 10),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    unnamed(258614243451, 10),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621618, "None", 10),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621619, "None", 7),
                    named(238373621620, "Laser", 3),
                ],
                reference: &[],
            },
        ],
        carried: 10,
    },
    WeaponInfo {
        id: Some(53995319945),
        name: "Q-929",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(60),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(10, 48, 10),
        chambered: Stat::observed(true, 17),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373621633, "Iron Sights", 48),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621636, "Muzzle Brake", 46),
                    inferred(238373621635, "Suppressor", 2),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621637, "None", 48),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621638, "None", 37),
                    named(238373621639, "Laser", 11),
                ],
                reference: &[],
            },
        ],
        carried: 48,
    },
    WeaponInfo {
        id: Some(68967567430),
        name: "RG15",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(38),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(15, 87, 15),
        chambered: Stat::observed(true, 19),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621676, 87),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621679, "Muzzle Brake", 80),
                    inferred(238373621678, "Suppressor", 7),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621680, "None", 87),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621681, "None", 50),
                    inferred(238373621682, "Laser", 37),
                ],
                reference: &[],
            },
        ],
        carried: 88,
    },
    WeaponInfo {
        id: Some(78526932130),
        name: "SMG-12",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::MachinePistol),
        damage: Stat::both(16, 7, 16),
        rpm: Stat::confirmed(1270, 1268, 24),
        magazine: Stat::both(22, 55, 22),
        chambered: Stat::observed(true, 32),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621849, 51),
                    unnamed(367839580064, 3),
                    unnamed(367839580067, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621851, "None", 55),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(238373621853, "Vertical Grip", 55),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621856, "Laser", 28),
                    named(238373621855, "None", 27),
                ],
                reference: &[],
            },
        ],
        carried: 56,
    },
    WeaponInfo {
        id: Some(78526932169),
        name: "C75 Auto",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::MachinePistol),
        damage: Stat::reference(35),
        rpm: Stat::confirmed(1000, 998, 19),
        magazine: Stat::both(26, 79, 26),
        chambered: Stat::observed(true, 26),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373620455, "Iron Sights", 79),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620456, "None", 66),
                    inferred(261951977525, "Suppressor", 13),
                ],
                reference: &["Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373620458, "None", 79),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(261952103079, "None", 50),
                    inferred(381724063158, "Laser", 29),
                ],
                reference: &[],
            },
        ],
        carried: 79,
    },
    WeaponInfo {
        id: Some(127174503972),
        name: "Bailiff 410",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(30),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(5, 91, 5),
        chambered: Stat::observed(false, 60),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373620439, 91),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620440, "None", 91),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373620441, "None", 91),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620442, "None", 68),
                    inferred(269097048259, "Laser", 23),
                ],
                reference: &[],
            },
        ],
        carried: 91,
    },
    WeaponInfo {
        id: Some(127174503990),
        name: "Keratos .357",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Revolver),
        damage: Stat::reference(78),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(6, 53, 6),
        chambered: Stat::observed(false, 35),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373620846, "Iron Sights", 53),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373620849, "Muzzle Brake", 51),
                    inferred(238373620848, "Suppressor", 2),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373620850, "None", 53),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620851, "None", 49),
                    named(238373620852, "Laser", 4),
                ],
                reference: &[],
            },
        ],
        carried: 53,
    },
    WeaponInfo {
        id: Some(139558931689),
        name: "P-10C",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(40),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(15, 25, 15),
        chambered: Stat::observed(true, 10),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(238373621395, 25),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621398, "Muzzle Brake", 21),
                    inferred(238373621397, "Suppressor", 3),
                    inferred(238373621396, "None", 1),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621399, "None", 25),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621400, "None", 23),
                    named(238373621401, "Laser", 2),
                ],
                reference: &[],
            },
        ],
        carried: 25,
    },
    WeaponInfo {
        id: Some(139558931707),
        name: "1911 TACOPS",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(55),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(8, 13, 8),
        chambered: Stat::observed(true, 4),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373619959, "Iron Sights", 13),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373619962, "Muzzle Brake", 11),
                    inferred(238373619961, "Suppressor", 2),
                ],
                reference: &["Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373619963, "None", 13),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373619964, "None", 8),
                    named(238373619965, "Laser", 5),
                ],
                reference: &[],
            },
        ],
        carried: 13,
    },
    WeaponInfo {
        id: Some(139558931725),
        name: "Super Shorty",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(35),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(3, 123, 3),
        chambered: Stat::observed(false, 81),
        fire_modes: PUMP,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373621998, "Iron Sights", 78),
                    unnamed(367839582910, 33),
                    unnamed(367839582899, 4),
                    unnamed(367839582898, 3),
                    unnamed(367839582900, 3),
                    unnamed(367839582902, 2),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373622000, "None", 123),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373622001, "None", 123),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373622003, "Laser", 92),
                    named(238373622002, "None", 31),
                ],
                reference: &[],
            },
        ],
        carried: 123,
    },
    WeaponInfo {
        id: Some(139558932060),
        name: "SPSMG9",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::MachinePistol),
        damage: Stat::both(35, 4, 35),
        rpm: Stat::confirmed(980, 969, 11),
        magazine: Stat::both(20, 26, 20),
        chambered: Stat::observed(true, 17),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(367839580123, 19),
                    unnamed(367839580124, 3),
                    unnamed(367839580129, 3),
                    unnamed(367839580128, 1),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621934, "Flash Hider", 17),
                    inferred(238373621933, "Suppressor", 4),
                    unnamed(384304388223, 4),
                    unnamed(386266743820, 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621935, "None", 26),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621937, "Laser", 18),
                    named(238373621936, "None", 8),
                ],
                reference: &[],
            },
        ],
        carried: 26,
    },
    WeaponInfo {
        id: Some(161289761776),
        name: ".44 Mag Semi-Auto",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(54),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(7, 36, 7),
        chambered: Stat::observed(true, 7),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373619943, "Iron Sights", 36),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373619944, "None", 36),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373619945, "None", 36),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373619946, "None", 25),
                    inferred(269097048242, "Laser", 11),
                ],
                reference: &[],
            },
        ],
        carried: 36,
    },
    WeaponInfo {
        id: Some(168013421977),
        name: "SDP 9mm",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Handgun),
        damage: Stat::reference(47),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(16, 12, 16),
        chambered: Stat::observed(true, 4),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373621742, "Iron Sights", 12),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(238373621745, "Muzzle Brake", 10),
                    inferred(238373621744, "Suppressor", 2),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373621746, "None", 12),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373621748, "Laser", 7),
                    named(238373621747, "None", 5),
                ],
                reference: &[],
            },
        ],
        carried: 12,
    },
    WeaponInfo {
        id: Some(263048049828),
        name: "5.7 USG (Zero)",
        slot: Some(ItemKind::Secondary),
        class: None,
        damage: Stat::UNKNOWN,
        rpm: Stat::UNKNOWN,
        magazine: Stat::observed(20, 2),
        chambered: Stat::UNKNOWN,
        fire_modes: NONE,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(238373620029, "Iron Sights", 2),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(338495374535, "Suppressor", 2),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(238373620033, "None", 2),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(238373620034, "None", 1),
                    named(238373620035, "Laser", 1),
                ],
                reference: &[],
            },
        ],
        carried: 2,
    },
    WeaponInfo {
        id: Some(350658628377),
        name: "Gonne-6",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::HandCannon),
        damage: Stat::reference(10),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(1, 15, 1),
        chambered: Stat::observed(false, 6),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(338495478756, "Iron Sights", 15),
                ],
                reference: &[],
            },
        ],
        carried: 15,
    },
    WeaponInfo {
        id: Some(405866692215),
        name: ".44 Vendetta",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Revolver),
        damage: Stat::both(78, 6, 78),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(6, 19, 6),
        chambered: Stat::observed(false, 14),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    named(402828642512, "Iron Sights", 19),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(402828642502, "None", 19),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(402828642507, "None", 19),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(402828642517, "None", 10),
                    named(406663202146, "Laser", 9),
                ],
                reference: &[],
            },
        ],
        carried: 19,
    },
    WeaponInfo {
        id: Some(410903041523),
        name: "Reaper MK2",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::MachinePistol),
        damage: Stat::reference(31),
        rpm: Stat::reference(765),
        magazine: Stat::both(33, 57, 33),
        chambered: Stat::observed(true, 13),
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(415209461530, "Flash Hider", 52),
                    inferred(425171859937, "Suppressor", 4),
                    unnamed(418757780169, 1),
                ],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(417867535365, "None", 38),
                    named(417867535370, "Laser", 19),
                ],
                reference: &[],
            },
        ],
        carried: 57,
    },
    WeaponInfo {
        id: Some(424122423516),
        name: "Glaive-12",
        slot: Some(ItemKind::Secondary),
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(63),
        rpm: Stat::UNKNOWN,
        magazine: Stat::both(4, 27, 4),
        chambered: Stat::observed(false, 21),
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[
                    unnamed(431203370052, 25),
                    unnamed(431496217709, 2),
                ],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    inferred(431203370515, "None", 27),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    inferred(431203370560, "Vertical Grip", 25),
                    unnamed(431496217718, 2),
                ],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    named(431203370497, "None", 24),
                    named(431203370498, "Laser", 3),
                ],
                reference: &[],
            },
        ],
        carried: 27,
    },
    WeaponInfo {
        id: Some(441691679238),
        name: "TACIT .45",
        slot: Some(ItemKind::Secondary),
        class: None,
        damage: Stat::UNKNOWN,
        rpm: Stat::UNKNOWN,
        magazine: Stat::observed(8, 39),
        chambered: Stat::observed(true, 6),
        fire_modes: NONE,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[
                    unnamed(436340829096, 39),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[
                    named(433965900918, "None", 39),
                ],
                reference: &[],
            },
            AttachmentSlot {
                slot: Slot::Underbarrel,
                seen: &[
                    unnamed(436860602719, 23),
                    unnamed(436860602536, 16),
                ],
                reference: &[],
            },
        ],
        carried: 39,
    },
    WeaponInfo {
        id: None,
        name: "G36C",
        slot: None,
        class: Some(WeaponClass::AssaultRifle),
        damage: Stat::reference(38),
        rpm: Stat::reference(780),
        magazine: Stat::reference(30),
        chambered: Stat::UNKNOWN,
        fire_modes: FULL,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[],
                reference: SIGHTS_MAGNIFIED,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[],
                reference: &["Flash Hider", "Compensator", "Muzzle Brake", "Suppressor", "Extended Barrel"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
        ],
        carried: 0,
    },
    WeaponInfo {
        id: None,
        name: "SASG-12",
        slot: None,
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(26),
        rpm: Stat::UNKNOWN,
        magazine: Stat::reference(10),
        chambered: Stat::UNKNOWN,
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[],
                reference: SIGHTS_1X,
            },
            AttachmentSlot {
                slot: Slot::Barrel,
                seen: &[],
                reference: &["Suppressor"],
            },
            AttachmentSlot {
                slot: Slot::Grip,
                seen: &[],
                reference: &["Vertical Grip", "Angled Grip", "Horizontal Grip"],
            },
        ],
        carried: 0,
    },
    WeaponInfo {
        id: None,
        name: "Super 90",
        slot: None,
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(27),
        rpm: Stat::UNKNOWN,
        magazine: Stat::reference(8),
        chambered: Stat::UNKNOWN,
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[],
                reference: SIGHTS_1X,
            },
        ],
        carried: 0,
    },
    WeaponInfo {
        id: None,
        name: "SIX12 SD",
        slot: None,
        class: Some(WeaponClass::Shotgun),
        damage: Stat::reference(46),
        rpm: Stat::UNKNOWN,
        magazine: Stat::reference(6),
        chambered: Stat::UNKNOWN,
        fire_modes: SEMI,
        attachments: &[
            AttachmentSlot {
                slot: Slot::Sight,
                seen: &[],
                reference: SIGHTS_1X,
            },
        ],
        carried: 0,
    },
];

/// Guns no reference lists: what the catalog says of them is what replays
/// show.
pub(super) const UNKNOWN: &[u64] = &[
    431308949290, // PMR90A2
    437464101745, // XK23
    263048049828, // 5.7 USG (Zero)
    441691679238, // TACIT .45
];
