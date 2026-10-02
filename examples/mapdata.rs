//! Writes one map data file per map of a match folder or a `MatchReplay`
//! folder: `cargo run --release --example mapdata -- <folder> <out dir>`.
//!
//! Every `.rec` under the folder is read with its movement and its map
//! objects. A file the out dir already has for a map is merged with what
//! the folder gives, so the same out dir fills in over many runs, and what
//! was drawn into a file by hand stays.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use replay_analyzer::mapdata::{self, MapData, Observed, Source, WallKind};

/// Every `.rec` under `dir`, sorted.
fn replays(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if d.is_file() {
            out.push(d);
            continue;
        }
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

fn summary(data: &MapData) -> String {
    let walls = |kind| data.walls.iter().filter(|w| w.kind == kind).count();
    let measured = (data.walls.iter())
        .filter(|w| w.kind == WallKind::Soft && w.source == Source::Derived)
        .count();
    let floors: Vec<String> = (data.floors.iter())
        .map(|f| format!("{}@{}", f.name.as_deref().unwrap_or("?"), f.z))
        .collect();
    format!(
        "{:<20} rounds {:>3}  floors {} [{}]  rooms {}  sites {} ({} named)  spawns {}  doors {}  \
         windows {}  hatches {}  walls {} reinforceable + {} soft ({} measured)  cameras {}  \
         pieces {}  walked cells {}",
        data.map.name,
        data.rounds.len(),
        data.floors.len(),
        floors.join(" "),
        data.rooms.len(),
        data.sites.len(),
        data.sites.iter().filter(|s| s.name.is_some()).count(),
        data.spawns.len(),
        data.doors.len(),
        data.windows.len(),
        data.hatches.len(),
        walls(WallKind::Reinforceable),
        walls(WallKind::Soft),
        measured,
        data.cameras.len(),
        data.pieces.len(),
        (data.walkable.iter())
            .flat_map(|w| &w.rows)
            .map(|r| r.bytes().filter(|b| *b == b'#').count())
            .sum::<usize>(),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, out_dir] = args.as_slice() else {
        return Err("usage: mapdata <match folder or MatchReplay folder> <out dir>".into());
    };
    let files = replays(Path::new(input));
    // A round that does not read (an unfinished recording, an older
    // version) is left out.
    let rounds: Vec<Observed> = (files.par_iter())
        .filter_map(|path| match mapdata::read(path) {
            Ok(round) => Some(round),
            Err(e) => {
                eprintln!("skipping {}: {e}", path.display());
                None
            }
        })
        .collect();
    let mut maps: BTreeMap<u64, Vec<Observed>> = BTreeMap::new();
    for round in rounds {
        (maps.entry(round.round.header.map.0).or_default()).push(round);
    }
    std::fs::create_dir_all(out_dir)?;
    for (id, rounds) in &maps {
        let mut data = mapdata::harvest_with(rounds);
        let name: String = (data.map.name.chars())
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        let path = Path::new(out_dir).join(format!("{id}-{name}.json"));
        if let Ok(text) = std::fs::read_to_string(&path) {
            let stored: MapData = serde_json::from_str(&text)?;
            data = mapdata::merge(&stored, &data);
        }
        std::fs::write(&path, serde_json::to_string(&data)?)?;
        println!("{}", summary(&data));
    }
    Ok(())
}
