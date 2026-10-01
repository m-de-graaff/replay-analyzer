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
| `loadouts` | Guns and gadgets per player, once per operator played, so attacker swaps get their own entry: `weapons` and `gadgets` are item ids, and kill `weapon` ids match them. From Y11S3 the entry of the operator a player spawned with adds `primary` and `secondary` (name, attachments, ammunition) and `ability` and `gadget` (how many the player had, and when each was used). See [Loadouts](#loadouts). | Y8S1+ (detail Y11S3+) |
| `health` | Every health change in the action phase, with the clock. From Y11S3 each adds `maxHealth`, `overheal` (health above the maximum) and, when the change was not damage, its `cause`. See [Health and damage](#health-and-damage). | Y8S1+ (detail Y11S3+) |
| `lifeEvents` | Downs (DBNO) and revives. From Y11S3 a down names who dealt it (`by`) and how it ended (`outcome`, `finishedBy`); a revive names who gave it (`by`, `self`) and the `health` the player got up with. | Y8S1+ (detail Y11S3+) |
| `hits` | Every hit a player took: damage, damage type, direction, and the attacker where one can be named. | Y11S3+ |
| `timelineEvents` | The round's own timeline: kills, team kills, deaths with no killer, downs and revives, each with both players. | Y11S3+ |
| `heals`, `plates` | Each heal (receiver, amount, kind, giver) and each Rook plate picked up. | Y11S3+ |
| `effects` | Status effects per player with start and duration: poison, burning, tracking, scans and more. | Y11S3+ |
| `friendlyFire` | Players whose reverse friendly fire was on, with when it turned on and off. | Y11S3+ |
| `flashes` | When the recording player was flashed (player recordings only). | Y11S3+ |
| `players[].maxHealth` | The operator's maximum health: 100, 110 or 125. Replays hold no armor rating; this is what the game has in its place. | Y11S3+ |
| `matchFeedback[].teamKill`, `finish`, `downedBy`, `victimEffects` | On kills: the victim was a teammate; the victim was down, and who downed them; the status effects the victim was under. | Y11S3+ |
| `observation` | Drone and camera sessions: who, whose device, tool, phase and duration. | Y8S1+ |
| `stats[]` | Adds `damageTaken`, `downs`, `revives`, `droneSeconds` and `cameraSeconds`, summed per match too. From Y11S3 also `damageDealt` and `teamDamage` (estimates, see [Health and damage](#health-and-damage)), `downsDealt`, `finishes`, `revivesGiven`, `teamKills`, `healingGiven` and `healingReceived`. | Y8S1+ (detail Y11S3+) |
| `weaponActivity`, `shots`, `bulletHits`, `throws`, `meleeHits`, `shieldActions` | What players held, fired, reloaded, threw and struck. See [Weapons and shooting](#weapons-and-shooting). | Y11S3+ |
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

A match folder adds `loadoutChanges` (Y11S3+): per player, each round whose loadout differs from the player's previous round on the same side, as `username`, `round`, `previousRound`, `side` and `changes[]` of `{field, from, to}`. `field` is `operator`, `primary`, `secondary`, `gadget`, an attachment such as `primary.sight`, or `ability` for the operators who choose a gadget in that slot; `from` and `to` are `{id, name}` (null for an empty slot). Attachments are compared only on the same gun.

A match folder adds `analytics`: per team attack and defense records, rounds started a player down, plants, disables and prep swaps (late ones counted apart); per site the defense win rate overall and per team; per attacker spawn and team the pick and round win rates; per operator and team rounds, win rate, kills, deaths, headshots and how often it was swapped to; and a count of rounds per end reason and winning side. `summary.rounds[]` also gains `winProbability`, `endReason` and `playersAtStart`.

Every round also says where it came from and how far it can be trusted:

| Key | What it holds |
|---|---|
| `replay.file` | Path, size, modified time and SHA-256, for deduplication and "already imported" checks. |
| `replay.format` | The `dissect` prelude: format version (7 before Y8S4, 8 since), layout, declared frame count and header property count. A format version, label or layout not seen before is flagged in `decodeStatus.header`. |
| `replay.version` | `Y11S3_Alpha04` split into season, year, season number and branch, plus the build number (`code`). |
| `replay.parser` | Parser version and the decoder profile and revision chosen for the build. A decoding fix bumps the revision of the profiles it touches, so stored rounds with an older `(decoder, decoderRevision)` than `--decoders` lists for their build are the ones to re-parse. `untestedBuild` flags builds newer than any the decoders were checked against (9883691, 9901603 and 9918362, all `Y11S3_Alpha04`). |
| `replay.container` | Y8S4+: the streams the round was recorded in (id, name hash, role where known, frames covered, snapshot blocks, record count), the compressed blocks, `recordingId`, and `complete`, false when the game did not finish writing the file. See [File format notes](#file-format-notes). |
| `decodeStatus` | Per field (`container`, `players`, `kills`, `scoreboard`, `bans`, `health`, `result`, `timing`, ...): `decoded`, `inferred`, `partial`, `missing`, `notInVersion` or `skipped`, with a count and warnings such as how many packets failed. `trusted` is false when any field is partial or missing. What a replay never records (text feed messages, from Y9S1) is `notInVersion`, and a player who never spawned has no body to link, so `trusted` marks faults: a field left unread (`skipped`: a partial read, a custom game's party) does not lower it either. Of the 167 real rounds in one `MatchReplay` folder, the 6 untrusted ones were 3 unfinished files and 3 rounds that were not played out (no result). |
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
| `teams` | Name, final score, starting side, and players with profile id, level and platform: every player seen on the team in any round, so one who joined late is listed and a team can list more than five. Matchmaking names the teams `YOUR TEAM` and `ENEMY TEAM` from the recorder's side; custom games carry the real names. |
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

- Before Y11S3, replays give a player's health and never who caused a change, so there is damage taken but no damage dealt. From Y11S3 every hit is read, and its attacker is named or inferred; see [Health and damage](#health-and-damage). `bulletHits` separately matches each bullet hit to the shot that made it; see [Weapons and shooting](#weapons-and-shooting).
- Before Y11S3, replays sometimes skip the last health update before a kill, which makes `damageTaken` a lower bound.
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

## Loadouts

From Y11S3, the `loadouts[]` entry of the operator a player spawned with carries the whole loadout. Entries of operators an attacker swapped away from keep `weapons` and `gadgets` only.

| Key | What it holds |
|---|---|
| `primary`, `secondary` | `id` (the item id kill feed `weapon` ids use) with its `name`, the gun's `asset` id, and its attachments `sight`, `barrel`, `grip`, `underbarrel` and `magazine`, each `{id, name}` and absent when the gun has no such slot. An attachment whose name is worked out rather than read has `inferred: true`, and a sight has `magnified` when that is known (see below). `ammo` holds `magazineSize`, the rounds in the gun and in reserve at spawn (`start`) and at the end (`end`), and `fired`, every drop of that total added up. A shield operator's `primary` is the shield: `shield: true`, no asset, attachments or ammunition. |
| `ability`, `gadget` | `id` and `name`, the count at spawn (`start`) and at the end (`end`), the most the slot holds (`max`), `used` and `gained` (every drop and every rise of the count added up, so `start - used + gained` is `end`), and `uses[]`: each drop with the `count` left after it and the `time`, `phase`, `elapsed` and `recordingTime` health changes carry. Abilities that refill over time have `regenerates: true` and no `max`. |

`weapons` and `gadgets` of that entry are filled from the same slots: guns, primary first, then the ability and the gadget.

Where the data lives:

- **The HUD, in the `state` stream.** Each controller links a `PlayerLoadoutViewModel` with one field per slot: primary, secondary, ability, gadget and drone. A slot is a `WeaponViewModel`, whose `WeaponAmmoViewModel` carries `TotalAmmo` and `MagazineSize`, or a `GadgetViewModel` with `Ammo` and `MaxAmmo`, and links the item id. The field says what an item is, not its class: a shield sits in the primary field as a gadget, a launcher (Grim, Hibana) in the ability field as a weapon with its ammunition as the count, and Striker and Sentry carry a gadget in the ability field. An attacker's swap in prep links new slots, so the last link of each field counts. A gadget's count is set up before its owner spawns; the count in force when `MaxAmmo` first shows is the start.
- **Entities, in the `movement` stream.** Each entity is created with a descriptor (`617385fe`) that lists its asset and its slots. A body's slots name the assets it carries; a gun's slots name its attachments. A body belongs to the player whose `playerid` ends one of its movement messages, which also links the recording player's own body. A gun belongs to the body whose slot holds its asset; when two players carry the same gun, the body id inside the gun's first movement messages tells them apart.

Names come from a lookup table of 200 item ids, each matched to its name by which operators carry it.

Attachment names come from a second table, built from 174 rounds (971 ids):

- **Read from the file:** an id for which no attachment entity is ever created is an empty slot: no laser, no grip, a pistol's iron sights. An underbarrel that does have an entity is a laser. These names carry no `inferred`.
- **Inferred:** every attachment entity can carry a skin, and a skin id is shared by every gun's copy of one attachment model, which groups per-gun ids into models. Within a gun's id block the barrels come in one order (suppressor, flash hider, compensator, muzzle brake, extended barrel), which names the barrel models; grips are named by which guns offer them. These carry `inferred: true`: 239 barrel ids, 54 grip ids (vertical or angled; the horizontal grip is not identified), 5 lasers.
- **Sights:** only iron sights and one magnified model (`Magnified 2.5x`, inferred) have names. Which 1x sight is the red dot, holographic or reflex is not known, so most sights have an `id` and `magnified` (false for a model that also fits guns limited to 1x sights) and no name. Magazines have no names.

No in-game check has confirmed an inferred name.

What is and is not recorded:

- **Attachment ids are options of one gun.** The same sight on two guns has two ids, and the file names none of them; the names above come from a table. Zoom is not written: it follows from the sight's id.
- **Abilities without a count show no use.** Skopos's shells never show a `MaxAmmo`, so they have an `id` and nothing else. An attacker who never spawned (a round that ended in prep, or a player gone before action) has slots without counts and no body, so no attachments.
- **A count dropping means used, not deployed.** The count is what the HUD shows, and anything that lowers it is a use; the file does not say whether the gadget ended up placed. A rise is counted in `gained`.
- **Attackers' prep drones are not counted.** The drone slot is not read.
- **Skins, charms, headgear and uniforms** sit in the same descriptors; `players[].cosmetics` carries them (see [Cosmetics](#cosmetics)).
- **Only full reads decode this**, and only files the game finished writing: the HUD settles as the round goes on.

Of 1,661 player-rounds in a real folder of 167 rounds, the 1,631 in finished files all have a loadout. Every gun has its attachments (1,577 primaries and 1,626 secondaries; 49 primaries are shields) except for 5 players who never spawned. 1,621 abilities and 1,626 gadgets have counts; the rest are Skopos's (5) and those 5 players'. `decodeStatus.loadouts` counts the players whose loadout was found and warns about each body or gun that could not be linked.

## Weapons and shooting

From Y11S3, full reads of files the game finished writing carry what players did with their weapons. Every event has `time`, `phase`, `elapsed` and `recordingTime` like the kill feed.

| Key | What it holds | How |
|---|---|---|
| `weaponActivity[]` | Per player: `held` (each change of the item in hand), `swaps`, `fired` (each drop of a gun's ammunition, with what was left), `reloads` and `atDeath`. | Decoded; swaps and reload outcomes derived |
| `shots[]` | Every shot: who, the gun, the muzzle position, the direction and the distance to what it struck. | Decoded |
| `bulletHits[]` | Every bullet that struck a player: victim, position, damage, limb or not, and the result. The shooter is the shot whose ray fits. | Decoded; shooter inferred |
| `throws[]` | Every grenade, thrown gadget, drone throw and launcher projectile: who, what, from where, the flight path and where it ended. | Decoded; direction derived |
| `meleeHits[]` | Every melee hit on a barricade or a destructible part of the map: who, what, which hit on it, and whether it broke. A hit with a shield in hand is a bash. | Decoded |
| `shieldActions[]` | Held shields raised, stowed and dropped, and each extension of Montagne's shield. | Decoded |

What the file does not hold, after searching every stream:

- **Fire mode.** No property of the HUD or of a gun changes with it, and none of about 186,000 guessed names for one exists in the file. Burst against full-auto can only be guessed from the spacing of `shots`.
- **The body part of a hit.** A hit says limb or not. A headshot is known only for a kill, from the kill feed, so a headshot rate over all hits cannot be had.
- **Who fired the bullet that hit.** Neither the hit nor the damage names the shooter; `shooter` is matched by ray and says so (`shooterSource`).
- **Melee swings and melee hits on players.** See [Melee and shields](#melee-and-shields).
- **A detonation.** A thrown object is removed; nothing says it went off.

### Weapon handling

`weaponActivity[]` has one entry per player:

| Key | What it holds |
|---|---|
| `held[]` | Each change of what is in the player's hands: `item` is `primary`, `secondary`, `ability`, `gadget`, `drone` or `none`, with the `id` and `name` `loadouts` gives that slot. `none` is the hands empty: between two items, on a drone or camera, down or dead. |
| `swaps[]` | One item put away for another: `from`, `to`, and `duration`, the seconds the hands were empty in between (0 when the game went straight from one to the other). The time is the moment the old item left the hands. Hands empty for more than 3 seconds are not a swap. |
| `fired[]` | Each drop of a gun's ammunition: `slot`, `rounds` (several when more than one shot fell between two updates), `magazine` (rounds left in the gun, the chambered one included), `reserve`, and `aiming` (the HUD's `IsAiming`). A launcher in the ability slot counts its rounds here too. |
| `reloads[]` | `slot`, `outcome`, `duration`, `magazineBefore`, `magazineAfter` and `reserveAfter`, timed at the start. `completed`: the gun holds more than before. `cancelled`: the reload ended and the gun gained nothing; when the magazine was already out, `magazineAfter` is the round left in the chamber. `unfinished`: the reload never ended, because the player died or the recording stopped. |
| `atDeath` | For a player who was killed: `held` with its `id` and `name`, `magazine`, `reloading`, and `swappingFrom` and `swappingTo` when a swap was in progress. A player downed first has empty hands from then on, so this is read at the down. |

Where it lives: the `PlayerLoadoutViewModel` each controller links in the `state` stream (see [Loadouts](#loadouts)).

- `ActiveReticleType` on the view is the item in hand: 0 nothing, 1 the drone, otherwise the number the slot itself carries in `EquippedWeaponType` (2 primary, 3 secondary, 4 ability, 5 gadget).
- A gun slot's `WeaponAmmoViewModel` holds `AmmoInWeapon` (the magazine and the chambered round, so a 30-round gun reads 31 after a reload with a round in the chamber), `AmmoLeft` (the reserve, written when a reload moves ammunition) and `TotalAmmo`, their sum. A gun with a launcher under it (Nomad's, Kali's) links a second one for the launcher through another field.
- `IsReloading` on the slot is 1 for the length of a reload. Reloading a gun that is not empty is two pulses a few frames apart: the magazine comes out (the gun keeps 1, the reserve takes the rest), then the new one goes in. They are given as one reload. A shotgun loads shell by shell inside one pulse. Pulses under 0.35 s that move no ammunition are left out: the game raises the flag for a few frames on an empty gun.
- There is no flag for a swap or for a cancelled reload; both are read off the values above.

Checked on the 10 test rounds and on every fourth round of a real folder (52 rounds):

| | Test rounds | Real rounds |
|---|---:|---:|
| Players with activity | 100 of 100 | 527 of 527 |
| Guns whose `fired` adds up to the loadout's `ammo.fired` | 193 of 193 | 1,030 of 1,030 |
| Ammunition drops with that gun in hand | 4,862 of 4,868 | 16,400 of 16,420 |
| Kills with a carried gun, with that gun in the killer's hands | 60 of 62 | 361 of 362 |
| Gadget uses with the gadget in hand | 105 of 105 | 376 of 376 |
| Reloads completed | 512 of 566 | 1,598 of 1,789 |

Over those rounds and every eighth real round, each of 12,180 consecutive drops without a reload in between leaves the magazine exactly the rounds fired lower.

- The knife is not a HUD slot, so `held` never says melee: the gun stays in hand through a swing.
- The controller's `a4dc8dd4`, output as `weaponReady`, is `CanFire`. It does not drop on reloads or on swaps between guns (it stayed 1 through 552 of 565 reloads); it is 0 while a drone is in hand and mostly while a gadget is.
- An ability used without being held (159 of 196 ability uses in the test rounds had it in hand) is not a fault: some abilities are triggered, not held.

### Shots and bullet hits

`shots` lists every shot of the round and `bulletHits` every bullet that struck a player's body.

| Key | What it holds |
|---|---|
| `shots[].username` | Who fired. Absent when the gun could not be linked to a player (`decodeStatus.shots` warns). |
| `shots[].slot`, `weapon` | The loadout slot that fired: `primary` or `secondary` for a gun, `ability` or `gadget` for a device that shoots (Twitch's drone, a bulletproof camera). `weapon` is the item in it as `{id, name}`, the id `loadouts` and the kill feed use. |
| `shots[].origin`, `direction` | The muzzle in map coordinates (metres, z up) and a unit vector. |
| `shots[].distance`, `eyeDistance` | Metres to what the bullet struck first, from the muzzle and from the shooter's eye. |
| `bulletHits[].victim`, `position` | The player struck and where, in map coordinates. |
| `bulletHits[].damage`, `limb`, `result` | Health taken, whether an arm or leg was struck, and `alive`, `down` or `dead` after it. Absent for a bullet in a body that was already down or dead. |
| `bulletHits[].shooter`, `shooterSource`, `shot` | Who fired, how that is known (`ray`), and the index of the shot in `shots`. Absent when no shot fits. |

How they are found:

- **Shots.** In the `movement` stream a gun's `607385fe` update ends in a list of events. Event `06` (63 bytes) is the gun firing: muzzle position, direction, and the distance to the impact from the eye and from the muzzle. The game repeats the event in every update while an automatic gun fires, and a player's own recording repeats each about 16 times, so a shot is a run of events of one gun with the same direction and distance. A shotgun writes one event per shell. The gun is linked to the body that carries it and the body to its player, as loadouts are.
- **Hits.** A bullet striking a body spawns an effect in the `FXChannel` stream (`f5ee6a3d`): asset `d5 6d 41 58`, the body as its target, and where it struck in parameter `56 95 b5 31`.
- **Damage.** The update of the body that took the damage ends in a 24-byte block: the health left as a share of the maximum, a damage multiplier (1.0, about 0.75 for a limb), the state after (1 alive, 3 down, 4 dead) and the damage type (0 for a bullet). `damage` is the drop of that share since the body's block before, times the player's maximum health.
- **Shooter.** Nothing names it. `shooter` is the player whose shot passes within 0.6 m of the hit at about that time.

| | Test rounds (10) | Real rounds (167) |
|---|---:|---:|
| Shots | 5,375 | 59,326 |
| Rounds the HUD counted down on the same guns | 5,348 for 5,335 shots | |
| Hits that did damage, with a shooter | 184 of 187 | 3,673 of 3,722 |
| Kills whose lethal hit is the feed's killer's | 59 of 66 | 1,109 of 1,183 |
| Kills whose lethal hit is another player's | 2 | 11 |
| Kills with no bullet hit (explosive, melee, bleed-out) | 5 | 63 |

In the test rounds 133 of 155 guns that fired agree exactly with the HUD's count, and 152 of 184 damages equal a change in `health` at that moment.

- Two players firing along the same line at the same moment cannot be told apart.
- Health regained between two hits is not seen: a hit after healing reads low, or has no `damage`.
- A shell is one shot, and of its pellets in one body only the first carries the damage.
- Launcher abilities drop ammunition without a fire event; their rounds are in `throws`.
- Fire events and damage blocks are found by their shape at the end of a message; what precedes them in the message is not decoded.
- Bullets that hit walls are in the `DecalChannel` stream and are not output, so a hit through a wall is not marked.

### Throws and launches

`throws` lists every grenade, thrown gadget, drone throw and launcher projectile, in the order they were released.

| Key | What it holds |
|---|---|
| `username` | Who threw or fired it. |
| `slot` | `ability`, `gadget` or `drone`: the loadout slot it came from. Absent for the ammunition of launchers and for sub-munitions. |
| `asset` | The object's asset id. |
| `id`, `name` | The item, as `loadouts` names that slot. Launcher ammunition has only a `name`, from a table, marked `inferred: true`. |
| `subMunition` | `true` for what another object let go: Candela charges, cluster charge pucks, Kawan swarms. |
| `origin` | Where it left the hand or muzzle, in map coordinates (metres, z up). |
| `direction`, `speed` | Unit vector and metres a second of the first full step of the flight. |
| `path` | `[seconds since release, x, y, z]`, thinned to at most 60 points. |
| `end`, `flightTime` | Where the flight stopped and how long it took. |
| `ended`, `endedAfter` | `deleted` (the game removed the object) or `returned` (taken back into its pool), and seconds since the release. Absent when it was still there at the end of the recording. |

How it is found: the `movement` stream creates each object with a `617385fe` message that lists its component classes. Everything a player can let go of has a component of class `8490f616` in its `607385fe` updates: a `u8` mask, then a `u16` (bit 01), the owner's `playerid` (02), the owner's alliance (04) and a flag (08) that is 1 once the object is released and 0 when it is taken back. A throw is the update that sets the flag to 1. That message carries the release position, and each update after it one position, about 30 a second. The game writes no velocity, so direction and speed come from the second and third position. The item is the slot of the thrower's body (`PrimaryGadget`, `SecondaryGadget`, `Drone`) whose asset equals the object's. Launcher ammunition is in no slot and is named by a table of 20 assets, each assigned to the launcher of the only operator who fires it. A drone is driven straight after it lands, so its path is cut at the landing. Objects are pooled and created under the map, so the position an object is created at is not where it was thrown from.

| | Test rounds (10) | Real rounds (167) |
|---|---:|---:|
| Throws | 348 | 3,802 |
| Count drops of hand-thrown items with a release at most 1.1 s before | 168 of 168 | 2,362 of 2,366 |
| Count drops of launchers with a release | 48 of 48 | 367 of 371 |

The count drops 0.38 s after the release (median). Grenades in free flight fit a parabola of 9.3 m/s�.

- `ended` is the object being removed. For a grenade that is within a frame or two of it going off; for a gadget it may be minutes later.
- No release was found for Thatcher's EMP grenade.
- Gadgets that are placed, not thrown (barbed wire, deployable shields, breach charges, cameras on walls), are another class (`4c60869a`) and are not covered.
- A few pooled sub-munitions are released without ever being given an owner (8 in the real rounds); they are left out and counted in `decodeStatus.throws`.
- Both of Capitao's bolts share one name, as do both of Zofia's grenades: which asset is which type is not known. Four assets seen in real rounds have no name.
- Drones report the 15.9 m/s they leave the hand with.
- Attackers start prep already on their drones, so prep has no drone throws.

### Melee and shields

`meleeHits` lists every melee hit on a barricade or a destructible part of the map; `shieldActions` lists what players did with a held shield. Both come from the `movement` stream.

Anything that can be damaged carries a damage list in its update messages: a `u32` count, then one entry per hit. An entry is a kind byte (0 melee, 1 bullet), the hit point in the object's own space, the body that did it, a damage id and a list of impacts; an entry of the single byte `fe` says the object is destroyed. A melee hit is an entry with damage id 34118943362.

| Key | What it holds |
|---|---|
| `meleeHits[].username` | The player whose body the entry names. |
| `target`, `object` | `barricade`, `mapObject` (a wall, hatch or prop: an id that is the same in every round on the map) or `entity` (another entity that takes damage), and the id of what was hit. The file does not say whether a map object is a wall, a hatch or a prop. |
| `hit` | Which melee hit on that object it is, from 1, starting over once it broke. The game's own counter also counts bullets. |
| `broke` | The hit destroyed the barricade. A barricade takes three hits; a reinforced one took ten. |
| `position` | Where the hit landed, in map coordinates (`point`, relative to the object, when the object's place is not known). |
| `withShield` | The player had a shield in hand, so the hit is a shield bash. |
| `knifeSeen` | The knife was seen in the player's hand for the swing. The game writes that for about 28% of hits, so its absence means nothing. |
| `shieldActions[].action` | `raise` when a held shield goes in hand, `stow` when it is put away, `drop` when it leaves the hand without being put away (taken to be the holder dying). `extend` is one extension of Montagne's shield, timed at its start, with `extendTime` (seconds until fully extended), `extendedFor`, `retractTime` and `duration`; `duration` is absent when the shield never came back. |

A held shield is an entity of its own, attached to a socket of its holder's body: in hand, or on the back. Montagne's also carries a state (normal, extending, extended, retracting) in component `36638d75`. Covered: Montagne, Blitz, Fuze, Blackbeard and Clash.

| | Test rounds (10) | Real rounds (167) |
|---|---:|---:|
| Melee hits | 79 | 1,136 |
| On a barricade / map object / entity | 51 / 27 / 1 | 755 / 380 / 1 |
| Barricades broken | 10 | 117 |
| Shield bashes | 15 | 68 |
| Raise / stow / drop | 28 / 24 / 4 | 135 / 98 / 28 |
| Montagne extensions | 8 | 49 |

Not recorded, or not decoded:

- **Melee kills** need nothing new: the kill feed gives them weapon id 3099101909 (6 of 1,249 kills in the real rounds). The id is carried by no loadout; that it means melee is inferred from the killers' knives being out at 4 of those kills.
- **Melee hits on players that do not kill** are not in the replay: bodies carry no such entry.
- **Swings that hit nothing.** The knife is seen in hand for about a third of swings, so no list of swings is given.
- **A ballistic shield held up in guard or aimed over** is not in what was decoded.
- **Blitz's flash** is the ability's `uses[]` in `loadouts`; who it blinded is not recorded.
- **Clash's, Blackbeard's and Osa's shield states** are not decoded; Osa gets no shield actions.
- Blitz's and Fuze's shields and the ten-hit barricades occur only in real rounds, so the test replays do not cover them.

## Health and damage

From Y11S3, three streams hold what happened to a player's health. All of it needs a full read. Numbers below are from 164 rounds of a real `MatchReplay` folder and the 10 test rounds.

### Downs, kills and revives

The game writes the finished round's timeline into the file (`TimelineChannel`): every kill, team kill, down and revive, each naming both players. `timelineEvents[]` carries it as `type` (`Kill`, `TeamKill`, `Death`, `Down`, `Revive`), `username` (the victim, or the player revived), `by`, and for kills `weapon` and `headshot`. It parsed to its last byte in every finished round.

It is joined onto the events the parser already had:

| Key | What it holds |
|---|---|
| `lifeEvents[].by` | Who downed the player, or who revived them. All 32 revives and 314 of 319 downs name someone. |
| `lifeEvents[].self` | The player revived themselves (6 of 32: Doc's and Finka's own abilities). |
| `lifeEvents[].outcome` | For a down: `Finished` (with `finishedBy`), `Revived`, `DownAtEnd`, or `Died` when the player died with no killer named. |
| `matchFeedback[].finish`, `downedBy` | The kill ended a down, and who dealt that down. Where the scoreboard credits the kill to another player (`creditedTo`), it is always the downer. |
| `matchFeedback[].teamKill` | The timeline lists the kill as a team kill (12 in the real folder, the same 12 whose killer and victim share a team). |

- **No bleed-out has been seen.** No player bled out in 174 rounds, so what one looks like is untested. It should read as `Died`. The bleed-out timer itself (`DBNOProgress`) is in the file and not output.
- **A kill that ends the round is not a down.** The HUD passes through the down state as the last player of a team dies; those are dropped (23 in the real folder).
- **Two fixes to older output.** A death written through the down state used to be reported as a revive: 143 "revives" in the real folder, 32 of them real. `damageTaken` used to miss the killing blow of most deaths and count an overheal wearing off as damage.

### Hits

Every hit on a player is in the movement stream, on the victim's body. `hits[]` carries:

| Key | What it holds | How |
|---|---|---|
| `username` | The victim. | Decoded |
| `damage` | Health the hit took. Absent on a hit that downs or kills: the game stores an overkill value there, not the health removed. | Decoded |
| `health`, `result` | Health after the hit, and `Alive`, `Down` or `Dead`. | Decoded |
| `type` | `{id, name}`: 0 `bullet`, 1 `melee`, 2 `explosion`, 9 `gas`, 36 `fire`. Other ids have no name yet. | Id decoded, names inferred from which operators were in the round |
| `multiplier` | Final damage over base damage: 1.0 or about 0.75 for bullets, lower for explosions with distance. | Inferred |
| `direction` | Where the hit came from, in eighths of a turn clockwise from where the victim was looking. | Inferred |
| `by`, `attackerSource` | The attacker, and how they were found (below). | See below |
| `distance` | Metres between attacker and victim. | Derived, only with `by` |

**The file never names the attacker of a hit.** `attackerSource` says where `by` comes from:

- `Timeline`: the hit downed or killed, and the timeline names who did it. Read, not inferred. 36.5% of hits.
- `Shot`: for bullets, the opponent whose ammunition dropped within about a tenth of a second and who was aiming closest to the victim. Checked against hits the timeline names: right for 1313 of 1350. 51.3% of hits.
- `Aim`: no opponent's ammunition dropped; the opponent aiming within 10 degrees of the victim. Right for 35 of 41. 3.6% of hits.
- Absent (8.7% of hits): nobody could be named. Explosions, gas, fire and gadget damage never get an inferred attacker.

`stats[].damageDealt` adds up the hits a player is named for, so it is an estimate and leaves unnamed hits out; hits on teammates go to `teamDamage`. A hit that downs or kills counts the health the victim had left. `damageTaken` adds up every hit on the player the same way.

- About 1.3% of the health losses the HUD shows have no hit found for them.
- **Body part is not recorded.** A kill's `headshot` is; for other hits a `multiplier` near 0.75 fits a limb hit, which is unconfirmed.
- **Whether a hit went through a wall is not recorded.**

### Health, overheal, armor and heals

The HUD's life object gives each player `Health`, `MaxHealth` and a life state. `health[]` adds `maxHealth`, and `overheal` when health is above it (the most seen is 20). An overheal wears off at 1 health a second; those steps have `cause: "decay"`.

- **Armor.** Replays hold no armor rating. Each operator has one `MaxHealth`, 100, 110 or 125 (`players[].maxHealth`), which is what the game uses in its place.
- **Plates.** A Rook plate raises `MaxHealth` and health by 25. `plates[]` lists each pickup with `username` and `by` (the team's Rook). When the pack was put down is the drop of Rook's ability count in `loadouts`.

`heals[]` lists every other rise in health: `username`, `amount`, `health` after it, `overheal`, `kind`, `by`, and `revive` when it got the player up from a down.

| `kind` | Told by | Seen |
|---|---|---|
| `finkaSurge` | The Finka boost appears in the player's effects in the same frame. +20. | 115 |
| `docStim` | Health jumps to the overheal limit as Doc's stim count drops. | 12 |
| `konaBurst`, `konaTick` | The Thunderbird station's effect is listed: +20 at once, then +1 about three times a second. | 12, 131 |

The kinds are inferred from those signs, and **the giver is inferred too**: the file links no heal to a player, so `by` is the team's Finka, Doc or Thunderbird. Two rises in the real folder fit none of the kinds and are left out. The test rounds have none of these operators, so heals and plates are checked on the real folder only.

### Status effects

Each player's HUD keeps a list of the effects on them. `effects[]` has one entry per stretch: `username`, `type` (the game's number), `name`, `buff` (true for the player's own or a teammate's ability), when it started and `seconds`. An effect names neither the gadget nor the player behind it.

The numbers are the game's; **the names are inferred** from which operator was in every round a type showed up in:

| Name | Type | Name | Type |
|---|---|---|---|
| `JackalTracked`, `JackalTracking` | 0, 33 | `GrimSwarm`, `GrimTracked` | 22, 23 |
| `LesionPoison` | 1 | `FenrirMine`, `FenrirFear` | 26, 27 |
| `FinkaSurge` | 2 | `TubaraoZoto` | 28 |
| `DokkaebiCall` | 3 | `DeimosMarked`, `DeimosTracking` | 30, 31 |
| `RookArmor` | 4 | `ThunderbirdHeal` | 35 |
| `AlibiTracked` | 5 | `ThornRazorbloom` | 39 |
| `ClashShock` | 6 | `SnakeRadar` | 43 |
| `EnemyJammer`, `FriendlyJammer` | 8, 34 | `NoorLance` | 46 |
| `LionScan` | 11 | `Burning` | 52 |
| `ProximityAlarm` | 13 | `MelusiBanshee` | 14 |

Types 15, 21, 25, 29, 37 and 51 occur and have no name. A kill's `victimEffects` lists the named effects the victim was under.

What the list does not hold:

- **Flashes of other players.** The flash flag is written for the player whose screen the recording shows, so `flashes[]` covers the recording player only and a spectator recording has none.
- **Concussion, Smoke's gas, electricity and traps** have no effect entry. Gas and fire still show as `hits` with their damage type.
- **A hacked phone** (Dokkaebi) was not identified; type 51 is the only candidate.

### Friendly fire

`friendlyFire[]` lists each player whose reverse friendly fire was on in the round: `activeAtStart` when it carried over from an earlier round, and `on` and `off` with their times. It is the game's own flag (`IsReverseFriendlyFireActive`). It turned on for the killer after all 12 team kills in the real folder, and 5 times with no kill, after damage to a teammate. Team damage itself is only in `hits`, where `teamDamage` counts the hits a teammate is named for.

`decodeStatus` adds `combat` (hits read) and `vitals` (players with a maximum health).

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
| `platform` | `pc`, `playstation` or `xbox`. See [Platform](#platform). | Y11S3+ | Inferred |
| `usesNickname` | `username` is a nickname the game shows in place of the player's own name. Left out when false. | Y11S3+ | Decoded |
| `renamedTo` | The name the game gave the player as the match ended. See [Names](#names). | Y11S3+ | Decoded |
| `cosmetics` | Uniform, headgear, operator card, weapon skins, charms and attachments, as asset ids. See [Cosmetics](#cosmetics). | Y11S3+ | Decoded |

Kill feed entries name their players' `profileID` and `targetProfileID`. From Y11S3 a kill can also carry `creditedTo`: the scoreboard credits a kill to the teammate who downed the victim when another player finished them, while the feed names the finisher. With those credits counted, the scoreboard's kill and death totals match the kill feed in every test round.

`weaponReady` lists each change of the controller's `CanFire` flag (`ready`, `phase`, `elapsed`). Attackers hold it at `false` through prep, on their drones, until their body spawns; in action it drops while a drone or gadget is in hand, not on reloads or swaps between guns. `weaponActivity` says what a player held (see [Weapon handling](#weapon-handling)).

`decodeStatus` adds `profileIds`, `recorder`, `entities`, `movement`, `party`, `weaponReady`, `platform`, `names` and `cosmetics`. `summary.teams[].players[]` gains `key`, `relation`, `party`, `platform`, `usesNickname` and `renamedTo`, and `summary.recording.party` lists who queued with the recorder.

`--players` reads every match folder under a folder (partially: players, relations, parties, platform and names) and prints a directory: per player the `key`, `platform`, every username used with first and last seen (`nickname` marks one the game showed in place of the player's own, `atMatchEnd` one the game gave as a match ended), matches with and against you, matches in your party, and `queueMate` (queued with you once, or on your team in two or more matches). `username` is the latest match's name, the one given at its end when the recording has it. `you` lists the recording accounts. A match imported twice (same `matchID`) counts once. The library equivalent is `PlayerDirectory::new(&summaries)`.

How the links are made (details in `src/entities.rs`):

- The stream is a tree of replicated objects. Besides `23`/`22` property records, `1b <parent> <field> <child>` and `1a <field> <child>` records hang child objects off a parent. Each player has one controller object under their team's object, holding the name, operator, profile id and the header `playerid`; the scoreboard, health, inventory and a profile object hang off it.
- The profile object carries the relation to the recorder (`05c7b949`: 1 opponent, 2 teammate, 3 teammate in the recorder's party, 5 the recorder) and the party role (`af6bb287`: 0, 1 member, 2 leader). Checked on Y8S1 ranked and quick matches (a clan-tagged five-stack, a duo with randoms), Y8S2 and Y9S1.
- Movement is sent apart from the tree. A player table links each body to a player explicitly, so the link does not depend on player order. In Y11S3 the table is a stream of its own (`aca4c435`): its snapshot is the table that opens the round and each frame record a table of what changed, per player the `playerid`, the body they move and what they look through (their drone or a camera). Every player of the round has an entry, the recorder included, so the recorder's own body is linked like any other. A player with an entry and no body never spawned: they left, or the recording ended first. `decodeStatus.movement` says so without calling it a fault. The entry layout is in `src/entities.rs`.
- A player the header does not list (it is written before late joiners arrive) is read from their pick packet and takes the `playerid` their controller holds. A player who reconnects comes back with a new `playerid` under the same profile id, so `key` holds across the match. The controller's `HasLeft` is not output: a seat that was filled again keeps it set.

### Platform

The controller's `PlayerPlatform` (`7dd4fc18`) is 0, 5 or 7 for every player in the 167 real rounds and the test match. Next to it sits `PlatformPlayerID` (`e7ceb836`), a console account id:

| `PlayerPlatform` | `platform` | Why |
|---|---|---|
| 0 | `pc` | The recording players, on PC, are 0, and every 0 has an empty `PlatformPlayerID`. |
| 5 | `playstation` | Every 5 has a `PlatformPlayerID` that is a random-looking 64-bit number, as PlayStation account ids are. |
| 7 | `xbox` | Every 7 has a `PlatformPlayerID` in the range Xbox ids are numbered in (`0x0009...`). |

So `pc` is confirmed for the recorder and the consoles are inferred from the shape of an id, which `decodeStatus.platform` says (`inferred`). Of 238 players in the real folder, 216 were on PC, 16 on PlayStation and 6 on Xbox, each keeping one value across rounds. Whether 5 and 7 mean one console generation or the family is not known, and any other value is left out and reported. `PlatformPlayerID` itself is not output. A seat whose player left is sent again with 12; the first value read is the player's. There is no input-device or cross-play flag: the profile's `PlatformFamilyIcon` is the same for everyone.

### Names

A player has one name in a round: the controller's, which the header's `playername` repeats and the profile object holds twice more. Some of them are not the player's own:

- The profile's `UsesNickname` (`feacf17b`) is 1 for players the game shows under a generated nickname (19 of 238 in the real folder). `usesNickname` marks them; their `profileID` is still their own.
- As a match ends, the game writes another name to the controllers of those players and of console players: `renamedTo`. For a nickname it reads as the player's own name, for a console player as the name on their platform, both judged by how the names look. PC players under their own name are never renamed.

The rename is the last thing in the last round's file, so only a recording that runs to the end of the match has it: 15 of the 30 real matches had one, and never before the final round. `summary` and `--players` carry it, so a nickname and the name behind it land on the same `key`. A seat taken over by another player also gets a new name, with a new profile; that is not a rename and is left out.

### Cosmetics

The movement stream creates each body, and each item a body carries, with a message listing named slots (the layout is at `Spawn` in `src/entities.rs`). Slot names are CRC-32 hashes like every other name, and each value is an asset id. `players[].cosmetics` holds:

| Key | Slot | |
|---|---|---|
| `uniform`, `headgear` | `Uniform`, `Headgear` on the body | |
| `operatorCard` | `OperatorCardBackground`, `OperatorCardPortrait` and up to three `...OperatorCardBadge` | |
| `mvpAnimation` | `MVPData` | The animation shown for the match's MVP. |
| `weapons[]` | `PrimaryWeapon` and `SecondaryWeapon` on the body give `slot` and `item`; the weapon's own message gives `skin` (`WeaponSkin`), `charm`, `attachmentSkin` (`WeaponAttachmentSkinSet`), `sight`, `barrel`, `grip` and `underbarrel` | A weapon without a charm holds a fixed id, which is left out. |
| `gadgets[]` | `PrimaryGadget`, `SecondaryGadget`, `TertiaryGadget` and `Drone`, with the item's `Skin` | Only those that carry a skin. |

That the ids are the player's own and not the operator's shows in the 167 real rounds: a player wore the same uniform and headgear on an operator in every round (1064 of 1064 player and operator pairs), while on 75 of the 76 operators played by more than one player the uniforms differed; a weapon held by several players had different skins on 213 of 214 weapons. The spectator-recorded test match carries them too.

- Ids only. Replays hold no names for assets, so which uniform or charm an id is needs a table from elsewhere. Equal ids are the same item, which is enough to count and compare.
- A body exists once the player spawns: defenders from the start, attackers at the end of prep, so a partial read has the defenders only.
- A weapon is matched to its player by its asset; when two players carry the same one, by the list of what each body carries. 4 of 3216 weapons came out with only their `item`: no skin set, or no telling whose it was.
- No slot says a set is an elite: `CharacterSet` and `WeaponSet` exist and are always 0.
- The weapon `item` ids are not the ids `loadouts` uses.

Not found:

- **Other players' parties.** Only the recorder's party is recorded: `SquadStatus` is set for the recorder and their party and for nobody else, and no property of any player object, nor any list on the team objects, holds a value the recorder shares only with their party. Premades among other players can only be guessed from match history (`--players`).

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

- **Streams.** Most rounds have 10 (8 to 11 seen). `state` holds the clock, kill feed, health and picks; `movement` holds every entity message. Their hashes are the CRC-32 of the game's names: `HUDChannel` (`a98fdd0b`, `state`), `EntityChannel` (`20a5c4e3`, `movement`), `ControllerChannel` (`aca4c435`, the player table), `FXChannel` (`f5ee6a3d`, effects), `DecalChannel` (`5f87976f`, bullet holes and marks), `SoundChannel` (`63fe54d3`), `MarkerChannel` (`26b9c2c1`, pings), `TimelineChannel` (`eee42d83`, a log of kills and downs) and `WorldChannel` (`e3f6781c`); `be5e4267` is always empty and unnamed. Only the first four and `TimelineChannel` are read. `movement` and `state` have a record at nearly every update, 35 ms apart.
- **Recording ids.** Stream ids come from one counter per run of the game. A round takes its main id (`recordingId`) and one per stream, and the next recording starts right after, so a skipped id is a recording that was started and never saved. The folder name ends in the same run's process id.
- **Unfinished files.** 3 of the 203 real rounds end on a block whose packed size is 0xFFFFFFFF, the game's compressor having failed on a 5 to 11 MB block. The main stream was never written and the directory holds uninitialized memory, but the frame index and snapshots survive, so the header and players still read.
- **Rates.** The index rate follows whoever recorded. Spectator recordings (the Y11S3 test rounds) index a steady 29.4 frames a second; a player's own recording indexes every rendered frame, about 300 a second on the PC checked, 0.1 to 66 ms apart. Records arrive about 28 times a second either way. The 200 to 260 a second seen in Y8 and Y9 replays fits the second kind.
- **Temporary files.** A current install has an empty `DissectTmp` folder next to `MatchReplay`, and the process id and stream id in the reported `.tmprec` names match what round files hold. No `.tmprec` file was available, so their contents are unchecked.
- **Hashes are names.** Every property, field and class hash in the stream is the CRC-32 of the game's name for it, stored little-endian: `crc32("Health")` is `0xC9762625`, written `25 26 76 c9`. Guessing a name and hashing it tests what a field is. Names found this way include `ProfileType` (`05c7b949`, the relation to the recorder), `SquadStatus` (`af6bb287`, the party role), `ClearanceLevelText`, `TeamColor`, `HeroTeam`, `BanState`, `HasLeft`, `MatchKills`, `PlayerPlatform`, `PlatformPlayerID`, `OnlinePlayerID` (the header's `playerid`), `UsesNickname`, `IsBot`, `PlayerSlotType` (1 while a player is in the slot), the cosmetic slots (`Uniform`, `Headgear`, `WeaponSkin`, `Charm` and the rest), `LocationName` (the spawn voted for), `TimerInSeconds`, `TimerInMilliseconds` and `TimerState` on the clock object, `IsDefuserStarted`, `DefuserInteractionType`, `DefuserInteractionRemainingTime` and `HasDefuser` (who carries the defuser; not output yet).
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

Set `R6_MATCH_REPLAY` to a game `MatchReplay` folder to also check real match folders: file and folder names agree with the headers, every round lands in one session, nothing is found twice, and loadout counts add up. The folder changes as matches are played, so these tests check what holds for any such folder.
