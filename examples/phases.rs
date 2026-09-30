//! Times each parsing phase: `cargo run --release --example phases -- <file.rec>...`
use std::time::Instant;

fn main() {
    for path in std::env::args().skip(1) {
        let raw = std::fs::read(&path).unwrap();
        let t = Instant::now();
        let data = replay_analyzer::decompressed_bytes(&raw).unwrap();
        let decompress = t.elapsed();
        let t = Instant::now();
        let round =
            replay_analyzer::Round::from_bytes(&raw, replay_analyzer::ReadMode::Full).unwrap();
        let full = t.elapsed();
        let t = Instant::now();
        let json = serde_json::to_string(&round).unwrap();
        let ser = t.elapsed();
        println!(
            "{path}: {} MB -> {} MB, decompress {decompress:?}, full parse {full:?}, json {ser:?} ({} B)",
            raw.len() >> 20,
            data.len() >> 20,
            json.len()
        );
    }
}
