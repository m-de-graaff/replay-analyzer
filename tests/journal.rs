//! Sessions, the journal and insights against real replays: the ten rounds
//! of the Y11S3 test match in `test_recordings/valid/Y11S3`, and every
//! match folder in the folder `R6_MATCH_REPLAY` names, when set. That
//! folder changes as matches are played and is only read, so the tests on
//! it check what must hold for any such folder and print aggregates only.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use replay_analyzer::journal::{
    self, InsightRules, Journal, MatchOutcome, MatchRecord, PlaySession, SessionRules, Target,
    TiltRules, Totals,
};
use replay_analyzer::{Match, ReadMode};

/// The test match, read once.
fn test_match() -> &'static Match {
    static MATCH: std::sync::OnceLock<Match> = std::sync::OnceLock::new();
    MATCH.get_or_init(|| {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings/valid/Y11S3");
        Match::open(&dir).unwrap()
    })
}

/// A `MatchReplay` folder as the game writes it, from `R6_MATCH_REPLAY`.
fn match_replay_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("R6_MATCH_REPLAY")?);
    dir.is_dir().then_some(dir)
}

/// What holds for the sessions of any set of records under any rules.
fn check_sessions(records: &[MatchRecord], cut: &[PlaySession], rules: &SessionRules) {
    // Every match is in exactly one session.
    let keys: HashSet<String> = records.iter().map(MatchRecord::key).collect();
    let mut seen = HashSet::new();
    for m in cut.iter().flat_map(|s| &s.matches) {
        assert!(keys.contains(&m.match_id), "a match no record has");
        assert!(seen.insert(m.match_id.clone()), "a match in two sessions");
    }
    assert_eq!(seen.len(), keys.len(), "a match in no session");

    for s in cut {
        let first = s.matches.first().expect("a session has a match");
        assert_eq!(s.id, format!("session:{}", first.match_id));
        assert_eq!(s.started, first.started);
        assert!(s.ended >= s.started);
        let positions: Vec<u32> = s.matches.iter().map(|m| m.position).collect();
        assert_eq!(positions, (1..=s.matches.len() as u32).collect::<Vec<_>>());
        // Matches are in start order and the session spans them all.
        for pair in s.matches.windows(2) {
            assert!(pair[0].started <= pair[1].started);
        }
        for m in &s.matches {
            assert!(m.started >= s.started && m.ended.unwrap_or(m.started) <= s.ended);
        }
        assert_eq!(first.gap_seconds, None);
        for m in &s.matches[1..] {
            let gap = m.gap_seconds.expect("a later match has a gap");
            assert!((0..rules.session_gap_seconds).contains(&gap));
            assert_eq!(m.after_break, gap >= rules.break_seconds);
        }
        // One break per match that follows one, and no match is being
        // played during a break.
        let after_break = s.matches.iter().filter(|m| m.after_break).count();
        assert_eq!(s.breaks.len(), after_break);
        for b in &s.breaks {
            assert_eq!(b.seconds, (b.to - b.from).num_seconds());
            assert!(b.seconds >= rules.break_seconds && b.seconds < rules.session_gap_seconds);
            for m in &s.matches {
                let end = m.ended.unwrap_or(m.started);
                assert!(
                    end <= b.from || m.started >= b.to,
                    "a break overlaps a match"
                );
            }
            let before = s.matches.iter().position(|m| m.match_id == b.before);
            let after = s.matches.iter().position(|m| m.match_id == b.after);
            assert_eq!(before, after.map(|i| i + 1));
        }
        for pair in s.breaks.windows(2) {
            assert!(pair[0].to <= pair[1].from);
        }
        let r = &s.record;
        let total = r.wins + r.losses + r.draws + r.undecided;
        assert_eq!(total as usize, s.matches.len());
        assert_eq!(s.totals.matches as usize, s.matches.len());
        assert_eq!((s.totals.wins, s.totals.losses), (r.wins, r.losses));
        let launched: u32 = s.launches.iter().map(|l| l.matches).sum();
        assert!(launched as usize <= s.matches.len());
        assert!(s.seconds_since_break() <= (s.ended - s.started).num_seconds());
    }
    // Sessions are in order and a session gap apart.
    for pair in cut.windows(2) {
        assert!(pair[0].started < pair[1].started);
        let gap = (pair[1].started - pair[0].ended).num_seconds();
        assert!(gap >= rules.session_gap_seconds || rules.split_on_launch);
    }
    // Derived data is the same when derived again, and reads back.
    assert_eq!(cut, journal::sessions(records, rules));
    let json = serde_json::to_string(cut).unwrap();
    let back: Vec<PlaySession> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, cut);
}

/// What holds for the insights of any records.
fn check_insights(records: &[MatchRecord], rules: &SessionRules) {
    let limits = InsightRules::default();
    let i = journal::insights(records, rules, &limits);
    let matches = i.overall.sample;
    assert_eq!(i.overall.totals, Totals::of(records));
    let sum = |groups: &[journal::Group]| groups.iter().map(|g| g.sample).sum::<u32>();
    assert_eq!(sum(&i.by_position), matches);
    assert_eq!(sum(&i.by_session_length), matches);
    assert!(sum(&i.by_time_of_day) <= matches);
    // A session's first match has none before it.
    let later = matches - i.sessions;
    for c in [&i.on_loss_streak, &i.after_break_vs_without] {
        assert_eq!(c.first.sample + c.second.sample, later);
    }
    let after = &i.after_loss_vs_after_win;
    assert!(after.first.sample + after.second.sample <= later);
    // No rate and no effect below the minimum sample.
    let compared = [after, &i.on_loss_streak, &i.after_break_vs_without];
    let groups = (compared.iter().flat_map(|c| [&c.first, &c.second]))
        .chain(&i.by_position)
        .chain(&i.by_session_length)
        .chain(&i.by_time_of_day)
        .chain([&i.overall]);
    for g in groups {
        assert_eq!(g.enough, g.sample >= limits.min_matches, "{}", g.label);
        if !g.enough {
            assert_eq!(g.rates, journal::Rates::default(), "{}", g.label);
        }
        if let (Some(rate), Some([low, high])) = (g.rates.win_rate, g.rates.win_rate_interval) {
            assert!(0.0 <= low && low <= rate && rate <= high && high <= 1.0);
        }
    }
    for c in compared {
        assert!(c.effect.is_none() || (c.first.enough && c.second.enough));
    }
    let rounds: u32 = records.iter().map(|r| r.rounds.len() as u32).sum();
    for c in [&i.after_dying_first, &i.after_round_losses] {
        assert!(c.first.sample + c.second.sample <= rounds);
        for g in [&c.first, &c.second] {
            assert_eq!(g.enough, g.sample >= limits.min_rounds);
            assert!(g.enough || (g.win_rate.is_none() && g.kills_per_round.is_none()));
        }
        assert!(c.effect.is_none() || (c.first.enough && c.second.enough));
    }
    // Insights read back; rates may differ in the last digit, since
    // serde_json does not parse every float exactly.
    let json = serde_json::to_string(&i).unwrap();
    let back: journal::Insights = serde_json::from_str(&json).unwrap();
    assert_eq!(back.overall.totals, i.overall.totals);
    assert_eq!(
        back.after_round_losses.first.sample,
        i.after_round_losses.first.sample
    );
}

#[test]
fn the_test_match_is_one_session_of_one_match() {
    let m = test_match();
    let summary = m.summary().unwrap();
    // A spectator recorded it: there is no "you", so no player line.
    let record = MatchRecord::from_match(m, true).unwrap();
    assert_eq!(record.match_id, summary.match_id);
    assert_eq!(record.rounds.len(), 10);
    assert!(record.player.is_none());
    assert!(
        record
            .rounds
            .iter()
            .all(|r| r.won.is_none() && r.player.is_none())
    );
    assert!(!matches!(
        record.outcome,
        MatchOutcome::Win | MatchOutcome::Loss
    ));
    assert_eq!(
        (record.started, record.ended),
        (summary.start_time, summary.end_time)
    );
    assert!(record.ended.is_some_and(|end| end > record.started));
    // The replay holds the recording PC's offset from UTC, in quarter hours.
    let offset = record.utc_offset_minutes.expect("Y11S3 holds a UTC offset");
    assert_eq!(offset % 15, 0);
    assert!(record.local_start().is_some());
    // The files are not in a folder the game named.
    assert!(record.launch.is_none() && record.folder.is_none());
    let json = serde_json::to_string(&record).unwrap();
    assert_eq!(serde_json::from_str::<MatchRecord>(&json).unwrap(), record);
    assert_eq!(MatchRecord::from_summary(&summary).rounds.len(), 10);

    let rules = SessionRules::default();
    let records = [record.clone(), record];
    let cut = journal::sessions(&records, &rules);
    assert_eq!((cut.len(), cut[0].matches.len()), (1, 1));
    assert_eq!(cut[0].id, format!("session:{}", summary.match_id));
    assert_eq!(cut[0].record.undecided + cut[0].record.draws, 1);
    assert!(cut[0].breaks.is_empty() && cut[0].launches.is_empty());
    check_sessions(&records[..1], &cut, &rules);
    check_insights(&records[..1], &rules);
    let signal = journal::tilt(&cut[0], &rules, &TiltRules::default());
    assert_eq!((signal.sample, signal.loss_streak), (1, 0));
    assert!(!signal.suggest_break);
}

#[test]
fn the_test_match_seen_as_each_player_matches_the_match_stats() {
    let m = test_match();
    let summary = m.summary().unwrap();
    let stats = m.player_stats();
    assert!(!stats.is_empty());
    let mut winners = 0;
    for s in &stats {
        let record = MatchRecord::from_match_as(m, &s.username).unwrap();
        let line = record.player.as_ref().expect("the player has a line");
        assert_eq!(line.username, s.username);
        assert_eq!(
            (
                line.rounds,
                line.kills,
                line.deaths,
                line.assists,
                line.headshots
            ),
            (s.rounds, s.kills, s.deaths, s.assists, s.headshots)
        );
        assert_eq!(
            (line.damage_dealt, line.damage_taken),
            (s.damage_dealt, s.damage_taken)
        );
        // The round lines add up to the match line.
        let rounds: Vec<_> = record
            .rounds
            .iter()
            .filter_map(|r| r.player.as_ref())
            .collect();
        assert_eq!(rounds.len() as u32, line.rounds);
        assert_eq!(rounds.iter().map(|r| r.kills).sum::<u32>(), line.kills);
        assert_eq!(rounds.iter().filter(|r| r.died).count() as u32, line.deaths);
        let first_deaths = rounds.iter().filter(|r| r.opening_death).count() as u32;
        assert_eq!(first_deaths, line.opening_deaths);
        assert!(rounds.iter().all(|r| !r.opening_death || r.died));
        assert!(rounds.iter().all(|r| !r.opening_kill || r.kills > 0));
        // Score and rounds won are from the player's side.
        let team = s.team_index;
        let score = summary.result.final_score;
        assert_eq!(record.score, [score[team], score[team ^ 1]]);
        let won = record.rounds.iter().filter(|r| r.won == Some(true)).count() as u32;
        assert_eq!(won, score[team]);
        winners += u32::from(record.outcome == MatchOutcome::Win);
        let totals = Totals::of([&record]);
        assert_eq!((totals.rounds, totals.kills), (s.rounds, s.kills));
        check_insights(&[record], &SessionRules::default());
    }
    // At most one first kill and one first death per round over everyone.
    let records: Vec<_> = (stats.iter())
        .filter_map(|s| MatchRecord::from_match_as(m, &s.username))
        .collect();
    let first = |f: fn(&journal::PlayerLine) -> u32| {
        records
            .iter()
            .filter_map(|r| r.player.as_ref())
            .map(f)
            .sum::<u32>()
    };
    assert!(first(|p| p.opening_deaths) <= 10 && first(|p| p.opening_kills) <= 10);
    assert!(first(|p| p.opening_deaths) > 0);
    if summary.result.winner.is_some() {
        assert!(winners > 0 && (winners as usize) < stats.len());
    }
    assert!(MatchRecord::from_match_as(m, "nobody by this name").is_none());
}

#[test]
fn a_journal_about_the_test_match_saves_and_merges() {
    let m = test_match();
    let record = MatchRecord::from_match(m, false).unwrap();
    let cut = journal::sessions(std::slice::from_ref(&record), &SessionRules::default());
    let now = record.ended.unwrap();
    let on_match = Target::Match {
        match_id: record.key(),
    };
    let on_session = Target::Session {
        session_id: cut[0].id.clone(),
    };
    let on_round = Target::Round {
        match_id: record.key(),
        round: record.rounds[2].number,
    };
    let mut j = Journal::new();
    j.add_tag(on_match.clone(), "scrim", now);
    j.add_tag(on_round, "site hold", now);
    j.add_note(on_session.clone(), "first night on the new season", now);

    let dir =
        std::env::temp_dir().join(format!("replay-analyzer-journal-it-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("journal.json");
    j.save(&path).unwrap();
    let mut other = Journal::load(&path).unwrap();
    assert_eq!(other, j);
    // The other copy removes the tag; this one adds a note meanwhile.
    let later = now + chrono::Duration::minutes(5);
    let tag = j.tags_on(&on_match).next().unwrap().id.clone();
    assert!(other.remove_tag(&tag, later));
    j.add_note(on_match.clone(), "review round 3", later);
    j.merge(&other);
    j.save(&path).unwrap();
    let merged = Journal::load(&path).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(merged, j);
    assert_eq!(merged.tags_on(&on_match).count(), 0);
    assert_eq!(
        (merged.live_tags().count(), merged.live_notes().count()),
        (1, 2)
    );
    // What the journal points at is found again in the derived data.
    let note = merged.notes_on(&on_session).next().unwrap();
    let Target::Session { session_id } = &note.target else {
        panic!("a session note");
    };
    assert!(journal::find_session(&cut, session_id).is_some());
    // The journal holds ids and the player's words, nothing of the replay.
    let json = merged.to_json().unwrap();
    assert!(!json.contains(&record.map) && !json.contains("kills"));
}

#[test]
fn real_matches_are_each_in_one_session() {
    let Some(root) = match_replay_dir() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let records = journal::read_folder(&root, ReadMode::Header).unwrap();
    assert!(!records.is_empty());
    for r in &records {
        assert!(r.player.is_none(), "a header-only read has no player line");
        assert!(r.launch.is_some() && r.folder.is_some());
        assert!(r.ended.is_none_or(|end| end >= r.started));
    }
    for pair in records.windows(2) {
        assert!(pair[0].started <= pair[1].started);
    }
    // A library scan gives the same records.
    let library = replay_analyzer::library::scan(&root, ReadMode::Header).unwrap();
    let mut from_library = journal::records_from_library(&library);
    from_library.sort_by_key(|r| (r.started, r.key()));
    assert_eq!(from_library, records);

    let defaults = SessionRules::default();
    let split = SessionRules {
        split_on_launch: true,
        ..defaults.clone()
    };
    let fine = SessionRules {
        break_seconds: 60,
        session_gap_seconds: 300,
        split_on_launch: false,
    };
    for rules in [&defaults, &split, &fine] {
        let cut = journal::sessions(&records, rules);
        check_sessions(&records, &cut, rules);
        check_insights(&records, rules);
        for s in &cut {
            let signal = journal::tilt(s, rules, &TiltRules::default());
            assert_eq!(signal.sample as usize, s.matches.len());
            assert!(signal.matches_since_break >= 1 && signal.matches_since_break <= signal.sample);
            assert!(signal.loss_streak <= signal.matches_since_break);
            assert_eq!(
                signal.before.matches + signal.during.matches,
                signal.matches_since_break
            );
        }
    }
    // Splitting on restarts can only cut finer.
    let cut = journal::sessions(&records, &defaults);
    assert!(journal::sessions(&records, &split).len() >= cut.len());
    // Aggregates only: nothing here names a match or a player.
    let mut gaps: Vec<i64> = (cut.iter().flat_map(|s| &s.matches))
        .filter_map(|m| m.gap_seconds)
        .collect();
    gaps.sort_unstable();
    let launches: usize = cut.iter().map(|s| s.launches.len()).sum();
    eprintln!(
        "{} matches, {} sessions, {} breaks, {} game launches; gaps inside sessions: {} from {:?} to {:?} s, median {:?}",
        records.len(),
        cut.len(),
        cut.iter().map(|s| s.breaks.len()).sum::<usize>(),
        launches,
        gaps.len(),
        gaps.first(),
        gaps.last(),
        gaps.get(gaps.len() / 2),
    );
}

/// Reads every real match in full, which takes a while in a debug build:
/// `cargo test --release --test journal -- --ignored --nocapture`.
#[test]
#[ignore = "needs R6_MATCH_REPLAY"]
fn real_matches_read_in_full_give_the_recording_players_lines() {
    let dir = std::env::var_os("R6_MATCH_REPLAY").expect("R6_MATCH_REPLAY names a folder");
    let records = journal::read_folder(Path::new(&dir), ReadMode::Full).unwrap();
    assert!(!records.is_empty());
    let rules = SessionRules::default();
    let mut with_line = 0;
    for r in &records {
        let Some(line) = &r.player else { continue };
        with_line += 1;
        assert!(line.rounds as usize <= r.rounds.len());
        let rounds: Vec<_> = r.rounds.iter().filter_map(|l| l.player.as_ref()).collect();
        assert_eq!(rounds.len() as u32, line.rounds);
        assert_eq!(rounds.iter().map(|l| l.kills).sum::<u32>(), line.kills);
        assert_eq!(rounds.iter().filter(|l| l.died).count() as u32, line.deaths);
        assert!(line.opening_deaths <= line.deaths && line.opening_kills <= line.kills);
        assert!(line.headshots <= line.kills);
        // A player's own recording says which rounds their team won.
        let won = r.rounds.iter().filter(|l| l.won == Some(true)).count() as u32;
        let lost = r.rounds.iter().filter(|l| l.won == Some(false)).count() as u32;
        assert_eq!([won, lost], r.score);
        match r.outcome {
            MatchOutcome::Win => assert!(won > lost),
            MatchOutcome::Loss => assert!(lost > won),
            _ => {}
        }
    }
    assert!(with_line > 0, "no match has a recording player's line");
    // The header-only read cuts the same sessions.
    let headers = journal::read_folder(Path::new(&dir), ReadMode::Header).unwrap();
    let cut = journal::sessions(&records, &rules);
    let ids = |cut: &[PlaySession]| cut.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&cut), ids(&journal::sessions(&headers, &rules)));
    check_sessions(&records, &cut, &rules);
    check_insights(&records, &rules);

    let limits = InsightRules::default();
    let i = journal::insights(&records, &rules, &limits);
    let enough = |c: &journal::Compared| (c.first.sample, c.second.sample, c.effect.is_some());
    let rounds =
        |c: &journal::RoundsCompared| (c.first.sample, c.second.sample, c.effect.is_some());
    eprintln!(
        "{} matches ({with_line} with a player line) in {} sessions",
        records.len(),
        cut.len()
    );
    eprintln!(
        "samples (first, second, effect given): after loss/win {:?}, loss streak {:?}, break {:?}, died first {:?}, round losses {:?}",
        enough(&i.after_loss_vs_after_win),
        enough(&i.on_loss_streak),
        enough(&i.after_break_vs_without),
        rounds(&i.after_dying_first),
        rounds(&i.after_round_losses),
    );
    let sizes = |groups: &[journal::Group]| groups.iter().map(|g| g.sample).collect::<Vec<_>>();
    eprintln!(
        "by position {:?}, by session length {:?}, by time of day {:?}",
        sizes(&i.by_position),
        sizes(&i.by_session_length),
        sizes(&i.by_time_of_day),
    );
}
