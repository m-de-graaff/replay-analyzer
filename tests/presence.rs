//! Leavers, reconnects and seats (Y11S3), against the test rounds and,
//! with `R6_MATCH_REPLAY` set, a real `MatchReplay` folder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use replay_analyzer::messages::{Kind, SystemMessage};
use replay_analyzer::presence::{
    EventKind, LINE_WINDOW, Presence, ReconnectKind, Returned, SeatState, Source,
};
use replay_analyzer::{Match, Phase, ReadMode, Round, Status};

/// A time to the millisecond and one to the frame can differ by this.
const ROUNDING: f64 = 0.002;

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
    line.when.recording_time.unwrap_or(f64::NAN)
}

/// The lines of `kinds` that name `username` and were written `window`
/// seconds from `time` (earliest, latest).
fn lines<'a>(
    round: &'a Round,
    kinds: &[Kind],
    username: &str,
    time: Option<f64>,
    window: (f64, f64),
) -> Vec<&'a SystemMessage> {
    let near = |l: &&SystemMessage| {
        let apart = time.map(|t| at(l) - t);
        apart.is_some_and(|d| d >= window.0 - ROUNDING && d <= window.1 + ROUNDING)
    };
    (round.system_messages.iter())
        .filter(|l| kinds.contains(&l.kind) && l.username.as_deref() == Some(username))
        .filter(near)
        .collect()
}

/// What the seats, leavers and reconnects of any round have to hold. The
/// messages name the round, never its players.
fn check(name: &str, round: &Round, presence: &Presence) {
    let players = &round.header.players;
    let listed = |username: &str| players.iter().any(|p| p.username == username);
    let status = round
        .decode
        .get("presence")
        .unwrap_or_else(|| panic!("{name}: no status"));
    let events = presence.leavers.len() + presence.reconnects.len();
    assert_eq!(status.count, events, "{name}");
    assert_ne!(status.status, Status::Missing, "{name}");
    if status.warnings.is_empty() {
        assert_eq!(status.status, Status::Decoded, "{name}");
    }
    // `HasLeft` never goes back to 0 within a round.
    let cleared = status.warnings.iter().any(|w| w.contains("HasLeft"));
    assert!(!cleared, "{name}: {:?}", status.warnings);

    for (i, s) in presence.seats.iter().enumerate() {
        let what = format!("{name}: seat {i} ({:?})", s.seat);
        // Only a seat that is not plain is listed.
        assert!(s.slot_type != 1 || s.has_left, "{what}");
        match s.seat {
            // A seat filled again is a player's of the header.
            SeatState::Refilled => {
                assert_eq!(s.slot_type, 1, "{what}");
                let profile = s.profile_id.as_ref();
                let plays = |p: &replay_analyzer::Player| Some(&p.profile_id) == profile;
                assert!(players.iter().any(plays), "{what}: not in the header");
            }
            // A seat nobody plays in was left, and the header has no
            // player of its name.
            SeatState::Reserved | SeatState::Opened | SeatState::Joining => {
                assert!(s.has_left, "{what}");
                assert!(!listed(&s.username), "{what}: in the header");
            }
            // A seat nobody sat in has no name and no ids.
            SeatState::Empty => {
                assert!(!s.has_left && s.slot_type == 2, "{what}");
                assert!(s.username.is_empty() && s.profile_id.is_none(), "{what}");
            }
            SeatState::Unknown => {}
        }
        // A reserved seat keeps the profile id, an opened one has none.
        match s.seat {
            SeatState::Reserved => assert!(s.profile_id.is_some(), "{what}"),
            SeatState::Opened => assert!(s.profile_id.is_none(), "{what}"),
            _ => {}
        }
        assert!(s.team.is_some_and(|t| t < 2), "{what}");
    }

    let mut last = None;
    for (i, l) in presence.leavers.iter().enumerate() {
        let what = format!("{name}: leaver {i} ({:?})", l.source);
        assert!(l.frame >= last, "{what}: out of order");
        last = l.frame;
        let time = l.when.recording_time;
        assert!(time.is_some() && !l.username.is_empty(), "{what}");
        match l.source {
            Source::Slot => {
                // The seat became reserved or opened, and the ids are the
                // ones it held before.
                let seat = (l.seat, l.slot_type);
                let known = [
                    (Some(SeatState::Reserved), Some(4)),
                    (Some(SeatState::Opened), Some(2)),
                ];
                assert!(known.contains(&seat), "{what}: {seat:?}");
                assert!(l.profile_id.is_some() && l.playerid.is_some(), "{what}");
                // Exactly one `playerLeft` line of the player follows.
                let said = lines(
                    round,
                    &[Kind::PlayerLeft],
                    &l.username,
                    time,
                    (0.0, LINE_WINDOW),
                );
                assert_eq!(said.len(), 1, "{what}: playerLeft lines");
            }
            Source::FeedOnly => {
                assert!(l.seat.is_none() && l.slot_type.is_none(), "{what}");
                let said = lines(round, &[Kind::PlayerLeft], &l.username, time, (0.0, 0.0));
                assert_eq!(said.len(), 1, "{what}: playerLeft lines");
            }
        }
        // Dead, alive or never there: one of the three.
        assert!(!(l.alive_at_leave && l.never_spawned), "{what}");
        assert!(
            !(l.alive_at_leave && l.died_seconds_before.is_some()),
            "{what}"
        );
        assert!(l.died_seconds_before.is_none_or(|d| d >= 0.0), "{what}");
        assert!(l.silent_seconds.is_none_or(|s| s >= 0.0), "{what}");
        assert!(l.silent_seconds.is_none() || l.alive_at_leave, "{what}");
        // Someone who left alive, before the round was decided, counts as
        // gone when the round's end is worked out.
        let at_start = !round.outcome.down_at_start.contains(&l.username);
        if l.alive_at_leave && at_start && l.when.phase != Phase::End {
            assert!(
                round.outcome.left.contains(&l.username),
                "{what}: not in round.left"
            );
        }
        assert!(l.team.is_some_and(|t| t < 2), "{what}");
    }
    // And nobody counts as gone who did not leave.
    for username in &round.outcome.left {
        let left = presence.leavers.iter().any(|l| &l.username == username);
        assert!(left, "{name}: round.left names someone who is no leaver");
    }

    for (i, r) in presence.reconnects.iter().enumerate() {
        let what = format!("{name}: reconnect {i} ({:?})", r.kind);
        let time = r.when.recording_time;
        assert!(time.is_some() && !r.username.is_empty(), "{what}");
        assert!(r.profile_id.is_some() && r.playerid.is_some(), "{what}");
        // One line of the feed says so: a reconnect or a join.
        let kind = match r.kind {
            ReconnectKind::Reconnect => Kind::PlayerReconnected,
            ReconnectKind::Join => Kind::PlayerJoined,
        };
        let window = (-LINE_WINDOW, LINE_WINDOW);
        let said = lines(round, &[kind], &r.username, time, window);
        assert_eq!(said.len(), 1, "{what}: lines");
        // They play from the next round: this one's header has them only
        // when they left during it.
        let left = (presence.leavers.iter())
            .rfind(|l| l.controller == Some(r.controller) && l.frame < r.frame);
        assert_eq!(r.left_before_recording, left.is_none(), "{what}");
        let seat = presence.seats.iter().find(|s| s.controller == r.controller);
        if r.left_before_recording {
            let state = seat.map(|s| s.seat);
            let open = [
                Some(SeatState::Reserved),
                Some(SeatState::Opened),
                Some(SeatState::Empty),
            ];
            assert!(open.contains(&state), "{what}: {state:?}");
            assert!(r.away_seconds.is_none(), "{what}");
        }
        // A reserved seat is taken by the player it was held for.
        if r.kind == ReconnectKind::Reconnect {
            let held = match left {
                Some(l) => l.profile_id.as_ref(),
                None => seat.and_then(|s| s.profile_id.as_ref()),
            };
            assert_eq!(held, r.profile_id.as_ref(), "{what}: another profile");
            assert!(r.new_playerid.is_some(), "{what}");
            let back = left.is_none_or(|l| l.returned == Some(Returned::SameRound));
            assert!(back, "{what}: the leaver is not back");
        }
        assert!(r.away_seconds.is_none_or(|s| s > 0.0), "{what}");
    }

    // A `connectionLost` line comes directly before a leave of its player.
    for l in &round.system_messages {
        if l.kind != Kind::ConnectionLost {
            continue;
        }
        let leave = presence.leavers.iter().find(|p| {
            let apart = p.when.recording_time.map(|t| t - at(l));
            let window = match p.source {
                Source::Slot => LINE_WINDOW,
                Source::FeedOnly => 2.0 * LINE_WINDOW,
            };
            Some(&p.username) == l.username.as_ref()
                && apart.is_some_and(|d| d >= -ROUNDING && d <= window + ROUNDING)
        });
        assert!(
            leave.is_some_and(|p| p.connection_lost),
            "{name}: a connectionLost line with no leave after it"
        );
    }
    let lost = presence
        .leavers
        .iter()
        .filter(|l| l.connection_lost)
        .count();
    let said = (round.system_messages.iter()).filter(|l| l.kind == Kind::ConnectionLost);
    assert_eq!(lost, said.count(), "{name}");

    // The stats say who left, and when.
    for s in round.player_stats() {
        let left = presence.leavers.iter().find(|l| l.username == s.username);
        assert_eq!(s.left_at, left.map(|l| l.when.elapsed), "{name}");
    }
}

#[test]
fn the_test_rounds_have_no_leavers_and_plain_seats() {
    for (name, round) in test_rounds() {
        let presence = round
            .presence
            .as_ref()
            .unwrap_or_else(|| panic!("{name}: not read"));
        check(name, round, presence);
        assert_eq!(presence, &Presence::default(), "{name}");
        let status = round.decode.get("presence").unwrap();
        assert_eq!(
            (status.status, status.count),
            (Status::Decoded, 0),
            "{name}"
        );
        assert!(status.warnings.is_empty(), "{name}: {:?}", status.warnings);
        assert!(round.outcome.left.is_empty(), "{name}");
        let json = serde_json::to_value(round).unwrap();
        for key in ["leavers", "reconnects", "seats"] {
            assert!(json.get(key).is_none(), "{name}: {key}");
        }
        for s in json["stats"].as_array().unwrap() {
            assert!(s.get("leftAt").is_none(), "{name}");
        }
    }
}

/// A partial read does not reach the feed: nothing is said of the seats.
#[test]
fn a_partial_read_has_no_presence() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings/valid/Y11S3");
    let round = Round::open(dir.join("custom_1.rec"), ReadMode::Partial).unwrap();
    assert!(round.presence.is_none());
    assert!(round.decode.get("presence").is_none());
    let json = serde_json::to_value(&round).unwrap();
    for key in ["leavers", "reconnects", "seats"] {
        assert!(json.get(key).is_none(), "{key}");
    }
}

/// Nobody left the test match: it has no rollup, and its stats no rounds
/// left or missed.
#[test]
fn a_match_nobody_left_has_no_presence() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings/valid/Y11S3");
    let folder = Match::open_with(&dir, ReadMode::Full).unwrap();
    assert!(folder.presence().is_empty());
    let json = serde_json::to_value(&folder).unwrap();
    assert!(json.get("presence").is_none());
    for s in json["stats"].as_array().unwrap() {
        assert!(s.get("roundsLeft").is_none() && s.get("roundsMissed").is_none());
    }
}

fn match_replay_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("R6_MATCH_REPLAY")?);
    dir.is_dir().then_some(dir)
}

/// What a match's rollup has to hold against its rounds.
fn check_match(name: &str, folder: &Match) {
    let rollup = folder.presence();
    let numbers: Vec<u32> = (folder.rounds.iter())
        .filter(|r| r.presence.is_some())
        .map(|r| r.header.round_number + 1)
        .collect();
    for (i, p) in rollup.iter().enumerate() {
        let what = format!("{name}: presence {i}");
        assert!(!p.events.is_empty() && !p.username.is_empty(), "{what}");
        assert!(p.events.is_sorted_by_key(|e| e.round), "{what}");
        assert!(
            p.events.iter().all(|e| numbers.contains(&e.round)),
            "{what}"
        );
        assert!(p.rounds_missed as usize <= numbers.len(), "{what}");
        // What happened during a round has a time, the rest has none.
        for e in &p.events {
            let live = matches!(
                e.kind,
                EventKind::Left | EventKind::Reconnected | EventKind::Joined
            );
            assert_eq!(e.when.is_some(), live, "{what}: {:?}", e.kind);
            assert_eq!(
                e.connection_lost.is_some(),
                e.kind == EventKind::Left,
                "{what}"
            );
            assert_eq!(
                e.alive_at_leave.is_some(),
                e.kind == EventKind::Left,
                "{what}"
            );
        }
        // `returned` tells of the last time they were away.
        let back = |e: &&replay_analyzer::presence::Event| {
            matches!(
                e.kind,
                EventKind::Reconnected | EventKind::Joined | EventKind::BackAtStart
            )
        };
        let away = |e: &&replay_analyzer::presence::Event| !back(e);
        let last_away = p.events.iter().rposition(|e| away(&e));
        let last_back = p.events.iter().rposition(|e| back(&e));
        assert_eq!(
            p.returned == Returned::Never,
            last_back < last_away,
            "{what}"
        );
        assert_eq!(p.likely.is_empty(), p.likely_source.is_none(), "{what}");
        if p.returned == Returned::Never {
            assert!(p.new_playerid.is_none(), "{what}");
        }
    }
    // Everyone who left a round they were in is followed, once.
    for round in &folder.rounds {
        let listed = |username: &str| round.header.players.iter().any(|p| p.username == username);
        for l in round.presence.iter().flat_map(|p| &p.leavers) {
            let followed = rollup.iter().filter(|p| p.profile_id == l.profile_id);
            let expected = usize::from(!l.never_spawned || listed(&l.username));
            assert!(
                followed.count() >= expected,
                "{name}: a leaver is not followed"
            );
        }
    }
    let mut keys: Vec<_> = rollup
        .iter()
        .map(|p| (&p.profile_id, &p.username))
        .collect();
    keys.sort();
    keys.dedup();
    assert_eq!(
        keys.len(),
        rollup.len(),
        "{name}: a player is followed twice"
    );
    // What was observed is said, and no verdict on a player. Names are
    // left out: a player may be called anything.
    let json = serde_json::to_value(&rollup).unwrap();
    for p in json.as_array().unwrap() {
        let types: Vec<_> = (p["events"].as_array().unwrap().iter())
            .map(|e| e["type"].clone())
            .collect();
        let said = format!("{} {} {types:?}", p["returned"], p["likely"]).to_lowercase();
        for word in ["crash", "rage", "abandon"] {
            assert!(!said.contains(word), "{name}: {said}");
        }
    }
    // The stats count the rounds left and missed.
    let stats = folder.player_stats();
    for p in &rollup {
        let left = p
            .events
            .iter()
            .filter(|e| e.kind == EventKind::Left)
            .count();
        if let Some(s) = stats.iter().find(|s| s.username == p.username) {
            assert_eq!(s.rounds_left as usize, left, "{name}");
            assert_eq!(s.rounds_missed, p.rounds_missed, "{name}");
        }
    }
}

/// Real rounds: a seat that is not a player's was left and is not in the
/// header, every change of a seat has its line of the feed and every such
/// line its change, a leaver who was alive counts as gone, and a reserved
/// seat goes back to the player it was held for.
#[test]
fn real_seats_and_lines_agree() {
    let Some(root) = match_replay_dir() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let (mut rounds, mut matches, mut followed) = (0, 0, 0);
    let mut found: BTreeMap<String, usize> = BTreeMap::new();
    let mut count = |what: String| *found.entry(what).or_default() += 1;
    for dir in replay_analyzer::matches::find_match_folders(&root).unwrap() {
        let folder = Match::open_with(&dir, ReadMode::Full).unwrap();
        for round in &folder.rounds {
            // Names the round without naming its players.
            let number = round.header.round_number + 1;
            let name = format!("{} R{number}", round.header.match_id);
            let Some(presence) = &round.presence else {
                continue;
            };
            rounds += 1;
            check(&name, round, presence);
            for l in &presence.leavers {
                let state = match (l.alive_at_leave, l.never_spawned) {
                    (true, _) => "alive",
                    (false, true) => "never spawned",
                    (false, false) => "dead",
                };
                count(format!("left {state}"));
                if l.connection_lost {
                    count("left with connectionLost".to_owned());
                }
                if l.source == Source::FeedOnly {
                    count("left by the feed only".to_owned());
                }
            }
            for r in &presence.reconnects {
                count(format!("{:?}", r.kind).to_lowercase());
            }
            for s in &presence.seats {
                count(format!("seat {:?}", s.seat).to_lowercase());
            }
            let status = round.decode.get("presence").unwrap();
            if status.status != Status::Decoded {
                count("rounds not decoded".to_owned());
            }
        }
        if folder.rounds.iter().any(|r| r.presence.is_some()) {
            matches += 1;
            check_match(&dir.file_name().unwrap().to_string_lossy(), &folder);
            let rollup = folder.presence();
            followed += rollup.len();
            for p in &rollup {
                count(format!("returned {:?}", p.returned).to_lowercase());
            }
        }
    }
    eprintln!("{rounds} rounds of {matches} matches, {followed} players followed: {found:?}");
    assert!(rounds > 0, "no Y11S3 rounds under {}", root.display());
}
