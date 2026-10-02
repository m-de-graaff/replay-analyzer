//! Profile stats joined onto the players of real rounds by profile id.
//!
//! The stats are made up at test time from each profile id, in the
//! provider's JSON shape, so nothing here is anyone's real rank. With
//! `R6_MATCH_REPLAY` set the same join runs over a real `MatchReplay`
//! folder, which is only read and of which nothing is printed.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use replay_analyzer::profiles::{
    self, Attribution, Basis, ProfileCache, ProfileKey, ProfileSource, ProfileStats, RankSystem,
};
use replay_analyzer::{Match, MatchSummary, ReadMode};
use serde_json::json;

/// The ten rounds of the Y11S3 test match, summarised.
fn test_match() -> Option<MatchSummary> {
    let root = match std::env::var_os("R6_TEST_DATA") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings"),
    };
    let dir = root.join("valid").join("Y11S3");
    if !dir.is_dir() {
        eprintln!("skipping: no test replays in {}", dir.display());
        return None;
    }
    Match::open_with(&dir, ReadMode::Partial).unwrap().summary()
}

fn hash(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// Made-up rank points for a profile id: 1000 to 4999, the same every time.
fn points(profile_id: &str) -> i32 {
    1000 + (hash(profile_id) % 4000) as i32
}

/// A source that answers every key with made-up stats in the provider's
/// JSON, read back through the adapter: what an app's source does with its
/// proxy's answers.
struct Synthetic {
    at: DateTime<Utc>,
    /// Added to everyone's rank points and match count.
    played: i32,
}

impl ProfileSource for Synthetic {
    fn fetch(&self, keys: &[ProfileKey]) -> profiles::Result<Vec<ProfileStats>> {
        let mut out = Vec::new();
        for key in keys {
            let rp = points(&key.profile_id) + 20 * self.played;
            let response = json!({
                "player": {"nameOnPlatform": key.name, "platformType": key.platform_type()},
                "account": {"level": hash(&key.name) % 400},
                "stats": {"platform_families_full_profiles": [{
                    "profile_id": key.profile_id,
                    "platform_family": key.platform_families(),
                    "board_ids_full_profiles": [{"board_id": "ranked", "full_profiles": [{
                        "profile": {
                            "season_id": 43, "rank": (rp - 1000) / 100 + 1, "rank_points": rp,
                            "max_rank_points": rp + 30
                        },
                        "season_statistics": {
                            "kills": 90, "deaths": 60,
                            "match_outcomes": {"wins": 9 + self.played, "losses": 6, "abandons": 0}
                        }
                    }]}]
                }]}
            });
            out.extend(profiles::from_r6data_value(&response, key, self.at)?);
        }
        Ok(out)
    }
}

fn mean(values: &[i32]) -> f64 {
    f64::from(values.iter().sum::<i32>()) / values.len() as f64
}

#[test]
fn synthetic_profiles_join_every_player_of_the_test_match() {
    let Some(summary) = test_match() else { return };
    let now = summary.start_time + Duration::hours(3);
    let day = Duration::hours(24);
    let mut cache = ProfileCache::new();

    // Everyone has a profile id and a platform to look up by.
    let keys = profiles::requests_for(&summary, &cache, now, day);
    let listed: usize = summary.teams.iter().map(|t| t.players.len()).sum();
    assert_eq!((keys.len(), listed), (10, 10));
    for key in &keys {
        assert_eq!(key.profile_id.len(), 36, "{key:?}");
        assert_eq!(
            (key.platform_type(), key.platform_families()),
            ("uplay", "pc")
        );
        assert!(!key.name.is_empty() && !key.name_is_nickname);
    }

    let source = Synthetic { at: now, played: 0 };
    assert_eq!(cache.refresh(&source, &keys, now).unwrap(), 10);
    assert!(profiles::requests_for(&summary, &cache, now, day).is_empty());
    assert_eq!(
        profiles::requests_for(&summary, &cache, now + day, day).len(),
        10
    );

    let stats = cache.profiles();
    let lobby = profiles::lobby_strength(&summary, &stats);
    assert_eq!(lobby.basis, Basis::RankPoints);
    assert_eq!(
        (lobby.players, lobby.profiles_known, lobby.ranks_known),
        (10, 10, 10)
    );
    assert_eq!(lobby.levels_known, 10);
    assert_eq!(lobby.max_fetch_gap_seconds, Some(3 * 3600));

    // Each player got the stats made from their own id, and the team
    // figures are those of its players.
    let mut means = [0.0; 2];
    for (t, team) in summary.teams.iter().enumerate() {
        let expected: Vec<i32> = team.players.iter().map(|p| points(&p.profile_id)).collect();
        for (p, want) in team.players.iter().zip(&expected) {
            let got = lobby.lobby.iter().find(|l| l.profile_id == p.profile_id);
            let got = got.unwrap();
            assert_eq!((got.team, got.rank_points), (t, Some(*want)));
            assert_eq!(got.rank, Some(RankSystem::RANKED_3.rank_of_points(*want)));
            // The replay's level wins over the provider's.
            assert_eq!(got.level, p.level);
            let pct = got.percentile.unwrap();
            assert!((0.0..=100.0).contains(&pct));
        }
        let spread = lobby.teams[t].rank_points.unwrap();
        means[t] = mean(&expected);
        assert_eq!(spread.known, 5);
        assert!((spread.mean - means[t]).abs() < 1e-9);
        assert_eq!(spread.min, f64::from(*expected.iter().min().unwrap()));
        assert_eq!(spread.max, f64::from(*expected.iter().max().unwrap()));
        assert!(spread.min <= spread.median && spread.median <= spread.max);
    }
    let from = lobby.from_team;
    assert_eq!(from, summary.your_team.unwrap_or(0));
    let difference = lobby.difference.unwrap();
    assert!((difference - (means[from] - means[from ^ 1])).abs() < 1e-9);
    let p = lobby.expected_win.unwrap();
    assert!((p - profiles::expected_win(difference)).abs() < 1e-12);
    assert_eq!(p > 0.5, difference > 0.0);
    // A percentile of your own needs you to be one of the ten.
    assert_eq!(lobby.your_percentile.is_some(), summary.your_team.is_some());
}

#[test]
fn levels_stand_in_when_no_profile_is_known() {
    let Some(summary) = test_match() else { return };
    let lobby = profiles::lobby_strength(&summary, &[]);
    assert_eq!(lobby.basis, Basis::Level);
    assert_eq!(
        (lobby.profiles_known, lobby.ranks_known, lobby.levels_known),
        (0, 0, 10)
    );
    assert_eq!(lobby.expected_win, None);
    for (t, team) in summary.teams.iter().enumerate() {
        let levels: Vec<i32> = team
            .players
            .iter()
            .map(|p| p.level.unwrap() as i32)
            .collect();
        assert!((lobby.teams[t].level.unwrap().mean - mean(&levels)).abs() < 1e-9);
        assert_eq!(lobby.teams[t].rank_points, None);
    }

    // Half the lobby known: still rank points, over those known.
    let keys = profiles::keys_of(&summary);
    let source = Synthetic {
        at: summary.start_time,
        played: 0,
    };
    let some: Vec<ProfileKey> = keys.into_iter().step_by(2).collect();
    let stats = source.fetch(&some).unwrap();
    let lobby = profiles::lobby_strength(&summary, &stats);
    assert_eq!((lobby.profiles_known, lobby.ranks_known), (5, 5));
    assert_eq!(lobby.basis, Basis::RankPoints);
    let known: usize = lobby
        .teams
        .iter()
        .map(|t| t.rank_points.unwrap().known)
        .sum();
    assert_eq!(known, 5);
}

#[test]
fn snapshots_around_a_custom_match_are_not_pinned_on_it() {
    let Some(summary) = test_match() else { return };
    let key = profiles::keys_of(&summary).remove(0);
    let before = summary.start_time - Duration::hours(1);
    let after = summary.start_time + Duration::hours(4);

    let mut cache = ProfileCache::new();
    for (at, played) in [(before, 0), (after, 1)] {
        let keys = std::slice::from_ref(&key);
        cache.refresh(&Synthetic { at, played }, keys, at).unwrap();
    }
    let history = cache.history(&key);
    assert_eq!(history.len(), 2);

    // The provider counted one ranked match between the snapshots, but
    // the match of the replays is a custom game: the change is the span's.
    assert_eq!(summary.queue, "custom");
    let progress = profiles::rank_progress(history, std::slice::from_ref(&summary));
    assert_eq!(progress.profile_id, key.profile_id);
    assert_eq!(progress.spans.len(), 1);
    let span = &progress.spans[0];
    assert_eq!((span.change, span.provider_matches), (Some(20), Some(1)));
    assert!(span.replay_matches.is_empty());
    assert_eq!(
        (span.attribution, span.match_id.as_deref()),
        (Attribution::Span, None)
    );

    // Had it been a ranked match, it would be that match's.
    let mut ranked = summary.clone();
    ranked.queue = "ranked";
    let progress = profiles::rank_progress(history, &[ranked]);
    let span = &progress.spans[0];
    assert_eq!(span.attribution, Attribution::Match);
    assert_eq!(span.match_id.as_deref(), Some(summary.match_id.as_str()));
    let rp = points(&key.profile_id) + 20;
    assert_eq!(progress.rank_points, Some(rp));
    assert_eq!(progress.season_peak.unwrap().rank_points, rp + 30);

    // The cache survives a save and a load.
    let dir = std::env::temp_dir().join(format!("profiles-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("profiles.json");
    cache.save(&path).unwrap();
    assert_eq!(ProfileCache::load(&path).unwrap(), cache);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A real `MatchReplay` folder, from `R6_MATCH_REPLAY`: every player with a
/// profile id joins, whatever the folder holds. Nothing of it is printed.
#[test]
fn synthetic_profiles_join_a_real_folder() {
    let Some(root) = std::env::var_os("R6_MATCH_REPLAY").map(PathBuf::from) else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let mut cache = ProfileCache::new();
    for (i, dir) in (replay_analyzer::matches::find_match_folders(&root)
        .unwrap()
        .iter())
    .enumerate()
    {
        let Ok(m) = Match::open_with(dir, ReadMode::Header) else {
            continue;
        };
        let Some(summary) = m.summary() else { continue };
        let now = summary.start_time + Duration::hours(1);
        let keys = profiles::requests_for(&summary, &cache, now, Duration::days(3650));
        cache
            .refresh(&Synthetic { at: now, played: 0 }, &keys, now)
            .unwrap();
        let lobby = profiles::lobby_strength(&summary, &cache.profiles());
        let with_id = (summary.teams.iter().flat_map(|t| &t.players))
            .filter(|p| !p.profile_id.is_empty())
            .count();
        assert_eq!(lobby.profiles_known, with_id, "match folder {i}");
        assert_eq!(lobby.ranks_known, with_id, "match folder {i}");
        for l in lobby.lobby.iter().filter(|l| !l.profile_id.is_empty()) {
            assert_eq!(
                l.rank_points,
                Some(points(&l.profile_id.to_ascii_lowercase()))
            );
        }
        if summary.your_team.is_some() && lobby.basis == Basis::RankPoints {
            assert!(lobby.your_percentile.is_some(), "match folder {i}");
        }
    }
}
