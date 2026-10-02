//! Pauses within a round and the breaks between rounds (Y11S3).
//!
//! A custom match can be paused by its host. How the game records that is
//! not known: none of the 185 rounds looked at holds a pause (10 of a
//! custom match recorded by a spectator, 175 of matchmaking), and no
//! property is named after one. So nothing here is read; it is inferred
//! from how the recording behaves, with thresholds set well above the
//! worst an unpaused round showed, and every finding says `inferred`.
//!
//! # The clock
//!
//! The clock object of the `state` stream (see [`crate::activity`]) writes
//! `TimerInMilliseconds` (`1837466c`), what is left of the timer that
//! runs, every dozen frames and at every frame near a whole second, and
//! `TimerState` (`bb094fd3`): 0, 1 for the last seconds, 3 once the round
//! is decided. The frame index (see [`crate::format`]) says how many
//! seconds into the recording each frame is. Between two writes of one
//! timer the clock drops by as much as the recording moved on: over the
//! 185 rounds the recording was at most 0.128 s ahead of the clock over
//! one step, and 0.530 s over a whole timer.
//!
//! A new timer is no stall. The clock restarts when action starts and
//! when the defuser is planted: it rises, or `IsDefuserStarted`
//! (`ff39f408`) turns 1 between the two writes. Nor is a clock at zero:
//! it stands there while a plant finishes, and the frame that decides the
//! round writes 0 just before `TimerState` 3.
//!
//! # What counts
//!
//! - `clockStall`: the recording moved on [`STALL`] seconds more than the
//!   clock did between two writes of one timer, both above zero, before
//!   the round was decided.
//! - `clockSlow`: the same summed over the steps of one timer, from where
//!   the clock was furthest ahead, reaching [`SLOW`] seconds.
//! - `clockTail`: the clock was last written [`STALL`] seconds or more
//!   before the end of a recording whose round was not decided, with more
//!   than that left on it.
//! - `dataHole`: [`HOLE`] seconds without a record of the `movement`
//!   stream, which has one at every update, before the round was decided.
//! - `indexGap`: two frames of the index [`HOLE`] seconds apart.
//! - `suspended`: the header's `endtime` less `starttime` is
//!   [`SUSPENDED`] seconds more than the index covers, after taking off
//!   what the clock jumped ahead. It has no place in the recording.
//!
//! Everyone standing still is not a pause: it happens for seconds in
//! rounds that were never paused. A finding says what the streams did
//! meanwhile (`behaviour`): no movement records for most of it, records
//! in which no body moved, or bodies moving throughout.
//!
//! Skipped game time (see [`crate::format::Skip`]) is the opposite: the
//! clock jumps ahead of the recording. It is summed here from the steps
//! where the clock dropped [`AHEAD`] seconds more than the recording moved
//! on, to take it off the header's length.
//!
//! A `TimerState` other than 0, 1 and 3 has never been written. One that
//! is would be the first sign of a pause the game records itself, so it
//! is kept with its time for `decodeStatus`.
//!
//! # Between rounds
//!
//! Each round's header has when its recording started and ended
//! (`starttime`, `endtime`), so the break after a round is the next
//! round's start less this round's end. What a break should take differs
//! per match, and operator bans and the side switch make it longer, so a
//! break is compared with the median of the match's plain ones.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::details::{Ban, Phase};
use crate::feedback::display_clock;
use crate::round::Round;

/// Seconds the recording moves on more than the clock, between two clock
/// writes, for a stall. The most in 185 unpaused rounds is 0.128.
pub const STALL: f64 = 1.0;
/// The same summed within one timer. The most in 185 rounds is 0.530.
pub const SLOW: f64 = 1.5;
/// Seconds without a movement record, or between two frames of the index.
/// No round has more than 0.5 before it is decided.
pub const HOLE: f64 = 1.0;
/// Seconds the header's length exceeds the index and the clock's jumps
/// ahead. The most in 185 rounds is 0.963.
pub const SUSPENDED: f64 = 2.0;
/// A clock that drops this many seconds more than the recording moved on
/// jumped ahead.
pub const AHEAD: f64 = 0.2;
/// A stream silent for this much of a finding stood still through it.
const STILL: f64 = 0.5;
/// A rise of the clock by more than this many milliseconds is a new timer.
const NEW_TIMER: u32 = 50;
/// A hole or gap within this many seconds of a finding's ends is that
/// finding.
const SAME: f64 = 0.2;
/// The `TimerState` values written in rounds that were not paused.
pub const KNOWN_TIMER_STATES: [u32; 3] = [0, 1, 3];
const DECIDED: u32 = 3;
/// A break this many seconds over the match's plain breaks, with no ban
/// phase and no side switch, may hold a pause. Ranked breaks stay within
/// 17.1 s of their match's median.
pub const LONG_BREAK: f64 = 20.0;
/// Fewer plain breaks than this give no level to compare with.
const MIN_PLAIN_BREAKS: usize = 3;

/// One write of `TimerInMilliseconds` by a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClockWrite {
    /// Seconds since the recording started.
    pub time: f64,
    /// What was left of the timer.
    pub ms: u32,
    /// `TimerState` once the frame was written.
    pub state: Option<u32>,
}

/// What the frames wrote of the round clock, in order. Times are seconds
/// since the recording started.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClockTrack {
    pub writes: Vec<ClockWrite>,
    /// Every `TimerState` written, with when.
    pub states: Vec<(f64, u32)>,
    /// When `IsDefuserStarted` was written 1: the defuser timer starts.
    pub plants: Vec<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    ClockStall,
    ClockSlow,
    ClockTail,
    DataHole,
    IndexGap,
    Suspended,
}

/// How a finding was come by. Nothing is read from the file so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    #[default]
    Inferred,
}

/// What the movement stream did during a finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Behaviour {
    /// It has no record for most of it.
    NoRecords,
    /// It has records, and no player's body moved for most of it.
    FrozenRecords,
    /// Bodies moved throughout.
    MovingRecords,
}

/// What a finding rests on.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    /// Seconds the recording moved on more than the clock did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock_lag: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behaviour: Option<Behaviour>,
    /// Records of the movement stream between `start` and `end`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub movement_records: Option<usize>,
    /// Seconds between the two frames of the index, when it has a gap
    /// there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_gap: Option<f64>,
    /// `suspended`: the header's length less the index's, and what the
    /// clock jumped ahead in all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_minus_index: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock_ahead: Option<f64>,
}

/// A stretch the round may have been paused for.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pause {
    pub kind: Kind,
    /// Seconds since the recording started. A `suspended` recording has
    /// neither: where it stood still is not known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
    /// Seconds the round stood still: the clock's lag, or the length of
    /// the hole.
    pub duration: f64,
    /// `start` and `end` in UTC, when the header says when the recording
    /// started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
    /// The round clock at `start`, and what was left of the timer in
    /// milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock_ms: Option<u32>,
    pub source: Source,
    pub evidence: Evidence,
}

/// What [`detect`] found, with the measurements the thresholds are set
/// against.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub pauses: Vec<Pause>,
    /// Clock writes read. Without any the round has no clock track.
    pub clock_writes: usize,
    /// The most the recording moved on more than the clock over one step,
    /// and summed within one timer.
    pub max_step_lag: f64,
    pub max_cumulative_lag: f64,
    /// Seconds the clock jumped ahead of the recording, in all.
    pub clock_ahead: f64,
    /// When `TimerState` said the round was decided.
    pub decided_at: Option<f64>,
    /// The header's `endtime` less `starttime`, less what the index covers.
    pub header_minus_index: Option<f64>,
    /// `(time, value)` of every `TimerState` outside
    /// [`KNOWN_TIMER_STATES`].
    pub unknown_states: Vec<(f64, u32)>,
}

/// What [`detect`] reads. Times are seconds since the recording started,
/// each list in order.
#[derive(Clone, Copy, Debug)]
pub struct Input<'a> {
    pub clock: &'a ClockTrack,
    /// The frame index.
    pub frame_times: &'a [f64],
    /// When the movement stream has a record.
    pub movement: &'a [f64],
    /// When a player's body was somewhere else than a moment before.
    pub moves: &'a [f64],
    /// When each phase started.
    pub phases: &'a [(f64, Phase)],
    /// The header's `endtime` less `starttime`, in seconds.
    pub header_seconds: Option<f64>,
    /// When the recording started.
    pub started: Option<DateTime<Utc>>,
}

fn millis(seconds: f64) -> f64 {
    (seconds * 1000.0).round() / 1000.0
}

/// How many of the sorted `times` lie strictly between `from` and `to`.
fn between(times: &[f64], from: f64, to: f64) -> usize {
    let first = times.partition_point(|&t| t <= from);
    let end = times.partition_point(|&t| t < to);
    end.saturating_sub(first)
}

/// The longest stretch from `from` to `to` without one of the sorted
/// `times`.
fn longest_gap(times: &[f64], from: f64, to: f64) -> f64 {
    let first = times.partition_point(|&t| t <= from);
    let end = times.partition_point(|&t| t < to);
    let inside = times.get(first..end).unwrap_or_default();
    let mut last = from;
    let mut longest: f64 = 0.0;
    for &t in inside.iter().chain([&to]) {
        longest = longest.max(t - last);
        last = t;
    }
    longest
}

impl<'a> Input<'a> {
    /// A clock track and a frame index, with nothing else known.
    pub fn new(clock: &'a ClockTrack, frame_times: &'a [f64]) -> Self {
        Input {
            clock,
            frame_times,
            movement: &[],
            moves: &[],
            phases: &[],
            header_seconds: None,
            started: None,
        }
    }

    fn duration(&self) -> f64 {
        match (self.frame_times.first(), self.frame_times.last()) {
            (Some(first), Some(last)) => last - first,
            _ => 0.0,
        }
    }

    fn phase(&self, time: f64) -> Phase {
        let i = self.phases.partition_point(|p| p.0 <= time);
        let at = i.checked_sub(1).and_then(|i| self.phases.get(i));
        at.map_or(Phase::Prep, |p| p.1)
    }

    /// The last clock write at or before `time`.
    fn clock_at(&self, time: f64) -> Option<u32> {
        let writes = &self.clock.writes;
        let i = writes.partition_point(|w| w.time <= time);
        i.checked_sub(1).and_then(|i| writes.get(i)).map(|w| w.ms)
    }

    fn utc(&self, seconds: f64) -> Option<String> {
        let at = self.started? + Duration::milliseconds((seconds * 1000.0).round() as i64);
        Some(at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
    }

    /// A finding from `start` to `end`, `clock_ms` on the clock at its
    /// start.
    fn pause(&self, kind: Kind, start: f64, end: f64, duration: f64, clock_ms: u32) -> Pause {
        // Two clock writes are a few frames apart, so a stall has records
        // at both ends: the streams stood still when they are silent for
        // most of what it took.
        let silent = |times: &[f64]| longest_gap(times, start, end) >= duration * STILL;
        let behaviour = match (silent(self.movement), silent(self.moves)) {
            (true, _) => Behaviour::NoRecords,
            (false, true) => Behaviour::FrozenRecords,
            (false, false) => Behaviour::MovingRecords,
        };
        Pause {
            kind,
            start: Some(millis(start)),
            end: Some(millis(end)),
            duration: millis(duration),
            started_at: self.utc(start),
            ended_at: self.utc(end),
            phase: Some(self.phase(start)),
            time: Some(display_clock(f64::from(clock_ms / 1000))),
            clock_ms: Some(clock_ms),
            source: Source::Inferred,
            evidence: Evidence {
                behaviour: Some(behaviour),
                movement_records: Some(between(self.movement, start, end)),
                ..Evidence::default()
            },
        }
    }
}

/// Looks for the stretches a round stood still (see the module).
pub fn detect(input: &Input) -> Report {
    let clock = input.clock;
    let mut report = Report {
        clock_writes: clock.writes.len(),
        ..Report::default()
    };
    // The round is decided from the first `TimerState` 3 that no other
    // state follows.
    for &(time, state) in &clock.states {
        if !KNOWN_TIMER_STATES.contains(&state) {
            report.unknown_states.push((time, state));
        }
        if state == DECIDED {
            report.decided_at.get_or_insert(time);
        } else if report.decided_at.is_some_and(|d| time > d) {
            report.decided_at = None;
        }
    }
    let decided = report.decided_at;

    // The steps of each timer. `sum` is the lag since the timer started,
    // `low` the least it was and `low_at` the write it was that at: the
    // clock has been slow by `sum - low` since.
    let mut prev: Option<ClockWrite> = None;
    let (mut sum, mut low, mut low_at) = (0.0, 0.0, None::<ClockWrite>);
    let mut slow_reported = false;
    for &write in &clock.writes {
        if write.state == Some(DECIDED) || decided.is_some_and(|d| write.time >= d) {
            prev = None;
            continue;
        }
        let Some(before) = prev.replace(write) else {
            (sum, low, low_at) = (0.0, 0.0, Some(write));
            continue;
        };
        let planted = |&t: &f64| before.time < t && t <= write.time;
        if write.ms > before.ms.saturating_add(NEW_TIMER) || clock.plants.iter().any(planted) {
            (sum, low, low_at) = (0.0, 0.0, Some(write));
            slow_reported = false;
            continue;
        }
        // A clock at zero waits for a plant or for the round to end.
        if before.ms == 0 || write.ms == 0 {
            continue;
        }
        let dropped = (f64::from(before.ms) - f64::from(write.ms)) / 1000.0;
        let lag = (write.time - before.time) - dropped;
        report.max_step_lag = report.max_step_lag.max(lag);
        if lag <= -AHEAD {
            report.clock_ahead -= lag;
        }
        if lag >= STALL {
            let mut pause = input.pause(Kind::ClockStall, before.time, write.time, lag, before.ms);
            pause.evidence.clock_lag = Some(millis(lag));
            report.pauses.push(pause);
            (sum, low, low_at) = (0.0, 0.0, Some(write));
            continue;
        }
        sum += lag;
        if sum < low {
            (low, low_at) = (sum, Some(write));
            slow_reported = false;
        }
        let slow = sum - low;
        report.max_cumulative_lag = report.max_cumulative_lag.max(slow);
        if let (true, false, Some(from)) = (slow >= SLOW, slow_reported, low_at) {
            let mut pause = input.pause(Kind::ClockSlow, from.time, write.time, slow, from.ms);
            pause.evidence.clock_lag = Some(millis(slow));
            report.pauses.push(pause);
            slow_reported = true;
        }
    }

    // The clock stops being written while the round is still on.
    let end = input.frame_times.last().copied().unwrap_or(0.0);
    if let (Some(last), None) = (prev.filter(|w| w.ms > 0), decided) {
        let tail = end - last.time;
        if tail >= STALL && f64::from(last.ms) / 1000.0 > tail {
            let mut pause = input.pause(Kind::ClockTail, last.time, end, tail, last.ms);
            pause.evidence.clock_lag = Some(millis(tail));
            report.pauses.push(pause);
        }
    }

    // Holes in the movement stream before the decision, and gaps in the
    // index, that are not one of the findings above.
    let covered = |pauses: &[Pause], from: f64, to: f64| {
        pauses.iter().position(|p| {
            p.start.is_some_and(|s| s <= from + SAME) && p.end.is_some_and(|e| e >= to - SAME)
        })
    };
    let limit = decided.unwrap_or(end);
    for w in input.movement.windows(2) {
        let (Some(&from), Some(&to)) = (w.first(), w.get(1)) else {
            continue;
        };
        if to - from >= HOLE && from < limit && covered(&report.pauses, from, to).is_none() {
            let left = input.clock_at(from);
            let mut pause = input.pause(Kind::DataHole, from, to, to - from, left.unwrap_or(0));
            if left.is_none() {
                (pause.time, pause.clock_ms) = (None, None);
            }
            report.pauses.push(pause);
        }
    }
    for w in input.frame_times.windows(2) {
        let (Some(&from), Some(&to)) = (w.first(), w.get(1)) else {
            continue;
        };
        if to - from < HOLE {
            continue;
        }
        let at = covered(&report.pauses, from, to).and_then(|i| report.pauses.get_mut(i));
        if let Some(pause) = at {
            pause.evidence.index_gap = Some(millis(to - from));
            continue;
        }
        let left = input.clock_at(from);
        let mut pause = input.pause(Kind::IndexGap, from, to, to - from, left.unwrap_or(0));
        if left.is_none() {
            (pause.time, pause.clock_ms) = (None, None);
        }
        pause.evidence.index_gap = Some(millis(to - from));
        report.pauses.push(pause);
    }
    report
        .pauses
        .sort_by(|a, b| a.start.unwrap_or(0.0).total_cmp(&b.start.unwrap_or(0.0)));

    // The recording as a whole took longer than its frames account for.
    if let Some(header) = input.header_seconds.filter(|_| input.frame_times.len() > 1) {
        let over = header - input.duration();
        report.header_minus_index = Some(millis(over));
        let unexplained = over - report.clock_ahead;
        if unexplained >= SUSPENDED {
            report.pauses.push(Pause {
                kind: Kind::Suspended,
                start: None,
                end: None,
                duration: millis(unexplained),
                started_at: None,
                ended_at: None,
                phase: None,
                time: None,
                clock_ms: None,
                source: Source::Inferred,
                evidence: Evidence {
                    header_minus_index: Some(millis(over)),
                    clock_ahead: Some(millis(report.clock_ahead)),
                    ..Evidence::default()
                },
            });
        }
    }
    report.clock_ahead = millis(report.clock_ahead);
    report
}

/// The break between two consecutive rounds of a match.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Break {
    /// The round it follows, counting from 1.
    pub after_round: u32,
    /// Seconds from the end of that round's recording to the start of the
    /// next one's.
    pub duration: f64,
    /// What the match's plain breaks take: the median of those with no ban
    /// phase, no side switch and no overtime round on either side. Absent
    /// when the match has fewer than three such breaks. Inferred.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<f64>,
    /// `duration` less `expected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excess: Option<f64>,
    /// The bans in force differ between the two rounds: operators were
    /// banned in the break. Absent when the bans were not read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ban_phase: Option<bool>,
    /// The teams changed sides. Absent when the sides were not read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side_switch: Option<bool>,
    /// The round before or the round after is an overtime round.
    pub overtime: bool,
    /// The break is more than [`LONG_BREAK`] seconds over `expected` with
    /// no ban phase and no side switch to explain it. Inferred, and absent
    /// when one of those is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pause_suspected: Option<bool>,
}

impl Break {
    /// Neither bans, a side switch nor overtime make it longer or shorter.
    fn plain(&self) -> bool {
        self.ban_phase == Some(false) && self.side_switch == Some(false) && !self.overtime
    }
}

/// What makes a ban the same ban in the next round.
fn ban_key(b: &Ban) -> (Option<usize>, Option<u32>, Option<u64>, bool, u8) {
    (b.team, b.slot, b.icon, b.no_ban, b.role as u8)
}

/// The breaks between `rounds`, which are in play order. A pair with a
/// round missing in between, of two matches, or without the header's
/// times (before Y11S3) has no entry.
pub fn breaks(rounds: &[Round]) -> Vec<Break> {
    let read = |r: &Round, field: &str| {
        (r.decode.get(field)).is_some_and(|f| f.status != crate::Status::Skipped)
    };
    let mut out = Vec::new();
    for pair in rounds.windows(2) {
        let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        let (ha, hb) = (&a.header, &b.header);
        if ha.match_id != hb.match_id || hb.round_number != ha.round_number + 1 {
            continue;
        }
        let (Some(end), Some(start)) = (ha.end_time, hb.start_time) else {
            continue;
        };
        let bans = |r: &Round| {
            let mut keys: Vec<_> = r.bans.iter().map(ban_key).collect();
            keys.sort_unstable();
            keys
        };
        let sides = |r: &Round| {
            let teams = &r.header.teams;
            Some([teams.first()?.role?, teams.get(1)?.role?])
        };
        out.push(Break {
            after_round: ha.round_number + 1,
            duration: (start - end).num_milliseconds() as f64 / 1000.0,
            ban_phase: (read(a, "bans") && read(b, "bans")).then(|| bans(a) != bans(b)),
            side_switch: sides(a).zip(sides(b)).map(|(a, b)| a != b),
            overtime: a.info().overtime || b.info().overtime,
            ..Break::default()
        });
    }
    rate(&mut out);
    out
}

/// Compares each break with the plain ones of its match.
fn rate(breaks: &mut [Break]) {
    let mut plain: Vec<f64> = (breaks.iter())
        .filter(|b| b.plain())
        .map(|b| b.duration)
        .collect();
    if plain.len() < MIN_PLAIN_BREAKS {
        return;
    }
    plain.sort_by(f64::total_cmp);
    let mid = plain.len() / 2;
    let median = match (
        plain.get(mid),
        mid.checked_sub(1).and_then(|i| plain.get(i)),
    ) {
        (Some(&upper), Some(&lower)) if plain.len() % 2 == 0 => (upper + lower) / 2.0,
        (Some(&upper), _) => upper,
        _ => return,
    };
    for b in breaks {
        let excess = b.duration - median;
        b.expected = Some(millis(median));
        b.excess = Some(millis(excess));
        b.pause_suspected = (b.ban_phase.zip(b.side_switch))
            .map(|(bans, switch)| excess > LONG_BREAK && !bans && !switch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A timer counting down from `from_ms`, written every `step` seconds
    /// from `start` for `seconds`.
    fn countdown(start: f64, from_ms: u32, seconds: f64, step: f64) -> Vec<ClockWrite> {
        let n = (seconds / step) as u32;
        (0..=n)
            .map(|i| ClockWrite {
                time: start + f64::from(i) * step,
                ms: from_ms.saturating_sub((f64::from(i) * step * 1000.0).round() as u32),
                state: Some(0),
            })
            .collect()
    }

    /// Thirty frames a second for `seconds`.
    fn frames(seconds: f64) -> Vec<f64> {
        (0..=(seconds * 30.0) as u32)
            .map(|i| f64::from(i) / 30.0)
            .collect()
    }

    /// The recording stands still for `seconds` at `at`: everything later
    /// is that much later.
    fn stand_still(times: &mut [f64], at: f64, seconds: f64) {
        for t in times.iter_mut().filter(|t| **t > at) {
            *t += seconds;
        }
    }

    fn run(clock: &ClockTrack, frame_times: &[f64]) -> Report {
        detect(&Input {
            movement: frame_times,
            moves: frame_times,
            phases: &[(0.0, Phase::Prep), (46.0, Phase::Action)],
            ..Input::new(clock, frame_times)
        })
    }

    /// Prep, then action from 46 s into the recording.
    fn round(seconds: f64) -> ClockTrack {
        let mut writes = countdown(0.0, 45_000, 45.0, 0.4);
        writes.extend(countdown(46.0, 180_000, seconds - 46.0, 0.4));
        ClockTrack {
            writes,
            ..ClockTrack::default()
        }
    }

    #[test]
    fn an_unpaused_round_has_no_pause() {
        let report = run(&round(200.0), &frames(200.0));
        assert_eq!(report.pauses, []);
        assert!(report.max_step_lag < 1e-6 && report.max_cumulative_lag < 1e-6);
        assert_eq!(report.clock_writes, 499);
    }

    #[test]
    fn a_clock_that_stands_for_twelve_seconds_is_one_stall() {
        let mut clock = round(200.0);
        let mut at: Vec<f64> = clock.writes.iter().map(|w| w.time).collect();
        stand_still(&mut at, 100.0, 12.0);
        for (w, t) in clock.writes.iter_mut().zip(at) {
            w.time = t;
        }
        // The index goes on through it and bodies stand.
        let times = frames(212.0);
        let moves: Vec<f64> = (times.iter().copied())
            .filter(|t| !(100.0..112.0).contains(t))
            .collect();
        let started = DateTime::from_timestamp(1_000_000, 0);
        let report = detect(&Input {
            clock: &clock,
            frame_times: &times,
            movement: &times,
            moves: &moves,
            phases: &[(0.0, Phase::Prep), (46.0, Phase::Action)],
            header_seconds: Some(212.0),
            started,
        });
        let [pause] = report.pauses.as_slice() else {
            panic!("{:?}", report.pauses);
        };
        assert_eq!(pause.kind, Kind::ClockStall);
        assert!((pause.duration - 12.0).abs() < 0.001, "{pause:?}");
        let (start, end) = (pause.start.unwrap(), pause.end.unwrap());
        assert!((start - 100.0).abs() < 0.4 && (end - 112.4).abs() < 0.4);
        assert_eq!(pause.phase, Some(Phase::Action));
        // 54 s into action: 2:06 left.
        assert_eq!(pause.time.as_deref(), Some("2:06"));
        assert_eq!(pause.clock_ms, Some(126_000));
        assert_eq!(pause.evidence.clock_lag, Some(12.0));
        assert_eq!(pause.evidence.behaviour, Some(Behaviour::FrozenRecords));
        assert!(pause.evidence.movement_records.unwrap() > 300);
        assert_eq!(pause.evidence.index_gap, None);
        assert_eq!(
            pause.started_at.as_deref(),
            Some("1970-01-12T13:48:20.000Z")
        );
        assert_eq!(report.header_minus_index, Some(0.0));
        let json = serde_json::to_value(pause).unwrap();
        assert_eq!(json["kind"], "clockStall");
        assert_eq!(json["source"], "inferred");
        assert_eq!(json["evidence"]["behaviour"], "frozenRecords");
        assert!(json.get("by").is_none());
    }

    /// The recording itself stands still: no frames, no records, and the
    /// gap in the index is the stall's evidence, not a second finding.
    #[test]
    fn a_stall_without_records_names_the_index_gap() {
        let mut clock = round(200.0);
        let mut times = frames(200.0);
        stand_still(&mut times, 100.0, 12.0);
        let mut at: Vec<f64> = clock.writes.iter().map(|w| w.time).collect();
        stand_still(&mut at, 100.0, 12.0);
        for (w, t) in clock.writes.iter_mut().zip(at) {
            w.time = t;
        }
        let report = run(&clock, &times);
        let [pause] = report.pauses.as_slice() else {
            panic!("{:?}", report.pauses);
        };
        assert_eq!(pause.kind, Kind::ClockStall);
        assert_eq!(pause.evidence.behaviour, Some(Behaviour::NoRecords));
        assert!((pause.evidence.index_gap.unwrap() - 12.033).abs() < 0.001);
    }

    /// Without a clock the hole in the streams is what is left to see.
    #[test]
    fn a_hole_in_the_movement_stream_is_a_data_hole() {
        let clock = ClockTrack::default();
        let mut times = frames(100.0);
        let movement: Vec<f64> = (times.iter().copied())
            .filter(|t| !(50.0..53.0).contains(t))
            .collect();
        let report = detect(&Input {
            movement: &movement,
            ..Input::new(&clock, &times)
        });
        assert_eq!(report.pauses.len(), 1);
        assert_eq!(report.pauses[0].kind, Kind::DataHole);
        assert_eq!(report.pauses[0].time, None);
        assert!((report.pauses[0].duration - 3.033).abs() < 0.001);
        stand_still(&mut times, 20.0, 5.0);
        let report = run(&clock, &times);
        assert_eq!(report.pauses.len(), 1);
        assert_eq!(report.pauses[0].kind, Kind::DataHole);
        assert!(report.pauses[0].evidence.index_gap.is_some());
    }

    #[test]
    fn a_new_timer_is_not_a_stall() {
        // Action starts 3 s after prep ran out; the clock rises.
        let mut writes = countdown(0.0, 45_000, 44.8, 0.4);
        writes.extend(countdown(48.0, 180_000, 100.0, 0.4));
        let clock = ClockTrack {
            writes,
            ..ClockTrack::default()
        };
        let report = run(&clock, &frames(148.0));
        assert_eq!(report.pauses, []);
        assert!(report.max_step_lag < 1e-6, "{}", report.max_step_lag);
    }

    #[test]
    fn a_plant_is_neither_a_stall_nor_a_skip() {
        // Planted with 2:00.4 left: the clock drops to the defuser timer,
        // which is written 2 s later.
        let mut writes = countdown(0.0, 180_000, 59.6, 0.4);
        writes.extend(countdown(62.0, 44_940, 30.0, 0.4));
        let clock = ClockTrack {
            writes,
            plants: vec![61.0],
            ..ClockTrack::default()
        };
        let report = run(&clock, &frames(92.0));
        assert_eq!(report.pauses, []);
        assert_eq!(report.clock_ahead, 0.0);
        // Without the plant the drop reads as the clock jumping ahead.
        let unplanted = ClockTrack {
            plants: Vec::new(),
            ..clock
        };
        assert!(run(&unplanted, &frames(92.0)).clock_ahead > 70.0);
    }

    #[test]
    fn a_clock_at_zero_is_not_a_stall() {
        // The action clock runs out while a plant finishes 6 s later.
        let mut writes = countdown(0.0, 10_000, 10.0, 0.4);
        writes.extend([10.4, 10.8, 13.0, 16.0].map(|time| ClockWrite {
            time,
            ms: 0,
            state: Some(1),
        }));
        writes.extend(countdown(16.4, 44_940, 20.0, 0.4));
        let clock = ClockTrack {
            writes,
            plants: vec![16.4],
            ..ClockTrack::default()
        };
        assert_eq!(run(&clock, &frames(36.4)).pauses, []);
    }

    #[test]
    fn the_end_of_the_round_is_not_a_stall() {
        // Decided at 60 s: the clock is zeroed with `TimerState` 3, and
        // the recording goes on for 8 s without a clock or movement.
        let mut clock = ClockTrack {
            writes: countdown(0.0, 180_000, 59.6, 0.4),
            states: vec![(60.0, 3)],
            ..ClockTrack::default()
        };
        clock.writes.push(ClockWrite {
            time: 60.0,
            ms: 0,
            state: Some(3),
        });
        let times = frames(68.0);
        let report = detect(&Input {
            movement: &times[..(61.0 * 30.0) as usize],
            ..Input::new(&clock, &times)
        });
        assert_eq!(report.pauses, []);
        assert_eq!(report.decided_at, Some(60.0));
        // The same recording with the round still on has lost its clock.
        clock.states.clear();
        clock.writes.pop();
        let report = run(&clock, &times);
        assert_eq!(report.pauses.len(), 1);
        assert_eq!(report.pauses[0].kind, Kind::ClockTail);
        assert!((report.pauses[0].duration - 8.4).abs() < 0.001);
    }

    #[test]
    fn a_skip_is_the_clock_ahead_and_no_pause() {
        // 3 s of game time missing at 100 s: the clock is 3 s further on.
        let mut clock = round(200.0);
        for w in clock.writes.iter_mut().filter(|w| w.time > 100.0) {
            w.ms -= 3000;
        }
        let times = frames(200.0);
        let report = detect(&Input {
            movement: &times,
            header_seconds: Some(203.5),
            ..Input::new(&clock, &times)
        });
        assert_eq!(report.pauses, []);
        assert_eq!(report.clock_ahead, 3.0);
        assert_eq!(report.header_minus_index, Some(3.5));
        // The same header over a clock that never jumped: the recording
        // stood still somewhere.
        let clock = round(200.0);
        let report = detect(&Input {
            movement: &times,
            header_seconds: Some(203.5),
            ..Input::new(&clock, &times)
        });
        let [pause] = report.pauses.as_slice() else {
            panic!("{:?}", report.pauses);
        };
        assert_eq!((pause.kind, pause.duration), (Kind::Suspended, 3.5));
        assert_eq!((pause.start, pause.phase), (None, None));
        assert_eq!(pause.evidence.header_minus_index, Some(3.5));
    }

    #[test]
    fn a_clock_that_runs_slow_adds_up() {
        // Each step of 0.4 s takes 0.1 s longer for a while: no step is a
        // stall, 20 of them are 2 s.
        let mut clock = round(200.0);
        let mut late = 0.0;
        for w in &mut clock.writes {
            if (100.0..108.0).contains(&w.time) {
                late += 0.1;
            }
            w.time += late;
        }
        let report = run(&clock, &frames(202.0));
        let [pause] = report.pauses.as_slice() else {
            panic!("{:?}", report.pauses);
        };
        assert_eq!(pause.kind, Kind::ClockSlow);
        assert!((pause.duration - SLOW).abs() < 0.1, "{pause:?}");
        assert!((report.max_cumulative_lag - 2.0).abs() < 0.001);
    }

    #[test]
    fn an_unknown_timer_state_is_kept() {
        let clock = ClockTrack {
            states: vec![(1.0, 0), (50.0, 2), (60.0, 1), (70.0, 3)],
            ..ClockTrack::default()
        };
        let report = run(&clock, &frames(80.0));
        assert_eq!(report.unknown_states, [(50.0, 2)]);
        assert_eq!(report.decided_at, Some(70.0));
        assert_eq!(report.clock_writes, 0);
    }

    fn plain(after_round: u32, duration: f64) -> Break {
        Break {
            after_round,
            duration,
            ban_phase: Some(false),
            side_switch: Some(false),
            ..Break::default()
        }
    }

    #[test]
    fn a_long_break_without_bans_is_suspected() {
        let mut breaks: Vec<Break> = [31.6, 28.2, 95.4, 31.7, 27.9, 180.9, 30.5, 73.1, 59.0]
            .iter()
            .zip(1..)
            .map(|(&d, n)| plain(n, d))
            .collect();
        for i in [2, 5, 8] {
            breaks[i].ban_phase = Some(true);
        }
        rate(&mut breaks);
        // The median of 27.9, 28.2, 30.5, 31.6, 31.7 and 73.1.
        assert!(breaks.iter().all(|b| b.expected == Some(31.05)));
        assert_eq!(breaks[7].excess, Some(42.05));
        let suspected: Vec<u32> = (breaks.iter())
            .filter(|b| b.pause_suspected == Some(true))
            .map(|b| b.after_round)
            .collect();
        assert_eq!(suspected, [8]);
    }

    #[test]
    fn too_few_plain_breaks_give_no_level() {
        let mut breaks = vec![plain(1, 30.0), plain(2, 90.0)];
        breaks.push(Break {
            overtime: true,
            ..plain(3, 20.0)
        });
        rate(&mut breaks);
        assert!(breaks.iter().all(|b| b.expected.is_none()));
        assert!(breaks.iter().all(|b| b.pause_suspected.is_none()));
        // Bans that were not read leave the question open.
        let mut breaks = vec![plain(1, 30.0), plain(2, 30.0), plain(3, 30.0)];
        breaks.push(Break {
            ban_phase: None,
            ..plain(4, 90.0)
        });
        rate(&mut breaks);
        assert_eq!(breaks[3].excess, Some(60.0));
        assert_eq!(breaks[3].pause_suspected, None);
        assert_eq!(breaks[0].pause_suspected, Some(false));
    }
}
