## Profiles from outside the replay

Rank, rank points and level come from Ubisoft's services, not from the replay. `replay_analyzer::profiles` joins stats fetched elsewhere onto the players a replay names and derives lobby strength and rank progress from them. The crate makes no network call and holds no key or URL: the app fetches through its own proxy and hands the answers in.

### What is joined on

| From the replay | Use |
| --- | --- |
| `players[].profileID` | The join key: a Ubisoft profile id, the same in every match. Players without one (older replays) cannot be joined. |
| `players[].platform` (Y11S3+) | Which of the account's ranked profiles applies: Ubisoft keeps one for PC and one for the consoles. PC is assumed when the replay does not say. |
| `players[].username`, `renamedTo`, `usesNickname` | The name to look up, since the provider looks up by name. `renamedTo` is used when the recording has it. A player behind a nickname in a recording that stops before the match ends has no name to look up (`nameIsNickname`). |
| `players[].level` (Y11S3+) | The clearance level, used as the fallback measure and preferred over the provider's. |
| `queue`, `matchID`, `endTime`, `result.winner` | Which matches are ranked, when they ended and who won, for rank progress. |

Replays hold no rank, rank points or reputation. Checked again for this work: a census of a Y11S3 test round lists 34 header keys, none unknown and none about rank, and 26 named stream fields, none about rank; the 230 unnamed fields are hashes the census cannot name, which the earlier search (see "Limits worth knowing" in the README) went through by value.

### What the provider gives

The R6 Data API (`r6data.com/api-docs` redirects to `r6.arenyze.com/api-docs`, where it is documented as the R6 Arenyze API; `r6data.eu` did not resolve), as its documentation stood on 2026-10-02:

- `GET https://public-api.arenyze.com/r6/api/v2/profile?nameOnPlatform=<name>&platformType=<uplay|psn|xbl>&platform_families=<pc|console>` with the header `api-key: <key>`.
- The answer holds `player` (`nameOnPlatform`, `platformType`), `account` (`level`, `xp`, `profilePicture`, `profiles`), `stats.platform_families_full_profiles`, `banned`, `seasons`, `history` and `meta`.
- Each entry of `platform_families_full_profiles` has `profile_id` and `board_ids_full_profiles[]`, each with a `board_id` (`ranked`, ...) and `full_profiles[]` holding `season_id`, `profile` (`rank`, `rank_points`, `max_rank`, `max_rank_points`, ...) and `season_statistics` (`kills`, `deaths`, `match_outcomes.{wins, losses, abandons}`). This is Ubisoft's own ranked shape passed through.
- The older `GET /api/stats?type=stats|accountInfo|seasonalStats|...` routes answer `410 Gone`.
- `GET /r6/api/me/usage` reports the plan and its call limit (the example shows `"plan": "pro", "limit": 100000`).

Not in the documentation, and so not relied on:

- **Lookup by profile id.** Every player route takes a name and a platform. The answer carries `profile_id`, so the adapter checks it against the id the replay gave and refuses an answer for another account (`ProfileError::Mismatch`): names change hands. Whether a name can be replaced by an id is an open question for the proxy.
- **Rate limits.** No per-second or per-minute limit and no `429` behaviour is documented, only a call limit per plan. One call fetches one player, so a lobby costs up to ten calls.
- **Rank ids.** The documented example pairs rank 18 with 3300 rank points, which its own history example names Platinum II (id 24 by Ubisoft's count from Copper V = 1). The adapter therefore names a ranked player's rank from the rank points and keeps the provider's ids apart in `providerRank` and `providerMaxRank`.
- **`top_rank_position`, `platform_family` and the placement of `season_id`.** Read where Ubisoft puts them and where the provider's example puts them.

`from_r6data_json` and `from_r6data_value` are the only functions that know these names. Everything is optional, numbers may be strings, and a response with no ranked profile still yields the level.

### What is stored

`ProfileStats` is the crate's own shape, in camelCase like the rest of the output:

```json
{
  "profileID": "…", "idConfirmed": true, "platform": "pc", "family": "pc",
  "username": "…", "fetchedAt": "2026-10-02T18:00:00Z", "season": 43, "level": 212,
  "ranked": {
    "season": 43, "rank": {"id": 19, "name": "Gold II"}, "rankPoints": 2850,
    "maxRank": {"id": 20, "name": "Gold I"}, "maxRankPoints": 2940,
    "providerRank": 19, "providerMaxRank": 20,
    "wins": 30, "losses": 20, "abandons": 1, "kills": 300, "deaths": 200
  },
  "boards": {"standard": {"…": "…"}}
}
```

`ProfileCache` keeps the latest stats per profile id and family in a JSON file the caller names, with every snapshot whose ranked board differs from the one before (the history rank progress needs) and the players the provider did not know. `save` writes a temporary file beside the target and renames it over.

### What the app does

```rust
let mut cache = ProfileCache::load(&path)?;
let now = profiles::now();
let due = profiles::requests_for(&summary, &cache, now, Duration::hours(6));
cache.refresh(&source, &due, now)?;      // source: the app's ProfileSource
cache.save(&path)?;
let lobby = profiles::lobby_strength(&summary, &cache.profiles());
```

`requests_for` lists the players with nothing cached younger than the max age, the recording player first, then teammates, then opponents, so an app short on calls can stop early. The app's `ProfileSource::fetch` makes one proxy request per key, using `key.name`, `key.platform_type()` and `key.platform_families()`, and passes each answer with the time it was fetched to `from_r6data_json`. The proxy adds the `api-key` header; the crate never sees it.

### What is modelled

**The rank table.** Rank points to rank: Copper V starts at 1000, each division is 100 points, five divisions a tier from Copper to Diamond.

| System | Seasons | Ranks | Champion | Confidence |
| --- | --- | --- | --- | --- |
| Ranked 2.0 | Y7S4 to Y11S1 (season ids below 42) | 36 | one rank, from 4500 | Divisions and 100 points each: reported widely. The base of 1000: from the provider's example (3300 is Platinum II) and the figure in common use, not from a Ubisoft page. |
| Ranked 3.0 | Y11S2 on (season ids 42 and up) | 40 | Champion V to I, assumed at 4500, 4600, 4700, 4800 and 4900 | Champion's five divisions, five placement matches and the end of hidden MMR: Ubisoft's Ranked 3.0 article. The thresholds: assumed to carry on from 2.0; not published where looked. Season id 42 for Y11S2 assumes ids count from Y1S1 = 1. |

Rank id 0 is unranked, including a player still in placement matches (assumed); unranked players are left out of every rank point figure.

**Lobby strength.** Per team and for the lobby: mean, median, minimum, maximum and standard deviation of the ranked rank points of the players who hold a rank, with the ranks the mean and median fall in; how many of the players had a profile, a rank, a level. `difference` is the mean of the recording player's team minus the other's (team 0 minus team 1 for a spectator). `percentile` is the share of the other players below a player, ties counting half.

- `basis` says what the difference and percentiles are measured in: `rankPoints` when each team has at least one ranked player known; else `level`, a fallback on clearance levels, which count time played and not skill; else `none`.
- `expectedWin` is `1 / (1 + 10^(-difference / 400))`: the Elo curve with its usual scale applied to the mean rank point difference. It is a model, not the game's figure, and has not been fitted to match results. It is absent on the level fallback.
- Stats are as of `fetchedAt`, not as of the match. `maxFetchGapSeconds` is the longest gap between the match and a profile used. A lobby looked up weeks later is measured by where its players are now.
- A team lists everyone seen on it, so a team with a leaver and a joiner has six players and all six count.

**Rank progress.** `rank_progress(history, matches)` takes one player's snapshots and the match summaries and gives, per pair of consecutive snapshots, the rank point change, the ranked matches the provider counted between them (the growth of wins, losses and abandons) and the ranked matches of the replays that ended between them.

- `match`: the provider counted one match and the replays hold exactly one. The change is that match's.
- `matchUnconfirmed`: the replays hold exactly one, and the provider gave no counts, so an unrecorded match may share the change.
- `span`: anything else. No split between the matches of a span is made up.

Per-match rank point changes therefore need a fetch before and after every ranked match; the app decides whether that is worth the calls. Across a season change no change is given, since the reset is not a result. `seasonPeak` is the provider's `max_rank_points` when present, else the highest snapshot of the season, marked `snapshots`: a lower bound. `trend` sums the change over the latest five spans of the season. `nextDivision` and `nextTier` come from the rank table and share its confidence.

### Suggested CLI

Not built: `src/main.rs` is unchanged.

- `replay-analyzer <match folder> --profiles <cache.json>` adds `lobbyStrength` to the match summary from the cache, and `profile` to each player. Without a fresh cache entry a player simply has none.
- `replay-analyzer <match folder> --profiles <cache.json> --profile-requests [--max-age 6h]` prints the `ProfileKey`s to fetch as JSON and nothing else, for a caller that fetches and writes the cache itself.
- `replay-analyzer <MatchReplay> --players --profiles <cache.json>` adds `rankProgress` for the recording player from the cache's history.

The CLI would never fetch: fetching needs the proxy.
