//! The container around the header and packets: the `dissect` prelude, the
//! game version, and the frame time index.
//!
//! Layout, as observed from Y8S1 to Y11S3:
//!
//! ```text
//! "dissect" 00                 magic
//! u32 format                   7 up to Y8S3, 8 from Y8S4 (chunked zstd layout)
//! string "UNKNOWN"             u64 length, then the text
//! u32 0
//! u32 last frame               number of frames in the time index, minus one
//! u32 property count           header key/value pairs that follow
//! u32 0
//! properties ...
//! u32 ?  u32 ?  u32 frames     frame time index: frames x (u32 index, f64 seconds)
//! ...                          per-player table, then a `CMPRV002` trailer
//! ```
//!
//! From Y8S4 the index sits uncompressed between the header and the first
//! zstd frame; before that it follows the header inside the zstd stream.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::cursor::Cursor;
use crate::error::{Error, Result};

pub const MAGIC: &[u8] = b"dissect";

/// Container format versions seen: 7 up to Y8S3, 8 from Y8S4.
pub const KNOWN_FORMAT_VERSIONS: [u32; 2] = [7, 8];
/// The text after the format version in every replay seen.
pub const KNOWN_LABEL: &str = "UNKNOWN";

/// A string in the prelude or header: u64 length, then the bytes. Every
/// length seen so far is under 256, so the upper seven bytes of the length
/// look like a run of zeros.
pub(crate) fn read_string(c: &mut Cursor) -> Result<String> {
    let at = c.pos();
    let len = u64::from_le_bytes(c.array()?);
    let bytes = usize::try_from(len)
        .ok()
        .and_then(|len| c.bytes(len).ok())
        .ok_or(Error::InvalidStringLength(at))?;
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

/// What the bytes before the header properties say.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FormatInfo {
    pub magic: String,
    /// Container format revision. 7 before Y8S4, 8 since.
    pub format_version: u32,
    /// Text slot after the version; always `UNKNOWN` so far.
    pub label: String,
    /// Frames the time index should hold, from the prelude.
    pub declared_frames: u32,
    /// Header properties the prelude announces.
    pub property_count: u32,
    /// `chunked` (Y8S4+: plain header, many zstd frames) or `stream` (one
    /// zstd stream holding everything).
    pub layout: Layout,
    /// False when the prelude did not have the expected shape; the header
    /// was then found by scanning, and the fields above are empty.
    pub prelude_decoded: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Layout {
    #[default]
    Stream,
    Chunked,
}

/// Reads the prelude. `c` must start at the magic; on success it is left at
/// the first property.
pub fn read_prelude(c: &mut Cursor) -> Option<FormatInfo> {
    let mut t = *c;
    if t.bytes(7).ok()? != MAGIC || t.u8().ok()? != 0 {
        return None;
    }
    let format_version = u32::from_le_bytes(t.array().ok()?);
    let label = read_string(&mut t).ok()?;
    let [zero, last_frame, property_count, zero2] =
        [(); 4].map(|_| t.array().map(u32::from_le_bytes).unwrap_or(u32::MAX));
    if zero != 0 || zero2 != 0 || property_count == 0 || property_count > 10_000 {
        return None;
    }
    *c = t;
    Some(FormatInfo {
        magic: String::from_utf8_lossy(MAGIC).into_owned(),
        format_version,
        label,
        declared_frames: last_frame.saturating_add(1),
        property_count,
        layout: Layout::Stream,
        prelude_decoded: true,
    })
}

/// The `version` header property, e.g. `Y11S3_Alpha04`.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GameVersion {
    /// As written in the header.
    pub raw: String,
    /// `Y11S3`, when the text starts with a year and season.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub season: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub season_number: Option<u32>,
    /// Anything after the season, such as `Alpha04`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub branch: String,
    /// The `code` header property: the game build number.
    pub build: u32,
}

impl GameVersion {
    pub fn parse(raw: &str, build: u32) -> Self {
        let mut v = GameVersion {
            raw: raw.to_owned(),
            build,
            ..Default::default()
        };
        let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
        let Some(rest) = raw.strip_prefix('Y') else {
            return v;
        };
        let y = digits(rest);
        let Some(after_s) = rest[y..].strip_prefix('S') else {
            return v;
        };
        let s = digits(after_s);
        if y == 0 || s == 0 {
            return v;
        }
        v.year = rest[..y].parse().ok();
        v.season_number = after_s[..s].parse().ok();
        v.season = Some(format!("Y{}S{}", &rest[..y], &after_s[..s]));
        v.branch = after_s[s..]
            .trim_start_matches(['_', '-', '.', ' '])
            .to_owned();
        v
    }
}

/// The per-frame timestamps written with every replay.
#[derive(Clone, Debug, Default)]
pub struct FrameIndex {
    /// Seconds since recording started, one per frame.
    pub times: Vec<f64>,
    /// Frame entries whose index did not match their position.
    pub out_of_order: usize,
    /// Bytes the index takes up; the stream list follows it (Y8S4+).
    pub len: usize,
}

/// Reads the index at `bytes`. `expected` is the frame count from the
/// prelude, used to reject bytes that are not an index.
pub fn read_frame_index(bytes: &[u8], expected: u32) -> Option<FrameIndex> {
    let mut c = Cursor::new(bytes, 0);
    c.skip(8).ok()?;
    let n = u32::from_le_bytes(c.array().ok()?);
    if n == 0 || (expected != 0 && n != expected) || n as usize > bytes.len() / 12 {
        return None;
    }
    let mut index = FrameIndex {
        times: Vec::with_capacity(n as usize),
        out_of_order: 0,
        len: 12 + 12 * n as usize,
    };
    for i in 0..n {
        let idx = u32::from_le_bytes(c.array().ok()?);
        let t = f64::from_le_bytes(c.array().ok()?);
        if idx != i {
            index.out_of_order += 1;
        }
        if !t.is_finite() {
            return None;
        }
        index.times.push(t);
    }
    Some(index)
}

/// How often the replay recorded a frame, and where it skipped.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Timing {
    pub frames: usize,
    /// Seconds from the first to the last frame.
    pub duration: f64,
    /// Typical time between frames.
    pub median_interval: f64,
    /// Frames per second, from the median interval. Spectator recordings
    /// index a fixed ~29.4 frames a second; a player's recording follows
    /// their frame rate, often hundreds a second.
    pub sample_rate: f64,
    /// Frames per second over the whole recording.
    pub mean_rate: f64,
    /// Records per second in the state stream: how often the game sent
    /// updates, whatever the frame rate (Y8S4+ full and partial reads).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_rate: Option<f64>,
    /// Frames whose timestamp went backwards.
    pub backwards: usize,
    /// Frame intervals much longer than usual.
    pub gaps: Vec<Gap>,
    /// Longest stretch without a frame.
    pub max_interval: f64,
    /// Stretches without a movement record, which the game writes at every
    /// update: the recording missed data there even if frames were indexed
    /// (Y8S4+ full and partial reads).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Hole>,
    /// Moments the game moved on by more than the recording's clock did:
    /// see [`Skip`] (Y11S3 full reads).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skips: Vec<Skip>,
    /// Places where the in-game clock skipped seconds.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clock_gaps: Vec<ClockGap>,
    /// When recording started in UTC: the header's `endtime` minus the
    /// duration (Y11S3+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// The header `datetime` is the recording PC's local time; this is its
    /// offset from UTC in minutes, found from `endtime` (Y11S3+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_utc_offset_minutes: Option<i64>,
    /// Seconds the header's `endtime` less `starttime` is longer than the
    /// frame index: a few milliseconds, up to seconds in a recording that
    /// skips game time, and more only when the recording stood still (see
    /// [`crate::pauses`]; Y11S3+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_minus_index: Option<f64>,
}

impl Timing {
    /// Places the recording in UTC from the header's `starttime` or, failing
    /// that, `endtime` minus the duration, and works out the offset of the
    /// local header timestamp. Returns a warning when start, end and the
    /// frame index disagree about the length of the recording.
    pub fn calibrate(
        &mut self,
        header_time: DateTime<Utc>,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> Option<String> {
        let length = Duration::milliseconds((self.duration * 1000.0).round() as i64);
        let start = start.or(end.map(|e| e - length))?;
        self.started_at = Some(start.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string());
        let offset = (header_time - start).num_seconds() as f64 / 60.0;
        // Time zones are whole quarter hours.
        self.header_utc_offset_minutes = Some((offset / 15.0).round() as i64 * 15);
        let wall = (end? - start).num_milliseconds() as f64 / 1000.0;
        self.header_minus_index = Some(((wall - self.duration) * 1000.0).round() / 1000.0);
        ((wall - self.duration).abs() > WALL_CLOCK_TOLERANCE).then(|| {
            format!(
                "recording spans {wall:.1}s by start/end time but {:.1}s by frame index",
                self.duration
            )
        })
    }
}

/// Start/end time and frame index may disagree by this many seconds.
const WALL_CLOCK_TOLERANCE: f64 = 2.0;

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Gap {
    /// Frame after the gap.
    pub frame: usize,
    /// Recording time where the gap starts.
    pub at: f64,
    pub seconds: f64,
}

/// A stretch without movement records.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Hole {
    /// Seconds since the recording started, at the last record before the
    /// hole.
    pub at: f64,
    pub seconds: f64,
}

/// A moment the game moved on by more than the recording's clock did.
/// Frames follow each other as always, but two or more players who were
/// walking are suddenly metres further on, as far as they would have got
/// in `seconds` more than passed. Whatever is timed across it comes out
/// that much shorter than it was in the game: a reinforcement that takes
/// 4.1 s is up in 3.1 s. Inferred from the bodies; the file has no marker
/// for it.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Skip {
    /// Seconds since the recording started of the last positions before
    /// the jump, and of the first after it.
    pub at: f64,
    pub until: f64,
    /// Game time missing, estimated: how long the players would have
    /// taken for the jump at the speed they had, less the time that
    /// passed; the median over them.
    pub seconds: f64,
    /// How many players jumped.
    pub bodies: usize,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClockGap {
    /// Clock shown before the jump.
    pub from: String,
    /// Clock shown after it.
    pub to: String,
    /// Seconds missing between the two.
    pub missing: f64,
}

/// An interval this many times the median is a gap.
const GAP_FACTOR: f64 = 4.0;
/// Intervals shorter than this are never gaps, whatever the median.
const MIN_GAP: f64 = 0.25;

impl Timing {
    pub fn from_index(index: &FrameIndex) -> Self {
        let t = &index.times;
        let mut timing = Timing {
            frames: t.len(),
            ..Default::default()
        };
        if t.len() < 2 {
            return timing;
        }
        timing.duration = t[t.len() - 1] - t[0];
        if timing.duration > 0.0 {
            timing.mean_rate = (t.len() - 1) as f64 / timing.duration;
        }
        let mut intervals: Vec<f64> = t.windows(2).map(|w| w[1] - w[0]).collect();
        timing.backwards = intervals.iter().filter(|&&d| d < 0.0).count();
        timing.max_interval = intervals.iter().copied().fold(0.0, f64::max);
        let threshold_of = |median: f64| (median * GAP_FACTOR).max(MIN_GAP);
        let gaps = |median: f64| {
            let limit = threshold_of(median);
            t.windows(2)
                .enumerate()
                .filter(|(_, w)| w[1] - w[0] > limit)
                .map(|(i, w)| Gap {
                    frame: i + 1,
                    at: w[0],
                    seconds: w[1] - w[0],
                })
                .collect()
        };
        intervals.sort_by(f64::total_cmp);
        let median = intervals[intervals.len() / 2];
        timing.median_interval = median;
        timing.sample_rate = if median > 0.0 { 1.0 / median } else { 0.0 };
        timing.gaps = gaps(median);
        timing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A header string as written: u64 length, then the bytes.
    fn string(out: &mut Vec<u8>, s: &[u8]) {
        out.extend((s.len() as u64).to_le_bytes());
        out.extend(s);
    }

    #[test]
    fn reads_strings_longer_than_255_bytes() {
        let mut data = Vec::new();
        string(&mut data, b"additionaltags");
        string(&mut data, &[b'x'; 300]);
        let mut c = Cursor::new(&data, 0);
        assert_eq!(read_string(&mut c).unwrap(), "additionaltags");
        assert_eq!(read_string(&mut c).unwrap().len(), 300);
        assert_eq!(c.pos(), data.len());
    }

    #[test]
    fn rejects_a_string_length_past_the_end_of_the_data() {
        let mut data = u64::MAX.to_le_bytes().to_vec();
        data.extend(b"short");
        let mut c = Cursor::new(&data, 0);
        assert!(matches!(
            read_string(&mut c),
            Err(Error::InvalidStringLength(0))
        ));
    }

    #[test]
    fn parses_game_versions() {
        let v = GameVersion::parse("Y11S3_Alpha04", 9883691);
        assert_eq!(v.season.as_deref(), Some("Y11S3"));
        assert_eq!((v.year, v.season_number), (Some(11), Some(3)));
        assert_eq!(v.branch, "Alpha04");
        let v = GameVersion::parse("Y8S1", 1);
        assert_eq!(v.season.as_deref(), Some("Y8S1"));
        assert_eq!(v.branch, "");
        let v = GameVersion::parse("weird", 1);
        assert_eq!(v.season, None);
        assert_eq!(v.raw, "weird");
    }

    fn index(times: &[f64]) -> Vec<u8> {
        let mut b = vec![0u8; 8];
        b.extend((times.len() as u32).to_le_bytes());
        for (i, t) in times.iter().enumerate() {
            b.extend((i as u32).to_le_bytes());
            b.extend(t.to_le_bytes());
        }
        b
    }

    #[test]
    fn finds_gaps_in_the_frame_index() {
        let mut times: Vec<f64> = (0..100).map(|i| f64::from(i) / 30.0).collect();
        for t in &mut times[50..] {
            *t += 2.0;
        }
        let idx = read_frame_index(&index(&times), 100).unwrap();
        let timing = Timing::from_index(&idx);
        assert_eq!(timing.frames, 100);
        assert!((timing.sample_rate - 30.0).abs() < 0.01);
        // The gap drags the mean rate down; the median ignores it.
        assert!(timing.mean_rate < 20.0, "{}", timing.mean_rate);
        assert_eq!(timing.gaps.len(), 1);
        assert_eq!(timing.gaps[0].frame, 50);
        assert!((timing.gaps[0].seconds - (2.0 + 1.0 / 30.0)).abs() < 1e-9);
        let mut timing = timing;
        let start = DateTime::from_timestamp(1_000_000, 0).unwrap();
        let end = start + Duration::milliseconds((timing.duration * 1000.0) as i64);
        // Header written three hours behind UTC.
        assert_eq!(
            timing.calibrate(start - Duration::hours(3), None, Some(end)),
            None
        );
        assert_eq!(timing.header_utc_offset_minutes, Some(-180));
        // An end ten seconds later than the frames account for is reported.
        let late = end + Duration::seconds(10);
        assert!(timing.calibrate(start, Some(start), Some(late)).is_some());
        // A count that disagrees with the prelude is not an index.
        assert!(read_frame_index(&index(&times), 99).is_none());
    }
}
