//! Objective finds and operator reveals (Y11S3), against the test rounds
//! and, with `R6_MATCH_REPLAY` set, a real `MatchReplay` folder.

use std::path::{Path, PathBuf};

use replay_analyzer::intel::{Source, Trigger};
use replay_analyzer::{GameMode, MatchUpdateType, Phase, ReadMode, Round, Status};

/// Who found the objective in each of the ten rounds, seconds into the
/// recording, and whether that was in prep. All are spectator recordings,
/// so the score says it.
const FINDS: [(&str, f64, bool); 10] = [
    ("vitaking.FaZe", 156.779, false),
    ("vitaking.FaZe", 97.744, false),
    ("kds.FaZe", 14.38, true),
    ("cyber.FaZe", 166.765, false),
    ("soulz1.FaZe", 11.429, true),
    ("cyber.FaZe", 148.91, false),
    ("pino.L5", 132.162, false),
    ("PSYCHO.L5", 83.54, false),
    ("WIZARD.L5", 13.232, true),
    ("Bassetto.L5", 166.087, false),
];

/// The reveals of each round in order: the player, and the victim of the
/// kill that revealed them.
const REVEALS: [&[(&str, Option<&str>)]; 10] = [
    &[
        ("Bassetto.L5", None),
        ("WIZARD.L5", None),
        ("Neskin.L5", None),
        ("PSYCHO.L5", None),
        ("soulz1.FaZe", Some("Neskin.L5")),
        ("vitaking.FaZe", Some("WIZARD.L5")),
        ("pino.L5", Some("Handyy.FaZe")),
    ],
    &[
        ("PSYCHO.L5", None),
        ("WIZARD.L5", None),
        ("Neskin.L5", None),
        ("pino.L5", None),
        ("soulz1.FaZe", Some("PSYCHO.L5")),
        ("Bassetto.L5", None),
    ],
    &[
        ("Neskin.L5", None),
        ("pino.L5", None),
        ("Bassetto.L5", None),
        ("PSYCHO.L5", None),
        ("cyber.FaZe", Some("Neskin.L5")),
        ("Handyy.FaZe", Some("Bassetto.L5")),
    ],
    &[
        ("pino.L5", None),
        ("WIZARD.L5", None),
        ("cyber.FaZe", Some("PSYCHO.L5")),
        ("Neskin.L5", None),
        ("kds.FaZe", Some("pino.L5")),
        ("Handyy.FaZe", Some("Neskin.L5")),
    ],
    &[
        ("PSYCHO.L5", None),
        ("Bassetto.L5", None),
        ("WIZARD.L5", None),
        ("pino.L5", None),
        ("Handyy.FaZe", Some("pino.L5")),
        ("soulz1.FaZe", Some("Neskin.L5")),
        ("vitaking.FaZe", Some("WIZARD.L5")),
    ],
    &[
        ("PSYCHO.L5", None),
        ("pino.L5", None),
        ("soulz1.FaZe", Some("pino.L5")),
        ("Neskin.L5", None),
        ("Handyy.FaZe", Some("WIZARD.L5")),
    ],
    &[
        ("Handyy.FaZe", None),
        ("vitaking.FaZe", None),
        ("cyber.FaZe", None),
        ("soulz1.FaZe", None),
        ("pino.L5", Some("kds.FaZe")),
        ("Neskin.L5", Some("vitaking.FaZe")),
        ("WIZARD.L5", None),
    ],
    &[
        ("cyber.FaZe", None),
        ("kds.FaZe", None),
        ("PSYCHO.L5", Some("cyber.FaZe")),
        ("WIZARD.L5", Some("Handyy.FaZe")),
        ("soulz1.FaZe", Some("WIZARD.L5")),
    ],
    &[
        ("kds.FaZe", None),
        ("cyber.FaZe", None),
        ("soulz1.FaZe", None),
        ("Handyy.FaZe", None),
        ("vitaking.FaZe", Some("Bassetto.L5")),
        ("PSYCHO.L5", Some("Handyy.FaZe")),
    ],
    &[
        ("kds.FaZe", None),
        ("vitaking.FaZe", None),
        ("Handyy.FaZe", None),
        ("Bassetto.L5", None),
        ("Neskin.L5", Some("cyber.FaZe")),
    ],
];

/// The ten rounds of one custom match, in round order: read once, shared
/// by the tests.
fn test_rounds() -> &'static [(String, Round)] {
    static ROUNDS: std::sync::OnceLock<Vec<(String, Round)>> = std::sync::OnceLock::new();
    ROUNDS.get_or_init(|| {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings/valid/Y11S3");
        (1..=10)
            .map(|n| {
                let name = format!("custom_{n}");
                let round = Round::open(dir.join(format!("{name}.rec")), ReadMode::Full).unwrap();
                (name, round)
            })
            .collect()
    })
}

fn team(round: &Round, name: &str) -> Option<usize> {
    let p = round.header.players.iter().find(|p| p.username == name);
    p.map(|p| p.team_index)
}

/// What every find and reveal has to hold, whatever the round.
fn check(name: &str, round: &Round) {
    if let Some(o) = &round.objective {
        assert_eq!(o.found, o.by.is_some(), "{name}: {o:?}");
        assert_eq!(o.found, o.source.is_some(), "{name}: {o:?}");
        assert_eq!(o.found, o.when.is_some(), "{name}: {o:?}");
        assert_eq!(o.found, o.in_prep.is_some(), "{name}: {o:?}");
        // Only a find by the score after prep is a guess.
        let guessed = o.source == Some(Source::Score) && o.in_prep == Some(false);
        assert_eq!(o.inferred, guessed, "{name}: {o:?}");
        if let Some(by) = &o.by {
            let side = team(round, by).and_then(|t| round.header.teams[t].role);
            assert_eq!(
                side,
                Some(replay_analyzer::TeamRole::Attack),
                "{name}: {o:?}"
            );
        }
        if let (Some(true), Some(when)) = (o.in_prep, &o.when) {
            assert_eq!(when.phase, Phase::Prep, "{name}: {o:?}");
        }
    }
    let mut last = f64::NEG_INFINITY;
    for (i, r) in round.operator_reveals.iter().enumerate() {
        assert!(team(round, &r.username).is_some(), "{name}: {r:?}");
        let earlier = &round.operator_reveals[..i];
        assert!(
            earlier.iter().all(|e| e.username != r.username),
            "{name}: {r:?} twice"
        );
        let at = r.when.recording_time.unwrap_or(0.0);
        assert!(at >= last, "{name}: reveals out of order");
        last = at;
        assert_eq!(
            r.trigger == Trigger::Kill,
            r.victim.is_some(),
            "{name}: {r:?}"
        );
        if let Some(victim) = &r.victim {
            let killed = (round.match_feedback.iter())
                .filter(|u| u.kind == MatchUpdateType::Kill && u.username == r.username)
                .any(|u| &u.target == victim);
            assert!(killed, "{name}: {r:?}");
        }
    }
}

#[test]
fn the_finder_is_the_reference_decoders() {
    for ((name, round), (by, at, in_prep)) in test_rounds().iter().zip(FINDS) {
        check(name, round);
        let o = round
            .objective
            .as_ref()
            .unwrap_or_else(|| panic!("{name}: no objective"));
        assert!(o.found, "{name}");
        assert_eq!(o.by.as_deref(), Some(by), "{name}");
        assert_eq!(o.source, Some(Source::Score), "{name}");
        assert_eq!(o.in_prep, Some(in_prep), "{name}");
        assert_eq!(o.inferred, !in_prep, "{name}");
        let when = o.when.as_ref().unwrap();
        assert_eq!(when.recording_time, Some(at), "{name}");
        let phase = if in_prep { Phase::Prep } else { Phase::Action };
        assert_eq!(when.phase, phase, "{name}");
    }
}

#[test]
fn the_reveals_are_the_reference_decoders() {
    let (mut kills, mut bonus) = (0, 0);
    for ((name, round), wanted) in test_rounds().iter().zip(REVEALS) {
        let found: Vec<(&str, Option<&str>)> = (round.operator_reveals.iter())
            .map(|r| (r.username.as_str(), r.victim.as_deref()))
            .collect();
        assert_eq!(found, wanted, "{name}");
        for r in &round.operator_reveals {
            kills += usize::from(r.trigger == Trigger::Kill);
            bonus += usize::from(r.team_bonus);
        }
        let status = round.decode.get("intel").unwrap();
        assert_eq!(status.status, Status::Decoded, "{name}");
        assert_eq!(status.count, 1 + wanted.len(), "{name}");
        assert!(status.warnings.is_empty(), "{name}: {:?}", status.warnings);
    }
    assert_eq!((kills, bonus), (22, 43));
}

#[test]
fn the_json_has_the_keys_of_a_find_and_a_reveal() {
    let (_, round) = &test_rounds()[2];
    let json = serde_json::to_value(round).unwrap();
    let o = &json["objective"];
    assert_eq!(o["found"], true);
    assert_eq!(o["by"], "kds.FaZe");
    assert_eq!(o["source"], "score");
    assert_eq!(o["inPrep"], true);
    assert!(o.get("inferred").is_none(), "{o}");
    assert_eq!(o["recordingTime"], 14.38);
    assert_eq!(o["phase"], "Prep");
    assert!(o["time"].is_string() && o["elapsed"].is_number(), "{o}");
    let reveals = json["operatorReveals"].as_array().unwrap();
    let kill = &reveals[4];
    assert_eq!(kill["username"], "cyber.FaZe");
    assert_eq!(kill["trigger"], "kill");
    assert_eq!(kill["victim"], "Neskin.L5");
    assert_eq!(kill["teamBonus"], true);
    assert_eq!(reveals[0]["trigger"], "identified");
    assert_eq!(reveals[0]["teamBonus"], true);
    assert!(reveals[0].get("victim").is_none());
    for key in ["time", "phase", "elapsed", "recordingTime"] {
        assert!(kill.get(key).is_some(), "{key}");
    }
    // An inferred find says so.
    let (_, round) = &test_rounds()[0];
    let json = serde_json::to_value(round).unwrap();
    assert_eq!(json["objective"]["inferred"], true);
    assert_eq!(json["objective"]["inPrep"], false);
}

/// A partial read does not reach the HUD: the keys are left out.
#[test]
fn a_partial_read_has_no_intel() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings/valid/Y11S3");
    let round = Round::open(dir.join("custom_1.rec"), ReadMode::Partial).unwrap();
    assert!(round.objective.is_none() && round.operator_reveals.is_empty());
    let json = serde_json::to_value(&round).unwrap();
    for key in ["objective", "operatorReveals", "metalDetectors"] {
        assert!(json.get(key).is_none(), "{key}");
    }
}

fn match_replay_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("R6_MATCH_REPLAY")?);
    dir.is_dir().then_some(dir)
}

/// Real rounds: a find in the feed is the score's find when the score has
/// one in prep, and every bomb round that reached its action phase says
/// whether the objective was found.
#[test]
fn real_finds_agree_with_the_score_and_every_bomb_round_has_one() {
    let Some(root) = match_replay_dir() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let (mut rounds, mut feed, mut confirmed, mut both, mut prep) = (0, 0, 0, 0, 0);
    let (mut inferred, mut not_found, mut absent) = (0, 0, 0);
    let (mut reveals, mut kills, mut bonus, mut late) = (0, 0, 0, 0);
    for dir in replay_analyzer::matches::find_match_folders(&root).unwrap() {
        let folder = replay_analyzer::Match::open_with(&dir, ReadMode::Full).unwrap();
        for round in &folder.rounds {
            // Names the round without naming its players.
            let name = format!(
                "{} R{}",
                round.header.match_id,
                round.header.round_number + 1
            );
            if round.decode.get("intel").is_none() {
                continue;
            }
            rounds += 1;
            check(&name, round);
            for r in &round.operator_reveals {
                reveals += 1;
                kills += usize::from(r.trigger == Trigger::Kill);
                bonus += usize::from(r.team_bonus);
                // A kill by the player up to 1.5 s before a reveal that is
                // not put down to it.
                let at = r.when.recording_time.unwrap_or(0.0);
                late += usize::from(
                    r.trigger == Trigger::Identified
                        && round.match_feedback.iter().any(|u| {
                            u.kind == MatchUpdateType::Kill
                                && u.username == r.username
                                && u.recording_time
                                    .is_some_and(|k| (0.0..1.5).contains(&(at - k)))
                        }),
                );
            }
            match &round.objective {
                Some(o) if o.source == Some(Source::Feed) => {
                    feed += 1;
                    confirmed += usize::from(o.score_confirmed);
                    if let Some(score) = &o.prep_score {
                        both += 1;
                        assert_eq!(o.by.as_ref(), Some(score), "{name}");
                    }
                }
                Some(o) if o.found => {
                    prep += usize::from(!o.inferred);
                    inferred += usize::from(o.inferred);
                }
                Some(_) => not_found += 1,
                None => {
                    absent += 1;
                    // Quick Match plays a bomb mode of its own, which shows
                    // neither sign; so does a round that ends in prep.
                    let phases = round.timeline.spans();
                    let played = phases.iter().any(|p| p.phase == Phase::Action);
                    let bomb = round.header.game_mode == GameMode::BOMB;
                    assert!(!(bomb && played), "{name}: a bomb round without");
                }
            }
        }
    }
    eprintln!(
        "{rounds} rounds: {feed} finds in the feed ({confirmed} with the +50, {both} with a \
         +50 in prep), {prep} by the score in prep, {inferred} inferred, {not_found} not \
         found, {absent} without"
    );
    eprintln!(
        "{reveals} reveals: {kills} by a kill, {bonus} with the team's bonus, {late} \
         identified within 1.5 s of a kill"
    );
    assert!(rounds > 0, "no Y11S3 rounds under {}", root.display());
}
