# Game settings (`GameSettings.ini`)

Proposed README section for `replay_analyzer::settings`, plus the CLI flag it
suggests. Nothing here is wired into `src/main.rs` yet.

## Why this reads outside the replay

A replay does not record sensitivity, field of view, resolution or any other
client setting. The census of a Y11S3 round lists 34 distinct property keys (match,
round, team and player facts, `gmsetting` asset ids for the match rules) and
none of them is a client setting; the packet census of the stream has no
such record either. The only place these live is the game's settings file:

```
<Documents>\My Games\Rainbow Six - Siege\<profile id>\GameSettings.ini
```

The folder name is the Ubisoft profile id, which is the recording player's
`profileID` in a replay (`summary.recording.profileID`), so a replay says
which folder belongs to the player who recorded it.

## Consent and read-only guarantees

- The library never looks for or opens the file on its own. No other module
  calls `settings`, and parsing a replay never touches it.
- `settings::locate(profile_id)` builds candidate paths from the
  `USERPROFILE`, `OneDrive`, `OneDriveConsumer` and `OneDriveCommercial`
  environment variables. It does not touch the filesystem, not even to check
  that a candidate exists. `settings::locate_in(documents, profile_id)` does
  the same under Documents folders the caller resolved.
- `settings::read(path)` and `Snapshot::take(path)` open the one path given,
  read-only. Nothing in the module writes, creates, renames or deletes.
- Only whitelisted keys are read (below). Hardware ids, the GPU adapter
  string, audio devices and the whole `[ONLINE]` section (data centre hint,
  proxy) have no field in the output type. `Snapshot::sha256` is a hash of
  the whole file; it reveals nothing about those keys beyond "something
  changed".
- Asking the user is the application's job: ask before the first read, and
  say that the file is read and never changed.

## Keys read

`[INPUT]`, mouse: `RawInputMouseKeyboard`, `InvertMouseAxisY`,
`MouseYawSensitivity`, `MousePitchSensitivity`,
`MouseSensitivityMultiplierUnit`, `XFactorAiming`, `AimDownSightsMouse`,
`ADSMouseUseSpecific`, `ADSMouseSensitivityGlobal`,
`ADSMouseSensitivity1x`, `1xHalf`, `2x`, `2xHalf`, `3x`, `4x`, `5x`, `8x`,
`12x`, `ADSMouseMultiplierUnit`, `ToggleAim`, `ToggleLean`.

`[INPUT]`, controller: `InvertAxisY`, `YawSensitivity`, `PitchSensitivity`,
`DeadzoneLeftStick`, `DeadzoneRightStick`, `ControllerStickRotationCurve`,
`AimDownSights`, `ADSGamepadUseSpecific`, `ADSGamepadSensitivityGlobal`,
`ADSGamepadSensitivity1x` .. `12x` (the same nine), `ADSGamepadMultiplierUnit`.

`[DISPLAY_SETTINGS]`: `DefaultFOV`, `AspectRatio`, `ResolutionWidth`,
`ResolutionHeight`, `RefreshRate`, `WindowMode`, `VSync`, `UseLetterbox`.

`[DISPLAY]`: `FPSLimit`, `NVReflex`.

Every field is optional: a missing key or a value that does not parse is
`null`, never an error. The parser accepts comments (`;`, `#`), CRLF, a
UTF-8 or UTF-16 byte order mark, unknown sections and keys, and matches
names case-insensitively.

## Derived values

These are computed, not read, and say so in their docs.

| Value | Formula | Confidence |
| --- | --- | --- |
| Horizontal FOV | `2 * atan(tan(DefaultFOV / 2) * aspect)` | Exact geometry. `DefaultFOV` being vertical is the file's own comment. |
| Aspect ratio | `ResolutionWidth / ResolutionHeight` when `AspectRatio` is 1 (resolution) or 0 (display, assumed to match the resolution) | Not derived for 2 and up: those are entries of the game's menu and the file does not say which ratio each is. Pass the ratio yourself. |
| Counts per 360 | `2 * pi / (MouseYawSensitivity * MouseSensitivityMultiplierUnit * 0.005)` | Medium. From the yaw constant sensitivity converters use for Siege (0.00572957795 degrees per count per slider step at the default unit 0.02, i.e. 0.0001 rad); a community measurement, not published by Ubisoft. |
| cm/360 | `counts per 360 / DPI * 2.54` | As above, and only as good as the DPI the caller supplies: the DPI is not in the file. |
| ADS multiplier | `ADS slider * ADSMouseMultiplierUnit` (the per-zoom slider when `ADSMouseUseSpecific` is on, else the global one) | Medium, and partial: the game also applies a per-sight FOV factor that is not in the file, so this compares one sight across changes; it is not an ADS cm/360. |

Sources: the mouse-sensitivity.com Siege entry and forum threads for the yaw
constant and the multiplier unit; Ubisoft's Y5S3 "FOV and sensitivity" dev
blog for the per-zoom ADS scheme.

## History: "since you changed sensitivity"

The app stores a `SettingsHistory` (plain serde data) and calls
`history.record(Snapshot::take(path)?)` whenever it is allowed to read the
file, ideally at every replay import.

- `record` appends only when a whitelisted value changed; otherwise it only
  moves the latest snapshot's `lastSeen` forward.
- `changes()` lists `{at, notBefore, field, from, to, aim}`.
- `periods()` gives the spans over which the aim settings were constant.
  Aim settings are every sensitivity, multiplier, deadzone, curve, inversion
  and raw input, plus FOV, aspect ratio, resolution and letterbox. Toggles,
  refresh rate, window mode, v-sync, frame limit and Reflex are tracked as
  changes but do not start a period.
- `split_matches(&history, start_times)` buckets matches into those periods.
- `since_change(&history, matches)` sums kills and headshots per period from
  `PlayerMatchStats`; `recording_stats(&match)` gives the recording player's
  row and the match start time.

A change is not dated exactly. The file holds the current values and one
modified time, so a change is known to lie after the last snapshot that
still showed the old value (`notBefore`) and no later than the file's
modified time, or the new snapshot when the modified time is unusable
(`at`). Matches that started inside that window are reported as `uncertain`
and matches older than the first snapshot's file time as `beforeHistory`;
neither is counted in a period. The latest period is taken to run on until a
snapshot shows otherwise, so snapshot before attributing new matches.

A headshot rate per period is a correlation. Show the match and kill counts
beside it.

## Limits

- Windows paths only. A Documents folder moved off the default location or
  with a localized folder name on disk (OneDrive does this in some locales)
  is not found by `locate`; resolve the folder with the shell's known-folder
  API and call `locate_in`.
- Mouse DPI, Windows pointer speed and acceleration, and the monitor's
  physical size are not in the file.
- Settings from before the first snapshot are unknown.
- Replays from before Y11S3 carry local start times
  (`startTimeIsLocal`); convert to UTC before splitting.

## Suggested CLI shape

Explicit path only, so running the tool is the consent:

```
replay-analyzer <replay or folder> --settings <path to GameSettings.ini> [--dpi <n>]
```

- `--settings <path>` adds a `settings` object to the output: the whitelisted
  values, plus `derived` (`horizontalFov`, `countsPer360`, and `cmPer360`
  when `--dpi` is given).
- No auto-discovery flag. A separate `--settings-path` could print the
  candidates from `locate` for the replay's recording profile id without
  opening any of them, for the user to copy into `--settings`.
- History stays an application concern (it needs storage); the CLI could
  take `--settings-history <file.json>` to print `since_change` for a match
  folder, reading that file and never writing it.
