//! What happens to gadgets and what gadgets do to players (Y11S3): score
//! changes, gadget removals, statuses and trap triggers of the ten test
//! rounds, and invariants over a real `MatchReplay` folder
//! (`R6_MATCH_REPLAY`, only read).

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
    if !dir.join("valid").join("Y11S3").is_dir() {
        eprintln!("skipping: no Y11S3 test replays in {}", dir.display());
        return None;
    }
    Some(dir)
}

/// Every `.rec` under `dir`, sorted.
fn replays(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
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

/// The ten test rounds as JSON, by number.
fn test_rounds() -> Option<Vec<(usize, Value)>> {
    let dir = data_dir()?.join("valid").join("Y11S3");
    let mut out = Vec::new();
    for n in 1..=10 {
        let path = dir.join(format!("custom_{n}.rec"));
        let round = Round::open(&path, ReadMode::Full).unwrap();
        out.push((n, serde_json::to_value(&round).unwrap()));
    }
    Some(out)
}

fn list<'a>(round: &'a Value, key: &str) -> &'a [Value] {
    round[key].as_array().map_or(&[], Vec::as_slice)
}

/// Username -> team index.
fn teams(round: &Value) -> HashMap<String, u64> {
    list(round, "players")
        .iter()
        .filter_map(|p| Some((p["username"].as_str()?.to_owned(), p["teamIndex"].as_u64()?)))
        .collect()
}

fn seconds(v: &Value) -> Option<f64> {
    v["recordingTime"].as_f64()
}

/// `event` with the entity, name and owner of the gadget it is on.
fn of_gadget(gadget: &Value, event: &Value) -> Value {
    let mut out = event.clone();
    for key in ["entity", "name", "username"] {
        if !gadget[key].is_null() {
            out[key] = gadget[key].clone();
        }
    }
    out
}

/// Every removal of a round with a cause or a scorer: the `end` of each
/// gadget that has one, and `deviceRemovals`. A gadget's `end` leaves the
/// team that held it out when it is its owner's.
fn removals(round: &Value) -> Vec<Value> {
    let teams = teams(round);
    let mut out: Vec<Value> = Vec::new();
    for g in list(round, "gadgets") {
        let end = &g["end"];
        if end["cause"].is_null() && end["by"].is_null() {
            continue;
        }
        let mut r = of_gadget(g, end);
        let owners = g["username"].as_str().and_then(|u| teams.get(u));
        if let (true, Some(team)) = (r["ownerTeam"].is_null(), owners) {
            r["ownerTeam"] = (*team).into();
        }
        out.push(r);
    }
    out.extend(list(round, "deviceRemovals").iter().cloned());
    out
}

/// Every status of a round: those on a gadget of `gadgets`, and
/// `gadgetStatuses`.
fn statuses(round: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for g in list(round, "gadgets") {
        out.extend(list(g, "statuses").iter().map(|s| of_gadget(g, s)));
    }
    out.extend(list(round, "gadgetStatuses").iter().cloned());
    out
}

/// Every trap trigger of a round: those of a gadget of `gadgets`, and
/// `trapTriggers`.
fn triggers(round: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for g in list(round, "gadgets") {
        for t in list(g, "triggers") {
            let mut t = of_gadget(g, t);
            t["trap"] = g["name"].clone();
            out.push(t);
        }
    }
    out.extend(list(round, "trapTriggers").iter().cloned());
    out
}

/// What must hold for the gadget events of any round. Returns the count
/// of destroyed drones and cameras, and how many of them name a scorer.
fn check(label: &str, round: &Value) -> (usize, usize) {
    let teams = teams(round);
    let player = |v: &Value| v.as_str().and_then(|u| teams.get(u).copied());

    // Every score change names a player, and a player's totals follow
    // from their deltas.
    let mut total: HashMap<&str, i64> = HashMap::new();
    for s in list(round, "scoreChanges") {
        let name = s["username"].as_str().unwrap_or_default();
        assert!(teams.contains_key(name), "{label}: {s}");
        let (delta, now) = (s["delta"].as_i64().unwrap(), s["total"].as_i64().unwrap());
        assert_ne!(delta, 0, "{label}: {s}");
        if let Some(before) = total.insert(name, now) {
            assert_eq!(before + delta, now, "{label}: {s}");
        }
        assert_eq!(
            s["reason"].is_null(),
            s["reasonSource"].is_null(),
            "{label}: {s}"
        );
    }
    // The last total is the score the scoreboard ends on.
    for p in list(round, "stats") {
        let name = p["username"].as_str().unwrap_or_default();
        if let (Some(&last), Some(score)) = (total.get(name), p["score"].as_i64()) {
            assert_eq!(last, score, "{label}: {name}");
        }
    }

    let (mut tools, mut scored) = (0, 0);
    for r in &removals(round) {
        assert!(r["entity"].is_string(), "{label}: {r}");
        // A cause says where it is from.
        assert_eq!(
            r["cause"].is_null(),
            r["causeSource"].is_null(),
            "{label}: {r}"
        );
        assert!(seconds(r).is_some(), "{label}: {r}");
        if !r["by"].is_null() {
            let by = player(&r["by"]).unwrap_or_else(|| panic!("{label}: {r}"));
            assert!(!r["bySource"].is_null(), "{label}: {r}");
            if r["bySource"] == "score" {
                // The score pairs by the team that held the gadget: the
                // alliance its owner component states, else the owner's.
                // A drone need not name its owner.
                let held = r["ownerTeam"].as_u64();
                assert!(held.is_some(), "{label}: {r}");
                if r["friendly"].as_bool().unwrap_or(false) {
                    assert_eq!(Some(by), held, "{label}: {r}");
                    assert_ne!(r["by"], r["username"], "{label}: {r}");
                } else {
                    assert_ne!(Some(by), held, "{label}: {r}");
                }
            }
        }
        if !r["means"].is_null() {
            assert!(!r["meansSource"].is_null(), "{label}: {r}");
        }
        let signals: Vec<&str> = list(r, "signals")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(!signals.is_empty(), "{label}: {r}");
        if signals.contains(&"destroyedFlag") {
            assert_eq!(r["cause"], "destroyed", "{label}: {r}");
            tools += 1;
            scored += usize::from(!r["by"].is_null());
        }
    }

    for s in &statuses(round) {
        assert!(s["entity"].is_string(), "{label}: {s}");
        let from = seconds(s).unwrap_or_else(|| panic!("{label}: {s}"));
        if let Some(until) = seconds(&s["until"]) {
            assert!(until >= from, "{label}: {s}");
        }
        if !s["by"].is_null() {
            assert!(player(&s["by"]).is_some(), "{label}: {s}");
            assert_eq!(s["bySource"], "score", "{label}: {s}");
        }
    }

    for t in &triggers(round) {
        assert!(t["trap"].is_string(), "{label}: {t}");
        assert!(seconds(t).is_some(), "{label}: {t}");
        if !t["username"].is_null() {
            assert!(player(&t["username"]).is_some(), "{label}: {t}");
        }
        for v in list(t, "victims") {
            assert!(player(&v["username"]).is_some(), "{label}: {t}");
            assert_eq!(v["victimSource"], "time", "{label}: {t}");
            // A trap does not go off on its owner's team.
            if let Some(owner) = player(&t["username"]) {
                assert_ne!(player(&v["username"]), Some(owner), "{label}: {t}");
            }
        }
    }

    // The join: what is about a gadget of `gadgets` is on that gadget,
    // and only what has no gadget is listed by itself.
    assert!(round["gadgetRemovals"].is_null(), "{label}");
    let gadgets: Vec<&str> = (list(round, "gadgets").iter())
        .filter_map(|g| g["entity"].as_str())
        .collect();
    let listed = |v: &Value| v["entity"].as_str().is_some_and(|e| gadgets.contains(&e));
    for r in list(round, "deviceRemovals") {
        assert!(!r["cause"].is_null(), "{label}: {r}");
        if listed(r) {
            // The entity was another thing then: no gadget had started,
            // or its gadget was gone.
            let at = seconds(r).unwrap_or_default();
            let lives = list(round, "gadgets")
                .iter()
                .filter(|g| g["entity"] == r["entity"]);
            for g in lives {
                let start = seconds(&g["placing"]).or(seconds(&g["released"]));
                let gone = seconds(&g["end"]).zip(g["end"]["goneAfter"].as_f64());
                let before = start.is_some_and(|s| at < s);
                let after = gone.is_some_and(|(end, after)| at > end + after);
                assert!(before || after, "{label}: {r} belongs to {g}");
            }
        }
    }
    for g in list(round, "gadgets") {
        let end = &g["end"];
        // What the scoreboard or the type says of an end is inferred.
        if matches!(
            end["causeSource"].as_str(),
            Some("score" | "type" | "adsFired")
        ) {
            let refined = matches!(end["how"].as_str(), Some("destroyed" | "wentOff"));
            assert!(refined, "{label}: {g}");
        }
        if end["how"] == "pickedUp" {
            assert_eq!(end["source"], "inferred", "{label}: {g}");
        }
        if !g["hostKind"].is_null() {
            assert!(g["host"].is_string(), "{label}: {g}");
        }
    }
    (tools, scored)
}

fn status<'a>(round: &'a Value, field: &str) -> Option<&'a Value> {
    let fields = round["decodeStatus"]["fields"].as_array()?;
    fields.iter().find(|f| f["field"] == field)
}

#[test]
fn test_rounds_hold_the_invariants() {
    let Some(rounds) = test_rounds() else { return };
    for (n, round) in &rounds {
        let label = format!("custom_{n}");
        let s = status(round, "gadgetEvents").unwrap_or_else(|| panic!("{label}: no status"));
        assert_eq!(s["status"], "decoded", "{label}: {s}");
        assert!(!list(round, "scoreChanges").is_empty(), "{label}");
        assert!(!removals(round).is_empty(), "{label}");
        check(&label, round);
    }
}

/// The counts the reference implementation has for the ten rounds
/// (`SP\B\out\1.json` .. `10.json`): score changes, removals with a
/// scorer, statuses and trap triggers.
#[test]
fn test_rounds_match_the_reference() {
    let Some(rounds) = test_rounds() else { return };
    let expected = [
        (114, 21, 1, 4),
        (100, 15, 2, 0),
        (80, 13, 0, 5),
        (110, 13, 0, 3),
        (99, 13, 0, 1),
        (87, 6, 0, 0),
        (126, 12, 14, 0),
        (102, 9, 0, 0),
        (91, 10, 0, 3),
        (106, 10, 9, 2),
    ];
    for ((n, round), want) in rounds.iter().zip(expected) {
        let by = |r: &&Value| !r["by"].is_null();
        let got = (
            list(round, "scoreChanges").len(),
            removals(round).iter().filter(by).count(),
            statuses(round).len(),
            triggers(round).len(),
        );
        assert_eq!(got, want, "custom_{n}");
    }
}

/// Round 1: Fenrir's mine goes off on an attacker, a drone is shot by a
/// player the scoreboard pays for it, and Wamai's Mag-NET catches a hive.
#[test]
fn round_one_names_who_and_how() {
    let Some(rounds) = test_rounds() else { return };
    let round = &rounds[0].1;
    // A drone is no gadget of `gadgets`: its removal is listed by itself.
    let drone = list(round, "deviceRemovals")
        .iter()
        .find(|r| r["entity"] == "f02b8bcc")
        .expect("the drone's removal");
    assert_eq!(drone["cause"], "destroyed");
    assert_eq!(drone["username"], "soulz1.FaZe");
    assert_eq!(drone["by"], "Neskin.L5");
    assert_eq!(drone["bySource"], "score");
    assert_eq!(drone["means"], "bullet");
    assert_eq!(drone["meansSource"], "shotRay");
    assert_eq!(drone["weapon"], "UZK50GI");
    assert_eq!(drone["recordingTime"], 17.014);

    // The mine and the Mag-NET are gadgets: the trigger and the status
    // are on them.
    assert!(list(round, "trapTriggers").is_empty());
    assert!(list(round, "gadgetStatuses").is_empty());
    let triggers = triggers(round);
    let mine = triggers
        .iter()
        .find(|t| t["trap"] == "F-NATT Dread Mine")
        .expect("the mine's trigger");
    assert_eq!(mine["entity"], "f02b848d");
    assert_eq!(mine["username"], "WIZARD.L5");
    assert_eq!(mine["recordingTime"], 176.259);
    assert_eq!(mine["victims"][0]["username"], "soulz1.FaZe");
    assert_eq!(mine["victims"][0]["effect"], 26);

    let caught = &statuses(round)[0];
    assert_eq!(caught["kind"], "caught");
    assert_eq!(caught["entity"], "f02ba4d1");
    assert_eq!(caught["by"], "pino.L5");
    assert_eq!(caught["until"]["recordingTime"], 167.914);

    let first = &list(round, "scoreChanges")[0];
    assert_eq!(first["username"], "WIZARD.L5");
    assert_eq!(first["delta"], 1);
    assert_eq!(first["reason"], "deployed");
    assert_eq!(first["reasonSource"], "coincidence");
}

/// With `R6_MATCH_REPLAY` set: the invariants on every Y11S3 round of
/// the folder; destroyed drones and cameras mostly name a scorer; and a
/// capture needs a Mozzie or a Brava.
#[test]
fn real_rounds_hold_the_invariants() {
    let Some(dir) = std::env::var_os("R6_MATCH_REPLAY") else {
        eprintln!("skipping: R6_MATCH_REPLAY is not set");
        return;
    };
    let (mut tools, mut scored, mut read) = (0, 0, 0);
    for path in replays(Path::new(&dir)) {
        let Ok(round) = Round::open(&path, ReadMode::Full) else {
            continue;
        };
        let label = path.file_name().unwrap_or_default().to_string_lossy();
        let round = serde_json::to_value(&round).unwrap();
        if status(&round, "gadgetEvents").is_none() {
            continue;
        }
        read += 1;
        let (t, s) = check(&label, &round);
        tools += t;
        scored += s;
        let captured = |s: &Value| s["kind"] == "captured";
        if statuses(&round).iter().any(captured) {
            let operators = list(&round, "players").iter().any(|p| {
                let name = p["operator"]["name"].as_str();
                name == Some("Mozzie") || name == Some("Brava")
            });
            assert!(operators, "{label}: a capture without Mozzie or Brava");
        }
    }
    eprintln!("{read} rounds: {scored} of {tools} destroyed drones and cameras name a scorer");
    if tools > 0 {
        assert!(scored * 10 >= tools * 9, "{scored} of {tools}");
    }
}

/// A jammed player names the owner of the Signal Disruptor nearest to
/// them, a Mute, and a barbed wire hit the owner of the nearest wire, an
/// opponent: both say they are the nearest, not read.
#[test]
fn jams_and_wire_hits_name_the_nearest_gadgets_owner() {
    let Some(rounds) = test_rounds() else { return };
    let (mut jams, mut wires) = ((0, 0), (0, 0));
    for (n, round) in &rounds {
        let teams = teams(round);
        let operator = |username: &Value| {
            let mut players = list(round, "players").iter();
            let player = players.find(|p| &p["username"] == username);
            player.and_then(|p| p["operator"]["name"].as_str())
        };
        for e in list(round, "effects") {
            if e["type"] != 8 {
                assert!(e["jammer"].is_null(), "custom_{n}: {e}");
                continue;
            }
            jams.0 += 1;
            if e["jammer"].is_null() {
                continue;
            }
            jams.1 += 1;
            assert_eq!(e["jammerSource"], "nearest", "custom_{n}: {e}");
            assert_eq!(operator(&e["jammer"]), Some("Mute"), "custom_{n}: {e}");
        }
        for h in list(round, "hits") {
            if h["type"]["id"] != 12 {
                assert!(h["gadgetOwner"].is_null(), "custom_{n}: {h}");
                continue;
            }
            wires.0 += 1;
            let Some(owner) = h["gadgetOwner"].as_str() else {
                continue;
            };
            wires.1 += 1;
            assert_eq!(h["gadgetOwnerSource"], "nearest", "custom_{n}: {h}");
            let victim = h["username"].as_str().unwrap_or_default();
            assert_ne!(teams.get(owner), teams.get(victim), "custom_{n}: {h}");
            // The wire is one of that player's gadgets.
            let wire = |g: &Value| g["name"] == "Barbed Wire" && g["username"] == owner;
            assert!(list(round, "gadgets").iter().any(wire), "custom_{n}: {h}");
        }
    }
    assert_eq!((jams, wires), ((5, 5), (14, 12)));
}

/// What the stats count per player adds up to what the round lists.
#[test]
fn stats_count_gadgets_panels_and_breaches() {
    let Some(rounds) = test_rounds() else { return };
    // Completed reinforcements per round, as `tests/panels.rs` has them.
    let reinforced = [8, 10, 9, 9, 9, 10, 9, 8, 10, 9];
    for ((n, round), reinforced) in rounds.iter().zip(reinforced) {
        let label = format!("custom_{n}");
        let sum = |key: &str| -> u64 {
            let stats = list(round, "stats").iter();
            stats.map(|s| s[key].as_u64().unwrap_or(0)).sum()
        };
        let of = |username: &str, key: &str| {
            let mut stats = list(round, "stats").iter();
            let player = stats.find(|s| s["username"] == username);
            player.and_then(|s| s[key].as_u64()).unwrap_or(0)
        };
        assert_eq!(sum("reinforcements"), reinforced, "{label}");
        let placed = |key: &str| {
            let done = |p: &&Value| p["completed"].is_object() && p["username"].is_string();
            list(round, key).iter().filter(done).count() as u64
        };
        assert_eq!(sum("barricades"), placed("barricades"), "{label}");
        let breaches = list(round, "breaches");
        assert_eq!(sum("breaches"), breaches.len() as u64, "{label}");
        let opened = |b: &&Value| b["openedReinforcement"] == true;
        let opened = breaches.iter().filter(opened).count();
        assert_eq!(sum("breachesOpened"), opened as u64, "{label}");
        assert!(sum("gadgetsDeployed") > 0, "{label}");

        // Everything someone is named for destroying counts for them,
        // unless it was their own team's.
        let removals = removals(round);
        let by_enemy = |r: &&Value| r["by"].is_string() && r["friendly"] != true;
        let destroyed = removals.iter().filter(by_enemy).count();
        assert_eq!(sum("gadgetsDestroyed"), destroyed as u64, "{label}");
        // What a player lost is what of theirs was destroyed, by anyone:
        // the map's own devices are nobody's loss.
        let owned = |v: &&Value| v["username"].is_string();
        let gone = |g: &&Value| g["end"]["how"] == "destroyed";
        let gadgets = list(round, "gadgets").iter().filter(owned).filter(gone);
        let broken = |r: &&Value| matches!(r["cause"].as_str(), Some("destroyed" | "intercepted"));
        let devices = list(round, "deviceRemovals")
            .iter()
            .filter(owned)
            .filter(broken);
        let lost = gadgets.count() + devices.count();
        assert_eq!(sum("gadgetsLost"), lost as u64, "{label}");
        assert!(lost > 0, "{label}");
        let triggered = triggers(round).len();
        assert_eq!(sum("trapsTriggered"), triggered as u64, "{label}");
        // Reinforcements are a pool of ten a team shares: a player can
        // put up more than two, a team no more than ten.
        assert!(sum("reinforcements") <= 10, "{label}");
        if *n == 1 {
            // Neskin shot the drone of `round_one_names_who_and_how`, and
            // the mine of that test is the only trap of WIZARD that went
            // off.
            assert!(of("Neskin.L5", "gadgetsDestroyed") >= 1);
            assert!(of("soulz1.FaZe", "gadgetsLost") >= 1);
            assert_eq!(of("WIZARD.L5", "trapsTriggered"), 1);
        }
    }
}
