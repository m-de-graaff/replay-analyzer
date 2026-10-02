## Sessions, journal and insights

`replay_analyzer::journal` is the app's own data layer. It keeps three kinds of data apart.

| Kind | Types | Where it lives |
|---|---|---|
| Derived from replays | `MatchRecord`, `PlaySession`, `Break`, `GoalProgress`, `Insights`, `TiltSignal` | Nowhere as truth. The same replays and rules give the same values again. |
| Owned by the player | `Journal`: `tags`, `notes`, `goals` | One JSON file. The caller chooses the path; the crate never does. |
| Rules | `SessionRules`, `InsightRules`, `TiltRules` | The caller's settings. Every threshold has a default and can be changed. |

Everything serializes and deserializes as camelCase JSON. Timestamps are UTC, written as `2026-01-31T20:15:00.000Z`. Nothing in the module reads the clock: functions that stamp a time take `now`.

The journal refers to matches by `matchID` and holds the player's words. It copies nothing out of a replay.

### Match records

A `MatchRecord` is what sessions, goals and insights need of one match: `matchID`, `started`, `ended`, `utcOffsetMinutes`, `queue`, `map`, `outcome`, `score` (yours first), the game `launch` that recorded it, the followed `player`'s match line and one line per round.

| Call | Reads | Gives |
|---|---|---|
| `journal::read_folder(root, ReadMode::Header)` | headers only, under a second for 30 folders | times, queue, map, result, rounds won; no player lines |
| `journal::read_folder(root, ReadMode::Full)` | every round in full, tens of seconds for 30 folders | the same plus the recording player's lines |
| `journal::records_from_library(&library)` | nothing: reuses a `library::scan` | the header-only records |
| `MatchRecord::from_match(&m, stats)` | one opened `Match` | one record |
| `MatchRecord::from_match_as(&m, username)` | one opened `Match` | the record from that player's side: for spectator recordings and a teammate's files |

The game keeps a fixed number of matches and deletes the oldest. A `MatchRecord` is small and reads back from JSON, so an app that wants history stores the records and derives sessions and insights from them. Two records of the same match count once; the fuller one is used.

What a full read gives for the followed player, per match and per round: rounds, kills, deaths, assists, headshots, damage taken, damage dealt (Y11S3+, an estimate, see [Health and damage](#health-and-damage)), whether they made the first kill of a round and whether they were the first to die, the operator and side per round, and whether their team won each round. Replays hold no rank, rank points or ping, so no insight can hold lobby strength equal.

`started` is when the first round read began recording, after the queue, the map load and the ban and pick phases. `ended` is when the last round read stopped. Before Y11S3 a replay has only the recording PC's local time and no end; such a record has `startedIsLocal` set and counts as over when it began.

### Play sessions

`journal::sessions(&records, &SessionRules)` cuts matches into sittings, oldest first.

```json
{
  "id": "session:<matchID of the first match>",
  "started": "...", "ended": "...", "utcOffsetMinutes": 120,
  "matches": [{ "matchID": "...", "position": 2, "started": "...", "ended": "...",
                "gapSeconds": 215, "afterBreak": false, "queue": "ranked", "map": "...",
                "outcome": "win", "score": [4, 2], "player": { "kills": 7, "deaths": 4, "...": 0 } }],
  "breaks": [{ "after": "<matchID>", "before": "<matchID>", "from": "...", "to": "...", "seconds": 857 }],
  "launches": [{ "processId": 1234, "matches": 4 }],
  "record": { "wins": 3, "losses": 1, "draws": 0, "undecided": 0 },
  "totals": { "matches": 4, "kills": 25, "...": 0 }
}
```

The gap measured is from the end of one match's last round to the start of the next match's first round. It always holds the end screens, the queue, the map load and the ban and pick phases, so it is never zero in real play.

| Rule | Default | Why |
|---|---|---|
| `breakSeconds` | 600 | A gap this long inside a session is a break; shorter is queue time. |
| `sessionGapSeconds` | 3600 | A gap this long starts a new session. |
| `splitOnLaunch` | false | A game restart alone does not start a new session. |

The evidence is one real folder: 30 matches over 8 evenings, 23 ranked, 4 unranked and 3 quick matches.

- The 22 gaps between matches of one evening ran from 2.3 to 14.3 minutes, with a median of 3.6. Twenty were 6.0 minutes or less; the other two were 7.8 and 14.3.
- The 7 gaps between evenings were all longer than 21 hours. Nothing fell between 15 minutes and 21 hours.
- Every change of game process in the folder came with one of those 21-hour gaps. Within an evening the process never changed.

So ten minutes sits well clear of a normal requeue and catches the one long pause in the data. Under the defaults the folder cuts into 8 sessions with 1 break.

The data cannot place the session threshold: any value between 15 minutes and 21 hours cuts this folder the same way. One hour is a convention. A pause for a meal is a break; an hour away is a new sitting.

The data also cannot say what a restart means, since no restart happened mid-evening. The default keeps the session, because a crash or an update restart costs minutes and the player has not left. Restarts are listed in `launches` either way: a launch changes when the process id changes, or when the same process id starts its recording counter over.

What always holds, and what `tests/journal.rs` checks on any folder:

- Every match is in exactly one session.
- Sessions are in start order, and so are the matches inside one.
- A break never overlaps a match. It runs from the latest end of the matches before it to the start of the next one.
- Matches that overlap, such as two recordings with clocks apart, get a gap of 0 and stay in one session.
- An unfinished or cancelled match is part of its session and counts as `undecided`.

The session id comes from its first match, so it changes once the game has deleted that match and the records were not kept. `journal::find_session(&sessions, id)` finds a session by its id or by any match in it, which resolves a stored id for as long as the match it was made from is still in the session.

### The journal

```json
{
  "version": 1,
  "tags":  [{ "id": "...", "target": { "kind": "round", "matchID": "...", "round": 3 },
              "label": "clutch", "created": "...", "edited": "..." }],
  "notes": [{ "id": "...", "target": { "kind": "session", "sessionID": "session:..." },
              "text": "...", "created": "...", "edited": "..." }],
  "goals": [{ "id": "...", "text": "...", "created": "...", "due": "...", "status": "active",
              "edited": "...",
              "metric": { "stat": "killDeathRatio", "comparison": "atLeast", "target": 1.0,
                          "scope": { "kind": "rolling", "matches": 10 } } }]
}
```

A `target` is one of:

| `kind` | Fields |
|---|---|
| `match` | `matchID` |
| `round` | `matchID`, `round` (from 1) |
| `session` | `sessionID` |
| `kill` | `matchID`, `round`, `index` (place among the round's kills and deaths, from 0), `time` (round clock, seconds) |

A tag's id is made from its target and its label without case, so tagging twice gives one tag, and the same tag set on two devices is one tag after a merge.

A goal's `metric` is optional. `stat` is one of `kills`, `deaths`, `assists`, `headshots`, `killDeathRatio`, `killsPerRound`, `deathsPerRound`, `survivalRate`, `headshotPercentage`, `damagePerRound`, `winRate`, `roundWinRate`, `openingKillRate`, `openingDeathRate`. `scope` is `perMatch`, `perSession` or `rolling` over the last N matches. `status` (`active`, `achieved`, `abandoned`) is the player's to set.

`journal::goal_progress(&goal, &records, &SessionRules)` measures a goal against the matches started from its creation up to its due time. It gives one point per match, session or full window, how many met the target, and the current value. A rolling goal short of its window reports a value and says nothing about whether it is met.

**Saving.** `Journal::save(path)` writes a temporary file next to the target, flushes it to disk and renames it over the target. A crash leaves the old file or the new one. `Journal::load(path)` gives an empty journal when there is no file, and an error when the file is not a journal.

**Newer files.** Fields this version does not know, at the top level and in every item, are kept and written back. An item this version cannot read at all, such as a tag on a new kind of target, is kept in `Journal::unread` and written back. A newer file's `version` is left as it is. An older file goes through `migrate`, which has nothing to do yet: version 1 is the first.

**Merging.** `a.merge(&b)` takes in another copy of the journal. Per item the later change wins. Deleting an item leaves a tombstone: the item keeps its id and gets `deleted`, and its text is emptied. A copy that never saw the deletion therefore cannot bring the item back. Merging gives the same result either way round and can be repeated. `purge_deleted(before)` drops old tombstones; that is only safe once every copy has merged since.

Last writer wins by the time each device stamped. Two devices with clocks apart can let the older edit win.

### Insights

`journal::insights(&records, &SessionRules, &InsightRules)` compares the followed player's matches by where they sit in a session and what came before them.

These are descriptive statistics of one player's own matches. They are correlations on small samples, not causes. A worse result after a loss can as well come from a stronger lobby, a later hour or chance, and none of those is held equal. The text of this warning is in every result as `caveat`.

| Field | Groups |
|---|---|
| `overall` | every match |
| `byPosition` | first, second, ... match of the session, then `5+` |
| `afterLossVsAfterWin` | matches right after a loss in the same session, against right after a win |
| `onLossStreak` | matches after 2 or more straight losses in the session, against the session's other later matches |
| `afterBreakVsWithout` | matches after a break, against after a normal requeue |
| `bySessionLength` | matches of sessions of 1-2, 3-4 and 5+ matches |
| `byTimeOfDay` | `night` (0-6), `morning`, `afternoon`, `evening`, by local start time from the replay's UTC offset |
| `afterDyingFirst` | rounds right after one in which the player was the first to die, against the other rounds with a round before them |
| `afterRoundLosses` | rounds after 2 or more straight lost rounds of the match, against the other rounds with a round before them |

Every group carries `sample`, `enough` and the raw counts (`totals`), so the numbers behind a rate are always there. Rates are win rate with its 95% Wilson interval, K/D, kills and deaths per round, headshot percentage and round win rate.

Below the minimum sample a group holds counts and no rate, and a comparison holds no `effect`. The minimum is `minMatches` (default 10) for groups of matches and `minRounds` (default 30) for groups of rounds. Ten is low: at ten matches a win rate still has a standard error of about 16 points, which the interval shows.

An `effect` is the first group minus the second: the difference in win rate with Cohen's h, the difference in K/D, the difference in kills per round with Cohen's d over the matches, and the difference in round win rate. As a rule of thumb 0.2 is small, 0.5 medium and 0.8 large.

On the real folder, with all 30 matches read in full, the defaults give no effect at all. The groups were 5 matches after a loss against 14 after a win, 1 on a loss streak, 1 after a break, 15 rounds after dying first and 18 after two lost rounds. The 30 matches the game keeps are too few for these questions, which is the reason to keep records.

What the replays can and cannot answer:

- From a header-only read: position in session, after a loss or a win, loss streaks, breaks, session length, time of day and round losses, on win rates only.
- From a full read: the same with K/D, kills per round and headshots, and `afterDyingFirst`.
- Not from replays: anything about lobby strength, rank or ping; time spent in the queue as opposed to the menu; what the player did during a break. Before Y11S3 there is no UTC offset and no match end, so time of day uses the local time as written and gaps are measured from the start of the match before.

### Tilt

`journal::tilt(&session, &SessionRules, &TiltRules)` reads a session in progress. It counts the straight losses at the end of the session, back to the last break, and compares the matches of that streak with the ones before it.

| Level | When |
|---|---|
| `none` | fewer than `watchLosses` (2) straight losses |
| `watch` | 2 straight losses |
| `tilted` | 3 straight losses (`tiltedLosses`), or 2 with the K/D in the streak at least 20% (`kdDrop`) below the K/D before it |

`suggestBreak` is set when the level is `tilted`, or after `maxSecondsWithoutBreak` (7200) of play without a break whatever the results. `suggestedBreakSeconds` is the session rules' `breakSeconds`: the shortest pause after which the next match counts as coming after a break, which also resets the streak count. The signal carries its `reasons`, the totals before and during the streak, and the session's match count as `sample`.

This is a rule of thumb on a handful of matches. It says the last matches went worse than the ones before; it does not say why.

### Dependencies

None added. `chrono` is built without its `serde` and `clock` features, so the module writes timestamps itself and takes `now` from the caller. Item ids are the first 16 hex digits of a SHA-256 (`sha2`, already present) of what the item is.
