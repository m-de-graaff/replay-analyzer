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
}

impl Timeline {
    /// Resolves `readings` (distinct consecutive clock values in stream
    /// order). `plant` is the tick at which the plant completed, if it did.
    pub fn resolve(readings: &[f64], plant: Option<usize>) -> Self {
        let n = readings.len();
        let jumped_up = |i: usize| i > 0 && readings[i] > readings[i - 1] + 1.0;
        let action_start = (0..n).find(|&i| readings[i] > ACTION_START && (i == 0 || jumped_up(i)));
        // The round ends on its final 0:00, once action has started.
        let end_start = match (action_start, readings.last()) {
            (Some(a), Some(&last)) if last == 0.0 && n - 1 > a => Some(n - 1),
            _ => None,
        };
        let live_end = end_start.unwrap_or(n);
        // The defuser timer takes over at the first break in the countdown
        // after the plant (down from a late plant, up from one at 0:00); if
        // the round ended first, at the plant tick itself.
        let plant_start = plant.filter(|&p| p < live_end).map(|p| {
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
                // defuser timer) takes over one second after the last one.
                elapsed += if step > 0.0 && Some(i) != plant_start {
                    step
                } else {
                    1.0
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

    /// Seconds from prep start to the last reading.
    pub fn duration(&self) -> f64 {
        self.ticks.last().map_or(0.0, |t| t.elapsed)
    }

    /// The phases in order, with when each started and ended.
    pub fn spans(&self) -> Vec<PhaseSpan> {
        let mut spans: Vec<PhaseSpan> = Vec::new();
        for t in &self.ticks {
            match spans.last_mut() {
                Some(s) if s.phase == t.phase => {
                    s.end = t.elapsed;
                    s.end_time = display_clock(t.seconds);
                }
                _ => {
                    if let Some(s) = spans.last_mut() {
                        s.end = t.elapsed;
                    }
                    spans.push(PhaseSpan {
                        phase: t.phase,
                        start_time: display_clock(t.seconds),
                        end_time: display_clock(t.seconds),
                        start: t.elapsed,
                        end: t.elapsed,
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
        let t = Timeline::resolve(&r, Some(plant));
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
        let t = Timeline::resolve(&r, None);
        let last = t.at(Some(r.len() - 1));
        assert_eq!((last.phase, last.seconds), (Phase::End, 0.0));
        assert_eq!(last.elapsed, 45.0 + 1.0 + 180.0);
        assert!(t.plant_start.is_none());
    }
}
