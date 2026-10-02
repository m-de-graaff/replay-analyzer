## Maps

A replay holds no floor plan, no room outlines and no blueprint of the building. It does hold the objects of the map a round touched, each with the same id and the same place in every round on that map, and where players stood. `replay_analyzer::mapdata` puts those into one file per map id, in a schema that also holds what somebody draws by hand, so a map fills in with every replay parsed and the hand-drawn parts stay. Nothing in it comes from the game's files.

```rust
use replay_analyzer::mapdata::{self, MapData};

// One file per map id; `stored` is what the app has for this map so far.
let rounds: Vec<_> = paths.iter().filter_map(|p| mapdata::read(p).ok()).collect();
let found = mapdata::harvest_with(&rounds);
let stored: MapData = mapdata::merge(&stored, &found);

let floor = stored.floor_at(z);                // which storey a body at this height is on
let room = stored.room_at(x, y, z);            // needs outlines, which are drawn by hand
let (opening, metres) = stored.nearest_opening(x, y, z)?;
```

`cargo run --release --example mapdata -- <match folder or MatchReplay folder> <out dir>` writes `<map id>-<name>.json` per map and merges into a file that is already there. [413779563590-BankY10.json](413779563590-BankY10.json) is what the ten test rounds give (150 KB). The key is the map id (`summary.map.id`): a rebuilt map gets a new id and a new file.

Numbers below are from the ten test rounds (Bank) and from a real `MatchReplay` folder: 178 rounds on 15 maps, read with `--movement`.

### What a round holds of the map

| | Where | How |
|---|---|---|
| Bombs | Eight map objects per map, two per site, in every round; the round's two have a name in the header. | Read; which name is which bomb by order, assumed (see [Objective](../../README.md#objective)) |
| Default cameras | `mapCameras[]`: id, place, rotation. | Read |
| Reinforced walls | A reinforcement names its wall (`host`) and stands at the wall's foot, in the middle of its width. | Read; the width by the asset, inferred |
| Hatches | A hatch reinforcement names its hatch and lies in its middle; the hatch itself is a destructible object with its origin at a corner. | Read; 2 m square derived |
| Doors and windows | A barricade is at the top of its opening, in the middle. Only an opening that was barricaded is seen. | Read; door or window by the asset, inferred; the width assumed |
| Destructible objects | `627385fe` creates one when the round damages it: id, origin, rotation. No size, and no word on what it is. | Read; wall, floor, hatch or prop from the catalog or the round's impacts |
| Spawns | Each attacker's `spawn` name and `spawnPosition`. | Read; one point per spawn derived |
| Floor heights | Not written. Walls, hatches, doors and bombs stand on few heights. | Derived |
| Floor names | The prefix of the site names: `B`, `1F`, `2F`. | Derived |
| Where one can walk | Every player's position, 28 times a second. | Derived, with `--movement` |
| Room names | The two sites of the round, and the room of each Fenrir mine that went off. Nothing else: a player's callout is not in the file. | Read, as points |
| Room outlines, walls that do not break, doorways nobody barricades, stairs | Not in the file. | Authored |

- **A round lists only what it changed.** The stream never lists the map. A round creates the 8 bombs, the cameras, and the destructible objects that were shot, blown up or walked through: 11 to 283 of them, 102 at the median (187 rounds). Bank has 1,064 distinct ones after 31 rounds and still gains about ten a round. A wall nobody touched in any round parsed is not in the file of the map.
- **A reinforced wall is another object than the wall that breaks.** The `host` of a reinforcement is never created in the stream: its id is known only from the reinforcement, its place from where the reinforcement stands. The destructible wall at the same place has its own id, which is what `destruction[]` names. 555 of the 998 destructible walls of the real folder have their origin on a reinforced wall and say so in `partOf`; the reinforced wall lists them in `parts`.
- **Where a reinforcement stands.** 0.1 m off the middle plane of the wall, on the player's side: a wall reinforced from both sides has two places 0.2 m apart (72 of the 418 hosts seen), and the wall is between them. Neighbouring walls of one asset are exactly its width apart (three 2.0 m walls on Bank at x -57.7, -55.7, -53.7), and a destructible wall's origin lies half the asset's width from the reinforcement's middle, which is what the widths `panels` infers (1.6 to 2.4 m) predict.
- **A destructible wall has no length.** Its origin is at its foot, its x axis runs along it, and the origin is at an end of the wall or in its middle, differing from wall to wall. `a` and `b` are the reach of the impacts on it (`destruction[].objects[].impacts`, `surfaces[]`) along that axis: 588 of 998 have one, 1.3 m at the median, and 221 are under a metre. A wall with `a` equal to `b` is known by its origin alone. This is a lower bound, not a width.
- **A hatch is 2 m square.** In 25 of 27 reinforced hatches the hatch under the reinforcement was seen too, and the reinforcement lies at (1, 1) of it.
- **A door is 2.2 m high.** 134 of 140 doors are 2.2 m under their barricade on a height a reinforced wall, a hatch or a bomb stands on; the other six are on a roof or a half level nothing else stands on. A window's barricade is 2.5 m (165) or 2.9 m (58) above a height of its floor. How wide an opening is and where a window starts are not in the file.
- **Doors and windows are those that were barricaded.** The map barricades the openings to the outside (88 doors, 225 windows); players add inside doors (61 doors name a frame). A doorway nobody barricaded in any round parsed is missing, and one that cannot be barricaded always is.
- **No room names for players.** Searched again for this: the text of 22 decompressed rounds (58 to 92 MB each) holds 2 to 13 room names a round, all of them the header's two sites and the rooms of Fenrir's mines. The parser gives a mine's room only when the mine goes off (`trapTriggers[].location`, 4 in 188 rounds).

### The file

```json
{
  "schema": 1,
  "map": { "id": 413779563590, "name": "BankY10", "base": "Bank", "version": "Y10" },
  "rounds": ["<match id>/1", "..."],
  "bounds": { "min": [-134.0, -35.0, -5.101], "max": [-21.0, 51.0, 9.45], "source": "derived" },
  "floors": [
    { "index": 0, "name": "B", "z": -3.8, "ceiling": 0.0, "levels": [-3.8], "source": "derived", "rounds": 10 }
  ],
  "rooms": [
    { "name": "B CCTV Room", "floor": 0, "polygon": [], "anchor": [-68.022, 7.681, -3.8], "source": "read", "rounds": 5 }
  ],
  "sites": [
    { "name": "B CCTV Room", "objectId": "60572f3500", "position": [-68.022, 7.681, -3.8], "floor": 0,
      "partner": "60572f71b8", "source": "read", "rounds": 10, "played": 5 }
  ],
  "spawns": [
    { "name": "Alley Access", "position": [-22.0, -34.1, 0.0], "radius": 0.0, "source": "derived", "rounds": 9, "players": 15 }
  ],
  "doors": [
    { "a": [-70.4, 0.2], "b": [-71.6, 0.2], "normal": [0.0, 1.0], "bottom": -3.8, "top": -1.6, "floor": 0, "width": 1.2,
      "objectId": "60572f4834", "seeThrough": true, "source": "read", "rounds": 2, "assumed": ["width"] }
  ],
  "windows": [
    { "a": [-45.69, -6.9], "b": [-45.69, -5.7], "normal": [1.0, 0.0], "bottom": 1.07, "top": 2.47, "floor": 1, "width": 1.2,
      "defaultRounds": 10, "seeThrough": true, "source": "read", "rounds": 10, "assumed": ["width", "bottom"] }
  ],
  "hatches": [
    { "position": [-75.2, -3.399, 0.0], "corners": [[-76.2, -4.399], [-74.2, -4.399], [-74.2, -2.399], [-76.2, -2.399]],
      "size": 2.0, "floor": 1, "objectId": "60572f75ab", "panelId": "60572f4fee", "reinforceable": true, "source": "read", "rounds": 5 }
  ],
  "walls": [
    { "a": [-62.6, 4.1], "b": [-62.6, 5.8], "floor": 0, "bottom": -3.8, "top": -0.8, "kind": "reinforceable", "width": 1.7,
      "objectId": "60572f14ce", "parts": ["60572f1451", "60572f29b9", "60572f3aab"], "source": "read", "rounds": 4, "assumed": ["top"] },
    { "a": [-73.0, 8.5], "b": [-73.0, 8.974], "floor": 0, "bottom": -3.8, "top": -0.8, "kind": "soft", "width": 0.474,
      "objectId": "60572f6e3d", "origin": [-73.0, 8.5], "source": "derived", "rounds": 5, "assumed": ["top"] }
  ],
  "cameras": [
    { "objectId": "60572f1958", "position": [-102.9, -3.2, -1.0], "rotation": [0.0, 0.0, -0.724, 0.69], "floor": 0, "source": "read", "rounds": 10 }
  ],
  "pieces": [
    { "objectId": "60572f144c", "kind": "unknown", "position": [-72.809, 8.599, 0.0], "rotation": [0.0, 0.0, -0.342, 0.94], "source": "read", "rounds": 1 }
  ],
  "walkable": [
    { "floor": 0, "cell": 1.0, "origin": [-134, -7], "rows": ["...##..", "..."], "source": "derived", "rounds": 10 }
  ]
}
```

Positions are the game's: metres, z up, the same as every position in the round output. Object ids are hex, as everywhere.

| Key | What it holds |
|---|---|
| `source` | On every element: `read` (the file states it), `derived` (worked out from what it states) or `authored` (drawn by hand). |
| `rounds` | On every harvested element: the rounds it was seen in. On the map data: the rounds harvested, as `<match id>/<round number>`, so an app can skip one it has. |
| `assumed` | The fields of the element that are neither read nor measured: a door's `width` (1.2 m, 2.2 m for a wide one), a window's `bottom` (1.4 m under its top), a wall's `top` (3 m above its foot, or the floor above where that is lower), the `ceiling` of the top floor. |
| `floors[]` | Lowest first. `z` is the height most of the floor stands on, `levels` every height that belongs to it (Club House's ground floor is at -0.5 and 0.4), `ceiling` where the next floor starts. `index` is the place in the list and changes when a floor is found below. |
| `rooms[]` | A name, a floor and an outline. A harvested room has no outline, only the `anchor` the file named it at. |
| `sites[]` | One entry per bomb: the two of a site name each other in `partner`. `played` counts the rounds it was the objective, `name` is absent for a bomb never played. |
| `spawns[]` | The median of where the attackers who picked the spawn first stood, and the median distance from it. |
| `doors[]`, `windows[]` | Two ends over the ground, `bottom` and `top`, `normal`, `wide`, the frame's `objectId` when a player's barricade named it, `defaultRounds` when the map barricades it. `seeThrough` is true: an opening hides nothing until something closes it, which is in the round's `barricades[]`. |
| `hatches[]` | The middle, the four corners, the floor it is the floor of. `objectId` is what a hatch reinforcement names, `panelId` the destructible hatch. |
| `walls[]` | Two ends over the ground on the wall's middle plane, `bottom`, `top`, and `kind`: `reinforceable` (a reinforcement was seen on it), `soft` (it breaks, and none was) or `solid` (authored: nothing breaks it). A `soft` wall with `partOf` is the same stretch as the reinforceable wall it names; leave it out of a test of what walls hide. |
| `cameras[]` | Place and rotation of each default camera. |
| `pieces[]` | Every other destructible object: `floor` panels, `breakable` props and panes, and `unknown` ones. Origin and rotation, no size. |
| `walkable[]` | Per floor, a grid of 1 m cells where a player stood on their feet: not in the air, on a rope, vaulting or dead. `rows[r]` is the row at `y = origin[1] + r`, its characters the cells from `x = origin[0]`. |

A wall or an opening is what a line-of-sight test needs: a segment over the ground with a bottom and a top, and whether one sees through it.

### Floors

A floor is a group of heights that walls, doors, hatches and bombs stand on, not more than 2 m above the lowest of the group: a storey is 3.4 m and more in every map seen, a split level 1.9 m at most. Destructible walls do not count, since some of them do not stand on the floor.

- The ten test rounds give Bank its three: `B` at -3.8, `1F` at 0, `2F` at 4.0. All 15 maps come out with two or three floors, 40 in all.
- **A floor's name needs a site played on it.** 33 of the 40 have one. Bank's `1F` has none in the real folder, where nobody played a first-floor site in 21 rounds.
- **A roof with a door is a floor.** Fortress gets a third, unnamed floor at 30.5 from two roof doors.
- **A level nothing stands on is none.** Nine windows of Tower are above its top known floor: its upper level has no reinforced wall, door or bomb in the four rounds parsed.
- `floor_at(z)` gives the highest floor that starts at or below `z`, with 0.6 m to spare; stairs belong to the floor they leave, and a roof (at or above the top floor's ceiling) to none. It says which storey a height is in, also outside the building.

### Merging

`merge(a, b)` holds everything of both, the same thing once: by object id, else by place within 0.35 m (a room and a spawn by name).

- **An authored element is never changed.** It wins over a harvested one at its place, so drawing a wall better than the harvest has it means copying the entry, correcting it and setting `"source": "authored"`. Authored floors stand for the derived ones within a metre of them.
- **`rounds` add up** when the two sides share no round, and are the larger of the two when they do, so merging a file with itself or with a part of itself changes nothing. Sides that overlap in part give a count that is too low.
- **Round by round is the same as all at once** for what is read: merging ten harvests of one round each gives the walls, doors, windows, hatches, sites, cameras, pieces and walked cells of one harvest of the ten, counts included (checked on the test rounds; walls, doors, windows, hatches and sites also on two halves of each of the 15 real maps). Spawn points and anchors are medians and come out a little different.
- An object is a wall as soon as one round says so; in the other rounds it is a piece of unknown kind, and the merge makes it the wall.
- `harvest` does not depend on the order of the rounds, and counts a round once however often it is given (a spectator's and a player's recording of one round are one round).

`harvest(&[Round])` works from rounds as the parser returns them. It has the reinforced walls, doors, windows, cameras, spawns and the sites in play, and no map objects, so no destructible walls, no pieces and only the bombs that were played. `harvest_with` takes rounds read by `mapdata::read`, which also scans the decompressed bytes for the `627385fe` messages. A round does not keep its map objects.

### Per map

| Map | Rounds | Floors | Rooms | Bombs (named) | Spawns | Doors | Windows | Hatches | Walls reinforceable | Walls soft (with a stretch) | Cameras | Pieces | Walked cells |
|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| BankY10 (test) | 10 | B -3.8, 1F 0, 2F 4 | 6 | 8 (6) | 3 | 14 | 16 | 7 | 23 | 146 (74) | 8 | 633 | 3,998 |
| BankY10 | 21 | B -3.8, ? 0, 2F 4 | 5 | 8 (4) | 3 | 13 | 17 | 7 | 29 | 135 (68) | 8 | 635 | 4,849 |
| BorderY10 | 13 | 1F -0.2, 2F 4.2 | 6 | 8 (6) | 3 | 9 | 10 | 1 | 30 | 101 (53) | 8 | 413 | 3,751 |
| CalypsoCasino | 6 | B 26.8, 1F 31.2, 2F 35.6 | 6 | 8 (6) | 4 | 12 | 15 | 3 | 29 | 68 (26) | 10 | 229 | 3,700 |
| ClubHouseY10 | 25 | B -4.4, ? 0.4, 2F 4.3 | 6 | 8 (6) | 4 | 19 | 8 | 3 | 34 | 95 (76) | 8 | 543 | 5,200 |
| CoastlineY11 | 14 | 1F 3, 2F 6.4 | 7 | 8 (6) | 3 | 11 | 15 | 0 | 30 | 84 (48) | 8 | 475 | 3,915 |
| ConsulateY10 | 20 | B 0, 1F 3.4, 2F 7.8 | 6 | 8 (6) | 4 | 16 | 15 | 3 | 37 | 81 (46) | 11 | 565 | 5,264 |
| FortressY10 | 6 | 1F 22.6, 2F 27, ? 30.5 | 8 | 8 (8) | 3 | 9 | 11 | 1 | 30 | 59 (39) | 11 | 239 | 3,776 |
| KafeDostoyevskyY10 | 6 | 1F 0, ? 4.4, 3F 8.8 | 4 | 8 (4) | 3 | 6 | 13 | 2 | 19 | 54 (27) | 7 | 321 | 3,096 |
| KanalY11 | 4 | ? 0.6, 1F 4, 2F 7.4 | 6 | 8 (6) | 3 | 7 | 20 | 2 | 16 | 18 (10) | 8 | 142 | 2,602 |
| LairY10 | 5 | B 27.2, 1F 31.2, 2F 35.2 | 6 | 8 (6) | 4 | 12 | 15 | 2 | 24 | 26 (13) | 11 | 184 | 3,702 |
| NighthavenLabsY10 | 20 | B 27.9, 1F 32.3, 2F 36.7 | 6 | 8 (6) | 3 | 8 | 12 | 3 | 29 | 73 (56) | 11 | 303 | 5,167 |
| SkyscraperY10 | 5 | ? 0.9, 2F 4.3 | 4 | 8 (4) | 3 | 5 | 12 | 0 | 12 | 39 (16) | 8 | 224 | 1,581 |
| ThemeParkY10 | 17 | 1F 0, 2F 4.4 | 8 | 8 (8) | 3 | 8 | 12 | 1 | 30 | 86 (56) | 8 | 374 | 5,557 |
| TowerY11 | 4 | 1F -2.2, 2F 2.2 | 6 | 8 (6) | 2 | 0 | 43 | 0 | 18 | 19 (9) | 8 | 163 | 1,777 |
| VillaY11 | 12 | B 6, ? 10, 2F 14.6 | 6 | 8 (6) | 3 | 5 | 16 | 1 | 24 | 60 (45) | 9 | 424 | 3,895 |
| 15 real maps | 178 | 40 | 90 | 120 (88) | 48 | 140 | 234 | 29 | 391 | 998 (588) | 134 | 5,234 | 57,832 |

A `?` is a floor no site was played on. The 15 files are 46 to 159 KB, 1.4 MB together.

How it was checked, on the ten test rounds and on every map of the real folder:

- Every completed reinforcement lies on a harvested wall or hatch that names its host, within 0.15 m (91 in the test rounds), and every barricade at the top of a harvested door or window of its kind, within 0.2 m (212).
- Every reinforced wall, door, reinforced hatch and bomb is on a floor; every site's name starts with the name of its floor.
- Every map has eight bombs, and the two of a site name each other.
- Every destructible object the test rounds created is in the file once: a wall, a hatch or a piece.
- The file reads back from JSON as it was written, with and without hand-drawn entries.

### Not in the file, or not known

- **Room outlines.** A room is a name and a point. `room_at` answers only for rooms somebody drew; `nearest_room` gives the nearest named point on the floor, which is a guess.
- **Every wall that does not break**, the outer walls first of all, and every wall nobody damaged. What is harvested is far from a floor plan: it is the soft part of the building.
- **Open doorways, stairs, ladders, vault spots.** `walkable` shows where players went; nothing says what connects two rooms, and with no outlines there is no room adjacency.
- **Widths of doors and windows, and where a window starts.** Assumed, and listed in `assumed`.
- **How long a destructible wall is.** Only the stretch that was hit.
- **What most destructible objects are.** 3,586 of the 5,234 pieces are `unknown`: no catalog entry and no impacts that tell a wall from a floor.
- **Which bomb is A**, and the name of a site nobody played (32 of 120 bombs).
- **A level nothing stands on**, and anything of a map on a build with another map id.
- **More room names.** The file has the room of every Fenrir mine in the HUD list of its owner, gone off or not, and (see [Movement](../../README.md#movement)) the rooms of Solid Snake's radar marks. The parser gives the first only for a mine that went off and does not decode the second; each would be a named point per mine or mark.
