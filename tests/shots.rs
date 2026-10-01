//! Shots and bullet hits of the Y11S3 test rounds, checked against what the
//! rest of a round says: the players, the HUD's ammunition and the kill
//! feed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use replay_analyzer::{ReadMode, Round};
use serde_json::Value;

fn data_dir() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = match std::env::var_os("R6_TEST_DATA") {
        Some(dir) => PathBuf::from(dir),
        None => root.join("test_recordings"),
    };
    let found = dir.join("valid").join("Y11S3").is_dir();
    if !found {
        eprintln!("skipping: no Y11S3 test replays in {}", dir.display());
    }
    found.then_some(dir)
}

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

/// The ten rounds of the custom test match as JSON, read once.
fn rounds() -> Option<&'static [(String, Value)]> {
    static ROUNDS: std::sync::OnceLock<Option<Vec<(String, Value)>>> = std::sync::OnceLock::new();
    let rounds = ROUNDS.get_or_init(|| {
        let dir = data_dir()?.join("valid").join("Y11S3");
        let rounds: Vec<(String, Value)> = replays(&dir)
            .into_iter()
            .filter(|p| {
                let name = p.file_name().unwrap().to_string_lossy();
                name.starts_with("custom_")
            })
            .map(|p| {
                let round = Round::open(&p, ReadMode::Full).unwrap();
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                (name, serde_json::to_value(&round).unwrap())
            })
            .collect();
        assert!(!rounds.is_empty(), "no custom_*.rec in {}", dir.display());
        Some(rounds)
    });
    rounds.as_deref()
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v[key].as_array().map_or(&[], Vec::as_slice)
}

fn vec3(v: &Value) -> [f64; 3] {
    let a = v.as_array().expect("a vector");
    assert_eq!(a.len(), 3);
    [0, 1, 2].map(|i| a[i].as_f64().expect("a number"))
}

/// What must hold for the shots and hits of any round.
fn check_invariants(name: &str, round: &Value) {
    let players: Vec<&str> = list(round, "players")
        .iter()
        .filter_map(|p| p["username"].as_str())
        .collect();
    let duration = round["timing"]["duration"].as_f64();
    let within = |e: &Value, what: &str| {
        if let Some(t) = e["recordingTime"].as_f64() {
            let end = duration.expect("a recording time without a duration");
            assert!(
                t >= 0.0 && t <= end + 0.001,
                "{name}: {what} at {t} of {end}"
            );
        }
        assert!(
            e["time"].is_string() && e["phase"].is_string(),
            "{name}: {e}"
        );
    };
    let shots = list(round, "shots");
    for s in shots {
        if let Some(user) = s["username"].as_str() {
            assert!(players.contains(&user), "{name}: shooter {user}");
        }
        let d = vec3(&s["direction"]);
        let length = d.iter().map(|v| v * v).sum::<f64>().sqrt();
        assert!((length - 1.0).abs() < 0.01, "{name}: direction {d:?}");
        assert!(vec3(&s["origin"]).iter().all(|v| v.abs() < 1e4));
        let (muzzle, eye) = (s["distance"].as_f64(), s["eyeDistance"].as_f64());
        assert!(muzzle.is_some() && eye.is_some(), "{name}: {s}");
        if let Some(slot) = s["slot"].as_str() {
            assert!(["primary", "secondary", "ability", "gadget"].contains(&slot));
        }
        within(s, "shot");
    }
    // Shots are in the order they were fired.
    let times: Vec<f64> = shots
        .iter()
        .filter_map(|s| s["recordingTime"].as_f64())
        .collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]), "{name}: shot order");

    for h in list(round, "bulletHits") {
        let victim = h["victim"].as_str().expect("a victim");
        assert!(players.contains(&victim), "{name}: victim {victim}");
        if !h["position"].is_null() {
            assert!(vec3(&h["position"]).iter().all(|v| v.abs() < 1e4));
        }
        within(h, "hit");
        // Damage, limb and result come from one block: all or none, but for
        // a pellet sharing its block, which has no damage of its own.
        assert_eq!(h["limb"].is_null(), h["result"].is_null(), "{name}: {h}");
        assert!(
            h["damage"].is_null() || h["limb"].is_boolean(),
            "{name}: {h}"
        );
        if let Some(result) = h["result"].as_str() {
            assert!(["alive", "down", "dead"].contains(&result), "{name}: {h}");
        }
        assert!(h["damage"].as_u64().is_none_or(|d| d <= 400), "{name}: {h}");
        assert!(h.get("headshot").is_none(), "the file has no headshot");
        // A shooter is the shooter of the shot named, found by ray, and is
        // never the victim.
        match h["shooter"].as_str() {
            Some(shooter) => {
                assert!(players.contains(&shooter), "{name}: shooter {shooter}");
                assert_ne!(shooter, victim, "{name}: {h}");
                assert_eq!(h["shooterSource"], "ray");
                let shot = &shots[h["shot"].as_u64().expect("a shot") as usize];
                assert_eq!(shot["username"], h["shooter"], "{name}: {h}");
                let (a, b) = (shot["recordingTime"].as_f64(), h["recordingTime"].as_f64());
                let apart = (a.unwrap() - b.unwrap()).abs();
                assert!(apart < 0.5, "{name}: shot and hit {apart} s apart");
            }
            None => assert!(h["shooterSource"].is_null(), "{name}: {h}"),
        }
    }
}

#[test]
fn shots_and_hits_name_players_and_hold_together() {
    let Some(rounds) = rounds() else { return };
    for (name, round) in rounds {
        check_invariants(name, round);
        assert!(list(round, "shots").len() > 100, "{name}: few shots");
        assert!(!list(round, "bulletHits").is_empty(), "{name}: no hits");
        // In these rounds every gun that fired is linked to its player and
        // to a slot of their loadout.
        for s in list(round, "shots") {
            assert!(
                s["username"].is_string() && s["slot"].is_string(),
                "{name}: {s}"
            );
            assert!(s["weapon"]["id"].is_u64(), "{name}: {s}");
        }
        let status = list(&round["decodeStatus"], "fields")
            .iter()
            .find(|f| f["field"] == "shots")
            .unwrap_or_else(|| panic!("{name}: no decode status for shots"));
        assert_eq!(status["count"], list(round, "shots").len(), "{name}");
        assert!(status["warnings"].is_null(), "{name}: {status}");
    }
}

/// `(username, slot)` -> shots fired with the gun in that slot.
fn shots_per_gun(round: &Value) -> HashMap<(String, String), i64> {
    let mut out = HashMap::new();
    for s in list(round, "shots") {
        let slot = s["slot"].as_str().unwrap_or_default();
        if slot == "primary" || slot == "secondary" {
            let user = s["username"].as_str().unwrap_or_default();
            *out.entry((user.to_string(), slot.to_string())).or_default() += 1;
        }
    }
    out
}

#[test]
fn shots_agree_with_the_ammunition_the_hud_counts_down() {
    let Some(rounds) = rounds() else { return };
    let (mut all_shots, mut all_drops, mut guns, mut exact) = (0, 0, 0, 0);
    for (name, round) in rounds {
        let shots = shots_per_gun(round);
        // Rounds of each gun the HUD counted down: the loadout's total, and
        // the bursts of the weapon activity added up.
        let mut drops: HashMap<(String, String), i64> = HashMap::new();
        let mut launchers: Vec<(String, String)> = Vec::new();
        for l in list(round, "loadouts") {
            for slot in ["primary", "secondary"] {
                let ammo = &l[slot]["ammo"];
                if let Some(fired) = ammo["fired"].as_i64() {
                    let gun = (
                        l["username"].as_str().unwrap().to_string(),
                        slot.to_string(),
                    );
                    if ammo["start"].as_i64() < ammo["magazineSize"].as_i64() {
                        launchers.push(gun.clone());
                    }
                    *drops.entry(gun).or_default() += fired;
                }
            }
        }
        let mut bursts: HashMap<(String, String), i64> = HashMap::new();
        for a in list(round, "weaponActivity") {
            for f in list(a, "fired") {
                let user = a["username"].as_str().unwrap().to_string();
                let slot = f["slot"].as_str().unwrap().to_string();
                *bursts.entry((user, slot)).or_default() += f["rounds"].as_i64().unwrap();
            }
        }
        // The counter of a gun with a launcher under it (Nomad's, Kali's)
        // is the launcher's: it starts below a magazine, and says nothing
        // about the gun.
        let (mut total, mut counted, mut off) = (0, 0, 0);
        for (gun, &fired) in drops.iter().filter(|d| !launchers.contains(d.0)) {
            let shot = shots.get(gun).copied().unwrap_or(0);
            total += shot;
            counted += fired;
            off += (shot - fired).abs();
            guns += i64::from(shot + fired > 0);
            exact += i64::from(shot + fired > 0 && shot == fired);
            assert!(
                (shot - fired).abs() <= 3.max(fired / 10),
                "{name}: {gun:?} shot {shot} times, the HUD counted {fired}"
            );
            // The bursts miss rounds the loadout's total has, never the
            // other way around.
            let burst = bursts.get(gun).copied().unwrap_or(0);
            assert!(
                burst <= fired && (shot - burst).abs() <= 3.max(shot / 10),
                "{name}: {gun:?}"
            );
        }
        assert!(total > 100, "{name}: {total} shots");
        assert!(off * 50 <= total, "{name}: {off} off over {total} shots");
        // Every gun that shot is a gun of a loadout.
        for gun in shots.keys() {
            assert!(drops.contains_key(gun), "{name}: {gun:?} has no ammunition");
        }
        all_shots += total;
        all_drops += counted;
    }
    eprintln!("{all_shots} shots, {all_drops} ammunition drops, {exact} of {guns} guns exact");
    assert!((all_shots - all_drops).abs() * 200 <= all_drops);
    assert!(exact * 10 >= guns * 7, "{exact} of {guns} guns exact");
}

/// `[kills, lethal hit by the killer, by another, no hit found]` of a round's
/// kill feed.
fn kill_attribution(round: &Value) -> [usize; 4] {
    let mut out = [0; 4];
    let hits = list(round, "bulletHits");
    for k in list(round, "matchFeedback") {
        let Some(at) = k["recordingTime"]
            .as_f64()
            .filter(|_| k["type"]["name"] == "Kill")
        else {
            continue;
        };
        out[0] += 1;
        // The victim's attributed hits around the kill: the one that left
        // them dead, or else the nearest.
        let apart = |h: &Value| (h["recordingTime"].as_f64().unwrap_or(f64::MAX) - at).abs();
        let mut near: Vec<&Value> = hits
            .iter()
            .filter(|h| h["victim"] == k["target"] && h["shooter"].is_string())
            .filter(|h| apart(h) <= 0.6)
            .collect();
        near.sort_by(|a, b| apart(a).total_cmp(&apart(b)));
        let lethal = near.iter().find(|h| h["result"] == "dead").or(near.first());
        match lethal {
            Some(h) if h["shooter"] == k["username"] => out[1] += 1,
            Some(_) => out[2] += 1,
            None => out[3] += 1,
        }
    }
    out
}

#[test]
fn the_lethal_hit_of_a_kill_is_the_killers() {
    let Some(rounds) = rounds() else { return };
    let mut sum = [0; 4];
    for (_, round) in rounds {
        let found = kill_attribution(round);
        sum = std::array::from_fn(|i| sum[i] + found[i]);
    }
    let [kills, right, wrong, unseen] = sum;
    eprintln!("{kills} kills: {right} to the killer, {wrong} to another, {unseen} without a hit");
    assert!(kills >= 50);
    assert!(right * 100 >= kills * 85, "{right} of {kills}");
    assert!(wrong * 20 <= kills, "{wrong} of {kills}");
}

#[test]
fn hits_take_the_health_the_state_stream_shows() {
    let Some(rounds) = rounds() else { return };
    let (mut with_block, mut attributed, mut damaging, mut same) = (0, 0, 0, 0);
    for (name, round) in rounds {
        let health = list(round, "health");
        let lost: i64 = health
            .iter()
            .filter_map(|h| h["change"].as_i64())
            .filter(|&c| c < 0)
            .map(|c| -c)
            .sum();
        let mut dealt = 0;
        for h in list(round, "bulletHits") {
            with_block += i64::from(h["result"].is_string());
            attributed += i64::from(h["result"].is_string() && h["shooter"].is_string());
            let Some(damage) = h["damage"].as_i64() else {
                continue;
            };
            dealt += damage;
            damaging += 1;
            let at = h["recordingTime"].as_f64().unwrap();
            same += i64::from(health.iter().any(|u| {
                u["username"] == h["victim"]
                    && u["change"].as_i64() == Some(-damage)
                    && (u["recordingTime"].as_f64().unwrap_or(f64::MAX) - at).abs() < 0.15
            }));
        }
        // Bullets take most of the health lost, and no more than was lost
        // but for a last update before a kill the state stream skipped.
        assert!(dealt * 10 >= lost * 6, "{name}: {dealt} of {lost}");
        assert!(dealt * 10 <= lost * 12, "{name}: {dealt} of {lost}");
    }
    eprintln!(
        "{with_block} hits with a damage block, {attributed} with a shooter; \
         {same} of {damaging} damages equal a health change"
    );
    assert!(attributed * 100 >= with_block * 95);
    assert!(same * 10 >= damaging * 7);
}

/// Real recordings, when `R6_MATCH_REPLAY` names the game's folder. It
/// changes as matches are played, so only what must hold for any round is
/// checked.
#[test]
fn real_rounds_hold_together() {
    let Some(root) = std::env::var_os("R6_MATCH_REPLAY").map(PathBuf::from) else {
        eprintln!("skipping: R6_MATCH_REPLAY not set");
        return;
    };
    if !root.is_dir() {
        eprintln!("skipping: {} is no folder", root.display());
        return;
    }
    let (mut read, mut shots, mut hits, mut unlinked) = (0, 0, 0, 0);
    let (mut with_block, mut attributed, mut kills) = (0, 0, [0; 4]);
    for path in replays(&root) {
        // Unfinished files and older versions have no shots to check.
        let Ok(round) = Round::open(&path, ReadMode::Full) else {
            continue;
        };
        let value = serde_json::to_value(&round).unwrap();
        check_invariants(&path.display().to_string(), &value);
        read += 1;
        shots += round.shots.len();
        hits += round.bullet_hits.len();
        unlinked += round.shots.iter().filter(|s| s.username.is_none()).count();
        let hit = |h: &&replay_analyzer::shots::Hit| h.result.is_some();
        with_block += round.bullet_hits.iter().filter(hit).count();
        let by = |h: &&replay_analyzer::shots::Hit| h.result.is_some() && h.shooter.is_some();
        attributed += round.bullet_hits.iter().filter(by).count();
        let found = kill_attribution(&value);
        kills = std::array::from_fn(|i| kills[i] + found[i]);
    }
    eprintln!("{with_block} hits with a damage block, {attributed} with a shooter");
    let [all, right, wrong, unseen] = kills;
    eprintln!("{all} kills: {right} to the killer, {wrong} to another, {unseen} without a hit");
    eprintln!("{read} rounds: {shots} shots ({unlinked} without a shooter), {hits} hits");
}
