# Gadgets, World and Destruction Implementation Plan

> **To execute:** use the `executing-plans` skill. Steps use `- [ ]` for tracking.

**Goal:** Decode gadget objects (type, owner, place, time, state, end), what happens to them (destroyed, disabled, hacked, captured), traps set off, reinforcements, barricades, destruction, breaches, smoke/fire/gas areas and environment objects from Y11S3 replays into round JSON.

**Architecture:** One shared pass over the `movement` stream (`src/world.rs`) and one over `FXChannel` (`src/fx.rs`) turn the bytes into per-entity change lists. Four decoders read those and nothing else from the streams (the gadget-event decoder also reads the HUD): `src/panels.rs`, `src/gadgets.rs`, `src/gadget_events.rs`, `src/destruction.rs`, `src/areas.rs`. `Parser::resolve_loadouts` builds the world once and calls each; a last pass joins them and adds stats and README text.

**Tech stack:** Rust 2024, existing dependencies only. No new crates.

This plan is a hypothesis built from research on 10 test rounds and about 175 real rounds. Where the bytes disagree with it, the bytes win: fix the claim in the code's docs and say so in the hand-back.

## Global Constraints

- Y11S3+ only (`self.code() >= version::Y11S3`), full reads only. Older rounds keep today's output.
- Nothing inferred is presented as read: every inferred value carries a `...Source` field (`score`, `proximity`, `shotRay`, `nearest`, `assumed`, `derived`) or `inferred: true`.
- A malformed message is skipped and counted in the decoder's warnings, never a panic. No `unwrap` on stream bytes, no indexing without `get`.
- Never copy real replays into the repo. Real-folder tests read `R6_MATCH_REPLAY` and assert invariants, not counts.
- Some files are CRLF (`README.md`, `src/details.rs`, `src/stats.rs`, `src/decoder.rs`, `src/types/mod.rs`, ...). Never run `cargo fmt`; format only new files (`rustfmt --edition 2024 src/world.rs`), then check `git diff --stat`.
- Never `git stash` (shared by all worktrees). Commit in your own worktree and branch only.
- Hashes are written in stream byte order: class `4c60869a` is `[0x4C, 0x60, 0x86, 0x9A]`.
- Object ids in JSON are lowercase hex strings without prefix, as `meleeHits[].object` is today. Times are `crate::loadout::When` (flattened), as `throws` and `meleeHits` use.
- Reference implementations (Python, run and checked on the data) live in `SP` = `C:\Users\Mark\AppData\Local\Temp\claude\D--Development-replay-analyzer\4552b641-01d1-48d9-bf84-1627e02cf63c\scratchpad`: `SP\A` gadgets, `SP\B` gadget events, `SP\C` panels, `SP\D` destruction, `SP\E` areas. Each has `reference.py <dump> <json> [<rec>]` and expected outputs for the test rounds. Dumps: `C:\Users\Mark\AppData\Local\Temp\claude\D--Development-replay-analyzer\34887d58-de12-4169-a185-367c7c593659\scratchpad\throws\c1.bin` .. `c10.bin`; JSON of the current parser: `SP\json\c1.json` .. `c10.json`. A Rust decoder is done when its output for the 10 test rounds equals its reference's, field by field, or each difference is explained.

## File map

| File | Responsibility |
|---|---|
| `src/world.rs` (new) | One pass over the movement stream: entities, map objects, per-entity changes (transform, components, damage records), body tracks |
| `src/fx.rs` (new) | `FXChannel` records: spawns with parameters, stops, point lists |
| `src/panels.rs` (new) | Reinforcements and barricades |
| `src/gadgets.rs` (new) | Gadget objects and default cameras |
| `src/gadget_events.rs` (new) | Score changes, gadget removals with who and how, statuses (EMP, frozen, hacked, captured, caught), traps |
| `src/destruction.rs` (new) | Destruction events, surfaces (derived hole labels), breaches |
| `src/areas.rs` (new) | Smoke, fire, gas, swarm areas; environment objects; light screens |
| `src/types/gadget_tables.rs`, `damage_tables.rs`, `fx_tables.rs`, `map_tables.rs` (new) | Id to name tables, one file per decoder so branches do not collide |
| `src/round.rs` | Fields, calls, serialization, `decodeStatus` entries |
| `src/stats.rs` | Per-player counts |
| `src/shots.rs` | Its FX reader moves to `src/fx.rs` |
| `tests/world.rs`, `tests/panels.rs`, `tests/gadgets.rs`, `tests/gadget_events.rs`, `tests/destruction.rs`, `tests/areas.rs` (new) | Test rounds and real-folder invariants |
| `README.md`, `src/decoder.rs` | Output rows, a "Gadgets, world and destruction" section, revision bump |

---

### Task 1: `src/world.rs` and `src/fx.rs` — the shared pass

**Files:** Create `src/world.rs`, `src/fx.rs`, `tests/world.rs`. Modify `src/lib.rs` (`pub mod world; pub mod fx;`), `src/shots.rs` (use `fx.rs`), `src/round.rs` (build both in `resolve_loadouts`, keep on `Parser` for the later tasks, `decodeStatus.world`).

**Interfaces:**
- Consumes: `crate::loadout::{Input, messages, MOVEMENT_STREAM, DESCRIPTOR, UPDATE}`, `crate::entities::Hash`.
- Produces (names later tasks rely on; fields may gain members, not lose them):

```rust
// src/world.rs
pub(crate) fn decode(input: &Input) -> World;
pub(crate) struct World {
    pub entities: HashMap<u64, Entity>,
    pub order: Vec<u64>,                 // ids in creation order
    pub bodies: HashMap<u64, Vec<(u32, [f32; 3])>>, // player body id -> (frame, position), about 10 a second
    pub warnings: Vec<String>,
    pub updates: usize, pub unparsed: usize,
}
pub(crate) enum Kind { Entity, MapObject }   // 617385fe / 627385fe
pub(crate) struct Entity {
    pub id: u64, pub kind: Kind,
    pub asset: u64, pub archetype: u64,
    pub classes: Vec<Hash>, pub slots: Vec<(Hash, u64)>, pub create_size: usize,
    pub created: Option<u32>,            // frame; None = in the snapshot
    pub position: [f32; 3], pub rotation: [f32; 4],   // of the create message
    pub deleted: Option<u32>,            // frame of 637385fe
    pub changes: Vec<Change>,            // in stream order; empty for bodies, guns, attachments
}
pub(crate) struct Change {
    pub frame: Option<u32>, pub full: bool,   // full = sub 1f, the state it was created with
    pub position: Option<[f32; 3]>, pub rotation: Option<[f32; 4]>,
    pub live: Option<u8>, pub flags: Option<u16>,
    pub placed: Option<Placed>,          // 4c60869a
    pub owner: Option<Owner>,            // 8490f616
    pub state: Option<State>,            // 513b13b2
    pub device: Option<Device>,          // 587f5a72
    pub damage: Vec<Damage>,             // 6ea51c35 records, new ones only
    pub destroyed: bool,                 // an `fe` entry
}
pub(crate) struct Placed { pub type_index: Option<(u16, u8)>, pub owner: Option<u64>, pub host: Option<u64> }
pub(crate) struct Owner { pub type_index: Option<u16>, pub player: Option<u64>, pub alliance: Option<u32>, pub released: Option<u8> }
pub(crate) struct State { pub a: Option<u8>, pub b: Option<u8>, pub blob: Option<(Hash, Vec<u8>)> }
pub(crate) struct Device { pub destroyed: Option<bool>, pub captured: Option<bool>, pub mount: Option<u64> }
pub(crate) struct Damage {
    pub kind: u8, pub point: [f32; 3], pub direction: [f32; 4],
    pub instigator: u64, pub id: u64, pub slot: u32, pub part: u32,
    pub impacts: Vec<Impact>,
}
pub(crate) struct Impact { pub position: [f32; 3], pub normal: [f32; 3], pub index: u32, pub scale: f32 }
impl World {
    pub(crate) fn to_map(position: [f32; 3], q: [f32; 4], point: [f32; 3]) -> [f32; 3]; // position + q * point
    pub(crate) fn body_at(&self, body: u64, frame: u32) -> Option<[f32; 3]>;            // last position at or before frame
}
pub(crate) const POOL: [f32; 3] = [0.0, 0.0, -100.0];

// src/fx.rs
pub(crate) fn decode(input: &Input) -> Effects;
pub(crate) struct Effects { pub spawns: Vec<Spawn>, pub warnings: Vec<String>, pub records: usize, pub unparsed: usize }
pub(crate) struct Spawn {
    pub frame: Option<u32>,              // None = in the snapshot
    pub asset: u64, pub parent: u64, pub target: u64, pub instance: u32,
    pub position: Option<[f32; 3]>,      // vector parameter 56 95 b5 31
    pub alliance: Option<u32>,           // int parameter 0a d9 18 33
    pub points: Vec<[f32; 3]>, pub capacity: u32,   // last point list
    pub stopped: Option<u32>,            // frame of its stop
}
```

**Format, as verified.**

Movement payload: `u16 count`, then `u64 object, u32 size, body`. Bodies by their first four bytes:

```text
617385fe create   +4 id u64, +16 position 3 x f32, +28 quaternion 4 x f32, +44 u8, +45 u64 archetype,
                  +53 u32 n, n class hashes, asset u64, u32, u32 count, count x {value u64, slot hash, u32}, tail
627385fe create of a map object: same head; one class, no asset, no slots; always 131 bytes
637385fe delete
607385fe update   u8 mask
  mask & 80       u8 sub, then in this order
     sub & 01     f32 x, y, z, u32 0
     sub & 02     f32 qx, qy, qz, qw
     sub & 04     u8 live
     sub & 08     u16 flags
     sub & 10     u8 n, n x {u32 op; op 0: 34 bytes; op 1: 80 bytes; op 2, 3: nothing}
  mask & (40 >> i)  the component of class i of the create message, in class order
```

The `u16 mask` `src/throws.rs` documents is these two bytes read together. Components, by class:

| Class | Layout | Size |
|---|---|---|
| `4c60869a` | `u8 m; m&01: u16 type index, u8 variant; m&02: u64 owner playerid; m&04: u64 host` | from `m` |
| `8490f616` | `u8 m; m&01: u16 type index; m&02: u64 playerid; m&04: u32 alliance; m&08: u8 released` | from `m` |
| `513b13b2` | `u8 m; m&01: u8; m&02: u8; m&04: u16 n, u32 hash, u32 len (= n - 8), len bytes`; with all of `f8` set (full state) 2 more bytes | from `m` |
| `587f5a72` | `u64 own id, u8 aim (1: 16 bytes follow), f32 field of view, u8, u8, u8 full; full 1: 104 bytes follow, 0: one u64` = 24, 40, 120 or 136 bytes. In the full form the destroyed and captured bytes and the mount are at the offsets `SP\B\c587b.py` reads | by the rule |
| `6ea51c35` | damage list, to the end of the message: `u32 count`, per entry `fe` or a record of 109 + 40 n bytes (below) | to the end |
| any other | unknown size: stop reading the message there, keep what was read | |

Damage record: `+0 u8 kind, +1 3 x f32 point, +13 f32, +17 4 x f32 direction, +33 u64 instigator, +41 u64 0, +49 u64 damage id, +57 u32 shooter slot, +61 u32 part, +65 9 x f32, +101 u32, +105 u32 n, n x {3 x f32 position, f32 0, 3 x f32 normal, f32 0, u32 index, f32 scale}`. Each update carries only new entries; a count can be several hundred.

Which entities get `changes`: every map object, and every entity with one of the five known classes or with no class at all (area entities). Player bodies (the ids in `players[].entities.movement`) get a thinned track in `bodies` instead; guns and attachments get nothing.

FX record: `u8 mask`, then one section per set bit in this order, each `u32 n` and its entries:

```text
01 spawn       36: u64 asset, u64 parent, u32 instance, u32, u64 target, u32
04 int         12: u32 instance, hash, u32
08 float       12: u32 instance, hash, f32
10 vector      24: u32 instance, hash, 4 x f32
20 quaternion  24: u32 instance, hash, 4 x f32
02 stop         7: u32 instance, u8, u8, u8
40 points      u32 instance, u32 capacity, u32 count, count x 4 x f32
80 sound       per entry a type byte: 01 = 34 bytes, 02 = 6 bytes, 03 = 41 bytes (sizes include the type byte: check against `SP\E\reference.py`)
```

The snapshot has the same layout. Point lists are cumulative: keep the last.

- [ ] **Step 1: failing test.** `tests/world.rs`: every test round, full read, has `decodeStatus.world` `decoded` with `unparsed == 0` for FX records and a movement `unparsed` share under 0.1% of tracked updates.
- [ ] **Step 2:** `cargo test --test world` fails (no such status).
- [ ] **Step 3:** implement `world.rs` with unit tests on synthetic bytes: each transform sub bit, each component, a damage record with two impacts, an `fe` entry, an unknown class stopping the walk, a truncated message.
- [ ] **Step 4:** implement `fx.rs` with unit tests per section, and one record with sections `01`, `20`, `02` to pin the order. Move `shots.rs` onto it (its hit effects: asset `d5 6d 41 58`); `tests/shots.rs` must still pass unchanged.
- [ ] **Step 5:** compare against Python: counts of creates, deletes, `live` changes, damage records and `fe` per round equal `SP\D\reference.py`'s `counts` and `SP\A\reference.py`'s `checks`; FX spawn and stop counts equal `SP\E\scan.py`'s.
- [ ] **Step 6:** `cargo test`, `cargo clippy --all-targets`, `cargo bench` round/full before and after (report both). Commit.

### Task 2: `src/panels.rs` — reinforcements and barricades

**Files:** Create `src/panels.rs`, `src/types/map_tables.rs` (panel assets), `tests/panels.rs`. Modify `src/lib.rs`, `src/round.rs`.

**Interfaces:**
- Consumes: `World`, `Entity`, `Change`, `Placed`, `Damage`, `World::body_at` (Task 1); `Input`.
- Produces: `pub(crate) fn decode(input: &Input, world: &World) -> Decoded { reinforcements: Vec<Reinforcement>, barricades: Vec<Barricade>, warnings }`; JSON keys `reinforcements`, `barricades`; `pub(crate) fn kind_of(asset: u64) -> Option<PanelKind>` for Task 5.

**Format.** A panel is an entity with classes `4c60869a 6ea51c35` and the slot set {`b4d93e43`, `2e4bce49`} (compare as a set: the order differs between builds; Black Mirror has the same classes and other slots). Life: start = first position that is not the pool, with `02 <playerid>`; complete = `live` 1, host in that message or up to 4 frames later; cancelled = back to the pool or deleted before `live` 1; default = placed by its full state with no owner (the map's own barricades, Quick Match's reinforcements). Normal = quaternion applied to (0,0,1). A start without a quaternion keeps identity.

Assets: 406076330074 hatch reinforcement; 406076330089/090 barricade door wide/door, 091/092 window wide/window; 361321226204/207 Castle door wide/door, 213/210 Castle window wide/window; wall reinforcements 417911059814, 417911060317 + 623 i (i = 0..8; width 1.6 + 0.1 i m, inferred), 417911026321. Unknown asset: normal up = hatch; default or under 3.3 s = barricade; else wall.

Barricade end: `fe`; who and how = the last damage entry at most 2 s before it, ignoring id 42593091656: kind 1 `bullet`, id 34118943362 `melee`, instigator a gadget entity `gadget` (with its owner), else `other`; source `read`. With no entry: the nearest body within 2.5 m, `removed` when it stands 0.38-0.42 m away horizontally, else `brokenThrough`; source `proximity`.

JSON: `reinforcements[]` `{entity, kind: wall|hatch, username, host, position, normal, width?, widthInferred, started: When, completed: When?, cancelled, default, opened?: When (set by Task 5)}`; `barricades[]` `{entity, kind: barricade|castle, opening: door|window (inferred), wide, username?, host?, position, normal, default, started?, completed?, cancelled, destroyed?: {when, by?, how, source}}`.

- [ ] **Step 1: failing tests** on the 10 test rounds: completed reinforcements per round are 8, 10, 9, 9, 9, 10, 9, 8, 10, 9; every completed one has a defender `username` and a `host`; wall ones complete 4.0-4.2 s after the start and hatch ones 4.3-4.5 s; every round has 18 default barricades; every non-default barricade completes 2.4-2.7 s after its start.
- [ ] **Step 2:** watch them fail to compile.
- [ ] **Step 3:** implement; unit-test the life rules on synthetic `Change` lists (complete, host late, cancelled, default).
- [ ] **Step 4:** output equals `SP\C\out_c1.json` .. `out_c10.json` (entity, player, frames, host, kind, destroyed).
- [ ] **Step 5:** real-folder test: never more than 10 completed reinforcements per round; all by defenders; every Castle panel owned by a Castle. Commit.

### Task 3: `src/gadgets.rs` — gadget objects

**Files:** Create `src/gadgets.rs`, `src/types/gadget_tables.rs`, `tests/gadgets.rs`. Modify `src/lib.rs`, `src/round.rs`.

**Interfaces:**
- Consumes: `World` (Task 1); `Input`; `Round.loadouts` names via `crate::loadout::hud_items` as `throws.rs` does.
- Produces: `pub(crate) fn decode(input: &Input, world: &World) -> Decoded { gadgets: Vec<Gadget>, cameras: Vec<MapCamera>, warnings }`; `Gadget.entity: u64` (kept, serialized as hex) for Task 4's join; JSON keys `gadgets`, `mapCameras`.

**Format.** Placed gadget: an entity with class `4c60869a` that is not a panel (Task 2's rule) and not the defuser (`4c60869a d0f65929`). Start = `02 <playerid>` with the first real position (wall charges and Black Mirror write `06 <playerid> <host>`); called off = back to the pool, the retry does not repeat the owner; deployed = `live` 1; `live` 0 after that = destroyed or taken; delete = gone. Thrown gadget that stays: classes `[8490f616]`, `[513b13b2, 8490f616]` or `[587f5a72, 513b13b2, 8490f616]`; released = owner flag 1; rest = end of the first run of positions without a 0.25 s pause; drones (classes `47e5f600`, `7a8ac28f`) stay with `throws`. Owner-less children (Kiba barrier 375382544571 from kunai 373332376744, R.O.U. posts 377423931277 from ball 376357912467, D.O.M. panel 418590879539/534 from 416011594318) take the owner of the nearest thrown object of the parent asset at that moment. Area entities (no classes; Task 6's table) are not gadgets.

Type: `typeIndex` from the full state of `4c60869a` / `8490f616`; name from the owner's body slot whose asset equals the entity's (`nameSource: slot`), else `gadget_tables` by asset, else by type index (`nameSource: table`, `inferred: true`). Tables from `SP\A\gadget_assets.json` (228) and `gadget_types.json` (72).

End (`end.how`, with `source`): `presentAtEnd`; `wentOff` (delete with no marker and no HUD gain); `pickedUp` (`live` 0 then delete 0.7-1.1 s later, or a bare delete of a thrown gadget inside a rise of the owner's HUD count; `source: inferred`); `destroyed` (`live` 0 with the type's other delay, flags `4000` on wire and mats, `fe` on a shield). Task 4 adds who.

JSON `gadgets[]`: `{entity, kind: placed|thrown, typeIndex, asset, name, nameSource, slot?, username, position, rotation, host?, parent?, placing?: When, deployed: When, rest?: [x,y,z], cancels, states: [{state, when}], end: {how, source, when?}}`. `mapCameras[]`: `{object, position, rotation, destroyed?: When}` (map objects of class `587f5a72`; destroyed = flags `4000`).

- [ ] **Step 1: failing tests**: every HUD count drop (`loadouts[].ability.uses` and `gadget.uses`) of a slot whose item is a placed gadget has exactly one gadget of that owner and name deployed within -0.3..+0.8 s (118 of 118 in the test rounds; Armor Pack allowed +2.1 s); every gadget names a player of the round; the owner's body is within 4 m at the placement start except Aqua Breacher; no deployed gadget is at the pool position; 8 map cameras per round.
- [ ] **Step 2:** watch them fail.
- [ ] **Step 3:** implement; unit-test life rules on synthetic `Change` lists (called off and retried, deployed, picked up by delay, wire going `4000`).
- [ ] **Step 4:** output equals `SP\A\reference.py`'s for the 10 rounds (entity, type index, name, owner, frames, end).
- [ ] **Step 5:** real-folder test: every placed gadget's owner is a player; no entity is two gadgets. Commit.

### Task 4: `src/gadget_events.rs` — score, removals, statuses, traps

**Files:** Create `src/gadget_events.rs`, `tests/gadget_events.rs`. Modify `src/lib.rs`, `src/round.rs`.

**Interfaces:**
- Consumes: `World`, `Effects` (Task 1); `Input`; `&[crate::shots::Shot]`, `&[crate::combat::Hit]`, the vitals effects, kill feed (all already on `Round` when it runs); HUD records via `crate::entities::for_each_record` as `vitals.rs` reads them.
- Produces: `pub(crate) fn decode(input, world, fx, round_so_far) -> Decoded { score: Vec<ScoreChange>, removals: Vec<Removal>, statuses: Vec<Status>, traps: Vec<TrapTrigger>, warnings }`, each event carrying `entity: u64`; JSON keys `scoreChanges`, `gadgetStatuses`, `trapTriggers`; removals are joined into `gadgets[].end` in Task 7.

**Format.** Port `SP\B\reference.py` (900 lines, self-contained) section by section; its findings in short:

- Score: `MatchScore` `[0xEC, 0xDA, 0x4F, 0x80]`, i32, on the player's scoreboard object (`players[].entities.scoreboard`). One summed write per frame; there is no per-event feed. `scoreChanges[]` `{username, delta, total, reason?, detail?, reasonSource: coincidence, when}` with the reasons of `SP\B\tables\score_deltas.json`.
- Removal signals on a deployed entity: `live` 1 to 0, device destroyed flag (`587f5a72`), flags `4000`, `fe`, delete, owner flag back to 0; signals within 0.3 s belong together. What a pattern means per type: `SP\B\tables\gadget_removal_signatures.json`.
- By whom (`bySource: score`): the opponent whose score rises by the gadget's points (10; 20 Black Eye, Welcome Mat, Claymore; 5 T.R.I.P.) between -0.25 and +0.13 s; a teammate's -10 first; one write pays for `amount / points` removals; more removals than points in the window sets `ambiguous`.
- How (`meansSource`): `shotRay` when a shot of that player passes within 0.6 m in the 0.45 s before (with `weapon`), `explosion` when an explosive of theirs ended within 0.35 s and 8 m, else absent.
- Statuses (FX spawn with the gadget as parent, until its stop; assets in `SP\B\tables\status_fx.json`): `empDisabled`, `frozen`, `hacking`, `hacked`, `caught`, `adsFired`; `captured` = the owner component's alliance rewritten on a live object. `by` from the score in the next frames (`bySource: score`).
- Jams: HUD effect 8 on a player with the nearest live Signal Disruptor within 3 m of the body (`jammerSource: nearest`), added to `effects[]` entries as `jammer` (owner) in Task 7.
- Traps: Razorbloom state `Closed` (`d2486b03`) to `Opening` (`5fed88c1`); Fenrir HUD mine `State` 3; Proximity Alarm FX 73593090661; Banshee FX 262855816735; Gu, EDD, Grzmot, Claymore removed with no `live` 0; Welcome Mat flags `4000`. Victim = the `hits` or `effects` entry in the same frames (`victimSource: time`), plus `nearestEnemy` with its distance. Barbed wire has no marker: hits of type 12 name the wire's owner by the nearest live wire within 1.5 m.

- [ ] **Step 1: failing tests**: every score change names a player and the per-player sum equals the scoreboard's final `score` where the JSON has one; every removal with `by` names a player of the other team unless `friendly`; a status `until` is not before its `when`; every trap trigger's entity is a gadget of Task 3 when both run (join test lives in Task 7).
- [ ] **Step 2:** watch them fail.
- [ ] **Step 3:** implement score, removals and who/how; unit-test the pairing (one +30 paying three removals, penalty first, ambiguous).
- [ ] **Step 4:** implement statuses and traps.
- [ ] **Step 5:** output equals `SP\B\out\` for the 10 test rounds (removals: entity, cause, by, means; traps: entity, time, victims; statuses).
- [ ] **Step 6:** real-folder test: destroyed drones and cameras with a scorer are at least 90%; every `captured` has a Mozzie or Brava in the round. Commit.

### Task 5: `src/destruction.rs` — destruction, surfaces, breaches

**Files:** Create `src/destruction.rs`, `src/types/damage_tables.rs`, `tests/destruction.rs`. Modify `src/lib.rs`, `src/round.rs`.

**Interfaces:**
- Consumes: `World`, `Damage`, `World::to_map` (Task 1); `Input`. Panels are recognised by the class and slot rule of Task 2, restated here so the task stands alone: classes `4c60869a 6ea51c35`, slot set {`b4d93e43`, `2e4bce49`}; hatch asset 406076330074.
- Produces: `pub(crate) fn decode(input: &Input, world: &World) -> Decoded { destruction: Vec<Destruction>, surfaces: Vec<Surface>, breaches: Vec<Breach>, opened: Vec<(u64, u32)>, warnings }` (`opened`: reinforcement entity and frame, for Task 7); JSON keys `destruction`, `surfaces`, `breaches`.

**Format.** Port `SP\D\reference.py`.

- Records: kind 0 not a bullet, 1 another player's bullet, 2 the recorder's own bullet predicted, 3 the same confirmed (drop 3). Skip preset damage: in a full state in the first second with instigator 0 or a map object. Map point = `position + q * point` with the object's transform at that time.
- Event = records grouped by (instigator, damage id, 0.15 s); `{cause: {id, name?, category}, username?, instigator?, position, objects: [{object, kind, impacts, destroyed?}], when}`. `username` = the body's player, or the owner of the instigating gadget entity (`4c60869a` owner or `8490f616` player). Bullets are left out of `destruction` (they stay in `surfaces`).
- Causes: `SP\D\causes.json` (72 non-bullet, 28 bullet ids, each with a status; carry `inferred: true` where its status says so).
- Object kind: `reinforcedWall`, `reinforcedHatch`, `barricade` from the entity; map objects from the catalog `SP\D\map_object_kinds.json` (per map: wall, floor, hatch, breakable; 2,483 objects, embedded as a table keyed by map id), else the vote of the impact rule (floor: `|n.z| > 0.99 and |p.z| <= 0.03`; wall: `|n.y| > 0.99 and |p.y| <= 0.12 and -0.05 <= p.z <= 6`, or the same on x with 0.03), else `object`; `kindSource: catalog|impacts`.
- Opened: a reinforcement's flags `8000` to `0000` within 0.35 s after a record; a hatch reinforcement also gets `fe`.
- Surfaces (all `derived: true`): impact points clustered at 0.45 m; labels `verticalPlay`, `rotationHole`, `breach`, `murderHole`, `bulletHoles` by the thresholds in `reference.py`; each with `width`, `height`, `points`, `causes`, `makers`, `phase`.
- Breaches: devices are placed gadgets whose name is Exothermic Charge, Hard Breach Charge, Breach Charge or the S.E.L.M.A. stage (asset 388352500293), X-KAIROS pellets (391794748337), and launched breachers (Ash 391794703535, Zofia 311776456053, Gonne-6 392233335597, Kali 392233337653). Outcome: `detonated` (deleted while `live` 1, records by it within 0.15 s), `destroyed` (`live` 1 to 0, no record, deleted about 3 s later), `removed`, `armedAtEnd`. `{device, username, entity, target, targetKind, position, placed, armed, outcome, when, affected: [...], openedReinforcement}`. For `destroyed` add `near: [{gadget, username, distance}]` listing Shock Wire (382651791603) and Electroclaw (238373641367) within 2.5 m, defenders' shots within 1.3 m, explosions within 3 m, and `stoppedBy` (`electricity`, `shot`, `explosion`) with `stoppedBySource: proximity` only for those three; nothing else is named a cause (the Python's Horus Lance guess is dropped).

- [ ] **Step 1: failing tests**: every destruction event's `username`, when present, is a player; every breach names a player and an outcome; in the test rounds every Thermite charge and Hibana volley detonated and `openedReinforcement`; round 7 has one `destroyed` Hard Breach Charge with no `stoppedBy`; no event is kind 3; every surface has `derived`.
- [ ] **Step 2:** watch them fail.
- [ ] **Step 3:** implement records to events with unit tests (grouping, preset skip, kind 2/3 pair).
- [ ] **Step 4:** implement kinds, opened, surfaces, breaches.
- [ ] **Step 5:** output equals `SP\D\out\c1.json` .. `c10.json` (event count, causes, objects; breach outcomes; surface labels) apart from the dropped guess.
- [ ] **Step 6:** real-folder test: every `openedReinforcement` breach has a reinforcement among `affected`; no kind 2 without its kind 3 counted twice. Commit.

### Task 6: `src/areas.rs` — areas, environment, light screens

**Files:** Create `src/areas.rs`, `src/types/fx_tables.rs`, `tests/areas.rs`. Modify `src/lib.rs`, `src/round.rs`.

**Interfaces:**
- Consumes: `World`, `Effects`, `Spawn` (Task 1); `Input`; `&[crate::shots::Shot]`, `&[crate::throws::Throw]`.
- Produces: `pub(crate) fn decode(input, world, fx, shots, throws) -> Decoded { areas: Vec<Area>, environment: Vec<EnvironmentEvent>, light_screens: Vec<LightScreen>, warnings }`; JSON keys `areas`, `environment`, `lightScreens`.

**Format.** Port `SP\E\reference.py`.

- Area = FX spawn with a position, no parent, an alliance and (usually) a stop. Kind by FX asset: 27012166679 smoke grenade, 73006469354 smoke bolt, 223502673865 fire, 297392295395 gas, 379510214638 swarm, 401787054484 extinguisher burst, 440937948606 one-cell fire.
- Source by the area entity (no classes) placed at the same spot in the same frame: 339570061825 Volcan, 361809340830 Shumikha, 361809343208 fire bolt, 407899098848 gas pipe, 361147040848 gas canister, 385049618011 swarm, 319577383929 Logic Bomb fire (inferred).
- Owner = the owner of the gadget entity that ended within 0.26 m in the 0.7 s before (grenade, canister, bolt), or the nearest owned entity within 1 m (hive, stuck bolt). A Volcan's `triggeredBy` = the shot ending within 0.6 m in the second before, else a grenade ending within 5 m; `triggerSource: shotRay|explosion`.
- `{kind, source, username?, position, alliance, points?, capacity?, radius?, radiusSource: assumed, started: When, ended?: When, triggeredBy?, triggerSource?}`. Smoke 3.0 m and extinguisher 2.5 m radii are assumptions and say so; areas with points give the cells instead.
- Environment: gas pipe explosion (FX 407430409561 = `5ed909cd59`, then the fire area 0.27-0.38 s later; `by` from the object's damage list, `bySource: read`, else an explosive ending within 5 m, `explosion`); fire extinguisher (FX 401787054484 with the object as parent; shooter from the bullet entry); metal detector alarm (FX `617eb2dce7` on its light object, 3.0 s; `by` the nearest body within 1.5 m, `bySource: nearest`). Objects per map from `SP\E\env_objects.json` where known.
- Light screens: R.O.U. posts (asset 377423931277) carrying FX `59b4a16992` from placement until its stop: `{username, posts: [[x,y,z]...], started, ended?}` grouped by the roll that dropped them.
- Kills gain `inArea` in Task 7: the victim's body within 1.0 m horizontally of a point (the point 1.0 m below to 2.2 m above the body) of a fire, gas or swarm area alive at the kill; `inAreaSource: derived`. Shots gain `throughSmoke` when the segment passes within the assumed radius of a smoke area alive at that time.

- [ ] **Step 1: failing tests**: every fire, gas and swarm area has a `source`; every area's `ended` is after `started`; gas areas last 9.7-9.9 s and smoke grenades 13.9-14.2 s when ended; every fire or gas `hits` entry (damage type 36 or 9) in the test rounds lies in such an area alive at that time, by the inside rule (16 of 16 and 8 of 8); rounds 4, 6, 7, 8, 10 have a gas area owned by the round's Smoke.
- [ ] **Step 2:** watch them fail.
- [ ] **Step 3:** implement areas; unit-test the join on synthetic spawns and entities.
- [ ] **Step 4:** implement environment and light screens.
- [ ] **Step 5:** output equals `SP\E\out_c1.json` .. `out_c10.json` (kind, source, owner, start, end).
- [ ] **Step 6:** real-folder test: every area with an owner names a player; no area's `ended` precedes `started`. Commit.

### Task 7: join, stats, README

**Files:** Modify `src/round.rs`, `src/stats.rs`, `src/feedback.rs`, `src/shots.rs` (field only), `src/decoder.rs`, `README.md`, `tests/replays.rs`.

**Interfaces:** Consumes the `Decoded` of Tasks 2-6 by the names above.

- [ ] **Step 1:** `gadgets[].end` takes Task 4's removal for its entity: `by`, `bySource`, `friendly`, `means`, `meansSource`, `weapon`, `ambiguous`; `gadgets[].statuses[]` and `gadgets[].triggers[]` likewise. Test: every removal, status and trap entity that is a gadget is attached; the rest stay in their own lists with a warning count.
- [ ] **Step 2:** `reinforcements[].opened` from Task 5's `opened`; `breaches[].target` gets `reinforcedBy` (the reinforcement's player). Gadgets whose `host` is a reinforcement or barricade entity get `hostKind`.
- [ ] **Step 3:** kills gain `inArea` `{kind, source, username}`; shots gain `throughSmoke: true`; `effects[]` type 8 gain `jammer`.
- [ ] **Step 4:** `stats[]`: `gadgetsDeployed`, `gadgetsDestroyed` (enemy gadgets, score-attributed), `gadgetsLost`, `reinforcements`, `barricades`, `breaches`, `breachesOpened`, `trapsTriggered` (own traps set off).
- [ ] **Step 5:** `decodeStatus`: `world`, `gadgets`, `gadgetEvents`, `panels`, `destruction`, `areas`, each with a count and warnings; inferred-only outputs (`surfaces`) report `inferred`.
- [ ] **Step 6:** bump the Y11S3 decoder revision; README: output rows and a "Gadgets, world and destruction" section (what is read, what is inferred and how often each rule held, what is not in the file); fix the lines the new work makes wrong (throws: "placed gadgets are not covered"; shots: FX layout; melee: "kind byte 0 melee").
- [ ] **Step 7:** `cargo test`, `cargo clippy --all-targets`, the same with `R6_MATCH_REPLAY` set, `cargo bench`.

## Requirement coverage

| Asked for | Where | Verdict |
|---|---|---|
| Gadgets placed or thrown: who, when | Task 3 | read |
| Gadget type and position | Task 3 | type index and asset read; name from the loadout slot or a table |
| Destroyed, disabled, jammed, hacked: by whom, how | Tasks 3, 4 | the event is read; who is inferred from the score, how from the shot ray; jam only as the player's effect |
| Traps and denial set off: by whom, damage | Task 4 | trigger read per trap type; victim by time; Grzmot victims, Airjab, Trax, Jackal not in the file |
| Breaches: tried, worked, what stopped them | Task 5 | outcome and opened read; what stopped it is not in the file, only what was near |
| Reinforcements: who, when, which wall or hatch | Task 2 | read |
| Barricades: placed, destroyed by whom | Task 2 | read; removal by hand by proximity |
| Destruction: holes, floors, hatches, doors, windows | Task 5 | impacts, cause and maker read; hole size not in the file; wall or floor from a catalog and a rule; labels derived |
| Smoke, fire and gas clouds: where, from, until | Task 6 | read; smoke radius not in the file |
| Environment objects | Task 6 | pipes and extinguishers read; which object is one is by a per-map table; detector user by nearest body |
