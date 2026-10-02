# replay-analyzer

Parses Rainbow Six Siege match replays (`.rec` files) into JSON.

```sh
replay-analyzer R01.rec                 # one round as JSON
replay-analyzer Match-2024-05-04/ -o match.json   # every round in a match folder, plus totals
replay-analyzer R01.rec --info          # short header summary
replay-analyzer R01.rec --partial       # header and players only (faster)
replay-analyzer R01.rec --census        # also count every packet and field seen
replay-analyzer R01.rec --movement      # also every player's position, view and posture at every update
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
| `observation` | Drone and camera sessions: who, whose device, tool, phase and duration. From Y11S3 `device` is the entity id of the drone or camera, as `drones` and `cameras` list it. | Y8S1+ (`device` Y11S3+) |
| `stats[]` | Adds `damageTaken`, `downs`, `revives`, `droneSeconds` and `cameraSeconds`, summed per match too. From Y11S3 also `damageDealt` and `teamDamage` (estimates, see [Health and damage](#health-and-damage)), `downsDealt`, `finishes`, `revivesGiven`, `teamKills`, `healingGiven` and `healingReceived`, and `pings`, `timesSpotted`, `spotsMade`, `spotAssists`, `devicesDestroyed`, `dronesLost`, `timesJammed` and `objectiveFound` (see [Pings, spotting and information](#pings-spotting-and-information); `spotsMade`, `spotAssists` and `devicesDestroyed` are inferred), and `gadgetsDeployed`, `gadgetsDestroyed`, `gadgetsLost`, `reinforcements`, `barricades`, `breaches`, `breachesOpened` and `trapsTriggered` (see [What is joined onto other events](#what-is-joined-onto-other-events)). | Y8S1+ (detail Y11S3+) |
| `weaponActivity`, `shots`, `bulletHits`, `throws`, `meleeHits`, `shieldActions` | What players held, fired, reloaded, threw and struck. See [Weapons and shooting](#weapons-and-shooting). | Y11S3+ |
| `pings` | Every ping a player put on the map: who, on what kind of thing, where. | Y11S3+ |
| `spots`, `spotAssists` | Operators spotted through a drone or camera, with the spotter where one can be inferred, and the points a spotter got for a teammate's kill of the spotted player (inferred). | Y11S3+ |
| `abilityMarkers`, `deviceMarkers` | Tracking markers abilities put on players (Jackal, Alibi, Lion, Grim, Deimos), and the device markers of Solis. | Y11S3+ |
| `drones`, `cameras`, `deviceEvents`, `cameraCounts` | Every drone and camera with its owner, path and end; jams, captures and offline spans; cameras alive per team. Who destroyed a device is inferred. | Y11S3+ |
| `objective`, `operatorReveals`, `phoneHacks` | Who found the objective, when each player's operator became known to the other team and what showed in that moment, and Dokkaebi's phone hacks. | Y11S3+ |
| `systemMessages` | The lines of the feed that are no kills: players leaving, joining and reconnecting, reverse friendly fire turning on and off, the objective being found, the announcements of a phase, and any line of an id not known, kept with its raw id. See [System messages and BattlEye](#system-messages-and-battleye). | Y11S3+ |
| `battlEye` | Whether the feed showed a line that says "BattlEye" (`flagged`), which lines, and how many lines are of a kind not known. It marks the round and repeats what the game showed; it says nothing of any player. A match folder adds `battlEye.flaggedRounds`. | Y11S3+, and before Y9S1 |
| `leavers`, `reconnects`, `seats` | Who left during the round, when, alive or dead, and whether the feed showed the line taken to say the connection was lost (inferred); who took a seat again during it; and the seats that were not plainly a player's when it started. A match folder adds `presence`, per player who left: what they did round by round, the rounds they missed and whether they came back. See [Leavers and reconnects](#leavers-and-reconnects). | Y11S3+ |
| `metalDetectors` | Alarms of the map's metal detectors, with the nearest player (inferred). | Y11S3+ |
| `matchFeedback[].victimSpotted`, `victimPinged` | On kills: the killer's team had the victim spotted, or had pinged where the victim was, at most 15 seconds before. Derived. | Y11S3+ |
| `gadgets`, `mapCameras` | Every gadget object placed or thrown: what, whose, where, when, its states, the statuses put on it, each time it went off as a trap, and how it ended, with who destroyed it where the scoreboard tells. The map's own cameras, with when each was destroyed. See [Gadgets, world and destruction](#gadgets-world-and-destruction). | Y11S3+ |
| `deviceRemovals`, `gadgetStatuses`, `trapTriggers` | The same events for what is no entry of `gadgets`: drones and cameras destroyed, a status on a drone or a camera's mount, a trap no object was matched to. | Y11S3+ |
| `scoreChanges` | Every change of a player's score, with what it was probably for. | Y11S3+ |
| `scoreboard` | Per player: the match totals of score, kills, deaths, assists and plants at the start and the end of the round, and what the round added. See [Scoreboard](#scoreboard). | Y11S3+ |
| `reinforcements`, `barricades` | Who put up what, where, when and on which wall, hatch, door or window; when a reinforcement was opened, and how a barricade was destroyed. | Y11S3+ |
| `destruction`, `surfaces`, `breaches` | What damaged the map and its panels, bullets apart; the holes that left (derived); every breach device with what became of it. | Y11S3+ |
| `areas`, `environment`, `lightScreens` | Smoke, fire, gas and swarm areas with their source and owner; gas pipes, fire extinguishers and metal detectors set off; the light screens of Sens. | Y11S3+ |
| `matchFeedback[].inArea`, `hits[].inArea`, `hits[].gadgetOwner`, `shots[].throughSmoke`, `effects[].jammer` | The area a victim stood in, the owner of the barbed wire that hurt, whether a shot passed through smoke, and whose Signal Disruptor jammed a player. All derived, each with its source. | Y11S3+ |
| `matchFeedback[].previousOperator` | For operator swaps, the operator swapped from (`operator` is the one swapped to). | Y8S1+ |
| `activity` | Who carried the defuser, plants and disables with their outcome, what each player held, reloads and ability signals. See [Activity](#activity). | Y11S3+ |
| `objectiveState` | The game mode and, for Bomb, what the defuser did: the round's two bombs, each carry with how it started and ended, where the defuser was dropped, came to lie and was picked up, plants and disables with where the defuser is, on which bomb, and what was left on the defuser timer. See [Objective](#objective). | Y11S3+ |
| `movement` | With `--movement`: position, view direction, stance, lean, aiming, gait, rappel, falls, what each player looked through and what they placed. See [Movement](#movement). | Y11S3+ |

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

A match folder adds `presence` (Y11S3+): one entry per player who left at some point of the match, with `events[]` round by round (`left`, `reconnected`, `joined`, `absentAtStart`, `backAtStart`), `roundsMissed`, `returned` and what the leave is likely to have been (`likely`, inferred). See [Leavers and reconnects](#leavers-and-reconnects).

A match folder adds `breaks` (Y11S3+): per pair of consecutive rounds, the seconds between the two recordings, whether operators were banned or sides switched in between, and whether the break is long for the match (`pauseSuspected`, inferred). See [Pauses](#pauses).

A match folder adds `analytics`: per team attack and defense records, rounds started a player down, plants, disables and prep swaps (late ones counted apart); per site the defense win rate overall and per team; per attacker spawn and team the pick and round win rates; per operator and team rounds, win rate, kills, deaths, headshots and how often it was swapped to; and a count of rounds per end reason and winning side. `summary.rounds[]` also gains `winProbability`, `endReason` and `playersAtStart`.

Every round also says where it came from and how far it can be trusted:

| Key | What it holds |
|---|---|
| `replay.file` | Path, size, modified time and SHA-256, for deduplication and "already imported" checks. |
| `replay.format` | The `dissect` prelude: format version (7 before Y8S4, 8 since), layout, declared frame count and header property count. A format version, label or layout not seen before is flagged in `decodeStatus.header`. |
| `replay.version` | `Y11S3_Alpha04` split into season, year, season number and branch, plus the build number (`code`). |
| `replay.parser` | Parser version and the decoder profile and revision chosen for the build. A decoding fix bumps the revision of the profiles it touches, so stored rounds with an older `(decoder, decoderRevision)` than `--decoders` lists for their build are the ones to re-parse. `untestedBuild` flags builds newer than any the decoders were checked against (9883691, 9901603 and 9918362, all `Y11S3_Alpha04`). |
| `replay.container` | Y8S4+: the streams the round was recorded in (id, name hash, role where known, frames covered, snapshot blocks, record count), the compressed blocks, `recordingId`, and `complete`, false when the game did not finish writing the file. See [File format notes](#file-format-notes). |
| `decodeStatus` | Per field (`container`, `players`, `kills`, `scoreboard`, `bans`, `health`, `result`, `timing`, ...): `decoded`, `inferred`, `partial`, `missing`, `notInVersion` or `skipped`, with a count and warnings such as how many packets failed. `trusted` is false when any field is partial or missing. What this parser does not decode for a version (the feed's lines that are no kills, from Y9S1 to Y11S2: `feedbackMessages`) is `notInVersion`, and a player who never spawned has no body to link, so `trusted` marks faults: a field left unread (`skipped`: a partial read, a custom game's party) does not lower it either. From Y11S3 `feedbackMessages` counts `systemMessages`; a line with an id not known is kept and named in a warning, which does not lower the status. Of the 167 real rounds in one `MatchReplay` folder, the 6 untrusted ones were 3 unfinished files and 3 rounds that were not played out (no result). |
| `timing` | From the frame index: frame count, duration, median interval and the `sampleRate` it gives, `meanRate`, and intervals over 4x the median (`gaps`). `dataRate` is how often the game sent updates, whatever the frame rate: records per second in the state stream, about 28. `holes` are stretches over 0.5 s without a movement record, which the game writes at every update. `skips` lists moments the game moved on by more than the recording's clock did (Y11S3; see [File format notes](#file-format-notes)). `clockGaps` lists seconds the in-game clock skipped, ignoring the reset at round end and the switch to the defuser timer. From Y11S3, `startedAt` (UTC), the UTC offset of the header's local `timestamp`, and `headerMinusIndex`: seconds the header's `endtime` less `starttime` is longer than the index, a few milliseconds unless the recording skipped game time or stood still. |
| `pauses` | Y11S3 full reads: stretches the round may have been paused for, each with `kind`, `start`, `end`, `duration`, the clock at its start and the `evidence`. Inferred from the clock and the streams; no round checked holds a pause. See [Pauses](#pauses). |
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
- **Replays hold no server region and no ping.** The same rounds were searched for the data-center names the game itself uses (`gamelift/eu-west-1` and so on, plain and hashed), for region, host and IP text, and for any per-player value that behaves like latency. The scoreboard object carries score, kills, deaths, assists, defuser plants and a placement that is always 0, and nothing else (see [Scoreboard](#scoreboard)). A second search walked every property of every object in the state stream of 38 rounds (28 real, recorded as a player, and the 10 test rounds) and tried the hashes of about 490,000 names for ping, latency and connection quality against every hash seen: no property is one, and none behaves like one. The only geographic hint is the recording PC's UTC offset (`timing`).
- **`endedEarly`** comes from `matchresult`. A forfeit, where a team surrenders, should carry 1 or 2 like a normal ending and so get a winner, but no forfeit has been seen. Value 7 has been seen once: the server ended a ranked match 0.03 s into round 4, with no clock, both teams marked as losing and a system notice no other round has. The cause, which the file does not say, may be a ban of a cheating player, which ends a match for everyone.
- **Dual Front** (6v6, respawns) has not been seen in a replay, and no source says whether it records one. Pick packets are split into teams by `maxPlayersPerTeam`, and `picks` can hold several operators per player, but respawns are not decoded.

- Before Y11S3, replays give a player's health and never who caused a change, so there is damage taken but no damage dealt. From Y11S3 every hit is read, and its attacker is named or inferred; see [Health and damage](#health-and-damage). `bulletHits` separately matches each bullet hit to the shot that made it; see [Weapons and shooting](#weapons-and-shooting).
- Before Y11S3, replays sometimes skip the last health update before a kill, which makes `damageTaken` a lower bound.
- Observation tool ids 1 (drone), 2 (camera), 6 (Black Eye), 8 (Flores drone) and 9 (shock drone) are confirmed; 3 is a second camera kind seen on defenders with a camera gadget. 5 (Evil Eye), 7 (Yokai), 10 (Kludge Drone) and 11 (a Pantheon shell of Skopos) are known from the device a session is on, but have no name in the table yet: they print as `ObservationTool(n)` and count for neither `droneSeconds` nor `cameraSeconds`. From Y11S3 `observation[].device` names the device, and `drones` and `cameras` its kind.
- No round in the test match or the real folder ends with the defuser going off, so `DefusedBomb` is untested from Y11S3: it is given when attackers win after a plant with defenders left and the defuser timer at zero.
- `elapsed` counts whole clock seconds, and the first second of a recording is cut short, so it can sit up to two seconds from `recordingTime` minus the prep start.
- An attacker whose pick stays `RANDOM` in the file (3 of 50 random picks) has no spawn name; `spawnPosition` still says where the body appeared.

Y11S3 attacker swaps are linked through the player's state object, because the caster UI id older seasons use is shared by a whole team there.

Y11S3 scoreboard packets no longer carry ids that match players; they are linked through each player's scoreboard object instead (see [Players and identity](#players-and-identity)).

## Scoreboard

From Y11S3 every player has a scoreboard object (`PlayerStatsViewModel`, `players[].entities.scoreboard`). It holds seven properties and nothing else, the same seven in each of the 1,837 objects looked at (10 test rounds, 175 real ones). All are match totals: the round's opening snapshot carries what the match stood at, and each later write replaces a total.

| Property | Hash | In `scoreboard[]` | What it is |
|---|---|---|---|
| `MatchKills` | `1cd2b19d` | `kills` | Kills the game credits. A kill of a player an opponent downed goes to whoever downed them, not to the finisher; a team kill goes to nobody. |
| `MatchDeaths` | `cd9c5d72` | `deaths` | Deaths, team kills and deaths without a killer included. |
| `MatchAssists` | `4d737f9e` | `assists` | Assists. Nothing else in the file records one. |
| `MatchGameModeActions` | `31a3d4d2` | `gameModeActions` | In Bomb, defusers planted. Disabling one does not count. Only Bomb rounds were seen. |
| `MatchScore` | `ecda4f80` | `score` | Score, signed: it went below zero mid-round in 3 real rounds. |
| `MatchPlacement` | `8310f4f4` | `placement` | 0 in every snapshot, never written after it. |
| `MatchPlacementLabel` | `841d24ab` | not in the output | A text list, empty in every snapshot. |

`scoreboard[]` has one entry per player:

| Field | Meaning | Decoded or inferred |
|---|---|---|
| `start` | The totals in the opening snapshot: what the match stood at going into the round. | Decoded |
| `end` | The totals when the recording ends. | Decoded |
| `round` | `end` less `start`: the round's score, kills, deaths, assists and plants. | Derived |
| `placement` | As above. | Decoded |

How it compares with the rest of the round, over the 10 test rounds and 178 real ones (1,867 players):

- **Deaths** equal the feed's in every case, and `stats[].died` is `round.deaths == 1`.
- **Kills** equal the feed's in every case once a kill is counted for `matchFeedback[].creditedTo` where it has one, and team kills are left out. `stats[].kills` counts what the feed names a player for, so it differs from `round.kills` for a finisher, for the player who downed, and for a team killer.
- **Score**: `round.score` is the sum of the player's `scoreChanges[].delta` in every case. `stats[].score` is `end.score`, a match total.
- **Assists**: `stats[].assists` is `round.assists`.
- **Round to round**: a round starts on the totals the round before ended on. Of 1,557 player pairs, 1,533 do; 20 are across a round that is missing from its folder, 3 are a plant counted after the recording stopped, and 1 is the reconnect below.
- **Headshots are not on the scoreboard.** They come from the kill feed (`matchFeedback[].headshot`, summed in `stats[].headshots`); every kill has the flag.

`scoreChanges[]` is the raw history of `MatchScore`: each change with `delta`, `total` and when. Only the `reason` on it is inferred (see [What happens to gadgets](#what-happens-to-gadgets)).

Limits:

- **A player who reconnects mid-round gets a new scoreboard object**, and what is written to it is not read: 1 of the 1,867 (10 points missing from `end.score`). The next round's `start` has the right total.
- **A plant can be counted without a `DefuserPlantComplete`** in the feed, when the round is decided while the defuser is being planted (4 real rounds).
- **A player who left before the round** keeps a scoreboard object with frozen totals; they are not in `players` and have no entry.

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
- **A detonation.** A thrown object is removed; nothing says it went off. `gadgets[].end` says `wentOff` for the types that end no other way, which is inferred (see [Gadgets, world and destruction](#gadgets-world-and-destruction)).

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
| `shots[].throughSmoke` | `true` when that path passes through a smoke cloud of `areas[]`. Derived: the cloud's radius is assumed. Left out otherwise. |
| `bulletHits[].victim`, `position` | The player struck and where, in map coordinates. |
| `bulletHits[].damage`, `limb`, `result` | Health taken, whether an arm or leg was struck, and `alive`, `down` or `dead` after it. Absent for a bullet in a body that was already down or dead. |
| `bulletHits[].shooter`, `shooterSource`, `shot` | Who fired, how that is known (`ray`), and the index of the shot in `shots`. Absent when no shot fits. |

How they are found:

- **Shots.** In the `movement` stream a gun's `607385fe` update ends in a list of events. Event `06` (63 bytes) is the gun firing: muzzle position, direction, and the distance to the impact from the eye and from the muzzle. The game repeats the event in every update while an automatic gun fires, and a player's own recording repeats each about 16 times, so a shot is a run of events of one gun with the same direction and distance. A shotgun writes one event per shell. The gun is linked to the body that carries it and the body to its player, as loadouts are.
- **Hits.** A bullet striking a body spawns an effect in the `FXChannel` stream (`f5ee6a3d`): asset `d5 6d 41 58`, the body as its target, and where it struck in the vector parameter `56 95 b5 31`. The stream's records are read whole, section by section (see [The world](#the-world)).
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
- Bullets that strike a destructible wall, floor or barricade leave a record in its damage list, with the shooter's body: they are counted in `surfaces[]` (see [Destruction, surfaces and breaches](#destruction-surfaces-and-breaches)) and not listed one by one. The marks of `DecalChannel` are not read. A bullet hit on a player is still not marked as having gone through a wall.

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
| `direction`, `speed` | Unit vector and metres a second of the first full step of the flight. Absent for an object that stuck to something within its first step. |
| `path` | `[seconds since release, x, y, z]`, thinned to at most 60 points. |
| `end`, `flightTime` | Where the flight stopped and how long it took. |
| `ended`, `endedAfter` | `deleted` (the game removed the object) or `returned` (taken back into its pool), and seconds since the release. Absent when it was still there at the end of the recording. |

How it is found: the `movement` stream creates each object with a `617385fe` message that lists its component classes. Everything a player can let go of has a component of class `8490f616` in its `607385fe` updates: a `u8` mask, then a `u16` (bit 01), the owner's `playerid` (02), the owner's alliance (04) and a flag (08) that is 1 once the object is released and 0 when it is taken back. A throw is the update that sets the flag to 1. That message carries the release position, and each update after it one position, about 30 a second. The game writes no velocity, so direction and speed come from the second and third position; the first step starts at a position that can be a frame old and is no measure. An object thrown at something an arm away sticks within that first step, and its next positions are it settling by millimetres: when the second step has less than a twentieth of the first step's speed, the throw has no direction and no speed. The item is the slot of the thrower's body (`PrimaryGadget`, `SecondaryGadget`, `Drone`) whose asset equals the object's. Launcher ammunition is in no slot and is named by a table of 20 assets, each assigned to the launcher of the only operator who fires it. A drone is driven straight after it lands, so its path is cut at the landing. Objects are pooled and created under the map, so the position an object is created at is not where it was thrown from.

| | Test rounds (10) | Real rounds (167) |
|---|---:|---:|
| Throws | 348 | 3,802 |
| Count drops of hand-thrown items with a release at most 1.1 s before | 168 of 168 | 2,362 of 2,366 |
| Count drops of launchers with a release | 48 of 48 | 367 of 371 |

The count drops 0.38 s after the release (median). Grenades in free flight fit a parabola of 9.3 m/s².

- `ended` is the object being removed. For a grenade that is within a frame or two of it going off; for a gadget it may be minutes later.
- No release was found for Thatcher's EMP grenade.
- Gadgets that are placed, not thrown (barbed wire, deployable shields, breach charges, cameras on walls), are another class (`4c60869a`) and are in `gadgets[]`, as is every thrown gadget that stays where it lands (see [Gadgets](#gadgets)).
- A drone thrown again after its owner picked it up has no throw: the released flag does not change, the drone only comes back into the world. 3 of 50 drone deployments in the test rounds and 15 of 429 in real ones are such. `drones[].deployments` lists every one.
- A few pooled sub-munitions are released without ever being given an owner (8 in the real rounds); they are left out and counted in `decodeStatus.throws`.
- Both of Capitao's bolts share one name, as do both of Zofia's grenades: which asset is which type is not known. Four assets seen in real rounds have no name.
- Drones report the 15.9 m/s they leave the hand with.
- Attackers start prep already on their drones, so prep has no drone throws.

### Melee and shields

`meleeHits` lists every melee hit on a barricade or a destructible part of the map; `shieldActions` lists what players did with a held shield. Both come from the `movement` stream.

Anything that can be damaged carries a damage list in its update messages: a `u32` count, then one entry per hit. An entry is a kind byte (0 for anything that is not a bullet, 1 for another player's bullet, 2 and 3 for the recording player's own bullet as their game predicted it and as it was confirmed), the hit point in the object's own space, the body that did it, a damage id and a list of impacts; an entry of the single byte `fe` says the object is destroyed. A melee hit is an entry with damage id 34118943362.

| Key | What it holds |
|---|---|
| `meleeHits[].username` | The player whose body the entry names. |
| `target`, `object` | `barricade`, `mapObject` (a wall, hatch or prop: an id that is the same in every round on the map) or `entity` (another entity that takes damage), and the id of what was hit. The file does not say whether a map object is a wall, a hatch or a prop; `destruction[]` gives a kind from a catalog. |
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
| `matchFeedback[].finish`, `downedBy` | The kill ended a down, and who dealt that down. Where the scoreboard credits the kill to another player (`creditedTo`), it is the downer, in all but 1 of 22 such kills in the test and real rounds: there the feed names the player who dealt the down and the scoreboard credits a teammate. |
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
- **Whether a hit went through a wall is not recorded.** A fire or gas hit names the area the victim stood in (`inArea`), and a barbed wire hit the owner of the nearest wire (`gadgetOwner`); both are derived (see [What is joined onto other events](#what-is-joined-onto-other-events)).

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

Each player's HUD keeps a list of the effects on them. `effects[]` has one entry per stretch: `username`, `type` (the game's number), `name`, `buff` (true for the player's own or a teammate's ability), when it started and `seconds`. An effect names neither the gadget nor the player behind it; for a jammed player (type 8), `jammer` is the owner of the nearest Signal Disruptor, with `jammerSource: "nearest"`.

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
| `OutsideWarning` | 15 | `DokkaebiOverload` | 51 |

- `OutsideWarning` showed on defenders only, for at most 1.05 s, with the body outside the area the defenders were in during prep in 17 of 24 cases checked: the warning before a defender outside is detected. No marker follows it.
- `DokkaebiOverload` started 7.03 to 7.07 s after a Logic Bomb, as a 40-damage explosion hit that defender, and lasted until death or the end of the round (6 of 6): the phone she called, going off.

Types 21, 25, 29 and 37 occur and have no name (29 on shield operators only, 37 only in rounds with Denari). A kill's `victimEffects` lists the named effects the victim was under.

What the list does not hold:

- **Flashes of other players.** The flash flag is written for the player whose screen the recording shows, so `flashes[]` covers the recording player only and a spectator recording has none.
- **Concussion, Smoke's gas, electricity and traps** have no effect entry. Gas and fire still show as `hits` with their damage type.
- **A hacked phone** (Dokkaebi) is no effect on anyone. The hack itself is in `phoneHacks[]`, see [Phone hacks](#phone-hacks).

### Friendly fire

`friendlyFire[]` lists each player whose reverse friendly fire was on in the round: `activeAtStart` when it carried over from an earlier round, and `on` and `off` with their times. It is the game's own flag (`IsReverseFriendlyFireActive`). It turned on for the killer after all 12 team kills in the real folder, and 5 times with no kill, after damage to a teammate. Team damage itself is only in `hits`, where `teamDamage` counts the hits a teammate is named for.

`decodeStatus` adds `combat` (hits read) and `vitals` (players with a maximum health).

## Pings, spotting and information

From Y11S3, full reads of files the game finished writing carry what the teams showed and learned of each other. Every event has `time`, `phase`, `elapsed` and `recordingTime` like the kill feed. Numbers below are from 175 rounds of a real `MatchReplay` folder and the 10 test rounds.

| Key | What it holds | How |
|---|---|---|
| `pings[]` | Every ping: who, on what kind of thing, where. | Decoded; object names inferred |
| `spots[]` | Operators spotted through a drone or camera: who was seen, by which team, where, for how long, and who spotted. | Marks decoded; the spotter inferred |
| `spotAssists[]` | The 50 points a spotter gets when a teammate kills the spotted player. | Inferred |
| `abilityMarkers[]` | Tracking markers of Jackal, Alibi, Lion, Grim and Deimos on a player, with their path and end. | Decoded; ability names and `by` inferred |
| `deviceMarkers[]` | The device markers of Solis. | Decoded; meaning inferred |
| `drones[]`, `cameras[]` | Every drone and camera: owner, deployments, path, how it ended. | Decoded; who destroyed it inferred |
| `deviceEvents[]`, `cameraCounts[]` | Jams, countdowns, captures and offline spans of a device, and the cameras alive per team after each change. | Decoded; who captured inferred |
| `objective` | Whether and by whom the objective was found. | Decoded from the feed or the score in prep; inferred after prep without a feed |
| `operatorReveals[]` | When each player's operator became known to the other team. | Decoded; `cause` inferred |
| `phoneHacks[]` | Dokkaebi hacking the phone of a dead defender. | Layout decoded; meaning inferred |
| `metalDetectors[]` | Alarms of the map's metal detectors. | Decoded; the player inferred |

What the file does not hold, after searching every stream:

- **When a ping ends.** A ping is written once and never removed (none of 3,358 real pings is). How long it showed is not in the file.
- **Whether a ping on a gadget revealed the operator.** A ping holds a label for the kind of object, not the entity, and no flag for a reveal.
- **Who spotted.** A spot names the spotted player and the team that sees it. `by` is inferred.
- **Who destroyed a device.** The device says it is destroyed, not by whom. `by` is inferred from the score and the shots.
- **Why a device went offline.** There is a flag and no cause. An EMP is not named.
- **Which phone was hacked.** The tablet's state says a hack runs; no defender is linked to it.
- **Caveira's interrogation.** No marker, effect or HUD property was found for it.
- **Anything specific to Pulse, Vigil, Nokk, IQ, Zero or Skopos.** No marker or effect is theirs alone. Zero's and Skopos's cameras are in `cameras[]` like any other.
- **An intel assist.** The kill feed is the same for a kill of a spotted player as for any other, the timeline has no such entry, and `MatchAssists` does not count it (42 of 42 unchanged).
- **The reason for a score.** The scoreboard holds totals. Only the amounts can be compared with what else happened in that moment.

### Pings

The `MarkerChannel` stream (`26b9c2c1`) holds what changed among the markers in each frame. A record is two lists: `u16` count and 55-byte markers, then `u16` count and 42-byte device entries. A marker is a position, a `u64` (the pinger's `playerid`, or the marked player's body entity), a `u64` label, then `u32` class (0 ping, 1 spotted operator, 2 tracking marker), alliance, source, the pinger's place in the team and target, and three flag bytes, the first set on a removal. Every record parsed to its last byte in the 10 test rounds and in all 175 finished real rounds.

| Key | What it holds | How |
|---|---|---|
| `username`, `team` | Who pinged, and their team from the alliance the record states. | Decoded |
| `kind` | `location`, `locationRepeat` (the second press of a double ping), `enemyObject` or `teamObject`. | Decoded; the two object kinds were named by whose entity was nearest (188 of 194) |
| `target` | The record's number behind `kind`: 3, 1 or 2. | Decoded |
| `label` | `{id, name}` of the kind of object pinged. Absent on a plain location ping. | Id decoded; name inferred |
| `position` | Map coordinates in metres, z up. | Decoded |

- 138 pings in the test rounds, 3,358 in the real folder (1,737 by the recorder's team, 1,621 by the other). Every one names a player of its round, and the alliance is that player's every time.
- A `locationRepeat` follows the same player's ping at the same place: 11 of 11 in the test rounds, 529 of 539 within 0.5 s in real ones. What the game shows for it is not known.
- A label id occurs nowhere else in the file. 22 are named by the entity that sat within 0.5 m of the pings with that label (the drone in 46 of 47, an Entry Denial Device in 19 of 19). About 30 more have no name.
- The stream's opening snapshot is the last marker sent before the recording began. It is left out.

### Spots and spot assists

A class 1 marker is a spot: the body of the spotted player, at that body's position (0.07 m off at the median), with the alliance of the team that sees it. A player of that team was on a drone or camera at 41 of 41 marks in the test rounds and 860 of 862 in real ones, which is why it is taken to be the red ping or scan. A scan that goes on writes a new mark every 1.5 to 1.9 s. Marks on one player at most 2.5 s apart are joined into one spot: 41 marks make 33 spots in the test rounds, 862 make 543 in real ones.

| Key | What it holds | How |
|---|---|---|
| `username` | The player who was spotted. | Decoded |
| `seenBy` | Index of the team that sees the marker. | Decoded |
| `position` | Where the spotted player was at the first mark. | Decoded |
| `seconds`, `marks` | From the first mark to the last, and how many marks. A single mark has 0 seconds. | Derived |
| `by`, `bySource` | Who spotted, and the rule that named them. | Inferred |
| `byCandidates` | When no rule names one player: the opponents who looked through a device. | Inferred |
| `with`, `device` | `drone` or `camera`, and the entity id as `drones[]` and `cameras[]` give it: the device of `by`, or the one all candidates were on. | Inferred |

`bySource` is the first of these that names a player:

- `spotAssistScore`: the player who got a spot assist for this spot (below). 1 of 33 test spots, 46 of 543 real ones.
- `onlyObserver`: one opponent of the spotted player looked through a device at the first mark, by the player tables. 6 of 33, 103 of 543.
- `nearestFacingTool`: several looked through several devices. The device nearest to the spotted player is also the one turned most towards them, and one player was on it. 9 of 33, 134 of 543.
- Absent (17 of 33, 260 of 543): `byCandidates` lists everyone who was on a device, or those on the nearest facing device when several were on it.

Checked against the first rule on 41 real spots that have its points: `onlyObserver` named that player in 19 of 20, `nearestFacingTool` in 6 of 8, and the player was among the candidates in 13 of 13. Several players can watch one device, so the device can be right while the player is not.

How long the red marker stays is not in the file: no removal is ever written for a spot.

A spot itself pays nothing. The spotter gets 50 points when a teammate kills the spotted player. The score holds no reasons, so `spotAssists[]` is inferred: a +50 of one player alone, with no assist counted (`MatchAssists`), from 0.3 s before to 0.8 s after a teammate's kill of a player whose spot has a mark at most 14 s old.

| Key | What it holds | How |
|---|---|---|
| `username` | Who got the points. | Inferred |
| `victim`, `killer` | The kill they were paid for. | Decoded (the kill feed) |
| `markAge` | Seconds from the newest mark on the victim to the kill. | Derived |

- In the real folder 36 of 39 kills within 6 s of the newest mark came with the +50, 10 of 21 kills 6 to 14 s after it, and none of 24 later ones. The oldest mark that still paid was 13.2 s old.
- 1 spot assist in the test rounds, 53 in the real folder. The recipient was on a device at that spot in 47 of 53.
- Other awards are 50 too: finding the objective, a revive. The rule can miss an assist and can take another award for one.

### Ability markers and device markers

A class 2 marker is a tracking marker an ability put on a player. There is one per body and source. It is written again as it moves and ends with a record that has the removed flag, or with one of source 14 that clears every marker of a body as its player dies (66 of 66 such records in the test rounds match a kill).

| Key | What it holds | How |
|---|---|---|
| `username` | The player who was marked. | Decoded |
| `source` | `{id, name}`: 0 `JackalTracked`, 1 `AlibiTracked`, 3 `LionScan`, 4 `GrimSwarm`, 5 `GrimTracked`, 6 `DeimosMarked`, 7 `DeimosTracking`, 8 `TrackerJammed`. | Id decoded; names inferred |
| `by`, `bySource` | Whose ability it was: the one opponent who plays that operator (`operator`). Absent when none or several do, and for sources 7 and 8. | Inferred |
| `seenBy` | Index of the team that sees it. | Decoded |
| `position`, `path` | Where it was first put, and each move as `[seconds since the start, x, y, z]`. | Decoded |
| `ended`, `seconds`, `open` | `removed` or `cleared` (the player died), how long it stayed, and `open` when it was still there as the recording ended. | Decoded |
| `pulse` | Lion's scan and Grim's swarm write a marker per pulse and never remove it: no end and no `seconds`. | Decoded |

- The sources were named by the status effect on the marked player in the same moment: Jackal 10 of 10, Alibi 12 of 12, Lion 23 of 23, Grim's swarm 17 of 17, Grim's tracking 37 of 37, Deimos's target 23 of 23, Deimos himself 21 of 21.
- Source 8 takes the place of 6 and 7 for about 2 s, only in rounds with both Deimos and Mute. `TrackerJammed` is a guess from that.
- 48 markers in the test rounds, 37 with `by`; 228 in the real folder, 199 with `by`.
- Dokkaebi's call, a proximity alarm, Melusi, Fenrir and the radar of Solid Snake put no marker. They show only in `effects[]`.

`deviceMarkers[]` is the second list of a record: `op` (`add`, `remove`, `update`), `handle` (a counter), `target` (an entity id) and `position`. Entries occur only in rounds with Solis: 18 of 24 real ones and both test ones, and 0 of 149 rounds without her. They sit on attackers' devices and on fixed objects of the map and last 0.1 to 3 s. That they are what her SPEC-IO detects, and that an `update` is a device identified, is inferred. It is the only sign of her sensor in the file.

### Drones and cameras

A device is an entity of the `movement` stream whose first component class is `587f5a72`. With the class `47e5f600` it is a drone, else a camera. The cameras of the map are created in the stream's snapshot with ids that are the same in every round on a map. The device's own component holds nine asset ids, seven flag bytes (offline, disabled, destroyed, in flight, a timer, signal lost) and a position. Every update of a device was read to its last byte in the test rounds (130,733) and none was left unread in the real folder (1,616,585; 10,094 of an Evil Eye or of Skopos's body stop at a component that is not decoded).

`drones[]`:

| Key | What it holds | How |
|---|---|---|
| `entity` | The entity id in hex. `observation[].device`, `deviceEvents[].device` and `spots[].device` name it. | Decoded |
| `kind` | `drone`, `shockDrone`, `kludgeDrone`, `yokai` or `rceRatero`. | Inferred from the entity's classes |
| `owner`, `team` | The first player the entity names. | Decoded |
| `deployments` | Each time it went out: thrown, thrown again after a pickup, or out as the recording started. | Decoded |
| `pickups` | Each time its owner took it back. | Decoded |
| `sessions` | Each stretch a player drove it: `username`, `seconds`. | Decoded |
| `path`, `position` | `[seconds since the recording started, x, y, z]`, thinned to at most 60 points, and where it was at its end. | Decoded |
| `end` | `kind` (`destroyed`, `pickedUp`, `expired`, `removed`), and for a destruction `by`, `bySource`, `teamKill` and `noFlag`. | Kind decoded; `by` inferred |

`cameras[]` has `entity`, `kind` (`default` for a camera of the map, `blackEye`, `argus`, `bulletproof`, `evilEye`, `pantheonShell` for the body Skopos is not in, `unknown` for classes no kind has), `owner`, `team`, `position`, `placed` and `end`. A camera of the map has no owner and serves the defenders.

- **Checked against the HUD.** A throw matches `throws[]` for 47 of 47 drone throws in the test rounds and 414 of 414 real ones. A destruction falls in the frame the HUD's `DroneState` turns 2 for 58 of 58 and 586 of 586. A pickup matches the HUD's `IsDeployed` for 4 of 4 and 64 of 64.
- **Who destroyed a device is inferred.** The destroyer's `MatchScore` rises in the same moment (a teammate's falls instead: `teamKill`), and a shot of `shots[]` passes within 0.5 m of the device. `bySource` says which was found. Of 1,327 real destructions: `score+shot` 91.9%, `score` 6.0%, `shot` 0.5%, nobody named 1.7%. Of 127 in the test rounds: 124, 1, 0 and 2. Whenever a shot fitted, it was the scorer's.
- **How it was destroyed** is not told beyond a shot fitting or not. Melee, explosives and electricity are not told apart.
- A drone the game removes while it is in the world, with no destroyed flag, is reported destroyed with `noFlag` (4 real, 2 in the test rounds, both in rounds with Aruni).

`deviceEvents[]` has `type`, `device`, the time fields and, for a span, `seconds`:

| `type` | What it is | How |
|---|---|---|
| `jam` | A jammer disabled the device. `jammer` is its entity and `by` the player who carries that gadget. | Decoded: the component's position becomes the jammer's, to the centimetre |
| `countdown`, `signalLost` | The device is disabled with a timer running (`timer`, seconds), and the timer ran out. Seen on devices outside. | Decoded; the cause inferred |
| `capture` | The device changed sides. `kind` is `pest` (Mozzie) or `kludge` (Brava); `by` is who took it, with `inferred: true`. | Decoded; `by` inferred from the nearest Pest or Kludge Drone |
| `offline` | The offline flag was set. The file gives no cause. | Decoded |

- 222 jams in the real folder, each naming a Signal Disruptor and its owner; 7 of 7 in the test rounds.
- 16 captures in the real folder, none in the test rounds: 13 by a Pest (the Pest's owner was within 3 m in 13 of 13) and 3 by a Kludge Drone (its owner within 8 m in 3 of 3).
- 262 offline spans in the real folder, none in the test rounds. 77 fall within a jam, 2 last 15.0 s right after a Thatcher EMP, 5 last 6.0 s, and 172 are short with no cause found.

`cameraCounts[]` gives, after each change, the cameras alive per team as `teams[]` of `{default, gadget}`.

`observation[].device` is the entity id of the device a session is on: the view the player table (`ControllerChannel`) gave the player as the session started. 5,046 of 5,055 real sessions have one.

### Objective found

The game keeps no flag for the objective being found. Two things show it, both in the `state` stream:

- **The feed**, in a player's recording only. A feed entry holds the text shown (`<username> has found the bombs`) and names no killer and no victim. 147 real rounds have one. An entry is told by its shape and by the attacker its text names, not by its words, which may be translated.
- **The score**, in every recording. The finder's `MatchScore` rises by 50 in the frame of the find (146 of the 147 feed finds). When prep ends with the objective not found, every defender's rises by 100.

| Key | What it holds | How |
|---|---|---|
| `found` | False when prep ended unfound and no find followed. | Decoded |
| `by` | The finder. | Decoded, or inferred when `inferred` is set |
| `source` | `feed` or `score`. | Decoded |
| `inferred` | Set when the find is the first +50 of an attacker after prep that comes without an assist. Other awards are 50 too. | Derived |
| `inPrep` | Found before the action phase started. | Decoded |

- The score in prep named the player of the feed in 90 of 90 real rounds that have both, so it is not marked inferred. The score after prep named them in 49 of 57, so it is.
- Of 175 real rounds: 147 found by the feed, 2 inferred from the score, 16 not found, 10 with no objective to find (Quick Match rounds, and rounds that end in prep). All 10 test rounds are spectator recordings and take the score.

### Operator reveals

A player's controller has `HasBeenDiscovered` (`41f2118a`, one byte). It is 0 in every snapshot and turns 1 when the other team learns which operator the player is. `operatorReveals[]` has one entry per player it turned for.

| Key | What it holds | How |
|---|---|---|
| `username` | The player revealed. | Decoded |
| `trigger` | `kill` when the player killed an opponent at most 0.35 s before (0.27 s at the median), else `identified`. | Derived from the kill feed |
| `victim` | For `kill`: whom they killed. | Decoded (the kill feed) |
| `cause` | For `identified`: what the marker stream wrote within 0.25 s. `spot` (a spot mark on the player), `abilityMarker` (a tracking marker on them), `teammateSpot` (a spot mark on a teammate) or `ping` (an opponent's ping on an object), in that order. Absent when it wrote nothing that fits. | Inferred |
| `teamBonus` | Every player of the other team scored 10 (or 20, 30 for two or three reveals at once) in that moment. | Derived from the score |

- 60 reveals in the test rounds, 1,082 in the real folder (519 by a kill).
- Of 563 real `identified` reveals: `spot` 233, `teammateSpot` 21, `ping` 70, `abilityMarker` 24, and 215 with no cause. In the test rounds: 19, 2, 3, 3 and 11 of 38. Most reveals without a cause are defenders in prep, presumably seen by a drone without a scan.
- Who identified the player is not in the data. The file hides nothing by itself: every operator is in the controllers from the start, and dying reveals nobody.

### Phone hacks

Dokkaebi's ability object links a tablet through its field `Tablet` (`b4928f1d`). The tablet has an `EquipState` (`e5e20d29`): 0, 1, 2 in the two seconds before a call, and 3 while she hacks a phone. `phoneHacks[]` has one entry per stretch of 3: `username`, `seconds` (absent when the recording ended first), `completed` (the stretch ran 2.4 s or more; a hack takes 2.52 to 2.57 s) and the time fields.

- The meaning is inferred. All 24 stretches in the real folder (16 rounds) start after a defender died, 22 run their full length and 2 are cut short. 8 of 9 sessions in which an attacker then looks through a defender's camera follow a completed one.
- The test rounds have no Dokkaebi. The decoder is tested on bytes built by hand and on the real folder.
- Effect type 51 (`DokkaebiOverload`, see [Status effects](#status-effects)) is the phone she called going off, not the hack.

### Metal detectors

The `SoundChannel` stream (`63fe54d3`) is the command list of the sound engine. A record is a `u16` mask and, per set bit, a counted list: posted events, positions, orientations, switches, parameters and stops. All 60,325 records of the test rounds and 924,852 of the real folder parse to their last byte. An alarm is a posted event of kind 0 on a fixed object of the map whose owner is another object, the detector. It lasts 3.0 s.

| Key | What it holds | How |
|---|---|---|
| `detector` | The detector's entity id in hex, the same in every round on a map. | Decoded |
| `position` | Where the alarm sounds. | Decoded |
| `username` | The player whose body was nearest as it started, when within 2.5 m. | Inferred |
| `complete`, `seconds` | The posts span 2.8 s or more, and how long they span. | Decoded |

- Real folder: Bank 100 alarms, Border 37, Consulate 27, Kanal 15, Nighthaven Labs 1. The other ten maps seen have none. The 10 test rounds are on Bank.
- A body was within 2.5 m as the alarm started in 254 of 259 alarms, 1.1 m away at the median. Defenders set them off in prep too.
- The same stream holds every player's footsteps (walking and running, with the body they belong to) and gunshots (60 to 80% of `shots[]` have one). They are not output: the movement stream and `shots[]` say the same.

### On kills and in stats

A kill in `matchFeedback` gains two derived fields. Both say what was on the killer's team's screen, not that it led to the kill.

| Key | What it holds | How |
|---|---|---|
| `victimSpotted` | `{secondsAgo, by}`: the newest spot mark on the victim that the killer's team saw is at most 15 s old. `by` is the spot's inferred `by`, absent when it names nobody. | Derived; `by` inferred |
| `victimPinged` | `{secondsAgo, by}`: a player of the killer's team put a ping within 3 m of where the victim was hit, at most 15 s before. | Derived |

- The victim's position is where their body was at the hit that killed them (1,233 of 1,268 real kills have such a hit), else where a bullet struck them. A kill with neither gets no `victimPinged`.
- Test rounds: 2 of 66 kills have `victimSpotted`, 5 have `victimPinged`. Real folder: 60 and 117 of 1,268.

`stats[]` adds, each left out when 0 and summed per match too:

| Key | What it holds | How |
|---|---|---|
| `pings` | Pings the player put on the map. | Decoded |
| `timesSpotted` | Spots of this player. | Decoded |
| `spotsMade` | Spots whose `by` is this player. Spots nobody is named for count for nobody. | Inferred |
| `spotAssists` | Entries of `spotAssists[]` for this player. | Inferred |
| `devicesDestroyed` | Drones and cameras of the other team whose `end.by` is this player. | Inferred |
| `dronesLost` | Drones of this player that were destroyed. | Decoded |
| `timesJammed` | Jams of a drone of this player. | Decoded |
| `objectiveFound` | 1 when `objective.by` is this player (`objectivesFound` in the match totals). | As `objective` |

`decodeStatus` adds `markers`, `devices`, `intel` and `sound`.

## Chat and system

From Y11S3, full reads of files the game finished writing carry what the game told the players in the feed besides the kills. Numbers below are from 175 rounds of a real `MatchReplay` folder and the 10 test rounds.

### System messages and BattlEye

Every line of the on-screen feed is one entry of the HUD's `Messages` array in the `state` stream, the same entries the kills are. An entry's `Message` is one of two things:

- **A text**, written out by the game: `<username> has found the bombs`. `text` keeps it as it is.
- **An id**, eight bytes that name a line of the game's language files, with the values that fill its placeholders: `[PLAYER]` or `[STRING]`, each a player's name. `messageId` is the eight bytes as hex and `args[]` the values with the placeholder's hash (`key`) and `name`.

`systemMessages[]` lists each such line once, in order, with `time`, `phase`, `elapsed` and `recordingTime` like the kill feed. Kills stay in `matchFeedback`.

| Key | What it holds | How |
|---|---|---|
| `kind` | What the line says: see the table below. | From the text, or inferred from the id |
| `kindSource` | `decoded` for a text, `inferred` for an id. Absent for `unknown`. | |
| `messageId`, `args[]` | The id and its values. Absent for a text. | Decoded |
| `text` | The text, for a line that is one. | Decoded |
| `username`, `profileID` | The player the line names: the first value of an id, or the player a text names. `profileID` when that is a player of the header. | Decoded |
| `backgroundColor` | 1 or 2, a team's colour, on finds, leaves, joins and reconnects; 0 on the rest. | Decoded |

The wording of an id is not in the file. What each id means is inferred from when it shows:

| `messageId` | Value | `kind` | Seen |
|---|---|---|---|
| (a text) | | `objectiveFound` | 147 lines. The text names an attacker, and it is `objective.by` in all 147. Told by its shape, not its words, which may be translated. |
| `c3c5050000000065` | | `phase` | As the recording starts: 174 of 175 real rounds, all 10 test rounds. |
| `c4c5050000000065` | | `phase` | As the action phase starts: 173 real rounds, all 10 test rounds. |
| `39f4000000000065` | `[PLAYER]` | `playerLeft` | 10 lines in 7 rounds. |
| `38f4000000000065` | `[PLAYER]` | `playerJoined` | 1 line: a player not in the header, 12 s after a leave. |
| `fcdb020000000065` | `[PLAYER]` | `playerReconnected` | 3 lines. |
| `66f4020000000065` | `[STRING]` | `connectionLost` | 4 lines, each 0.39 to 0.42 s before the same player's `playerLeft`; 6 leaves come without. The weakest of these readings. |
| `bea3040000000065` | `[PLAYER]` | `reverseFriendlyFireOn` | 24 lines. 14 come within 1.5 s of the player's flag turning on in `friendlyFire[]`; the other 10 are said again in later rounds, 1.2 s and 46 s in, of a player whose flag carried over (`activeAtStart`). |
| `2ca9040000000065` | `[PLAYER]` | `reverseFriendlyFireOnSquad` | 4 lines in 2 rounds, each naming a squad-mate of the player it turned on for, just before that player's own line. |
| `bfa3040000000065` | `[PLAYER]` | `reverseFriendlyFireOff` | 6 lines, all within 1.5 s of the flag turning off. |

Any other id is kept as `unknown` with its `messageId` and `args`, never dropped. Three are seen and not named: `dc02060000000065` (16 lines in the 8 rounds of two matches, as the action phase starts and 6.8 s later), `9154050000000065` (once, as a recording starts, in a round without the two `phase` lines) and `0451343166523165` (3 lines, with a `[PLAYER]`).

An entry is written again each time the feed scrolls, so one line shown is several writes. A write is a new line when its content was not on a line above it before that frame, and not on the same line in the last 5 seconds (the entries' `Duration`). Counting kill lines that way gives the kills of `matchFeedback` in 185 of 185 rounds.

`battlEye` is the round's flag:

| Key | What it holds |
|---|---|
| `flagged` | True when a line's text says "BattlEye", in any case. Before Y9S1, where the feed is text: when the feed has such a line. |
| `messages` | Indexes into `systemMessages` of those lines. |
| `texts` | Their texts, as the game wrote them. |
| `unknownMessages` | How many lines are `unknown`. |

- **The flag marks the round, not a player.** It repeats what the game showed. Nothing in the output calls a player a cheater, and no field is derived to that end.
- **No BattlEye line exists in any Y11S3 round at hand**, so `flagged` is false in all 185. Its id, if it is one, is not known: such a line would show as `unknown`, which is why those are kept and counted. A round with `unknownMessages` above 0 is worth a look.
- Rounds from Y9S1 to Y11S2 have no `battlEye`: their feed's lines that are no kills are not decoded.
- A match folder adds `battlEye.flaggedRounds`, the numbers of the rounds flagged.

Not recorded, or not decoded:

- **The wording of an id.** Only the id and its values are in the file.
- **Why a player left**: the line names the player and nothing else. `leavers[]` pairs each line with the seat it tells of (see [Leavers and reconnects](#leavers-and-reconnects)).

`decodeStatus` adds `feedbackMessages`: the count of `systemMessages`, with a warning naming the ids not known. Only a line that could not be read makes it `partial`.

### Leavers and reconnects

Every seat of a match is a controller object, and a controller says who sits in it. Two of its properties tell of leaving:

- `PlayerSlotType` (`b6b71dd2`) takes four values. What each means is not in the file; it is inferred from what else the controller holds and from what follows:

| Value | `seat` | What the controller shows |
|---|---|---|
| 1 | | A player is in the seat. |
| 4 | `reserved` | The seat is held for the player who left: the profile id and the `playerid` stay. Seen in Ranked (5 times during a round, 10 seats at the start of one). |
| 2 | `opened` | The seat was emptied for another player to take: the `playerid` turns ff*8, the profile id a UUID of zeros and the platform 12; the name stays. Seen in Quick Match (4 times during a round, 2 seats at the start of one). |
| 3 | `joining` | A player has connected and waits for the next round. The name is written empty and 0.1 to 0.4 s later in full. |

- `HasLeft` (`ca35436c`) turns 1 at a leave and never back to 0. A seat at 1 with `HasLeft` 1 was vacated at some point of the match and filled again (`refilled`): 18 seats, each a player of the header. A seat at 2 with `HasLeft` 0 and no name is one nobody ever sat in (`empty`): each round of a custom match of two players has eight.

When a player leaves, their controller is sent again in full. Controllers of teammates may be sent again with nothing changed, so only a value that differs from the one before is an event. The changes seen during a round are 1 to 4 (5), 1 to 2 (3), 3 to 2 (1), 4 to 3 (3, a reconnect) and 2 to 3 (1, a join). Nothing turns 1 while a round is recorded: a seat at 3 reads 1 in the snapshot that opens the next round. A seat at 2, 3 or 4 with `HasLeft` set when a round starts has no player in the header (13 of 13).

The feed says the same. Every change to 2 or 4 is followed by the player's `playerLeft` line 0 to 0.4 s later (9 of 9), every change from 4 to 3 comes with a `playerReconnected` line and the one from 2 to 3 with a `playerJoined` line. A change and a line are paired when they name the same player within 0.5 s. One `playerLeft` line came with no change of a seat, 2.9 s after the last round of its match was decided and 0.03 s before the file ended: that leaver says `source: "feedOnly"`.

`leavers[]` lists each leave of a round, with `time`, `phase`, `elapsed` and `recordingTime` like the kill feed:

| Key | What it holds | How |
|---|---|---|
| `username`, `profileID`, `playerid`, `team` | Who left: the ids are the ones the controller held before the change, so an opened seat's leaver has them too. | Decoded |
| `seat`, `slotType` | `reserved` or `opened`, and the value written. Absent for `feedOnly`. | `slotType` decoded; the names are inferred |
| `aliveAtLeave` | The player had spawned and the kill feed has no death of theirs before the leave. The movement stream agrees in all 8 leaves of a spawned player: the body of the 4 who left alive is deleted in the frame of the change or up to 0.03 s before, the corpse of the 4 who left dead is not. | Decoded |
| `diedSecondsBefore` | Seconds from the player's death to the leave: 6.7 to 60.7. | Decoded |
| `neverSpawned` | The player had no body this round: they were waiting for the next one (3 to 2), or never had health. Left out when false. | Decoded |
| `connectionLost`, `connectionLostSource` | The feed showed the `connectionLost` line just before the leave (0 to 0.04 s before the change of the seat). The key repeats what the line is taken to mean, so the source is always `inferred`. | Inferred |
| `silentSeconds` | For a player who left alive: seconds since their body last moved or turned, or they last switched to a drone or camera. | Derived |
| `returned` | `sameRound` when the player took the seat again before the recording ended. Absent otherwise. | Decoded |
| `source` | `slot`, or `feedOnly` when only the line exists. | |

`reconnects[]` lists each player taking a seat during a round. They play from the next round on:

| Key | What it holds | How |
|---|---|---|
| `username`, `profileID`, `playerid`, `team` | Who came: the ids the controller held once the name was written. | Decoded |
| `kind` | `reconnect` for a reserved seat (4 to 3), `join` for an opened one (2 to 3). | Inferred from the slot types |
| `awaySeconds` | Seconds since the same player left the seat, when that leave is in this recording. | Decoded |
| `leftBeforeRecording` | The seat was already reserved or opened when the recording started. Left out when false. | Decoded |
| `newPlayerid` | For a reconnect: whether the `playerid` differs from the one the seat held. | Decoded |

`seats[]` lists the seats that are not plain when the recording starts, a slot type other than 1 or `HasLeft` set: `username` (the name the controller holds: the leaver's, for a reserved or opened seat), `profileID` (absent for an opened seat, whose ids were wiped), `team`, `slotType`, `seat` (`reserved`, `opened`, `joining`, `refilled` or `empty`) and `hasLeft`. An empty seat has no `username`.

A match folder adds `presence[]`, one entry per player who left at some point, keyed by profile id like the match's players:

| Key | What it holds | How |
|---|---|---|
| `username`, `profileID`, `team` | The player. | Decoded |
| `events[]` | In order, each with its `round` and `type`: `left` (with the time fields, `connectionLost` and `aliveAtLeave`), `reconnected` and `joined` (with the time fields and `newPlayerid`), `absentAtStart` (the first round whose header lacks the player) and `backAtStart` (the header lists them again, with `newPlayerid`). | Decoded |
| `roundsMissed` | Rounds whose header lacks the player while a seat was reserved for them. Rounds after an opened seat are not counted: the seat is no longer theirs. | Decoded |
| `returned` | For the last time the player was away: `sameRound`, `nextRound`, `later` or `never`, counted in rounds from the one they left in. For a player whose seat was reserved before the first recording, from the round before it. | Derived |
| `newPlayerid` | Whether they came back with another `playerid`, the last time. | Decoded |
| `likely`, `likelySource` | What was observed that bears on why: `connectionLost`, `gameRestarted`, `gameKeptRunning`. Always `inferred`; left out when nothing was. | Inferred |

A player who took a seat someone else left is not a leaver, and neither is one who took a seat and left it again before their first round.

`stats[]` gains `leftAt` for a player who left during the round: the `elapsed` of the leave. Their other numbers of that round are those of a player who was there until then only. A match's `stats[]` gains `roundsLeft` and `roundsMissed`; missed rounds are not among `rounds`. No existing number changes, and `round.left` stays what it was: the players alive at the start who left before the round was decided, which is what decides how a round ended.

What tells a lost connection from a player who quit is inferred, all of it. Nothing in the file says why:

- **The `connectionLost` line** came with 4 of the 9 leaves and not with the other 5. The three players who left alive with it had sent no input for 12.75, 15.5 and 91 s (`silentSeconds`), which fits a server that gave up on a client gone silent. The one who left alive without it had moved 3.6 s before. The fourth with the line had been dead for 37.6 s. Of the four others without it, three left 6.7 to 60.7 s after dying and one had never spawned.
- **The `playerid` on return.** It is fixed for one launch of the game: other players kept theirs from one match to the next in 54 of 56 cases. A player who comes back with a new one started the game again (`gameRestarted`, 3 of 5 returns); one who comes back with the same one kept it running (`gameKeptRunning`, 2 of 5).
- Nothing here says "crashed" or "quit". The output keeps to what was observed.

Not recorded, or not decoded:

- **The reason for a leave.** A player who quits and a game that crashes and closes its connection cleanly look the same.
- **When a player left or came back between two rounds.** Neither recording has it: the next round's snapshot shows the seat (`seats[]`) and the header lists the player or not, so `absentAtStart` and `backAtStart` have a round and no time. A player who left and came back between the same two rounds shows only as a `refilled` seat.
- **Whether a `refilled` seat holds the player who left it.** In Ranked it does; in Quick Match another player may have taken it, which shows only when the leave was recorded.
- **A leave in prep, or in Unranked.** None is among the rounds at hand, so what the seat does there is not known.

`decodeStatus` adds `presence`: `decoded`, with the count of leavers and reconnects. It is `partial`, with a warning, when a change of a seat has no line of the feed, a line has no change (other than a `playerLeft` in the last half second of a recording), a leaver is alive by the kill feed while their body stays or the other way round, `HasLeft` goes back to 0, or a slot type or a change not listed above occurs, which the warning names.
### Text chat and voice

Neither is in a replay. `decodeStatus` says so on Y11S3 full reads: `chat` and `voice` are `notInVersion`, each with a warning that states it, and there is no `chat` key in the output.

- **Text chat.** Every text value of the `state` stream was listed by class and property in 185 rounds (the 10 test rounds, recorded by a spectator, and 175 real ones, recorded by a player): player names, profile ids, spawn, site and location names, the timer's text, one markup string of the game, and the feed's `<name> has found the bombs`. A sweep of every stream for UTF-8 and UTF-16 text found nothing else, and none of 142,168 candidate names (`ChatMessage`, `IsMuted`, `IsTalking` and the like, with prefixes and suffixes) is the name of a class or property in the file. What this does not show is that a typed message was looked for and missed: nobody is known to have typed in those rounds. One round with a known line typed in team chat and in all chat would settle it.
- **Voice.** No audio container or codec header (`OggS`, `OpusHead`, `RIFF`, `BKHD` and others) is in any stream, and every stream compresses (zlib to 7-29% of its size; compressed audio would not shrink). `SoundChannel` parses to its last byte as sound-engine commands. No property changes the way a talking flag would: on player objects every property changes as often for opponents as for teammates.
- **If a later season records chat**, the text of a message is other people's personal data: it will not be stored by default, only who wrote, where and when, with the text behind a flag.
- **Stale memory in `FXChannel`.** Its sound entries of type 3 are 41 bytes of which 15 are padding the game never cleared, so they hold whatever was in the recording PC's memory: nearly always binary, now and then the tail of an asset name, and in 5 real rounds a few fragments of at most 15 bytes that read like typed text, with no sender or time. They are not chat as the game recorded it, and the parser walks past the sound section without keeping any of it.

### Pauses

The host of a custom match can pause it. How the game records that is not known: none of the rounds checked was paused (the 10 test rounds, a custom match recorded by a spectator, and 175 matchmaking rounds, one of them a file the game did not finish), and no property in the stream is named after a pause. So `pauses[]` is not read from the file. It is inferred from how a recording behaves, with thresholds set on unpaused rounds only, and every entry says `source: "inferred"`. A round without a finding has no `pauses` key; `decodeStatus.pauses` is `inferred` with the count, 0 included, on every Y11S3 full read, and `missing` when the round's clock was never written.

What it rests on: the clock object writes `TimerInMilliseconds`, what is left of the timer that runs, every dozen frames, and `TimerState` (0, 1 for the last seconds, 3 once the round is decided), and the frame index says how many seconds into the recording each frame is. Between two writes of one timer the clock drops by as much as the recording moved on: over all 185 rounds the recording is at most 0.13 s ahead of the clock over one step and 0.53 s over a whole timer.

| `kind` | What was seen | Threshold | Most in an unpaused round |
|---|---|---|---|
| `clockStall` | Between two clock writes of one timer the recording moved on more than the clock dropped, both values above zero, before the round was decided. | 1.0 s | 0.13 s |
| `clockSlow` | The same, summed over the steps of one timer. | 1.5 s | 0.53 s |
| `clockTail` | The clock was last written this long before the end of a recording whose round was not decided, with more than that left on it. | 1.0 s | none |
| `dataHole` | No record of the movement stream, which has one at every update, before the round was decided. | 1.0 s | under 0.5 s |
| `indexGap` | Two frames of the index this far apart. A gap inside another finding is that finding's `evidence.indexGap`. | 1.0 s | under 0.25 s |
| `suspended` | The header's `endtime` less `starttime` is this much more than the index covers, after taking off what the clock jumped ahead. It has no `start`: where the recording stood still is not known. | 2.0 s | 0.96 s |

| Key | What it holds |
|---|---|
| `start`, `end` | Seconds since the recording started: the two clock writes, or the two records around the hole. |
| `duration` | How long the round stood still: the clock's lag, or the length of the hole. |
| `startedAt`, `endedAt` | The same in UTC, when `timing.startedAt` is known. |
| `phase`, `time`, `clockMs` | The phase and the round clock at `start`, and what was left of the timer in milliseconds. |
| `evidence.clockLag` | Seconds the recording moved on more than the clock. |
| `evidence.behaviour` | What the movement stream did meanwhile: `noRecords` (silent for half of `duration` or more), `frozenRecords` (records, and no player's body moved for that long) or `movingRecords`. |
| `evidence.movementRecords` | Movement records between `start` and `end`. |
| `evidence.headerMinusIndex`, `evidence.clockAhead` | On `suspended`: the two numbers it is the difference of. |

- **What is not a pause.** A new timer: the clock rises when action starts, and at a plant `IsDefuserStarted` turns 1 between the two writes. A clock at zero: it stands at `0:00` while a plant finishes (6.0 s in the seventh test round), and the frame that decides the round writes 0 just before `TimerState` 3. Everyone standing still: in the same round nobody moves for 6.2 s while the clock runs on.
- **Skipped game time** is the opposite sign: the clock jumps ahead of the recording (see [File format notes](#file-format-notes)). **A recording gap** is `timing.gaps` and `timing.holes`: frames or movement records missing for a moment, 0.5 s at most in the rounds checked, and only past a second do they count here. **A suspended recording** is seen from the header alone: `timing.headerMinusIndex` is 1 to 4 ms in the test rounds, up to 0.36 s in a real round without skips, and up to 4.8 s in one with them, all but 0.96 s of which the clock's jumps account for.
- **Who paused is not recorded**, as far as is known, so an entry names nobody. Nor is there a reason or a tactical timeout to tell apart.
- **An unknown `TimerState`.** Only 0, 1 and 3 are ever written. Any other value is listed in the warnings of `decodeStatus.pauses` with its time and count: it would be the first sign of a pause the game records itself.
- **What is needed.** A custom match recorded with a deliberate pause of known length. It would show whether the clock stops, whether frames go on, and whether a property marks it; until then the thresholds are margins, not measurements of a pause.

A match folder adds `breaks[]`, one per pair of consecutive rounds read (none across a missing round):

| Key | What it holds | How |
|---|---|---|
| `afterRound` | The round the break follows, from 1. | Decoded |
| `duration` | Seconds from that round's `endtime` to the next round's `starttime`. | Decoded |
| `banPhase` | The bans in force differ between the two rounds: operators were banned in the break. Absent on a header-only read. | Decoded |
| `sideSwitch` | The teams changed sides. | Decoded |
| `overtime` | The round before or after is an overtime round. | Decoded |
| `expected`, `excess` | The median of the match's breaks with no ban phase, no side switch and no overtime, and how much longer this one is. A match with fewer than three such breaks (Ranked bans before every round; a 1v1 switches sides after every round) gives the median of its commonest kind of break to the breaks of that kind, when it has three. | Inferred |
| `pauseSuspected` | `excess` is over 20 s and the break has no ban phase and no side switch, or is of the kind the others it is compared with are. Absent without `expected` or `banPhase`. | Inferred |

- A break is the time between two recordings, not between two rounds: it holds the end-of-round replay, the operator picks, and whatever else the lobby waited for. It can say a break was long. It cannot say why, and a pause shorter than the spread of a match's breaks does not show.
- The test match: 31.6, 28.2, 95.4, 31.7, 27.9, 180.9, 30.5, 73.1 and 59.0 s. Operators were banned after rounds 3, 6 (the side switch) and 9. The other six give 31.0 s as `expected`, and the 73.1 s after round 8 is `pauseSuspected`: nothing in the two rounds explains it, and nothing confirms a pause either.
- Ranked bans before every round, so its breaks have no plain ones and are compared with each other: none of the real folder's Ranked breaks is `pauseSuspected`. They are 57 to 88 s, each within 10 s or so of its match's median, 17 s once before a file the game did not finish, and 7 to 10 s shorter before overtime.

## Movement

`--movement` adds a `movement` block (Y11S3+, full reads): where every player is, where they look and what their body is doing, at every update the game recorded. It is left out by default because it is large, about 3 MB of JSON a round. The library equivalent is `ReadOptions { movement: true, .. }` and `Round::movement`.

`movement.players[]` holds one track per player. A track starts when the game shows the body (defenders at the start, attackers as prep ends) and ends with the first sample of the dead body.

| Key | What it holds | How |
|---|---|---|
| `time` | Seconds since the recording started, one per sample: the `recordingTime` every other event carries, so kills, health and phases line up with it. | Decoded |
| `x`, `y`, `z` | Metres, in the map's coordinates, at the player's feet; `z` is the height. | Decoded |
| `yaw`, `pitch` | Where the player looks, in degrees. Yaw 0 looks along +y and 90 along -x; pitch is positive upwards. | Decoded |
| `speed` | Metres a second over the ground, from the positions of the last quarter second. | Derived |
| `stance` | `standing`, `crouched` or `prone`. | Decoded |
| `aiming` | Whether the player aims down sights. | Decoded |
| `gait` | `still`, `creeping`, `walking`, `running`, `sprinting`, or `animated` while an animation moves the body. | Decoded |
| `doing` | `nothing`, `vaulting`, `onDrone`, `downed`, `dead`, `rappelling`, `reviving` (both the player revived and the one reviving) and `interacting` (the 8 seconds of a defuser disable, and half-second stretches whose cause is not known). | Decoded; `reviving` and `interacting` seen on few events |
| `deploying` | Whether the hands are putting something in place: a reinforcement, a barricade, a gadget. | Inferred |
| `airborne`, `falls` | Whether the player is in the air, and each stretch in the air that ended a metre or more lower, with the `drop`. A hatch or a window is a drop of a storey; the file does not say which it was. | Decoded, derived |
| `rope`, `inverted` | On a rope: `attaching`, `mounting`, `hanging`, `moving`, `stopping`, `running` (fast travel: down, or along the wall and round corners), `flipping`, `entering` (through a window), `entryAborted`, `leaving`; `off` otherwise. `inverted` is whether the player hangs head down. | Order decoded, names inferred |
| `lean` | `left`, `right` or `none`. | Inferred, unchecked |

`time` to `speed` are columns: element `i` of each belongs to sample `i`. The others are lists of `{time, value}`, a value holding from its `time` until the next entry, so a state costs nothing while it does not change. A value the tables do not know reads `{"other": n}`.

`movement.views[]` lists what each player looked through when not their own eyes: `username`, `kind` (`drone`, `camera`, or `teammate` for a dead player following one), the `device` id, its `owner`, `fixed` for a camera of the map, a camera's `position`, and `start` and `end`. One entry per device, where `observation` merges a run of cameras into one session.

`movement.placements[]` lists what players put in place (the defuser is not among them; see [Objective](#objective)): `kind` (`reinforcement`, `barricade`, `gadget`), who, the `asset` and `object` ids, the `position`, the `time` the placing started and the `end`, when the player's hands were done with it. A reinforcement takes 4.5 seconds and a barricade 2.9; one that ends sooner was given up.

Where the data lives:

- **The movement stream** holds one message per object and update. A message starts with a byte of flags, one for the transform and one for each class the object was created with, and the sections follow without lengths, so a body reads only because all five of its sections are understood. The layout is at the top of `src/movement.rs`. Every field is sent only when it changes; a sample carries the last value forward.
- **No message has a time.** A sample belongs to the frame of its record, and the frame index gives the seconds. A body has at most one message per record: about 28 samples a second, 35 ms apart, in a spectator's recording and in a player's own alike, whatever the frame rate of the index.
- **Posture is one block.** Each body sends a 722-byte block of its character's state with nearly every update. Stance, aiming, gait and the rest are numbers at fixed offsets in it (listed at `State` in `src/movement.rs`).
- **Drones and placed things name their player** in a section of their own, which is how a drone gets its owner and a reinforcement the player who put it up.

How it was checked, on the ten test rounds:

- Every update of every body reads to its last byte (about 630,000 messages with four real recordings added), and each track starts at the body's `spawnPosition`. No two samples of a track are more than 2 metres apart.
- At a kill the killer's yaw points at the victim within 10 degrees in over nine of ten kills; the rest are gadget kills and flicks. More than four of five killers were aiming a quarter second before.
- A victim's track ends within a second of the kill, and a downed player reads `downed` at the down.
- Standing players are faster than crouched ones, and those faster than prone ones. Only attackers rappel; only defenders reinforce, and nine of ten reinforcements take the 4.5 seconds.
- Every view session resolves to a device and, unless it is a camera of the map, to its owner. That held for 1,665 sessions in 18 real rounds too.

What is not there, or not known:

- **No room names.** The game's callout ("2F Aviator Room") is not recorded for players: every per-player property, every text in the file and every record type of the state stream were searched in 31 rounds. Room text exists only for two abilities, the room of each of Fenrir's mines and of each enemy Solid Snake's radar marks. A player's room has to come from their position and a table per map.
- **No floor.** `z` is the height; which floor that is needs the map's floor heights from outside the file.
- **Lean is unchecked.** The value sits next to the stance, takes three values, flips from side to side directly and clears on a sprint, but it is set far more than expected (four tenths of all samples in the test match) and nothing in the file tells a lean from a side the weapon is held to. Compare it with a moment you know before relying on it; `decodeStatus.lean` says `inferred`.
- **Planting is not a `doing` value.** It shows as `deploying`; who planted and when is in `activity.interactions`.
- **Thrown gadgets have no placement.** Grenades, launchers and thrown devices name no player in the movement stream; the loadout's `uses[]` gives the time.
- **Rope names are read from movement.** The values come in the same order on every rope checked; calling one a flip or an entry is a reading of how the body moves during it.

## Activity

From Y11S3 a full read adds `activity`: what the HUD objects of the state stream say each player did. Every entry names the player and carries seconds since the recording started, like `recordingTime`.

| Key | What it holds | How |
|---|---|---|
| `defuser[]` | Each stretch a player carried the defuser: `username`, `start`, and `end` unless they still had it when the file ended. It ends at a plant, at the carrier's death or down, or when it is dropped; `objectiveState.bomb.carrier[]` says which. | Decoded (`HasDefuser`) |
| `interactions[]` | Each plant or disable: `username`, `kind`, `start`, `end` and `outcome`: `Completed`, `Aborted` (given up, or the player died) or `Unfinished` (the round was decided first). One not completed adds `remaining`, the seconds it still had to go of the 7 it takes. | Decoded (`DefuserInteractionType`, `IsDefuserStarted`, the countdown text) |
| `equipped[]` | Each change of what a player holds: `Nothing`, `Drone`, `Primary`, `Secondary`, `Ability` or `Gadget`. `Nothing` is a player busy with their hands: placing, reinforcing, planting, between two weapons. | Decoded (`EquippedWeaponType`) |
| `reloads[]` | Each reload starting and ending, per weapon. | Decoded (`IsReloading`) |
| `ability[]` | Each change of a signal on the ability or gadget slot: `slot`, `signal` and `value`. See below. | Mostly inferred |
| `reinforcementPool[]` | The reinforcements a defending team has left. It drops when a player starts one and rises when one is given up; `movement.placements` says who. | Decoded |

Ability signals:

| `signal` | Meaning | |
|---|---|---|
| `Equipped` | The item is in hand, or a toggled ability is running: for Vigil it is 1 exactly while the cloak drains. | Checked on Vigil and Thermite |
| `Cooldown` | 2 while the ability cools down. | Checked |
| `Active` | A placed device is armed and waiting. | Inferred |
| `GaugeState` | For abilities with a gauge (Vigil, Caveira, Nøkk, Warden, Clash, Solis): 0 idle, 1 draining, 2 locked after use, 3 refilling. | Inferred |
| `Extended` | Montagne's shield is extended. | By name |
| `ShieldEquipped` | Blackbeard's shield is up. | By name |
| `DeviceState`, `Tracking`, `Activating`, `ScreenActive`, `CallState` | Solis, Deimos, Thatcher and Dokkaebi: the game's own property names, values as written. | By name |

"By name" means the property's hash is the CRC-32 of that name and it changes while the operator uses the ability; the values were not checked against the game. Operators not listed show their ability only as `Equipped` and as the count dropping in `loadouts[].ability.uses`. No property says that Glaz's scope or IQ's scanner is on, or that Jackal or Lion is scanning.

Checked on the ten test rounds and 167 real ones: only attackers carry the defuser and never two at once, every plant starts with the carrier, and every plant or disable the kill feed completes has a `Completed` interaction by the same player.

Not recorded: what a player interacts with beyond the defuser. No per-player property names an interaction or its progress. Reinforcing, barricading and placing come from the movement stream (`movement.placements`, `deploying`); a revive shows as `doing: reviving` on both players, and the state stream names no reviver.
## Gadgets, world and destruction

From Y11S3, full reads of files the game finished writing carry what was put into the world and what happened to it. The `movement` stream is read once into a list of every entity and map object with what its updates said over time, and the `FXChannel` stream once into a list of effects; everything below is read from those two lists and the HUD. Numbers are from the 10 test rounds and from the 175 rounds of a real `MatchReplay` folder (15 maps) that have a world.

| Key | What it holds | How |
|---|---|---|
| `gadgets[]` | Every gadget object: what, whose, where, when, its states, and how it ended. | Read; the name from the loadout slot or a table; who destroyed it inferred |
| `mapCameras[]` | The map's default cameras, with when each was destroyed. | Read |
| `deviceRemovals[]` | Drones and cameras destroyed, with who and how. | The event read; who and how inferred |
| `gadgetStatuses[]`, `trapTriggers[]` | Statuses and trap triggers of things that are no entry of `gadgets[]`. | See [What happens to gadgets](#what-happens-to-gadgets) |
| `scoreChanges[]` | Every change of a player's score. | Read; the reason inferred |
| `reinforcements[]`, `barricades[]` | Every reinforcement and barricade: who, where, when, on what. | Read; a removal by hand by proximity |
| `destruction[]` | What damaged the map and its panels, bullets apart. | Read; what a map object is from a catalog |
| `surfaces[]` | Holes and patches of bullet holes. | Derived |
| `breaches[]` | Every breach device and what became of it. | Read; what stopped one is a guess |
| `areas[]` | Smoke, fire, gas and swarm areas. | Read; the owner by proximity, a cloud's radius assumed |
| `environment[]` | Gas pipes, fire extinguishers and metal detectors set off. | Read; which object and who by a table or by proximity |
| `lightScreens[]` | The light screens of R.O.U. Projector Systems. | Read; the owner by proximity |

Nothing inferred is given as read: an inferred value has a field next to it that says where it is from (`bySource`, `meansSource`, `causeSource`, `usernameSource`, `kindSource`, `radiusSource`, `inAreaSource`, `jammerSource`, ...), or `inferred: true` or `derived: true`.

### The world

A snapshot or record of the `movement` stream is a `u16` count and that many messages, `u64 object, u32 size, payload`. A payload is one of four:

```text
617385fe create  +4 u64 id, +16 3 x f32 position, +28 4 x f32 rotation, +45 u64 archetype,
                 +53 u32 n, n class hashes, u64 asset, u32, u32 count, count x {u64 item, slot hash, u32}
627385fe create of a map object: the same with one class and no slots, 131 bytes
637385fe delete
607385fe update  u8 mask
  mask & 80        u8 sub, then in this order
     sub & 01      f32 x, y, z (metres, z up), u32 0
     sub & 02      f32 x, y, z, w rotation quaternion
     sub & 04      u8 live: 1 while the object is in use
     sub & 08      u16 flags
     sub & 10      u8 n, n x attach or detach operations
  mask & (40 >> i) the component of class i of the create message
```

The `u16` mask the sections above describe is these two bytes read together. `sub` `1f` is the full state an object is created with. The game creates what a round may need long before it is used, at (0, 0, -100), and moves an object to its place when a player uses it.

| Class | Component |
|---|---|
| `4c60869a` placed | `u8 m`; `01`: `u16` type index, `u8` variant; `02`: `u64` playerid of who places it; `04`: `u64` the object it is fixed to |
| `8490f616` owner | `u8 m`; `01`: `u16` type index; `02`: `u64` playerid; `04`: `u32` alliance; `08`: `u8` released (1 out of the hand, 0 taken back) |
| `513b13b2` state | `u8 m`; `01`: `u8`; `02`: `u8`; `04`: a state machine's blob, a hash and the state |
| `587f5a72` device | A camera or a drone: its own id, field of view, and in its full form a destroyed byte, a captured byte and the object it is mounted on |
| `6ea51c35` damage | `u32 count`, then per entry `fe` (the object is destroyed) or a record of 109 bytes and 40 per impact |

A component of any other class has no known size: the walk of a message stops there and keeps what it read, which happens in about one update in fifteen. A damage record holds a kind byte (0 not a bullet, 1 another player's bullet, 2 the recording player's own bullet as their game predicted it, 3 the same bullet confirmed), the point struck in the object's own space, the body or gadget that did it, a damage id that says what did it, and the impacts with their normals.

A record of `FXChannel` is a `u8` mask and one section per set bit, each a `u32` count and its entries: `01` spawn (36 bytes: the effect asset, the object it is attached to, an instance number, the object it plays on), `04` int, `08` float, `10` vector and `20` quaternion parameters of an instance, `02` stop, `40` a list of points (the cells of a fire, gas or swarm area), `80` sound. The sections come in that order, not in the order of the bits.

| | Test rounds (10) | Real rounds (175) |
|---|---:|---:|
| Updates of followed objects read | 1,028,173 | 13,850,502 |
| Updates that did not hold what their mask promises | 0 | 0 |

Every effects record of those rounds (422,731) reads to its last byte.

### Gadgets

A placed gadget is an entity with the placed component that is no panel and not the defuser. A thrown one has the owner component and one of three class lists; drones have more classes and stay in `throws`. One entity can be several gadgets in a round, one after the other.

| Key | What it holds |
|---|---|
| `entity` | The entity's id, in hex. |
| `kind` | `placed` or `thrown`. |
| `typeIndex`, `asset` | The type index its component states, and the object's asset id. |
| `name`, `nameSource`, `inferred`, `slot` | The name of the owner's loadout slot whose asset is the object's (`slot`), else the asset's row in a table of 228 assets (`table`), else the type index's row in a table of 72 (`typeIndex`). `inferred` marks a name no loadout slot ever gave: what a launcher fires and what a gadget leaves behind. |
| `username`, `usernameSource` | The player its component names. The expanded Kiba Barrier, the posts of an R.O.U. Projector System and the deployed D.O.M. panel name nobody and take the owner of the nearest thrown object that left them behind (`nearest`, with `parent`). |
| `position`, `rotation`, `origin` | Where it was deployed or came to rest, and where its placement started or it left the hand. |
| `host`, `hostKind` | The object it is fixed to, and `reinforcedWall`, `reinforcedHatch` or `barricade` when that is a panel. |
| `placing`, `released`, `deployed`, `rested` | When the placement started or it left the hand, when it went live, when a thrown one came to rest. |
| `cancels` | Placements of this object called off before this one. |
| `states[]` | Each state of its state machine: the names `Invalid`, `Closed`, `Opening`, `Opened`, `Closing`, `Idle`, `InAir`, `Landing` where the state is the CRC-32 of one, else the four bytes. |
| `statuses[]`, `triggers[]` | What was put on it, and each time it went off as a trap (below). |
| `end` | How it left play (below). |

A count dropping in the HUD (`loadouts[].ability.uses`, `gadget.uses`) says a gadget was used; the object says it was deployed. In the test rounds every one of the 118 count drops of a placed gadget has exactly one object of that player and name, deployed from 0.8 s before the drop to 0.3 s after it (an Armor Pack's count drops up to 2.1 s after).

| | Test rounds | Real rounds |
|---|---:|---:|
| Gadgets | 499 | 5,563 |
| Placed / thrown | 205 / 294 | 2,022 / 3,541 |
| Named by the loadout slot / by the table | 269 / 230 | 4,092 / 1,471 |
| With an owner | 499 | 5,549 |
| Fixed to a reinforcement or a barricade | 18 | 72 |

**How a gadget ended.** `end.signals` is what the entity showed, read: `returned` (the owner flag back to 0), `notLive`, `broken` (an `fe` entry in its damage list), `destroyedFlag` (a camera's), `inert` (flags `4000` on barbed wire or a Welcome Mat, which stay where they are), `deleted`, `pooled`. `end.goneAfter` is the seconds from the first signal to the object going. The same signals mean a detonation for one type and a destruction for another, so by themselves they give `end.how` as `presentAtEnd`, `destroyed` (`broken`, `destroyedFlag`, or barbed wire gone inert), `wentOff` (a Welcome Mat gone inert), `pickedUp` or `removed`.

`end.cause` then says more, and where it is from (`causeSource`):

| `cause` | `causeSource` | Told by | Test | Real |
|---|---|---|---:|---:|
| `destroyed` | `score` | An opponent's score rises by the gadget's points as it goes (below). | 57 | 723 |
| `destroyed` | `signals` | Read: the destroyed flag, an `fe` entry, a wire gone inert, with no scorer. | 1 | 18 |
| `intercepted` | `adsFired` or `score` | A projectile deleted as an Active Defense System fires. | 0 | 9 |
| `detonated` | `type` | A grenade, charge or canister deleted and nothing else; a Razorbloom past `Opening`. | 100 | 874 |
| `triggered` | `type` or `signals` | A trap deleted without first going out of use; a Welcome Mat gone inert. | 2 | 88 |
| `used` | `type` | A Mag-NET deleted and nothing else. | 3 | 55 |
| `pickedUp` | `hudGadgetState` | The HUD's state of one of the owner's gadgets of that name goes back to 0. | 0 | 24 |
| `roundEnd` | `time` | Deleted in the last second of the recording. | 0 | 7 |

A cause turns a `how` of `removed` into `destroyed`, `wentOff` or `pickedUp`, with `end.source` `inferred` unless the cause was read from the signals. A gadget with no cause keeps what the signals said: 211 ends in the test rounds and 1,683 in the real ones stay `removed`. Among those are the flash charges of a Candela, the pellets of an X-KAIROS and the posts of a light screen, for which no rule is written.

**Pick-up has no byte of its own.** A gadget taken back shows the same `notLive` and delete as one destroyed. `pickedUp` with `source: inferred` and no cause is a placed gadget deleted 0.7 to 1.1 s after it went out of use, the time it takes to pick one up (21 in the real rounds); the HUD rule above found 24.

### What happens to gadgets

**Who destroyed a gadget is not in the file. There is no feed for it.** What is read is the scoreboard: `MatchScore` on each player's scoreboard object, one total per frame. `scoreChanges[]` lists every change as `username`, `delta`, `total`, and a `reason` with `reasonSource: "coincidence"`: what else happened in the same frames (a kill, an assist, a gadget deployed, a reinforcement, a gadget destroyed, a trap going off). 202 of 1,015 changes in the test rounds and 2,635 of 14,740 in the real ones have no reason.

`by` with `bySource: "score"` is the player whose score rises by the gadget's points from 0.25 s before to 0.13 s after the removal: 10 for most gadgets, 20 for a Black Eye, Welcome Mat or Claymore, 5 for a T.R.I.P. Connector, and a teammate's -10 first (`friendly`). One write pays for as many removals as its amount covers. When more removals were in the window than it pays for, those it took say `ambiguous`.

| | Test rounds | Real rounds |
|---|---:|---:|
| Gadgets with a scorer | 57 | 724 |
| Of those `ambiguous` / `friendly` | 9 / 4 | 42 / 50 |
| Drones and cameras that say destroyed, with a scorer | 62 of 62 | 634 of 659 |
| Removals with a scorer where a shot of anyone passes the gadget: the scorer is among the shooters | 87 of 89 | 947 of 959 |

That last row is the check on the rule: shots are read from another part of the file, and in 1,034 of 1,048 cases (98.7%) the player the scoreboard paid is one of those whose shot passed within 0.6 m of the gadget in the 0.45 s before. In 13 of the other 14 the shooter was the scorer's teammate.

`means` says how: `bullet` with `meansSource: "shotRay"` when a shot of that player passes the gadget (with `weapon`), else `explosion` with `meansSource: "proximity"` when an explosive of theirs ended within 0.35 s and 8 m. Of the gadgets with a `by`, 26 and 13 in the test rounds and 363 and 200 in the real ones have one; 18 and 169 have none. A gadget destroyed by a cause that gives no score (its owner's own explosive, fire) has no `by`.

Drones and the cameras of the map are no entries of `gadgets[]`. Their removals are `deviceRemovals[]` (65 and 674), the same fields with the entity, its `name` and its owner. A removal of an entity that is no gadget and has no known cause is left out and counted in `decodeStatus.gadgetEvents` (25 and 238): swarms, panes, pooled objects.

**Statuses** are effects the game spawns on a gadget, until their stop: `empDisabled`, `frozen`, `hacking`, `hacked`, `caught` (by a Mag-NET), `adsFired`; `captured` is the alliance of the owner component being rewritten. Which effect asset is which status is inferred from who is in every round it shows in. `by` is the player who scored for it (`bySource: "score"`): 15 of 24 in the test rounds, 249 of 368 in the real ones. A status of something that is no gadget of `gadgets[]` (a drone, a camera's mount: 2 and 35) is in `gadgetStatuses[]` with its entity.

**Traps.** `gadgets[].triggers[]` has one entry per time a trap went off: `marker` (what says so), `victims[]`, `nearestEnemy` with its distance, and the `points` its owner scored.

| Trap | Marker | Test | Real |
|---|---|---:|---:|
| Razorbloom Shell | Its state machine goes from `Closed` to `Opening` | 8 | 73 |
| Banshee Sonic Defense | An effect on it | 5 | 50 |
| Proximity Alarm | An effect on it | 0 | 45 |
| F-NATT Dread Mine | The HUD's list of Fenrir's mines: `State` becomes 3 | 3 | 45 |
| Gu Mine, Entry Denial Device, Claymore, Grzmot Mine | Deleted without first going out of use | 2 | 84 |
| Welcome Mat | Flags `4000` | 0 | 4 |

A victim is a hit or a status effect of the type the trap deals, in the same frames (`victimSource: "time"`): 11 of 18 triggers in the test rounds and 208 of 301 in the real ones name one. A mine of Fenrir's list that no entity was matched to is in `trapTriggers[]` (4 in the real rounds).

### Reinforcements and barricades

Whatever closes a wall, a hatch, a door or a window is one kind of entity: the placed and the damage component and two empty slots. Its asset says which panel it is. A panel starts with its first position that is not the pool, with the playerid of who places it; it is complete at `live` 1, and the object it is fixed to (`host`) is written then or up to four frames later. One that went back to the pool before that was called off (`cancelled`); one placed with no owner is the map's own (`default`).

| | Test rounds | Real rounds |
|---|---:|---:|
| Reinforcements completed (wall / hatch) | 91 (68 / 23) | 1,292 (1,198 / 94) |
| Called off | 2 | 42 |
| Opened | 27 | 229 |
| Barricades the map placed | 180 | 3,295 |
| Barricades players placed (Castle's) | 32 (0) | 134 (79) |
| Barricades destroyed | 101 | 685 |

A wall reinforcement takes 4.08 s from start to complete (1,245 of 1,266 within 0.1 s of that), a hatch reinforcement 4.41 s, a barricade 2.53 s and Castle's 2.82 s. `width` is by the asset and inferred (`widthInferred`): nine assets, taken to run from 1.6 to 2.4 m.

**Reinforcements are a pool of 10 a team shares**, not two per player: no team put up more than 10 in any of the 185 rounds, 52 teams put up exactly 10, and one player put up three in a test round. `reinforcements[].opened` is when the panel's intact flag was cleared (flags `8000` to `0000`); a hatch reinforcement that is opened also gets an `fe` entry (`destroyed`).

How a barricade ended is the last record of its damage list up to its `fe` entry, at most 2 s old: `bullet`, `melee`, `explosion`, `gadget` (with the gadget and its owner) or `other`, all with `source: "read"` (100 of 101 in the test rounds, 574 of 685 in the real ones). With no such record the barricade was taken down by hand or gone through, which the file does not say: the nearest body within 2.5 m is named with `source: "proximity"`, as `removed` when it stands 0.3 to 0.5 m from the panel and `brokenThrough` otherwise (1 and 111).

### Destruction, surfaces and breaches

`destruction[]` has one entry for the records of one instigator and damage id that follow each other within 0.15 s: `cause` (`id`, `name`, `category`), `username`, the `instigator` when it is a gadget, the `position` of the first record, and `objects[]` with `kind`, `impacts`, and `opened` or `destroyed` when that followed within 0.35 s. `username` is the player whose body the record names, or the owner of the gadget it names.

- **Bullets are left out**: `shots` has them. Their impacts are counted in `surfaces`.
- **Debris is left out.** Records of the cause `Physics collision (debris, props)` name no player; they are 508 of 980 events in the test rounds and 2,717 of 7,904 in the real ones, and `decodeStatus.destruction` counts them.
- **What a damage id stands for** is a table built from 184 rounds. A name confirmed by the entity or player that did it is read; one worked out by exclusion has `inferred: true`.
- **Hole size is not in the file.** A record gives impact points and normals, no extent.

**Wall, floor or hatch is not in the file either.** A panel says what it is by its asset (`reinforcedWall`, `reinforcedHatch`, `barricade`). A map object does not: its `kind` comes with a `kindSource`.

| `kindSource` | What it is | Objects of events, test | Real |
|---|---|---:|---:|
| `catalog` | A table of 2,483 objects of 15 maps, built from the impacts on each object over 184 rounds | 440 | 4,305 |
| `impacts` | The vote of this round's bullet and melee impacts on the object, when four in five agree: a floor has impacts with a vertical normal in its own plane, a wall has them along one axis within its thickness | 24 | 540 |
| `derived` | A hatch known by the record a hatch reinforcement leaves on the hatch under it | 8 | 19 |

**`surfaces[]` is derived, every entry of it** (`derived: true`): the impact points of bullets and of causes that remove material, on walls, floors and hatches, put together when closer than 0.45 m in one plane. `width` and `height` are the extent of the points, not the size of a hole. The labels are this parser's rules of thumb with its own thresholds, not the game's:

| Label | Rule | Test | Real |
|---|---|---:|---:|
| `verticalPlay` | On a floor: an explosive, breach or ability cause, a hatch that was destroyed, or 8 points and more | 80 | 744 |
| `rotationHole` | On a wall: such a cause, or 12 points over 0.6 x 0.9 m and more, most of them by defenders | 84 | 414 |
| `breach` | The same, most of them by attackers | 30 | 358 |
| `murderHole` | On a wall: a melee hit, or 8 points within 0.6 m | 40 | 358 |
| `bulletHoles` | Anything else of three bullet impacts and more | 321 | 2,230 |

`breaches[]` has one entry per breach device: a charge a player places (Hard Breach Charge, Breach Charge, Exothermic Charge, S.E.L.M.A. Aqua Breacher), a breaching projectile (Ash, Zofia, Gonne-6, Kali), an X-KAIROS volley, or a run of Maverick's torch on one object. `outcome` is read: `detonated` (records name the device), `destroyed` (`live` back to 0 with no record), `removed`, `armedAtEnd`, `noDestruction`, `burned`. `affected[]` lists the objects its records landed on, `openedReinforcement` says one of them is a reinforcement it opened, and `reinforcedBy` names who had put that reinforcement up.

| | Test rounds | Real rounds |
|---|---:|---:|
| Breach devices | 28 | 591 |
| Detonated / burned | 25 / 0 | 369 / 173 |
| Opened a reinforcement | 20 | 171 |
| Destroyed before going off | 1 | 31 |
| Of those, with a `stoppedBy` | 0 | 28 |

Every Exothermic Charge (7) and X-KAIROS volley (4) of the test rounds went off and opened a reinforcement.

**What stopped a breach charge is not written.** A charge that died without going off gets `near[]`: a Shock Wire or Electroclaw within 2.5 m, a defender's bullet within 1.3 m, an explosion within 3 m, each within 0.3 s. `stoppedBy` (`electricity`, `shot`, `explosion`) names one of those with `stoppedBySource: "proximity"`: 21, 3 and 4 in the real rounds. It is what was near, not what did it. With nothing of the kind near, nothing is named: the one destroyed charge of the test rounds, a Hard Breach Charge on a reinforced hatch that died 0.14 s after it was armed, has no `stoppedBy`.

### Areas, environment and light screens

An area is an effect with a position and no parent, of one of seven assets. It starts in the frame of its spawn and ends in the frame of its stop.

| `kind` | From | Lasts | Test | Real |
|---|---|---|---:|---:|
| `smoke` | A smoke grenade or a smoke bolt | 14.0 s, 17.0 s | 5 | 77 |
| `fire` | A Volcan canister, a Shumikha grenade, a fire bolt, a gas pipe, a Logic Bomb | 1.8 to 19.9 s | 33 | 207 |
| `gas` | A Remote Gas Grenade | 9.8 s | 10 | 33 |
| `swarm` | A Kawan hive | 15.8 s | 13 | 79 |
| `extinguisher` | The burst of a fire extinguisher | 7.0 s | 23 | 119 |

- **Fire, gas and swarm areas carry their cells** (`points`), which the game writes again as the area spreads; the last list is kept. Their `source` is told by the entity without classes the game puts at the effect's place in the same frame: all 56 in the test rounds and all 319 in the real ones have one.
- **Smoke has no size in the file.** A smoke cloud has no cells: `radius` is 3.0 m (2.5 m for an extinguisher) with `radiusSource: "assumed"`.
- **Whose area it is, is not written with it.** `username` is the owner of the gadget that was within 0.5 m as the area started and was deleted just before (`usernameSource: "proximity"`), for a swarm the nearest owned entity within 1 m (`nearest`), for a Logic Bomb's fire the Dokkaebi whose ability count dropped 7.0 to 8.2 s before (`abilityUse`). All 56 areas that can have an owner in the test rounds and 318 of 320 in the real ones name one; the fire of a gas pipe and the cloud of a fire extinguisher have none.
- **Who set off a Volcan canister** is the shooter of a shot that ended within 0.6 m of the fire in the second before (`triggerSource: "shotRay"`), else the thrower of an object that ended within 5 m (`explosion`).

`environment[]` lists gas pipes blown up (5 and 76), fire extinguishers burst (23 and 119) and metal detectors sounding or switched off (75 and 139). A fire extinguisher is the map object its cloud is attached to, and who shot it the last body in its damage list (`bySource: "read"`). A gas pipe's explosion has no parent: the pipe is the map object a per-map table lists within 1.5 m (`objectSource: "table"`), else one found by its flags changing or by distance. Who walked through a metal detector is not written: `by` is the nearest body within 1.5 m (`bySource: "nearest"`).

`lightScreens[]` groups the posts of an R.O.U. Projector System by the projector that rolled past as each light came up, and takes that projector's owner (`usernameSource: "proximity"`).

### What is joined onto other events

| Key | What it holds | Test | Real |
|---|---|---:|---:|
| `matchFeedback[].inArea` | On a kill or a death: the fire, gas or swarm area the victim's body was in, as `{area, kind, source, username}`, with `inAreaSource: "derived"`. The body is within 1.0 m of one of the area's cells, the cell from 1.0 m below it to 2.2 m above. | 2 of 66 | 11 of 1,270 |
| `hits[].inArea` | The same for a fire or gas hit (type 36 or 9). | 24 of 24 | 86 of 92 |
| `shots[].throughSmoke` | The bullet's path from the muzzle to what it struck passes within the assumed radius of a smoke cloud that is there. The burst of a fire extinguisher does not count. | 20 of 5,375 | 751 of 63,734 |
| `effects[].jammer` | On a jammed player (type 8): the owner of the nearest Signal Disruptor within 3.5 m of the player or of a device of theirs, with `jammerSource: "nearest"`. | 5 of 5 | 218 of 218 |
| `hits[].gadgetOwner` | On a barbed wire hit (type 12): the owner of the nearest wire in use within 1.5 m, with `gadgetOwnerSource: "nearest"`. | 12 of 14 | 42 of 45 |
| `reinforcements[].opened`, `breaches[].reinforcedBy`, `gadgets[].hostKind` | See above. | | |

`stats[]`, and the match totals, count per player:

| Key | What it counts |
|---|---|
| `gadgetsDeployed` | Entries of `gadgets[]` that are in one of the player's loadout slots. What a launcher fires and what a gadget leaves behind is not counted. |
| `gadgetsDestroyed` | Gadgets, drones and cameras of the other team the player is named for (`by`). Inferred, as `by` is. |
| `gadgetsLost` | The player's own gadgets, drones and cameras that were destroyed, by anyone. |
| `reinforcements`, `barricades` | Those the player completed. |
| `breaches`, `breachesOpened` | Breach devices the player used, and how many opened a reinforcement. A soft wall has no flag that says it was opened, so a soft breach counts in `breaches` only. |
| `trapsTriggered` | Times a trap of the player went off. |

### Not recorded, or not decoded

- **Who destroyed a gadget, and with what.** Inferred from the scoreboard and the shots, as above.
- **Mute's jammer at work.** Nothing on a Signal Disruptor says what it jams. A jam shows only as the jammed player's effect; which jammer it is, is the nearest.
- **Who a Grzmot Mine stunned, and what an Airjab or a Trax Stinger did to whom.** No marker was found. Jackal's scans show only as the tracked player's effects.
- **What barbed wire hurt.** A wire has no marker as it hurts; a hit names the nearest wire.
- **What stopped a breach charge, hole size, the size of a smoke cloud, whether a map object is a wall, and whether a gadget was picked up**: see above.
- **Areas that were there when the recording started** are left out: when they started is not known.
- **What some devices are.** Some entities that say destroyed and pay a scorer name no owner and are in no loadout slot (9 in the test rounds, 96 in the real ones, nearly all of type index 300); they are listed in `deviceRemovals[]` with their asset and no `name`.

`decodeStatus` adds `world` (updates read), `gadgets`, `gadgetEvents` (removals, statuses and triggers, on a gadget or listed), `scoreChanges`, `panels`, `destruction`, `breaches`, `surfaces` (always `inferred`) and `areas`, each with a count and what was left out.

## Objective

From Y11S3 a full read adds `objectiveState`: what the round's objective did, in one place. `mode` is the header's game mode. A Bomb round (`Bomb`, `QuickMatchBomb`) adds `bomb`; any other mode has `mode` alone, because no recording of Secure Area or Hostage has been seen and nothing about them is decoded. A section for each would sit beside `bomb`.

```json
"objectiveState": {
  "mode": { "name": "Bomb", "id": 327933806 },
  "bomb": {
    "sites": [
      { "index": 1, "object": "60572f71b8", "name": "B Lockers", "position": [-54.453, 8.401, -3.8] },
      { "index": 2, "object": "60572f3500", "name": "B CCTV Room", "position": [-68.022, 7.681, -3.8] }
    ],
    "carrier": [
      { "username": "Bassetto.L5", "team": 0, "start": 0.069, "end": 218.509, "started": "spawn", "ended": "downed",
        "endedBy": "vitaking.FaZe", "position": null, "endPosition": [-49.762, 12.512, -3.801] }
    ],
    "drops": [
      { "username": "Bassetto.L5", "team": 0, "reason": "downed", "by": "vitaking.FaZe", "time": "0:06", "phase": "Action",
        "elapsed": 218, "recordingTime": 218.442, "position": [-49.762, 12.512, -3.374], "restPosition": [-49.762, 12.512, -3.804],
        "pickup": { "username": "PSYCHO.L5", "time": "0:04", "phase": "Action", "elapsed": 220, "recordingTime": 220.141,
                    "secondsOnGround": 1.699, "byOther": true } }
    ],
    "plants": [
      { "username": "Bassetto.L5", "team": 0, "side": "Attack", "start": 215.315, "end": 218.442, "outcome": "Aborted", "remaining": 3.908,
        "time": "0:09", "phase": "Action", "elapsed": 215, "position": [-49.758, 12.509, -3.801], "endPosition": [-49.762, 12.512, -3.801],
        "defuserPosition": [-49.752, 12.513, -3.801],
        "site": { "index": 1, "name": "B Lockers", "source": "nearest", "distances": [6.25, 18.9] } }
    ],
    "disables": [
      { "username": "Handyy.FaZe", "team": 1, "side": "Defense", "start": 263.959, "end": 270.979, "outcome": "Completed",
        "time": "0:11", "phase": "Planted", "elapsed": 264, "position": [-49.521, 12.199, -3.801], "endPosition": [-49.521, 12.199, -3.801],
        "defuserPosition": [-49.321, 12.286, -3.801],
        "site": { "index": 1, "name": "B Lockers", "source": "decoded", "distances": [6.44, 19.26] },
        "defuserTimeLeft": 11.877, "defuserTimeLeftAtEnd": 4.856 }
    ],
    "plantedAt": 230.873, "defuserTimer": 45, "defuserTimeLeft": 4.856
  }
}
```

That is round 7 of the test match, shortened to one entry per list.

| Key | What it holds | How |
|---|---|---|
| `mode` | The game mode, as in the header. | Decoded |
| `sites[]` | The round's two bombs: the game's number for each (`index`, 1 or 2), its `object` (the same in every round on the map), its `position`, and `name`, the header's two site names taken in order. | Decoded; the names by order, assumed |
| `carrier[]` | Each stretch of `activity.defuser[]` with the player's `team`, how it `started` and `ended`, `endedBy` (who downed or killed the carrier), and the carrier's `position` at the start and `endPosition` at the end. | Stretch decoded (`HasDefuser`), reasons derived |
| `carrier[].started` | `spawn`: the game gave the player the defuser, before action started or from a carrier it took it from. `pickup`: they took it off the ground. | Derived |
| `carrier[].ended` | `planted`, `downed`, `died`, `dropped` (put down), `roundEnd` (still carrying when the round was decided or the file ended) or `reassigned` (the game gave it to someone else). See below for how each is told. | Derived |
| `drops[]` | Each time the defuser left its carrier without a plant: who lost it, the clock (`time`, `phase`, `elapsed`, `recordingTime`), `position`, where it was let go (at the carrier's hands), and `restPosition`, where it came to lie. | Decoded (the defuser object) |
| `drops[].reason`, `by` | `downed`, `died` or `dropped`, and who downed or killed the carrier. | Derived (the timeline) |
| `drops[].pickup` | Who picked it up and when, `secondsOnGround`, and `byOther` (false when the player who lost it took it back); null when nobody did. | Decoded (the defuser object) |
| `plants[]`, `disables[]` | Each entry of `activity.interactions[]` with `team`, `side`, the round clock at its start (`time`, `phase`, `elapsed`), the player's `position` at the start and `endPosition` at the end (null for one the round's end cut short), and `remaining`: the seconds one that did not complete still had to go. | Decoded |
| `defuserPosition` | Where the defuser is planted: for a plant where this one put it, completed or not; for a disable where the round's plant did. | Decoded (the defuser object) |
| `site` | The bomb a plant or disable is at: `index`, `name`, `source` and `distances`, the metres over the ground from the defuser to each of `sites`. `source` is `decoded` when the game names the bomb the defuser is on, which it does for a completed plant in a spectator's or an attacker's recording, and `nearest` otherwise. | Decoded or derived, as `source` says |
| `plantedAt` | Seconds since the recording started when the plant completed. | Decoded (`IsDefuserStarted`) |
| `defuserTimeLeft`, `defuserTimeLeftAtEnd` | Seconds left on the defuser timer: on a disable at its start and at its end, on the round when it was decided. The game writes the timer every dozen frames and at every frame near a whole second; a moment between two samples is taken between them, and one after the last sample is that sample, since the timer stops with the disable that completes. | Decoded (`TimerInMilliseconds`) |
| `defuserTimer` | Length of the defuser timer in seconds. The file has no such value: after a plant the round clock shows what is left in whole seconds, rounded down, and every reading of every planted round says 45. The timer's first sample is about 44.94. | Derived (the clock) |

Where the data lives:

- **The defuser is an object of the movement stream**, created in the round's opening snapshot at (0, 0, 0), where it stays while a player carries it. When it is dropped a state bit turns on and the update carries the position it was let go at; position-only updates follow as it falls, and the last is where it lies. A pickup puts it back at (0, 0, 0) and names the player, unless it is the one who dropped it. The position and rotation it is planted at arrive when a plant starts; a plant given up puts it at (0, 0, -100). The layout is at `DefuserUpdate` in `src/movement.rs`. Before, `movement.placements[]` listed it as a gadget its carrier placed at the origin; it no longer does.
- **Every site's two bombs are objects of the map**, eight on a map with four sites. The round's two say so in the snapshot, each with its number.
- **The state stream** has the timer on the clock object, the number of the bomb the defuser is on on the game-mode object, and the countdown of each plant and disable as text (`7.000` to `0.000`) on the player's interaction object; see the top of `src/activity.rs`.

How a carry's end is told. A carry is a stretch of the HUD's `HasDefuser`, which follows the defuser itself by a few frames. The first of these that fits:

1. `planted`: a completed plant by the carrier ended within 0.5 s of the carry.
2. `downed`, `died`, `dropped`: the defuser was dropped by the carrier during the carry, up to 0.5 s either side, and the reason is the drop's. A drop is `downed` or `died` when `timelineEvents` has the carrier going down or dying in the 0.5 s up to it; when both are there, the earlier one, which is the down. A drop with neither is `dropped`: a carrier who puts the defuser down and is shot afterwards stays `dropped`. A round without a timeline (2 of the real ones) uses the kill feed and `lifeEvents`.
3. `roundEnd`: the carry has no end, or ends at or after the round was decided.
4. `downed`, `died`: the carrier fell in the 0.5 s before the carry ended and the defuser never lay anywhere, because a teammate had it in the same frame. Seen once. Such a carry still gets a drop, at the carrier's feet and with no `restPosition`.
5. `reassigned`: none of these. The game gave the defuser to another player without it lying anywhere. Seen twice: once 12 seconds before action started, and once in action, from one attacker straight to another in one frame. Why is not known.

A recording whose movement stream has no defuser would fall back on the carries alone, with every drop at its carrier's feet and `dropped` for a carry nothing else explains; none of the rounds checked is one.

Positions of players are those of the body, at the feet, from the movement stream: the last sample at or before the moment, since a body that stands still sends nothing. They are there on every full read, with or without `--movement`. An attacker has no body before prep ends, so a `spawn` carry has a null `position`.

Checked on the ten test rounds and 174 real ones (288 carries, 215 drops, 103 pickups, 65 plants, 10 disables):

- The defuser and `HasDefuser` tell the same story in all 184 rounds: every drop falls in a carry of the player's, every pickup starts a carry of the player's, and every carry that lost the defuser has one drop. The HUD follows the defuser by 0.14 s at most, and in a player's own recording can be 0.005 s ahead. In 25 carries `HasDefuser` never ended although the defuser was dropped; the HUD alone would have them as `roundEnd`.
- Every `downed` and `died` drop has its timeline entry, 0 to 0.07 s before it (166 drops).
- A defuser is let go about a metre at most from where its carrier's body is as the carry ends (0.02 m at the median), and comes to lie within 1.3 m of there over the ground.
- Every plant puts the defuser within a metre of the planter, and every disable is from within 2.5 m of it. No player moves more than a metre during either.
- Of the 33 completed plants the game names the bomb of 15: every one in the spectator's recordings of the test match and 11 of 29 in players' own. The nearer bomb is the same one in 14; in the other the defuser is 7.0 m from one bomb and 8.7 m from the one the game names. `distances` shows such a call.
- The timer is there in all 33 planted rounds. At the start of a disable it is 0.01 to 0.07 s below 45 less the time since the plant, and a disable's `time` is its `defuserTimeLeft` rounded down, or a second off when the clock turns within a tenth of a second of it. When a disable ends the round, the round's `defuserTimeLeft` is that disable's `defuserTimeLeftAtEnd`.
- `remaining` is within a quarter second of 7 less the time the plant or disable ran.
- The objective is the same whether or not the movement was read.

What is not there, or not known:

- **A defuser that went off.** No round checked ended that way, so the timer reaching 0 has not been seen.
- **Two of the defuser's state bits.** `08` is lying in the world and `02` planted; `01` and `04` turn on at moments that fit no event known here (`04` some time before a disable starts), and are not in the output.
- **Which bomb is A.** The game numbers the round's bombs 1 and 2. That 1 is A, and that the header's two site names are in that order, is assumed: nothing in the file says so.
- **A header that names the wrong site.** In 1 of the 184 rounds the header's `site` is that of the round before, while the bombs in play, the defenders' spawns and the plant are at another. The bombs are right, and the names on `sites` are then wrong. A single round cannot tell; a match folder can, and `folder.warnings` names the round when the same site name goes with other bombs elsewhere in the match.
- **Why the game moved the defuser** in the two `reassigned` carries, and whether a carrier who leaves the match reads that way: no carrier left in the rounds checked.
- **Other game modes.** Secure Area and Hostage give `mode` only.

`decodeStatus` gains `objectiveState` (carries; `inferred`, or `notInVersion` for a mode other than Bomb), `defuserDrops` (`decoded`, or `inferred` without the defuser object), `bombSites`, `objectivePositions` (plants and disables with the player's position), and in a planted round `plantSite` (`decoded` when the game names the bomb, `inferred` for the nearer one) and `defuserTimer` (`decoded` when the timer was written).

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

Kill feed entries name their players' `profileID` and `targetProfileID`. From Y11S3 a kill can also carry `creditedTo`: the scoreboard credits a kill to the teammate who downed the victim when another player finished them, while the feed names the finisher. The credit is written within a frame of the feed entry, before or after it. With those credits counted, the scoreboard's kill and death totals match the kill feed in every test round and in the 178 real rounds checked.

`weaponReady` lists each change of the controller's `CanFire` flag (`ready`, `phase`, `elapsed`). Attackers hold it at `false` through prep, on their drones, until their body spawns; in action it drops while a drone or gadget is in hand, not on reloads or swaps between guns. `weaponActivity` says what a player held (see [Weapon handling](#weapon-handling)).

`decodeStatus` adds `profileIds`, `recorder`, `entities`, `movement`, `party`, `weaponReady`, `platform`, `names` and `cosmetics`. `summary.teams[].players[]` gains `key`, `relation`, `party`, `platform`, `usesNickname` and `renamedTo`, and `summary.recording.party` lists who queued with the recorder.

`--players` reads every match folder under a folder (partially: players, relations, parties, platform and names) and prints a directory: per player the `key`, `platform`, every username used with first and last seen (`nickname` marks one the game showed in place of the player's own, `atMatchEnd` one the game gave as a match ended), matches with and against you, matches in your party, and `queueMate` (queued with you once, or on your team in two or more matches). `username` is the latest match's name, the one given at its end when the recording has it. `you` lists the recording accounts. A match imported twice (same `matchID`) counts once. The library equivalent is `PlayerDirectory::new(&summaries)`.

How the links are made (details in `src/entities.rs`):

- The stream is a tree of replicated objects. Besides `23`/`22` property records, `1b <parent> <field> <child>` and `1a <field> <child>` records hang child objects off a parent. Each player has one controller object under their team's object, holding the name, operator, profile id and the header `playerid`; the scoreboard, health, inventory and a profile object hang off it.
- The profile object carries the relation to the recorder (`05c7b949`: 1 opponent, 2 teammate, 3 teammate in the recorder's party, 5 the recorder) and the party role (`af6bb287`: 0, 1 member, 2 leader). Checked on Y8S1 ranked and quick matches (a clan-tagged five-stack, a duo with randoms), Y8S2 and Y9S1.
- Movement is sent apart from the tree. A player table links each body to a player explicitly, so the link does not depend on player order. In Y11S3 the table is a stream of its own (`aca4c435`): its snapshot is the table that opens the round and each frame record a table of what changed, per player the `playerid`, the body they move and what they look through (their drone or a camera). Every player of the round has an entry, the recorder included, so the recorder's own body is linked like any other. A player with an entry and no body never spawned: they left, or the recording ended first. `decodeStatus.movement` says so without calling it a fault. The entry layout is in `src/entities.rs`.
- A player the header does not list (it is written before late joiners arrive) is read from their pick packet and takes the `playerid` their controller holds. A player who reconnects keeps their profile id, so `key` holds across the match; their `playerid` is new when the game was started again, which it was in 3 of 5 returns, and the same when it kept running. The controller's `HasLeft` is output as `seats[].hasLeft`: a seat that was filled again keeps it set (see [Leavers and reconnects](#leavers-and-reconnects)).

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

- **Streams.** Most rounds have 10 (8 to 11 seen). `state` holds the clock, kill feed, health and picks; `movement` holds every entity message. Their hashes are the CRC-32 of the game's names: `HUDChannel` (`a98fdd0b`, `state`), `EntityChannel` (`20a5c4e3`, `movement`), `ControllerChannel` (`aca4c435`, the player table), `FXChannel` (`f5ee6a3d`, effects: spawns, their parameters, stops and the cells of areas), `DecalChannel` (`5f87976f`, bullet holes and marks), `SoundChannel` (`63fe54d3`, the sound engine's commands), `MarkerChannel` (`26b9c2c1`, pings, spots and tracking markers), `TimelineChannel` (`eee42d83`, a log of kills and downs) and `WorldChannel` (`e3f6781c`, the round's timer and state); `DeferredChannel` (`be5e4267`) is always empty, and `CameraEffectChannel` (`014e3af4`, in 23 of 175 real rounds and no test round) lists the players whose screen shows Dokkaebi's call: a `u32` count and 16-byte entries (`u64` 0, `u64` player id) removed, then the same added. Its 50 added entries are the `DokkaebiCall` effects of those rounds, so it is not read. All but `DecalChannel`, `WorldChannel`, `CameraEffectChannel` and the empty one are read. `movement` and `state` have a record at nearly every update, 35 ms apart.
- **Layouts of the smaller streams.** `MarkerChannel`: `u16` count and 55-byte markers, then `u16` count and 42-byte device entries (see [Pings](#pings)). `SoundChannel`: a `u16` mask and, per set bit from the lowest, a `u16` count and that many entries: posted events (58 to 136 bytes), positions and orientations (24 bytes each), switches and parameters (16), two lists of unknown meaning (11 and 8) and stops (12); see [Metal detectors](#metal-detectors). It holds every player's footsteps and gunshots, which are not output. `WorldChannel`: a `u8` mask, then a `u32` timer length in milliseconds (bit 0: 45000 for prep, 180000 for action, 3000 or 2000 once the round is decided), a `u8` round index (bit 1, snapshot only) and a `u16` state (bit 2: 4 prep, 8 action, 16 round over). It parsed with nothing left over in 186 rounds and says nothing the `state` stream does not, so it is not read; it has no signal for a plant. `DecalChannel`: `u32` count and 82-byte entries (the entity struck, an asset, a position and a normal), then a `u32` 0.
- **Recording ids.** Stream ids come from one counter per run of the game. A round takes its main id (`recordingId`) and one per stream, and the next recording starts right after, so a skipped id is a recording that was started and never saved. The folder name ends in the same run's process id.
- **Unfinished files.** 3 of the 203 real rounds end on a block whose packed size is 0xFFFFFFFF, the game's compressor having failed on a 5 to 11 MB block. The main stream was never written and the directory holds uninitialized memory, but the frame index and snapshots survive, so the header and players still read.
- **Rates.** The index rate follows whoever recorded. Spectator recordings (the Y11S3 test rounds) index a steady 29.4 frames a second; a player's own recording indexes every rendered frame, about 300 a second on the PC checked, 0.1 to 66 ms apart. Records arrive about 28 times a second either way. The 200 to 260 a second seen in Y8 and Y9 replays fits the second kind.
- **Temporary files.** A current install has an empty `DissectTmp` folder next to `MatchReplay`, and the process id and stream id in the reported `.tmprec` names match what round files hold. No `.tmprec` file was available, so their contents are unchecked.
- **Hashes are names.** Every property, field and class hash in the stream is the CRC-32 of the game's name for it, stored little-endian: `crc32("Health")` is `0xC9762625`, written `25 26 76 c9`. Guessing a name and hashing it tests what a field is. Names found this way include `ProfileType` (`05c7b949`, the relation to the recorder), `SquadStatus` (`af6bb287`, the party role), `ClearanceLevelText`, `TeamColor`, `HeroTeam`, `BanState`, `HasLeft`, `MatchKills`, `PlayerPlatform`, `PlatformPlayerID`, `OnlinePlayerID` (the header's `playerid`), `UsesNickname`, `IsBot`, `PlayerSlotType` (1 while a player is in the slot), the classes and fields `PlayerLifeVM` (`4154dcc4`), `PlayerLoadoutVM` (`e8d1e539`), `PlayerStatsVM` (`eb219b38`), `OperatorVM` (`379e2280`), `OperatorCard` (`b62a2fc7`), `OperatorName` (`63cc188f`), `TeamVM` (`951c1650`), `GamerProfileVM` (`77b15e33`) and `GameModeInteractionVM` (`27c08dca`), the cosmetic slots (`Uniform`, `Headgear`, `WeaponSkin`, `Charm` and the rest), `LocationName` (the spawn voted for), `TimerInSeconds`, `TimerInMilliseconds` and `TimerState` on the clock object, `IsDefuserStarted`, `DefuserInteractionType`, `DefuserInteractionRemainingTime` and `HasDefuser` (who carries the defuser; not output yet), and on a feed entry `BackgroundColor` (`5934e58b`), `Message` (`e3090079`), `KillerName` (`d9133cba`), `VictimName` (`ac190f70`), `Index` (`0548b241`) and `Duration` (`96e2297f`), with `Messages` (`c07c7422`) the array the entries are in. The placeholders of a feed line are hashed the same way: `[PLAYER]` is `3c7fb10e`, `[STRING]` `6f56659c`.
- **Skipped game time.** A recording can leave out game time without a gap in its frames: two or more players who were walking are, a tenth of a second later, metres further on than anyone can run. `timing.skips` lists each such moment with `at`, `until`, the `seconds` missing (the median of what the players' speed says) and how many `bodies` jumped; it is inferred from the bodies, and `decodeStatus.timing` is `partial` with it. 22 skips of 0.3 to 1.0 s in 12 of the 175 real rounds, none in the test rounds. Whatever is timed across one is that much shorter than it was: in the one round checked against other evidence a second is missing as action starts, a wall reinforcement that takes 4.08 s is up in 3.06 s, and two melee hits of one player are 0.76 s apart where the game allows no less than 1.02 s.
- **Stray records.** Binary data between record runs can read as records. In one real round such a "record" covered a team object's first record and moved its properties to another object. Two rules reject them: an array record claiming an index of 65536 or more (real ones reach 64), and any record that does not name its object yet covers records that do (a `23` or `1b` and what follows) ending exactly where it ends.

The index is wall-clock accurate: in Y11S3, `starttime` plus the index duration lands within 5 ms of `endtime` in the test rounds and within 0.4 s in a real round without skips. A round with skips ends up to 4.8 s later than its index says; `timing.headerMinusIndex` gives the difference. The header `datetime` is the recording PC's local time, not UTC.

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

Set `R6_MATCH_REPLAY` to a game `MatchReplay` folder to also check real match folders: file and folder names agree with the headers, every round lands in one session, nothing is found twice, loadout counts add up, and what holds for the gadgets, panels, destruction and areas of any round holds for each of them. The folder changes as matches are played, so these tests check what holds for any such folder.
