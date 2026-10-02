//! What the teams learn of each other (Y11S3): who found the objective,
//! when each player's operator became known to the other team, and when
//! Dokkaebi hacked a phone.
//!
//! Both are HUD state in the `state` stream, read the way
//! [`crate::loadout`] reads slots.
//!
//! The game keeps no flag for the objective being found. Two things show
//! it:
//!
//! - The feed, in a player's recording only. A feed entry (class
//!   `2b9d6947`) gets a `Message` (`e3090079`) whose array element 0 is the
//!   text shown, `<username> has found the bombs`, and names no killer
//!   (`d9133cba`) and no victim (`ac190f70`). The text is the game's and
//!   may be translated, so an entry is told by its shape and by the
//!   attacker its text names, not by its words. The entry is written again
//!   as the feed scrolls; the first is the find.
//! - The score, in every recording. The finder's `MatchScore` (`ecda4f80`,
//!   on the scoreboard object) rises by 50 in the frame of the find. When
//!   prep ends with the objective not found, every defender's rises by 100.
//!
//! The feed entry is used when there is one. Else the find is the first
//! attacker's +50 in prep, which named the player of the feed in 90 of 90
//! real rounds that have both. Else, when the defenders got their +100, it
//! is the first +50 of an attacker after prep that comes without an assist
//! (`MatchAssists`, `4d737f9e`, also worth 50): other awards are 50 as
//! well, and this named the player of the feed in 49 of 57 rounds, so that
//! find says `inferred`. A round whose defenders got the +100 and where no
//! find followed has `found: false`; a round that shows neither has no
//! objective at all, which is so in the bomb mode of Quick Match and in a
//! round that ends in prep.
//!
//! A controller's `HasBeenDiscovered` (`41f2118a`, one byte) turns 1 when
//! the other team learns which operator the player is. It does so about
//! 0.27 s after a kill by the player, and that reveal names the victim.
//! Any other is `identified`: the player was seen by a drone, a camera or
//! an opponent. Who saw them is not in the data, only that everyone on the
//! other team scores 10 in that moment (`teamBonus`), which holds for 571
//! of 601 such reveals, and for some by a kill too. Dying reveals nobody,
//! and the file hides nothing on its own: every operator is in the
//! controllers from the start. [`crate::joins`] adds what else the stream
//! wrote in that moment as the `cause`.
//!
//! Dokkaebi's ability object links a tablet through its field `Tablet`
//! (`b4928f1d`), and the tablet has an `EquipState` (`e5e20d29`): 0, 1, 2
//! in the two seconds before a call, and 3 while she hacks the phone of a
//! dead defender. A hack takes 2.52 to 2.57 s; a shorter stretch of 3 was
//! cut off. In 31 real rounds with her, all 24 stretches start after a
//! defender died and 22 run their full length. Whose phone it was is not
//! in the file. The scores read here also tell [`crate::joins`] of the
//! points a spotter gets.

use std::collections::HashMap;

use serde::Serialize;

use crate::details::Phase;
use crate::entities::{Hash, Record, for_each_record, u32_at};
use crate::feedback::{MatchUpdate, MatchUpdateType};
use crate::header::Player;
use crate::loadout::{Clock, Input, STATE_STREAM, When};
use crate::types::TeamRole;

/// `HasBeenDiscovered`, on the controller.
const DISCOVERED: Hash = [0x41, 0xF2, 0x11, 0x8A];
/// `MatchScore` and `MatchAssists`, on the scoreboard object.
const MATCH_SCORE: Hash = [0xEC, 0xDA, 0x4F, 0x80];
const MATCH_ASSISTS: Hash = [0x4D, 0x73, 0x7F, 0x9E];
/// Class of an entry of the feed.
const FEED_ENTRY: Hash = [0x2B, 0x9D, 0x69, 0x47];
/// `Message` of a feed entry; its array element 0 is the text.
const FEED_MESSAGE: Hash = [0xE3, 0x09, 0x00, 0x79];
const FEED_KILLER: Hash = [0xD9, 0x13, 0x3C, 0xBA];
const FEED_VICTIM: Hash = [0xAC, 0x19, 0x0F, 0x70];
/// Field of Dokkaebi's ability object that links her tablet, and the
/// tablet's `EquipState`.
const TABLET: Hash = [0xB4, 0x92, 0x8F, 0x1D];
const EQUIP_STATE: Hash = [0xE5, 0xE2, 0x0D, 0x29];
/// The `EquipState` of a tablet that hacks a phone.
const HACKING: u32 = 3;
/// A hack that ran this long was completed (seconds).
const HACK_SECONDS: f64 = 2.4;
/// Most links between an object and the controller it hangs off.
const MAX_DEPTH: usize = 32;

/// Points for finding the objective, and for an assist.
const FIND: i64 = 50;
/// Points each defender gets when prep ends with the objective not found.
const NOT_FOUND: i64 = 100;
/// The defenders get them within this of the end of prep (seconds).
const PREP_END: f64 = 1.5;
/// A find up to this long after prep ended still counts as one in prep.
const PREP_SLACK: f64 = 0.5;
/// A +50 this close to an assist is the assist's.
const ASSIST: f64 = 0.3;
/// The feed and the score write one find within this of each other.
const SAME_FIND: f64 = 0.5;
/// A killer is flagged at most this long after the feed shows the kill
/// (0.30 s at most in 541 reveals), or this long before it.
const KILL_LAG: f64 = 0.35;
const KILL_LEAD: f64 = 0.2;
/// The other team scores from this long before the flag to this long after.
const BONUS_BEFORE: f64 = 0.05;
const BONUS_AFTER: f64 = 0.4;
/// What a reveal is worth to each opponent: 10, or more when the same
/// frame reveals two or three.
const BONUS: [i64; 3] = [10, 20, 30];

/// How a find was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// The feed named the finder.
    Feed,
    /// The finder's score rose by 50.
    Score,
}

/// Whether and by whom the objective was found.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Objective {
    /// False when prep ended with the objective not found and no find
    /// followed.
    pub found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// The find is the first +50 of an attacker in the action phase, which
    /// other awards can be too.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub inferred: bool,
    /// Found before the action phase started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_prep: Option<bool>,
    #[serde(flatten)]
    pub when: Option<When>,
    /// Who the score names as the finder in prep, whatever `source` is: a
    /// check on the feed.
    #[serde(skip)]
    pub prep_score: Option<String>,
    /// With a feed entry: the finder's score rose by 50 in that moment.
    #[serde(skip)]
    pub score_confirmed: bool,
}

/// What made a player's operator known.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Trigger {
    /// The player killed an opponent.
    Kill,
    /// The player was seen.
    #[default]
    Identified,
}

/// What else the marker stream wrote as a player was identified, by
/// [`crate::joins`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Cause {
    /// A spot mark on the player.
    Spot,
    /// A tracking marker of an ability on the player.
    AbilityMarker,
    /// A spot mark on a teammate.
    TeammateSpot,
    /// An opponent's ping on an object.
    Ping,
}

/// A player's operator becoming known to the other team.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reveal {
    pub username: String,
    pub trigger: Trigger,
    /// For `identified`: what the marker stream wrote in the same moment.
    /// Absent when it wrote nothing that fits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<Cause>,
    /// Whom the player killed, for a `kill`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub victim: Option<String>,
    /// Every player of the other team scored for the reveal.
    pub team_bonus: bool,
    #[serde(flatten)]
    pub when: When,
}

/// Dokkaebi hacking the phone of a dead defender. `when` is the start.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneHack {
    pub username: String,
    /// Absent when the recording ended first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    /// The hack ran its full length.
    pub completed: bool,
    #[serde(flatten)]
    pub when: When,
}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    pub objective: Option<Objective>,
    pub reveals: Vec<Reveal>,
    pub hacks: Vec<PhoneHack>,
    /// Per player, each change of their score.
    pub gains: Vec<Vec<Gain>>,
    /// Per player, when their assist count was written, in seconds since
    /// the recording started.
    pub assists: Vec<Vec<f64>>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
}

/// A value written for a player: the frame (`None` in the opening
/// snapshot) and the offset in the data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Write {
    value: u32,
    frame: Option<u32>,
    at: usize,
}

/// What one frame wrote to an object that may be a feed entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Line {
    text: Option<String>,
    killer: bool,
    victim: bool,
    /// Offset of the text in the data.
    at: usize,
}

/// The scores, flags and feed entries of the state stream.
#[derive(Debug, Default)]
struct Hud {
    controllers: HashMap<u32, usize>,
    scoreboards: HashMap<u32, usize>,
    /// Per player, every write in stream order.
    scores: Vec<Vec<Write>>,
    assists: Vec<Vec<Write>>,
    /// Per player, the first write of a 1.
    discovered: Vec<Option<Write>>,
    /// Whether the class an object was last linked with is the feed's.
    entries: HashMap<u32, bool>,
    /// The object each object was first linked from.
    parents: HashMap<u32, u32>,
    /// Objects linked through the field `Tablet`.
    tablets: Vec<u32>,
    /// Every `EquipState` written, in stream order.
    equips: Vec<(u32, Write)>,
    /// In the order first written.
    lines: Vec<(u32, Option<u32>, Line)>,
    line_of: HashMap<(u32, Option<u32>), usize>,
    /// Values whose size the property never has.
    malformed: usize,
}

impl Hud {
    fn new(players: &[Player]) -> Self {
        let n = players.len();
        let mut hud = Hud {
            scores: vec![Vec::new(); n],
            assists: vec![Vec::new(); n],
            discovered: vec![None; n],
            ..Hud::default()
        };
        for (i, p) in players.iter().enumerate() {
            let Some(e) = &p.entities else { continue };
            hud.controllers.insert(e.controller, i);
            if let Some(scoreboard) = e.scoreboard {
                hud.scoreboards.insert(scoreboard, i);
            }
        }
        hud
    }

    /// Reads the records of one snapshot or frame record. `base` is where
    /// `block` starts in the data.
    fn read(&mut self, block: &[u8], base: usize, frame: Option<u32>) {
        // Each record block names its object before writing to it.
        let mut current: Option<u32> = None;
        // A link ends in the class of its child.
        let class = |at: usize| -> Option<Hash> { block.get(at..at + 4)?.try_into().ok() };
        for_each_record(block, |at, r| match r {
            Record::Set(obj, hash, from, to) => {
                current = Some(obj);
                if let Some(value) = block.get(from..to) {
                    self.property(obj, hash, None, value, frame, base + at);
                }
            }
            Record::Prop(hash, from, to) => {
                // A `26` is an array element and says which.
                let element = match block.get(at) {
                    Some(0x26) => u32_at(block, at + 5),
                    _ => None,
                };
                if let (Some(obj), Some(value)) = (current, block.get(from..to)) {
                    self.property(obj, hash, element, value, frame, base + at);
                }
            }
            Record::ParentChild(parent, field, child) => {
                current = Some(parent);
                self.link(current, field, child, class(at + 21));
            }
            Record::Child(field, child) => self.link(current, field, child, class(at + 13)),
            Record::Element(field, _, child) => self.link(current, field, child, class(at + 17)),
        });
    }

    fn link(&mut self, parent: Option<u32>, field: Hash, child: u32, class: Option<Hash>) {
        if child == 0 {
            return;
        }
        self.entries.insert(child, class == Some(FEED_ENTRY));
        if let Some(parent) = parent {
            self.parents.entry(child).or_insert(parent);
        }
        if field == TABLET && !self.tablets.contains(&child) {
            self.tablets.push(child);
        }
    }

    /// The player whose controller `object` hangs off.
    fn owner(&self, mut object: u32) -> Option<usize> {
        for _ in 0..MAX_DEPTH {
            if let Some(&player) = self.controllers.get(&object) {
                return Some(player);
            }
            object = *self.parents.get(&object)?;
        }
        None
    }

    fn property(
        &mut self,
        obj: u32,
        hash: Hash,
        element: Option<u32>,
        value: &[u8],
        frame: Option<u32>,
        at: usize,
    ) {
        let number = || Some(u32::from_le_bytes(value.try_into().ok()?));
        match (hash, element) {
            (MATCH_SCORE | MATCH_ASSISTS, None) => {
                let Some(&p) = self.scoreboards.get(&obj) else {
                    return;
                };
                let series = if hash == MATCH_SCORE {
                    &mut self.scores
                } else {
                    &mut self.assists
                };
                match (number(), series.get_mut(p)) {
                    (Some(value), Some(s)) => s.push(Write { value, frame, at }),
                    _ => self.malformed += 1,
                }
            }
            (DISCOVERED, None) => {
                let Some(&p) = self.controllers.get(&obj) else {
                    return;
                };
                let first = self.discovered.get_mut(p).filter(|d| d.is_none());
                if let ([1], Some(d)) = (value, first) {
                    *d = Some(Write {
                        value: 1,
                        frame,
                        at,
                    });
                }
            }
            (EQUIP_STATE, None) => {
                // A number, or one byte of it.
                let state = match value {
                    &[v] => Some(u32::from(v)),
                    _ => number(),
                };
                match state {
                    Some(value) => self.equips.push((obj, Write { value, frame, at })),
                    None => self.malformed += 1,
                }
            }
            (FEED_MESSAGE, Some(0)) => {
                let line = self.line(obj, frame);
                line.text = Some(String::from_utf8_lossy(value).into_owned());
                line.at = at;
            }
            (FEED_KILLER, None) => self.line(obj, frame).killer = !value.is_empty(),
            (FEED_VICTIM, None) => self.line(obj, frame).victim = !value.is_empty(),
            _ => {}
        }
    }

    /// What `frame` wrote to `obj`.
    fn line(&mut self, obj: u32, frame: Option<u32>) -> &mut Line {
        let i = *self.line_of.entry((obj, frame)).or_insert_with(|| {
            self.lines.push((obj, frame, Line::default()));
            self.lines.len() - 1
        });
        &mut self.lines[i].2
    }
}

/// A change of a player's score.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Gain {
    /// Seconds since the recording started; 0 in the opening snapshot.
    pub time: f64,
    pub points: i64,
    pub frame: Option<u32>,
    /// Offset of the write in the data.
    pub at: usize,
}

/// A find one of the two sources gives.
#[derive(Clone, Debug, PartialEq)]
struct Find {
    time: f64,
    player: usize,
    frame: Option<u32>,
    at: usize,
}

fn round_ms(t: f64) -> f64 {
    (t * 1000.0).round() / 1000.0
}

/// Reads who found the objective and whose operator became known, from the
/// state stream. `sides` are the teams' roles and `feed` the round's feed,
/// whose kills explain reveals.
pub(crate) fn decode(input: &Input, sides: [Option<TeamRole>; 2], feed: &[MatchUpdate]) -> Decoded {
    let mut hud = Hud::new(input.players);
    for (start, end, frame) in input.blocks(STATE_STREAM) {
        if let Some(block) = input.data.get(start..end) {
            hud.read(block, start, frame);
        }
    }
    resolve(&hud, input.players, input.clock, sides, feed)
}

fn resolve(
    hud: &Hud,
    players: &[Player],
    clock: &Clock,
    sides: [Option<TeamRole>; 2],
    feed: &[MatchUpdate],
) -> Decoded {
    // The opening snapshot is the start of the recording.
    let seconds = |frame: Option<u32>| clock.seconds(frame).unwrap_or(0.0);
    let gains: Vec<Vec<Gain>> = (hud.scores.iter())
        .map(|writes| {
            (writes.windows(2))
                .filter(|w| w[0].value != w[1].value)
                .map(|w| Gain {
                    time: seconds(w[1].frame),
                    points: i64::from(w[1].value) - i64::from(w[0].value),
                    frame: w[1].frame,
                    at: w[1].at,
                })
                .collect()
        })
        .collect();
    let side = |player: usize| {
        let team = players.get(player).map(|p| p.team_index);
        team.and_then(|t| sides.get(t).copied().flatten())
    };
    let scored = |player: usize| {
        let entities = players.get(player).and_then(|p| p.entities.as_ref());
        entities.is_some_and(|e| e.scoreboard.is_some())
    };

    let mut out = Decoded {
        objective: objective(hud, players, clock, &gains, &side, &scored),
        ..Decoded::default()
    };

    let kills: Vec<(f64, &MatchUpdate)> = (feed.iter())
        .filter(|u| u.kind == MatchUpdateType::Kill)
        .filter_map(|u| Some((u.recording_time?, u)))
        .collect();
    for (i, p) in players.iter().enumerate() {
        let Some(Some(flag)) = hud.discovered.get(i) else {
            continue;
        };
        let time = seconds(flag.frame);
        let kill = kills.iter().find(|(at, k)| {
            k.username == p.username && time - at > -KILL_LEAD && time - at <= KILL_LAG
        });
        let mut opponents = (0..players.len())
            .filter(|&o| players.get(o).is_some_and(|o| o.team_index != p.team_index))
            .filter(|&o| scored(o))
            .peekable();
        let paid = |o: usize| {
            gains.get(o).is_some_and(|g| {
                g.iter().any(|g| {
                    BONUS.contains(&g.points)
                        && g.time - time >= -BONUS_BEFORE
                        && g.time - time < BONUS_AFTER
                })
            })
        };
        out.reveals.push(Reveal {
            username: p.username.clone(),
            trigger: match kill {
                Some(_) => Trigger::Kill,
                None => Trigger::Identified,
            },
            cause: None,
            victim: kill.map(|k| k.1.target.clone()),
            team_bonus: opponents.peek().is_some() && opponents.all(paid),
            when: clock.when(flag.at, flag.frame),
        });
    }
    let at = |r: &Reveal| r.when.recording_time.unwrap_or(0.0);
    out.reveals.sort_by(|a, b| at(a).total_cmp(&at(b)));
    out.hacks = hacks(hud, players, clock);
    // The first write of a count is the one the round started with.
    out.assists = (hud.assists.iter())
        .map(|writes| {
            let later = writes.iter().filter(|w| w.frame.is_some());
            later.map(|w| seconds(w.frame)).collect()
        })
        .collect();
    out.gains = gains;
    if hud.malformed > 0 {
        out.warnings.push(format!(
            "{} scores or states of another size than a number's",
            hud.malformed
        ));
    }
    out
}

/// Each stretch of a tablet hacking a phone. A stretch that was on as the
/// recording started has no start and is left out.
fn hacks(hud: &Hud, players: &[Player], clock: &Clock) -> Vec<PhoneHack> {
    let mut out: Vec<PhoneHack> = Vec::new();
    for &tablet in &hud.tablets {
        let Some(player) = hud.owner(tablet).and_then(|p| players.get(p)) else {
            continue;
        };
        let mut writes = (hud.equips.iter())
            .filter(|e| e.0 == tablet && e.1.frame.is_some())
            .map(|e| e.1);
        let mut start: Option<Write> = None;
        let mut close = |start: Write, end: Option<Write>| {
            let span = |end: Write| {
                let (from, to) = (clock.seconds(start.frame)?, clock.seconds(end.frame)?);
                Some(round_ms(to - from))
            };
            let seconds = end.and_then(span);
            out.push(PhoneHack {
                username: player.username.clone(),
                seconds,
                completed: seconds.is_some_and(|s| s >= HACK_SECONDS),
                when: clock.when(start.at, start.frame),
            });
        };
        for w in writes.by_ref() {
            match (start, w.value == HACKING) {
                (None, true) => start = Some(w),
                (Some(from), false) => {
                    close(from, Some(w));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(from) = start {
            close(from, None);
        }
    }
    let at = |h: &PhoneHack| h.when.recording_time.unwrap_or(0.0);
    out.sort_by(|a, b| at(a).total_cmp(&at(b)));
    out
}

/// Who found the objective: the feed's entry, else the score.
fn objective(
    hud: &Hud,
    players: &[Player],
    clock: &Clock,
    gains: &[Vec<Gain>],
    side: &dyn Fn(usize) -> Option<TeamRole>,
    scored: &dyn Fn(usize) -> bool,
) -> Option<Objective> {
    let seconds = |frame: Option<u32>| clock.seconds(frame).unwrap_or(0.0);
    let spans = clock.timeline.spans();
    // A custom game can be set to play without a prep phase: the feed
    // still names the finder, and nothing is asked of the score.
    let (prep_end, prep) = match spans.as_slice() {
        [prep, _, ..] if prep.phase == Phase::Prep => (prep.recording_end?, true),
        [action, ..] if action.phase == Phase::Action => (action.recording_start?, false),
        _ => return None,
    };

    // The feed: an entry with a text that is no number and names an
    // attacker, and no killer or victim. A longer name is tried first, as
    // it can start with a shorter one.
    let mut names: Vec<usize> = (0..players.len()).collect();
    names.sort_by_key(|&i| std::cmp::Reverse(players.get(i).map_or(0, |p| p.username.len())));
    let named = |text: &str| {
        let has = |f: &dyn Fn(&str) -> bool| {
            (names.iter().copied()).find(|&i| players.get(i).is_some_and(|p| f(&p.username)))
        };
        has(&|n| text.starts_with(n)).or_else(|| has(&|n| text.contains(n)))
    };
    let mut said: Vec<(Find, &str)> = Vec::new();
    for (obj, frame, line) in &hud.lines {
        let Some(text) = line.text.as_deref() else {
            continue;
        };
        let number = text.chars().all(|c| c.is_ascii_digit());
        if hud.entries.get(obj) != Some(&true) || text.is_empty() || number {
            continue;
        }
        if line.killer || line.victim {
            continue;
        }
        if let Some(player) = named(text).filter(|&p| side(p) == Some(TeamRole::Attack)) {
            let find = Find {
                time: seconds(*frame),
                player,
                frame: *frame,
                at: line.at,
            };
            said.push((find, text));
        }
    }
    let username = |i: usize| players.get(i).map_or("", |p| p.username.as_str());
    said.sort_by(|a, b| {
        (a.0.time.total_cmp(&b.0.time))
            .then_with(|| username(a.0.player).cmp(username(b.0.player)))
            .then_with(|| a.1.cmp(b.1))
    });

    // The score: an attacker's +50 that is no assist's, and the +100 of
    // every defender as prep ends.
    let mut jumps: Vec<Find> = Vec::new();
    let (mut defenders, mut paid) = (0, 0);
    for (i, gains) in gains.iter().enumerate().filter(|g| scored(g.0)) {
        match side(i) {
            Some(TeamRole::Attack) => {
                // The first write is the count the round started with.
                let assists = hud.assists.get(i).map_or(&[][..], |a| a.as_slice());
                let assists: Vec<f64> =
                    (assists.iter().skip(1)).map(|w| seconds(w.frame)).collect();
                let own = |g: &&Gain| {
                    g.points == FIND && !assists.iter().any(|a| (g.time - a).abs() < ASSIST)
                };
                jumps.extend(gains.iter().filter(own).map(|g| Find {
                    time: g.time,
                    player: i,
                    frame: g.frame,
                    at: g.at,
                }));
            }
            Some(TeamRole::Defense) => {
                defenders += 1;
                let bonus =
                    |g: &Gain| g.points == NOT_FOUND && (g.time - prep_end).abs() < PREP_END;
                paid += usize::from(gains.iter().any(bonus));
            }
            None => {}
        }
    }
    jumps.sort_by(|a, b| {
        (a.time.total_cmp(&b.time)).then_with(|| username(a.player).cmp(username(b.player)))
    });
    let not_found_in_prep = defenders > 0 && paid == defenders;
    let in_prep = |f: &Find| f.time < prep_end + PREP_SLACK;
    let prep_score = jumps.first().filter(|f| in_prep(f));

    let (find, source, inferred) = match (said.first(), prep_score, jumps.first()) {
        (Some((find, _)), _, _) => (find, Source::Feed, false),
        _ if !prep => return Some(Objective::default()),
        (None, Some(find), _) => (find, Source::Score, false),
        (None, None, Some(find)) if not_found_in_prep => (find, Source::Score, true),
        _ => {
            return not_found_in_prep.then(Objective::default);
        }
    };
    let when = clock.when(find.at, find.frame);
    Some(Objective {
        found: true,
        by: Some(username(find.player).to_owned()),
        source: Some(source),
        inferred,
        in_prep: Some(round_ms(find.time) < prep_end + PREP_SLACK),
        when: Some(when),
        prep_score: prep_score.map(|f| username(f.player).to_owned()),
        score_confirmed: source == Source::Feed
            && (jumps.iter())
                .any(|j| j.player == find.player && (j.time - find.time).abs() < SAME_FIND),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::PlayerEntities;
    use crate::timeline::Timeline;

    const CONTROLLER: u32 = 0xF000_0100;
    const SCOREBOARD: u32 = 0xF000_0200;
    const FEED: u32 = 0xF000_0300;
    const ENTRY: u32 = 0xF000_0400;
    const OTHER: Hash = [9, 9, 9, 9];
    const SIDES: [Option<TeamRole>; 2] = [Some(TeamRole::Attack), Some(TeamRole::Defense)];

    /// `23 <obj> 00000000 <hash> <size> <value>`.
    fn set(d: &mut Vec<u8>, obj: u32, hash: Hash, value: &[u8]) {
        d.push(0x23);
        d.extend(obj.to_le_bytes());
        d.extend([0; 4]);
        d.extend(hash);
        d.push(value.len() as u8);
        d.extend(value);
    }

    /// `22 <hash> <size> <value>`.
    fn prop(d: &mut Vec<u8>, hash: Hash, value: &[u8]) {
        d.push(0x22);
        d.extend(hash);
        d.push(value.len() as u8);
        d.extend(value);
    }

    /// `26 <hash> <index> <size> <value>`.
    fn element(d: &mut Vec<u8>, hash: Hash, index: u32, value: &[u8]) {
        d.push(0x26);
        d.extend(hash);
        d.extend(index.to_le_bytes());
        d.push(value.len() as u8);
        d.extend(value);
    }

    /// `1b <parent> 00000000 <field> <child> 00000000 <class>`.
    fn link(d: &mut Vec<u8>, parent: u32, child: u32, class: Hash) {
        d.push(0x1B);
        d.extend(parent.to_le_bytes());
        d.extend([0; 4]);
        d.extend(OTHER);
        d.extend(child.to_le_bytes());
        d.extend([0; 4]);
        d.extend(class);
    }

    /// A round of two attackers (players 0 and 1) and two defenders, with
    /// the HUD fed frame by frame. Frame `n` is `n / 10` seconds into the
    /// recording; prep ends at 45 s.
    struct Fixture {
        players: Vec<Player>,
        hud: Hud,
        feed: Vec<MatchUpdate>,
    }

    impl Fixture {
        fn new() -> Self {
            let players: Vec<Player> = (0..4u32)
                .map(|i| Player {
                    username: format!("p{i}"),
                    team_index: (i / 2) as usize,
                    entities: Some(PlayerEntities {
                        controller: CONTROLLER + i,
                        scoreboard: Some(SCOREBOARD + i),
                        ..PlayerEntities::default()
                    }),
                    ..Player::default()
                })
                .collect();
            let mut f = Fixture {
                hud: Hud::new(&players),
                players,
                feed: Vec::new(),
            };
            // The snapshot states every score and assist count.
            let mut d = vec![];
            for i in 0..4 {
                set(&mut d, SCOREBOARD + i, MATCH_SCORE, &0u32.to_le_bytes());
                prop(&mut d, MATCH_ASSISTS, &0u32.to_le_bytes());
                set(&mut d, CONTROLLER + i, DISCOVERED, &[0]);
            }
            f.hud.read(&d, 0, None);
            f
        }

        fn frame(&mut self, frame: u32, block: &[u8]) {
            self.hud.read(block, 1000 * frame as usize, Some(frame));
        }

        fn score(&mut self, frame: u32, player: u32, score: u32) {
            let mut d = vec![];
            set(
                &mut d,
                SCOREBOARD + player,
                MATCH_SCORE,
                &score.to_le_bytes(),
            );
            self.frame(frame, &d);
        }

        /// Both defenders get their 100 as prep ends.
        fn not_found_in_prep(&mut self) {
            self.score(451, 2, 100);
            self.score(452, 3, 100);
        }

        fn kill(&mut self, by: &str, target: &str, at: f64) {
            self.feed.push(MatchUpdate {
                username: by.into(),
                target: target.into(),
                recording_time: Some(at),
                ..MatchUpdate::new(MatchUpdateType::Kill, &crate::feedback::Clock::default())
            });
        }

        fn resolve(&self) -> Decoded {
            let times: Vec<f64> = (0..3000).map(|f| f64::from(f) / 10.0).collect();
            // A clock that reads 0:45, 0:00, 3:00 and 2:59.
            let recording = vec![Some(0.0), Some(45.0), Some(46.0), Some(47.0)];
            let mut timeline =
                Timeline::resolve(&[45.0, 0.0, 180.0, 179.0], None, None, &recording);
            timeline.recording = recording;
            let clock = Clock {
                timeline: &timeline,
                reading_offsets: &[0, 450_000, 460_000, 470_000],
                frame_times: &times,
            };
            resolve(&self.hud, &self.players, &clock, SIDES, &self.feed)
        }
    }

    #[test]
    fn a_find_in_prep_is_the_first_fifty_of_an_attacker() {
        let mut f = Fixture::new();
        f.score(120, 1, 50);
        f.score(200, 0, 50);
        // Defenders score other things.
        f.score(130, 2, 50);
        let o = f.resolve().objective.unwrap();
        assert!(o.found && !o.inferred);
        assert_eq!(o.by.as_deref(), Some("p1"));
        assert_eq!(o.source, Some(Source::Score));
        assert_eq!(o.in_prep, Some(true));
        assert_eq!(o.prep_score.as_deref(), Some("p1"));
        let when = o.when.unwrap();
        assert_eq!((when.phase, when.recording_time), (Phase::Prep, Some(12.0)));
    }

    #[test]
    fn a_find_after_prep_is_inferred_and_no_assist() {
        let mut f = Fixture::new();
        f.not_found_in_prep();
        // An assist is worth 50 too: the count rises with the score.
        let mut d = vec![];
        set(&mut d, SCOREBOARD, MATCH_SCORE, &50u32.to_le_bytes());
        prop(&mut d, MATCH_ASSISTS, &1u32.to_le_bytes());
        f.frame(600, &d);
        let o = f.resolve().objective.unwrap();
        assert_eq!(o, Objective::default(), "not found");
        let json = serde_json::to_value(&o).unwrap();
        assert_eq!(json, serde_json::json!({ "found": false }));

        f.score(700, 1, 10);
        f.score(800, 1, 60);
        let o = f.resolve().objective.unwrap();
        assert!(o.found && o.inferred);
        assert_eq!((o.by.as_deref(), o.in_prep), (Some("p1"), Some(false)));
        assert_eq!(o.prep_score, None);
        assert_eq!(o.when.unwrap().phase, Phase::Action);
    }

    #[test]
    fn a_round_that_shows_neither_has_no_objective() {
        let mut f = Fixture::new();
        // One defender alone gets 100, and no attacker 50.
        f.score(451, 2, 100);
        f.score(800, 1, 60);
        assert_eq!(f.resolve().objective, None);
        // A +50 after prep is a find only when the defenders got theirs.
        f.score(900, 0, 50);
        assert_eq!(f.resolve().objective, None);
    }

    /// A feed entry's write: the message, its text, and who it names.
    fn entry(text: &str, killer: &[u8]) -> Vec<u8> {
        let mut d = vec![];
        link(&mut d, FEED, ENTRY, FEED_ENTRY);
        set(&mut d, ENTRY, FEED_MESSAGE, &[0xFF; 8]);
        element(&mut d, FEED_MESSAGE, 0, text.as_bytes());
        prop(&mut d, FEED_KILLER, killer);
        prop(&mut d, FEED_VICTIM, &[]);
        d
    }

    #[test]
    fn the_feed_names_the_finder_before_the_score_does() {
        let mut f = Fixture::new();
        f.frame(100, &entry("p0 has found the bombs", &[]));
        // Written again as the feed scrolls; the first is the find.
        f.frame(140, &entry("p0 has found the bombs", &[]));
        f.score(101, 0, 50);
        f.score(90, 1, 50);
        let o = f.resolve().objective.unwrap();
        assert_eq!(
            (o.by.as_deref(), o.source),
            (Some("p0"), Some(Source::Feed))
        );
        assert!(o.score_confirmed && !o.inferred);
        assert_eq!(o.when.unwrap().recording_time, Some(10.0));
        // The score rule alone would have named the other attacker.
        assert_eq!(o.prep_score.as_deref(), Some("p1"));
    }

    #[test]
    fn only_a_text_that_names_an_attacker_and_no_killer_is_a_find() {
        for (text, killer) in [
            ("p2 has found the bombs", &[][..]),
            ("p0 has found the bombs", &[1, 2, 3, 4][..]),
            ("12", &[]),
            ("", &[]),
            ("somebody has left", &[]),
        ] {
            let mut f = Fixture::new();
            f.frame(100, &entry(text, killer));
            assert_eq!(f.resolve().objective, None, "{text:?}");
        }
        // The same text on an object of another class.
        let mut f = Fixture::new();
        let mut d = entry("p0 has found the bombs", &[]);
        link(&mut d, FEED, ENTRY, OTHER);
        f.frame(100, &d);
        assert_eq!(f.resolve().objective, None);
    }

    fn discovered(f: &mut Fixture, frame: u32, player: u32, value: u8) {
        let mut d = vec![];
        set(&mut d, CONTROLLER + player, DISCOVERED, &[value]);
        f.frame(frame, &d);
    }

    #[test]
    fn a_reveal_is_the_first_one_and_a_kill_explains_it() {
        let mut f = Fixture::new();
        f.kill("p0", "p3", 60.0);
        f.kill("p2", "p1", 80.0);
        discovered(&mut f, 603, 0, 1);
        // Written again, and taken back: the first write stands.
        discovered(&mut f, 700, 0, 1);
        discovered(&mut f, 710, 0, 0);
        // Too long after the kill to be its doing.
        discovered(&mut f, 805, 2, 1);
        // Seen in prep: both opponents score 10 in that moment.
        discovered(&mut f, 300, 3, 1);
        f.score(300, 0, 10);
        f.score(302, 1, 10);
        let reveals = f.resolve().reveals;
        let told: Vec<_> = (reveals.iter())
            .map(|r| {
                let victim = r.victim.as_deref();
                (r.username.as_str(), r.trigger, victim, r.team_bonus)
            })
            .collect();
        assert_eq!(
            told,
            [
                ("p3", Trigger::Identified, None, true),
                ("p0", Trigger::Kill, Some("p3"), false),
                ("p2", Trigger::Identified, None, false),
            ]
        );
        assert_eq!(reveals[1].when.recording_time, Some(60.3));
        assert_eq!(reveals[0].when.phase, Phase::Prep);
    }

    /// A write of the tablet's state.
    fn equip(f: &mut Fixture, frame: u32, tablet: u32, state: u32) {
        let mut d = vec![];
        set(&mut d, tablet, EQUIP_STATE, &state.to_le_bytes());
        f.frame(frame, &d);
    }

    #[test]
    fn a_stretch_of_the_tablet_hacking_is_a_phone_hack() {
        const ABILITY: u32 = 0xF000_0500;
        const TABLET_OBJECT: u32 = 0xF000_0600;
        let mut f = Fixture::new();
        // Player 1's controller holds the ability object, which links the
        // tablet through `Tablet`.
        let mut d = vec![];
        link(&mut d, CONTROLLER + 1, ABILITY, OTHER);
        d.push(0x1B);
        d.extend(ABILITY.to_le_bytes());
        d.extend([0; 4]);
        d.extend(TABLET);
        d.extend(TABLET_OBJECT.to_le_bytes());
        d.extend([0; 4]);
        d.extend(OTHER);
        set(&mut d, TABLET_OBJECT, EQUIP_STATE, &0u32.to_le_bytes());
        f.hud.read(&d, 0, None);
        // A call: 1, 2, back to 0. Then a hack of 2.5 s, said twice.
        for (frame, state) in [(500, 1), (510, 2), (530, 0), (900, 3), (910, 3), (925, 0)] {
            equip(&mut f, frame, TABLET_OBJECT, state);
        }
        // One cut short, and one the recording ends in.
        for (frame, state) in [(1000, 3), (1010, 1), (1200, 3)] {
            equip(&mut f, frame, TABLET_OBJECT, state);
        }
        // The same state on an object no `Tablet` field links is not one.
        equip(&mut f, 950, ABILITY, 3);
        let hacks = f.resolve().hacks;
        let told: Vec<_> = (hacks.iter())
            .map(|h| (h.when.recording_time, h.seconds, h.completed))
            .collect();
        assert_eq!(
            told,
            [
                (Some(90.0), Some(2.5), true),
                (Some(100.0), Some(1.0), false),
                (Some(120.0), None, false),
            ]
        );
        assert!(hacks.iter().all(|h| h.username == "p1"));
        let json = serde_json::to_value(&hacks[2]).unwrap();
        assert!(json.get("seconds").is_none());
        assert_eq!(json["completed"], false);
    }

    #[test]
    fn a_tablet_of_no_player_or_a_state_of_another_size_is_no_hack() {
        let mut f = Fixture::new();
        let mut d = vec![];
        // Linked from an object that hangs off no controller.
        d.push(0x1B);
        d.extend(0xF000_0700u32.to_le_bytes());
        d.extend([0; 4]);
        d.extend(TABLET);
        d.extend(0xF000_0800u32.to_le_bytes());
        d.extend([0; 4]);
        d.extend(OTHER);
        set(&mut d, 0xF000_0800, EQUIP_STATE, &3u32.to_le_bytes());
        prop(&mut d, EQUIP_STATE, &[3, 0]);
        f.frame(100, &d);
        // Bytes cut anywhere do not panic.
        for cut in 0..d.len() {
            let mut hud = Hud::new(&f.players);
            hud.read(&d[..cut], 0, Some(1));
        }
        let out = f.resolve();
        assert!(out.hacks.is_empty());
        assert_eq!(out.warnings.len(), 1);
    }

    #[test]
    fn the_scores_are_kept_for_the_joins() {
        let mut f = Fixture::new();
        f.score(100, 0, 50);
        let mut d = vec![];
        set(&mut d, SCOREBOARD, MATCH_SCORE, &125u32.to_le_bytes());
        prop(&mut d, MATCH_ASSISTS, &1u32.to_le_bytes());
        f.frame(200, &d);
        let out = f.resolve();
        let points: Vec<_> = out.gains[0].iter().map(|g| (g.time, g.points)).collect();
        assert_eq!(points, [(10.0, 50), (20.0, 75)]);
        assert_eq!(out.assists[0], [20.0]);
        assert!(out.gains[1].is_empty() && out.assists[1].is_empty());
    }

    #[test]
    fn values_of_another_size_are_skipped_and_counted() {
        let mut f = Fixture::new();
        let mut d = vec![];
        set(&mut d, SCOREBOARD, MATCH_SCORE, &[1, 2]);
        set(&mut d, CONTROLLER, DISCOVERED, &[1, 0, 0, 0]);
        f.frame(10, &d);
        let out = f.resolve();
        assert_eq!(out.warnings.len(), 1);
        assert!(out.reveals.is_empty());
    }
}
