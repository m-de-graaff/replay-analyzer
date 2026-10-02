//! Effect assets and environment objects (Y11S3), for [`crate::areas`].
//!
//! Nothing in a replay names an effect asset. Every name here is inferred
//! from what the effect does in 10 test rounds and 175 real rounds: where
//! it plays, how long, on what, and which thrown object ended there.

/// What an effect asset of the `FXChannel` stream is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxAsset {
    pub name: &'static str,
    /// What kind of effect: `area`, `area visual`, `environment`,
    /// `objective`, `explosion`, `flash`, `emp`, `trap`, `breach`,
    /// `gadget`, `projectile`, `hit`, `impact` or `map`.
    pub category: &'static str,
}

/// `(asset, name, category)`, the names inferred.
const FX_ASSETS: &[(u64, &str, &str)] = &[
    (27012166679, "Smoke grenade cloud", "area"),
    (73007452394, "Capitao smoke bolt cloud", "area"),
    (
        223502673865,
        "Burning floor: Volcan, Shumikha, fire bolt, gas pipe (told apart by the area entity)",
        "area",
    ),
    (
        440770340286,
        "One-cell fire 7.6 s after a Logic Bomb (Dokkaebi)",
        "area",
    ),
    (297391115747, "Smoke remote gas cloud", "area"),
    (379510214638, "Grim Kawan swarm cloud", "area"),
    (401787054484, "Fire extinguisher burst cloud", "area"),
    (407368224089, "Gas pipe explosion", "environment"),
    (418737471057, "Metal detector idle light", "environment"),
    (418737478887, "Metal detector alarm", "environment"),
    (
        418737478950,
        "Metal detector switched off (Mute in every round)",
        "environment",
    ),
    (
        14196639350,
        "Bomb site alarm after the plant (on both bombs)",
        "objective",
    ),
    (36281862849, "Bomb site alarm after the plant", "objective"),
    (24908932859, "Bomb site alarm after the plant", "objective"),
    (
        7729294373,
        "Bomb site effect at the end of the round",
        "objective",
    ),
    (755139546, "Frag grenade explosion", "explosion"),
    (39930571910, "Impact grenade explosion", "explosion"),
    (13159102487, "Nitro cell explosion", "explosion"),
    (
        1412993321,
        "Nitro cell explosion (second form)",
        "explosion",
    ),
    (
        2439246751,
        "Stun grenade flash (also Grzmot mine, Zofia concussion)",
        "flash",
    ),
    (53352485036, "Ying Candela flash charge burst", "flash"),
    (52899546045, "Ying Candela device going off", "flash"),
    (58430194579, "Ying Candela flash charge in flight", "flash"),
    (386515793555, "Impact EMP grenade burst", "emp"),
    (
        386515701546,
        "Electronics burst with no thrown object ending there; rounds with Thatcher, Kali, Twitch or Brava: source not identified",
        "emp",
    ),
    (386515701681, "Companion of 59fe218f2a (no position)", "emp"),
    (244963865692, "Tachanka Shumikha explosion", "explosion"),
    (149135584895, "Smoke gas grenade detonation", "explosion"),
    (
        296027842133,
        "Smoke gas puff (many per cloud)",
        "area visual",
    ),
    (387430200499, "Grim hive releasing its swarm", "explosion"),
    (375748522318, "Thorn Razorbloom explosion", "explosion"),
    (71127325879, "Zofia KS79 grenade explosion", "explosion"),
    (
        9472304510,
        "Fuze cluster charge puck explosion",
        "explosion",
    ),
    (6798395880, "Ash breaching round explosion", "explosion"),
    (228342131206, "Kali LV lance explosion", "explosion"),
    (
        44338907834,
        "Hibana X-KAIROS pellets detonating",
        "explosion",
    ),
    (266057087243, "Ace S.E.L.M.A. detonation", "explosion"),
    (156258452874, "Nomad Airjab burst", "explosion"),
    (240681674602, "Wamai Mag-NET detonation", "explosion"),
    (51584061930, "Lesion Gu mine triggered", "trap"),
    (352450803191, "Gonne-6 round explosion", "explosion"),
    (350341387339, "Flores RCE-Ratero explosion", "explosion"),
    (364332895068, "Goyo Volcan canister explosion", "explosion"),
    (
        456988545148,
        "Goyo Volcan canister bursting (on the canister)",
        "explosion",
    ),
    (191754370014, "Capitao bolt impact", "explosion"),
    (
        198327402025,
        "Capitao bolt impact (second effect)",
        "explosion",
    ),
    (
        205171344728,
        "Capitao bolt in the world (on the bolt)",
        "projectile",
    ),
    (
        428213181853,
        "Thermite exothermic charge burning through",
        "breach",
    ),
    (44898845502, "Mira (black mirror) effect, 8 s", "gadget"),
    (
        12520128474,
        "Explosion with alliance, 5 s, in 180 of 185 rounds: unnamed",
        "explosion",
    ),
    (
        285380329407,
        "Breach charge or hard breach burning, 8 s: unnamed",
        "breach",
    ),
    (2439242220, "8 s effect with alliance: unnamed", "breach"),
    (
        385282566546,
        "Sens R.O.U. light wall segment (on a projector)",
        "area",
    ),
    (385575871625, "Sens R.O.U. projector expiring", "gadget"),
    (387002442194, "Sens R.O.U. roller stopping", "gadget"),
    (385907458857, "Sens R.O.U. roller rolling", "gadget"),
    (
        401355040844,
        "Tubarao Zoto canister freezing (12 s, on the canister)",
        "area",
    ),
    (399364199034, "Tubarao Zoto canister arming", "gadget"),
    (396818254481, "Fenrir F-NATT mine (on the mine)", "gadget"),
    (
        371882659065,
        "Melusi Banshee active (6 s, on the Banshee)",
        "area",
    ),
    (360024787637, "Melusi Banshee (second effect)", "gadget"),
    (375630094805, "Azami Kiba barrier expanding", "gadget"),
    (
        375371530017,
        "Azami Kiba barrier (on the barrier)",
        "gadget",
    ),
    (
        378016358700,
        "Azami Kiba barrier (on the barrier)",
        "gadget",
    ),
    (
        379509191905,
        "Goyo Volcan canister (on the canister)",
        "gadget",
    ),
    (236942668790, "Wamai Mag-NET active", "gadget"),
    (230182264546, "Wamai Mag-NET catching", "gadget"),
    (
        393053067438,
        "Grzmot mine / Zoto canister sticking",
        "gadget",
    ),
    (
        1480682965,
        "Bullet hitting a body (target = the body)",
        "hit",
    ),
    (
        249218831041,
        "Effect on a body, never stopped: unnamed",
        "hit",
    ),
    (27330765575, "Bullet impact on a surface", "impact"),
    (27330765740, "Bullet impact on a surface", "impact"),
    (27330806628, "Bullet impact on metal", "impact"),
    (420569506690, "Debris of a destroyed surface", "impact"),
    (22334292155, "Effect on a map object, 3 s: unnamed", "map"),
    (412473058092, "Drone jumping", "gadget"),
];

/// The inferred name and category of an effect asset.
pub fn fx_asset(asset: u64) -> Option<FxAsset> {
    let found = FX_ASSETS.iter().find(|a| a.0 == asset)?;
    Some(FxAsset {
        name: found.1,
        category: found.2,
    })
}

/// An object of the map that players set off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EnvObject {
    GasPipe,
    FireExtinguisher,
    MetalDetector,
}

use EnvObject::{FireExtinguisher, GasPipe, MetalDetector};

/// `(map, object, what it is)`: the map objects seen going off in 185
/// rounds, by the map's id (see `MAPS`). A map object has the same id in
/// every round on its map. Objects no round set off are not listed.
const ENV_OBJECTS: &[(u64, u64, EnvObject)] = &[
    // BankY10
    (413779563590, 0x60_7AFD_5716, FireExtinguisher),
    (413779563590, 0x60_7AFD_5733, FireExtinguisher),
    (413779563590, 0x60_7AFD_5735, FireExtinguisher),
    (413779563590, 0x60_7AFD_5737, FireExtinguisher),
    (413779563590, 0x60_7AFD_573B, FireExtinguisher),
    (413779563590, 0x60_7AFD_57A6, FireExtinguisher),
    (413779563590, 0x60_7AFD_57BD, FireExtinguisher),
    (413779563590, 0x60_7AFD_57F2, FireExtinguisher),
    (413779563590, 0x61_E910_A0E2, GasPipe),
    (413779563590, 0x61_E910_A0E8, GasPipe),
    (413779563590, 0x61_E910_A0ED, GasPipe),
    (413779563590, 0x63_0F39_FA15, MetalDetector),
    (413779563590, 0x63_0F39_FA59, MetalDetector),
    (413779563590, 0x63_0F39_FAA3, MetalDetector),
    (413779563590, 0x63_1500_877C, MetalDetector),
    // KafeDostoyevskyY10
    (413845419788, 0x60_5B1B_FC7A, GasPipe),
    (413845419788, 0x60_65FB_CA46, FireExtinguisher),
    (413845419788, 0x60_65FB_CA62, FireExtinguisher),
    (413845419788, 0x60_65FB_CA79, FireExtinguisher),
    (413845419788, 0x60_7AFD_2508, FireExtinguisher),
    (413845419788, 0x60_7AFD_250A, FireExtinguisher),
    (413845419788, 0x60_7AFD_2536, FireExtinguisher),
    (413845419788, 0x62_08FC_0AA2, GasPipe),
    (413845419788, 0x62_08FC_0AA8, GasPipe),
    // NighthavenLabsY10
    (418119057546, 0x63_0157_D2C0, FireExtinguisher),
    (418119057546, 0x63_0189_EB1C, FireExtinguisher),
    (418119057546, 0x63_0189_EB1E, FireExtinguisher),
    (418119057546, 0x63_0189_EB61, GasPipe),
    (418119057546, 0x63_0189_EB67, GasPipe),
    (418119057546, 0x66_DEF5_C053, MetalDetector),
    // ConsulateY10
    (418126004176, 0x61_E910_86B8, GasPipe),
    (418126004176, 0x61_E910_86BD, GasPipe),
    (418126004176, 0x62_F616_7C20, FireExtinguisher),
    (418126004176, 0x62_F616_7C62, FireExtinguisher),
    (418126004176, 0x62_F616_7C66, FireExtinguisher),
    (418126004176, 0x64_AAF6_8BCD, MetalDetector),
    (418126004176, 0x64_AB89_0806, MetalDetector),
    // BorderY10
    (407987100456, 0x60_7AFD_3961, FireExtinguisher),
    (407987100456, 0x60_7AFD_3995, FireExtinguisher),
    (407987100456, 0x60_7AFD_39C3, FireExtinguisher),
    (407987100456, 0x60_7AFD_39DC, FireExtinguisher),
    (407987100456, 0x60_7AFD_39F9, FireExtinguisher),
    (407987100456, 0x62_0763_F801, GasPipe),
    // ThemeParkY10
    (430788891316, 0x65_84A0_47D1, FireExtinguisher),
    (430788891316, 0x65_84A1_95EE, FireExtinguisher),
    (430788891316, 0x65_84A1_9637, FireExtinguisher),
    (430788891316, 0x65_84A1_965C, FireExtinguisher),
    (430788891316, 0x65_84A1_9680, FireExtinguisher),
    (430788891316, 0x65_84A1_96FA, GasPipe),
    (430788891316, 0x65_84A1_9763, GasPipe),
    // CalypsoCasino
    (419965653950, 0x62_BCE7_9344, FireExtinguisher),
    (419965653950, 0x62_BCE7_93AF, FireExtinguisher),
    (419965653950, 0x62_BCE7_9412, FireExtinguisher),
    (419965653950, 0x62_E968_E134, GasPipe),
    (419965653950, 0x62_E969_FD8D, GasPipe),
    (419965653950, 0x62_E969_FDD4, GasPipe),
    (419965653950, 0x62_E969_FE5B, GasPipe),
    // ClubHouseY10
    (407193663917, 0x5F_9AE4_C5BD, FireExtinguisher),
    (407193663917, 0x5F_9AE6_970A, FireExtinguisher),
    (407193663917, 0x5F_9AE6_970C, FireExtinguisher),
    (407193663917, 0x5F_9AE6_9728, FireExtinguisher),
    (407193663917, 0x5F_9AE6_9737, FireExtinguisher),
    (407193663917, 0x5F_9AE6_9764, FireExtinguisher),
    (407193663917, 0x5F_9AE6_97B3, FireExtinguisher),
    (407193663917, 0x60_321C_EAAB, FireExtinguisher),
    (407193663917, 0x60_321C_EB04, FireExtinguisher),
    (407193663917, 0x60_A59A_7DAD, GasPipe),
    (407193663917, 0x60_BAF0_8B02, GasPipe),
    (407193663917, 0x60_BAF0_8B29, GasPipe),
    (407193663917, 0x60_BAF0_8B50, GasPipe),
    (407193663917, 0x60_BAF0_8B77, GasPipe),
    (407193663917, 0x60_BAF0_8B79, GasPipe),
    (407193663917, 0x60_BAF0_8BCB, GasPipe),
    (407193663917, 0x60_BAF0_8BF6, GasPipe),
    // CoastlineY11
    (436375283234, 0x66_8DDB_8076, FireExtinguisher),
    (436375283234, 0x66_9A7D_A978, GasPipe),
    (436375283234, 0x66_9A7D_AA33, GasPipe),
    // FortressY10
    (398899676157, 0x63_0155_429A, FireExtinguisher),
    (398899676157, 0x63_0157_B4C1, GasPipe),
    (398899676157, 0x63_0157_B535, GasPipe),
    // VillaY11
    (409325881472, 0x60_7B48_E87E, FireExtinguisher),
    (409325881472, 0x60_7B48_E896, FireExtinguisher),
    (409325881472, 0x60_7B48_E8B1, FireExtinguisher),
    // KanalY11
    (441408792952, 0x66_DECB_61AE, MetalDetector),
    // LairY10
    (417890697769, 0x63_01A1_1134, FireExtinguisher),
    (417890697769, 0x63_01A1_1158, GasPipe),
    (417890697769, 0x63_01A1_124B, GasPipe),
];

/// What the map object `object` of the map `map` is, when a round was seen
/// setting it off.
pub(crate) fn env_object(map: u64, object: u64) -> Option<EnvObject> {
    let found = ENV_OBJECTS.iter().find(|o| o.0 == map && o.1 == object)?;
    Some(found.2)
}

/// The objects of that kind known on `map`.
pub(crate) fn env_objects(map: u64, kind: EnvObject) -> impl Iterator<Item = u64> {
    let of = move |o: &&(u64, u64, EnvObject)| o.0 == map && o.2 == kind;
    ENV_OBJECTS.iter().filter(of).map(|o| o.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_and_objects_are_listed_once() {
        for (i, a) in FX_ASSETS.iter().enumerate() {
            assert!(FX_ASSETS.iter().skip(i + 1).all(|b| b.0 != a.0), "{}", a.0);
        }
        for (i, a) in ENV_OBJECTS.iter().enumerate() {
            let again = |b: &(u64, u64, EnvObject)| b.0 == a.0 && b.1 == a.1;
            assert!(!ENV_OBJECTS.iter().skip(i + 1).any(again), "{:x}", a.1);
        }
    }

    #[test]
    fn an_asset_and_an_object_are_found() {
        let smoke = fx_asset(27012166679).unwrap();
        assert_eq!(smoke.category, "area");
        assert_eq!(fx_asset(1), None);
        let bank = 413779563590;
        assert_eq!(env_object(bank, 0x61_E910_A0E2), Some(GasPipe));
        assert_eq!(env_object(bank, 0x60_7AFD_5716), Some(FireExtinguisher));
        assert_eq!(env_object(bank + 1, 0x61_E910_A0E2), None);
        assert_eq!(env_objects(bank, MetalDetector).count(), 4);
    }
}
