## Operator catalog

A replay holds what each player picked, never what they could have picked. `catalog::operators` is the menu: per season, for each operator, the side, role tags, armor and speed, maximum health, the unique ability with its count, and the gadgets, primaries and secondaries to choose from. It is static data, keyed by the season in the header's version (`Y11S3`), and it is not part of a round's JSON.

```rust
use replay_analyzer::catalog::operators::{catalog_for, operator_info};

let ace = operator_info("Y11S3", player.operator);
let resolved = catalog_for(&round); // falls back to the latest season, with `fallback: true`
```

| Key | What it holds |
|---|---|
| `operator` | `{name, id}`, as everywhere else. |
| `side`, `sideEvidence` | `Attack` or `Defense`. |
| `roles` | The game's role tags (`Breach`, `Anti-Gadget`, `Intel`, ...). Always reference: no replay holds them. |
| `maxHealth`, `armor`, `speed`, `healthEvidence` | `players[].maxHealth` (100, 110 or 125), the armor it stands for (1, 2 or 3) and the speed left of 4. Absent for an operator nobody played. |
| `ability` | `id`, `name`, the `count` the HUD starts with, `max` when the HUD states one, `regenerates`, and `evidence`. Absent for Striker and Sentry, whose ability slot holds a second gadget (`gadgetSlots: 2`). |
| `gadgets[]` | `id`, `name`, `count` at spawn and `evidence`. |
| `primaries[]`, `secondaries[]` | `id`, `name` and `evidence`. A shield in the primary slot is listed as a primary. |

Ids are the HUD item ids of `loadouts[]` and the kill feed, so an entry joins to `loadouts[].primary.id`, `.ability.id` and the rest.

Every value says where it comes from. `evidence` is `{source, rounds}`:

- **`observed`:** read from replays, with the number of loadouts (one player in one round) it was seen in. `catalog::operators::observe(rounds)` is the harvest: per operator, the sides, maximum health, and every item seen in each slot with its counts at spawn. The season table is that harvest over the 10 test rounds and a real folder of 178 rounds: 1,837 loadouts on builds 9883691 to 9918362.
- **`reference`:** not in any replay read so far, and taken from the game's operator pages (October 2026): role tags, items no player picked, and Iana, the one operator nobody played. A reference item has its id when another operator was seen carrying it, and never a count.

What the 188 rounds cover, of 78 operators:

| Field | Observed | Reference | Unknown |
|---|---|---|---|
| Side | 77 | 1 | |
| Role tags | | 78 | |
| Maximum health, armor, speed | 77 | | 1 (Iana) |
| Ability | 75 | 1 | (2 have none) |
| Ability count | 74 | | 2 (Skopos, Iana) |
| Gadgets | 156 picks, each with its count | 43, without a count | |
| Primaries | 130 | 38, 9 of them without an id | |
| Secondaries | 121 | 27 | |

For 47 operators every listed gadget was seen, for 43 every primary and for 56 every secondary.

What to know about the values:

- **Armor is not recorded.** `MaxHealth` is what the game has in its place, and every operator seen has one value. Year 11 differs from older seasons here: Ace, Thatcher, Osa, Blackbeard, Aruni, Melusi and Warden are at 125.
- **A count is what the HUD counts**, which is not always a number of devices: 36 for Buck's Skeleton Key (shells), 320 for Maverick's torch (fuel), 25 for Sledge's hammer, 18 for Hibana's pellets, and 1 for an ability that is on or off. A refilling ability starts below what it holds (Lesion 1, Azami 1), so `regenerates` is the flag to check before comparing counts. Dokkaebi starts at 1 of a `max` of 5.
- **Skopos's shells show no count** in the HUD, so the ability has an id and no count.
- **Blackbeard's rifle is his secondary.** The shield sits in the primary slot, so the MK17 CQB and SR-25 are under `secondaries`, as `loadouts[]` has them.
- **Zero's 5.7 USG has an id of its own** (`5.7 USG (Zero)`), not the one other operators' 5.7 USG has.
- **A player's start can be lower than the catalog's.** Apart from Dokkaebi, 15 of the 1,837 loadouts start an ability or gadget below `max` (a Jäger at 0 of 3), so the tests check that nobody starts above it.
- **The operator pages are not complete.** Three picks seen in replays are not on the pages as read: Fuze's Hard Breach Charge (18 loadouts), and the XK23 on Sens (3) and Rauora (6). The catalog keeps them as observed.
- **An unknown season gets the latest catalog**, flagged `fallback`. Loadouts change every season, so treat a fallback as a guess.

Checked by `tests/catalog_operators.rs`: each of the 100 loadouts in the test rounds, and with `R6_MATCH_REPLAY` set each of the 1,737 in the real folder, has a primary, secondary and gadget the catalog lists for that operator, the catalog's ability, no count above the catalog's, and the catalog's side. Maximum health agrees except for 31 attackers at 125 that `players[].maxHealth` gives as 100 (3 in the test rounds; a rise of 25 while the attacker picks is read as a Rook plate and subtracted), and one round where an Ash reads 110 and a Hibana 125. In the real folder a pick the catalog does not list is printed, not failed: the folder grows as matches are played.
