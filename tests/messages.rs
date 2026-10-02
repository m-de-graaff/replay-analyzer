//! The feed's lines that are no kills and the BattlEye flag (Y11S3),
//! against the test rounds and, with `R6_MATCH_REPLAY` set, a real
//! `MatchReplay` folder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use replay_analyzer::messages::{Kind, KindSource, SystemMessage};
use replay_analyzer::{ReadMode, Round, Status};

/// The ids of the two lines every round shows.
const ROUND_START: &str = "c3c5050000000065";
const ACTION_START: &str = "c4c5050000000065";
/// A line and the flag it reports are written within this (seconds).
const SAME_MOMENT: f64 = 1.5;
/// How long a line stays up (seconds): the same line within that is the
/// feed scrolling.
const SHOWN: f64 = 5.0;

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

fn at(line: &SystemMessage) -> f64 {
    line.when.recording_time.unwrap_or(0.0)
}

/// What a line shows, apart from when: its kind, id, text and arguments.
type Content<'a> = (
    Kind,
    Option<&'a str>,
    Option<&'a str>,
    Vec<(&'a str, &'a str)>,
);

fn content(line: &SystemMessage) -> Content<'_> {
    let args = line.args.iter().map(|a| (a.key.as_str(), a.value.as_str()));
    (
        line.kind,
        line.message_id.as_deref(),
        line.text.as_deref(),
        args.collect(),
    )
}

/// What every line and flag has to hold, whatever the round. The lines
/// named in the messages name the round, never its players.
fn check(name: &str, round: &Round) {
    let lines = &round.system_messages;
    let flag = round
        .battl_eye
        .as_ref()
        .unwrap_or_else(|| panic!("{name}: no battlEye"));
    let mut last = f64::NEG_INFINITY;
    for (i, l) in lines.iter().enumerate() {
        let what = format!("{name}: line {i} ({:?})", l.kind);
        assert!(at(l) >= last, "{what}: out of order");
        last = at(l);
        assert!(l.frame.is_some() && l.offset > 0, "{what}");
        // A line is a text or an id, never both and never neither.
        assert_ne!(l.message_id.is_some(), l.text.is_some(), "{what}");
        if let Some(id) = &l.message_id {
            assert_eq!(id.len(), 16, "{what}");
            assert_ne!(id, &"f".repeat(16), "{what}");
        }
        match l.kind {
            Kind::Unknown => assert_eq!(l.kind_source, None, "{what}"),
            Kind::ObjectiveFound => {
                assert_eq!(l.kind_source, Some(KindSource::Decoded), "{what}");
                // The text names the finder, and the round's objective is
                // that find.
                let by = round.objective.as_ref().and_then(|o| o.by.as_deref());
                let text = l.text.as_deref().unwrap_or_default();
                assert!(by.is_some_and(|by| text.contains(by)), "{what}");
                assert_eq!(l.username.as_deref(), by, "{what}");
            }
            Kind::Phase => {
                assert_eq!(l.kind_source, Some(KindSource::Inferred), "{what}");
                assert!(l.args.is_empty() && l.username.is_none(), "{what}");
            }
            _ => {
                assert_eq!(l.kind_source, Some(KindSource::Inferred), "{what}");
                // Every line of a player names one.
                assert_eq!(l.args.len(), 1, "{what}");
                assert!(l.username.as_ref().is_some_and(|n| !n.is_empty()), "{what}");
            }
        }
        // A profile id is the header's for that name.
        if let Some(id) = &l.profile_id {
            let player =
                (round.header.players.iter()).find(|p| Some(&p.username) == l.username.as_ref());
            assert_eq!(player.map(|p| &p.profile_id), Some(id), "{what}");
        }
        // Reverse friendly fire is the game's flag too: `friendlyFire[]`
        // has the same player turning on or off in that moment. A flag
        // carried over from an earlier round is said again as the round
        // starts, with no turn to go with it.
        let flags = round.vitals.as_ref().map_or(&[][..], |v| &v.friendly_fire);
        let own = || {
            flags
                .iter()
                .filter(|f| Some(&f.username) == l.username.as_ref())
        };
        let turned = |on: bool| {
            own().any(|f| {
                let when = if on { &f.on } else { &f.off };
                let time = when.as_ref().and_then(|w| w.recording_time);
                time.is_some_and(|t| (t - at(l)).abs() <= SAME_MOMENT)
            })
        };
        match l.kind {
            Kind::ReverseFriendlyFireOn => {
                let carried = own().any(|f| f.active_at_start);
                assert!(turned(true) || carried, "{what}: no flag on");
            }
            Kind::ReverseFriendlyFireOff => assert!(turned(false), "{what}: no flag off"),
            _ => {}
        }
        // A line the feed scrolls is written again, and is one line.
        let again = lines
            .iter()
            .take(i)
            .any(|e| content(e) == content(l) && at(l) - at(e) < SHOWN && l.kind != Kind::Unknown);
        assert!(!again, "{what}: twice");
    }

    // The flag repeats what the lines say.
    let said = |l: &SystemMessage| {
        let text = l.text.as_deref().unwrap_or_default();
        text.to_lowercase().contains("battleye")
    };
    let hits: Vec<usize> = (0..lines.len()).filter(|&i| said(&lines[i])).collect();
    assert_eq!(flag.flagged, !hits.is_empty(), "{name}");
    assert_eq!(flag.messages, hits, "{name}");
    assert_eq!(flag.texts.len(), hits.len(), "{name}");
    let unknown = lines.iter().filter(|l| l.kind == Kind::Unknown).count();
    assert_eq!(flag.unknown_messages, unknown, "{name}");

    // `decodeStatus` counts the lines, and names unknown ids in a warning.
    let status = round
        .decode
        .get("feedbackMessages")
        .unwrap_or_else(|| panic!("{name}: no status"));
    assert_eq!(status.count, lines.len(), "{name}");
    let ids = (lines.iter()).any(|l| l.kind == Kind::Unknown && l.message_id.is_some());
    if ids {
        assert!(!status.warnings.is_empty(), "{name}");
    } else if status.warnings.is_empty() {
        assert_eq!(status.status, Status::Decoded, "{name}");
    }
    assert_ne!(status.status, Status::Missing, "{name}");
}

#[test]
fn the_test_rounds_show_the_two_announcements_and_no_player() {
    for (name, round) in test_rounds() {
        check(name, round);
        let ids: Vec<_> = (round.system_messages.iter())
            .map(|l| (l.kind, l.message_id.as_deref(), l.background_color))
            .collect();
        assert_eq!(
            ids,
            [
                (Kind::Phase, Some(ROUND_START), 0),
                (Kind::Phase, Some(ACTION_START), 0),
            ],
            "{name}"
        );
        // One as the recording starts, one as the action phase does.
        let [start, action] = round.system_messages.as_slice() else {
            unreachable!();
        };
        assert!(at(start) < 1.0, "{name}: {start:?}");
        let spans = round.timeline.spans();
        let begun = (spans.iter())
            .find(|s| s.phase == replay_analyzer::Phase::Action)
            .and_then(|s| s.recording_start);
        assert!(
            begun.is_some_and(|b| (at(action) - b).abs() < 1.0),
            "{name}: {action:?} against {begun:?}"
        );
        let flag = round.battl_eye.as_ref().unwrap();
        assert!(!flag.flagged && flag.unknown_messages == 0, "{name}");
        let status = round.decode.get("feedbackMessages").unwrap();
        assert_eq!(
            (status.status, status.count),
            (Status::Decoded, 2),
            "{name}"
        );
        assert!(status.warnings.is_empty(), "{name}: {:?}", status.warnings);
    }
}

#[test]
fn the_json_has_the_keys_of_a_line_and_of_the_flag() {
    let (_, round) = &test_rounds()[0];
    let json = serde_json::to_value(round).unwrap();
    let lines = json["systemMessages"].as_array().unwrap();
    let line = &lines[1];
    assert_eq!(line["kind"], "phase");
    assert_eq!(line["kindSource"], "inferred");
    assert_eq!(line["messageId"], ACTION_START);
    assert_eq!(line["backgroundColor"], 0);
    for key in ["time", "phase", "elapsed", "recordingTime"] {
        assert!(line.get(key).is_some(), "{key}");
    }
    // A line that names nobody and has no text leaves those keys out.
    for key in ["text", "args", "username", "profileID"] {
        assert!(line.get(key).is_none(), "{key}");
    }
    assert_eq!(
        json["battlEye"],
        serde_json::json!({ "flagged": false, "unknownMessages": 0 })
    );
    // The flag marks the round: nothing here judges a player.
    let said = format!("{} {}", json["systemMessages"], json["battlEye"]).to_lowercase();
    assert!(!said.contains("cheat"), "{said}");
}

/// Kills stay in `matchFeedback`, and no line doubles as one there.
#[test]
fn kills_are_no_system_messages() {
    for (name, round) in test_rounds() {
        assert!(!round.match_feedback.is_empty(), "{name}");
        for l in &round.system_messages {
            assert!(l.username.is_none(), "{name}: {l:?}");
        }
    }
}

/// A partial read does not reach the HUD: the keys are left out.
#[test]
fn a_partial_read_has_no_system_messages() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings/valid/Y11S3");
    let round = Round::open(dir.join("custom_1.rec"), ReadMode::Partial).unwrap();
    assert!(round.system_messages.is_empty() && round.battl_eye.is_none());
    assert!(round.decode.get("feedbackMessages").is_none());
    let json = serde_json::to_value(&round).unwrap();
    for key in ["systemMessages", "battlEye"] {
        assert!(json.get(key).is_none(), "{key}");
    }
}

/// The match's rollup lists the rounds flagged: none of the test match.
#[test]
fn a_match_lists_its_flagged_rounds() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings/valid/Y11S3");
    let folder = replay_analyzer::Match::open_with(&dir, ReadMode::Full).unwrap();
    let rollup = folder.battl_eye().unwrap();
    assert!(rollup.flagged_rounds.is_empty());
    let json = serde_json::to_value(&folder).unwrap();
    assert_eq!(json["battlEye"], serde_json::json!({ "flaggedRounds": [] }));
}

fn match_replay_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("R6_MATCH_REPLAY")?);
    dir.is_dir().then_some(dir)
}

/// Real rounds: every line of a player names one, reverse friendly fire
/// lines come with the game's flag, a find's text names the round's
/// finder, no line is there twice, and the flag says what the lines do.
#[test]
fn real_lines_hold_what_any_round_does() {
    let Some(root) = match_replay_dir() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let (mut rounds, mut flagged) = (0, 0);
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for dir in replay_analyzer::matches::find_match_folders(&root).unwrap() {
        let folder = replay_analyzer::Match::open_with(&dir, ReadMode::Full).unwrap();
        let mut listed = Vec::new();
        for round in &folder.rounds {
            // Names the round without naming its players.
            let number = round.header.round_number + 1;
            let name = format!("{} R{number}", round.header.match_id);
            if round.decode.get("feedbackMessages").is_none() || round.battl_eye.is_none() {
                continue;
            }
            rounds += 1;
            check(&name, round);
            for l in &round.system_messages {
                let kind = serde_json::to_value(l.kind).unwrap();
                *kinds.entry(kind.as_str().unwrap().to_owned()).or_default() += 1;
            }
            if round.battl_eye.as_ref().is_some_and(|b| b.flagged) {
                listed.push(number);
            }
        }
        flagged += listed.len();
        let rollup = folder.battl_eye().map(|b| b.flagged_rounds);
        assert_eq!(rollup.unwrap_or_default(), listed, "{}", dir.display());
    }
    eprintln!("{rounds} rounds, {flagged} flagged for BattlEye: {kinds:?}");
    assert!(rounds > 0, "no Y11S3 rounds under {}", root.display());
}
