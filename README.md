# replay-analyzer

A Rust port of [r6-dissect](https://github.com/redraskal/r6-dissect): it parses Rainbow Six Siege match replays (`.rec` files) into JSON.

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

## Compared with r6-dissect

The JSON output is the same as r6-dissect's (checked against its test replays and the Go binary). The differences:

- **Faster.** Packet markers are found with one SIMD Aho-Corasick pass, split across cores. Y8S4+ zstd frames are decompressed in parallel, and match folders parse all rounds at once. On the test replays a single round is about 3× faster and a 5-round folder about 4× faster.
- **Robust.** A malformed packet is logged and skipped instead of aborting the whole read or panicking. Unknown operators, maps and modes keep their raw id instead of crashing role lookups.
- **Correct marker search.** The Go byte-matcher could miss a marker that followed a partial match, or one that crossed a worker boundary. This port uses exact substring search.
- **Scope.** Excel export and the custom-listener API are not ported.

## Beyond r6-dissect

Round JSON also carries data r6-dissect does not extract. It sits in its own keys, so the r6-dissect fields are unchanged apart from `weapon` on kills.

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

## Tests

`cargo test` compares output against the expected JSON next to each replay in `test_recordings/valid/` (falling back to `.opensrc/r6-dissect`; override with `R6_TEST_DATA`), and checks that everything in `test_recordings/invalid/` is rejected. It also re-packs a replay into the Y8S4+ chunked layout to cover that path. Replay tests are skipped when the data is missing.
