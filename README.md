# replay-analyzer

Parses Rainbow Six Siege match replays (`.rec` files) into JSON.

```sh
replay-analyzer R01.rec                 # one round as JSON
replay-analyzer Match-2024-05-04/ -o match.json   # every round in a match folder, plus totals
replay-analyzer R01.rec --info          # short header summary
replay-analyzer R01.rec --partial       # header and players only (faster)
replay-analyzer R01.rec --dump -o raw.bin  # decompressed stream, for format research
```

Pass `--pretty` for indented JSON and `--debug` for a packet-level log on stderr.

## Library

```rust
use replay_analyzer::{Match, ReadMode, Round};

let round = Round::open("R01.rec", ReadMode::Full)?;
for kill in round.kills_and_deaths() {
    println!("{} {} -> {}", kill.time, kill.username, kill.target);
}
let game = Match::open("Match-2024-05-04/")?;
```

## Design

- **Fast.** Packet markers are found with one SIMD Aho-Corasick pass, split across cores. Y8S4+ zstd frames are decompressed in parallel, and match folders parse all rounds at once. See [Benchmarks](#benchmarks).
- **Robust.** A malformed packet is logged and skipped instead of aborting the whole read or panicking. Unknown operators, maps and modes keep their raw id instead of crashing role lookups.

## Output

Besides the header, players, kill feed and scoreboard, round JSON carries:

| Key | What it holds | Versions |
|---|---|---|
| `bans` | Banned operators with their side. The replay stores only the operator icon, so names resolve for icons in the lookup table (current season); older seasons keep the raw `icon` id. | Y8S1+ |
| `matchFeedback[].weapon` | Id of the gun or gadget behind each kill. | Y8S1+ |
| `loadouts` | Guns and gadgets per player, once per operator played, so attacker swaps get their own entry. Ids only: replays carry no item names. Kill `weapon` ids match these. | Y8S1+ |
| `health` | Every health change in the action phase, with the clock. | Y8S1+ |
| `lifeEvents` | Downs (DBNO) and revives. | Y8S1+ |
| `observation` | Drone and camera sessions: who, whose device, tool, phase and duration. | Y8S1+ |
| `stats[]` | Adds `damageTaken`, `downs`, `revives`, `droneSeconds` and `cameraSeconds`, summed per match too. | Y8S1+ |

Limits worth knowing:

- Replays record a player's health, never who caused a change, so there is damage taken but no damage dealt.
- Older replays sometimes skip the last health update before a kill, which makes `damageTaken` a lower bound.
- Observation tool ids 1 (drone), 2 (camera), 6 (Black Eye), 8 (Flores drone) and 9 (shock drone) are confirmed; 3 is a second camera kind seen on defenders with a camera gadget. Others print as `ObservationTool(n)`.

Y11S3 attacker swaps are linked through the player's state object, because the caster UI id older seasons use is shared by a whole team there.

## Benchmarks

`cargo bench` runs [Criterion](https://github.com/bheisler/criterion.rs) over the Y11S3 replays in `test_recordings/valid/Y11S3`. Single-round benches parse an ~8.7 MiB round already in memory; the match bench reads a 10-round folder (~81 MiB) from disk.

| Bench | What it does | Time | Throughput |
|---|---|---|---|
| `round/decompress` | zstd frames to the raw stream | 76 ms | 115 MiB/s |
| `round/header` | header and players only | 58 ms | 151 MiB/s |
| `round/partial` | `ReadMode::Partial` | 72 ms | 121 MiB/s |
| `round/full` | `ReadMode::Full`, every packet | 141 ms | 62 MiB/s |
| `match/folder_10_rounds` | `Match::open` on 10 rounds | 823 ms | 98 MiB/s |

Measured on an Intel Core Ultra 7 255H (16 threads), Windows 11, Rust 1.97.1, release profile. Numbers are medians; expect run-to-run variation of a few percent.

### Compared with other parsers

`benches/compare.sh` times the command-line tools end to end with [hyperfine](https://github.com/sharkdp/hyperfine) (process start, file read, parse and JSON write), on the same machine. Tools compared:

- [r6-dissect](https://github.com/redraskal/r6-dissect) (Go): the v0.24.0 release and `master` at `e6c2ca8`, built with Go 1.27.1.
- [replay-tool](https://github.com/wnc-replay/replay-tool) (Go): `master`. It also reconstructs positions, aim and shots, so it does far more work per round and is not a like-for-like comparison.

Python parsers ([draguve/R6-Replays](https://github.com/draguve/R6-Replays), [kevprakash/R6-Match-Replay-Analysis](https://github.com/kevprakash/R6-Match-Replay-Analysis)) only read the pre-Y8S4 layout and fail on current replays, so they are left out.

**Compatibility.** replay-analyzer reads all 10 Y11S3 test rounds. Both r6-dissect builds panic on 7 of them (`role unknown for operator ID …`, for operators added after its role table).

| Task | replay-analyzer | r6-dissect `master` | r6-dissect v0.24.0 | replay-tool |
|---|---:|---:|---:|---:|
| One round to JSON (`custom_1.rec`) | **197 ms** | 690 ms (3.5×) | 797 ms (4.0×) | 72.4 s |
| Header and players only | **168 ms** | 559 ms (3.3×) | | 57.7 s¹ |
| 3-round match folder to JSON² | **281 ms** | 1,811 ms (6.4×) | | |

Means over at least 10 runs after 3 warm-up runs (replay-tool: 3 runs). ¹ replay-tool's `-header` still decompresses and scans the whole replay. ² Only the 3 rounds r6-dissect can read; it reads a folder's rounds one after another, replay-analyzer in parallel.

Reproduce with `cargo build --release` and `R6_DISSECT=… REPLAY_TOOL=… benches/compare.sh`; results land in `target/compare/`.

## Tests

`cargo test` checks the replays in `test_recordings/valid/` against facts known from the game, compares output against a `.rec.json` expectation where one sits next to a replay, and checks that everything in `test_recordings/invalid/` is rejected. It also re-packs a replay into the Y8S4+ chunked layout to cover that path. Set `R6_TEST_DATA` to test against another folder. Replay tests are skipped when the data is missing.
