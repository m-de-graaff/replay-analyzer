# Health and Damage Implementation Plan

> **To execute:** use the `executing-plans` skill. Steps use `- [ ]` for tracking.

**Goal:** Decode downs, finishes, revives, team kills, reverse friendly fire, HP with overheal, max health and plates, every damage hit, heals and status effects from Y11S3 replays into round JSON.

**Architecture:** Two new modules, each reading streams the way `src/loadout.rs` does (`RecordMap` blocks, `for_each_record`, `loadout::Clock` for times). `src/combat.rs` reads `TimelineChannel` and the hit blocks of `EntityChannel`; `src/vitals.rs` reads the life, effects and friendly-fire view models of `HUDChannel`. `Parser` calls both after `resolve_loadouts`, and a last pass joins them into stats and the README.

**Tech stack:** Rust 2024, existing dependencies only (rayon, serde, memchr). No new crates.

This plan is a hypothesis built from research on 174 rounds (10 test, 164 real). Where the bytes disagree with it, the bytes win: fix the plan's claim in the README text and say so in the hand-back.

## Global Constraints

- Y11S3+ only (`self.code() >= version::Y11S3`), full reads only. Older rounds keep today's output.
- Nothing inferred is presented as read: every inferred value carries a field saying so (`attackerSource`, `inferred: true`) and the README says how often the rule held.
- A malformed block is skipped and counted in `decodeStatus`, never a panic. No `unwrap` on stream bytes, no indexing without `get`.
- Never copy real replays into the repo. Real-folder tests read `R6_MATCH_REPLAY` and assert invariants, not counts.
- Some files are CRLF (`README.md`, `src/details.rs`, `src/stats.rs`, `src/decoder.rs`, `src/types/mod.rs` ...). Do not run `cargo fmt` over them; format only new files (`rustfmt src/combat.rs`), then check `git diff --stat`.
- Hashes are written in stream byte order: `crc32("Health")` = `0xC9762625` is `[0x25, 0x26, 0x76, 0xC9]`.
- Reference implementations (Python, proven on the data) live in the session scratchpad `S` = `C:\Users\Mark\AppData\Local\Temp\claude\D--Development-replay-analyzer\c34e2160-ed28-48c2-a421-9bf58325a491\scratchpad`: `S\damage`, `S\health`, `S\heal`, `S\status`, `S\verify`. Decompressed test rounds are in `S\dumps`.

## File map

| File | Responsibility |
|---|---|
| `src/combat.rs` (new) | `TimelineChannel` entries; hit blocks in body updates; attacker inference; `Combat` |
| `src/vitals.rs` (new) | Life view model series, heal and plate classification, effects list, reverse friendly fire, flash; `Vitals` |
| `src/round.rs` | `Round.combat`, `Round.vitals`, calls after `resolve_loadouts`, serialization, `decodeStatus` entries, fix of the false-revive rule |
| `src/details.rs` | `LifeEvent` gains `by`, `health`, `outcome`; `HealthUpdate` gains `maxHealth`, `overheal`, `cause` |
| `src/stats.rs` | `damageDealt`, `downsDealt`, `finishes`, `revivesGiven`, `teamKills`, `healingGiven`, `healingReceived`; `damageTaken` without overheal decay |
| `src/decoder.rs` | Bump the Y11S3 profile revision |
| `tests/combat.rs`, `tests/vitals.rs` (new) | Test rounds and real-folder invariants |
| `README.md` | Output table rows, a "Health and damage" section, corrected Limits |

---

### Task 1: `src/combat.rs` — timeline entries and hits

**Files:** Create `src/combat.rs`, `tests/combat.rs`. Modify `src/lib.rs` (`pub mod combat;`), `src/round.rs` (field, call, serialization, `decodeStatus.combat`).

**Interfaces:**
- Consumes: `crate::records::RecordMap` (`stream_index`, `records_of`, `snapshots`), `crate::container::StreamInfo`, `crate::header::Player` (its `id` and entity ids), `crate::loadout::Clock`, `crate::loadout::messages` (make it `pub(crate)`), `crate::entities::{for_each_record, Record, Hash}`.
- Produces: `pub fn decode(data: &[u8], map: &RecordMap, streams: &[StreamInfo], players: &[Player], clock: &Clock) -> Combat` and

```rust
pub struct Combat {
    pub events: Vec<TimelineEvent>, // kills, team kills, deaths, downs, revives
    pub hits: Vec<Hit>,
    pub warnings: Vec<String>,
}
pub enum TimelineKind { Kill, TeamKill, Death, Down, Revive }
pub struct TimelineEvent {
    pub kind: TimelineKind,
    pub username: String,          // victim, or the player revived
    pub by: Option<String>,        // killer, downer, reviver; None when the file names nobody
    pub weapon: Option<u64>,       // Kill, TeamKill, Death
    pub headshot: Option<bool>,    // Kill, TeamKill
    pub time: String, pub phase: Phase, pub elapsed: f64, pub recording_time: Option<f64>,
}
pub struct Hit {
    pub username: String,              // victim
    pub damage: Option<u32>,           // None on a lethal hit (see below)
    pub health: u32,                   // after the hit; 0 when down or dead
    pub result: HitResult,             // Alive, Down, Dead
    pub kind: DamageType,              // { id: u32, name: Option<&'static str> }
    pub multiplier: f32,
    pub direction: u32,                // octant 0..=7, clockwise from the victim's aim
    pub by: Option<String>,
    pub attacker_source: Option<AttackerSource>, // Timeline, Shot, Aim
    pub distance: Option<f32>,         // metres, when `by` has a body position
    pub time: String, pub phase: Phase, pub elapsed: f64, pub recording_time: Option<f64>,
}
```

**Format, as verified.**

`TimelineChannel` is stream hash `[0xEE, 0xE4, 0x2D, 0x83]` (`crc32("TimelineChannel")`, bytes as in the stream list). The game writes the finished round's whole timeline into a record within the first second of the file; take the stream's largest record. It is `u32 n`, then `n` entries of `u8 type, u32 frame, body`, and must parse to its last byte (162 of 162 real rounds, `S\verify\tl2.py`):

| Type | Body | Meaning |
|---|---|---|
| 1 | `u64 weapon, u64 attacker body, u64 attacker playerid, u64 role icon, u32 team, u64 victim playerid, u64 role icon, u32 team, u8 headshot` | kill |
| 2 | same as 1 | team kill (both teams equal in 12 of 12) |
| 3 | `u64 weapon, u64 body, u64 playerid, u64 role icon, u32 team` | death with no killer |
| 5 | type 1 without weapon and headshot (48 bytes) | down; attacker playerid `ff..ff` when nobody downed them |
| 7 | same as 5 | revive: "attacker" is the reviver, equal to the victim on a self-revive |
| 9 | `u32` | phase change (skip) |
| 10 | `u64 playerid, u64 icon, u32 team, u8` | defuser event (skip) |

Any other type: stop, keep what was read, warn. `playerid` is the header's (`Player.id`). `frame` indexes `clock.frame_times`.

Hit blocks sit in `EntityChannel` (`[0x20, 0xA5, 0xC4, 0xE3]`) body updates: `607385fe`, `u16 mask`, position `3 x f32`. With mask bit `0x0008` the message ends in a tail component `u16 F, u16 G`, then by flag: `F&0x0002` aim quaternion `4 x f32` (forward is +Y), `F&0x0008`, `0x0010`, `0x0040`, `0x0080` one `f32` each, `F&0x0100` a `u8`, `F&0x0200` the 24-byte hit block, `G&0x0008` `30fe44a6` + `f32`, then 1 to 3 bytes. Port the locator from `S\damage\hits.py` exactly (signature scan of the last 140 bytes plus the check that `F` sits where the layout predicts).

| Offset | Type | Meaning |
|---|---|---|
| +0 | f32 | health ratio after the hit; negative (an overkill value) on a hit that downs or kills |
| +4 | f32 | damage multiplier |
| +8 | u32 | direction octant |
| +12 | u32 | life state after: 1 alive, 3 down, 4 dead |
| +16 | u32 | unknown (keep out of the output) |
| +20 | u32 | damage type |

Damage is `(previous ratio - ratio) * max`, rounded, with `max` from the HUD's `MaxHealth` (`[0x11, 0x49, 0xA6, 0x72]`) on the victim's life object and the previous ratio from the HUD's `HealthRatio` series; `None` when the block's ratio is negative. Damage type names, all inferred: 0 `bullet`, 1 `melee`, 2 `explosion`, 9 `gas`, 36 `fire`; others `None`.

Attacker, port of `S\damage\attacker.py`:
1. A hit that produced a timeline kill or down (same victim, within 0.5 s) takes that entry's attacker: `Timeline`.
2. Else, for type 0 only: opponents whose HUD `TotalAmmo` (`[0x40, 0x0A, 0xC8, 0x29]`) dropped within -140 to +70 ms; the one aiming closest to the victim: `Shot`.
3. Else the opponent aiming within 10 degrees, closest first: `Aim`.
4. Else `None`. Never guess for other damage types.

- [ ] **Step 1: failing test for the timeline.** In `tests/combat.rs`, open every round of `test_recordings/valid/Y11S3` with `ReadMode::Full` and assert: `combat` is `Some`; every `matchFeedback` kill has a `TimelineKind::Kill` or `TeamKill` event with the same victim within 0.5 s of `recordingTime` and the same `by`, `headshot`; every `Down` with `by` names a player of the round.
- [ ] **Step 2:** `cargo test --test combat` fails to compile (`combat` missing).
- [ ] **Step 3:** implement the timeline parser with a unit test on synthetic bytes for each entry type, an unknown type, and a truncated record.
- [ ] **Step 4:** tests pass.
- [ ] **Step 5: failing test for hits.** Every hit's `username` is a player; per player, `health` after each non-lethal hit equals the `health[]` value written at the same `recordingTime` when there is one (allow hits `health[]` merges); the count of hits over the 10 rounds is 243; hits with `attacker_source == Timeline` name the timeline's attacker.
- [ ] **Step 6:** implement the tail walker, hit block and attacker inference; unit-test the tail walker on synthetic bytes for each flag combination.
- [ ] **Step 7:** wire into `Parser` after `resolve_loadouts`; serialize as `hits` and `combat` events (see Task 3 for where events surface); add `decodeStatus.combat` (`decoded`, count of hits, warnings).
- [ ] **Step 8:** real-folder test behind `R6_MATCH_REPLAY`: the timeline parses to its last byte in every finished round; every revive names a reviver on the revived player's team; report (print, not assert) the share of timeline kills whose final hit the `Shot`/`Aim` rules alone would have attributed correctly.

### Task 2: `src/vitals.rs` — health, heals, plates, effects, friendly fire

**Files:** Create `src/vitals.rs`, `tests/vitals.rs`. Modify `src/lib.rs`, `src/round.rs`, `src/details.rs`.

**Interfaces:**
- Consumes: same as Task 1, plus `Round.loadouts` (operator and ability `uses` per player) passed as `&[Loadout]`.
- Produces: `pub fn decode(data, map, streams, players, loadouts, clock) -> Vitals` and

```rust
pub struct Vitals {
    pub players: Vec<PlayerVitals>,       // username, max_health (base), samples
    pub life: Vec<LifeChange>,            // Down, Revive with health revived to, bleed-out progress at the end
    pub heals: Vec<Heal>,
    pub plates: Vec<Plate>,
    pub effects: Vec<Effect>,
    pub friendly_fire: Vec<ReverseFriendlyFire>,
    pub flashes: Vec<Flash>,              // the recording player's own, player recordings only
    pub decay: Vec<(String, Option<f64>)>, // health drops that are overheal decay: username, recording time
    pub warnings: Vec<String>,
}
pub struct Heal { pub username: String, pub by: Option<String>, pub amount: u32, pub health: u32,
                  pub overheal: u32, pub kind: HealKind /* FinkaSurge, DocStim, KonaBurst, KonaTick */,
                  pub revive: bool, /* time fields */ }
pub struct Plate { pub username: String, pub by: Option<String>, /* time fields */ }
pub struct Effect { pub username: String, pub kind: u32, pub name: Option<&'static str>,
                    pub buff: bool, pub start: /* time fields */, pub seconds: f64, pub open: bool }
pub struct ReverseFriendlyFire { pub username: String, pub active_at_start: bool,
                    pub on: Option</* time fields */>, pub off: Option</* time fields */> }
```

**Format, as verified.** All in `HUDChannel` (`[0xA9, 0x8F, 0xDD, 0x0B]`), read with `for_each_record` over the snapshot and each frame record, as `loadout::Hud::read` does.

Life object: controller field `PlayerLifeVM` `[0x41,0x54,0xDC,0xC4]` (`Player`'s health entity id).

| Property | Bytes | Type | Rule |
|---|---|---|---|
| `Health` | `25 26 76 c9` | u32 | includes overheal |
| `MaxHealth` | `11 49 a6 72` | u32 | 100, 110 or 125; +25 with a plate |
| `OverhealedMaxHealth` | `01 3f d2 da` | u32 | `MaxHealth + 20` |
| `PlayerLifeState` | `e7 88 f6 a5` | u32 | 0 above 20 HP, 2 at or under 20, 1 overhealed, 3 down, 4 dead |
| `DBNOProgress` | `72 5e 99 f9` | f32 (u32 0 for zero) | bleed-out, 0 towards 1 |

- Revive: state 3 to 0, 1 or 2 with `Health > 0` in the same frame or by the next state write (3 of 33 write the health a frame later). Today's `(3, 0|2)` rule reports a death written 3, 2, 4 as a revive (117 false of 144); fix it in `resolve_samples` too.
- Base max health is the first `MaxHealth` above 0.
- Plate: `MaxHealth` and `Health` both +25 in one record. `by` is the round's Rook on the same team, `None` without one.
- Overheal decay: a -1 step while `Health > MaxHealth`.

Effects: controller field `EffectsVM` `[0x4C,0x37,0x23,0x5C]` to `EffectsViewModel`; its list `[0xB5,0x04,0xBF,0x0F]` is written as a `23 <list obj> ... 01 01` record followed by one `1e` element per active effect (class `[0x9E,0x9C,0x89,0xBE]`). Each list write is a full enumeration: an effect starts at the first write listing its item and ends at the first that omits it. Item properties: `Type` `17 f8 ec 2c` u32, `State` `ff fd 52 62` (2 buff, 3 debuff). Names (inferred from which operator is present; mark the table so): 0 `JackalTracked`, 33 `JackalTracking`, 1 `LesionPoison`, 2 `FinkaSurge`, 3 `DokkaebiCall`, 4 `RookArmor`, 5 `AlibiTracked`, 6 `ClashShock`, 8 `EnemyJammer`, 34 `FriendlyJammer`, 11 `LionScan`, 13 `ProximityAlarm`, 14 `MelusiBanshee`, 22 `GrimSwarm`, 23 `GrimTracked`, 26 `FenrirMine`, 27 `FenrirFear`, 28 `TubaraoZoto`, 30 `DeimosMarked`, 31 `DeimosTracking`, 35 `ThunderbirdHeal`, 39 `ThornRazorbloom`, 43 `SnakeRadar`, 46 `NoorLance`, 52 `Burning`. Others keep the id and no name. Port from `S\status\effects.py`.

Heals, port of `S\heal\classify.py`: a `Health` rise in a live phase that is not a spawn (0 to max), a plate, or a plain revive (0 to 20):
- `FinkaSurge`: a new type 2 item on the receiver in the same frame, or Finka's ability count dropping within 0.11 s before. +20. `by` is the round's Finka on the team.
- `DocStim`: `Health` set to `OverhealedMaxHealth` with the `TotalAmmo` of Doc's ability slot dropping in the same frame or up to 0.31 s before. `by` is that Doc.
- `KonaBurst` (+20 or +21) and `KonaTick` (+1 while type 35 is listed). `by` is the team's Thunderbird.
- `revive: true` when the rise starts from state 3.
- `by` is inferred (the file links no giver): say so in the README, not per event.

Friendly fire: controller field `[0x5E,0xF7,0xD9,0x9D]` (`FriendlyFireFeedbackVM`) to an object with `IsReverseFriendlyFireActive` `[0x4A,0xCD,0x2D,0x46]` u8. Flash: controller properties `IsAffectedByFlashbang` `[0x56,0x78,0x29,0x44]` u8, written to every controller in the same frame: it is the recording player's state, so emit `flashes` only when `players` has a recorder (`relation == you`) and attribute them to that player.

- [ ] **Step 1: failing tests** in `tests/vitals.rs` on the 10 test rounds: no `Revive` life event without `health > 0`; every player has a `max_health` of 100, 110 or 125; every effect has `seconds >= 0` and a player of the round; an effect interval never overlaps another of the same type on the same player.
- [ ] **Step 2:** watch them fail to compile.
- [ ] **Step 3:** implement the HUD walker and life series; unit-test the revive rule on the 3, 2, 4 sequence and on 3 to 1 with health.
- [ ] **Step 4:** implement effects with a unit test on synthetic list writes (start, replace, empty).
- [ ] **Step 5:** implement heals, plates, decay, friendly fire, flash.
- [ ] **Step 6:** wire into `Parser`; serialize `heals`, `plates`, `effects`, `friendlyFire`, `flashes`; `players[].maxHealth`; `HealthUpdate` gains `max_health`, `overheal` (skip when 0) and `cause` (`decay`, `finkaSurge`, `docStim`, `konaBurst`, `konaTick`, `plate`, `revive`; absent for damage); `decodeStatus.vitals`.
- [ ] **Step 7:** real-folder tests behind `R6_MATCH_REPLAY`: every `FinkaSurge` receiver has a Finka on the team; every plate has a Rook on the team; `Health - MaxHealth` never exceeds 20; every reverse-friendly-fire `on` in a round with a team kill names the killer or a teammate.

### Task 3: join, stats, README

**Files:** Modify `src/round.rs`, `src/details.rs`, `src/stats.rs`, `src/feedback.rs`, `src/decoder.rs`, `README.md`, `tests/replays.rs`.

**Interfaces:** Consumes `Round.combat: Option<Combat>` and `Round.vitals: Option<Vitals>` from Tasks 1 and 2.

- [ ] **Step 1:** `lifeEvents[]`: a `Down` gains `by` (timeline down for that victim within 0.5 s) and `outcome`: `finished` with `finishedBy` (next kill of that victim), `revived`, `downAtEnd`. A `Revive` gains `by`, `self` and `health`. Test: in the test rounds every down has an outcome.
- [ ] **Step 2:** kill feed entries gain `teamKill: true` (timeline type 2) and `downedBy` when the victim was down. Test against `creditedTo`: where both exist they agree.
- [ ] **Step 3:** `stats[]`: `damageDealt` (hits with `by`), `downsDealt`, `finishes`, `revivesGiven`, `teamKills`, `healingGiven`, `healingReceived`; `damageTaken` skips decay. Test: per round, the sum of `damageDealt` is at most the sum of `damageTaken`.
- [ ] **Step 4:** kills gain `victimEffects` (names of effects open on the victim at the kill) and `victimHealthBefore`.
- [ ] **Step 5:** bump the Y11S3 decoder revision in `src/decoder.rs`; README: output rows, a "Health and damage" section with the tables above and what is not recorded (attacker per hit, body part, wall penetration, concussion, gas and trap effects, armor rating), and replace the two Limits bullets that say damage dealt is not recorded.
- [ ] **Step 6:** `cargo test`, `cargo clippy --all-targets`, then the same with `R6_MATCH_REPLAY` set.

## Requirement coverage

| Asked for | Where | Verdict |
|---|---|---|
| Downs: who downed, who finished, bleed-outs | Task 1 timeline, Task 3 step 1 | decoded; no bleed-out death exists in 174 rounds, so that outcome is untested |
| Team kills, reverse friendly fire | Task 1 type 2, Task 2 friendly fire | decoded |
| Revives: who, whom, when | Task 1 type 7 | decoded |
| HP over time, overheal, boosts | Task 2 life series | decoded |
| Armor and plates | Task 2 `maxHealth`, plates | max health decoded; no armor property exists |
| Every damage hit | Task 1 hits | victim, amount, type decoded; attacker inferred; body part and wall not recorded |
| Heals | Task 2 heals | receiver and amount decoded; giver inferred |
| Status effects | Task 2 effects, flashes | list decoded; concussion, gas, shock, traps not in the list; flash for the recorder only |
