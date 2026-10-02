## Weapon catalog

`replay_analyzer::catalog::weapons` is a table of the guns of Y11S3, keyed by the item id `loadouts` and the kill feed use: class, damage, fire rate, magazine, fire modes and attachments, with time-to-kill helpers. It is a library module: nothing of it is in a round's JSON.

A replay holds no weapon statistics. It holds what each gun did, and the catalog is built on that. Every number says where it comes from:

| `source` | Meaning |
|---|---|
| `observed` | Measured in replays; `samples` says on how much. `reference` is filled as well when a reference has the number, equal or not. |
| `reference` | From the reference table. Where replays measured it too (`observed`), they agree within the measurement's precision. |
| `unknown` | Neither has it. No number is guessed. |

Observed means 188 rounds: the 10 test rounds and a real folder of 178 (builds 9883691, 9901603 and 9918362), 3,595 guns carried, 68,766 shots. The reference is the weapon table of r6data.com as fetched on 2026-10-02, with three changes of the Y11S3 patch notes over it (SMG-12 16 damage and 22 rounds, AR-15.50 59 damage, SPSMG9 35 damage); it is a third-party table, not Ubisoft's.

| Field | What it is | Observed | Reference only | Unknown |
|---|---|---:|---:|---:|
| `magazine` | Rounds in a full magazine. | 108 | 6 | 1 |
| `chambered` | A round stays in the chamber on a reload. | 106 | | 9 |
| `rpm` (62 automatic guns) | Rounds a minute. | 51 measured | 11 | |
| `damage` | Health a bullet takes off a torso. | 54 | 58 | 3 |
| `class` | Assault rifle, shotgun and so on. | | 111 | 4 |
| `fireModes` | Follows from the class. | 1 | 110 | 4 |
| `attachments` | Per slot: ids seen, names listed. | 108 | 6 | 1 |

The table has 115 entries: the 111 primaries and secondaries of the item table (one of them the Ballistic Shield, which has a class and nothing else), and 4 guns only the reference knows (G36C, SASG-12, Super 90, SIX12 SD), which have no `id` because no replay has shown one.

How each is measured, by `harvest(&[Round])`, which returns the observed side for any set of rounds:

- **Magazine.** `loadouts[].primary.ammo.magazineSize`, read. Every gun had one size in all its loadouts. `chambered` is read too: the most rounds a player had in the gun is one above the magazine for 85 guns and equal to it for 21 (revolvers, most shotguns, the Gonne-6, and the DP27, 6P41, M249 and ALDA 5.56). `capacity()` adds the two.
- **Fire rate.** The median rate of a gun's steady bursts: 8 shots or more of one player, each at most 0.25 s after the last, the longest and shortest gap no more than one update apart. A shot's time is an update of the movement stream, 34 ms apart (the stream runs at about 28 Hz), so one burst's rate is good to 34 ms over its length: 2.4% for 20 rounds at 800 a minute, 8% for the 16 rounds of an SMG-11. The median over a gun's bursts (2,021 in all) is much closer: for the 48 automatic guns where reference and measurement agree, the two differ by 0.4% at the median and 2.5% at worst (FMG-9, 3 bursts). The catalog then gives the reference's figure, which is exact, and keeps the measured one beside it.
- **Only automatic guns have a fire rate.** A steady burst of a semi-automatic gun is the player's finger: the 417, Mk 14 EBR, PMR90A2, TCSG12, D-50, P9 and P12 gave medians of 407 to 476 a minute, with single bursts up to 502. That is a lower bound on what the gun allows and no more, and the reference has no figure either, so `rpm` is absent for them.
- **Damage.** The damage most hits did, of `bulletHits` that were not on a limb, left the victim standing and came from a barrel named something other than `Extended Barrel`. It counts when at least 3 hits and half of the gun's hits share it: 54 guns, 1,158 of their 1,486 hits. Hits that down or kill carry no damage in the file and head and torso are not told apart, so a headshot is never a sample and headshot damage is not observed. Pellet shotguns are left to the reference: of a shell's pellets in one body only the first carries the damage, and it carries their sum.
- **Attachments.** `seen` lists the ids players had in each slot (874 over all guns), with the names of the attachment table: 599 have one, 336 of them inferred (see [Loadouts](#loadouts)). `reference` lists the option names the reference gives the gun. A sight's name there (`Red Dot A`, `Holo B`) cannot be matched to an id. Underbarrels (laser or none) are observed only.

What replays say about the rules, which the helpers apply:

- **Armor takes nothing off a bullet.** A gun did its usual damage in 252 of 331 hits on targets of 100 health, 668 of 835 on 110 and 176 of 238 on 125 (76%, 80%, 74%), and in 87 of 89 pairs of a gun and a target health with 5 hits or more the usual damage is the same. Armor is the health itself. `armor_multiplier` is 1.0.
- **A limb takes three quarters, rounded down.** For 39 of 41 guns with 3 or more limb hits the most common limb damage is `floor(0.75 x damage)` (673 limb hits).
- **An extended barrel adds 12%, rounded down**, for 7 of 9 guns seen with one (MP7 32 to 35, MPX 26 to 29, MP5 27 to 30, C8-SFW 40 to 44). Two do not fit: the P90 did 29 (4 hits, base 22) and the 9mm C1 46 (2 hits, base 36). The barrel's name is inferred, so either those two ids are something else or the rule is not flat; `hit_damage` applies 12%.
- **A headshot kills.** From the reference: the file marks a headshot on kills only, so this cannot be checked on hits.

Where replays and the reference differ, both are kept and the catalog goes by what was observed:

| Gun | Field | Observed | Reference |
|---|---|---:|---:|
| Scorpion EVO 3 A1 | `rpm` | 1076 (99 bursts) | 1800 |
| MP5K | `rpm` | 797 (54 bursts) | 900 |
| MK17 CQB | `magazine` | 20 (16 loadouts) | 25 |
| UZK50GI | `damage` | 36 (21 hits) | 40 |
| AUG A3 | `damage` | 36 (8 hits) | 40 |
| Commando 9 | `damage` | 36 (7 hits) | 40 |
| Mk 14 EBR | `damage` | 56 (5 hits) | 60 |

The two fire rates look like slips of the reference (1080 and 800 are the long-standing figures). For the other five the reference is likely a patch behind; a second site gives the UZK50GI 36.

Time to kill:

| Function | What it gives |
|---|---|
| `hit_damage(base, extended_barrel, location, health)` | The health one bullet takes on the torso, a limb or the head. |
| `shots_to_kill(damage, health)` | Hits to bring `health` (100, 110 or 125) to zero. |
| `time_to_kill(shots, rpm)` | Seconds from the first shot to the last: `(shots - 1) x 60 / rpm`. |
| `expected_shots_to_kill`, `expected_time_to_kill` | The same on average for a mix of torso, limb and head hits (`HitMix`), a headshot ending it. |
| `WeaponInfo::time_to_kill`, `times_to_kill` | Shots and seconds for a gun of the catalog, with or without an extended barrel. |

They count hits, not shots fired: misses, travel time, recoil and range are not in them. "Kill" is health reaching zero, which the game may turn into a down.

What is not known, or not in the catalog:

- **Four guns no reference lists**: XK23, PMR90A2, TACIT .45 and Zero's 5.7 USG (`unknown()`). They have no class. The XK23 has what replays show: 35 rounds, 49 damage (18 hits), 676 rounds a minute over 64 bursts, which is taken as automatic fire. The PMR90A2 has 20 rounds and 62 damage (16 hits). The TACIT .45 and Zero's 5.7 USG have a magazine only.
- **Damage fall-off with range.** No figures were confirmed, so the catalog has none. Hits beyond about 20 m do read lower (an MP7's 32 becomes 22 or 23), which is why damage is the most common value and not the highest.
- **Hits that do less at any range.** About 5% of torso hits did half the gun's damage and another 5% seven tenths, at 5 to 28 m. Nothing in the hit says why; a bullet that went through a surface or another player first is the likely cause, unconfirmed. A few did double: two bullets between two updates of the body.
- **Fire modes are not in the file** (see [Weapons and shooting](#weapons-and-shooting)). `fireModes` gives the one the class implies, and not the other settings of a selector (burst, single).
- **Semi-automatic and pump fire rates**, the reload time, recoil, and what each attachment does apart from the extended barrel's damage.
- **Two guns of the item table were in no replay** (LMG-E and M249 SAW), so they are reference only, as are the 4 without an id.
- **The reference is unofficial and undated.** Where it was neither measured nor contradicted (58 damages, 11 fire rates, 6 magazines) it is taken as given.

`tests/catalog_weapons.rs` holds every gun of the test rounds against the catalog (56 guns: 56 magazines equal, 14 fire rates within 3%, 8 damages not above) and does the same for a real folder when `R6_MATCH_REPLAY` is set (107 guns: 107 magazines, 51 fire rates, 53 damages).
