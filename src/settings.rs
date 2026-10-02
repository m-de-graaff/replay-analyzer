//! Read-only reader for the game's `GameSettings.ini` (sensitivity, field of
//! view, resolution) and a history of how those settings changed, so match
//! stats can be compared before and after a change.
//!
//! Nothing in a replay records sensitivity or field of view, so this is the
//! one part of the crate that looks outside `.rec` files. That makes consent
//! the caller's job, and the API is shaped to keep it there:
//!
//! - Nothing here discovers or opens the settings file on its own. No other
//!   module calls into this one.
//! - [`locate`] and [`locate_in`] only build candidate paths. They do not
//!   touch the filesystem, not even to check that a path exists.
//! - [`read`] and [`Snapshot::take`] open the one path they are given,
//!   read-only. No function in this module writes, creates, renames or
//!   deletes anything.
//! - Only a fixed list of keys is read ([`KEYS`]). Everything else in the
//!   file, such as hardware ids and the `[ONLINE]` section, is skipped and has
//!   no field to land in.
//!
//! ```no_run
//! use replay_analyzer::settings::{self, SettingsHistory, Snapshot};
//!
//! // The app asked the user first, then:
//! let path = &settings::locate("2f0c0a52-0000-4000-8000-000000000000")[0];
//! let mut history = SettingsHistory::default();
//! history.record(Snapshot::take(path)?);
//! # Ok::<(), replay_analyzer::Error>(())
//! ```
//!
//! # When a change happened
//!
//! The file says what the settings are, not when they were set. A change is
//! seen as a difference between two snapshots, so it is dated to a window:
//! after the last snapshot that still showed the old values, and no later
//! than the file's modified time (or the new snapshot, when the modified time
//! is missing or outside that window). [`Period`]s leave that window out, and
//! [`split_matches`] reports matches that started inside it as uncertain
//! instead of guessing a side. Snapshot often (at every import) to keep the
//! windows short.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::error::Result;
use crate::matches::Match;
use crate::stats::{PlayerMatchStats, headshot_percentage};

/// The file's name inside the profile folder.
pub const FILE_NAME: &str = "GameSettings.ini";

/// Every `(section, key)` this module reads. Nothing else leaves the file.
pub const KEYS: &[(&str, &str)] = &[
    ("DISPLAY", "FPSLimit"),
    ("DISPLAY", "NVReflex"),
    ("DISPLAY_SETTINGS", "ResolutionWidth"),
    ("DISPLAY_SETTINGS", "ResolutionHeight"),
    ("DISPLAY_SETTINGS", "RefreshRate"),
    ("DISPLAY_SETTINGS", "WindowMode"),
    ("DISPLAY_SETTINGS", "AspectRatio"),
    ("DISPLAY_SETTINGS", "VSync"),
    ("DISPLAY_SETTINGS", "UseLetterbox"),
    ("DISPLAY_SETTINGS", "DefaultFOV"),
    ("INPUT", "RawInputMouseKeyboard"),
    ("INPUT", "InvertMouseAxisY"),
    ("INPUT", "MouseYawSensitivity"),
    ("INPUT", "MousePitchSensitivity"),
    ("INPUT", "MouseSensitivityMultiplierUnit"),
    ("INPUT", "XFactorAiming"),
    ("INPUT", "AimDownSightsMouse"),
    ("INPUT", "ADSMouseUseSpecific"),
    ("INPUT", "ADSMouseSensitivityGlobal"),
    ("INPUT", "ADSMouseSensitivity1x"),
    ("INPUT", "ADSMouseSensitivity1xHalf"),
    ("INPUT", "ADSMouseSensitivity2x"),
    ("INPUT", "ADSMouseSensitivity2xHalf"),
    ("INPUT", "ADSMouseSensitivity3x"),
    ("INPUT", "ADSMouseSensitivity4x"),
    ("INPUT", "ADSMouseSensitivity5x"),
    ("INPUT", "ADSMouseSensitivity8x"),
    ("INPUT", "ADSMouseSensitivity12x"),
    ("INPUT", "ADSMouseMultiplierUnit"),
    ("INPUT", "ToggleAim"),
    ("INPUT", "ToggleLean"),
    ("INPUT", "InvertAxisY"),
    ("INPUT", "YawSensitivity"),
    ("INPUT", "PitchSensitivity"),
    ("INPUT", "DeadzoneLeftStick"),
    ("INPUT", "DeadzoneRightStick"),
    ("INPUT", "ControllerStickRotationCurve"),
    ("INPUT", "AimDownSights"),
    ("INPUT", "ADSGamepadUseSpecific"),
    ("INPUT", "ADSGamepadSensitivityGlobal"),
    ("INPUT", "ADSGamepadSensitivity1x"),
    ("INPUT", "ADSGamepadSensitivity1xHalf"),
    ("INPUT", "ADSGamepadSensitivity2x"),
    ("INPUT", "ADSGamepadSensitivity2xHalf"),
    ("INPUT", "ADSGamepadSensitivity3x"),
    ("INPUT", "ADSGamepadSensitivity4x"),
    ("INPUT", "ADSGamepadSensitivity5x"),
    ("INPUT", "ADSGamepadSensitivity8x"),
    ("INPUT", "ADSGamepadSensitivity12x"),
    ("INPUT", "ADSGamepadMultiplierUnit"),
];

/// The whitelisted settings. Every field is `None` when its key is missing
/// or its value does not parse.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GameSettings {
    pub input: InputSettings,
    pub display: DisplaySettings,
}

/// `[INPUT]`: mouse settings here, controller settings in `controller`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InputSettings {
    /// `RawInputMouseKeyboard`.
    pub raw_input: Option<bool>,
    /// `InvertMouseAxisY`.
    pub invert_mouse_y: Option<bool>,
    /// `MouseYawSensitivity`: the horizontal slider, 1 to 100.
    pub mouse_yaw_sensitivity: Option<u32>,
    /// `MousePitchSensitivity`: the vertical slider, 1 to 100.
    pub mouse_pitch_sensitivity: Option<u32>,
    /// `MouseSensitivityMultiplierUnit`: what one slider step is worth. Only
    /// editable in the file; the game's default is 0.02.
    pub mouse_sensitivity_multiplier_unit: Option<f64>,
    /// `XFactorAiming`: the multiplier unit of the single ADS slider the game
    /// had before per-zoom sliders (Y5S3).
    pub x_factor_aiming: Option<f64>,
    /// `AimDownSightsMouse`: the single ADS slider from before Y5S3, still
    /// written.
    pub aim_down_sights_mouse: Option<u32>,
    /// `ADSMouseUseSpecific`: per-zoom sliders (`ads_mouse`) are in use
    /// instead of `ads_mouse_global`.
    pub ads_mouse_use_specific: Option<bool>,
    /// `ADSMouseSensitivityGlobal`.
    pub ads_mouse_global: Option<u32>,
    /// `ADSMouseSensitivity1x` .. `ADSMouseSensitivity12x`.
    pub ads_mouse: AdsSensitivities,
    /// `ADSMouseMultiplierUnit`: what one ADS slider step is worth.
    pub ads_mouse_multiplier_unit: Option<f64>,
    /// `ToggleAim`: aiming is a toggle instead of a hold.
    pub toggle_aim: Option<bool>,
    /// `ToggleLean`.
    pub toggle_lean: Option<bool>,
    pub controller: ControllerSettings,
}

/// One ADS slider per sight magnification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdsSensitivities {
    pub x1: Option<u32>,
    /// 1.5x (`...1xHalf`).
    pub x1_5: Option<u32>,
    pub x2: Option<u32>,
    /// 2.5x (`...2xHalf`).
    pub x2_5: Option<u32>,
    pub x3: Option<u32>,
    pub x4: Option<u32>,
    pub x5: Option<u32>,
    pub x8: Option<u32>,
    pub x12: Option<u32>,
}

/// A sight magnification with its own ADS slider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Zoom {
    X1,
    X1_5,
    X2,
    X2_5,
    X3,
    X4,
    X5,
    X8,
    X12,
}

impl AdsSensitivities {
    pub fn get(&self, zoom: Zoom) -> Option<u32> {
        match zoom {
            Zoom::X1 => self.x1,
            Zoom::X1_5 => self.x1_5,
            Zoom::X2 => self.x2,
            Zoom::X2_5 => self.x2_5,
            Zoom::X3 => self.x3,
            Zoom::X4 => self.x4,
            Zoom::X5 => self.x5,
            Zoom::X8 => self.x8,
            Zoom::X12 => self.x12,
        }
    }
}

/// The controller half of `[INPUT]`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ControllerSettings {
    /// `InvertAxisY`.
    pub invert_y: Option<bool>,
    /// `YawSensitivity`.
    pub yaw_sensitivity: Option<u32>,
    /// `PitchSensitivity`.
    pub pitch_sensitivity: Option<u32>,
    /// `DeadzoneLeftStick`, percent.
    pub deadzone_left_stick: Option<u32>,
    /// `DeadzoneRightStick`, percent.
    pub deadzone_right_stick: Option<u32>,
    /// `ControllerStickRotationCurve`: the game's index of the response curve.
    pub stick_rotation_curve: Option<u32>,
    /// `AimDownSights`: the single ADS slider from before Y5S3.
    pub aim_down_sights: Option<u32>,
    /// `ADSGamepadUseSpecific`.
    pub ads_use_specific: Option<bool>,
    /// `ADSGamepadSensitivityGlobal`.
    pub ads_global: Option<u32>,
    /// `ADSGamepadSensitivity1x` .. `ADSGamepadSensitivity12x`.
    pub ads: AdsSensitivities,
    /// `ADSGamepadMultiplierUnit`.
    pub ads_multiplier_unit: Option<f64>,
}

/// `[DISPLAY_SETTINGS]`, plus the frame limit and Reflex mode of `[DISPLAY]`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DisplaySettings {
    /// `DefaultFOV`: the vertical field of view in degrees (the file's own
    /// comment says so).
    pub fov: Option<f64>,
    /// `AspectRatio`, as the file's code: 0 the display's, 1 the
    /// resolution's, 2 and up the entries of the game's aspect ratio menu.
    /// Which ratio each menu entry is, is not in the file; see
    /// [`DisplaySettings::aspect`].
    pub aspect_ratio: Option<u32>,
    /// `ResolutionWidth`; 0 means autodetect.
    pub resolution_width: Option<u32>,
    /// `ResolutionHeight`; 0 means autodetect.
    pub resolution_height: Option<u32>,
    /// `RefreshRate` in Hz; 0 lets DirectX pick.
    pub refresh_rate: Option<f64>,
    /// `WindowMode`.
    pub window_mode: Option<WindowMode>,
    /// `VSync`: 0 off, 1 every frame, 2 every second frame.
    pub vsync: Option<u32>,
    /// `UseLetterbox`.
    pub use_letterbox: Option<bool>,
    /// `[DISPLAY] FPSLimit`; below 30 means no limit.
    pub fps_limit: Option<u32>,
    /// `[DISPLAY] NVReflex`: 0 off, 1 on, 2 on with boost.
    pub nv_reflex: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowMode {
    Fullscreen,
    Windowed,
    Borderless,
}

impl GameSettings {
    /// Reads the whitelisted keys out of INI text. Never fails: comments,
    /// unknown sections and keys, and values that do not parse are skipped.
    /// Section and key names match case-insensitively; when a key repeats,
    /// the last one wins.
    pub fn parse(text: &str) -> Self {
        let ini = Ini::new(text);
        let ads = |prefix: &str| AdsSensitivities {
            x1: ini.int("input", &format!("{prefix}1x")),
            x1_5: ini.int("input", &format!("{prefix}1xHalf")),
            x2: ini.int("input", &format!("{prefix}2x")),
            x2_5: ini.int("input", &format!("{prefix}2xHalf")),
            x3: ini.int("input", &format!("{prefix}3x")),
            x4: ini.int("input", &format!("{prefix}4x")),
            x5: ini.int("input", &format!("{prefix}5x")),
            x8: ini.int("input", &format!("{prefix}8x")),
            x12: ini.int("input", &format!("{prefix}12x")),
        };
        let input = InputSettings {
            raw_input: ini.flag("input", "RawInputMouseKeyboard"),
            invert_mouse_y: ini.flag("input", "InvertMouseAxisY"),
            mouse_yaw_sensitivity: ini.int("input", "MouseYawSensitivity"),
            mouse_pitch_sensitivity: ini.int("input", "MousePitchSensitivity"),
            mouse_sensitivity_multiplier_unit: ini.float("input", "MouseSensitivityMultiplierUnit"),
            x_factor_aiming: ini.float("input", "XFactorAiming"),
            aim_down_sights_mouse: ini.int("input", "AimDownSightsMouse"),
            ads_mouse_use_specific: ini.flag("input", "ADSMouseUseSpecific"),
            ads_mouse_global: ini.int("input", "ADSMouseSensitivityGlobal"),
            ads_mouse: ads("ADSMouseSensitivity"),
            ads_mouse_multiplier_unit: ini.float("input", "ADSMouseMultiplierUnit"),
            toggle_aim: ini.flag("input", "ToggleAim"),
            toggle_lean: ini.flag("input", "ToggleLean"),
            controller: ControllerSettings {
                invert_y: ini.flag("input", "InvertAxisY"),
                yaw_sensitivity: ini.int("input", "YawSensitivity"),
                pitch_sensitivity: ini.int("input", "PitchSensitivity"),
                deadzone_left_stick: ini.int("input", "DeadzoneLeftStick"),
                deadzone_right_stick: ini.int("input", "DeadzoneRightStick"),
                stick_rotation_curve: ini.int("input", "ControllerStickRotationCurve"),
                aim_down_sights: ini.int("input", "AimDownSights"),
                ads_use_specific: ini.flag("input", "ADSGamepadUseSpecific"),
                ads_global: ini.int("input", "ADSGamepadSensitivityGlobal"),
                ads: ads("ADSGamepadSensitivity"),
                ads_multiplier_unit: ini.float("input", "ADSGamepadMultiplierUnit"),
            },
        };
        let display = DisplaySettings {
            fov: ini.float("display_settings", "DefaultFOV"),
            aspect_ratio: ini.int("display_settings", "AspectRatio"),
            resolution_width: ini.int("display_settings", "ResolutionWidth"),
            resolution_height: ini.int("display_settings", "ResolutionHeight"),
            refresh_rate: ini.float("display_settings", "RefreshRate"),
            window_mode: match ini.int("display_settings", "WindowMode") {
                Some(0) => Some(WindowMode::Fullscreen),
                Some(1) => Some(WindowMode::Windowed),
                Some(2) => Some(WindowMode::Borderless),
                _ => None,
            },
            vsync: ini.int("display_settings", "VSync"),
            use_letterbox: ini.flag("display_settings", "UseLetterbox"),
            fps_limit: ini.int("display", "FPSLimit"),
            nv_reflex: ini.int("display", "NVReflex"),
        };
        Self { input, display }
    }

    /// [`GameSettings::parse`] over a file's bytes: UTF-8 with or without a
    /// byte order mark, or UTF-16 with one. Invalid sequences are replaced,
    /// not rejected.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::parse(&decode(bytes))
    }

    /// The settings as `path -> value`, one entry per field that is set, with
    /// paths as they serialize (`input.mouseYawSensitivity`,
    /// `input.adsMouse.x1_5`, `display.fov`).
    pub fn fields(&self) -> BTreeMap<String, Value> {
        let mut out = BTreeMap::new();
        if let Ok(value) = serde_json::to_value(self) {
            flatten("", &value, &mut out);
        }
        out
    }

    /// The same settings with everything that is not an aim setting
    /// ([`is_aim_field`]) cleared.
    pub fn aim(&self) -> Self {
        let mut aim = self.clone();
        aim.input.toggle_aim = None;
        aim.input.toggle_lean = None;
        aim.display.refresh_rate = None;
        aim.display.window_mode = None;
        aim.display.vsync = None;
        aim.display.fps_limit = None;
        aim.display.nv_reflex = None;
        aim
    }
}

/// Whether a field path from [`GameSettings::fields`] is an aim setting: one
/// that changes how far the view turns for a hand movement, or how large
/// targets are on screen. Those are every sensitivity, multiplier, deadzone,
/// curve, inversion and raw input under `input`, and `display`'s field of
/// view, aspect ratio, resolution and letterbox. Toggles, refresh rate,
/// window mode, v-sync, frame limit and Reflex are not: they matter for feel
/// and latency, but they do not start a new [`Period`].
pub fn is_aim_field(field: &str) -> bool {
    match field.split_once('.') {
        Some(("input", rest)) => !rest.starts_with("toggle"),
        Some(("display", rest)) => matches!(
            rest,
            "fov" | "aspectRatio" | "resolutionWidth" | "resolutionHeight" | "useLetterbox"
        ),
        _ => false,
    }
}

fn flatten(prefix: &str, value: &Value, out: &mut BTreeMap<String, Value>) {
    match value {
        Value::Null => {}
        Value::Object(map) => {
            for (key, value) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(&path, value, out);
            }
        }
        other => {
            out.insert(prefix.to_owned(), other.clone());
        }
    }
}

// Derived values. None of these are in the file.

/// Radians the view turns per mouse count, per unit of `slider x multiplier
/// unit`. See [`InputSettings::counts_per_360`].
pub const RADIANS_PER_COUNT_UNIT: f64 = 0.005;

impl InputSettings {
    /// Derived: mouse counts for a full horizontal turn from the hip.
    ///
    /// ```text
    /// radians per count = MouseYawSensitivity * MouseSensitivityMultiplierUnit * 0.005
    /// counts per 360    = 2 * pi / radians per count
    /// ```
    ///
    /// Source: the yaw constant sensitivity converters use for Siege,
    /// 0.00572957795 degrees per count per slider step at the default
    /// multiplier unit of 0.02 (mouse-sensitivity.com's Siege entry and
    /// forum). That is 0.0001 radians, so 0.005 per unit of slider times
    /// multiplier. Confidence: medium. It is a community measurement, not a
    /// formula Ubisoft published, and hip fire in Siege does not depend on
    /// field of view. `None` when a value is missing or not positive.
    pub fn counts_per_360(&self) -> Option<f64> {
        let step = f64::from(self.mouse_yaw_sensitivity?)
            * self.mouse_sensitivity_multiplier_unit?
            * RADIANS_PER_COUNT_UNIT;
        (step.is_finite() && step > 0.0).then(|| std::f64::consts::TAU / step)
    }

    /// Derived: centimetres of mouse travel for a full horizontal turn from
    /// the hip, `counts_per_360 / dpi * 2.54`. The mouse's DPI is not in the
    /// file, so the caller supplies it; the result is only as good as that
    /// number and the formula of [`InputSettings::counts_per_360`]. Windows
    /// pointer speed and acceleration are not accounted for; they do not
    /// apply with raw input on.
    pub fn cm_per_360(&self, dpi: f64) -> Option<f64> {
        (dpi.is_finite() && dpi > 0.0)
            .then(|| self.counts_per_360())
            .flatten()
            .map(|counts| counts / dpi * 2.54)
    }

    /// The mouse ADS slider in effect for a sight: its own when
    /// `ADSMouseUseSpecific` is on, else the global one.
    pub fn ads_mouse_slider(&self, zoom: Zoom) -> Option<u32> {
        if self.ads_mouse_use_specific? {
            self.ads_mouse.get(zoom)
        } else {
            self.ads_mouse_global
        }
    }

    /// Derived: the factor the ADS slider applies on top of hip sensitivity,
    /// `slider * ADSMouseMultiplierUnit` (1.0 at the default 50 and 0.02).
    ///
    /// This is not the full ADS sensitivity: the game also scales by a
    /// per-sight field of view factor that is not in the file (Ubisoft's
    /// Y5S3 "FOV and sensitivity" dev blog describes the scheme). Use it to
    /// compare one sight before and after a change, not as a cm/360.
    /// Confidence: medium for the product, from the same sources as
    /// [`InputSettings::counts_per_360`].
    pub fn ads_mouse_multiplier(&self, zoom: Zoom) -> Option<f64> {
        let factor = f64::from(self.ads_mouse_slider(zoom)?) * self.ads_mouse_multiplier_unit?;
        factor.is_finite().then_some(factor)
    }
}

/// A derived aspect ratio and what it was derived from.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Aspect {
    /// Width over height.
    pub ratio: f64,
    pub source: AspectSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AspectSource {
    /// `AspectRatio` 1: the game uses the resolution's ratio.
    Resolution,
    /// `AspectRatio` 0: the game uses the display's ratio, which is not in
    /// the file. The resolution's ratio stands in for it; right in
    /// fullscreen at the display's native shape, a guess otherwise.
    ResolutionAssumedDisplay,
}

impl DisplaySettings {
    /// Derived: the aspect ratio the game renders at, when the file is
    /// enough to tell. `None` when `AspectRatio` picks an entry of the game's
    /// menu (2 and up; the file does not say which ratio each is, so pass the
    /// ratio to [`DisplaySettings::horizontal_fov`] yourself), or when the
    /// resolution is missing or set to autodetect.
    pub fn aspect(&self) -> Option<Aspect> {
        let source = match self.aspect_ratio? {
            0 => AspectSource::ResolutionAssumedDisplay,
            1 => AspectSource::Resolution,
            _ => return None,
        };
        let (width, height) = (self.resolution_width?, self.resolution_height?);
        (width > 0 && height > 0).then(|| Aspect {
            ratio: f64::from(width) / f64::from(height),
            source,
        })
    }

    /// Derived: the horizontal field of view in degrees for an aspect ratio
    /// (width over height).
    ///
    /// ```text
    /// horizontal = 2 * atan(tan(vertical / 2) * aspect)
    /// ```
    ///
    /// The formula is exact for a rectilinear projection; that `DefaultFOV`
    /// is the vertical angle is the file's own comment. `None` when the field
    /// of view is missing or outside (0, 180), or the aspect is not positive.
    pub fn horizontal_fov(&self, aspect: f64) -> Option<f64> {
        let vertical = self.fov?;
        (vertical > 0.0 && vertical < 180.0 && aspect.is_finite() && aspect > 0.0).then(|| {
            2.0 * ((vertical.to_radians() / 2.0).tan() * aspect)
                .atan()
                .to_degrees()
        })
    }

    /// Derived: [`DisplaySettings::horizontal_fov`] at
    /// [`DisplaySettings::aspect`].
    pub fn horizontal_fov_auto(&self) -> Option<f64> {
        self.horizontal_fov(self.aspect()?.ratio)
    }
}

// INI.

/// `section -> key -> value`, names lowercased.
struct Ini(BTreeMap<String, BTreeMap<String, String>>);

impl Ini {
    fn new(text: &str) -> Self {
        let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
            if line.is_empty() || line.starts_with([';', '#']) {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                // A header with no closing bracket still ends the section
                // before it, so its keys cannot leak into that one.
                section = match rest.split_once(']') {
                    Some((name, _)) => name.trim().to_ascii_lowercase(),
                    None => String::new(),
                };
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            sections
                .entry(section.clone())
                .or_default()
                .insert(key.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
        Self(sections)
    }

    fn get(&self, section: &str, key: &str) -> Option<&str> {
        let value = self.0.get(section)?.get(&key.to_ascii_lowercase())?;
        Some(value.as_str())
    }

    fn float(&self, section: &str, key: &str) -> Option<f64> {
        let value: f64 = self.get(section, key)?.parse().ok()?;
        value.is_finite().then_some(value)
    }

    /// A non-negative whole number, also when written as `50.000000`.
    fn int(&self, section: &str, key: &str) -> Option<u32> {
        let value = self.float(section, key)?;
        (value >= 0.0 && value <= f64::from(u32::MAX) && value.fract() == 0.0)
            .then_some(value as u32)
    }

    fn flag(&self, section: &str, key: &str) -> Option<bool> {
        match self.get(section, key)?.to_ascii_lowercase().as_str() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        }
    }
}

fn decode(bytes: &[u8]) -> String {
    let utf16 = |bytes: &[u8], unit: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| unit(*pair))
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xff, 0xfe, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xfe, 0xff, rest @ ..] => utf16(rest, u16::from_be_bytes),
        [0xef, 0xbb, 0xbf, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

// Locating and reading.

/// Candidate paths of a profile's settings file, most likely first:
/// `<Documents>/My Games/Rainbow Six - Siege/<profile id>/GameSettings.ini`
/// under the user's profile folder and under each OneDrive folder (Documents
/// can be redirected into OneDrive).
///
/// The profile id is the recording player's in a replay
/// (`MatchSummary::recording.profile_id`). This reads the `USERPROFILE`,
/// `OneDrive`, `OneDriveConsumer` and `OneDriveCommercial` environment
/// variables and nothing else: it does not touch the filesystem, so a
/// returned path may not exist. Empty when the id is not a profile id (see
/// [`locate_in`]) or none of the variables is set.
///
/// A Documents folder moved elsewhere, or with a localized name on disk, is
/// not found; resolve the Documents folder yourself and use [`locate_in`].
pub fn locate(profile_id: &str) -> Vec<PathBuf> {
    let mut documents: Vec<PathBuf> = Vec::new();
    for name in [
        "USERPROFILE",
        "OneDrive",
        "OneDriveConsumer",
        "OneDriveCommercial",
    ] {
        let Some(base) = std::env::var_os(name).filter(|v| !v.is_empty()) else {
            continue;
        };
        let dir = PathBuf::from(base).join("Documents");
        if !documents.contains(&dir) {
            documents.push(dir);
        }
    }
    locate_in(&documents, profile_id)
}

/// [`locate`] under the given Documents folders, one candidate per folder in
/// the order given. Pure path building.
///
/// Empty unless `profile_id` is made of ASCII letters, digits and dashes
/// only (profile ids are UUIDs), so an id taken from a replay cannot steer
/// the path somewhere else.
pub fn locate_in<P: AsRef<Path>>(documents: &[P], profile_id: &str) -> Vec<PathBuf> {
    let valid = !profile_id.is_empty()
        && profile_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if !valid {
        return Vec::new();
    }
    documents
        .iter()
        .map(|dir| {
            dir.as_ref()
                .join("My Games")
                .join("Rainbow Six - Siege")
                .join(profile_id)
                .join(FILE_NAME)
        })
        .collect()
}

/// Reads the settings file at `path`. Opens it read-only and never writes.
/// Fails only when the file cannot be read; its contents cannot make it
/// fail.
pub fn read(path: impl AsRef<Path>) -> Result<GameSettings> {
    Ok(GameSettings::from_bytes(&fs::read(path)?))
}

// History.

/// The settings as read at one moment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// When the file was read, UTC.
    #[serde(with = "rfc3339")]
    pub taken_at: DateTime<Utc>,
    /// The latest time a later read found these same settings; `taken_at`
    /// until then. [`SettingsHistory::record`] keeps it up to date.
    #[serde(with = "rfc3339")]
    pub last_seen: DateTime<Utc>,
    /// The file's modified time, UTC, when the filesystem gave one. The game
    /// rewrites the file at times of its own choosing, so this bounds a
    /// change from above; it is not the time of the change.
    #[serde(with = "rfc3339_opt", default)]
    pub file_modified: Option<DateTime<Utc>>,
    /// SHA-256 of the whole file, lowercase hex. It covers keys this module
    /// does not read, so it can differ between snapshots with equal
    /// `settings`.
    pub sha256: String,
    pub settings: GameSettings,
}

impl Snapshot {
    /// Reads the file at `path` now. Opens it read-only and never writes.
    pub fn take(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = fs::read(path)?;
        let modified = fs::metadata(path).and_then(|m| m.modified()).ok();
        Ok(Self::from_bytes(
            &bytes,
            SystemTime::now().into(),
            modified.map(DateTime::<Utc>::from),
        ))
    }

    /// A snapshot of bytes the caller already holds.
    pub fn from_bytes(
        bytes: &[u8],
        taken_at: DateTime<Utc>,
        file_modified: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            taken_at,
            last_seen: taken_at,
            file_modified,
            sha256: crate::file::sha256(bytes),
            settings: GameSettings::from_bytes(bytes),
        }
    }

    /// The earliest time these settings are known to have been in the file,
    /// given the time the previous settings were last seen: the file's
    /// modified time when it lies after that and not after `taken_at`, else
    /// `taken_at`.
    fn known_from(&self, previous_seen: Option<DateTime<Utc>>) -> DateTime<Utc> {
        match self.file_modified {
            Some(modified)
                if modified <= self.taken_at && previous_seen.is_none_or(|p| modified > p) =>
            {
                modified
            }
            _ => self.taken_at,
        }
    }
}

/// One whitelisted value that differs between two consecutive snapshots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    /// The earliest time the new value is known to have been in the file:
    /// the upper end of the window the change happened in.
    #[serde(with = "rfc3339")]
    pub at: DateTime<Utc>,
    /// When the old value was last seen: the lower end of that window.
    #[serde(with = "rfc3339")]
    pub not_before: DateTime<Utc>,
    /// The field's path, as in [`GameSettings::fields`].
    pub field: String,
    /// `None` when the key was absent or unreadable before.
    pub from: Option<Value>,
    /// `None` when the key is absent or unreadable now.
    pub to: Option<Value>,
    /// [`is_aim_field`] of `field`.
    pub aim: bool,
}

/// A span over which the aim settings ([`GameSettings::aim`]) did not change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Period {
    /// The earliest time these settings are known to have been in the file.
    /// For the first period that is the first snapshot's file modified time
    /// (or the snapshot itself): what the settings were before is unknown.
    #[serde(with = "rfc3339")]
    pub start: DateTime<Utc>,
    /// The last time a snapshot saw these settings.
    #[serde(with = "rfc3339")]
    pub last_seen: DateTime<Utc>,
    /// True for the latest period, which is taken to run on past
    /// `last_seen` until a snapshot shows otherwise.
    pub open: bool,
    /// The aim settings of the span; non-aim fields are cleared.
    pub settings: GameSettings,
}

/// Where a moment falls among a history's periods.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Placement {
    /// Before the first period, or the history is empty: settings unknown.
    BeforeHistory,
    /// Inside the period with this index.
    Period(usize),
    /// In the window between this period's last sighting and the next
    /// period's start: the change happened somewhere in it.
    AfterPeriod(usize),
}

/// Snapshots in time order, one per distinct set of whitelisted values. The
/// app stores it (it is plain serde data) and calls
/// [`SettingsHistory::record`] whenever the user lets it read the file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsHistory {
    pub snapshots: Vec<Snapshot>,
}

impl SettingsHistory {
    /// Appends `snapshot` when its whitelisted values differ from the latest
    /// one's, and returns whether it did. An unchanged snapshot only moves
    /// the latest one's `last_seen` forward. A snapshot older than the latest
    /// one's `last_seen` is ignored: history is append-only.
    pub fn record(&mut self, snapshot: Snapshot) -> bool {
        let Some(last) = self.snapshots.last_mut() else {
            self.snapshots.push(snapshot);
            return true;
        };
        if snapshot.taken_at < last.last_seen {
            return false;
        }
        if snapshot.settings == last.settings {
            last.last_seen = snapshot.taken_at;
            return false;
        }
        self.snapshots.push(snapshot);
        true
    }

    /// The settings of the latest snapshot.
    pub fn latest(&self) -> Option<&GameSettings> {
        self.snapshots.last().map(|s| &s.settings)
    }

    /// Every value that differs between consecutive snapshots, oldest first,
    /// fields of one snapshot in path order. See the module docs for what
    /// `at` and `not_before` mean.
    pub fn changes(&self) -> Vec<Change> {
        let mut out = Vec::new();
        for pair in self.snapshots.windows(2) {
            let (old, new) = (&pair[0], &pair[1]);
            let at = new.known_from(Some(old.last_seen));
            let (from, to) = (old.settings.fields(), new.settings.fields());
            let fields: std::collections::BTreeSet<&String> =
                from.keys().chain(to.keys()).collect();
            for field in fields {
                let (a, b) = (from.get(field), to.get(field));
                if a != b {
                    out.push(Change {
                        at,
                        not_before: old.last_seen,
                        field: field.clone(),
                        from: a.cloned(),
                        to: b.cloned(),
                        aim: is_aim_field(field),
                    });
                }
            }
        }
        out
    }

    /// The spans of constant aim settings, oldest first. Consecutive
    /// snapshots that differ only in non-aim fields share a period. The
    /// window between one period's `last_seen` and the next one's `start` is
    /// when the change happened and belongs to neither.
    pub fn periods(&self) -> Vec<Period> {
        let mut out: Vec<Period> = Vec::new();
        let mut previous_seen = None;
        for snapshot in &self.snapshots {
            let aim = snapshot.settings.aim();
            match out.last_mut() {
                Some(period) if period.settings == aim => period.last_seen = snapshot.last_seen,
                _ => out.push(Period {
                    start: snapshot.known_from(previous_seen),
                    last_seen: snapshot.last_seen,
                    open: false,
                    settings: aim,
                }),
            }
            previous_seen = Some(snapshot.last_seen);
        }
        if let Some(period) = out.last_mut() {
            period.open = true;
        }
        out
    }

    /// Where `at` falls among [`SettingsHistory::periods`].
    pub fn place(&self, at: DateTime<Utc>) -> Placement {
        place(&self.periods(), at)
    }
}

fn place(periods: &[Period], at: DateTime<Utc>) -> Placement {
    let mut placement = Placement::BeforeHistory;
    for (index, period) in periods.iter().enumerate() {
        if at < period.start {
            break;
        }
        placement = if period.open || at <= period.last_seen {
            Placement::Period(index)
        } else {
            Placement::AfterPeriod(index)
        };
    }
    placement
}

/// Matches bucketed by settings period. Every number is an index into the
/// start times given to [`split_matches`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchSplit {
    /// One bucket per period of [`SettingsHistory::periods`], in order.
    pub periods: Vec<Vec<usize>>,
    /// Matches from before the first period: their settings are unknown.
    pub before_history: Vec<usize>,
    /// Matches that started inside a change window, so either side's
    /// settings could have applied.
    pub uncertain: Vec<UncertainMatch>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UncertainMatch {
    /// Index into the start times.
    pub index: usize,
    /// The period before the window; the one after is `after_period + 1`.
    pub after_period: usize,
}

/// Buckets matches, by their start times (`MatchSummary::start_time`, UTC),
/// into the history's periods so stats can be compared before and after a
/// change. Order of the input does not matter.
///
/// Replays from before Y11S3 only carry the recording PC's local time
/// (`MatchSummary::start_time_is_local`); convert it to UTC first or the
/// buckets are off by the UTC offset.
pub fn split_matches(
    history: &SettingsHistory,
    starts: impl IntoIterator<Item = DateTime<Utc>>,
) -> MatchSplit {
    let periods = history.periods();
    let mut split = MatchSplit {
        periods: vec![Vec::new(); periods.len()],
        ..MatchSplit::default()
    };
    for (index, start) in starts.into_iter().enumerate() {
        match place(&periods, start) {
            Placement::BeforeHistory => split.before_history.push(index),
            Placement::Period(period) => split.periods[period].push(index),
            Placement::AfterPeriod(after_period) => split.uncertain.push(UncertainMatch {
                index,
                after_period,
            }),
        }
    }
    split
}

/// Headshot rate per settings period.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SinceChange {
    /// One entry per period of [`SettingsHistory::periods`], in order; the
    /// last is "since the latest change".
    pub periods: Vec<PeriodStats>,
    /// Matches left out because they predate the history.
    pub before_history: u32,
    /// Matches left out because they started inside a change window.
    pub uncertain: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodStats {
    pub period: Period,
    pub matches: u32,
    pub kills: u32,
    pub headshots: u32,
    /// Headshot kills over kills, in percent, as
    /// [`crate::stats::headshot_percentage`]; 0 without kills.
    pub headshot_percentage: f64,
}

/// Sums one player's kills and headshots per settings period, from each
/// match's start time and that player's stats (see [`recording_stats`]).
///
/// The periods say what the settings file held, not what caused the numbers:
/// few matches, a different map pool or different opponents move a headshot
/// rate as easily as a sensitivity change does. Show `matches` and `kills`
/// next to the rate.
pub fn since_change<'a>(
    history: &SettingsHistory,
    matches: impl IntoIterator<Item = (DateTime<Utc>, &'a PlayerMatchStats)>,
) -> SinceChange {
    let periods = history.periods();
    let mut out = SinceChange {
        periods: periods
            .iter()
            .map(|period| PeriodStats {
                period: period.clone(),
                matches: 0,
                kills: 0,
                headshots: 0,
                headshot_percentage: 0.0,
            })
            .collect(),
        before_history: 0,
        uncertain: 0,
    };
    for (start, stats) in matches {
        match place(&periods, start) {
            Placement::BeforeHistory => out.before_history += 1,
            Placement::AfterPeriod(_) => out.uncertain += 1,
            Placement::Period(index) => {
                let period = &mut out.periods[index];
                period.matches += 1;
                period.kills += stats.kills;
                period.headshots += stats.headshots;
            }
        }
    }
    for period in &mut out.periods {
        period.headshot_percentage = headshot_percentage(period.headshots, period.kills);
    }
    out
}

/// A match's start time and the recording player's stats in it, the input
/// [`since_change`] takes. `None` for a spectator's recording, a match with
/// no rounds, or when the recording player has no stats row.
pub fn recording_stats(m: &Match) -> Option<(DateTime<Utc>, PlayerMatchStats)> {
    let summary = m.summary()?;
    if summary.recording.spectator {
        return None;
    }
    let username = summary.recording.username?;
    let stats = m
        .player_stats()
        .into_iter()
        .find(|s| s.username == username)?;
    Some((summary.start_time, stats))
}

mod rfc3339 {
    use super::*;

    pub fn serialize<S: Serializer>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&t.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        let text = String::deserialize(d)?;
        DateTime::parse_from_rfc3339(&text)
            .map(|t| t.with_timezone(&Utc))
            .map_err(serde::de::Error::custom)
    }
}

mod rfc3339_opt {
    use super::*;

    pub fn serialize<S: Serializer>(t: &Option<DateTime<Utc>>, s: S) -> Result<S::Ok, S::Error> {
        match t {
            Some(t) => rfc3339::serialize(t, s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DateTime<Utc>>, D::Error> {
        Option::<String>::deserialize(d)?
            .map(|text| {
                DateTime::parse_from_rfc3339(&text)
                    .map(|t| t.with_timezone(&Utc))
                    .map_err(serde::de::Error::custom)
            })
            .transpose()
    }
}
