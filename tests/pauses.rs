//! Pauses and the breaks between rounds (Y11S3), against the test rounds
//! and, with `R6_MATCH_REPLAY` set, a real `MatchReplay` folder. None of
//! these rounds was paused: the tests hold the detector to finding nothing
//! in them, and the measurements to the margins its thresholds rest on.

use std::path::{Path, PathBuf};

use replay_analyzer::pauses::{self, Kind};
use replay_analyzer::{Match, ReadMode, Round, Status};

fn test_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings/valid/Y11S3")
}

/// The ten rounds of the test match, in play order.
fn test_rounds() -> Vec<(String, Round)> {
    (1..=10)
        .map(|n| {
            let name = format!("custom_{n}");
            let path = test_dir().join(format!("{name}.rec"));
            (name, Round::open(path, ReadMode::Full).unwrap())
        })
        .collect()
}

/// The most the recording ran ahead of the clock over one step and within
/// one timer, per round, as the reference decoder measured it.
const LAGS: [(f64, f64); 10] = [
    (0.015, 0.018),
    (0.014, 0.018),
    (0.024, 0.042),
    (0.042, 0.128),
    (0.033, 0.078),
    (0.033, 0.062),
    (0.038, 0.097),
    (0.022, 0.056),
    (0.016, 0.018),
    (0.024, 0.074),
];

/// The breaks of the test match in seconds, by the round they follow.
const BREAKS: [f64; 9] = [31.6, 28.2, 95.4, 31.7, 27.9, 180.9, 30.5, 73.1, 59.0];

#[test]
fn no_test_round_has_a_pause() {
    for ((name, round), (step, sum)) in test_rounds().iter().zip(LAGS) {
        let report = round.pauses.as_ref().unwrap();
        assert_eq!(report.pauses, [], "{name}");
        assert!(report.unknown_states.is_empty(), "{name}");
        assert!(report.clock_writes > 1000, "{name}");
        assert!(report.decided_at.is_some(), "{name}");
        // Times are kept to the millisecond.
        assert!((report.max_step_lag - step).abs() < 0.003, "{name}");
        assert!((report.max_cumulative_lag - sum).abs() < 0.003, "{name}");
        assert!(report.max_step_lag < 0.2, "{name}");
        assert!(report.max_cumulative_lag < 0.6, "{name}");
        assert_eq!(report.clock_ahead, 0.0, "{name}");

        let status = round.decode.get("pauses").unwrap();
        assert_eq!((status.status, status.count), (Status::Inferred, 0));
        assert!(status.warnings.is_empty(), "{name}: {:?}", status.warnings);
        assert!(round.decode.trusted, "{name}");
        let json = serde_json::to_value(round).unwrap();
        assert!(json.get("pauses").is_none(), "{name}");
    }
}

#[test]
fn the_header_and_the_index_agree_on_the_length() {
    for (name, round) in test_rounds() {
        let timing = round.timing.as_ref().unwrap();
        let over = timing.header_minus_index.unwrap();
        assert!((0.0..=0.005).contains(&over), "{name}: {over}");
        assert_eq!(round.pauses.unwrap().header_minus_index, Some(over));
        let json = serde_json::to_value(timing).unwrap();
        assert_eq!(json["headerMinusIndex"], over, "{name}");
    }
    // The header alone says it too.
    let round = Round::open(test_dir().join("custom_1.rec"), ReadMode::Header).unwrap();
    assert_eq!(round.timing.unwrap().header_minus_index, Some(0.002));
    assert!(round.pauses.is_none() && round.decode.get("pauses").is_none());
}

/// In custom_7 the action clock runs out while a plant is under way: it
/// stands at zero from 224.9 s until the plant completes at 230.9 s and
/// the defuser timer takes over. And from 264.8 s to 271.0 s nobody
/// moves, while the clock runs on. Neither is a pause.
#[test]
fn a_clock_at_zero_and_players_standing_still_are_not_pauses() {
    let (_, round) = &test_rounds()[6];
    let clock = &round.activity.as_ref().unwrap().clock;
    // The last write of the action timer and the first of the defuser's.
    let stood = (clock.writes.windows(2))
        .find(|w| w[1].ms > w[0].ms + 1000 && w[0].time > 100.0)
        .unwrap();
    assert!((stood[0].time - 224.9).abs() < 0.1, "{stood:?}");
    assert!((stood[1].time - 230.9).abs() < 0.1, "{stood:?}");
    assert!(stood[0].ms < 100 && stood[1].ms > 44_000, "{stood:?}");
    assert_eq!(clock.plants.len(), 1);
    assert!((clock.plants[0] - 230.9).abs() < 0.1, "{:?}", clock.plants);

    // The clock drops by as much as the recording moves on while
    // everyone stands.
    let still: Vec<_> = (clock.writes.iter())
        .filter(|w| (264.8..271.0).contains(&w.time))
        .collect();
    let (first, last) = (still[0], still[still.len() - 1]);
    let dropped = f64::from(first.ms - last.ms) / 1000.0;
    assert!(last.time - first.time > 6.0);
    assert!((last.time - first.time - dropped).abs() < 0.05);
    assert!(still.len() > 15, "written throughout: {}", still.len());

    assert_eq!(round.pauses.as_ref().unwrap().pauses, []);
    assert!(round.timing.as_ref().unwrap().skips.is_empty());
}

/// A plant moves the clock from the action timer to the defuser's, down
/// or up: no stall, and not the clock jumping ahead.
#[test]
fn a_plant_is_neither_a_stall_nor_a_skip() {
    let mut planted = 0;
    for (name, round) in test_rounds() {
        let clock = &round.activity.as_ref().unwrap().clock;
        let Some(&plant) = clock.plants.first() else {
            continue;
        };
        planted += 1;
        // The timer written with the plant is the defuser's, just started.
        let after = clock.writes.iter().find(|w| w.time >= plant).unwrap();
        assert!((44_000..45_000).contains(&after.ms), "{name}: {after:?}");
        let report = round.pauses.unwrap();
        assert_eq!(report.pauses, [], "{name}");
        assert_eq!(report.clock_ahead, 0.0, "{name}");
    }
    assert!(planted >= 2, "{planted} rounds with a plant");
}

#[test]
fn a_partial_read_does_not_look_for_pauses() {
    let round = Round::open(test_dir().join("custom_1.rec"), ReadMode::Partial).unwrap();
    assert!(round.pauses.is_none());
    assert!(round.decode.get("pauses").is_none());
}

/// The breaks of the test match. Operators were banned after rounds 3, 6
/// and 9 (6 is the side switch too); the long break after round 8 has
/// nothing to explain it.
#[test]
fn the_breaks_of_the_test_match() {
    let folder = Match::open(test_dir()).unwrap();
    let breaks = folder.breaks();
    assert_eq!(breaks.len(), 9);
    for ((b, wanted), after) in breaks.iter().zip(BREAKS).zip(1..) {
        assert_eq!(b.after_round, after);
        assert!((b.duration - wanted).abs() < 0.1, "after {after}: {b:?}");
        assert_eq!(b.ban_phase, Some([3, 6, 9].contains(&after)), "{b:?}");
        assert_eq!(b.side_switch, Some(after == 6), "{b:?}");
        assert!(!b.overtime, "{b:?}");
        // The median of the six plain breaks.
        assert_eq!(b.expected, Some(31.046), "{b:?}");
        assert_eq!(b.pause_suspected, Some(after == 8), "{b:?}");
    }
    assert!(breaks[7].excess.unwrap() > pauses::LONG_BREAK);

    let json = serde_json::to_value(&folder).unwrap();
    let after_8 = &json["breaks"][7];
    assert_eq!(after_8["afterRound"], 8);
    assert_eq!(after_8["duration"], 73.054);
    assert_eq!(after_8["excess"], 42.008);
    assert_eq!(after_8["banPhase"], false);
    assert_eq!(after_8["sideSwitch"], false);
    assert_eq!(after_8["overtime"], false);
    assert_eq!(after_8["pauseSuspected"], true);
}

/// A round missing between two files leaves no break to measure, and a
/// header-only read has the durations and the sides but not the bans.
#[test]
fn breaks_need_consecutive_rounds() {
    let open = |n: u32, mode| Round::open(test_dir().join(format!("custom_{n}.rec")), mode);
    let rounds: Vec<Round> = [1, 2, 4, 5]
        .iter()
        .map(|&n| open(n, ReadMode::Header).unwrap())
        .collect();
    let breaks = pauses::breaks(&rounds);
    let after: Vec<u32> = breaks.iter().map(|b| b.after_round).collect();
    assert_eq!(after, [1, 4]);
    for b in &breaks {
        assert_eq!((b.ban_phase, b.side_switch), (None, Some(false)), "{b:?}");
        assert_eq!((b.expected, b.pause_suspected), (None, None), "{b:?}");
    }
    assert!((breaks[1].duration - BREAKS[3]).abs() < 0.1);
}

fn match_replay_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("R6_MATCH_REPLAY")?);
    dir.is_dir().then_some(dir)
}

/// Real rounds, none of them paused: the clock never stalls, the streams
/// have no hole before the decision, `TimerState` stays within its three
/// values, and what the header runs longer than the index is the game
/// time the recording skipped.
#[test]
fn real_rounds_have_no_pause() {
    let Some(root) = match_replay_dir() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let (mut rounds, mut matches, mut breaks, mut rated, mut suspected) = (0, 0, 0, 0, 0);
    let (mut step, mut sum, mut over, mut ahead) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let (mut unwritten, mut steady, mut longer) = (0, 0.0f64, 0.0f64);
    let mut findings: Vec<String> = Vec::new();
    for dir in replay_analyzer::matches::find_match_folders(&root).unwrap() {
        let folder = Match::open_with(&dir, ReadMode::Full).unwrap();
        for round in &folder.rounds {
            let Some(report) = &round.pauses else {
                continue;
            };
            // Names the round without naming its players.
            let file = round.file.as_ref().map_or("?", |f| f.file_name.as_str());
            let name = format!("{} {file}", round.header.match_id);
            let status = round.decode.get("pauses").unwrap();
            // A file the game did not finish has no clock track.
            if status.status == Status::Missing {
                assert_eq!(report.clock_writes, 0, "{name}");
                unwritten += 1;
                continue;
            }
            rounds += 1;
            assert_eq!(status.status, Status::Inferred, "{name}");
            assert!(report.unknown_states.is_empty(), "{name}: {status:?}");
            for p in &report.pauses {
                findings.push(format!("{name}: {p:?}"));
            }
            step = step.max(report.max_step_lag);
            sum = sum.max(report.max_cumulative_lag);
            ahead = ahead.max(report.clock_ahead);
            if let Some(header) = report.header_minus_index {
                let unexplained = header - report.clock_ahead;
                assert!(
                    unexplained.abs() < pauses::SUSPENDED,
                    "{name}: {unexplained}"
                );
                over = over.max(unexplained.abs());
                longer = longer.max(header);
                // A recording that skipped nothing is as long as its index.
                let skips = round.timing.as_ref().map_or(0, |t| t.skips.len());
                if skips == 0 && report.clock_ahead == 0.0 {
                    steady = steady.max(header.abs());
                }
            }
            assert!(report.max_step_lag < 0.2, "{name}");
            assert!(report.max_cumulative_lag < 0.6, "{name}");
        }
        let between = folder.breaks();
        matches += usize::from(!between.is_empty());
        breaks += between.len();
        rated += between.iter().filter(|b| b.expected.is_some()).count();
        for b in between.iter().filter(|b| b.pause_suspected == Some(true)) {
            suspected += 1;
            let id = &folder.rounds[0].header.match_id;
            eprintln!("{id}: a long break after round {}: {b:?}", b.after_round);
        }
    }
    eprintln!(
        "{rounds} rounds: the recording at most {step:.3} s ahead of the clock over a step and \
         {sum:.3} s within a timer, the clock at most {ahead:.3} s ahead in a round, the \
         header at most {over:.3} s from the index and the clock ({longer:.3} s from the \
         index alone, {steady:.3} s in a round without skips); {unwritten} more without a \
         clock track"
    );
    eprintln!(
        "{breaks} breaks in {matches} matches, {rated} with a level to compare with, \
         {suspected} long without bans or a side switch"
    );
    assert!(rounds > 0, "no Y11S3 rounds under {}", root.display());
    let stalls = [
        Kind::ClockStall,
        Kind::ClockSlow,
        Kind::DataHole,
        Kind::IndexGap,
    ];
    let wrong: Vec<&String> = (findings.iter())
        .filter(|f| stalls.iter().any(|k| f.contains(&format!("{k:?}"))))
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
    for f in &findings {
        eprintln!("{f}");
    }
}
