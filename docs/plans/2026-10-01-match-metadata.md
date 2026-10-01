# Match metadata from the bytes Implementation Plan

> **To execute:** use the `executing-plans` skill. Steps use `- [ ]` for tracking.

**Goal:** Every item of the match-metadata list (match id, start time, queue and playlist, mode,
map, rules, settings, teams, score and result, picks and bans, levels, forfeits, Dual Front) is
either decoded from the replay bytes into the JSON, or documented as absent with the evidence.

**Architecture:** Five read-only research passes over the 167 real rounds of one `MatchReplay`
folder and the 10 test rounds (findings below) decided the work. Most items already exist in the
match `summary`; this plan fixes what the bytes contradict (ban teams, `matchresult`, the
spectator fallback), fills the lookup tables, reads ban slots from the snapshot's object tree,
names match type 7, and removes 5v5 assumptions. Ranks, reputation, server region and ping are
not in the files: the README says so, with what was searched.

**Tech stack:** Rust 2024, existing crates only. Python 3.13 for byte probing in the scratchpad.

## Global Constraints

- No new dependencies.
- JSON keys camelCase; new optional fields use `skip_serializing_if` like their neighbours.
- `ReadMode::Header` must not decompress Y8S4+ files.
- The game folder (`MatchReplay`, `DissectTmp`) is read only; real-data tests read
  `R6_MATCH_REPLAY` and skip when it is unset; they test invariants, not counts (the folder keeps
  the newest ~30 matches).
- Never commit replay files.
- Existing tests stay green; `cargo clippy --all-targets` stays warning-free.
- A decoding change bumps `revision` of every profile whose output changes (`src/decoder.rs`).
- CRLF files (`README.md`, `src/decoder.rs`, `details.rs`, `header.rs`, `matches.rs`,
  `summary.rs`, `types/mod.rs`) stay CRLF: check `git diff --stat` and
  `git ls-files --eol` after edits and after `cargo fmt`.
- No attribution trailers in commits or PRs.

## Findings this plan rests on

All hashes are CRC-32 of the game's property name, little-endian (`crc32("Health")` = bytes
`25 26 76 c9`); names below come from that.

| Item | Finding | Confidence |
|---|---|---|
| Level | `ClearanceLevelText` (`3f0fdc1f`) on the profile object; stable per match, never drops, recorder 233 → 237 over 10 days, ranked minimum 54 (Ranked needs 50) | confirmed |
| Rank, rank points, max rank, reputation | Not in any stream or header; the profile schema is identical in ranked, unranked, quick and custom | not found (high) |
| Server region, ping | Not found: header, every string (ASCII/UTF-16), hashed data-center names, every tree property, every side stream | not found (high) |
| `matchresult` | Written on the deciding round: 2 = the `TeamColor` 1 team won, 1 = it lost (16/16 wins, 10/10 losses); `TeamColor` 1 is the recorder's team in player recordings, header team 0 in the one spectator match; 7 = the server ended the match before round 4's prep with no winner (both teams get the loss marker) | confirmed / 7 inferred |
| Spectator | Y11S3 headers write `isspectator` only when true; a file cut during prep lacks the attackers' header block, which made `--list` call the recorder a spectator | confirmed |
| Match type 7 | Unranked: ranked rules and bans, 11/40 players below level 50, Tower in rotation; playlist `416350367764` | inferred |
| Bans (Siege X) | Ban manager → two arrays of 3 slots (class `c7b2b204`): `HeroTeam` side banned, `TeamColor` banning team, `BanState` 0/3, `ResultType` 1 banned / 2 no ban, `Operator` → `OperatorInfo` → `BadgeIcon` icon. Ranked: each team bans one opposing-side operator per round, 2-4-6 per half, rebuilt at the side swap, overtime reuses the end-of-half set | confirmed |
| Ban team bug | `team` maps `TeamColor` as `t - 1`; wrong in all 70 rounds where the recorder is header team 1 | confirmed |
| Icons | 26 operators missing from `ROLE_IMAGES`; 77 of 628 real bans unnamed | confirmed (Iana inferred) |
| Maps | 9 unknown world ids named from sites and spawns | names confirmed, some versions inferred/guessed |
| Dual Front | No replay seen; no source says it records | not found |

## File map

| File | Change |
|---|---|
| `src/types/tables.rs` | 9 `MAPS` rows, 26 `ROLE_IMAGES` rows, `PLAYLISTS` table |
| `src/types/mod.rs` | `MatchType` knows its build; `version::Y11S3`; `playlist_name` |
| `src/entities.rs` | tree keeps array indices; `Snapshot { players, ban_slots }`; `PlayerObjects.team_color`; `BanSlot` |
| `src/header.rs` | `Team.color`, `Header::team_of_color`, `isspectator` default, `matchresult` docs, `MatchType::new` |
| `src/details.rs` | `Ban` gains `slot`, `no_ban`, `color`; `icon` optional |
| `src/round.rs` | bans from slots, team colors from the tree, seats from `maxnbplayersperteam`, levels decoded |
| `src/summary.rs` | `Outcome::Cancelled`, result from `matchresult`, `bans` decisions, `playlist`, `queue` |
| `src/decoder.rs` | revision bumps |
| `README.md`, `tests/replays.rs` | docs and real-data invariants |

---

### Task 1: Name current maps and ban icons

**Files:**
- Modify: `src/types/tables.rs` (`MAPS`, `ROLE_IMAGES`), `src/summary.rs` (`MapInfo` docs)
- Test: `src/types/mod.rs` tests, `tests/replays.rs`

**Interfaces:** Produces table rows only.

- [x] **Step 1: Failing test** in `src/types/mod.rs`:

```rust
#[test]
fn names_current_maps_and_icons() {
    assert_eq!(Map(419965653950).name(), Some("CalypsoCasino"));
    assert_eq!(Map(398899676157).name(), Some("FortressY10"));
    assert_eq!(Operator::from_role_image(445433447900).and_then(Operator::name), Some("Dokkaebi"));
    assert_eq!(Operator::from_role_image(104189663973).and_then(Operator::name), Some("Iana"));
}
```

- [x] **Step 2: Run** `cargo test --lib names_current_maps_and_icons` — expect FAIL.
- [x] **Step 3: Implement.** `MAPS` gains (comments give the evidence):
  `FortressY10` 398899676157 (Y10S4 rework: new sites), `VillaY11` 409325881472 (Y11S3: basement
  site), `CalypsoCasino` 419965653950 (Y11S2 map), `SkyscraperY10` 423767322185,
  `ThemeParkY10` 430788891316 (Y10S4 update), `CoastlineY11` 436375283234 (Y11S1),
  `KanalY11` 441408792952 (Y11S2), `PresidentialPlaneY11` 439976373310 and `TowerY11`
  454245490351 (year from the id range only). `ROLE_IMAGES` gains Amaru 104189663542, Blitz
  2461366787, Brava 288200866784, Buck 32822532297, Caveira 34075810132, Denari 374667787928,
  Doc 2461366796, Dokkaebi 445433447900, Finka 104189661900, Fuze 1326495659, Gridlock
  183220539159, Jackal 39149215409, Lesion 39149215517, Maestro 104189662110, Maverick
  104189662319, Nokk 104189662959, Oryx 104189664090, Osa 288200867407, Pulse 2461366790, Rauora
  386098331848, Sentry 409899350066, Skopos 386098331638, Sledge 2461366799, Thunderbird
  288200867313, Zero 291191151539, Iana 104189663973 (inferred: operator id − 65, banned once,
  never in a header). Ace's comment: now seen in real headers. `MapInfo::version` doc: the suffix
  marks a new world build (new id); the floor plan can be unchanged.
- [x] **Step 4: Run** — PASS (also `every_role_image_names_a_known_operator`).
- [x] **Step 5: Real-data test** `real_maps_have_names` (header reads of every folder under
  `R6_MATCH_REPLAY`): every `summary.map.base` is `Some`.
- [x] **Step 6: Commit** `feat(src): name current maps and ban icons`.

### Task 2: Team colors and the ban team fix

**Files:**
- Modify: `src/entities.rs`, `src/header.rs`, `src/round.rs`, `src/details.rs`
- Test: `src/header.rs` tests, `tests/replays.rs`

**Interfaces:**
- Produces:
  - `entities::PlayerObjects.team_color: Option<u32>`: `TeamColor` (`2e61a2a9`) of the team object
    the controller's `951c1650` points to.
  - `header::Team.color: Option<u32>` (JSON `color`), set on full and partial reads.
  - `Header::team_of_color(&self, color: u32) -> Option<usize>`: by `teams[i].color`; else color 1
    is the recorder's team (player recordings) or team 0 (spectators); color 2 the other.
  - `details::Ban.color: Option<u32>` (`#[serde(skip)]`): the raw `TeamColor` of the slot.

- [x] **Step 1: Failing unit test** in `src/header.rs`: a header with the recorder in team 1 and
  no colors gives `team_of_color(1) == Some(1)`, `team_of_color(2) == Some(0)`; with
  `is_spectator: Some(true)` gives `Some(0)` / `Some(1)`; with `teams[0].color = Some(2)` gives
  `team_of_color(2) == Some(0)`; `team_of_color(3) == None`.
- [x] **Step 2: Run** — FAIL. **Step 3: Implement** `team_of_color` and `Team.color`;
  `PlayerObjects.team_color` (`t.u32(team_object, TEAM_COLOR)`); in `apply_entities`, set
  `teams[t].color` when every player of team `t` with a color agrees; `read_role_image` stores
  the raw value in `Ban.color` and leaves `team` to a pass after `apply_entities` that maps it
  with `team_of_color`. **Step 4: Run** — PASS.
- [x] **Step 5: Real-data test** `real_bans_are_made_by_the_other_side` (full reads,
  `R6_MATCH_REPLAY`): in every round, every ban with a `team` has
  `teams[team].role != Some(ban.role)`; the recorder's team has `color == Some(1)`.
  Run it before Step 3 to see it fail on the inverted matches.
- [x] **Step 6: Commit** `fix(src): credit bans to the team that made them`.

### Task 3: Ban slots from the snapshot

**Files:**
- Modify: `src/entities.rs`, `src/details.rs`, `src/round.rs`
- Test: `src/entities.rs` tests, `tests/replays.rs`

**Interfaces:**
- Consumes: `Header::team_of_color` (Task 2).
- Produces:
  - `entities::Snapshot { players: Vec<PlayerObjects>, ban_slots: Vec<BanSlot> }`,
    `entities::snapshot(body: &[u8]) -> Snapshot`; `players(body)` returns `snapshot(body).players`.
  - `entities::BanSlot { index: u32, side: u32, color: u32, state: u32, result: u32, icon: Option<u64> }`.
  - `details::Ban.slot: Option<u32>` (JSON `slot`), `no_ban: bool` (JSON `noBan`, skipped when
    false), `icon: Option<u64>` (skipped when none).

- [x] **Step 1: Failing unit test** in `entities.rs`: a synthetic snapshot with a manager object
  holding `1e 561e4c23 <0> <slot>` and `1e 561e4c23 <1> <slot2>`; slot: `HeroTeam` 2,
  `TeamColor` 1, `BanState` 3, `ResultType` 1, `1b <slot> Operator <op>`, op `1a OperatorInfo
  <info>`, info `BadgeIcon` u64 39149215445; slot2: state 3, result 2, no operator. Expect two
  `BanSlot`s: index 0 with icon 39149215445, index 1 with result 2 and no icon.
- [x] **Step 2: Run** — FAIL. **Step 3: Implement**: `Record::Element(field, index, child)` for
  `1e` (still a child of the current object); `Object.elements`; `snapshot()` builds the tree
  once; slots are the non-zero elements of `561e4c23` / `e6372c1e`, deduplicated by
  `(color, index)` (a second copy is sometimes sent with new object ids).
- [x] **Step 4: Wire** in `round.rs`: `read_entities` keeps the slots; after `apply_entities`,
  when slots exist, `round.bans` becomes one `Ban` per resolved slot (`state == 3`): result 1
  with an icon → the operator; result 2 → `no_ban`; side 1 Attack / 2 Defense; `team` from
  `team_of_color`; sorted by team, then slot. If the packet icons differ from the slot icons,
  warn in `decodeStatus.bans`. Builds without slots keep the packet path.
- [x] **Step 5: Run** all tests — PASS (`y11s3_bans_levels_and_picks` order unchanged).
- [x] **Step 6: Real-data test** extends Task 2's: every banned slot's operator resolves to a name
  and its side matches the operator's role; every round's bans per team have consecutive slots.
- [x] **Step 7: Commit** `feat(src): read ban slots, their order and skipped bans`.

### Task 4: Ban decisions in the match summary

**Files:** Modify `src/summary.rs`. Test: `src/summary.rs` tests.

**Interfaces:**
- Consumes: `Ban` with `slot`, `no_ban` (Task 3).
- Produces: `MatchSummary.bans: Vec<BanDecision>`;
  `BanDecision { round: u32, #[serde(flatten)] ban: Ban }`: each distinct ban, keyed by
  `(team, role, slot, icon, no_ban)`, with the first round (from 1) it applied to.

- [x] **Step 1: Failing test**: rounds 1–3 holding bans `{A}`, `{A, B}`, `{A, B}` give decisions
  `A @ 1`, `B @ 2`.
- [x] **Step 2: Run** — FAIL. **Step 3: Implement** `ban_decisions(rounds)`. **Step 4: Run** — PASS.
- [x] **Step 5: Commit** `feat(src): list each ban with the round it was made for`.

### Task 5: Result, cancelled matches and the spectator flag

**Files:** Modify `src/header.rs`, `src/summary.rs`. Test: `src/summary.rs` tests, `tests/replays.rs`.

**Interfaces:**
- Consumes: `Header::team_of_color` (Task 2).
- Produces: `summary::Outcome::Cancelled` (JSON `cancelled`); `Header.is_spectator` is
  `Some(false)` when the header has `starttime` but no `isspectator`.

- [x] **Step 1: Failing tests** in `summary.rs`, rounds built with `Round::default()`:
  (a) recorder in team 1, final round `matchresult` 2 at 4-6 (ranked rules 6+3, no winner by
  score yet) → `winner == Some(1)`, `outcome == Win`, `ended_early == Some(true)`;
  (b) `matchresult` 7 at 1-2 → `winner == None`, `outcome == Cancelled`,
  `ended_early == Some(true)`, `complete == true`;
  (c) a header with `start_time` and no `isspectator` reads `is_spectator == Some(false)`
  (`header.rs` test through `parse`).
- [x] **Step 2: Run** — FAIL. **Step 3: Implement**: the winner comes from the score when it
  decides the match; else from `matchresult` (2 → `team_of_color(1)`, 1 → the other team,
  7 → cancelled); other values stay raw with `ended_early` set. Correct the `matchresult` docs.
- [x] **Step 4: Run** — PASS.
- [x] **Step 5: Real-data test** `real_results_agree_with_matchresult`: for every finished
  player-recorded match, `matchresult` 2 ⇔ `outcome == win`; no non-spectator file is listed as a
  spectator.
- [x] **Step 6: Commit** `fix(src): read matchresult from the recorder's side and flag cancelled matches`.

### Task 6: Unranked and playlists

**Files:** Modify `src/types/mod.rs`, `src/types/tables.rs`, `src/header.rs`, `src/summary.rs`,
`src/round.rs` (`.0` uses). Test: `src/types/mod.rs`, `src/summary.rs`.

**Interfaces:**
- Produces: `MatchType { pub id: u32, pub build: u32 }`, `MatchType::new(id, build)`,
  `name()`, `is_custom()`; `version::Y11S3 = 9_883_691` (oldest Y11S3 build seen);
  `types::playlist_name(category: i64) -> Option<&'static str>`;
  `MatchSummary.playlist: Option<&'static str>`.

- [x] **Step 1: Failing tests**: `MatchType::new(7, 9_901_603).name() == Some("Unranked")`,
  `MatchType::new(7, 8_673_114).name() == None`, `MatchType::new(9, 0).name() == Some("Unranked")`;
  `queue(MatchType::new(7, 9_901_603)) == "unranked"`; `playlist_name(416350367764) ==
  Some("Unranked")`; JSON of `MatchType::new(2, 1)` is `{"name":"Ranked","id":2}`.
- [x] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** — PASS.
- [x] **Step 5: Commit** `feat(src): name Unranked and playlists`.

### Task 7: Teams sized by the header

**Files:** Modify `src/round.rs`. Test: `src/round.rs` tests.

**Interfaces:** Produces `fn pick_team(pick: u32, per_team: u32) -> usize` (private).

- [x] **Step 1: Failing test**: `pick_team(6, 6) == 0`, `pick_team(7, 6) == 1`, `pick_team(6, 5) == 1`.
- [x] **Step 2: Run** — FAIL. **Step 3: Implement**: `read_player` uses `pick_team`;
  `players_read == 10` and `< 10` use `2 * maxnbplayersperteam` (default 5).
- [x] **Step 4: Run** — PASS. **Step 5: Commit** `fix(src): size teams from maxnbplayersperteam`.

### Task 8: Levels decoded, docs, decoder revisions

**Files:** Modify `src/round.rs` (levels status), `src/header.rs` and `src/summary.rs` (level
docs), `src/decoder.rs`, `README.md`.

- [x] **Step 1:** `decodeStatus.levels` → `decoded` (the property is `ClearanceLevelText`); fix
  the test that expects `inferred`, if any.
- [x] **Step 2:** Bump `revision` of profiles Y8S1 through Y9S4 (ban icons, teams, slots,
  match type 7, spectator default), with a note in the module doc.
- [x] **Step 3:** README: bans (Siege X rules, slots, `noBan`, team fix), summary keys
  (`playlist`, `bans`, `cancelled`, `teams[].color`), level decoded, maps and version meaning,
  match type 7, `matchresult` meaning, "Limits" rewritten with what was searched for ranks,
  reputation, region and ping, Dual Front status, hashes are CRC-32 of names.
- [x] **Step 4:** `cargo fmt`, `cargo clippy --all-targets`, `cargo test --release` with and
  without `R6_MATCH_REPLAY`; `--list` and one full match read by hand.
- [x] **Step 5: Commit** `docs: document match metadata, bans and what replays lack`.

## Open questions for the user

- Match type 7: was it Unranked? (inferred from levels and map pool)
- 29 Sep ~01:26, Bank, 1-2: cancelled by the game? (7 = no winner is inferred)
- Current clearance level (expected 237)?

## Execution notes

All tasks done on `feat/match-metadata`. Where execution departed from the plan:

- **Object-tree fix (not planned).** Task 2's invariant "the recorder's team has TeamColor 1"
  failed in one real round: a stray `26` record (index 0x69a8fc11) covered the team object's
  first record, so its TeamColor went to another object. Array records with an index of 65536
  or more are now rejected (real indices reach 64). A before/after diff of all 30 real matches
  changed only that round. Committed on its own, after the ban fix.
- **Ban decisions** (Task 4) are verified by a unit test and a by-hand read of a real overtime
  match, not by a real-data test.
- **Real-data tests** share one full read of the folder (`real_rounds`), which took the run
  from 19 s to 6 s.
- **`MatchType`** became a struct with `id` and `build` (breaking for library users), and
  `queue` maps names rather than ids.
- **Levels and decoder revisions** were committed apart from the README.
- **Platform**: `PlayerPlatform` (`7dd4fc18`) varies (0, 5, 7) in real PC matches; the README
  no longer says no field varies, but the values are not output.
