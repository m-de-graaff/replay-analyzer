//! The weapon catalog held against replays: every gun of the Y11S3 test
//! rounds, and of a real `MatchReplay` folder when `R6_MATCH_REPLAY` names
//! one, must fit its entry.
//!
//! - The magazine size is read from the file, so it must be the catalog's.
//! - An automatic gun's measured fire rate may not be above the catalog's
//!   by more than the measurement's precision.
//! - The damage most hits did may not be above the catalog's.

use std::path::{Path, PathBuf};

use replay_analyzer::catalog::weapons::{self, Harvester, Observed, Source};
use replay_analyzer::{ReadMode, Round};

/// A fire rate is the median of at least three bursts, each good to one
/// update in its length (about 2.5% for 20 rounds at 800 a minute).
const RPM_TOLERANCE: f64 = 0.03;

fn replays(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "rec") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn test_rounds() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = match std::env::var_os("R6_TEST_DATA") {
        Some(dir) => PathBuf::from(dir),
        None => root.join("test_recordings"),
    };
    let dir = dir.join("valid").join("Y11S3");
    if !dir.is_dir() {
        eprintln!("skipping: no Y11S3 test replays in {}", dir.display());
        return None;
    }
    Some(dir)
}

fn real_folder() -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("R6_MATCH_REPLAY").map(PathBuf::from) else {
        eprintln!("skipping: R6_MATCH_REPLAY is not set");
        return None;
    };
    if !dir.is_dir() {
        eprintln!("skipping: {} is not a folder", dir.display());
        return None;
    }
    Some(dir)
}

/// Adds every round under `dir`. Files the game had not finished writing
/// and rounds without loadouts (another season) are passed over.
fn add(harvester: &mut Harvester, dir: &Path) {
    for path in replays(dir) {
        match Round::open(&path, ReadMode::Full) {
            Ok(round) if !round.loadouts.is_empty() => harvester.add(&round),
            Ok(_) => {}
            Err(e) => eprintln!("passing over {}: {e}", path.display()),
        }
    }
}

/// What the rounds under `dir` show of each gun.
fn harvest(dir: &Path) -> (u32, Vec<Observed>) {
    let mut harvester = Harvester::default();
    add(&mut harvester, dir);
    (harvester.rounds(), harvester.finish())
}

/// Checks every observed gun against the catalog and returns how many
/// magazines, fire rates and damages were compared.
fn check(observed: &[Observed]) -> [usize; 3] {
    let mut problems = Vec::new();
    let mut compared = [0; 3];
    for gun in observed {
        let Some(info) = weapons::weapon_info(gun.id) else {
            problems.push(format!("{:?} ({}) has no entry", gun.name, gun.id));
            continue;
        };
        assert_eq!(gun.name, Some(info.name));
        problems.extend(gun.disagreements(info, RPM_TOLERANCE));
        let both = |seen: bool, listed: bool| usize::from(seen && listed);
        compared[0] += both(gun.magazine_size().is_some(), info.magazine.value.is_some());
        compared[1] += both(
            gun.fire.rpm.is_some(),
            info.rpm.value.is_some() && info.is_automatic(),
        );
        compared[2] += both(gun.damage.value.is_some(), info.damage.value.is_some());
        // A gun has one magazine size.
        if gun.magazine.len() > 1 {
            problems.push(format!("{}: magazines {:?}", info.name, gun.magazine));
        }
        // One burst is good to a tenth at worst (the 16 rounds of an
        // SMG-11 are gone in 0.7 s); the median of several is far better.
        if let Some(precision) = gun.fire.precision {
            assert!(precision < 0.1, "{}: {precision}", info.name);
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    compared
}

#[test]
fn guns_of_the_test_rounds_fit_the_catalog() {
    let Some(dir) = test_rounds() else { return };
    let (rounds, observed) = harvest(&dir);
    assert_eq!(rounds, 10);
    let [magazines, rates, damages] = check(&observed);
    eprintln!(
        "{rounds} rounds, {} guns: {magazines} magazines, {rates} fire rates, {damages} damages compared",
        observed.len()
    );
    assert!(observed.len() >= 55, "{} guns", observed.len());
    assert!(magazines >= 55 && rates >= 8 && damages >= 3);

    // The MP7 of the test rounds: 30 rounds and one in the chamber.
    let mp7 = observed.iter().find(|g| g.name == Some("MP7")).unwrap();
    assert_eq!(mp7.magazine_size(), Some(30));
    assert_eq!(mp7.chamber(), Some(true));
    assert!(mp7.attachments.len() >= 4);
}

#[test]
fn guns_of_a_real_folder_fit_the_catalog() {
    let Some(dir) = real_folder() else { return };
    let (rounds, observed) = harvest(&dir);
    let [magazines, rates, damages] = check(&observed);
    eprintln!(
        "{rounds} rounds, {} guns: {magazines} magazines, {rates} fire rates, {damages} damages compared",
        observed.len()
    );
    assert!(rounds > 0 && !observed.is_empty());
}

/// Armor takes nothing off a bullet: a gun does its usual damage as often
/// to targets of 125 health as to targets of 100. With armor that cut
/// damage, a heavier target would hardly ever take the full figure.
///
/// Not every hit does: about one in eight does half or seven tenths of it
/// at any range, which nothing in the hit explains, and fall-off takes
/// more beyond it. So the check is on shares, not on every hit.
#[test]
fn armor_does_not_reduce_damage() {
    let Some(dir) = real_folder().or_else(test_rounds) else {
        return;
    };
    let (_, observed) = harvest(&dir);
    // Per target health: hits, and hits that did the gun's damage.
    let mut by_health = [(100, 0, 0), (110, 0, 0), (125, 0, 0)];
    let (mut pairs, mut agree) = (0, 0);
    for gun in observed.iter().filter(|g| g.damage.value.is_some()) {
        for health in &gun.damage.by_health {
            let total = by_health
                .iter_mut()
                .find(|h| h.0 == health.max_health)
                .unwrap();
            total.1 += health.hits;
            total.2 += health.full;
            if health.hits >= 5 {
                pairs += 1;
                agree += usize::from(Some(health.value) == gun.damage.value);
            }
        }
    }
    eprintln!("{agree} of {pairs} pairs of a gun and a target health take the gun's damage most");
    for (health, hits, full) in by_health {
        eprintln!("{health} health: {full} of {hits} hits did the gun's damage");
        if hits >= 20 {
            assert!(full * 3 >= hits * 2, "{health} health: {full} of {hits}");
        }
    }
    assert!(agree * 10 >= pairs * 9, "{agree} of {pairs}");
    assert_eq!(weapons::armor_multiplier(125), 1.0);
}

/// Entries whose observed and reference values differ keep both, and go by
/// what was observed.
#[test]
fn disagreements_keep_both_sides() {
    let mut disagreements = 0;
    for weapon in weapons::all() {
        for stat in [&weapon.damage, &weapon.rpm, &weapon.magazine] {
            if stat.disagrees() && stat.source == Source::Observed {
                disagreements += 1;
                assert_eq!(stat.value, stat.observed, "{}", weapon.name);
                assert!(stat.reference.is_some(), "{}", weapon.name);
            }
        }
    }
    eprintln!("{disagreements} values where replays and the reference differ");
}

/// The observed side of the catalog as JSON, for rebuilding the table:
/// `CATALOG_DUMP=out.json cargo test --release --test catalog_weapons dump -- --ignored`.
/// Reads the test rounds and, when set, the `R6_MATCH_REPLAY` folder.
#[test]
#[ignore = "writes the file CATALOG_DUMP names"]
fn dump_observed() {
    let Some(out) = std::env::var_os("CATALOG_DUMP") else {
        eprintln!("skipping: CATALOG_DUMP is not set");
        return;
    };
    let mut harvester = Harvester::default();
    if let Some(dir) = test_rounds() {
        add(&mut harvester, &dir);
    }
    if let Some(dir) = real_folder() {
        add(&mut harvester, &dir);
    }
    let rounds = harvester.rounds();
    let json = serde_json::json!({ "rounds": rounds, "guns": harvester.finish() });
    std::fs::write(out, serde_json::to_string_pretty(&json).unwrap()).unwrap();
}
