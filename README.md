# replay-analyzer

Parses Rainbow Six Siege match replays (`.rec` files) into JSON.

```sh
replay-analyzer R01.rec                 # one round as JSON
replay-analyzer Match-2024-05-04/ -o match.json   # every round in a match folder, plus totals
replay-analyzer R01.rec --info          # short header summary
replay-analyzer R01.rec --partial       # header and players only (faster)
replay-analyzer R01.rec --census        # also count every packet and field seen
replay-analyzer MatchReplay/ --list     # every match folder, game session, unfinished or unsaved round, copy and leftover
replay-analyzer MatchReplay/ --players  # every player across matches: name history, you, queue-mates
replay-analyzer --decoders              # decoder profiles and tested builds, to find rounds worth re-parsing
replay-analyzer R01.rec --dump -o raw.bin  # decompressed stream, for format research
```

Only finished `.rec` files are read. While a round records, the game keeps it in temporary `.tmprec` files and deletes them once the round is saved. Players have found leftovers named like `P15440_50_Y2022_M1_D16_H23_M58_FrameDataStream.tmprec`, with `StaticData` and `StreamInfo` parts: the game's process id, a stream id, the local time and the part. They are refused as input, and folders and `--list` report any they find with what their names say.

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
| `bans` | Banned operators with their side. The replay stores only the operator icon, so names resolve for icons in the lookup table (current season); older seasons keep the raw `icon` id. From Y11S3, bans come from the ban slots in the round's opening snapshot: `team` is the team that banned, `slot` its place in that team's order (from 0), and `noBan` marks a vote that ended without a ban. Bans are listed by team, then slot. The ban phase happens before recording starts, so bans carry no time. See [Bans](#bans). | Y8S1+ |
| `players[].level` | The clearance level, from the round's opening snapshot (the game's property is `ClearanceLevelText`). Replays hold no rank, rank points or reputation; see [Limits](#limits-worth-knowing). | Y11S3+ |
| `teams[].color` | The game's number for the team (`TeamColor`, 1 or 2), from its team object. Ban slots and `matchresult` name teams by it: in a player's own recording the player's team is always 1, whatever its index in the header. | Y11S3+ |
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
| `sides`, `site` | Attack or defense per team, and the defended site. From Y11S3 each team object states its side (`HeroTeam`); before, the side comes from the operators picked (`decodeStatus.teamRoles` says which). | Decoded (Y11S3+), else inferred |
| `winner`, `winnerSide`, `endReason`, `endReasonSource` | Who won and how: `KilledOpponents`, `DefusedBomb` (the defuser went off), `DisabledDefuser` or `Time`. The reason comes from the kill feed, players leaving, the defuser state and whether the clock ran down to zero. From Y11S3 the game also writes its own verdict per team when the round is decided (`Team0RoundState`, `Team1RoundState`: lost, won by elimination with no plant, won with the defuser planted, won on time, cancelled); it agreed with the events in all 171 decided rounds checked, and `decodeStatus.winCondition` is `decoded` when it does. `endReasonSource` is `confirmed` when the header's score names the same winner, `header` when they disagree (see `warnings`), `events` before Y9S4, and `unfinished` when the score did not change: the recording stopped or the match was ended mid-round, and the round has no winner or reason. | Decoded and cross-checked (Y11S3+), else inferred |
| `planted`, `plant`, `ended` | Whether and when the defuser was planted, and when the round was decided. `plant.time` is the action clock at the plant. | Decoded |
| `playersAtStart`, `startedDown`, `downAtStart` | Players per team alive when action started, and which teams started a player down. Left out when the round never reached action. | Inferred |
| `left` | Players alive at the start who left or lost connection before the round was decided. The kill feed has no entry for them; they count as gone when deciding how the round ended. | Decoded (Y11S3+) |
| `lineup` | Per player: side, the operator played after prep, the attacker spawn (defenders get the site), and operators swapped away from. The spawn is the last one picked in prep; `RANDOM` stays only when the game never wrote the spawn it chose. | Decoded |
| `swaps` | Attacker operator swaps: who, `from`, `to`, clock, and `late` for the last 10 seconds of prep (clock at `0:09` or below; the clock shows whole seconds rounded down). | Decoded |
| `phases` | `Prep`, `Action`, `Planted`, `End`, each with its start and end on the round clock and in seconds since prep started. | Decoded (Y11S3+), else inferred |

Every kill feed entry, health change, life event and observation session carries `phase` and `elapsed` (seconds since prep started), so everything sits on one timeline across the prep, action and defuser clocks. From Y8S4 they also carry `recordingTime`: seconds since the recording started, to the frame, taken from the frame record the packet sits in. Round phases carry `recordingStart` and `recordingEnd` the same way, and `timing.startedAt` turns either into UTC. In every test round a kill's `recordingTime` minus its `elapsed` stays within a second across the round, so the round clock and the recording agree; where the clock cannot say (a new timer taking over, the clock standing at `0:00` while a plant finishes) `elapsed` takes the whole seconds from the recording. `time` stays the in-game clock: whole seconds, counting down, restarting at the plant. Events the game logs after resetting the clock at round end keep the last live second, so the kill that ended a round at 0:12 reads `0:12`, not `0:00`.

From Y11S3 each player has a defuser interaction object, hung off their controller, whose countdown runs from 7.000 towards 0; it starts a `DefuserPlantStart` or `DefuserDisableStart` with that player's name. The countdown does not decide anything: the game has completed a plant with 0.635 on it and abandoned one at 0.826. What does is `IsDefuserStarted` on the game-mode object, which turns 1 in the frame a plant completes (the clock switches to the defuser timer in that frame) and back to 0 when a disable completes. A plant that runs out after the round is decided leaves it at 0 and is not a plant. The round's end is `TimerState` 3 on the clock object, written in the frame the round is decided: when time ran out, or a plant was under way at `0:00`, that is up to several seconds after the clock's last reading.

A match folder adds `analytics`: per team attack and defense records, rounds started a player down, plants, disables and prep swaps (late ones counted apart); per site the defense win rate overall and per team; per attacker spawn and team the pick and round win rates; per operator and team rounds, win rate, kills, deaths, headshots and how often it was swapped to; and a count of rounds per end reason and winning side. `summary.rounds[]` also gains `winProbability`, `endReason` and `playersAtStart`.

Every round also says where it came from and how far it can be trusted:

| Key | What it holds |
|---|---|
| `replay.file` | Path, size, modified time and SHA-256, for deduplication and "already imported" checks. |
| `replay.format` | The `dissect` prelude: format version (7 before Y8S4, 8 since), layout, declared frame count and header property count. A format version, label or layout not seen before is flagged in `decodeStatus.header`. |
| `replay.version` | `Y11S3_Alpha04` split into season, year, season number and branch, plus the build number (`code`). |
| `replay.parser` | Parser version and the decoder profile and revision chosen for the build. A decoding fix bumps the revision of the profiles it touches, so stored rounds with an older `(decoder, decoderRevision)` than `--decoders` lists for their build are the ones to re-parse. `untestedBuild` flags builds newer than any the decoders were checked against (9883691, 9901603 and 9918362, all `Y11S3_Alpha04`). |
| `replay.container` | Y8S4+: the streams the round was recorded in (id, name hash, role where known, frames covered, snapshot blocks, record count), the compressed blocks, `recordingId`, and `complete`, false when the game did not finish writing the file. See [File format notes](#file-format-notes). |
| `decodeStatus` | Per field (`container`, `players`, `kills`, `scoreboard`, `bans`, `health`, `result`, `timing`, ...): `decoded`, `inferred`, `partial`, `missing`, `notInVersion` or `skipped`, with a count and warnings such as how many packets failed. `trusted` is false when any field is partial or missing. What a replay never records (text feed messages, from Y9S1) is `notInVersion`, and a player's own recording leaving out their own body is expected, so `trusted` marks faults: a field left unread (`skipped`: a partial read, a custom game's party) does not lower it either. Of the 167 real rounds in one `MatchReplay` folder, the 8 untrusted ones were 3 unfinished files, 3 rounds that were not played out (no result), and 2 where the recorder's own body was not linked. |
| `timing` | From the frame index: frame count, duration, median interval and the `sampleRate` it gives, `meanRate`, and intervals over 4x the median (`gaps`). `dataRate` is how often the game sent updates, whatever the frame rate: records per second in the state stream, about 28. `holes` are stretches over 0.5 s without a movement record, which the game writes at every update. `clockGaps` lists seconds the in-game clock skipped, ignoring the reset at round end and the switch to the defuser timer. From Y11S3, `startedAt` (UTC) and the UTC offset of the header's local `timestamp`. |
| `census` | With `--census`: every known packet marker with seen and failed counts, known markers never seen, every header key (unknown ones listed), and every property hash seen three or more times with its value sizes and the stream it was seen in, known or not. `unknownStreams` lists streams whose role is unknown and `streamsNotSeen` known ones the replay lacks, so a stream or field added by a patch stands out. |
| `startTime`, `endTime`, `isSpectator`, `maxPlayersPerTeam`, `matchResult` | Header keys added in Y11S3. The game writes `isspectator` only for spectators, so it reads `false` when absent. `matchResult` appears only on the round that ends the match, as the result of the team numbered 1 (`teams[].color`): 2 won, 1 lost, 7 the game ended the match with no winner. |

A match folder adds `summary`, one record per match for match history. It is built from round headers only, so `--list` carries it too:

| Key | What it holds |
|---|---|
| `matchID` | Shared by every round and every player's recording, so teammates importing the same match dedupe on it. |
| `startTime`, `endTime` | UTC, from the first and last round read. Before Y11S3 only the recording PC's local time exists; `startTimeIsLocal` says so. |
| `matchType`, `queue`, `playlistCategory`, `playlist` | The raw match type with its name, and its queue family: `ranked`, `unranked`, `quickMatch`, `custom`, `standard`. Match type ids have been renumbered over the years; in Y11S3, 7 is Unranked (inferred: ranked rules and bans, but players below the level Ranked needs, and Tower in the pool). `playlistCategory` is an asset id with one value per playlist, named in `playlist` for the ones seen (`Ranked`, `QuickMatch`, `Unranked`). |
| `gameMode` | Bomb, Secure Area, Hostage, ... with the raw id. |
| `map` | `id`, full `name`, and `base` plus `version` (`BankY10` is `Bank`, `Y10`). The game gives a map a new id when it rebuilds it, so key floor plans and callouts by `id`. The version is the year of that build; the floor plan can still be the old one (`SkyscraperY10` keeps Skyscraper's sites). |
| `rules` | Regulation rounds and the rounds needed to win, overtime rounds and the total needed once overtime starts, players per team, and the raw `gameModeSettings` list (kept raw until each value is named). The settings, like `playlistCategory`, are asset ids: one fixed list per playlist, and the stream never refers to them. |
| `teams` | Name, final score, starting side, and players with profile id and level. Matchmaking names the teams `YOUR TEAM` and `ENEMY TEAM` from the recorder's side; custom games carry the real names. |
| `recording`, `yourTeam` | Who recorded, whether as a spectator, and the index of their team (absent for spectators). |
| `result` | Final score, `winner`, `outcome` from your side (`win`, `loss`, `draw`, `cancelled`, `decided` for spectators, `unfinished`), whether it went to overtime, `rawMatchResult`, and `endedEarly` when the game ended the match before either team reached the target (Y11S3+): a forfeit has a `winner` from `matchresult`; a match the game ended with no winner (`matchresult` 7) is `cancelled`. `unfinished` means the rounds read do not end the match, usually because the recorder left. |
| `bans` | Each ban once, with `round`, the first round it applied to: what each team banned and when. In ranked that is the round the team's vote came before. Needs a full read. |
| `rounds[]` | Per round: score before and after, winner, each team's side, which teams were on match point, overtime, the bomb sites, `bans` in force with the banning team and slot, and `picks`: every operator each player played, in order, so swaps show up as several operators. `bans` and `picks` need a full read. |

A match folder also adds `folder`: rounds found and missing (numbered from 1 like `R01.rec`), duplicate round numbers, skipped files (`.tmprec`, byte-identical copies, unreadable files), the match ids seen, the final score and whether the match finished. Rounds are ordered by the header's round number, not the file name. One unreadable round no longer fails the whole folder.

`name` reads the game's folder name, `Match-2026-09-20_00-32-29-13160`: when the folder was created, in local time, and the game's process id. Each round file's name (`…-R03.rec`) is checked against its header and its folder. `incomplete` lists round files the game did not finish writing, `unsavedRecordings` the stream ids skipped between two consecutive rounds (a recording that was started and never saved), and `temporary` any `.tmprec` files, with what their names say.

## Match history folder

`--list` reads every match folder under a folder, headers only (0.4 s for 30 folders), and prints:

| Key | What it holds |
|---|---|
| `folders[]` | Each folder's `folder` report and `summary`, and `roundList`: per round the file, round number, `matchID`, `startTime` (UTC, Y11S3+), `localTime`, version, parser, `recordingId`, `complete`, frame count, sample rate and gaps. |
| `sessions[]` | One per run of the game: process id, folders, rounds, and first and last `recordingId`. Ids start at 0 when the game starts, so a higher first id means rounds of that run are no longer in the folder, at 9 to 12 ids each. `idGaps` lists ids no round used between two rounds, within or across folders: fewer than 9 is a recording that was never saved, more can also be rounds no longer here. The same process id starting over at 0 is a new run. |
| `duplicates[]` | `sameFile`: byte-identical copies. `sameRound`: different files of the same round of the same match, such as a teammate's recording of it. |
| `temporary[]` | `.tmprec` files under the folder, in its `DissectTmp`, and, for a `MatchReplay` folder, in the game folder around it and that folder's `DissectTmp`. |

The game keeps a fixed number of matches: the folder held 30 on both days it was read, the oldest dropping out as new ones arrived. Copy out what should last; a session's `firstRecordingId` shows how much of it is already gone.

The library equivalent is `library::scan(path, ReadMode::Header)`.

### Limits worth knowing

- **Replays hold no rank, rank points, max rank or reputation.** All 167 rounds of a real `MatchReplay` folder (21 ranked, 4 unranked and 5 quick matches) and the 10 test rounds were searched: every header key, every property of every object in the state stream, every other stream, number scans for rank-point values, and text in ASCII and UTF-16. The per-player profile object holds the same 21 properties in ranked, unranked, quick and custom matches, and none is a rank. The in-game scoreboard shows ranks, so the game must fetch them from Ubisoft's services. Use the players' `profileID` with Ubisoft's stats services (or the R6 Data API) for lobby strength.
- **Replays hold no server region and no ping.** The same rounds were searched for the data-center names the game itself uses (`gamelift/eu-west-1` and so on, plain and hashed), for region, host and IP text, and for any per-player value that behaves like latency. The scoreboard object carries score, kills, deaths, assists, a placement that is always 0 and a per-match counter, and nothing else. The only geographic hint is the recording PC's UTC offset (`timing`).
- **`endedEarly`** comes from `matchresult`. A forfeit, where a team surrenders, should carry 1 or 2 like a normal ending and so get a winner, but no forfeit has been seen. Value 7 has been seen once: the server ended a ranked match 0.03 s into round 4, with no clock, both teams marked as losing and a system notice no other round has. The cause, which the file does not say, may be a ban of a cheating player, which ends a match for everyone.
- **Dual Front** (6v6, respawns) has not been seen in a replay, and no source says whether it records one. Pick packets are split into teams by `maxPlayersPerTeam`, and `picks` can hold several operators per player, but respawns are not decoded.

- Replays record a player's health, never who caused a change, so there is damage taken but no damage dealt.
- Older replays sometimes skip the last health update before a kill, which makes `damageTaken` a lower bound.
- Observation tool ids 1 (drone), 2 (camera), 6 (Black Eye), 8 (Flores drone) and 9 (shock drone) are confirmed; 3 is a second camera kind seen on defenders with a camera gadget. Others print as `ObservationTool(n)`.
- No round in the test match or the real folder ends with the defuser going off, so `DefusedBomb` is untested from Y11S3: it is given when attackers win after a plant with defenders left and the defuser timer at zero.
- `elapsed` counts whole clock seconds, and the first second of a recording is cut short, so it can sit up to two seconds from `recordingTime` minus the prep start.
- An attacker whose pick stays `RANDOM` in the file (3 of 50 random picks) has no spawn name; `spawnPosition` still says where the body appeared.

Y11S3 attacker swaps are linked through the player's state object, because the caster UI id older seasons use is shared by a whole team there.

Y11S3 scoreboard packets no longer carry ids that match players; they are linked through each player's scoreboard object instead (see [Players and identity](#players-and-identity)).

## Bans

Y11S3 replays hold the ban phase as objects in each round's opening snapshot: a ban manager with three slots per team. A slot names the side it bans (`HeroTeam`), the banning team (`TeamColor`), whether it has been used (`BanState`), whether the vote ended without a ban (`ResultType`), and the banned operator, whose descriptor links its icon (`BadgeIcon`, the id the header's `roleimage` uses).

As recorded in the 25 ranked and unranked matches of a real folder:

- Each team bans one operator of the other side before each round of a half, into its next slot: 2, 4, then 6 bans in rounds 1 to 3, and again after the side swap, when the slots start over.
- Overtime adds no bans. An overtime round holds the bans of the half whose sides it repeats.
- A team's vote can end without a ban (`noBan`), seen once in 25 matches.
- Quick matches have the ban manager but use no slots.
- The 12-round custom test match bans two operators per team at the start of each half and a third in the half's fourth round.

Per-player votes, the timing of the ban phase and the operator pick phase (hovers, lock order) are not recorded: the ban phase ends before the recording starts, and a file refers only to operators that were played or banned. `summary.bans` lists each ban once with the first round it applied to.

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

- **Platform.** The controller has a `PlayerPlatform` property (`7dd4fc18`): 0 for most players in a real folder of PC matches, 5 or 7 for a few. What the values stand for is not known, so it is not output. The profile object's `96ba4a74` is 3 for every player.
- **Cosmetics.** The controller's operator asset id (`f93911f2`), the header's `heroname` and `roleimage`, and the weapon item ids are the same for every player on the same operator in the test match, so no personal uniform, headgear, skin or charm id has been identified. Pro matches may force defaults; a ranked Y11S3 replay would settle it.
- **Other players' parties.** Only the recorder's party is recorded. Premades among other players can only be guessed from match history (`--players`).

## File format notes

What sits around the header, as observed from Y8S1 to Y11S3:

```text
"dissect" 00                 magic
u32 format                   7 up to Y8S3, 8 from Y8S4
str "UNKNOWN"                str: u64 length, then the bytes
u32 0, u32 last frame, u32 property count, u32 0
properties                   str key, str value
u32 ?, u32 ?, u32 frames     frame time index: frames x (u32 index, f64 seconds)
```

From Y8S4 the header and index are uncompressed and the rest follows. Walking it accounts for every byte of the 10 test rounds and of 200 of 203 real ones (a `MatchReplay` folder read on two days):

```text
12 zero bytes, u32 n         stream list, n x 25 bytes: u32 ?, u32 ?, u32 name hash,
                             u32 first frame, u32 last frame, u8 0, u32 stream id
u32 n                        directory, n x 36 bytes: u32 id, u64 offset, u64 size, u64 size, 8 more
n x (u32 1, blocks)          each stream's opening snapshot, in list order
56 bytes                     main-stream descriptor: its id, offset and size
u32 1, blocks                main stream, to the end of the file
block                        "CMPRV002" (stored as a u64), u32 raw size, u32 packed size, zstd frame
```

Decompressed, each snapshot is a u64 length and the snapshot. The main stream holds every stream's records, each a u32 frame, u32 size, u32 0 and the payload, so every packet belongs to a frame and the index gives its time. Reading only the header needs no decompression for Y8S4+ (`ReadMode::Header`); before Y8S4 everything is one zstd stream.

- **Streams.** Most rounds have 10 (8 to 11 seen). `state` holds the clock, kill feed, health and picks, every packet this parser decodes; `movement` holds every movement message. The rest are known only by hash. `movement` and `state` have a record at nearly every update, 35 ms apart.
- **Recording ids.** Stream ids come from one counter per run of the game. A round takes its main id (`recordingId`) and one per stream, and the next recording starts right after, so a skipped id is a recording that was started and never saved. The folder name ends in the same run's process id.
- **Unfinished files.** 3 of the 203 real rounds end on a block whose packed size is 0xFFFFFFFF, the game's compressor having failed on a 5 to 11 MB block. The main stream was never written and the directory holds uninitialized memory, but the frame index and snapshots survive, so the header and players still read.
- **Rates.** The index rate follows whoever recorded. Spectator recordings (the Y11S3 test rounds) index a steady 29.4 frames a second; a player's own recording indexes every rendered frame, about 300 a second on the PC checked, 0.1 to 66 ms apart. Records arrive about 28 times a second either way. The 200 to 260 a second seen in Y8 and Y9 replays fits the second kind.
- **Temporary files.** A current install has an empty `DissectTmp` folder next to `MatchReplay`, and the process id and stream id in the reported `.tmprec` names match what round files hold. No `.tmprec` file was available, so their contents are unchecked.
- **Hashes are names.** Every property, field and class hash in the stream is the CRC-32 of the game's name for it, stored little-endian: `crc32("Health")` is `0xC9762625`, written `25 26 76 c9`. Guessing a name and hashing it tests what a field is. Names found this way include `ProfileType` (`05c7b949`, the relation to the recorder), `SquadStatus` (`af6bb287`, the party role), `ClearanceLevelText`, `TeamColor`, `HeroTeam`, `BanState`, `HasLeft`, `MatchKills`, `PlayerPlatform`, `PlayerSlotType` (1 while a player is in the slot), `LocationName` (the spawn voted for), `TimerInSeconds`, `TimerInMilliseconds` and `TimerState` on the clock object, `IsDefuserStarted`, `DefuserInteractionType`, `DefuserInteractionRemainingTime` and `HasDefuser` (who carries the defuser; not output yet).
- **Stray records.** Binary data between record runs can read as records. In one real round such a "record" covered a team object's first record and moved its properties to another object. Two rules reject them: an array record claiming an index of 65536 or more (real ones reach 64), and any record that does not name its object yet covers records that do (a `23` or `1b` and what follows) ending exactly where it ends.

The index is wall-clock accurate: in Y11S3, `starttime` plus the index duration lands within 2 ms of `endtime`. The header `datetime` is the recording PC's local time, not UTC.

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

Set `R6_MATCH_REPLAY` to a game `MatchReplay` folder to also check real match folders: file and folder names agree with the headers, every round lands in one session, and nothing is found twice. The folder changes as matches are played, so these tests check what holds for any such folder.
