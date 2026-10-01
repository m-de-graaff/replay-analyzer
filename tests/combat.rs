//! The round timeline and damage hits (Y11S3), against the test rounds and,
//! with `R6_MATCH_REPLAY` set, a real `MatchReplay` folder.

use std::path::{Path, PathBuf};

use replay_analyzer::combat::{AttackerSource, Combat, HitResult, TimelineKind};
use replay_analyzer::{MatchUpdateType, ReadMode, Round};

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

fn combat<'a>(name: &str, round: &'a Round) -> &'a Combat {
    round
        .combat
        .as_ref()
        .unwrap_or_else(|| panic!("{name}: no combat"))
}

fn is_player(round: &Round, name: &str) -> bool {
    round.header.players.iter().any(|p| p.username == name)
}

/// Every kill of the kill feed is in the timeline, with the same killer and
/// headshot flag, at the same moment.
#[test]
fn the_timeline_holds_every_kill_of_the_feed() {
    let mut kills = 0;
    for (name, round) in test_rounds() {
        let combat = combat(name, round);
        assert!(combat.warnings.is_empty(), "{name}: {:?}", combat.warnings);
        for kill in &round.match_feedback {
            if kill.kind != MatchUpdateType::Kill {
                continue;
            }
            kills += 1;
            let at = kill.recording_time.unwrap();
            let found = combat.events.iter().any(|e| {
                matches!(e.kind, TimelineKind::Kill | TimelineKind::TeamKill)
                    && e.username == kill.target
                    && e.by.as_deref() == Some(kill.username.as_str())
                    && e.headshot == kill.headshot
                    && e.recording_time.is_some_and(|t| (t - at).abs() < 0.5)
            });
            assert!(
                found,
                "{name}: {} killing {} at {at} is not in the timeline",
                kill.username, kill.target
            );
        }
        for e in &combat.events {
            assert!(is_player(round, &e.username), "{name}: {e:?}");
            if let Some(by) = &e.by {
                assert!(is_player(round, by), "{name}: {e:?}");
            }
            // A kill names its weapon; downs and revives have none.
            let kill = matches!(e.kind, TimelineKind::Kill | TimelineKind::TeamKill);
            assert_eq!(e.headshot.is_some(), kill, "{name}: {e:?}");
            assert_eq!(
                e.weapon.is_some(),
                kill || e.kind == TimelineKind::Death,
                "{name}: {e:?}"
            );
        }
    }
    assert!(kills > 50, "{kills} kills in the test rounds");
}

/// How far apart the movement stream and the HUD can write one hit.
const SAME_HIT: f64 = 0.15;

/// Hits are read from the victims' bodies; the HUD's health series, read
/// from another stream, has to agree with them.
#[test]
fn hits_agree_with_the_health_series() {
    let (mut hits, mut compared, mut amounts, mut from_timeline) = (0, 0, 0, 0);
    for (name, round) in test_rounds() {
        let combat = combat(name, round);
        hits += combat.hits.len();
        for (i, hit) in combat.hits.iter().enumerate() {
            assert!(is_player(round, &hit.username), "{name}: {hit:?}");
            assert!(hit.direction <= 7, "{name}: {hit:?}");
            assert!(
                hit.multiplier > 0.0 && hit.multiplier <= 1.0,
                "{name}: {hit:?}"
            );
            let at = hit.recording_time.unwrap();
            assert_eq!(hit.by.is_some(), hit.attacker_source.is_some());
            if let Some(by) = &hit.by {
                assert!(is_player(round, by), "{name}: {hit:?}");
                assert_ne!(by, &hit.username, "{name}: {hit:?}");
            }
            if hit.attacker_source == Some(AttackerSource::Timeline) {
                from_timeline += 1;
                assert_ne!(hit.result, HitResult::Alive, "{name}: {hit:?}");
                let named = combat.events.iter().any(|e| {
                    matches!(
                        e.kind,
                        TimelineKind::Kill | TimelineKind::TeamKill | TimelineKind::Down
                    ) && e.username == hit.username
                        && e.by == hit.by
                        && e.recording_time.is_some_and(|t| (t - at).abs() <= 0.5)
                });
                assert!(named, "{name}: {hit:?}");
            }
            if hit.result != HitResult::Alive {
                assert_eq!(hit.health, 0, "{name}: {hit:?}");
                continue;
            }
            let damage = hit.damage.unwrap_or_else(|| panic!("{name}: {hit:?}"));
            assert!(damage > 0, "{name}: {hit:?}");
            // The health the HUD wrote for this hit. `health[]` shows hits
            // that follow each other closely as one change, with the
            // health the last one left.
            let written: Vec<(u32, i32)> = (round.health.iter())
                .filter(|h| h.username == hit.username)
                .filter(|h| (h.recording_time.unwrap() - at).abs() <= SAME_HIT)
                .map(|h| (h.health, h.change))
                .collect();
            let close = |other: &replay_analyzer::combat::Hit| {
                other.username == hit.username
                    && (other.recording_time.unwrap() - at).abs() <= SAME_HIT
            };
            let merged = combat.hits[i + 1..].iter().any(close);
            if !written.is_empty() && !merged {
                compared += 1;
                let same = written.iter().find(|w| w.0 == hit.health);
                assert!(same.is_some(), "{name}: {hit:?}, the HUD wrote {written:?}");
                // A hit with no other near it lost what the HUD says.
                if !combat.hits[..i].iter().any(close) {
                    amounts += 1;
                    assert_eq!(same.map(|w| -w.1), Some(damage as i32), "{name}: {hit:?}");
                }
            }
        }
    }
    assert_eq!(hits, 243);
    assert!(compared > 100, "{compared} hits compared with the HUD");
    assert!(amounts > 80, "{amounts} amounts compared with the HUD");
    // The other way around: every loss of health the HUD shows is a hit
    // that left that health.
    let mut losses = 0;
    for (name, round) in test_rounds() {
        let combat = combat(name, round);
        for h in round.health.iter().filter(|h| h.change < 0) {
            losses += 1;
            let at = h.recording_time.unwrap();
            let hit = combat.hits.iter().any(|hit| {
                hit.username == h.username
                    && hit.health == h.health
                    && (hit.recording_time.unwrap() - at).abs() <= SAME_HIT
            });
            assert!(hit, "{name}: no hit for {h:?}");
        }
    }
    assert!(losses > 150, "{losses} losses of health in the HUD");
    assert!(
        from_timeline > 50,
        "{from_timeline} hits named by the timeline"
    );
}

fn match_replay_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("R6_MATCH_REPLAY")?);
    dir.is_dir().then_some(dir)
}

/// `right` of `of`, with the share.
fn share(right: usize, of: usize) -> String {
    let percent = 100.0 * right as f64 / of.max(1) as f64;
    format!("{right} of {of} ({percent:.1}%)")
}

/// How the attacker of the hits was found, and how often the shot and aim
/// rules name the attacker the timeline names for the same hit.
#[derive(Default)]
struct Measured {
    hits: usize,
    timeline: usize,
    shot: usize,
    aim: usize,
    unnamed: usize,
    /// Bullet hits the timeline names the attacker of: `(right, of)` for
    /// the rule that gave an estimate, and how many got none.
    shot_checked: (usize, usize),
    aim_checked: (usize, usize),
    no_estimate: usize,
}

impl Measured {
    fn add(&mut self, combat: &Combat) {
        for hit in &combat.hits {
            self.hits += 1;
            match hit.attacker_source {
                Some(AttackerSource::Timeline) => self.timeline += 1,
                Some(AttackerSource::Shot) => self.shot += 1,
                Some(AttackerSource::Aim) => self.aim += 1,
                None => self.unnamed += 1,
            }
            if hit.attacker_source != Some(AttackerSource::Timeline) || hit.kind.id != 0 {
                continue;
            }
            let Some((by, rule)) = &hit.estimate else {
                self.no_estimate += 1;
                continue;
            };
            let checked = match rule {
                AttackerSource::Shot => &mut self.shot_checked,
                _ => &mut self.aim_checked,
            };
            checked.1 += 1;
            checked.0 += usize::from(Some(by) == hit.by.as_ref());
        }
    }

    fn print(&self, what: &str) {
        let (s, a) = (self.shot_checked, self.aim_checked);
        println!(
            "{what}: {} hits; attacker from the timeline {}, a shot {}, aim {}, none {}",
            self.hits,
            share(self.timeline, self.hits),
            share(self.shot, self.hits),
            share(self.aim, self.hits),
            share(self.unnamed, self.hits),
        );
        println!(
            "{what}: bullet hits the timeline names the attacker of: the shot rule is right for {}, the aim rule for {}, both together for {}; {} have no estimate",
            share(s.0, s.1),
            share(a.0, a.1),
            share(s.0 + a.0, s.1 + a.1 + self.no_estimate),
            self.no_estimate
        );
    }
}

/// Prints how the attacker rules do on the test rounds
/// (`cargo test --test combat -- --nocapture`).
#[test]
fn attacker_rules_measured_on_the_test_rounds() {
    let mut m = Measured::default();
    for (name, round) in test_rounds() {
        m.add(combat(name, round));
    }
    m.print("test rounds");
    assert!(m.shot + m.aim > 0);
}

/// Real rounds: the timeline parses to its last byte in every finished
/// round, and a revive is given by a player of the revived player's team.
#[test]
fn real_timelines_parse_and_revives_stay_in_the_team() {
    let Some(root) = match_replay_dir() else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    let mut m = Measured::default();
    let (mut rounds, mut finished, mut unparsed, mut kinds) = (0, 0, 0, [0; 5]);
    for dir in replay_analyzer::matches::find_match_folders(&root).unwrap() {
        let folder = replay_analyzer::Match::open_with(&dir, ReadMode::Full).unwrap();
        for round in &folder.rounds {
            // Names the round without naming its players.
            let name = format!(
                "{} R{}",
                round.header.match_id,
                round.header.round_number + 1
            );
            let Some(combat) = &round.combat else {
                continue;
            };
            rounds += 1;
            m.add(combat);
            let whole = round.container.as_ref().is_some_and(|c| c.complete);
            let timeline: Vec<&String> = (combat.warnings.iter())
                .filter(|w| w.contains("timeline"))
                .collect();
            if whole {
                finished += 1;
                assert!(timeline.is_empty(), "{name}: {timeline:?}");
            } else if !timeline.is_empty() {
                unparsed += 1;
            }
            let team = |username: &str| {
                let p = round.header.players.iter().find(|p| p.username == username);
                p.map(|p| p.team_index)
            };
            for e in &combat.events {
                kinds[e.kind as usize] += 1;
                assert!(team(&e.username).is_some(), "{name}: {:?}", e.kind);
                if e.kind == TimelineKind::Revive {
                    let by = e.by.as_deref();
                    assert!(by.is_some(), "{name}: a revive by nobody");
                    assert_eq!(
                        by.and_then(team),
                        team(&e.username),
                        "{name}: a revive across teams"
                    );
                }
            }
            for hit in &combat.hits {
                assert!(team(&hit.username).is_some(), "{name}: a hit on nobody");
                assert!(hit.direction <= 7, "{name}: {}", hit.direction);
                if hit.attacker_source == Some(AttackerSource::Timeline) {
                    assert_ne!(hit.result, HitResult::Alive, "{name}");
                }
            }
        }
    }
    println!(
        "real rounds: {rounds} with combat, {finished} finished (every timeline parses to its last byte), {unparsed} unfinished with a timeline that does not"
    );
    println!(
        "real rounds: timeline kills {}, team kills {}, deaths {}, downs {}, revives {}",
        kinds[0], kinds[1], kinds[2], kinds[3], kinds[4]
    );
    m.print("real rounds");
}

/// What must hold once the timeline is joined to the life events, the kill
/// feed and the stats.
fn check_join(name: &str, round: &Round) {
    use replay_analyzer::{DownOutcome, LifeEventType};
    for l in &round.life_events {
        let who = &l.username;
        match l.kind {
            LifeEventType::Down => {
                let outcome = l
                    .outcome
                    .unwrap_or_else(|| panic!("{name}: {who}'s down has no outcome"));
                assert_eq!(
                    outcome == DownOutcome::Finished,
                    l.finished_by.is_some(),
                    "{name}: {who}'s down names a finisher exactly when it was finished"
                );
                if let Some(by) = &l.by {
                    assert!(is_player(round, by), "{name}: {by} downed {who}");
                    assert_ne!(by, who, "{name}: {who} downed themselves");
                }
            }
            LifeEventType::Revive => {
                assert!(l.outcome.is_none() && l.finished_by.is_none());
                assert_eq!(l.self_revive, l.by.as_deref() == Some(who.as_str()));
            }
        }
    }
    for kill in &round.match_feedback {
        if kill.kind != MatchUpdateType::Kill {
            assert!(!kill.finish && !kill.team_kill && kill.downed_by.is_empty());
            continue;
        }
        if !kill.downed_by.is_empty() {
            assert!(
                kill.finish,
                "{name}: a kill names a downer but is no finish"
            );
            // The scoreboard credits the kill to whoever downed the victim.
            if !kill.credited_to.is_empty() {
                assert_eq!(kill.credited_to, kill.downed_by, "{name}: {}", kill.target);
            }
        }
        let team = |username: &str| {
            let player = round.header.players.iter().find(|p| p.username == username);
            player.map(|p| p.team_index)
        };
        assert_eq!(
            kill.team_kill,
            team(&kill.username) == team(&kill.target),
            "{name}: {} killing {}",
            kill.username,
            kill.target
        );
    }
    let stats = round.player_stats();
    let dealt: u32 = stats
        .iter()
        .map(|s| s.damage_dealt.unwrap_or(0) + s.team_damage)
        .sum();
    let taken: u32 = stats.iter().map(|s| s.damage_taken).sum();
    assert!(dealt <= taken, "{name}: {dealt} dealt, {taken} taken");
    let downs_dealt: u32 = stats.iter().map(|s| s.downs_dealt).sum();
    let downs: u32 = stats.iter().map(|s| s.downs).sum();
    assert!(
        downs_dealt <= downs,
        "{name}: {downs_dealt} downs dealt, {downs} suffered"
    );
}

#[test]
fn downs_kills_and_stats_are_joined_to_the_timeline() {
    use replay_analyzer::{DownOutcome, LifeEventType};
    let (mut downs, mut named, mut finished) = (0, 0, 0);
    for (name, round) in test_rounds() {
        check_join(name, round);
        for l in round
            .life_events
            .iter()
            .filter(|l| l.kind == LifeEventType::Down)
        {
            downs += 1;
            named += usize::from(l.by.is_some());
            finished += usize::from(l.outcome == Some(DownOutcome::Finished));
        }
        // The one revive of the match names who gave it.
        for l in round
            .life_events
            .iter()
            .filter(|l| l.kind == LifeEventType::Revive)
        {
            assert!(
                l.by.is_some(),
                "{name}: {} was revived by nobody",
                l.username
            );
        }
    }
    // Every down of the match was dealt by a named player; a kill that ends
    // the round is no down.
    assert_eq!((downs, named, finished), (18, 18, 16));
}

#[test]
fn real_rounds_join_the_timeline() {
    let Some(dir) = match_replay_dir() else {
        return;
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for folder in std::fs::read_dir(dir).unwrap().flatten() {
        let Ok(rounds) = std::fs::read_dir(folder.path()) else {
            continue;
        };
        files.extend(
            rounds
                .flatten()
                .map(|f| f.path())
                .filter(|p| p.extension().is_some_and(|e| e == "rec")),
        );
    }
    for file in files {
        let Ok(round) = Round::open(&file, ReadMode::Full) else {
            continue;
        };
        if round.combat.is_some() {
            check_join(&file.display().to_string(), &round);
        }
    }
}
