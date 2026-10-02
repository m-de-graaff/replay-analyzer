//! Research probe: splits a replay into per-stream record files for offline
//! analysis. `cargo run --release --example defuser_probe -- in.rec outdir`
//!
//! Each `<stem>.<streamhash>.bin` holds: n x { f64 time, u32 frame
//! (ffffffff for the snapshot), u32 len, payload }.

use std::io::Write;

use replay_analyzer::records::RecordMap;
use replay_analyzer::{ReadMode, ReadOptions, Round, decompressed_bytes};

fn frame_times(raw: &[u8]) -> Vec<f64> {
    // frame index: u32 n, then n x (u32 i, f64 t) with i = 0, 1, 2...
    let u32_at = |p: usize| u32::from_le_bytes(raw[p..p + 4].try_into().unwrap());
    let limit = raw.len().min(4 << 20);
    for p in 0..limit.saturating_sub(64) {
        let n = u32_at(p) as usize;
        if n < 100 || p + 4 + 12 * n > raw.len() {
            continue;
        }
        if u32_at(p + 4) == 0 && u32_at(p + 16) == 1 && u32_at(p + 28) == 2 && u32_at(p + 40) == 3 {
            return (0..n)
                .map(|i| f64::from_le_bytes(raw[p + 8 + 12 * i..p + 16 + 12 * i].try_into().unwrap()))
                .collect();
        }
    }
    Vec::new()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (path, out) = (&args[1], &args[2]);
    let stem = args.get(3).cloned().unwrap_or_else(|| {
        std::path::Path::new(path).file_stem().unwrap().to_string_lossy().into_owned()
    });
    let raw = std::fs::read(path)?;
    let times = frame_times(&raw);
    let round = Round::from_bytes(&raw, ReadOptions { mode: ReadMode::Full, census: false, movement: true })?;
    let data = decompressed_bytes(&raw)?;
    let container = round.container.as_ref().ok_or("no container")?;
    let map = RecordMap::parse(&data, container.streams.len()).ok_or("no record map")?;
    if out == "--summary" {
        return summary(path, &round, &data, &map, &times);
    }
    std::fs::write(format!("{out}/{stem}.json"), serde_json::to_vec(&round)?)?;
    for (i, s) in container.streams.iter().enumerate() {
        let h = s.name_hash;
        let name = format!("{:02x}{:02x}{:02x}{:02x}", h[0], h[1], h[2], h[3]);
        let mut f = std::io::BufWriter::new(std::fs::File::create(format!("{out}/{stem}.{name}.bin"))?);
        let mut put = |t: f64, frame: u32, p: &[u8]| -> std::io::Result<()> {
            f.write_all(&t.to_le_bytes())?;
            f.write_all(&frame.to_le_bytes())?;
            f.write_all(&(p.len() as u32).to_le_bytes())?;
            f.write_all(p)
        };
        if let Some(&(a, b)) = map.snapshots.get(i) {
            put(0.0, u32::MAX, &data[a..b])?;
        }
        if let Some(si) = map.stream_index(h) {
            for (frame, a, b) in map.records_of(si) {
                put(times.get(frame as usize).copied().unwrap_or(-1.0), frame, &data[a..b])?;
            }
        }
    }
    println!("{path}: {} frames, {} streams", times.len(), container.streams.len());
    Ok(())
}

/// One JSON line: the round's defuser facts and every message of the
/// objects created with class `d0f65929` (the defuser and the bombs).
fn summary(
    path: &str,
    round: &Round,
    data: &[u8],
    map: &RecordMap,
    times: &[f64],
) -> Result<(), Box<dyn std::error::Error>> {
    use replay_analyzer::movement::messages;
    const MOVEMENT: [u8; 4] = [0x20, 0xA5, 0xC4, 0xE3];
    const CLASS: [u8; 4] = [0xD0, 0xF6, 0x59, 0x29];
    let container = round.container.as_ref().ok_or("no container")?;
    let mut blocks: Vec<(f64, &[u8])> = Vec::new();
    if let Some(i) = container.streams.iter().position(|s| s.name_hash == MOVEMENT) {
        if let Some(&(a, b)) = map.snapshots.get(i) {
            blocks.push((0.0, &data[a..b]));
        }
    }
    if let Some(si) = map.stream_index(MOVEMENT) {
        for (frame, a, b) in map.records_of(si) {
            blocks.push((times.get(frame as usize).copied().unwrap_or(-1.0), &data[a..b]));
        }
    }
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let mut known = std::collections::HashSet::new();
    let mut events = Vec::new();
    let tracks = round.movement.as_ref().map(|m| &m.players[..]).unwrap_or(&[]);
    for (t, payload) in blocks {
        for m in messages(payload) {
            let create = m.class == [0x61, 0x73, 0x85, 0xFE] || m.class == [0x62, 0x73, 0x85, 0xFE];
            if create {
                // classes: u32 n at 49, then n hashes
                let n = m.body.get(49..53).map(|b| u32::from_le_bytes(b.try_into().unwrap())).unwrap_or(0) as usize;
                let has = n <= 64 && (0..n).any(|i| m.body.get(53 + 4 * i..57 + 4 * i) == Some(&CLASS[..]));
                if has {
                    known.insert(m.object);
                }
            }
            if !known.contains(&m.object) {
                continue;
            }
            let at: Vec<serde_json::Value> = if m.body.len() > 8 && !create {
                tracks
                    .iter()
                    .filter_map(|p| {
                        let i = p.time.partition_point(|&x| x <= t + 1e-6).checked_sub(1)?;
                        Some(serde_json::json!([p.username, p.x[i], p.y[i], p.z[i]]))
                    })
                    .collect()
            } else {
                Vec::new()
            };
            events.push(serde_json::json!({
                "t": t, "obj": m.object, "class": hex(&m.class),
                "body": hex(&m.body[..m.body.len().min(200)]), "len": m.body.len(), "players": at,
            }));
        }
    }
    let v = serde_json::to_value(round)?;
    let line = serde_json::json!({
        "path": path, "map": v["map"], "site": v["site"], "gamemode": v["gamemode"],
        "matchType": v["matchType"], "round": v["round"],
        "players": v["players"].as_array().map(|a| a.iter().map(|p| serde_json::json!([p["id"], p["username"], p["teamIndex"]])).collect::<Vec<_>>()),
        "teams": v["teams"],
        "defuser": v["activity"]["defuser"], "interactions": v["activity"]["interactions"],
        "duration": times.last(), "events": events,
    });
    println!("{line}");
    Ok(())
}
