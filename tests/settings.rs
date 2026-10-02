//! The settings reader and its history, on synthetic INI text only. The one
//! test that reads a real `GameSettings.ini` runs only when `R6_GAME_SETTINGS`
//! points at one, and prints none of its values.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use replay_analyzer::settings::{
    self, AspectSource, GameSettings, KEYS, Placement, SettingsHistory, Snapshot, UncertainMatch,
    WindowMode, Zoom, is_aim_field, locate_in, since_change, split_matches,
};
use replay_analyzer::{Match, PlayerMatchStats, ReadMode, Round};

const FIXTURE: &str = include_str!("fixtures/GameSettings.ini");

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

/// The fixture with one `Key=value` line replaced.
fn fixture_with(key: &str, value: &str) -> String {
    let mut found = false;
    let text: Vec<String> = FIXTURE
        .lines()
        .map(|line| {
            if line.split_once('=').is_some_and(|(k, _)| k == key) {
                found = true;
                format!("{key}={value}")
            } else {
                line.to_owned()
            }
        })
        .collect();
    assert!(found, "fixture has no {key}");
    text.join("\n")
}

fn snapshot(text: &str, taken: &str, modified: Option<&str>) -> Snapshot {
    Snapshot::from_bytes(text.as_bytes(), at(taken), modified.map(at))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

// Parsing.

#[test]
fn the_fixture_parses_into_every_field() {
    let s = GameSettings::parse(FIXTURE);
    let i = &s.input;
    assert_eq!(i.raw_input, Some(true));
    assert_eq!(i.invert_mouse_y, Some(false));
    assert_eq!(i.mouse_yaw_sensitivity, Some(10));
    assert_eq!(i.mouse_pitch_sensitivity, Some(12));
    assert_eq!(i.mouse_sensitivity_multiplier_unit, Some(0.02));
    assert_eq!(i.x_factor_aiming, Some(0.02));
    assert_eq!(i.aim_down_sights_mouse, Some(50));
    assert_eq!(i.ads_mouse_use_specific, Some(true));
    assert_eq!(i.ads_mouse_global, Some(40));
    let ads = [
        Zoom::X1,
        Zoom::X1_5,
        Zoom::X2,
        Zoom::X2_5,
        Zoom::X3,
        Zoom::X4,
        Zoom::X5,
        Zoom::X8,
        Zoom::X12,
    ];
    let mouse: Vec<u32> = ads.iter().map(|z| i.ads_mouse.get(*z).unwrap()).collect();
    assert_eq!(mouse, [31, 32, 33, 34, 35, 36, 37, 38, 39]);
    assert_eq!(i.ads_mouse_multiplier_unit, Some(0.02));
    assert_eq!(i.toggle_aim, Some(false));
    assert_eq!(i.toggle_lean, Some(true));

    let c = &i.controller;
    assert_eq!(c.invert_y, Some(false));
    assert_eq!(c.yaw_sensitivity, Some(40));
    assert_eq!(c.pitch_sensitivity, Some(30));
    assert_eq!(c.deadzone_left_stick, Some(12));
    assert_eq!(c.deadzone_right_stick, Some(14));
    assert_eq!(c.stick_rotation_curve, Some(1));
    assert_eq!(c.aim_down_sights, Some(45));
    assert_eq!(c.ads_use_specific, Some(false));
    assert_eq!(c.ads_global, Some(55));
    let pad: Vec<u32> = ads.iter().map(|z| c.ads.get(*z).unwrap()).collect();
    assert_eq!(pad, [51, 52, 53, 54, 55, 56, 57, 58, 59]);
    assert_eq!(c.ads_multiplier_unit, Some(0.02));

    let d = &s.display;
    assert_eq!(d.fov, Some(75.0));
    assert_eq!(d.aspect_ratio, Some(1));
    assert_eq!(d.resolution_width, Some(1920));
    assert_eq!(d.resolution_height, Some(1080));
    assert_eq!(d.refresh_rate, Some(144.0));
    assert_eq!(d.window_mode, Some(WindowMode::Borderless));
    assert_eq!(d.vsync, Some(0));
    assert_eq!(d.use_letterbox, Some(false));
    assert_eq!(d.fps_limit, Some(144));
    assert_eq!(d.nv_reflex, Some(1));
}

#[test]
fn every_whitelisted_key_is_in_the_fixture_and_lands_in_a_field() {
    let all = GameSettings::parse(FIXTURE).fields().len();
    assert_eq!(all, KEYS.len(), "one field per whitelisted key");
    for (section, key) in KEYS {
        // Dropping the key's line must clear exactly one field.
        let mut current = String::new();
        let without: Vec<&str> = FIXTURE
            .lines()
            .filter(|line| {
                if let Some(name) = line.strip_prefix('[') {
                    current = name.trim_end_matches(']').to_owned();
                }
                !(current == *section && line.split_once('=').is_some_and(|(k, _)| k == *key))
            })
            .collect();
        let fields = GameSettings::parse(&without.join("\n")).fields().len();
        assert_eq!(fields, all - 1, "[{section}] {key}");
    }
}

#[test]
fn nothing_outside_the_whitelist_reaches_the_output() {
    let json = serde_json::to_string(&GameSettings::parse(FIXTURE)).unwrap();
    for leaked in ["made-up", "region", "NOT-A-REAL", "5678", "16384", "Custom"] {
        assert!(!json.contains(leaked), "{leaked} leaked into {json}");
    }
    let lower = json.to_lowercase();
    for key in ["datacenter", "gpu", "hardware", "proxy", "volume"] {
        assert!(!lower.contains(key), "{key} leaked");
    }
}

#[test]
fn empty_text_and_missing_keys_give_none() {
    assert_eq!(GameSettings::parse(""), GameSettings::default());
    let s = GameSettings::parse("[INPUT]\nMouseYawSensitivity=7\n");
    assert_eq!(s.input.mouse_yaw_sensitivity, Some(7));
    assert_eq!(s.input.mouse_pitch_sensitivity, None);
    assert_eq!(s.display, Default::default());
    assert_eq!(s.fields().len(), 1);
}

#[test]
fn garbage_values_give_none_without_failing() {
    let text = "[INPUT]\n\
        MouseYawSensitivity=fast\n\
        MousePitchSensitivity=-3\n\
        MouseSensitivityMultiplierUnit=NaN\n\
        XFactorAiming=inf\n\
        RawInputMouseKeyboard=maybe\n\
        ADSMouseSensitivity1x=12.5\n\
        ADSMouseSensitivity2x=99999999999999999999\n\
        ADSMouseSensitivity3x=\n\
        [DISPLAY_SETTINGS]\n\
        DefaultFOV=wide\n\
        WindowMode=9\n\
        ResolutionWidth=0x780\n";
    assert_eq!(GameSettings::parse(text), GameSettings::default());
}

#[test]
fn tolerant_of_layout() {
    let text = "; comment\n# another\n\n\
        stray line without equals\n\
        MouseYawSensitivity=1\n\
        [ input ]\n\
        \t mouseyawsensitivity =  22  \n\
        MousePitchSensitivity=30.000000\n\
        ;MousePitchSensitivity=99\n\
        RawInputMouseKeyboard=TRUE\n\
        [UNKNOWN_SECTION]\n\
        MouseYawSensitivity=77\n\
        DefaultFOV=120\n\
        [display_settings]\n\
        DefaultFOV=70\n\
        DefaultFOV=80.5\n\
        [BROKEN\n\
        DefaultFOV=33\n";
    let s = GameSettings::parse(text);
    assert_eq!(s.input.mouse_yaw_sensitivity, Some(22));
    assert_eq!(s.input.mouse_pitch_sensitivity, Some(30));
    assert_eq!(s.input.raw_input, Some(true));
    assert_eq!(s.display.fov, Some(80.5), "the last duplicate wins");
}

#[test]
fn bom_crlf_and_utf16_read_the_same() {
    let expected = GameSettings::parse(FIXTURE);
    let crlf = FIXTURE.replace("\r\n", "\n").replace('\n', "\r\n");
    assert_eq!(GameSettings::parse(&crlf), expected);

    let mut bom = vec![0xef, 0xbb, 0xbf];
    bom.extend_from_slice(crlf.as_bytes());
    assert_eq!(GameSettings::from_bytes(&bom), expected);
    // A BOM that survived as a char in already decoded text.
    assert_eq!(GameSettings::parse(&format!("\u{feff}{crlf}")), expected);

    let mut le = vec![0xff, 0xfe];
    let mut be = vec![0xfe, 0xff];
    for unit in crlf.encode_utf16() {
        le.extend_from_slice(&unit.to_le_bytes());
        be.extend_from_slice(&unit.to_be_bytes());
    }
    assert_eq!(GameSettings::from_bytes(&le), expected);
    assert_eq!(GameSettings::from_bytes(&be), expected);

    // Invalid UTF-8 is replaced, not fatal.
    let mut bad = b"[INPUT]\nRumble=\xff\xfe\n".to_vec();
    bad.extend_from_slice(b"MouseYawSensitivity=9\n");
    assert_eq!(
        GameSettings::from_bytes(&bad).input.mouse_yaw_sensitivity,
        Some(9)
    );
}

#[test]
fn serde_round_trips_in_camel_case() {
    let s = GameSettings::parse(FIXTURE);
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["input"]["mouseYawSensitivity"], 10);
    assert_eq!(json["input"]["adsMouse"]["x1_5"], 32);
    assert_eq!(json["input"]["controller"]["adsGlobal"], 55);
    assert_eq!(json["display"]["windowMode"], "borderless");
    assert_eq!(serde_json::from_value::<GameSettings>(json).unwrap(), s);
    // Stored data from a version with fewer fields still loads.
    let old: GameSettings = serde_json::from_str(r#"{"display":{"fov":60.0}}"#).unwrap();
    assert_eq!(old.display.fov, Some(60.0));
}

// Derived values.

#[test]
fn cm_per_360_follows_the_documented_formula() {
    let s = GameSettings::parse(FIXTURE);
    // 10 * 0.02 * 0.005 = 0.001 rad per count.
    let counts = s.input.counts_per_360().unwrap();
    assert!(close(counts, std::f64::consts::TAU / 0.001));
    let cm = s.input.cm_per_360(800.0).unwrap();
    assert!(close(cm, counts / 800.0 * 2.54));
    assert!((cm - 19.949).abs() < 0.001, "{cm}");
    // Doubling DPI halves the distance; a bad DPI gives nothing.
    assert!(close(s.input.cm_per_360(1600.0).unwrap(), cm / 2.0));
    assert_eq!(s.input.cm_per_360(0.0), None);
    assert_eq!(s.input.cm_per_360(f64::NAN), None);

    let zero = GameSettings::parse(&fixture_with("MouseYawSensitivity", "0"));
    assert_eq!(zero.input.cm_per_360(800.0), None);
    assert_eq!(GameSettings::default().input.cm_per_360(800.0), None);
}

#[test]
fn ads_slider_respects_use_specific() {
    let s = GameSettings::parse(FIXTURE);
    assert_eq!(s.input.ads_mouse_slider(Zoom::X2_5), Some(34));
    assert!(close(
        s.input.ads_mouse_multiplier(Zoom::X2_5).unwrap(),
        0.68
    ));
    let global = GameSettings::parse(&fixture_with("ADSMouseUseSpecific", "0"));
    assert_eq!(global.input.ads_mouse_slider(Zoom::X2_5), Some(40));
    assert!(close(
        global.input.ads_mouse_multiplier(Zoom::X12).unwrap(),
        0.8
    ));
    assert_eq!(
        GameSettings::default().input.ads_mouse_slider(Zoom::X1),
        None
    );
}

#[test]
fn horizontal_fov_from_vertical_and_aspect() {
    let s = GameSettings::parse(&fixture_with("DefaultFOV", "90"));
    // tan(45 deg) = 1, so horizontal = 2 * atan(aspect).
    assert!(close(s.display.horizontal_fov(1.0).unwrap(), 90.0));
    let wide = s.display.horizontal_fov(16.0 / 9.0).unwrap();
    assert!(close(wide, 2.0 * (16.0f64 / 9.0).atan().to_degrees()));
    assert!((wide - 121.28).abs() < 0.01, "{wide}");

    let aspect = s.display.aspect().unwrap();
    assert!(close(aspect.ratio, 16.0 / 9.0));
    assert_eq!(aspect.source, AspectSource::Resolution);
    assert!(close(s.display.horizontal_fov_auto().unwrap(), wide));

    assert_eq!(s.display.horizontal_fov(0.0), None);
    assert_eq!(GameSettings::default().display.horizontal_fov(1.5), None);
    let flat = GameSettings::parse(&fixture_with("DefaultFOV", "180"));
    assert_eq!(flat.display.horizontal_fov(1.5), None);
}

#[test]
fn aspect_is_only_derived_when_the_file_is_enough() {
    let display = GameSettings::parse(&fixture_with("AspectRatio", "0")).display;
    assert_eq!(
        display.aspect().unwrap().source,
        AspectSource::ResolutionAssumedDisplay
    );
    // A menu entry: the file does not say which ratio it is.
    let menu = GameSettings::parse(&fixture_with("AspectRatio", "4")).display;
    assert_eq!(menu.aspect(), None);
    assert_eq!(menu.horizontal_fov_auto(), None);
    // Autodetected resolution.
    let auto = GameSettings::parse(&fixture_with("ResolutionHeight", "0")).display;
    assert_eq!(auto.aspect(), None);
}

// Locating and reading.

#[test]
fn locate_builds_one_candidate_per_documents_folder() {
    let id = "0a1b2c3d-0000-4000-8000-00000000abcd";
    let docs = [
        PathBuf::from("C:/Users/Someone/Documents"),
        PathBuf::from("C:/Users/Someone/OneDrive/Documents"),
    ];
    let found = locate_in(&docs, id);
    assert_eq!(found.len(), 2);
    for (path, doc) in found.iter().zip(&docs) {
        let expected = doc
            .join("My Games")
            .join("Rainbow Six - Siege")
            .join(id)
            .join("GameSettings.ini");
        assert_eq!(path, &expected);
    }
    assert!(locate_in::<&Path>(&[], id).is_empty());
}

#[test]
fn locate_refuses_ids_that_are_not_profile_ids() {
    let docs = [PathBuf::from("C:/Users/Someone/Documents")];
    for id in [
        "",
        "..",
        "../other",
        "a/b",
        "a\\b",
        "C:",
        "id with space",
        "a.b",
    ] {
        assert!(locate_in(&docs, id).is_empty(), "{id:?}");
        assert!(settings::locate(id).is_empty(), "{id:?}");
    }
}

#[test]
fn locate_from_the_environment_only_builds_paths() {
    let id = "0a1b2c3d-0000-4000-8000-00000000abcd";
    // Whatever the machine's variables are, every candidate ends the same
    // way, and none of them has to exist.
    for path in settings::locate(id) {
        assert!(
            path.ends_with(
                Path::new("Documents/My Games/Rainbow Six - Siege")
                    .join(id)
                    .join("GameSettings.ini")
            )
        );
    }
}

#[test]
fn read_and_take_read_a_file_and_leave_it_alone() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/GameSettings.ini");
    let before = std::fs::read(&path).unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();

    let read = settings::read(&path).unwrap();
    assert_eq!(read, GameSettings::from_bytes(&before));
    let snap = Snapshot::take(&path).unwrap();
    assert_eq!(snap.settings, read);
    assert_eq!(snap.sha256, replay_analyzer::file::sha256(&before));
    assert_eq!(snap.file_modified, Some(DateTime::<Utc>::from(modified)));
    assert_eq!(snap.last_seen, snap.taken_at);

    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified
    );
    assert!(settings::read(path.with_file_name("missing.ini")).is_err());
}

// History.

/// Three aim settings over time, with a non-aim change in the middle one:
/// yaw 10 (file written 1 Sep, seen 10 Sep and 12 Sep), yaw 14 (seen 20 Sep,
/// file written 15 Sep), then a v-sync change (22 Sep), then FOV 84 (seen
/// 30 Sep with a modified time that is no use).
fn history() -> SettingsHistory {
    let yaw14 = fixture_with("MouseYawSensitivity", "14");
    let vsync = yaw14.replace("VSync=0", "VSync=1");
    let fov = vsync.replace("DefaultFOV=75.000000", "DefaultFOV=84.000000");
    let mut h = SettingsHistory::default();
    assert!(h.record(snapshot(
        FIXTURE,
        "2026-09-10T12:00:00Z",
        Some("2026-09-01T08:00:00Z")
    )));
    assert!(!h.record(snapshot(
        FIXTURE,
        "2026-09-12T12:00:00Z",
        Some("2026-09-11T08:00:00Z")
    )));
    assert!(h.record(snapshot(
        &yaw14,
        "2026-09-20T12:00:00Z",
        Some("2026-09-15T18:30:00Z")
    )));
    assert!(h.record(snapshot(
        &vsync,
        "2026-09-22T12:00:00Z",
        Some("2026-09-21T09:00:00Z")
    )));
    // Modified before the previous snapshot: it cannot date this change.
    assert!(h.record(snapshot(
        &fov,
        "2026-09-30T12:00:00Z",
        Some("2026-09-05T00:00:00Z")
    )));
    h
}

#[test]
fn record_appends_only_when_whitelisted_values_change() {
    let h = history();
    assert_eq!(h.snapshots.len(), 4);
    assert_eq!(h.snapshots[0].taken_at, at("2026-09-10T12:00:00Z"));
    assert_eq!(h.snapshots[0].last_seen, at("2026-09-12T12:00:00Z"));
    assert_eq!(h.latest().unwrap().display.fov, Some(84.0));

    // A change to a key outside the whitelist changes the hash, not the
    // history.
    let mut h = SettingsHistory::default();
    let other = FIXTURE.replace("MasterVolume=80.000000", "MasterVolume=20.000000");
    let first = snapshot(FIXTURE, "2026-09-10T12:00:00Z", None);
    let second = snapshot(&other, "2026-09-11T12:00:00Z", None);
    assert_ne!(first.sha256, second.sha256);
    assert!(h.record(first));
    assert!(!h.record(second));
    assert_eq!(h.snapshots.len(), 1);
    assert_eq!(h.snapshots[0].last_seen, at("2026-09-11T12:00:00Z"));

    // Older than what is already recorded: ignored, changed or not.
    let yaw = fixture_with("MouseYawSensitivity", "3");
    assert!(!h.record(snapshot(&yaw, "2026-09-10T18:00:00Z", None)));
    assert!(!h.record(snapshot(FIXTURE, "2026-09-01T00:00:00Z", None)));
    assert_eq!(h.snapshots.len(), 1);
    assert_eq!(h.snapshots[0].last_seen, at("2026-09-11T12:00:00Z"));
}

#[test]
fn changes_list_each_field_with_its_window() {
    let changes = history().changes();
    let brief: Vec<(&str, String, String, bool)> = changes
        .iter()
        .map(|c| {
            let show = |v: &Option<serde_json::Value>| v.as_ref().unwrap().to_string();
            (c.field.as_str(), show(&c.from), show(&c.to), c.aim)
        })
        .collect();
    assert_eq!(
        brief,
        [
            (
                "input.mouseYawSensitivity",
                "10".to_owned(),
                "14".to_owned(),
                true
            ),
            ("display.vsync", "0".to_owned(), "1".to_owned(), false),
            ("display.fov", "75.0".to_owned(), "84.0".to_owned(), true),
        ]
    );
    // After the last sighting of the old value, no later than the file time.
    assert_eq!(changes[0].not_before, at("2026-09-12T12:00:00Z"));
    assert_eq!(changes[0].at, at("2026-09-15T18:30:00Z"));
    assert_eq!(changes[1].not_before, at("2026-09-20T12:00:00Z"));
    assert_eq!(changes[1].at, at("2026-09-21T09:00:00Z"));
    // A useless modified time falls back to the snapshot.
    assert_eq!(changes[2].not_before, at("2026-09-22T12:00:00Z"));
    assert_eq!(changes[2].at, at("2026-09-30T12:00:00Z"));
}

#[test]
fn a_key_that_appears_or_disappears_is_a_change() {
    let without: String = FIXTURE
        .lines()
        .filter(|l| !l.starts_with("XFactorAiming="))
        .collect::<Vec<_>>()
        .join("\n");
    let mut h = SettingsHistory::default();
    h.record(snapshot(FIXTURE, "2026-09-10T12:00:00Z", None));
    h.record(snapshot(&without, "2026-09-11T12:00:00Z", None));
    h.record(snapshot(FIXTURE, "2026-09-12T12:00:00Z", None));
    let changes = h.changes();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].field, "input.xFactorAiming");
    assert_eq!(
        (changes[0].from.is_some(), changes[0].to.is_some()),
        (true, false)
    );
    assert_eq!(
        (changes[1].from.is_some(), changes[1].to.is_some()),
        (false, true)
    );
}

#[test]
fn periods_span_constant_aim_settings() {
    let periods = history().periods();
    assert_eq!(periods.len(), 3, "the v-sync change starts no period");
    let spans: Vec<_> = periods
        .iter()
        .map(|p| (p.start, p.last_seen, p.open))
        .collect();
    assert_eq!(
        spans,
        [
            (
                at("2026-09-01T08:00:00Z"),
                at("2026-09-12T12:00:00Z"),
                false
            ),
            (
                at("2026-09-15T18:30:00Z"),
                at("2026-09-22T12:00:00Z"),
                false
            ),
            (at("2026-09-30T12:00:00Z"), at("2026-09-30T12:00:00Z"), true),
        ]
    );
    assert_eq!(periods[0].settings.input.mouse_yaw_sensitivity, Some(10));
    assert_eq!(periods[1].settings.input.mouse_yaw_sensitivity, Some(14));
    assert_eq!(periods[2].settings.display.fov, Some(84.0));
    assert_eq!(
        periods[1].settings.display.vsync, None,
        "non-aim fields are cleared"
    );
    assert!(SettingsHistory::default().periods().is_empty());

    // Without a modified time the first period starts at its snapshot.
    let mut h = SettingsHistory::default();
    h.record(snapshot(FIXTURE, "2026-09-10T12:00:00Z", None));
    assert_eq!(h.periods()[0].start, at("2026-09-10T12:00:00Z"));
    // A modified time after the read (clock skew) is not believed either.
    let mut h = SettingsHistory::default();
    h.record(snapshot(
        FIXTURE,
        "2026-09-10T12:00:00Z",
        Some("2026-09-11T00:00:00Z"),
    ));
    assert_eq!(h.periods()[0].start, at("2026-09-10T12:00:00Z"));
}

#[test]
fn aim_fields_are_sensitivity_and_view_not_toggles_or_refresh() {
    for field in GameSettings::parse(FIXTURE).fields().keys() {
        let expected = !matches!(
            field.as_str(),
            "input.toggleAim"
                | "input.toggleLean"
                | "display.refreshRate"
                | "display.windowMode"
                | "display.vsync"
                | "display.fpsLimit"
                | "display.nvReflex"
        );
        assert_eq!(is_aim_field(field), expected, "{field}");
    }
    // `aim()` clears exactly the fields `is_aim_field` rejects.
    let aim = GameSettings::parse(FIXTURE).aim();
    assert!(aim.fields().keys().all(|f| is_aim_field(f)));
    assert_eq!(aim.fields().len(), KEYS.len() - 7);
}

/// Match start times against `history()`, with where each belongs.
const STARTS: [(&str, Placement); 9] = [
    ("2026-08-20T20:00:00Z", Placement::BeforeHistory),
    ("2026-09-01T08:00:00Z", Placement::Period(0)),
    ("2026-09-12T12:00:00Z", Placement::Period(0)),
    ("2026-09-13T20:00:00Z", Placement::AfterPeriod(0)),
    ("2026-09-15T18:30:00Z", Placement::Period(1)),
    ("2026-09-21T20:00:00Z", Placement::Period(1)),
    ("2026-09-25T20:00:00Z", Placement::AfterPeriod(1)),
    ("2026-09-30T12:00:00Z", Placement::Period(2)),
    ("2026-12-24T20:00:00Z", Placement::Period(2)),
];

#[test]
fn matches_split_into_periods_with_change_windows_left_uncertain() {
    let h = history();
    for (start, expected) in STARTS {
        assert_eq!(h.place(at(start)), expected, "{start}");
    }
    let split = split_matches(&h, STARTS.iter().map(|(s, _)| at(s)));
    assert_eq!(split.before_history, [0]);
    assert_eq!(split.periods, [vec![1, 2], vec![4, 5], vec![7, 8]]);
    assert_eq!(
        split.uncertain,
        [
            UncertainMatch {
                index: 3,
                after_period: 0
            },
            UncertainMatch {
                index: 6,
                after_period: 1
            },
        ]
    );

    // Input order does not matter; indices follow the input.
    let reversed = split_matches(&h, STARTS.iter().rev().map(|(s, _)| at(s)));
    assert_eq!(reversed.periods, [vec![6, 7], vec![3, 4], vec![0, 1]]);

    let empty = split_matches(&SettingsHistory::default(), [at("2026-09-01T00:00:00Z")]);
    assert_eq!(empty.before_history, [0]);
    assert!(empty.periods.is_empty());
}

#[test]
fn since_change_sums_headshots_per_period() {
    let h = history();
    // Kills and headshots per start time of `STARTS`.
    let numbers = [
        (9, 9),
        (10, 2),
        (5, 1),
        (7, 7),
        (8, 4),
        (12, 6),
        (3, 3),
        (0, 0),
        (4, 3),
    ];
    let stats: Vec<PlayerMatchStats> = numbers
        .iter()
        .map(|&(kills, headshots)| PlayerMatchStats {
            kills,
            headshots,
            ..PlayerMatchStats::default()
        })
        .collect();
    let report = since_change(&h, STARTS.iter().map(|(s, _)| at(s)).zip(&stats));
    assert_eq!(report.before_history, 1);
    assert_eq!(report.uncertain, 2);
    let rows: Vec<_> = report
        .periods
        .iter()
        .map(|p| (p.matches, p.kills, p.headshots))
        .collect();
    assert_eq!(rows, [(2, 15, 3), (2, 20, 10), (2, 4, 3)]);
    assert!(close(report.periods[0].headshot_percentage, 20.0));
    assert!(close(report.periods[1].headshot_percentage, 50.0));
    assert!(close(report.periods[2].headshot_percentage, 75.0));
    assert_eq!(report.periods[2].period, h.periods()[2]);

    // A period without matches is still listed, at zero.
    let none = since_change(&h, std::iter::empty());
    assert_eq!(none.periods.len(), 3);
    assert_eq!(none.periods[0].headshot_percentage, 0.0);
}

#[test]
fn history_round_trips_through_json() {
    let h = history();
    let json = serde_json::to_string(&h).unwrap();
    assert!(
        json.contains(r#""takenAt":"2026-09-10T12:00:00Z""#),
        "{json}"
    );
    assert!(json.contains(r#""fileModified":"2026-09-15T18:30:00Z""#));
    let back: SettingsHistory = serde_json::from_str(&json).unwrap();
    assert_eq!(back, h);
    assert_eq!(back.changes(), h.changes());

    // Sub-second times survive, and a missing modified time stays missing.
    let precise = at("2026-09-10T12:00:00.123456Z");
    let mut h = SettingsHistory::default();
    h.record(Snapshot::from_bytes(FIXTURE.as_bytes(), precise, None));
    h.record(Snapshot::from_bytes(
        b"",
        precise + Duration::milliseconds(5),
        None,
    ));
    let back: SettingsHistory = serde_json::from_str(&serde_json::to_string(&h).unwrap()).unwrap();
    assert_eq!(back, h);
    assert_eq!(
        serde_json::from_str::<SettingsHistory>("{}").unwrap(),
        SettingsHistory::default()
    );
}

#[test]
fn recording_stats_come_from_the_recording_player() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = match std::env::var_os("R6_TEST_DATA") {
        Some(dir) => PathBuf::from(dir),
        None => root.join("test_recordings"),
    };
    let file = dir.join("valid/Y11S3/custom_1.rec");
    if !file.is_file() {
        eprintln!("skipping: no {}", file.display());
        return;
    }
    let m = Match {
        rounds: vec![Round::open(&file, ReadMode::Full).unwrap()],
        folder: None,
    };
    let summary = m.summary().unwrap();
    match settings::recording_stats(&m) {
        Some((start, stats)) => {
            assert!(!summary.recording.spectator);
            assert_eq!(start, summary.start_time);
            assert_eq!(Some(&stats.username), summary.recording.username.as_ref());
            assert!(stats.headshots <= stats.kills);
        }
        None => assert!(
            summary.recording.spectator
                || summary.recording.username.is_none()
                || m.player_stats()
                    .iter()
                    .all(|s| Some(&s.username) != summary.recording.username.as_ref())
        ),
    }
    assert!(settings::recording_stats(&Match::default()).is_none());
}

// The real file, only on request.

#[test]
fn a_real_settings_file_parses_when_one_is_pointed_at() {
    let Some(path) = std::env::var_os("R6_GAME_SETTINGS") else {
        eprintln!("skipping: R6_GAME_SETTINGS is not set");
        return;
    };
    // Assert without the values: a failure must not print them either.
    let s = settings::read(PathBuf::from(path)).expect("the file reads");
    let fov = s.display.fov.expect("DefaultFOV is present");
    assert!((40.0..=120.0).contains(&fov), "FOV is out of range");
    let yaw = s.input.mouse_yaw_sensitivity.expect("yaw is present");
    let pitch = s.input.mouse_pitch_sensitivity.expect("pitch is present");
    assert!((1..=100).contains(&yaw), "yaw is out of range");
    assert!((1..=100).contains(&pitch), "pitch is out of range");
    let unit = s
        .input
        .mouse_sensitivity_multiplier_unit
        .expect("unit is present");
    assert!(unit > 0.0 && unit < 1.0, "multiplier unit is out of range");
    assert!(s.input.counts_per_360().is_some(), "counts per 360 derive");
    assert!(s.fields().len() > KEYS.len() / 2, "most keys are present");
}
