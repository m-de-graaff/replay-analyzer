//! Map data harvested from the Y11S3 test replays (ten rounds on Bank),
//! and what holds of any harvest. Data is read from `R6_TEST_DATA`, else
//! `test_recordings/`; the tests are skipped when neither has replays.
//! With `R6_MATCH_REPLAY` set, the same is checked on every map of that
//! folder; it is only read.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rayon::prelude::*;
use replay_analyzer::mapdata::{
    self, Floor, MapData, MapObjectKind, Observed, Opening, Room, Source, Wall, WallKind,
};
use replay_analyzer::panels::{Opening as PanelOpening, ReinforcementKind};
use replay_analyzer::{ReadMode, Round};

/// Where the sample of the test rounds' harvest is kept.
const SAMPLE: &str = "docs/outside/413779563590-BankY10.json";

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

/// The folder of the Y11S3 test replays.
fn test_dir() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = match std::env::var_os("R6_TEST_DATA") {
        Some(dir) => PathBuf::from(dir),
        None => root.join("test_recordings"),
    };
    dir.join("valid").join("Y11S3")
}

/// The ten test rounds with their map objects, read once.
fn test_rounds() -> Option<&'static [Observed]> {
    static ROUNDS: OnceLock<Vec<Observed>> = OnceLock::new();
    let rounds = ROUNDS.get_or_init(|| {
        let dir = test_dir();
        if !dir.is_dir() {
            eprintln!("skipping: no Y11S3 test replays in {}", dir.display());
            return Vec::new();
        }
        (replays(&dir).par_iter())
            .map(|p| mapdata::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
            .collect()
    });
    (!rounds.is_empty()).then_some(rounds.as_slice())
}

/// The rounds of `R6_MATCH_REPLAY` that read, by map.
fn real_rounds() -> Option<BTreeMap<u64, Vec<Observed>>> {
    let dir = PathBuf::from(std::env::var_os("R6_MATCH_REPLAY")?);
    // Unfinished recordings and older versions do not read.
    let rounds: Vec<Observed> = (replays(&dir).par_iter())
        .filter_map(|path| mapdata::read(path).ok())
        .collect();
    let mut maps: BTreeMap<u64, Vec<Observed>> = BTreeMap::new();
    for round in rounds {
        (maps.entry(round.round.header.map.0).or_default()).push(round);
    }
    Some(maps)
}

fn floors(data: &MapData) -> Vec<(Option<&str>, f32)> {
    (data.floors.iter())
        .map(|f| (f.name.as_deref(), f.z))
        .collect()
}

/// What holds of every harvest: each reinforcement and barricade of the
/// rounds lies on a wall, hatch, door or window of the map data; every
/// element is on a floor that exists; no object is two walls.
fn check(data: &MapData, rounds: &[Observed]) -> Result<(), String> {
    let map = &data.map.name;
    for (i, floor) in data.floors.iter().enumerate() {
        let below = i.checked_sub(1).map(|b| &data.floors[b]);
        let rising = below.is_none_or(|b| b.z < floor.z);
        if floor.index != i as i32 || !rising {
            return Err(format!("{map}: floors out of order at {i}"));
        }
    }
    let on_floor = |f: Option<i32>| f.is_none_or(|f| (f as usize) < data.floors.len());
    let placed = (data.walls.iter().map(|w| w.floor))
        .chain(data.doors.iter().map(|d| d.floor))
        .chain(data.windows.iter().map(|w| w.floor))
        .chain(data.hatches.iter().map(|h| h.floor))
        .chain(data.sites.iter().map(|s| s.floor))
        .chain(data.rooms.iter().map(|r| Some(r.floor)));
    if !placed.into_iter().all(on_floor) {
        return Err(format!("{map}: an element on a floor that is not there"));
    }
    // What stands on a floor says which: a reinforced wall, a door, a
    // hatch, a bomb.
    let hard = (data.walls.iter()).filter(|w| w.kind == WallKind::Reinforceable);
    let hatches = data.hatches.iter().filter(|h| h.reinforceable);
    let standing = (hard.map(|w| w.floor))
        .chain(data.doors.iter().map(|d| d.floor))
        .chain(hatches.map(|h| h.floor))
        .chain(data.sites.iter().map(|s| s.floor));
    if standing.into_iter().any(|f| f.is_none()) {
        return Err(format!("{map}: something stands on no floor"));
    }
    let mut ids = BTreeSet::new();
    for wall in &data.walls {
        if !wall.object_id.iter().all(|id| ids.insert(*id)) {
            return Err(format!("{map}: an object that is two walls"));
        }
        if wall.top <= wall.bottom {
            return Err(format!("{map}: a wall with no height"));
        }
        // A part names the wall it is a part of, and the other way round.
        let whole = wall.part_of.and_then(|id| data.wall(id));
        let id = wall.object_id.unwrap_or(0);
        let listed = whole.is_some_and(|w| w.parts.contains(&id));
        let mut parts = wall.parts.iter().map(|id| data.wall(*id));
        let named = parts.all(|p| p.is_some_and(|p| p.part_of == wall.object_id));
        if wall.part_of.is_some() != listed || !named {
            return Err(format!("{map}: a part of no wall"));
        }
    }
    for o in rounds {
        let round = &o.round;
        let at = format!("{map} round {}", round.header.round_number);
        let done = round.reinforcements.iter();
        for r in done.filter(|r| r.completed.is_some() && !r.cancelled) {
            let [x, y, z] = r.position;
            match r.kind {
                ReinforcementKind::Wall => {
                    let wall = match r.host {
                        Some(host) => data.wall(host),
                        None => data.nearest_wall(x, y, z).map(|w| w.0),
                    };
                    let Some(wall) = wall else {
                        return Err(format!("{at}: no wall for {:x?}", r.host));
                    };
                    // 0.1 m off its middle plane, within its width.
                    let off = wall.distance(x, y, z);
                    if wall.kind != WallKind::Reinforceable || off > 0.15 {
                        return Err(format!("{at}: reinforcement {off} m off its wall"));
                    }
                }
                ReinforcementKind::Hatch => {
                    let Some((hatch, off)) = data.nearest_hatch(x, y, z) else {
                        return Err(format!("{at}: no hatch"));
                    };
                    let named = r.host.is_none() || hatch.object_id == r.host;
                    if !named || off > 0.1 || !hatch.reinforceable {
                        return Err(format!("{at}: reinforcement {off} m off its hatch"));
                    }
                }
            }
        }
        for b in round.barricades.iter().filter(|b| !b.cancelled) {
            let [x, y, z] = b.position;
            let found = match b.opening {
                Some(PanelOpening::Door) => data.nearest_door(x, y, z),
                Some(PanelOpening::Window) => data.nearest_window(x, y, z),
                None => continue,
            };
            // 0.125 m off the frame's plane, at the top of the opening.
            let Some((opening, _)) = found.filter(|f| f.1 <= 0.2) else {
                return Err(format!("{at}: barricade at {:?} on no opening", b.position));
            };
            let frame = b.host.is_none() || opening.object_id == b.host;
            if !frame || (opening.top - z).abs() > 0.01 {
                return Err(format!("{at}: barricade on another opening"));
            }
        }
    }
    Ok(())
}

/// Bank has a basement, a first and a second floor. The names are read
/// off the bomb sites played on each.
#[test]
fn the_test_rounds_give_banks_three_floors() {
    let Some(rounds) = test_rounds() else {
        return;
    };
    let data = mapdata::harvest_with(rounds);
    let map = &data.map;
    assert_eq!((map.name.as_str(), map.id), ("BankY10", 413779563590));
    assert_eq!(map.base.as_deref(), Some("Bank"));
    assert_eq!(map.version.as_deref(), Some("Y10"));
    assert_eq!(
        floors(&data),
        [(Some("B"), -3.8), (Some("1F"), 0.0), (Some("2F"), 4.0)]
    );
    assert!(data.floors.iter().all(|f| f.source == Source::Derived));
    assert_eq!(data.floors[0].ceiling, Some(0.0));
    assert_eq!(data.rounds.len(), 10);
    // Where bodies are: nobody stands under the basement, and the roof is
    // no floor.
    let floor = |z: f32| data.floor_at(z).map(|f| f.index);
    assert_eq!(
        (floor(-3.8), floor(-0.3), floor(1.2)),
        (Some(0), Some(1), Some(1))
    );
    assert_eq!((floor(4.0), floor(8.5), floor(-9.0)), (Some(2), None, None));
}

/// Rounds alone, read without the movement, give the same floors and the
/// same reinforced walls, doors, windows and cameras; the map objects add
/// the bombs not in play and the walls nobody reinforced.
#[test]
fn rounds_without_their_map_objects_give_less() {
    let Some(rounds) = test_rounds() else {
        return;
    };
    let full = mapdata::harvest_with(rounds);
    let plain: Vec<Round> = (replays(&test_dir()).iter())
        .map(|p| Round::open(p, ReadMode::Full).unwrap())
        .collect();
    let data = mapdata::harvest(&plain);
    assert_eq!(floors(&data), floors(&full));
    assert_eq!((&data.doors, &data.windows), (&full.doors, &full.windows));
    assert_eq!((&data.cameras, &data.spawns), (&full.cameras, &full.spawns));
    assert_eq!(data.rooms, full.rooms);
    let hard = |d: &MapData| -> Vec<Option<u64>> {
        let walls = d.walls.iter().filter(|w| w.kind == WallKind::Reinforceable);
        walls.map(|w| w.object_id).collect()
    };
    assert_eq!(hard(&data), hard(&full));
    assert!(data.walls.iter().all(|w| w.kind == WallKind::Reinforceable));
    assert!(data.walkable.is_empty() && data.pieces.is_empty());
    // Three sites were played in the ten rounds, of Bank's four.
    assert_eq!((data.sites.len(), full.sites.len()), (6, 8));
    let named = |d: &MapData| d.sites.iter().filter(|s| s.name.is_some()).count();
    assert_eq!((named(&data), named(&full)), (6, 6));
    assert!(full.walls.iter().any(|w| w.kind == WallKind::Soft));
    assert_eq!(full.walkable.len(), 3);
}

/// What the ten rounds hold of Bank.
#[test]
fn the_test_rounds_give_banks_sites_spawns_and_cameras() {
    let Some(rounds) = test_rounds() else {
        return;
    };
    let data = mapdata::harvest_with(rounds);
    let mut sites: Vec<_> = (data.sites.iter())
        .filter_map(|s| Some((s.floor?, s.name.as_deref()?, s.played)))
        .collect();
    sites.sort();
    assert_eq!(
        sites,
        [
            (0, "B CCTV Room", 5),
            (0, "B Lockers", 5),
            (1, "1F Open Area", 2),
            (1, "1F Staff Room", 2),
            (2, "2F CEO Office", 3),
            (2, "2F Executive Lounge", 3),
        ]
    );
    // The two bombs of a site name each other.
    for site in data.sites.iter().filter(|s| s.partner.is_some()) {
        let other = data.sites.iter().find(|o| o.object_id == site.partner);
        assert_eq!(other.and_then(|o| o.partner), site.object_id);
    }
    let spawns: Vec<_> = data.spawns.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(spawns, ["Alley Access", "Jewelry Front", "Parking Front"]);
    assert!(data.spawns.iter().all(|s| s.radius < 5.0 && s.players >= 5));
    assert_eq!(data.cameras.len(), 8);
    assert!(data.cameras.iter().all(|c| c.rounds == 10));
    // Each site played is a room.
    let rooms: Vec<_> = (data.rooms.iter())
        .map(|r| (r.floor, r.name.as_str()))
        .collect();
    assert!(rooms.contains(&(0, "B Lockers")) && rooms.contains(&(2, "2F CEO Office")));
    assert_eq!(rooms.len(), 6);
    let named = |r: &Room| r.polygon.is_empty() && r.anchor.is_some();
    assert!(data.rooms.iter().all(named));
    // No outline, so no room at a point; the nearest anchor is a guess.
    let site = data.sites.iter().find(|s| s.name.is_some()).unwrap();
    let [x, y, z] = site.position;
    assert_eq!(data.room_at(x, y, z), None);
    let near = data.nearest_room(x, y, z).map(|r| r.0.name.as_str());
    assert_eq!(near, site.name.as_deref());
    // Every destructible map object the rounds created is in it once.
    let mut objects = BTreeSet::new();
    for o in rounds.iter().flat_map(|r| &r.objects) {
        if o.kind == MapObjectKind::Destructible {
            objects.insert(o.id);
        }
    }
    let soft = data.walls.iter().filter(|w| w.kind == WallKind::Soft);
    let held: Vec<u64> = (soft.filter_map(|w| w.object_id))
        .chain(data.hatches.iter().filter_map(|h| h.panel_id))
        .chain(data.pieces.iter().filter_map(|p| p.object_id))
        .collect();
    assert_eq!(held.len(), objects.len());
    assert_eq!(held.into_iter().collect::<BTreeSet<_>>(), objects);
}

#[test]
fn every_panel_of_the_test_rounds_lies_on_a_wall_or_an_opening() {
    let Some(rounds) = test_rounds() else {
        return;
    };
    let data = mapdata::harvest_with(rounds);
    check(&data, rounds).unwrap();
    // The counts the check went over.
    let panels = |f: fn(&Round) -> usize| rounds.iter().map(|o| f(&o.round)).sum::<usize>();
    let reinforced = panels(|r| {
        let done = r.reinforcements.iter();
        done.filter(|r| r.completed.is_some() && !r.cancelled)
            .count()
    });
    let barricaded = panels(|r| r.barricades.iter().filter(|b| !b.cancelled).count());
    assert_eq!((reinforced, barricaded), (91, 212));
    // A door is as high as every door: 2.2 m from a height of its floor.
    for door in &data.doors {
        let floor = &data.floors[door.floor.unwrap() as usize];
        let stands = floor.levels.iter().any(|l| (l - door.bottom).abs() < 0.01);
        assert!(stands, "{door:?}");
    }
}

#[test]
fn a_harvest_does_not_depend_on_the_order_of_the_rounds() {
    let Some(rounds) = test_rounds() else {
        return;
    };
    let data = mapdata::harvest_with(rounds);
    let mut other = rounds.to_vec();
    other.reverse();
    assert!(mapdata::harvest_with(&other) == data);
    other.rotate_left(3);
    other.swap(0, 7);
    assert!(mapdata::harvest_with(&other) == data);
    // The same round twice is one round.
    other.extend(rounds[..2].iter().cloned());
    assert!(mapdata::harvest_with(&other) == data);
}

#[test]
fn merging_is_idempotent_and_adds_up_to_the_harvest_of_all() {
    let Some(rounds) = test_rounds() else {
        return;
    };
    let data = mapdata::harvest_with(rounds);
    assert!(mapdata::merge(&data, &data) == data);
    let empty = MapData::default();
    assert!(mapdata::merge(&data, &empty) == data);
    assert!(mapdata::merge(&empty, &data) == data);
    // One file per map, a round at a time.
    let mut stored = MapData::default();
    for round in rounds {
        let one = mapdata::harvest_with(std::slice::from_ref(round));
        stored = mapdata::merge(&stored, &one);
        assert!(mapdata::merge(&stored, &one) == stored);
    }
    assert_eq!(stored.rounds, data.rounds);
    assert_eq!(floors(&stored), floors(&data));
    // What is read is the same either way, rounds counted and all.
    assert_eq!(stored.sites, data.sites);
    assert_eq!(stored.cameras, data.cameras);
    assert_eq!(stored.doors, data.doors);
    assert_eq!(stored.windows, data.windows);
    assert_eq!(stored.hatches, data.hatches);
    assert_eq!(stored.walls, data.walls);
    assert_eq!(stored.pieces, data.pieces);
    assert_eq!(stored.walkable, data.walkable);
    // A median of medians is not the median: spawns and rooms keep their
    // names and counts, their places to a few metres.
    assert_eq!(stored.spawns.len(), data.spawns.len());
    for (a, b) in stored.spawns.iter().zip(&data.spawns) {
        assert_eq!(
            (&a.name, a.rounds, a.players),
            (&b.name, b.rounds, b.players)
        );
        let apart = (a.position[0] - b.position[0]).hypot(a.position[1] - b.position[1]);
        assert!(apart < 5.0, "{}: {apart}", a.name);
    }
    let names = |d: &MapData| d.rooms.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&stored), names(&data));
    check(&stored, rounds).unwrap();
}

/// What a person draws: a floor's name, a room's outline, a wall measured
/// better than the harvest has it, a doorway no barricade ever closed.
fn authored(harvested: &MapData) -> MapData {
    let hard = |w: &&Wall| w.kind == WallKind::Reinforceable && w.floor == Some(0);
    let wall = harvested.walls.iter().find(hard).unwrap();
    let door = &harvested.doors[0];
    let site = harvested.sites.iter().find(|s| s.name.is_some()).unwrap();
    let [x, y, _] = site.position;
    MapData {
        map: harvested.map.clone(),
        floors: vec![Floor {
            name: Some("Basement".to_owned()),
            z: -3.8,
            ceiling: Some(-0.4),
            source: Source::Authored,
            ..Floor::default()
        }],
        rooms: vec![Room {
            name: site.name.clone().unwrap(),
            floor: 0,
            polygon: vec![
                [x - 3.0, y - 3.0],
                [x + 3.0, y - 3.0],
                [x + 3.0, y + 3.0],
                [x - 3.0, y + 3.0],
            ],
            source: Source::Authored,
            ..Room::default()
        }],
        walls: vec![
            Wall {
                top: wall.bottom + 2.75,
                source: Source::Authored,
                assumed: Vec::new(),
                ..wall.clone()
            },
            Wall {
                a: [-200.0, 0.0],
                b: [-200.0, 10.0],
                bottom: -3.8,
                top: -0.8,
                kind: WallKind::Solid,
                width: 10.0,
                source: Source::Authored,
                ..Wall::default()
            },
        ],
        doors: vec![
            Opening {
                width: 1.1,
                source: Source::Authored,
                assumed: Vec::new(),
                ..door.clone()
            },
            Opening {
                a: [-200.0, 20.0],
                b: [-200.0, 21.0],
                normal: [1.0, 0.0],
                bottom: -3.8,
                top: -1.6,
                width: 1.0,
                see_through: true,
                source: Source::Authored,
                ..Opening::default()
            },
        ],
        ..MapData::default()
    }
}

#[test]
fn authored_entries_survive_a_merge() {
    let Some(rounds) = test_rounds() else {
        return;
    };
    let harvested = mapdata::harvest_with(rounds);
    let drawn = authored(&harvested);
    let own = |s: Source| s == Source::Authored;
    let first = mapdata::merge(&drawn, &harvested);
    for merged in [
        first.clone(),
        mapdata::merge(&harvested, &drawn),
        // The file an app keeps: every later harvest merged into it.
        mapdata::merge(&first, &harvested),
        mapdata::merge(&mapdata::harvest_with(&rounds[..3]), &first),
    ] {
        let walls: Vec<&Wall> = merged.walls.iter().filter(|w| own(w.source)).collect();
        assert_eq!(walls.len(), 2);
        assert_eq!(
            (walls[0].kind, walls[0].a),
            (WallKind::Solid, [-200.0, 0.0])
        );
        assert_eq!(walls[1].top, walls[1].bottom + 2.75);
        assert!(walls[1].assumed.is_empty() && walls[1].object_id.is_some());
        let doors = merged.doors.iter().filter(|d| own(d.source));
        assert_eq!(doors.map(|d| d.width).collect::<Vec<_>>(), [1.0, 1.1]);
        let floors: Vec<&Floor> = merged.floors.iter().filter(|f| own(f.source)).collect();
        assert_eq!(
            (floors.len(), floors[0].name.as_deref()),
            (1, Some("Basement"))
        );
        assert_eq!((floors[0].index, floors[0].ceiling), (0, Some(-0.4)));
        // Nothing harvested was lost, and nothing is there twice.
        assert_eq!(merged.walls.len(), harvested.walls.len() + 1);
        assert_eq!(merged.doors.len(), harvested.doors.len() + 1);
        assert_eq!(merged.rooms.len(), harvested.rooms.len());
        assert_eq!(merged.floors.len(), 3);
        assert_eq!(merged.floors[1].name.as_deref(), Some("1F"));
        // The drawn room is found, on the drawn floor.
        let rooms: Vec<&Room> = merged.rooms.iter().filter(|r| own(r.source)).collect();
        let site = harvested.sites.iter().find(|s| s.name.is_some()).unwrap();
        let [x, y, z] = site.position;
        let room = merged.room_at(x + 1.0, y - 1.0, z + 0.3).unwrap();
        assert_eq!((rooms.len(), rooms[0]), (1, room));
        assert_eq!(Some(&room.name), site.name.as_ref());
        assert_eq!(merged.room_at(x + 4.0, y, z), None);
        assert_eq!(merged.room_at(x + 1.0, y - 1.0, z + 4.0), None);
        check(&merged, rounds).unwrap();
        assert!(mapdata::merge(&merged, &merged) == merged);
    }
}

#[test]
fn the_json_reads_back() {
    let Some(rounds) = test_rounds() else {
        return;
    };
    let harvested = mapdata::harvest_with(rounds);
    let drawn = mapdata::merge(&authored(&harvested), &harvested);
    for data in [drawn, harvested] {
        let text = serde_json::to_string(&data).unwrap();
        let back: MapData = serde_json::from_str(&text).unwrap();
        assert!(back == data);
        let pretty = serde_json::to_string_pretty(&data).unwrap();
        assert!(serde_json::from_str::<MapData>(&pretty).unwrap() == data);
        // Ids are hex, as everywhere in the output, and keys camelCase.
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let wall = (data.walls.iter()).position(|w| w.object_id.is_some());
        let id = value["walls"][wall.unwrap()]["objectId"].as_str().unwrap();
        let read = u64::from_str_radix(id, 16).ok();
        assert_eq!(read, data.walls[wall.unwrap()].object_id);
        assert_eq!(value["map"]["id"], 413779563590u64);
        assert_eq!(value["doors"][0]["seeThrough"], true);
        assert!(text.len() < 200_000, "{} bytes", text.len());
    }
    // A file with only what somebody drew reads too.
    let drawn = r#"{"map": {"id": 413779563590},
        "rooms": [{"name": "Lobby", "floor": 1, "polygon": [[0, 0], [4, 0], [4, 4]],
                   "source": "authored"}],
        "walls": [{"a": [0, 0], "b": [4, 0], "bottom": 0, "top": 3, "kind": "solid",
                   "source": "authored"}]}"#;
    let drawn: MapData = serde_json::from_str(drawn).unwrap();
    assert_eq!(drawn.rooms[0].source, Source::Authored);
    assert_eq!(drawn.walls[0].kind, WallKind::Solid);
}

/// The sample next to the documentation is the harvest of the test
/// rounds. To write it again, delete it (the example merges into a file
/// that is there) and run
/// `cargo run --release --example mapdata -- test_recordings/valid/Y11S3 docs/outside`.
#[test]
fn the_sample_is_the_harvest_of_the_test_rounds() {
    let Some(rounds) = test_rounds() else {
        return;
    };
    if std::env::var_os("R6_TEST_DATA").is_some() {
        return;
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(SAMPLE);
    let text = std::fs::read_to_string(&path).unwrap();
    let sample: MapData = serde_json::from_str(&text).unwrap();
    let same = sample == mapdata::harvest_with(rounds);
    assert!(same, "{SAMPLE} is out of date");
}

/// Every map of a real folder: the same invariants, a floor for every
/// site, eight bombs, and a harvest that does not depend on the order.
#[test]
fn real_maps_hold_the_same() {
    let Some(maps) = real_rounds() else {
        eprintln!("skipping: R6_MATCH_REPLAY is not set");
        return;
    };
    let mut failures = Vec::new();
    for (id, rounds) in &maps {
        let data = mapdata::harvest_with(rounds);
        assert_eq!(data.map.id, *id);
        if let Err(e) = check(&data, rounds) {
            failures.push(e);
        }
        let name = &data.map.name;
        let mut expect = |ok: bool, what: &str| {
            if !ok {
                failures.push(format!("{name}: {what}"));
            }
        };
        expect((1..=5).contains(&data.floors.len()), "floors");
        expect(data.sites.len() == 8, "eight bombs");
        expect(data.sites.iter().any(|s| s.name.is_some()), "no site named");
        expect(!data.cameras.is_empty(), "cameras");
        expect(!data.spawns.is_empty(), "spawns");
        expect(!data.walls.is_empty(), "walls");
        expect(!data.windows.is_empty(), "windows");
        // A site's name starts with the name of its floor.
        for site in data.sites.iter().filter(|s| s.name.is_some()) {
            let floor = site.floor.and_then(|f| data.floors.get(f as usize));
            let prefix = floor.and_then(|f| f.name.as_deref()).unwrap_or("?");
            let named = site.name.as_deref().is_some_and(|n| n.starts_with(prefix));
            expect(named, "a site on a floor of another name");
        }
        let mut other = rounds.clone();
        other.reverse();
        expect(mapdata::harvest_with(&other) == data, "order of the rounds");
        expect(mapdata::merge(&data, &data) == data, "merge with itself");
        if rounds.len() > 1 {
            let half = rounds.len() / 2;
            let a = mapdata::harvest_with(&rounds[..half]);
            let b = mapdata::harvest_with(&rounds[half..]);
            let both = mapdata::merge(&a, &b);
            expect(both.walls == data.walls, "walls of two halves");
            expect(both.doors == data.doors, "doors of two halves");
            expect(both.windows == data.windows, "windows of two halves");
            expect(both.hatches == data.hatches, "hatches of two halves");
            expect(both.sites == data.sites, "sites of two halves");
        }
        let text = serde_json::to_string(&data).unwrap();
        let back = serde_json::from_str::<MapData>(&text).unwrap();
        expect(back == data, "JSON");
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
