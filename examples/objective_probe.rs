//! Research probe: writes one stream's snapshot and frame records of each
//! replay to a flat file, for scripts to walk.
//!
//! `cargo run --release --example objective_probe -- <out dir> [--stream <hex>] <file.rec>...`
//!
//! Output `<out dir>/<name>.state`: repeated blocks of
//! `f64 seconds, i32 frame (-1 for the snapshot), u32 length, payload`.

use std::io::Write;
use std::path::Path;

use replay_analyzer::{container, format, header};
use replay_analyzer::records::RecordMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().ok_or("out dir")?;
    std::fs::create_dir_all(&out_dir)?;
    let mut stream: [u8; 4] = [0xA9, 0x8F, 0xDD, 0x0B];
    let mut ext = "state".to_owned();
    let mut files = Vec::new();
    while let Some(a) = args.next() {
        if a == "--stream" {
            let h = args.next().ok_or("stream hash")?;
            for (i, b) in stream.iter_mut().enumerate() {
                *b = u8::from_str_radix(&h[2 * i..2 * i + 2], 16)?;
            }
            ext = h;
        } else {
            files.push(a);
        }
    }
    for path in files {
        let p = Path::new(&path);
        let name = format!(
            "{}__{}",
            p.parent()
                .and_then(|d| d.file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
        );
        let raw = std::fs::read(p)?;
        let Ok((head, fmt, header_end)) = header::parse(&raw) else {
            eprintln!("{path}: header failed");
            continue;
        };
        let Some(index) = format::read_frame_index(&raw[header_end..], fmt.declared_frames) else {
            eprintln!("{path}: no frame index");
            continue;
        };
        let cont = container::map(&raw, header_end + index.len);
        if !cont.complete {
            eprintln!("{path}: container incomplete");
            continue;
        }
        let mut data = Vec::new();
        for &(from, to) in &cont.frames {
            data.extend(zstd::stream::decode_all(&raw[from..to])?);
        }
        let Some(map) = RecordMap::parse(&data, cont.streams.len()) else {
            eprintln!("{path}: record map failed");
            continue;
        };
        let container = &cont;
        let times = &index.times;
        let mut out = std::io::BufWriter::new(std::fs::File::create(
            Path::new(&out_dir).join(format!("{name}.{ext}")),
        )?);
        let mut put = |seconds: f64, frame: i32, payload: &[u8]| -> std::io::Result<()> {
            out.write_all(&seconds.to_le_bytes())?;
            out.write_all(&frame.to_le_bytes())?;
            out.write_all(&(payload.len() as u32).to_le_bytes())?;
            out.write_all(payload)
        };
        if let Some(&(s, e)) = container
            .streams
            .iter()
            .position(|s| s.name_hash == stream)
            .and_then(|i| map.snapshots.get(i))
        {
            put(times.first().copied().unwrap_or(0.0), -1, &data[s..e])?;
        }
        if let Some(i) = map.stream_index(stream) {
            for (frame, s, e) in map.records_of(i) {
                let t = times
                    .get(frame as usize)
                    .or(times.last())
                    .copied()
                    .unwrap_or(0.0);
                put(t, frame as i32, &data[s..e])?;
            }
        }
        println!(
            "{name}\t{}\t{}\t{}\t{:.3}",
            head.game_mode,
            head.map,
            head.match_type,
            times.last().copied().unwrap_or(0.0)
        );
    }
    Ok(())
}
