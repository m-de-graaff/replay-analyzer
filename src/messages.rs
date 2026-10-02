//! The lines of the feed that are no kills (Y11S3): players leaving,
//! joining and reconnecting, reverse friendly fire turning on and off, the
//! objective being found and the announcements of a phase, and with them
//! the round's BattlEye flag.
//!
//! Every line of the on-screen feed is one object of class `2b9d6947`, an
//! element of the array field `Messages` (`c07c7422`) in the `state`
//! stream, read the way [`crate::loadout`] reads slots. One write of an
//! entry is a run of property records:
//!
//! ```text
//! 23 <obj> 00000000 5934e58b 04 <u32>   BackgroundColor: 0, or 1 or 2 for
//!                                       a team's colour
//! 22 e3090079 08 <8 bytes>              Message: the id of a localised
//!                                       line, ff*8 for none
//! 26 e3090079 <0> <size> <utf-8>        Message[0]: ready-made text
//! 26 e3090079 <1> 01 <n>                Message[1]: how many arguments
//! 26 e3090079 <2> 04 <hash>             Message[2]: the placeholder,
//!                                       crc32("[PLAYER]") or "[STRING]"
//! 26 e3090079 <3> <size> <utf-8>        Message[3]: its value, a name
//! 22 d9133cba <size> <utf-8>            KillerName
//! 22 ac190f70 <size> <utf-8>            VictimName
//! ...
//! 22 0548b241 04 <u32>                  Index: the line of the feed, 0 the
//!                                       newest
//! ```
//!
//! A line is either a text (`Message[0]`, with the id ff*8) or an id with
//! its arguments (no `Message[0]`). The wording of an id is not in the
//! file: the game looks it up in its own language files. What an id means
//! is therefore inferred from when it shows, over 185 real rounds, and
//! says `kindSource: inferred`; a text is the game's own and says
//! `decoded`. An id that is not in the table below is kept as `unknown`
//! with its raw bytes, never dropped: no BattlEye line exists in any round
//! at hand, so that is how one will surface.
//!
//! An entry is written again each time the feed scrolls (`Index` rises),
//! so one line shown is several writes. A write is a new line when its
//! content was not on a line above it before the frame, and not on the
//! same line in the last [`REPEAT`] seconds (the entries' `Duration`).
//! Counting kill lines that way gives the kills of the feed in 185 of 185
//! rounds.
//!
//! Two other objects have a `Message` and are no lines: the defuser's
//! interaction object (class `d71a5450`) and a counter under the field
//! `35a9ebc9`. Neither is an element of `Messages`.
//!
//! The BattlEye flag marks the round and repeats what the game showed: it
//! is set when a line's text says "BattlEye". It names no player as a
//! cheater, and nothing here does.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use crate::entities::{Hash, Record, for_each_record, u32_at};
use crate::header::Player;
use crate::loadout::{Clock, Input, STATE_STREAM, When};
use crate::types::TeamRole;

/// Class of an entry of the feed, and the array field it is an element of
/// (`Messages`).
const ENTRY: Hash = [0x2B, 0x9D, 0x69, 0x47];
const MESSAGES: Hash = [0xC0, 0x7C, 0x74, 0x22];
/// `BackgroundColor`, the first property of an entry.
const BACKGROUND: Hash = [0x59, 0x34, 0xE5, 0x8B];
/// `Message`: an id, and as array elements a text or arguments.
const MESSAGE: Hash = [0xE3, 0x09, 0x00, 0x79];
const KILLER: Hash = [0xD9, 0x13, 0x3C, 0xBA];
const VICTIM: Hash = [0xAC, 0x19, 0x0F, 0x70];
/// `Index`: the line of the feed the entry is on.
const INDEX: Hash = [0x05, 0x48, 0xB2, 0x41];
/// The id of a line that is a text or a kill.
const NO_ID: [u8; 8] = [0xFF; 8];
/// The placeholders an argument fills, by their hash.
const PLACEHOLDERS: [(Hash, &str); 2] = [
    ([0x3C, 0x7F, 0xB1, 0x0E], "[PLAYER]"),
    ([0x6F, 0x56, 0x65, 0x9C], "[STRING]"),
];
/// How long a line stays up, in seconds (the entries' `Duration`): the
/// same content on the same line after that is a new line.
const REPEAT: f64 = 5.0;
/// The same in frames, for a frame without a time.
const REPEAT_FRAMES: u32 = 1500;

/// What each id stood next to in 185 real rounds.
const IDS: [([u8; 8], Kind); 9] = [
    ([0x39, 0xF4, 0, 0, 0, 0, 0, 0x65], Kind::PlayerLeft),
    ([0x38, 0xF4, 0, 0, 0, 0, 0, 0x65], Kind::PlayerJoined),
    (
        [0xFC, 0xDB, 0x02, 0, 0, 0, 0, 0x65],
        Kind::PlayerReconnected,
    ),
    ([0x66, 0xF4, 0x02, 0, 0, 0, 0, 0x65], Kind::ConnectionLost),
    (
        [0xBE, 0xA3, 0x04, 0, 0, 0, 0, 0x65],
        Kind::ReverseFriendlyFireOn,
    ),
    (
        [0x2C, 0xA9, 0x04, 0, 0, 0, 0, 0x65],
        Kind::ReverseFriendlyFireOnSquad,
    ),
    (
        [0xBF, 0xA3, 0x04, 0, 0, 0, 0, 0x65],
        Kind::ReverseFriendlyFireOff,
    ),
    ([0xC3, 0xC5, 0x05, 0, 0, 0, 0, 0x65], Kind::Phase),
    ([0xC4, 0xC5, 0x05, 0, 0, 0, 0, 0x65], Kind::Phase),
];

/// What a line says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// A player left the match.
    PlayerLeft,
    /// A player joined a match under way.
    PlayerJoined,
    /// A player who had left is back.
    PlayerReconnected,
    /// Shown up to 0.4 s before some leaves; the weakest of the inferred
    /// kinds.
    ConnectionLost,
    /// Reverse friendly fire turned on for the player named.
    ReverseFriendlyFireOn,
    /// The same, said of a squad-mate of the player it turned on for.
    ReverseFriendlyFireOnSquad,
    ReverseFriendlyFireOff,
    /// The text names the attacker who found the objective.
    ObjectiveFound,
    /// One of the two lines every round shows: one as the recording
    /// starts, one as the action phase does.
    Phase,
    /// An id that is in no table, or a text that names no attacker.
    Unknown,
}

/// How a line's `kind` is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KindSource {
    /// The line is a text of the game's.
    Decoded,
    /// The line is an id, whose meaning is told by when it shows.
    Inferred,
}

/// One value a line's wording is filled with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Arg {
    /// The placeholder's hash, as hex in stream order.
    pub key: String,
    /// The placeholder, when its hash is a known one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'static str>,
    pub value: String,
}

/// A line of the feed that is no kill.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMessage {
    pub kind: Kind,
    /// Absent for `unknown`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind_source: Option<KindSource>,
    /// The id of the line's wording: its 8 bytes as hex. Absent for a text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// The text as the game wrote it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Arg>,
    /// The player the line names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Their profile id, when the name is a player's of the header.
    #[serde(rename = "profileID", skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    /// 0, or 1 or 2 for a team's colour.
    pub background_color: u32,
    #[serde(flatten)]
    pub when: When,
    /// The frame the line was first written in.
    #[serde(skip)]
    pub frame: Option<u32>,
    /// Where in the decompressed data that write starts.
    #[serde(skip)]
    pub offset: usize,
}

/// The round's BattlEye flag: whether the game showed a line that says
/// "BattlEye". It marks the round and repeats what the game showed; it
/// says nothing of any player.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BattlEye {
    pub flagged: bool,
    /// Indexes into `systemMessages` of the lines that set the flag.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<usize>,
    /// Their texts, as the game wrote them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub texts: Vec<String>,
    /// Lines of a kind not known, which a BattlEye line would be one of
    /// from Y11S3: a round with some is worth a look.
    pub unknown_messages: usize,
}

/// The rounds of a match whose `battlEye` is flagged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchBattlEye {
    /// Round numbers, from 1.
    pub flagged_rounds: Vec<u32>,
}

/// What [`decode`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decoded {
    pub lines: Vec<SystemMessage>,
    /// The ids of `unknown` lines, each once, as hex.
    pub unknown_ids: Vec<String>,
    /// What could not be read, for `decodeStatus`.
    pub warnings: Vec<String>,
}

/// Whether a text of the feed says "BattlEye".
pub(crate) fn names_battleye(text: &str) -> bool {
    text.to_ascii_lowercase().contains("battleye")
}

/// The lines of `lines` of one kind: what a join of leaves or reconnects
/// starts from. Each has its `username`, `frame`, `offset` and
/// `when.recording_time`.
pub(crate) fn of_kind(lines: &[SystemMessage], kind: Kind) -> impl Iterator<Item = &SystemMessage> {
    lines.iter().filter(move |l| l.kind == kind)
}

/// The flag of a round whose feed lines are `lines`.
pub(crate) fn battleye(lines: &[SystemMessage]) -> BattlEye {
    let said = |l: &&SystemMessage| l.text.as_deref().is_some_and(names_battleye);
    let hits: Vec<usize> = (lines.iter().enumerate())
        .filter(|(_, l)| said(l))
        .map(|(i, _)| i)
        .collect();
    BattlEye {
        flagged: !hits.is_empty(),
        texts: (lines.iter().filter(said))
            .filter_map(|l| l.text.clone())
            .collect(),
        messages: hits,
        unknown_messages: of_kind(lines, Kind::Unknown).count(),
    }
}

/// The flag of a round before Y9S1, whose feed is text: `texts` are the
/// lines that say "BattlEye", and `entry` whether the feed has an entry
/// made of one.
pub(crate) fn legacy_battleye(texts: Vec<String>, entry: bool) -> BattlEye {
    BattlEye {
        flagged: entry || !texts.is_empty(),
        texts,
        ..BattlEye::default()
    }
}

/// The rollup of a match: `None` when no round has the flag at all.
pub(crate) fn match_battleye<'a>(
    rounds: impl Iterator<Item = (u32, Option<&'a BattlEye>)>,
) -> Option<MatchBattlEye> {
    let mut out: Option<MatchBattlEye> = None;
    for (number, flag) in rounds {
        let Some(flag) = flag else { continue };
        let rollup = out.get_or_insert_with(MatchBattlEye::default);
        if flag.flagged {
            rollup.flagged_rounds.push(number);
        }
    }
    out
}

/// One write of an object that may be an entry of the feed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Write {
    obj: u32,
    frame: Option<u32>,
    /// Offset of the write's first record in the data.
    at: usize,
    background: Option<u32>,
    id: Option<[u8; 8]>,
    /// The array elements of `Message`, by index.
    elements: BTreeMap<u32, Vec<u8>>,
    killer: Vec<u8>,
    victim: Vec<u8>,
    index: u32,
}

/// The writes and links of the state stream that the feed is made of.
#[derive(Debug, Default)]
struct Hud {
    /// The class and field each object was last linked with.
    links: HashMap<u32, (Hash, Hash)>,
    /// In stream order.
    writes: Vec<Write>,
    /// Values whose size the property never has.
    malformed: usize,
}

impl Hud {
    /// Reads the records of one snapshot or frame record. `base` is where
    /// `block` starts in the data.
    fn read(&mut self, block: &[u8], base: usize, frame: Option<u32>) {
        // Each record block names its object before writing to it.
        let mut current: Option<u32> = None;
        // The write being gathered, and where the record after it starts:
        // a write ends at the next object, at a link and at bytes that are
        // no record.
        let mut open: Option<Write> = None;
        let mut next = 0;
        let mut done: Vec<Write> = Vec::new();
        let mut malformed = 0;
        let class = |at: usize| -> Option<Hash> { block.get(at..at + 4)?.try_into().ok() };
        for_each_record(block, |at, r| {
            if at != next {
                done.extend(open.take());
                current = None;
            }
            let start = |obj: u32| Write {
                obj,
                frame,
                at: base + at,
                ..Write::default()
            };
            match r {
                Record::Set(obj, hash, from, to) => {
                    done.extend(open.take());
                    current = Some(obj);
                    let write = open.insert(start(obj));
                    if let Some(value) = block.get(from..to) {
                        malformed += usize::from(!property(write, hash, None, value));
                    }
                    next = to;
                }
                Record::Prop(hash, from, to) => {
                    // A `26` is an array element and says which.
                    let element = match block.get(at) {
                        Some(0x26) => u32_at(block, at + 5),
                        _ => None,
                    };
                    if let (Some(obj), Some(value)) = (current, block.get(from..to)) {
                        let write = open.get_or_insert_with(|| start(obj));
                        malformed += usize::from(!property(write, hash, element, value));
                    }
                    next = to;
                }
                Record::ParentChild(parent, field, child) => {
                    done.extend(open.take());
                    current = Some(parent);
                    self.link(child, field, class(at + 21));
                    next = at + 25;
                }
                Record::Child(field, child) => {
                    done.extend(open.take());
                    self.link(child, field, class(at + 13));
                    next = at + 17;
                }
                Record::Element(field, _, child) => {
                    done.extend(open.take());
                    self.link(child, field, class(at + 17));
                    next = at + 21;
                }
            }
        });
        done.extend(open);
        self.malformed += malformed;
        // Only a write with a `Message` can be a line.
        (self.writes).extend(done.into_iter().filter(|w| w.id.is_some()));
    }

    fn link(&mut self, child: u32, field: Hash, class: Option<Hash>) {
        if let Some(class) = class {
            self.links.insert(child, (class, field));
        }
    }

    /// Whether `obj` is an entry of the feed.
    fn is_entry(&self, obj: u32) -> bool {
        self.links.get(&obj) == Some(&(ENTRY, MESSAGES))
    }
}

/// Takes one property of a write. False for a value of a size the property
/// never has, which is left out.
fn property(write: &mut Write, hash: Hash, element: Option<u32>, value: &[u8]) -> bool {
    let number = || Some(u32::from_le_bytes(value.try_into().ok()?));
    match (hash, element) {
        (BACKGROUND, None) => {
            write.background = number();
            write.background.is_some()
        }
        (MESSAGE, None) => {
            write.id = value.try_into().ok();
            write.id.is_some()
        }
        (MESSAGE, Some(i)) => {
            write.elements.insert(i, value.to_vec());
            true
        }
        (KILLER, None) => {
            write.killer = value.to_vec();
            true
        }
        (VICTIM, None) => {
            write.victim = value.to_vec();
            true
        }
        (INDEX, None) => {
            write.index = number().unwrap_or(0);
            true
        }
        _ => true,
    }
}

/// What a write shows: two writes with the same are the same line.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Content {
    id: [u8; 8],
    text: Vec<u8>,
    /// `(placeholder, value)` of each argument.
    args: Vec<(Vec<u8>, Vec<u8>)>,
    killer: Vec<u8>,
    victim: Vec<u8>,
}

impl Content {
    fn of(write: &Write) -> Option<Content> {
        let element = |i: u32| write.elements.get(&i);
        // Arguments follow the count: a placeholder and its value each.
        let args = (1u32..)
            .map_while(|n| Some((element(2 * n)?.clone(), element(2 * n + 1)?.clone())))
            .collect();
        Some(Content {
            id: write.id?,
            text: element(0).cloned().unwrap_or_default(),
            args,
            killer: write.killer.clone(),
            victim: write.victim.clone(),
        })
    }

    fn is_kill(&self) -> bool {
        !self.killer.is_empty() || !self.victim.is_empty()
    }

    /// An entry cleared, or one not yet used.
    fn is_empty(&self) -> bool {
        self.id == NO_ID && self.text.is_empty() && !self.is_kill()
    }
}

/// What an entry last showed.
struct Shown {
    obj: u32,
    content: Content,
    index: u32,
    frame: u32,
    time: Option<f64>,
}

/// The writes that are a new line: `(write, content)` in stream order,
/// kills among them, the rewrites of a scrolling feed left out.
fn new_lines<'a>(hud: &'a Hud, clock: &Clock) -> Vec<(&'a Write, Content)> {
    // The opening snapshot is what was on screen before the recording.
    let writes: Vec<(&Write, u32, Content)> = (hud.writes.iter())
        .filter(|w| hud.is_entry(w.obj) && w.background.is_some())
        .filter_map(|w| Some((w, w.frame?, Content::of(w)?)))
        .collect();
    // In the order the entries were first written.
    let mut shown: Vec<Shown> = Vec::new();
    let mut out: Vec<(&Write, Content)> = Vec::new();
    for batch in writes.chunk_by(|a, b| a.1 == b.1) {
        let Some(&(_, frame, _)) = batch.first() else {
            continue;
        };
        let time = clock.seconds(Some(frame));
        // What was up before this frame. Lines lower down go first, so
        // each takes the nearest line above it.
        let before: Vec<(Content, u32, u32, Option<f64>)> = (shown.iter())
            .map(|s| (s.content.clone(), s.index, s.frame, s.time))
            .collect();
        let mut used = vec![false; before.len()];
        let mut order: Vec<&(&Write, u32, Content)> = batch.iter().collect();
        order.sort_by_key(|w| std::cmp::Reverse(w.0.index));
        for (write, _, content) in order {
            let now = Shown {
                obj: write.obj,
                content: content.clone(),
                index: write.index,
                frame,
                time,
            };
            match shown.iter_mut().find(|s| s.obj == write.obj) {
                Some(s) => *s = now,
                None => shown.push(now),
            }
            let mut best: Option<usize> = None;
            for (n, (was, index, then, at)) in before.iter().enumerate() {
                if used.get(n) != Some(&false) || was != content {
                    continue;
                }
                let fresh = match (time, at) {
                    (Some(now), Some(at)) => now - at <= REPEAT,
                    _ => frame.saturating_sub(*then) <= REPEAT_FRAMES,
                };
                let scrolled = *index < write.index || (*index == write.index && fresh);
                let higher = best
                    .and_then(|b| before.get(b))
                    .is_none_or(|b| *index > b.1);
                if scrolled && higher {
                    best = Some(n);
                }
            }
            match best.and_then(|b| used.get_mut(b)) {
                Some(u) => *u = true,
                None if !content.is_empty() => out.push((write, content.clone())),
                None => {}
            }
        }
    }
    out.sort_by_key(|(w, _)| (w.frame, w.at));
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Reads the feed's lines that are no kills from the state stream. `sides`
/// are the teams' roles: a find is a text that names an attacker.
pub(crate) fn decode(input: &Input, sides: [Option<TeamRole>; 2]) -> Decoded {
    let mut hud = Hud::default();
    for (start, end, frame) in input.blocks(STATE_STREAM) {
        if let Some(block) = input.data.get(start..end) {
            hud.read(block, start, frame);
        }
    }
    resolve(&hud, input.players, input.clock, sides)
}

fn resolve(hud: &Hud, players: &[Player], clock: &Clock, sides: [Option<TeamRole>; 2]) -> Decoded {
    // The player a text names. A longer name is tried first, as it can
    // start with a shorter one.
    let mut names: Vec<&Player> = players.iter().collect();
    names.sort_by_key(|p| std::cmp::Reverse(p.username.len()));
    let named = |text: &str| {
        let has = |f: &dyn Fn(&str) -> bool| {
            (names.iter().copied()).find(|p| !p.username.is_empty() && f(&p.username))
        };
        has(&|n| text.starts_with(n)).or_else(|| has(&|n| text.contains(n)))
    };
    let attacks = |p: &Player| sides.get(p.team_index).copied().flatten() == Some(TeamRole::Attack);
    let profile = |name: &str| {
        let player = players.iter().find(|p| p.username == name);
        player
            .map(|p| p.profile_id.clone())
            .filter(|id| !id.is_empty())
    };

    let mut out = Decoded::default();
    for (write, content) in new_lines(hud, clock) {
        if content.is_kill() {
            continue;
        }
        let text = String::from_utf8_lossy(&content.text).into_owned();
        let args: Vec<Arg> = (content.args.iter())
            .map(|(key, value)| Arg {
                key: hex(key),
                name: (PLACEHOLDERS.iter())
                    .find(|p| p.0.as_slice() == key.as_slice())
                    .map(|p| p.1),
                value: String::from_utf8_lossy(value).into_owned(),
            })
            .collect();
        let (kind, kind_source, username) = if content.id == NO_ID {
            // A text is told by its shape, not its words, which may be
            // translated: a find names an attacker.
            let player = named(&text);
            let find = player.is_some_and(attacks) && !names_battleye(&text);
            let kind = if find {
                (Kind::ObjectiveFound, Some(KindSource::Decoded))
            } else {
                (Kind::Unknown, None)
            };
            (kind.0, kind.1, player.map(|p| p.username.clone()))
        } else {
            let kind = IDS.iter().find(|i| i.0 == content.id).map(|i| i.1);
            // The first argument is the player the line is about.
            let username = args.first().map(|a| a.value.clone());
            match kind {
                Some(kind) => (kind, Some(KindSource::Inferred), username),
                None => {
                    let id = hex(&content.id);
                    if !out.unknown_ids.contains(&id) {
                        out.unknown_ids.push(id);
                    }
                    // What an argument of an unknown line is, is not known
                    // either, unless it says a player.
                    let player = args.first().filter(|a| a.name == Some("[PLAYER]"));
                    (Kind::Unknown, None, player.map(|a| a.value.clone()))
                }
            }
        };
        out.lines.push(SystemMessage {
            kind,
            kind_source,
            message_id: (content.id != NO_ID).then(|| hex(&content.id)),
            text: (!text.is_empty()).then_some(text),
            args,
            profile_id: username.as_deref().and_then(profile),
            username,
            background_color: write.background.unwrap_or(0),
            when: clock.when(write.at, write.frame),
            frame: write.frame,
            offset: write.at,
        });
    }
    if hud.malformed > 0 {
        out.warnings.push(format!(
            "{} colours or ids of feed entries of another size than theirs",
            hud.malformed
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::details::Phase;
    use crate::timeline::Timeline;

    const FEED: u32 = 0xF000_0300;
    const ENTRY_OBJECT: u32 = 0xF000_0400;
    const OTHER: Hash = [9, 9, 9, 9];
    const SIDES: [Option<TeamRole>; 2] = [Some(TeamRole::Attack), Some(TeamRole::Defense)];
    const LEFT: [u8; 8] = [0x39, 0xF4, 0, 0, 0, 0, 0, 0x65];
    const PLAYER: Hash = [0x3C, 0x7F, 0xB1, 0x0E];

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
    fn element(d: &mut Vec<u8>, index: u32, value: &[u8]) {
        d.push(0x26);
        d.extend(MESSAGE);
        d.extend(index.to_le_bytes());
        d.push(value.len() as u8);
        d.extend(value);
    }

    /// `1b <parent> 00000000 <field> <child> 00000000 <class>`.
    fn link(d: &mut Vec<u8>, child: u32, field: Hash, class: Hash) {
        d.push(0x1B);
        d.extend(FEED.to_le_bytes());
        d.extend([0; 4]);
        d.extend(field);
        d.extend(child.to_le_bytes());
        d.extend([0; 4]);
        d.extend(class);
    }

    /// What a line shows.
    #[derive(Clone, Copy)]
    enum Says<'a> {
        Text(&'a str),
        Id([u8; 8], &'a [(Hash, &'a str)]),
        Kill(&'a str, &'a str),
    }

    /// One write of entry `n` on line `index` of the feed.
    fn entry(n: u32, index: u32, says: Says) -> Vec<u8> {
        let mut d = vec![];
        let (id, killer, victim) = match says {
            Says::Id(id, _) => (id, "", ""),
            Says::Text(_) => (NO_ID, "", ""),
            Says::Kill(killer, victim) => (NO_ID, killer, victim),
        };
        set(&mut d, ENTRY_OBJECT + n, BACKGROUND, &1u32.to_le_bytes());
        prop(&mut d, MESSAGE, &id);
        match says {
            Says::Text(text) => {
                element(&mut d, 0, text.as_bytes());
                element(&mut d, 1, &[0]);
            }
            Says::Kill(..) => {
                element(&mut d, 0, &[]);
                element(&mut d, 1, &[0]);
            }
            Says::Id(_, args) => {
                element(&mut d, 1, &[args.len() as u8]);
                for (i, (key, value)) in args.iter().enumerate() {
                    element(&mut d, 2 + 2 * i as u32, key);
                    element(&mut d, 3 + 2 * i as u32, value.as_bytes());
                }
            }
        }
        prop(&mut d, KILLER, killer.as_bytes());
        prop(&mut d, VICTIM, victim.as_bytes());
        prop(&mut d, INDEX, &index.to_le_bytes());
        d
    }

    /// A round of two attackers (`p0`, `p1`) and two defenders, with a feed
    /// of four entries. Frame `n` is `n / 10` seconds into the recording.
    struct Fixture {
        players: Vec<Player>,
        hud: Hud,
    }

    impl Fixture {
        fn new() -> Self {
            let players = (0..4usize)
                .map(|i| Player {
                    username: format!("p{i}"),
                    profile_id: format!("id-{i}"),
                    team_index: i / 2,
                    ..Player::default()
                })
                .collect();
            let mut d = vec![];
            for n in 0..4 {
                link(&mut d, ENTRY_OBJECT + n, MESSAGES, ENTRY);
            }
            let mut hud = Hud::default();
            hud.read(&d, 0, None);
            Fixture { players, hud }
        }

        fn frame(&mut self, frame: u32, block: &[u8]) {
            self.hud.read(block, 1000 * frame as usize, Some(frame));
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
            resolve(&self.hud, &self.players, &clock, SIDES)
        }
    }

    #[test]
    fn an_id_with_an_argument_names_its_player() {
        let mut f = Fixture::new();
        f.frame(600, &entry(0, 0, Says::Id(LEFT, &[(PLAYER, "p2")])));
        let out = f.resolve();
        let [line] = out.lines.as_slice() else {
            panic!("{:?}", out.lines);
        };
        assert_eq!(line.kind, Kind::PlayerLeft);
        assert_eq!(line.kind_source, Some(KindSource::Inferred));
        assert_eq!(line.message_id.as_deref(), Some("39f4000000000065"));
        assert_eq!(line.text, None);
        assert_eq!(line.username.as_deref(), Some("p2"));
        assert_eq!(line.profile_id.as_deref(), Some("id-2"));
        assert_eq!((line.frame, line.offset), (Some(600), 600_000));
        assert_eq!(line.when.phase, Phase::Action);
        let json = serde_json::to_value(line).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "playerLeft",
                "kindSource": "inferred",
                "messageId": "39f4000000000065",
                "args": [{ "key": "3c7fb10e", "name": "[PLAYER]", "value": "p2" }],
                "username": "p2",
                "profileID": "id-2",
                "backgroundColor": 1,
                "time": "2:59",
                "phase": "Action",
                "elapsed": 47,
                "recordingTime": 60.0,
            })
        );
    }

    #[test]
    fn a_text_that_names_an_attacker_is_a_find() {
        let mut f = Fixture::new();
        f.frame(100, &entry(0, 0, Says::Text("p1 has found the bombs")));
        // A defender's, and a text that names nobody.
        f.frame(200, &entry(1, 0, Says::Text("p3 has found the bombs")));
        f.frame(300, &entry(2, 0, Says::Text("something else")));
        let out = f.resolve();
        let told: Vec<_> = (out.lines.iter())
            .map(|l| (l.kind, l.kind_source, l.username.as_deref()))
            .collect();
        assert_eq!(
            told,
            [
                (Kind::ObjectiveFound, Some(KindSource::Decoded), Some("p1")),
                (Kind::Unknown, None, Some("p3")),
                (Kind::Unknown, None, None),
            ]
        );
        let line = &out.lines[0];
        assert_eq!(line.text.as_deref(), Some("p1 has found the bombs"));
        assert!(line.message_id.is_none() && line.args.is_empty());
        assert_eq!(line.when.recording_time, Some(10.0));
        // A text has no id to name.
        assert!(out.unknown_ids.is_empty());
        assert_eq!(battleye(&out.lines).unknown_messages, 2);
    }

    #[test]
    fn an_unknown_id_is_kept_with_its_bytes() {
        let mut f = Fixture::new();
        let id = [1, 2, 3, 4, 5, 6, 7, 8];
        f.frame(100, &entry(0, 0, Says::Id(id, &[])));
        f.frame(900, &entry(1, 0, Says::Id(id, &[(OTHER, "p0")])));
        let out = f.resolve();
        assert_eq!(out.unknown_ids, ["0102030405060708"]);
        assert_eq!(out.lines.len(), 2);
        assert!(out.lines.iter().all(|l| l.kind == Kind::Unknown));
        assert!(out.lines.iter().all(|l| l.kind_source.is_none()));
        // An argument of no known placeholder is kept and names nobody.
        let args = &out.lines[1].args;
        assert_eq!((args[0].key.as_str(), args[0].name), ("09090909", None));
        assert_eq!(out.lines[1].username, None);
        let flag = battleye(&out.lines);
        assert_eq!((flag.flagged, flag.unknown_messages), (false, 2));
    }

    #[test]
    fn a_scrolling_feed_writes_a_line_once() {
        let mut f = Fixture::new();
        let left = Says::Id(LEFT, &[(PLAYER, "p2")]);
        f.frame(600, &entry(0, 0, left));
        // A kill pushes it down a line: both entries are written again.
        let mut d = entry(0, 0, Says::Kill("p0", "p3"));
        d.extend(entry(1, 1, left));
        f.frame(620, &d);
        // And said again on its line while it is up.
        f.frame(630, &entry(1, 1, left));
        assert_eq!(f.resolve().lines.len(), 1);
        // The same line long after is a new one; kills are none at all.
        f.frame(900, &entry(0, 0, left));
        let out = f.resolve();
        let at: Vec<_> = (out.lines.iter()).map(|l| l.when.recording_time).collect();
        assert_eq!(at, [Some(60.0), Some(90.0)]);
        assert_eq!(of_kind(&out.lines, Kind::PlayerLeft).count(), 2);
    }

    #[test]
    fn only_an_element_of_the_feed_is_a_line() {
        // The same write on an object of another class, under another
        // field, and in the opening snapshot.
        let write = entry(0, 0, Says::Id(LEFT, &[(PLAYER, "p2")]));
        for (field, class) in [(MESSAGES, OTHER), (OTHER, ENTRY)] {
            let mut f = Fixture::new();
            let mut d = vec![];
            link(&mut d, ENTRY_OBJECT, field, class);
            d.extend(&write);
            f.frame(100, &d);
            assert!(f.resolve().lines.is_empty());
        }
        let mut f = Fixture::new();
        f.hud.read(&write, 0, None);
        assert!(f.resolve().lines.is_empty());
        // An entry cleared is no line.
        f.frame(100, &entry(0, 0, Says::Text("")));
        assert!(f.resolve().lines.is_empty());
    }

    #[test]
    fn a_text_that_says_battleye_flags_the_round() {
        let mut f = Fixture::new();
        f.frame(100, &entry(0, 0, Says::Text("p1 has found the bombs")));
        f.frame(200, &entry(1, 0, Says::Text("p0 was removed by BATTLEYE")));
        let out = f.resolve();
        // An attacker is named, yet it is no find.
        assert_eq!(out.lines[1].kind, Kind::Unknown);
        let flag = battleye(&out.lines);
        assert!(flag.flagged);
        assert_eq!(flag.messages, [1]);
        assert_eq!(flag.texts, ["p0 was removed by BATTLEYE"]);
        let rollup = match_battleye(
            [(1, None), (2, Some(&flag)), (3, Some(&BattlEye::default()))].into_iter(),
        );
        assert_eq!(rollup.unwrap().flagged_rounds, [2]);
        assert_eq!(match_battleye([(1, None)].into_iter()), None);
        let legacy = legacy_battleye(vec!["x has been kicked by BattlEye".to_owned()], true);
        assert!(legacy.flagged && legacy.messages.is_empty());
        assert!(!legacy_battleye(Vec::new(), false).flagged);
    }

    #[test]
    fn malformed_writes_are_skipped_and_counted() {
        let mut f = Fixture::new();
        let mut d = vec![];
        // A colour of two bytes and an id of four.
        set(&mut d, ENTRY_OBJECT, BACKGROUND, &[1, 0]);
        prop(&mut d, MESSAGE, &[1, 2, 3, 4]);
        f.frame(100, &d);
        let whole = entry(1, 0, Says::Id(LEFT, &[(PLAYER, "p2")]));
        // Bytes cut anywhere do not panic.
        for cut in 0..whole.len() {
            let mut hud = Hud::default();
            hud.read(&whole[..cut], 0, Some(1));
            let times = [0.0, 1.0];
            let timeline = Timeline::default();
            let clock = Clock {
                timeline: &timeline,
                reading_offsets: &[],
                frame_times: &times,
            };
            resolve(&hud, &f.players, &clock, SIDES);
        }
        let out = f.resolve();
        assert!(out.lines.is_empty());
        assert_eq!(out.warnings.len(), 1);
    }
}
