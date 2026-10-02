//! The operator catalog against what replays hold: every loadout must be
//! one the catalog allows. Data as in `tests/replays.rs`; with
//! `R6_MATCH_REPLAY` set, the same over a real `MatchReplay` folder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use replay_analyzer::catalog::operators::{
    Observer, OperatorInfo, Source, catalog, catalog_for, observe,
};
use replay_analyzer::loadout::Counted;
use replay_analyzer::{ReadMode, Round, TeamRole};

fn test_rounds() -> Vec<Round> {
    let root = match std::env::var_os("R6_TEST_DATA") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings"),
    };
    let dir = root.join("valid/Y11S3");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("skipping: no test replays in {dir:?}");
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rec"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|p| Round::open(p, ReadMode::Full).unwrap())
        .collect()
}

/// What a round's loadouts say against the catalog.
#[derive(Default)]
struct Findings {
    loadouts: usize,
    /// The catalog rules it out.
    wrong: Vec<String>,
    /// An item the catalog does not list for the operator. The lists are
    /// what was seen plus the operator pages, so a new pick can be missing.
    unlisted: Vec<String>,
    /// Per operator: `players[].maxHealth` that agrees, and that does not.
    health: BTreeMap<String, (usize, Vec<u32>)>,
    /// A 125 attacker read as 100: a rise of 25 while the attacker picks
    /// is taken for a Rook plate and subtracted again.
    plate_misreads: usize,
}

/// A count the HUD gave against the most the catalog allows.
fn count(f: &mut Findings, who: &str, item: &Counted, most: Option<u32>) {
    let (Some(counts), Some(most)) = (&item.counts, most) else {
        return;
    };
    if counts.start > most || counts.max.is_some_and(|m| m > most) {
        f.wrong.push(format!(
            "{who}: {:?} starts with {} of {:?}, the catalog has {most}",
            item.name, counts.start, counts.max
        ));
    }
}

fn gadget(f: &mut Findings, who: &str, info: &OperatorInfo, item: &Counted) {
    let Some(id) = item.id else { return };
    match info.gadget(id) {
        Some(known) => count(f, who, item, known.count),
        None => f
            .unlisted
            .push(format!("{who}: gadget {id} {:?}", item.name)),
    }
}

fn check(f: &mut Findings, label: &str, round: &Round) {
    let catalog = catalog_for(round).catalog;
    for l in &round.loadouts {
        // An operator an attacker swapped away from has no slots.
        if l.primary.is_none() && l.ability.is_none() {
            continue;
        }
        let who = format!("{label}: {} ({})", l.username, l.operator);
        let Some(info) = catalog.operator(l.operator) else {
            // Recruits have no side and no entry.
            if l.operator.role().is_some() {
                f.wrong.push(format!("{who}: not in the catalog"));
            }
            continue;
        };
        f.loadouts += 1;

        if let Some(id) = l.primary.as_ref().and_then(|w| w.id)
            && info.primary(id).is_none()
        {
            f.unlisted.push(format!("{who}: primary {id}"));
        }
        if let Some(id) = l.secondary.as_ref().and_then(|w| w.id)
            && info.secondary(id).is_none()
        {
            f.unlisted.push(format!("{who}: secondary {id}"));
        }
        if let Some(item) = &l.gadget {
            gadget(f, &who, info, item);
        }
        match (&l.ability, info.ability) {
            // Striker and Sentry: a second gadget.
            (Some(item), None) => gadget(f, &who, info, item),
            (Some(item), Some(ability)) => {
                if item.id != ability.id {
                    f.wrong.push(format!("{who}: ability {:?}", item.id));
                } else if !ability.regenerates {
                    count(f, &who, item, ability.max.or(ability.count));
                }
            }
            (None, _) => {}
        }

        let player = round
            .header
            .players
            .iter()
            .find(|p| p.username == l.username && p.operator == l.operator);
        let Some(player) = player else { continue };
        let side = round.header.teams[player.team_index].role;
        if side.is_some_and(|s| s != info.side) {
            f.wrong.push(format!("{who}: played on {side:?}"));
        }
        if let (Some(seen), Some(known)) = (player.max_health, info.max_health) {
            let entry = f.health.entry(l.operator.to_string()).or_default();
            if seen == known {
                entry.0 += 1;
            } else if (info.side, known, seen) == (TeamRole::Attack, 125, 100) {
                f.plate_misreads += 1;
            } else {
                entry.1.push(seen);
            }
        }
    }
}

#[test]
fn test_round_loadouts_agree_with_the_catalog() {
    let rounds = test_rounds();
    if rounds.is_empty() {
        return;
    }
    let mut f = Findings::default();
    for (i, round) in rounds.iter().enumerate() {
        assert!(
            !catalog_for(round).fallback,
            "round {i} has no season catalog"
        );
        check(&mut f, &format!("round {i}"), round);
    }
    eprintln!(
        "{} rounds, {} loadouts, {} plate misreads",
        rounds.len(),
        f.loadouts,
        f.plate_misreads
    );
    assert_eq!(f.loadouts, rounds.len() * 10);
    assert!(f.wrong.is_empty(), "{}", f.wrong.join("\n"));
    assert!(f.unlisted.is_empty(), "{}", f.unlisted.join("\n"));
    for (operator, (_, other)) in &f.health {
        assert!(other.is_empty(), "{operator}: max health {other:?}");
    }
}

/// Everything the test rounds show is in the catalog as observed, in at
/// least as many loadouts: the tables are a harvest that includes them.
#[test]
fn the_catalog_holds_what_the_test_rounds_show() {
    let rounds = test_rounds();
    let Some(catalog) = catalog("Y11S3") else {
        panic!("no Y11S3 catalog");
    };
    for seen in observe(&rounds) {
        let name = seen.operator;
        let info = catalog.operator(seen.operator).unwrap();
        assert_eq!(info.side_evidence.source, Source::Observed, "{name}");
        assert!(info.side_evidence.rounds >= seen.rounds, "{name}");
        assert_eq!(seen.sides.len(), 1, "{name}");
        assert_eq!(seen.sides[0].value, info.side, "{name}");
        let slots = [
            (&seen.gadgets, info.gadgets),
            (&seen.primaries, info.primaries),
            (&seen.secondaries, info.secondaries),
        ];
        for (items, known) in slots {
            for item in items {
                let entry = known.iter().find(|k| k.id == Some(item.id));
                let entry = entry.unwrap_or_else(|| panic!("{name}: {:?}", item.name));
                assert_eq!(entry.evidence.source, Source::Observed, "{name}");
                assert_eq!(Some(entry.name), item.name, "{name}");
            }
        }
        match info.ability {
            Some(ability) => {
                assert_eq!(seen.abilities.len(), 1, "{name}");
                assert_eq!(ability.id, Some(seen.abilities[0].id), "{name}");
                assert!(
                    ability.evidence.rounds >= seen.abilities[0].rounds,
                    "{name}"
                );
            }
            None => {
                for item in &seen.abilities {
                    assert!(info.gadget(item.id).is_some(), "{name}: {:?}", item.name);
                }
            }
        }
    }
}

/// With `R6_MATCH_REPLAY` set: no loadout in that folder is one the
/// catalog rules out, and each operator's usual maximum health is the
/// catalog's. Picks the catalog does not list yet are printed, not failed:
/// the folder grows as matches are played.
#[test]
fn real_loadouts_agree_with_the_catalog() {
    let Some(root) = std::env::var_os("R6_MATCH_REPLAY").map(PathBuf::from) else {
        eprintln!("skipping: R6_MATCH_REPLAY is not set");
        return;
    };
    let mut f = Findings::default();
    let mut observer = Observer::new();
    let (mut rounds, mut fallback) = (0, 0);
    for dir in replay_analyzer::matches::find_match_folders(&root).unwrap() {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "rec"))
            .collect();
        files.sort();
        for file in files {
            let Ok(round) = Round::open(&file, ReadMode::Full) else {
                continue;
            };
            rounds += 1;
            // A season without a catalog of its own proves nothing.
            if catalog_for(&round).fallback {
                fallback += 1;
                continue;
            }
            let label = file.file_name().unwrap_or_default().to_string_lossy();
            check(&mut f, &label, &round);
            observer.add(&round);
        }
    }
    let operators = observer.finish();
    let odd: usize = f.health.values().map(|h| h.1.len()).sum();
    eprintln!(
        "{rounds} rounds ({fallback} of a season without a catalog), {} loadouts, {} operators, \
         {} plate misreads, {odd} other maximum health mismatches, {} unlisted items",
        f.loadouts,
        operators.len(),
        f.plate_misreads,
        f.unlisted.len()
    );
    for line in &f.unlisted {
        eprintln!("unlisted: {line}");
    }
    assert!(f.wrong.is_empty(), "{}", f.wrong.join("\n"));
    for (operator, (same, other)) in &f.health {
        assert!(*same > other.len(), "{operator}: max health {other:?}");
    }
}
