//! Parse speed on the Y11S3 replays in `test_recordings/valid/Y11S3`.
//!
//! Single-round benches read the file into memory first, so they measure
//! parsing only. `match_folder` includes file reads, as the CLI does.
//!
//! Run with `cargo bench`.

use std::hint::black_box;
use std::path::{Path, PathBuf};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use replay_analyzer::{Match, ReadMode, Round, decompressed_bytes};

fn replay_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("test_recordings/valid/Y11S3")
}

fn round(c: &mut Criterion) {
    let raw = std::fs::read(replay_dir().join("custom_1.rec")).expect("test replay missing");
    let mut g = c.benchmark_group("round");
    g.throughput(Throughput::Bytes(raw.len() as u64));
    g.bench_function("decompress", |b| {
        b.iter(|| decompressed_bytes(black_box(&raw)).unwrap())
    });
    g.bench_function("header", |b| {
        b.iter(|| Round::header_only(black_box(&raw)).unwrap())
    });
    g.bench_function("partial", |b| {
        b.iter(|| Round::from_bytes(black_box(&raw), ReadMode::Partial).unwrap())
    });
    g.bench_function("full", |b| {
        b.iter(|| Round::from_bytes(black_box(&raw), ReadMode::Full).unwrap())
    });
    g.finish();
}

fn match_folder(c: &mut Criterion) {
    let dir = replay_dir();
    let bytes: u64 = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok()?.metadata().ok())
        .map(|m| m.len())
        .sum();
    let mut g = c.benchmark_group("match");
    g.throughput(Throughput::Bytes(bytes));
    g.sample_size(20);
    g.bench_function("folder_10_rounds", |b| {
        b.iter(|| Match::open(black_box(&dir)).unwrap())
    });
    g.finish();
}

criterion_group!(benches, round, match_folder);
criterion_main!(benches);
