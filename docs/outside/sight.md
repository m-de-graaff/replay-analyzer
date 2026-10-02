## Line of sight

`src/sight.rs` answers who could see whom, how exposed a player was and where a player's crosshair was when an enemy came into view (Y11S3+, rounds read with `ReadOptions { movement: true, .. }`). It is a library module: nothing of it is in the CLI's JSON yet.

```rust
use replay_analyzer::sight::{self, Geometry, Options};

let open = sight::analyze(&round, None, &Options::default());           // open field
let geometry: Geometry = serde_json::from_str(&json)?;
let walled = sight::analyze(&round, Some(&geometry), &Options::default());
```

**The file holds no level geometry and no head position.** Both are filled in from outside the file, and the output says which was.

| | Where it is from |
|---|---|
| Body origin, view direction, stance, lean, on a rope, on a drone | Read (`movement`) |
| The eye | Derived: the origin plus a height per stance, measured from the fire events (below) |
| The points a body shows (head, chest, pelvis, knees, feet) | The head is the eye; the rest is assumed |
| Walls, floors, doors, windows, hatches | **Authored**: a `Geometry` handed in. Not in the file |
| Reinforcements, barricades, Armor Panels, with their times | Read (`reinforcements`, `barricades`), put on the wall of the geometry that names their `host`, else on the nearest |
| Holes in walls | Derived from `surfaces[]` labelled `breach`, `rotationHole` or `murderHole`; the size is assumed |
| Hatches gone | Read (`surfaces[].hatchDestroyed`, a hatch reinforcement's `opened`) |
| Smoke | Read (`areas[]` of kind `smoke` and `extinguisher`); the radius is assumed there |
| A player's field of view | Not in the file: a parameter, 90 x 60 degrees by default |

### The eye

A fire event states the distance from the shooter's eye to where the bullet ended, and that point follows from the muzzle, the direction and the muzzle distance. Fitting an eye offset to those distances over the 4,618 shots of the ten test rounds fired off a rope:

| | Above the origin | Median residual |
|---|---:|---:|
| Standing | 1.44 m | 1.5 cm |
| Crouched | 0.96 m | 1.3 cm |
| Prone | 0.39 m | 0.3 cm |
| On a rope | 0: the origin is at the eye there, upright and head down | 13 cm |
| Leaning | 0.12 m to the side leaned to | |

There is no offset forwards. A downed player is taken to be as low as a prone one (not measured). The fit also says something of `lean`, which the README calls unchecked: the side it names is the side the eye moves to.

Checked on every shot of a gun by a player on their feet (`sight::validate`):

| | Test rounds (10) | Real rounds (175) |
|---|---:|---:|
| Shots | 4,618 | 60,866 |
| Stated eye distance against the derived eye, median / nine in ten within | 1.4 cm / 10.6 cm | 0.6 cm / 4.8 cm |
| Bullet hits with a shooter | 264 | 5,486 |
| View direction off the point struck, median / 95% / most | 1.1 / 3.4 / 5.7 degrees | 0.9 / 4.0 / 36.2 degrees |
| Hits that downed or killed, within 8 degrees | 77 of 77 (most 4.6) | 1,543 of 1,552 |

The rest of the eye error is a stance changing: its number changes at once, the body takes a third of a second. The view direction is the one of the last sample at or before the hit; recoil, a shotgun's spread and a flick between two samples 35 ms apart are in its error.

Two things the fire events turned out to say, both worth knowing elsewhere:

- **A shot's `distance` is to what stopped the bullet, not to the first thing in its way.** Of the first shots at a soft wall in each test round, 44 of 62 end behind it. The README describes `shots[].distance` and `eyeDistance` as "what the bullet struck first".
- **The recording player's own shots state an eye distance 0.20 m longer** than anybody else's shot does, in every stance (5,694 shots of 175 rounds, 0.196 to 0.210 m by stance and lean). It is measured from behind the eye for them. `validate` leaves them out of the eye check.

### Geometry

`Geometry` is serde, camelCase:

```json
{ "map": "Bank", "source": "authored",
  "walls": [{ "id": "vault east", "a": [-53.0, -3.08], "b": [-53.0, -4.61], "bottom": 0.0, "top": 2.6,
              "kind": "soft", "object": "60572f5567" }],
  "slabs": [{ "id": "1F floor", "z": 0.0, "polygon": [[-95, -10], [-35, -10], [-35, 35], [-95, 35]], "soft": true,
              "openings": [{ "polygon": [[-56.7, 5.1], [-54.7, 5.1], [-54.7, 7.1], [-56.7, 7.1]], "hatch": true }] }] }
```

A wall is a vertical quad without thickness: a segment seen from above, from `bottom` to `top`. A slab is a polygon at one height with openings; an opening that is a `hatch` is closed until the round destroys it. `object` is the map object's id in hex, the `host` of a reinforcement or barricade, and is how a panel finds its wall; without it the nearest wall within 0.6 m takes it.

| Element | Sight | Bullet |
|---|---|---|
| `solid` wall, slab | stopped | stopped |
| `soft` wall, `soft` slab, closed hatch | stopped | passes |
| The same, reinforced | stopped | stopped |
| `window`, `door` | passes | passes |
| The same, barricaded | stopped | passes |
| The same, with an Armor Panel | stopped | stopped |
| `seeThrough` | passes | stopped |
| A hole, an open hatch, an opening | passes | passes |
| Smoke | stopped | passes |

`Scene::new(&geometry)` indexes it in a grid of 2 m cells; `Scene::for_round` applies the round's state, and `scene.applied()` counts what found a wall and what did not. Every query takes the time. `Scene::open_field()` has no geometry: nothing stops a line.

**Without geometry the output says so**: `geometry: "none"`, `occlusion: false`. Every pair alive then "could see" each other, a sightline is the time both were alive, and only `inFov`, the distances and `engagements` carry information. Nothing in it means a wall was tested.

### Outputs of a round

| Key | What it holds |
|---|---|
| `geometry`, `occlusion`, `state` | `none`, or the geometry's source and map; whether lines were tested; what the round put on it. |
| `sightlines[]` | Each stretch in which nothing stood between the eye of `from` and the body of `to`: `start`, `end`, `mutual` and `mutualSeconds` (the other way too), `inFovSeconds` and the stretches `inFov[]` in which `to` was inside the field of view of `from`, `headSeconds`, the mean `fraction` of the body seen, and the `distance` at the start with the least and the most. Tested every 0.1 s; a line lost for 0.25 s or less is one sightline. Enemies only unless `teammates` is set. A player on a drone or a camera is seen and does not see. |
| `exposure[]` | Per player: seconds `alive`, `exposed` (at least one enemy could see them), `inView` (and had them in the field of view), the number of `enemies` that ever could, the most at once and the mean while any could, and the same per phase. |
| `firstSights[]` | Each time an enemy came into a player's view after 0.25 s or more out of it: `entry` (`appeared`: the line cleared while the player looked that way; `turned`: it was clear and the player turned to it; `start`), and how far the view direction was off the enemy's head: `yaw` and `pitch` in degrees, the `angle` between them, the `height` in metres the crosshair passed over (or under) the head, and the `distance`. |
| `engagements[]` | The first bullet of a player to strike an enemy after 5 s without one: `errorAtHit`, and `before`, the same errors off the enemy's head 0.5 s earlier. A hit proves a clear line, so this needs no geometry. |
| `crosshair[]` | Per player, medians of the absolute errors: `yawError`, `pitchError`, `angleError`, `headHeightError`, the mean signed `headHeightBias`, over the `appeared` first sights; `engagements` and `errorBeforeHit`. `basis` is `fovEntry` without geometry: an enemy then comes into view only by the edge of the field of view, the yaw error is near half of it by construction (median angle 35 degrees in the test rounds), and only `errorBeforeHit` says anything of crosshair placement. |

In the test rounds an open field gives 1,530 sightlines and 99 engagements; the crosshair was 4.2 degrees off the enemy's head half a second before the first hit (median) and 0.8 degrees off the point struck at it.

### Validation against the replays

A bullet that struck a player proves that nothing solid stood between the shooter's eye and the point struck, and so does every shot up to where it ended. `sight::validate(&scene, &round)` counts the hits and shots a geometry contradicts (`blocked`, `shotsBlocked`, with each hit in `misses[]`), and those that crossed something that stops only sight (`sightBlocked`, `shotsThrough`): a soft wall, a barricade, smoke.

No authored map exists yet, so the geometry tested is `Geometry::harvest`: the panels rounds put up. Every reinforced wall becomes a `soft` wall, every barricaded door and window an opening, every reinforced hatch a patch of floor with a hatch in it. For Bank the ten test rounds give 23 walls, 14 doors, 16 windows and 6 hatches. That is a small part of a map, and its extents are assumed (below), but it tests the whole path: matching panels to walls, their times, and the ray tests.

| | Test rounds, one geometry of all ten | Real rounds (175, 15 maps), each with its own panels |
|---|---:|---:|
| Panels of the round that found their wall | all | all |
| Bullet hits | 264 | 5,486 |
| Stopped as bullets: the geometry is wrong | 0 | 4 (0.07%) |
| Stopped as sight: through a soft wall, a barricade or smoke | 20 | 454 |
| Shots | 4,618 | 60,866 |
| Went on behind something that stops a bullet | 2 (0.04%) | 40 (0.07%) |
| Went through something soft | 1,091 | 7,625 |

The same Bank walls with none of the round's state applied stop no bullet (no wall is reinforced) and hide 48 victims in place of 20: the holes and the barricades that went account for the difference.

The panel extents were set on the 175 real rounds, so the right column is a fit, not a test. With a panel 3.0 m high and as wide as `width` says, 148 shots went on behind a standing reinforcement, 84 of them through its last tenth and 18 over 2.6 m; a harvested panel is therefore 2.6 m high and nine tenths of its `width`. Another 34 crossed an Armor Panel, 32 of them in its lowest 0.3 m: where its position is on the frame is not known, and that was left as it is.

### Timing

Measured in a release build on a machine doing other work, not benchmarked: one test round (214 s, 10 players, 2,100 tests of 50 pairs) takes 30 to 55 ms as an open field and 120 to 240 ms against the harvested geometry. 500,000 random lines across an 80 x 60 m map of 2,000 walls take 0.15 to 0.6 s.

### Limits

- **No map is authored.** Everything said of occlusion needs a `Geometry` from outside; the harvested one is a test fixture. Its `appeared` first sights are not crosshair placement: a handful of panels in an otherwise open field.
- **Walls have no thickness, bodies are five points.** A line that grazes a corner or a shoulder is decided by a centimetre.
- **A stance is a number.** The eye jumps where the body takes a third of a second, and a prone or downed body is laid out along the view direction, not the body's own heading.
- **On a rope the eye is the origin**, to 13 cm, and the body hangs straight down from it.
- **Hole sizes are assumed**: the extent of the impacts and 0.15 m around, at least 1.0 x 1.4 m for a breach or rotation hole and 0.3 x 0.3 m for a murder hole. A reinforcement that is `opened` goes back to a soft wall with whatever holes `surfaces` gives; holes in floors other than hatches are not applied.
- **A barricade is whole until it is destroyed.** One shot half away still hides what is behind it.
- **Smoke is a ball of the assumed radius** (3.0 m), opaque from its first frame to its last.
- **The field of view is one setting for everybody**, and a window in yaw and pitch, not a frustum. No scope narrows it.
- **Drones and cameras do not see.** A player on one is a target only; what the device could see is not computed, though `spots` could check it.
- **Glass, shields, deployable shields, Mira windows, light screens and gadgets** are not in the state applied.
- **Tested 0.1 s apart**: a line open for less can be missed.

### Tests

`cargo test --lib sight` (12 unit tests: ray against quad and slab with openings, what stops what, state over time, stance eye heights, the field of view, intervals, the grid against a brute-force test, JSON) and `cargo test --release --test sight` (8, on the test recordings; `real_rounds_hold_the_same` reads `R6_MATCH_REPLAY` and is skipped without it). `-- --nocapture` prints the numbers of the tables above.
