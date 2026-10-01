//! One continuous clock for a round.
//!
//! The in-game clock restarts twice: prep counts down from 45 seconds, action
//! from the round length, and a plant switches it to the defuser timer. When
//! the round is decided it drops to 0:00. The parser records every distinct
//! clock reading (a *tick*) in stream order and tags each event with the tick
//! it was read at; this module turns the ticks into phases and seconds elapsed
//! since prep started, so every event and sample sits on one timeline.

use serde::Serialize;

use crate::details::Phase;
use crate::feedback::display_clock;

/// Action phases start well above the 45 second prep phase.
const ACTION_START: f64 = 50.0;

/// What one clock reading resolves to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TickInfo {
    pub phase: Phase,
    /// Seconds since prep started.
    pub elapsed: f64,
    /// The clock to show for events at this tick. At the end tick this is the
    /// last live reading, not the 0:00 the game resets to: the kill or
    /// disable that ended the round happened then.
    pub seconds: f64,
}

/// A stretch of the round in one phase.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseSpan {
    pub phase: Phase,
    /// Round clock when the phase started and when it ended.
    pub start_time: String,
    pub end_time: String,
    /// Seconds since prep started.
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub start: f64,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub end: f64,
    /// Seconds since the recording started when the phase started and ended,
    /// to the frame (Y8S4+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_start: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_end: Option<f64>,
}

/// The resolved clock track of a round.
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    pub ticks: Vec<TickInfo>,
    /// Tick where the action phase started.
    pub action_start: Option<usize>,
    /// Tick where the defuser timer started.
    pub plant_start: Option<usize>,
    /// Tick where the round was decided.
    pub end_start: Option<usize>,
    /// Seconds since the recording started at each tick, when known.
    pub recording: Vec<Option<f64>>,
}

impl Timeline {
    /// Resolves `readings` (clock values in stream order, each different
    /// from the one before except an `end` tick at a clock already at 0:00).
    /// `plant` is the tick at which the plant completed, if it did, and
    /// `end` the tick at which the game said the round was decided (Y11S3+).
    /// `recording` gives seconds since the recording started per tick, where
    /// known: the clock itself cannot say how long a new timer took to take
    /// over, or how long it stood at 0:00.
    pub fn resolve(
        readings: &[f64],
        plant: Option<usize>,
        end: Option<usize>,
        recording: &[Option<f64>],
    ) -> Self {
        let n = readings.len();
        // Whole seconds between two ticks on the recording's clock.
        let measured = |i: usize| -> Option<f64> {
            let now = (*recording.get(i)?)?;
            let before = (*recording.get(i - 1)?)?;
            Some((now - before).round().max(0.0))
        };
        let jumped_up = |i: usize| i > 0 && readings[i] > readings[i - 1] + 1.0;
        let action_start = (0..n).find(|&i| readings[i] > ACTION_START && (i == 0 || jumped_up(i)));
        // The round ends on its final 0:00, once action has started.
        let end_start = end
            .filter(|&e| e < n)
            .or(match (action_start, readings.last()) {
                (Some(a), Some(&last)) if last == 0.0 && n - 1 > a => Some(n - 1),
                _ => None,
            });
        let live_end = end_start.unwrap_or(n);
        // The defuser timer takes over at the first break in the countdown
        // after the plant (down from a late plant, up from one at 0:00); if
        // the round ended first, at the plant tick itself.
        let plant_start = plant.filter(|&p| p < live_end).map(|p| {
            // The plant was read after the clock had switched already.
            if p > 0 && readings[p] != readings[p - 1] - 1.0 {
                return p;
            }
            (p + 1..live_end)
                .find(|&i| readings[i] != readings[i - 1] - 1.0)
                .unwrap_or(p + 1)
        });

        let mut ticks = Vec::with_capacity(n);
        let mut elapsed = 0.0;
        // The clock dropping straight to 0:00 means the round was decided
        // then; counting down to it means time ran out.
        let abrupt_end = end_start.is_some_and(|e| e > 0 && readings[e - 1] > 1.0);
        for i in 0..n {
            if i > 0 && !(Some(i) == end_start && abrupt_end) {
                let step = readings[i - 1] - readings[i];
                // Counting down (gaps included). A new timer (action, or the
                // defuser timer) takes over within a second of the last one,
                // unless that one stood at 0:00 while a plant finished.
                elapsed += if step > 0.0 && Some(i) != plant_start {
                    step
                } else {
                    measured(i).unwrap_or(1.0)
                };
            }
            let phase = if Some(i) >= end_start && end_start.is_some() {
                Phase::End
            } else if plant_start.is_some_and(|p| i >= p) {
                Phase::Planted
            } else if action_start.is_some_and(|a| i >= a) {
                Phase::Action
            } else {
                Phase::Prep
            };
            let seconds = if Some(i) == end_start && abrupt_end {
                readings[i - 1]
            } else {
                readings[i]
            };
            ticks.push(TickInfo {
                phase,
                elapsed,
                seconds,
            });
        }
        Self {
            ticks,
            action_start,
            plant_start,
            end_start,
            recording: Vec::new(),
        }
    }

    /// Phase, elapsed and clock for an event read at `tick`. Events read
    /// before the first clock reading belong to the start of prep.
    pub fn at(&self, tick: Option<usize>) -> TickInfo {
        tick.and_then(|t| self.ticks.get(t).copied())
            .unwrap_or(TickInfo {
                phase: Phase::Prep,
                elapsed: 0.0,
                seconds: self.ticks.first().map_or(0.0, |t| t.seconds),
            })
    }

    /// Like [`Timeline::at`], for an event read `recording` seconds into the
    /// recording. A clock standing at 0:00 while a plant finishes shows one
    /// reading for several seconds; the recording's clock places events in
    /// that stretch.
    pub fn at_time(&self, tick: Option<usize>, recording: Option<f64>) -> TickInfo {
        let mut at = self.at(tick);
        let Some(t) = tick else { return at };
        let stood = at.seconds == 0.0 && at.phase != Phase::End;
        if let (true, Some(now), Some(Some(then))) = (stood, recording, self.recording.get(t)) {
            let limit = self.ticks.get(t + 1).map_or(f64::MAX, |n| n.elapsed);
            at.elapsed = (at.elapsed + (now - then).floor().max(0.0)).min(limit);
        }
        at
    }

    /// Seconds from prep start to the last reading.
    pub fn duration(&self) -> f64 {
        self.ticks.last().map_or(0.0, |t| t.elapsed)
    }

    /// The phases in order, with when each started and ended.
    pub fn spans(&self) -> Vec<PhaseSpan> {
        let mut spans: Vec<PhaseSpan> = Vec::new();
        for (i, t) in self.ticks.iter().enumerate() {
            let recording = self.recording.get(i).copied().flatten();
            match spans.last_mut() {
                Some(s) if s.phase == t.phase => {
                    s.end = t.elapsed;
                    s.end_time = display_clock(t.seconds);
                    s.recording_end = recording.or(s.recording_end);
                }
                _ => {
                    if let Some(s) = spans.last_mut() {
                        s.end = t.elapsed;
                        s.recording_end = recording.or(s.recording_end);
                    }
                    spans.push(PhaseSpan {
                        phase: t.phase,
                        start_time: display_clock(t.seconds),
                        end_time: display_clock(t.seconds),
                        start: t.elapsed,
                        end: t.elapsed,
                        recording_start: recording,
                        recording_end: recording,
                    });
                }
            }
        }
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn countdown(from: u32, to: u32) -> Vec<f64> {
        (to..=from).rev().map(f64::from).collect()
    }

    #[test]
    fn prep_action_plant_and_end() {
        let mut r = countdown(45, 0); // prep
        r.extend(countdown(180, 60)); // action, planted at 1:00
        let plant = r.len() - 1;
        r.extend(countdown(44, 12)); // defuser timer, round decided at 0:12
        r.push(0.0);
        let t = Timeline::resolve(&r, Some(plant), None, &[]);
        let phases: Vec<_> = t.spans().iter().map(|s| s.phase).collect();
        assert_eq!(
            phases,
            [Phase::Prep, Phase::Action, Phase::Planted, Phase::End]
        );
        let end = t.at(Some(r.len() - 1));
        assert_eq!(end.phase, Phase::End);
        assert_eq!(end.seconds, 12.0, "end shows the last live second");
        // 45 prep seconds, 1 for the switch, 120 action, 1, 32 on the timer.
        assert_eq!(end.elapsed, 45.0 + 1.0 + 120.0 + 1.0 + 32.0);
        assert_eq!(t.at(None).phase, Phase::Prep);
    }

    #[test]
    fn a_round_that_runs_out_of_time_ends_at_zero() {
        let mut r = countdown(45, 0);
        r.extend(countdown(180, 0));
        let t = Timeline::resolve(&r, None, None, &[]);
        let last = t.at(Some(r.len() - 1));
        assert_eq!((last.phase, last.seconds), (Phase::End, 0.0));
        assert_eq!(last.elapsed, 45.0 + 1.0 + 180.0);
        assert!(t.plant_start.is_none());
    }

    /// A plant that completes after the action clock ran out: the clock
    /// stands at 0:00 for as long as the recording says, the plant tick is
    /// the defuser timer's first, and the end has a tick of its own.
    #[test]
    fn a_clock_standing_at_zero_is_measured_by_the_recording() {
        let mut r = countdown(3, 0); // last action seconds
        let mut rec: Vec<Option<f64>> = (0..4).map(|i| Some(f64::from(i))).collect();
        let plant = r.len();
        r.extend([44.0, 43.0, 0.0, 0.0]); // planted 6 s later; timer; time out; decided
        rec.extend([Some(9.0), Some(10.0), Some(53.0), Some(54.1)]);
        let end = r.len() - 1;
        let mut t = Timeline::resolve(&r, Some(plant), Some(end), &rec);
        t.recording = rec;
        assert_eq!(t.plant_start, Some(plant));
        assert_eq!(
            t.ticks[plant].elapsed, 9.0,
            "3 s of countdown and 6 at 0:00"
        );
        assert_eq!(t.end_start, Some(end));
        assert_eq!(t.ticks[end].elapsed, 54.0);
        assert_eq!(t.ticks[end].seconds, 0.0, "time ran out");
        // An event 4.7 s into the stand-still sits 4 s after the 0:00 tick.
        assert_eq!(t.at_time(Some(3), Some(7.7)).elapsed, 7.0);
        assert_eq!(t.at_time(Some(3), None).elapsed, 3.0);
    }
}
