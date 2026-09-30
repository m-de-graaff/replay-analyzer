# replay-analyzer

Parses Rainbow Six Siege match replays (`.rec` files) into JSON.

```sh
replay-analyzer R01.rec                 # one round as JSON
replay-analyzer Match-2024-05-04/ -o match.json   # every round in a match folder, plus totals
replay-analyzer R01.rec --info          # short header summary
replay-analyzer R01.rec --partial       # header and players only (faster)
replay-analyzer R01.rec --census        # also count every packet and field seen
replay-analyzer MatchReplay/ --list     # every match folder: rounds, gaps, versions, hashes
replay-analyzer MatchReplay/ --players  # every player across matches: name history, you, queue-mates
replay-analyzer R01.rec --dump -o raw.bin  # decompressed stream, for format research
```

Only finished `.rec` files are read. The game also writes in-progress recordings as `*_FrameDataStream.tmprec`, `*_StaticData.tmprec` and `*_StreamInfo.tmprec`; those are refused as input and listed as skipped in a folder.

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
| `bans` | Banned operators with their side. The replay stores only the operator icon, so names resolve for icons in the lookup table (current season); older seasons keep the raw `icon` id. From Y11S3, `team` is the team that banned the operator, and each team's bans keep their slot order. The ban phase happens before recording starts, so bans carry no time. | Y8S1+ |
| `players[].level` | A number per player from the round's opening snapshot. It is most likely the clearance level: it stays the same across rounds and differs per player, but it has not been checked against Ubisoft's stats. `decodeStatus.levels` reports it as `inferred`. | Y11S3+ |
| `matchFeedback[].weapon` | Id of the gun or gadget behind each kill. | Y8S1+ |
| `loadouts` | Guns and gadgets per player, once per operator played, so attacker swaps get their own entry. Ids only: replays carry no item names. Kill `weapon` ids match these. | Y8S1+ |
| `health` | Every health change in the action phase, with the clock. | Y8S1+ |
| `lifeEvents` | Downs (DBNO) and revives. | Y8S1+ |
| `observation` | Drone and camera sessions: who, whose device, tool, phase and duration. | Y8S1+ |
| `stats[]` | Adds `damageTaken`, `downs`, `revives`, `droneSeconds` and `cameraSeconds`, summed per match too. | Y8S1+ |
| `matchFeedback[].previousOperator` | For operator swaps, the operator swapped from (`operator` is the one swapped to). | Y8S1+ |

Each round also carries a `round` block with the round itself in one place:

| Key | What it holds | How |
|---|---|---|
| `number`, `overtime`, `overtimeNumber` | Round number from 1, and whether (and which) overtime round it is. | Decoded |
| `scoreBefore`, `scoreAfter`, `matchPoint` | Score going in and coming out, and which teams were one round from winning. | Decoded |
| `winProbability` | Each team's chance to win the match going into the round if every remaining round were a coin flip. A score-only baseline, not a prediction. | Derived |
| `sides`, `site` | Attack or defense per team, and the defended site. | Decoded |
| `winner`, `winnerSide`, `endReason`, `endReasonSource` | Who won and how: `KilledOpponents`, `DefusedBomb` (the defuser went off), `DisabledDefuser` or `Time`. The reason comes from the kill feed and defuser events; `endReasonSource` is `confirmed` when the header's score names the same winner, `header` when they disagree (see `warnings`), `events` before Y9S4. | Inferred, cross-checked |
| `planted`, `plant`, `ended` | Whether and when the defuser was planted, and when the round was decided. | Decoded |
| `playersAtStart`, `startedDown`, `downAtStart` | Players per team alive when action started, and which teams started a player down. | Inferred |
| `lineup` | Per player: side, the operator played after prep, the attacker spawn (defenders get the site), and operators swapped away from. | Decoded |
| `swaps` | Attacker operator swaps: who, `from`, `to`, clock, and `late` for the last 10 seconds of prep. | Decoded |
| `phases` | `Prep`, `Action`, `Planted`, `End`, each with its start and end on the round clock and in seconds since prep started. | Inferred |

Every kill feed entry, health change, life event and observation session carries `phase` and `elapsed` (seconds since prep started), so everything sits on one timeline across the prep, action and defuser clocks. `time` stays the in-game clock: whole seconds, counting down, restarting at the plant. Events the game logs after resetting the clock at round end keep the last live second, so the kill that ended a round at 0:12 reads `0:12`, not `0:00`.

From Y11S3 the defuser is an interaction object whose countdown runs from 7.000 to 0. Plants and disables that reach zero complete; abandoned ones only have a start. The object does not say who holds it, so defuser events carry the `team` and name the player only when one player of that side was alive (`decodeStatus.defuserPlayers`).

A match folder adds `analytics`: per team attack and defense records, rounds started a player down, plants, disables and prep swaps (late ones counted apart); per site the defense win rate overall and per team; per attacker spawn and team the pick and round win rates; per operator and team rounds, win rate, kills, deaths, headshots and how often it was swapped to; and a count of rounds per end reason and winning side. `summary.rounds[]` also gains `winProbability`, `endReason` and `playersAtStart`.

Every round also says where it came from and how far it can be trusted:

| Key | What it holds |
|---|---|
| `replay.file` | Path, size, modified time and SHA-256, for deduplication and "already imported" checks. |
| `replay.format` | The `dissect` prelude: format version (7 before Y8S4, 8 since), layout, declared frame count and header property count. |
| `replay.version` | `Y11S3_Alpha04` split into season, year, season number and branch, plus the build number (`code`). |
| `replay.parser` | Parser version and the decoder profile and revision chosen for the build. A decoding fix bumps the revision of the profiles it touches, so stored rounds with an older `(decoder, decoderRevision)` are the ones to re-parse. `untestedBuild` flags builds newer than any the decoders were checked against. |
| `decodeStatus` | Per field (`players`, `kills`, `scoreboard`, `bans`, `health`, `result`, `timing`, ...): `decoded`, `inferred`, `partial`, `missing`, `notInVersion` or `skipped`, with a count and warnings such as how many packets failed. `trusted` is false when any field is partial or missing. |
| `timing` | From the frame time index: frame count, duration, median interval, sample rate, and intervals over 4x the median (`gaps`). `clockGaps` lists seconds the in-game clock skipped, ignoring the reset at round end and the switch to the defuser timer. From Y11S3, `startedAt` (UTC) and the UTC offset of the header's local `timestamp`. |
| `census` | With `--census`: every known packet marker with seen and failed counts, known markers never seen, every header key (unknown ones listed), and every property hash seen three or more times with its value sizes, known or not. |
| `startTime`, `endTime`, `isSpectator`, `maxPlayersPerTeam`, `matchResult` | Header keys added in Y11S3. `matchResult` appears only on the round that decides the match. |

A match folder adds `summary`, one record per match for match history. It is built from round headers only, so `--list` carries it too:

| Key | What it holds |
|---|---|
| `matchID` | Shared by every round and every player's recording, so teammates importing the same match dedupe on it. |
| `startTime`, `endTime` | UTC, from the first and last round read. Before Y11S3 only the recording PC's local time exists; `startTimeIsLocal` says so. |
| `matchType`, `queue`, `playlistCategory` | The raw match type with its name, and its queue family: `ranked`, `unranked`, `quickMatch`, `custom`, `standard`. `playlistCategory` is raw. |
| `gameMode` | Bomb, Secure Area, Hostage, ... with the raw id. |
| `map` | `id`, full `name`, and `base` plus rework `version` (`BankY10` is `Bank`, `Y10`). Reworked maps get new ids, so key floor plans and callouts by `id`. |
| `rules` | Regulation rounds and the rounds needed to win, overtime rounds and the total needed once overtime starts, players per team, and the raw `gameModeSettings` list (kept raw until each value is named). |
| `teams` | Name, final score, starting side, and players with profile id and level. |
| `recording`, `yourTeam` | Who recorded, whether as a spectator, and the index of their team (absent for spectators). |
| `result` | Final score, `winner`, `outcome` from your side (`win`, `loss`, `draw`, `decided` for spectators, `unfinished`), whether it went to overtime, and `endedEarly` when the game ended the match before either team reached the target (forfeit or abandon; Y11S3+). |
| `rounds[]` | Per round: score before and after, winner, each team's side, which teams were on match point, overtime, the bomb sites, `bans` with the banning team, and `picks`: every operator each player played, in order, so swaps show up as several operators. `bans` and `picks` need a full read. |

A match folder also adds `folder`: rounds found and missing (numbered from 1 like `R01.rec`), duplicate round numbers, skipped files (`.tmprec`, byte-identical copies, unreadable files), the match ids seen, the final score and whether the match finished. Rounds are ordered by the header's round number, not the file name. One unreadable round no longer fails the whole folder.

Limits worth knowing:

- Replays hold no rank, reputation or server region. The Y11S3 test replays were searched for these and they weren't there. Use the players' `profileID` with Ubisoft's stats services for rank.
- `endedEarly` (forfeits, abandoned matches) is inferred: the game marks the deciding round with `matchresult`, so a marked round where neither team reached the win target means the match ended early. No forfeit replay has been checked yet.
- Dual Front (6v6, respawns) has not been seen in a replay. The player-count check follows `maxPlayersPerTeam`, and `picks` can hold several operators per player, but respawns are not decoded.

- Replays record a player's health, never who caused a change, so there is damage taken but no damage dealt.
- Older replays sometimes skip the last health update before a kill, which makes `damageTaken` a lower bound.
- Observation tool ids 1 (drone), 2 (camera), 6 (Black Eye), 8 (Flores drone) and 9 (shock drone) are confirmed; 3 is a second camera kind seen on defenders with a camera gadget. Others print as `ObservationTool(n)`.

Y11S3 attacker swaps are linked through the player's state object, because the caster UI id older seasons use is shared by a whole team there.

Y11S3 scoreboard packets no longer carry ids that match players; they are linked through each player's scoreboard object instead (see [Players and identity](#players-and-identity)).

## Players and identity

Every player in `players[]` carries:

| Key | What it holds | Versions | How |
|---|---|---|---|
| `profileID` | Ubisoft profile id: the stable key for ranks and stats. Can be missing, in older replays. | Y8S1+ | Decoded |
| `key` | `profileID`, or `name:<username>` without one. Use it to recognise a player across matches; usernames change. | all | Derived |
| `relation` | `you`, `teammate` or `opponent`, relative to whoever recorded. Absent for spectator recordings. | all | Decoded (Y8S1+), else from the header's recording ids |
| `party` | `leader` or `member` of the recording player's party. Only the recorder's own party is in the file; custom games put the whole lobby in one party, so no roles are given there. | Y8S1+ | Decoded |
| `entities` | Hex ids of the objects that carry the player in the packet stream: `controller` (name, operator, team, weapon-ready flag; pick and swap packets write to it), `scoreboard`, `health`, and `movement`, the body the movement stream moves. | Y8S1+ (`movement` Y11S3+) | Decoded |
| `spawnPosition` | Where the player's body was created, in map coordinates. | Y11S3+ | Decoded |

Kill feed entries name their players' `profileID` and `targetProfileID`. From Y11S3 a kill can also carry `creditedTo`: the scoreboard credits a kill to the teammate who downed the victim when another player finished them, while the feed names the finisher. With those credits counted, the scoreboard's kill and death totals match the kill feed in every test round.

`weaponReady` lists each change of the controller's weapon-ready flag (`ready`, `phase`, `elapsed`). Attackers hold it at `false` through prep, on their drones, until their body spawns; in action it drops for about a second at a time, as on reloads and weapon swaps. The meaning is inferred from that behaviour (`decodeStatus.weaponReady`).

`decodeStatus` adds `profileIds`, `recorder`, `entities`, `movement`, `party` and `weaponReady`. `summary.teams[].players[]` gains `key`, `relation` and `party`, and `summary.recording.party` lists who queued with the recorder.

`--players` reads every match folder under a folder (partially: players, relations and parties) and prints a directory: per player the `key`, every username used with first and last seen, matches with and against you, matches in your party, and `queueMate` (queued with you once, or on your team in two or more matches). `you` lists the recording accounts. A match imported twice (same `matchID`) counts once. The library equivalent is `PlayerDirectory::new(&summaries)`.

How the links are made (details in `src/entities.rs`):

- The stream is a tree of replicated objects. Besides `23`/`22` property records, `1b <parent> <field> <child>` and `1a <field> <child>` records hang child objects off a parent. Each player has one controller object under their team's object, holding the name, operator, profile id and the header `playerid`; the scoreboard, health, inventory and a profile object hang off it.
- The profile object carries the relation to the recorder (`05c7b949`: 1 opponent, 2 teammate, 3 teammate in the recorder's party, 5 the recorder) and the party role (`af6bb287`: 0, 1 member, 2 leader). Checked on Y8S1 ranked and quick matches (a clan-tagged five-stack, a duo with randoms), Y8S2 and Y9S1.
- Movement is sent apart from the tree. A player table (count byte, then per player the header `playerid` and, when it changed, the object the player now controls) links each body to a player explicitly, so the link does not depend on player order. The same table records drone and camera switches.

Not found:

- **Platform.** No field varies with platform in the replays available (all PC). The profile object has constant fields (`96ba4a74` = 3 from Y9S1) that could be one, unconfirmed.
- **Cosmetics.** The controller's operator asset id (`f93911f2`), the header's `heroname` and `roleimage`, and the weapon item ids are the same for every player on the same operator in the test match, so no personal uniform, headgear, skin or charm id has been identified. Pro matches may force defaults; a ranked Y11S3 replay would settle it.
- **Other players' parties.** Only the recorder's party is recorded. Premades among other players can only be guessed from match history (`--players`).

## File format notes

What sits around the header, as observed from Y8S1 to Y11S3:

```text
"dissect" 00                 magic
u32 format                   7 up to Y8S3, 8 from Y8S4
u8 7, 7 zero bytes, "UNKNOWN"
u32 0, u32 last frame, u32 property count, u32 0
properties                   u8 length, 7 zero bytes, text; key then value
u32 ?, u32 ?, u32 frames     frame time index: frames x (u32 index, f64 seconds)
per-player table, "CMPRV002" trailer
```

From Y8S4 the header and index are uncompressed and the packet stream follows as independent zstd frames; before that everything is one zstd stream. Reading only the header therefore needs no decompression for Y8S4+ (`ReadMode::Header`).

The index is wall-clock accurate: in Y11S3, `starttime` plus the index duration lands within 2 ms of `endtime`. It also shows the recording rate changed: roughly 200 to 260 frames a second in Y8 and Y9 replays, about 29 in Y11S3. The header `datetime` is the recording PC's local time, not UTC.

## Benchmarks

`cargo bench` runs [Criterion](https://github.com/bheisler/criterion.rs) over the Y11S3 replays in `test_recordings/valid/Y11S3`. Single-round benches parse an ~8.7 MiB round already in memory; the match bench reads a 10-round folder (~81 MiB) from disk.

| Bench | What it does | Time | Throughput |
|---|---|---|---|
| `round/decompress` | zstd frames to the raw stream | 76 ms | 115 MiB/s |
| `round/header` | header and frame index only; no decompression | 0.19 ms | n/a |
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
