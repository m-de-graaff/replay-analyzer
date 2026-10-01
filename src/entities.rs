//! The replicated object tree (Y8S1+): which objects belong to which player.
//!
//! Besides the property records the census describes, the stream links
//! objects into a tree:
//!
//! ```text
//! 23 <obj u32> 00000000 <hash> <size> <value>          property of <obj>; <obj> becomes current
//! 22 <hash> <size> <value>                             further property of the current object
//! 26 <hash> <index u32> <size> <value>                 array element of the current object
//! 1b <parent u32> 00000000 <field> <child u32> 00000000 <class>
//!                                                      <child> hangs off <parent>; <parent> becomes current
//! 1a <field> <child u32> 00000000 <class>              <child> hangs off the current object
//! 1e <field> <index u32> <child u32> 00000000 <class>  array slot <index> of <field> holds <child>
//! ```
//!
//! A child id of 0 is an empty slot. Each player has one *controller*
//! object (Ubisoft's class hash `2759e897`) under their team's object. It
//! holds the name, operator, Ubisoft profile id and the header `playerid`,
//! and parents the player's other objects: scoreboard, health, inventory,
//! observer, and a profile object with name, level and how the player
//! relates to whoever recorded.
//!
//! Movement is sent apart from the tree, as `<obj u32> 00000000 <size u32>
//! 607385fe ...` messages. Which moving object a player controls is written
//! in a player table: `<count u8>` then one entry per player, each starting
//! with the header `playerid` and, when it changed, the object the player now
//! controls. Tables are messages framed `<frame u32> <size u32> 00000000`, so
//! each change has a frame in the frame index.

use std::collections::HashMap;

use aho_corasick::AhoCorasick;

/// A property hash as it appears in the stream.
type Hash = [u8; 4];

const PLAYER_ID: Hash = [0xEE, 0xD4, 0x45, 0xC8];
const NAME: Hash = [0x07, 0x94, 0x9B, 0xDC];
const PROFILE_ID: Hash = [0x8A, 0x50, 0x9B, 0xD0];
const TEAM: Hash = [0x95, 0x1C, 0x16, 0x50];
/// Controller property that is 1 while the player's weapon is up and 0 while
/// it is not (attackers start prep at 0, on their drones).
pub const WEAPON_READY: Hash = [0xA4, 0xDC, 0x8D, 0xD4];
/// Controller -> scoreboard object.
const SCOREBOARD_FIELD: Hash = [0xEB, 0x21, 0x9B, 0x38];
/// Controller -> health object.
const HEALTH_FIELD: Hash = [0x41, 0x54, 0xDC, 0xC4];
/// Controller -> profile object.
const PROFILE_FIELD: Hash = [0x77, 0xB1, 0x5E, 0x33];
/// Profile: the player as seen by whoever recorded: 1 opponent, 2 teammate,
/// 3 teammate in the recorder's party, 5 the recorder.
const RELATION: Hash = [0x05, 0xC7, 0xB9, 0x49];
/// Profile: role in the recorder's party: 0 none, 1 member, 2 leader.
const PARTY_ROLE: Hash = [0xAF, 0x6B, 0xB2, 0x87];
/// Team object: the game's number for the team, 1 or 2 (`TeamColor`; every
/// hash is the CRC-32 of the game's property name). Ban slots and
/// `matchresult` name teams by it.
const TEAM_COLOR: Hash = [0x2E, 0x61, 0xA2, 0xA9];

/// Y11S3 ban manager: one array of three ban slots per team.
const BAN_SLOT_ARRAYS: [Hash; 2] = [[0x56, 0x1E, 0x4C, 0x23], [0xE6, 0x37, 0x2C, 0x1E]];
/// Ban slot: the side of the operator it bans, 1 attack, 2 defense
/// (`HeroTeam`).
const HERO_TEAM: Hash = [0x18, 0xFF, 0xCA, 0x5E];
/// Ban slot: 0 not used yet, 3 resolved (`BanState`).
const BAN_STATE: Hash = [0x60, 0x39, 0xD5, 0xB5];
/// Ban slot: 1 an operator was banned, 2 the vote ended without a ban
/// (`ResultType`).
const RESULT_TYPE: Hash = [0x32, 0x3D, 0xDC, 0xAA];
/// Ban slot -> the banned operator's descriptor (`Operator`), 0 when none.
const OPERATOR: Hash = [0xD7, 0xC5, 0xD0, 0x2E];
/// Operator descriptor -> its info object (`OperatorInfo`).
const OPERATOR_INFO: Hash = [0x32, 0x83, 0xB2, 0x1A];
/// Operator info: the operator's icon, the id `roleimage` uses (`BadgeIcon`).
const BADGE_ICON: Hash = [0xDA, 0x69, 0x14, 0xD5];

/// How far into the stream the opening snapshot of the tree can reach.
const SNAPSHOT_BYTES: usize = 4 << 20;

#[derive(Clone, Debug, Default)]
struct Object {
    /// First value of each property, as a range into the data.
    props: Vec<(Hash, usize, usize)>,
    /// `(field, child)` links, in stream order.
    children: Vec<(Hash, u32)>,
    /// `(field, index, child, offset)` of each array slot (`1e` records).
    elements: Vec<(Hash, u32, u32, usize)>,
}

/// The objects of the opening snapshot.
#[derive(Debug, Default)]
struct Tree<'a> {
    data: &'a [u8],
    objects: HashMap<u32, Object>,
}

impl<'a> Tree<'a> {
    fn prop(&self, obj: u32, hash: Hash) -> Option<&'a [u8]> {
        let o = self.objects.get(&obj)?;
        o.props
            .iter()
            .find(|p| p.0 == hash)
            .map(|&(_, from, to)| &self.data[from..to])
    }

    fn u32(&self, obj: u32, hash: Hash) -> Option<u32> {
        Some(u32::from_le_bytes(self.prop(obj, hash)?.try_into().ok()?))
    }

    fn u64(&self, obj: u32, hash: Hash) -> Option<u64> {
        Some(u64::from_le_bytes(self.prop(obj, hash)?.try_into().ok()?))
    }

    fn text(&self, obj: u32, hash: Hash) -> Option<String> {
        let v = self.prop(obj, hash)?;
        (!v.is_empty()).then(|| String::from_utf8_lossy(v).into_owned())
    }

    fn child(&self, obj: u32, field: Hash) -> Option<u32> {
        let o = self.objects.get(&obj)?;
        o.children.iter().find(|c| c.0 == field).map(|c| c.1)
    }
}

fn u32_at(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

fn hash_at(d: &[u8], i: usize) -> Option<Hash> {
    d.get(i..i + 4)?.try_into().ok()
}

fn zero4(d: &[u8], i: usize) -> bool {
    d.get(i..i + 4) == Some(&[0; 4])
}

enum Record {
    /// `23`: object, hash, value range.
    Set(u32, Hash, usize, usize),
    /// `22` or `26`: hash, value range.
    Prop(Hash, usize, usize),
    /// `1b`: parent, field, child.
    ParentChild(u32, Hash, u32),
    /// `1a`: field, child.
    Child(Hash, u32),
    /// `1e`: field, array index, child.
    Element(Hash, u32, u32),
}

/// Array indices are small (64 at most in real rounds). Binary data between
/// runs can read as an array record whose value ends on a real record, which
/// would swallow the records it covers; its "index" gives it away.
const MAX_INDEX: u32 = 1 << 16;

/// The record at `i` and where the next one starts.
fn record(d: &[u8], i: usize) -> Option<(Record, usize)> {
    let index_ok = |at: usize| u32_at(d, at).is_some_and(|x| x < MAX_INDEX);
    let value = |at: usize| -> Option<(usize, usize, usize)> {
        let size = *d.get(at)? as usize;
        let end = at + 1 + size;
        (end <= d.len()).then_some((at + 1, end, end))
    };
    match *d.get(i)? {
        0x23 if zero4(d, i + 5) => {
            let (from, to, next) = value(i + 13)?;
            Some((
                Record::Set(u32_at(d, i + 1)?, hash_at(d, i + 9)?, from, to),
                next,
            ))
        }
        0x22 => {
            let (from, to, next) = value(i + 5)?;
            Some((Record::Prop(hash_at(d, i + 1)?, from, to), next))
        }
        0x26 if index_ok(i + 5) => {
            let (from, to, next) = value(i + 9)?;
            Some((Record::Prop(hash_at(d, i + 1)?, from, to), next))
        }
        0x1a if zero4(d, i + 9) && i + 17 <= d.len() => {
            Some((Record::Child(hash_at(d, i + 1)?, u32_at(d, i + 5)?), i + 17))
        }
        0x1b if zero4(d, i + 5) && zero4(d, i + 17) && i + 25 <= d.len() => Some((
            Record::ParentChild(u32_at(d, i + 1)?, hash_at(d, i + 9)?, u32_at(d, i + 13)?),
            i + 25,
        )),
        0x1e if zero4(d, i + 13) && i + 21 <= d.len() && index_ok(i + 5) => Some((
            Record::Element(hash_at(d, i + 1)?, u32_at(d, i + 5)?, u32_at(d, i + 9)?),
            i + 21,
        )),
        _ => None,
    }
}

/// Reads the records in `data`. Between runs of records sit binary blocks;
/// a run counts when it has two records or starts by naming its object.
fn tree(data: &[u8]) -> Tree<'_> {
    let mut t = Tree {
        data,
        objects: HashMap::new(),
    };
    let mut current: Option<u32> = None;
    let mut i = 0;
    while i < data.len() {
        let Some((first, mut next)) = record(data, i) else {
            i += 1;
            continue;
        };
        let first_end = next;
        let names_object = names_its_object(&first);
        let mut run = vec![(i, first)];
        while let Some((r, n)) = record(data, next) {
            // A run can also carry on into binary data that reads as records.
            if !names_its_object(&r) && swallows_an_object(data, next, n) {
                break;
            }
            run.push((next, r));
            next = n;
        }
        if !names_object && (run.len() < 2 || swallows_an_object(data, i, first_end)) {
            i += 1;
            continue;
        }
        for (at, r) in run {
            match r {
                Record::Set(obj, hash, from, to) => {
                    current = Some(obj);
                    add_prop(&mut t, obj, hash, from, to);
                }
                Record::Prop(hash, from, to) => {
                    if let Some(obj) = current {
                        add_prop(&mut t, obj, hash, from, to);
                    }
                }
                Record::ParentChild(parent, field, child) => {
                    current = Some(parent);
                    if child != 0 {
                        t.objects
                            .entry(parent)
                            .or_default()
                            .children
                            .push((field, child));
                    }
                }
                Record::Child(field, child) => {
                    if let (Some(parent), true) = (current, child != 0) {
                        t.objects
                            .entry(parent)
                            .or_default()
                            .children
                            .push((field, child));
                    }
                }
                Record::Element(field, index, child) => {
                    if let (Some(parent), true) = (current, child != 0) {
                        let o = t.objects.entry(parent).or_default();
                        o.children.push((field, child));
                        o.elements.push((field, index, child, at));
                    }
                }
            }
        }
        i = next;
    }
    t
}

/// Whether the record from `start` to `end`, which does not name its object,
/// covers records that do (a `23` or `1b` and what follows it) ending exactly
/// where it ends: binary data that reads as a record and swallows real ones,
/// whose properties would then land on the previous object.
fn swallows_an_object(d: &[u8], start: usize, end: usize) -> bool {
    // A record's header and the smallest `23` record it could cover.
    if end - start < 6 + 14 {
        return false;
    }
    memchr::memchr2_iter(0x23, 0x1B, &d[start + 1..end])
        .map(|k| start + 1 + k)
        .any(|j| {
            record(d, j).is_some_and(|(r, _)| names_its_object(&r)) && records_end_at(d, j, end)
        })
}

/// `23` and `1b` records say which object they belong to; the others belong
/// to the current one.
fn names_its_object(r: &Record) -> bool {
    matches!(r, Record::Set(..) | Record::ParentChild(..))
}

/// Whether records read from `at` end exactly at `end`.
fn records_end_at(d: &[u8], mut at: usize, end: usize) -> bool {
    while at < end {
        match record(d, at) {
            Some((_, next)) => at = next,
            None => return false,
        }
    }
    at == end
}

fn add_prop(t: &mut Tree, obj: u32, hash: Hash, from: usize, to: usize) {
    let o = t.objects.entry(obj).or_default();
    if !o.props.iter().any(|p| p.0 == hash) {
        o.props.push((hash, from, to));
    }
}

/// A player's objects and what the snapshot says about them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerObjects {
    /// The header `playerid`, written on the controller.
    pub player_id: u64,
    pub username: String,
    pub profile_id: String,
    /// The controller object: identity, operator, team and per-player state
    /// such as the weapon-ready flag. Pick and swap packets write to it.
    pub controller: u32,
    /// The player's team object.
    pub team_object: Option<u32>,
    /// The team object's `TeamColor`: 1 or 2.
    pub team_color: Option<u32>,
    pub scoreboard: Option<u32>,
    pub health: Option<u32>,
    /// Raw relation to the recorder: 1 opponent, 2 teammate, 3 in the
    /// recorder's party (every lobby member in custom games), 5 the recorder.
    pub relation: Option<u32>,
    /// Raw party role: 0 none, 1 member, 2 leader.
    pub party_role: Option<u32>,
    /// Weapon-ready flag at the start of the recording.
    pub weapon_ready: Option<bool>,
}

/// A Y11S3 ban slot. Each team has three, filled in the order it bans:
/// ranked teams fill one per round of a half, and overtime reuses a half's
/// bans. Written once, in the opening snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BanSlot {
    /// Position in the team's list, from 0.
    pub index: u32,
    /// Side of the banned operator: 1 attack, 2 defense.
    pub side: u32,
    /// `TeamColor` of the team that bans in this slot.
    pub color: u32,
    /// 0 not used yet, 3 resolved.
    pub state: u32,
    /// 1 an operator was banned, 2 the vote ended without a ban.
    pub result: u32,
    /// The banned operator's icon, the id `roleimage` uses.
    pub icon: Option<u64>,
}

impl BanSlot {
    /// The team has used the slot: banned an operator or voted for none.
    pub fn resolved(&self) -> bool {
        self.state == 3
    }

    /// The team's vote ended without a ban.
    pub fn no_ban(&self) -> bool {
        self.result == 2
    }
}

/// What the opening snapshot of the object tree says.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Every player's objects, in stream order.
    pub players: Vec<PlayerObjects>,
    /// Y11S3 ban slots in stream order: per team, by index.
    pub ban_slots: Vec<BanSlot>,
}

/// The part of `body` the opening snapshot can be in. Y11S3 writes the
/// snapshot before the movement stream; older replays interleave the two,
/// and their snapshot fits in the opening bytes.
fn snapshot_bytes(body: &[u8]) -> &[u8] {
    let end = first_movement(body)
        .unwrap_or(body.len())
        .min(SNAPSHOT_BYTES);
    &body[..end]
}

/// Reads the opening snapshot of the object tree.
pub fn snapshot(body: &[u8]) -> Snapshot {
    let t = tree(snapshot_bytes(body));
    Snapshot {
        players: player_objects(&t),
        ban_slots: ban_slots(&t),
    }
}

/// Every player's objects in the opening snapshot, in stream order.
pub fn players(body: &[u8]) -> Vec<PlayerObjects> {
    player_objects(&tree(snapshot_bytes(body)))
}

/// The slots of the ban manager's arrays. The game sometimes sends the
/// slots a second time with new object ids; the first copy is kept.
fn ban_slots(t: &Tree) -> Vec<BanSlot> {
    let mut elements: Vec<(usize, u32, u32)> = t
        .objects
        .values()
        .flat_map(|o| &o.elements)
        .filter(|e| BAN_SLOT_ARRAYS.contains(&e.0))
        .map(|&(_, index, slot, at)| (at, index, slot))
        .collect();
    elements.sort_unstable();
    let mut out: Vec<BanSlot> = Vec::new();
    for (_, index, slot) in elements {
        // A side or team missing is 0, which the caller reports for a used
        // slot rather than dropping it here.
        let slot = BanSlot {
            index,
            side: t.u32(slot, HERO_TEAM).unwrap_or(0),
            color: t.u32(slot, TEAM_COLOR).unwrap_or(0),
            state: t.u32(slot, BAN_STATE).unwrap_or(0),
            result: t.u32(slot, RESULT_TYPE).unwrap_or(0),
            icon: t
                .child(slot, OPERATOR)
                .and_then(|op| t.child(op, OPERATOR_INFO))
                .and_then(|info| t.u64(info, BADGE_ICON))
                .filter(|&icon| icon != 0),
        };
        match out
            .iter_mut()
            .find(|s| s.color == slot.color && s.index == slot.index)
        {
            // A copy sent again: keep the one that has been used.
            Some(seen) if !seen.resolved() && slot.resolved() => *seen = slot,
            Some(_) => {}
            None => out.push(slot),
        }
    }
    out
}

fn player_objects(t: &Tree) -> Vec<PlayerObjects> {
    let mut out: Vec<(usize, PlayerObjects)> = t
        .objects
        .iter()
        .filter_map(|(&obj, o)| {
            let first = o.props.iter().find(|p| p.0 == PLAYER_ID)?.1;
            let profile = t.child(obj, PROFILE_FIELD);
            let team_object = t.u64(obj, TEAM).map(|v| v as u32).filter(|&v| v != 0);
            Some((
                first,
                PlayerObjects {
                    player_id: t.u64(obj, PLAYER_ID)?,
                    username: t.text(obj, NAME).unwrap_or_default(),
                    profile_id: t.text(obj, PROFILE_ID).unwrap_or_default(),
                    controller: obj,
                    team_object,
                    team_color: team_object.and_then(|team| t.u32(team, TEAM_COLOR)),
                    scoreboard: t.child(obj, SCOREBOARD_FIELD),
                    health: t.child(obj, HEALTH_FIELD),
                    relation: profile.and_then(|p| t.u32(p, RELATION)),
                    party_role: profile.and_then(|p| t.u32(p, PARTY_ROLE)),
                    weapon_ready: t.prop(obj, WEAPON_READY).and_then(|v| match v {
                        [0] => Some(false),
                        [1] => Some(true),
                        _ => None,
                    }),
                },
            ))
        })
        .collect();
    out.sort_by_key(|(at, _)| *at);
    out.into_iter().map(|(_, p)| p).collect()
}

/// How a player relates to whoever recorded the replay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Relation {
    You,
    Teammate,
    Opponent,
}

/// One change of the object a player controls.
#[derive(Clone, Debug, PartialEq)]
pub struct Possession {
    pub player_id: u64,
    /// Frame of the table message; `None` for the table that opens the
    /// stream, which is written before the first frame.
    pub frame: Option<u32>,
    /// The body the player moves: their operator.
    pub body: Option<u32>,
    /// What the player looks through instead, when not their body: a drone
    /// or camera.
    pub view: Option<u32>,
}

/// Reads every player table in `body`: each change of the body or view a
/// player controls. `ids` are the header `playerid`s.
pub fn possessions(body: &[u8], ids: &[u64]) -> Vec<Possession> {
    let ids: Vec<u64> = ids.iter().copied().filter(|&i| i != 0).collect();
    if ids.len() < 2 {
        return Vec::new();
    }
    // Tables come before the movement stream starts; without one, they are
    // in the opening part of the stream if anywhere.
    let body = &body[..first_movement(body).unwrap_or(body.len().min(SNAPSHOT_BYTES))];
    let patterns: Vec<[u8; 8]> = ids.iter().map(|i| i.to_le_bytes()).collect();
    let Ok(ac) = AhoCorasick::new(&patterns) else {
        return Vec::new();
    };
    let hits: Vec<(usize, u64)> = ac
        .find_iter(body)
        .map(|m| (m.start(), ids[m.pattern().as_usize()]))
        .collect();
    // A table starts with its entry count and lists every player a few bytes
    // apart. Tables can follow each other closely, so the count delimits
    // them.
    const GAP: usize = 40;
    let mut out = Vec::new();
    let mut start = 0;
    while start < hits.len() {
        let n = hits[start]
            .0
            .checked_sub(1)
            .map_or(0, |i| usize::from(body[i]));
        let end = start + n;
        let is_table = n >= ids.len().min(8)
            && n <= ids.len()
            && end <= hits.len()
            && (start + 1..end).all(|i| hits[i].0 - hits[i - 1].0 <= GAP);
        if !is_table {
            start += 1;
            continue;
        }
        let table = &hits[start..end];
        {
            let frame = frame_before(body, table[0].0 - 1);
            for (i, &(at, player_id)) in table.iter().enumerate() {
                let entry_end = table
                    .get(i + 1)
                    .map_or(at + 8 + GAP, |n| n.0)
                    .min(body.len());
                let (body_obj, view) = entry_objects(&body[at + 8..entry_end]);
                tracing::trace!(at, player_id, entry = ?&body[at + 8..entry_end.min(at + 30)], "table entry");
                if body_obj.is_some() || view.is_some() {
                    out.push(Possession {
                        player_id,
                        frame,
                        body: body_obj,
                        view,
                    });
                }
            }
        }
        start = end;
    }
    out
}

/// Offset of the first movement message: `<obj> 00000000 <size u32>
/// 607385fe`.
fn first_movement(body: &[u8]) -> Option<usize> {
    memchr::memmem::find_iter(body, &MOVE)
        .find(|&at| at >= 12 && object_at(body, at - 12).is_some())
        .map(|at| at - 12)
}

/// Class hash of a movement message.
const MOVE: [u8; 4] = [0x60, 0x73, 0x85, 0xFE];

/// The frame in the `<frame u32> <size u32> 00000000` message header ending
/// at `count_at` (where the table's count byte sits).
fn frame_before(d: &[u8], count_at: usize) -> Option<u32> {
    let head = count_at.checked_sub(12)?;
    let size = u32_at(d, head + 4)? as usize;
    (zero4(d, head + 8) && size > 0 && size < 1 << 16)
        .then(|| u32_at(d, head))
        .flatten()
}

/// An object reference: `<id u32> 00000000` with the id in the `f0xxxxxx`
/// range objects are numbered in.
fn object_at(seg: &[u8], k: usize) -> Option<u32> {
    let id = u32_at(seg, k)?;
    (id >> 24 == 0xF0 && zero4(seg, k + 4)).then_some(id)
}

/// The objects in one table entry after its `playerid`: a field mask, then
/// a flag byte (`04` the body changed, `01` the view changed), a few small
/// values up to `ff`, then the references: a body as `<id> 00000000`
/// (sometimes after two floats), a view as `<slot u8> [04] <id> 00000000`.
fn entry_objects(seg: &[u8]) -> (Option<u32>, Option<u32>) {
    let Some(&flags) = seg.get(1) else {
        return (None, None);
    };
    let Some(ff) = seg.iter().take(8).position(|&b| b == 0xFF) else {
        return (None, None);
    };
    // The first reference comes within 10 bytes of the `ff`, a second one
    // (a view after a body) within 2 bytes of the first.
    let find = |from: usize, slack: usize| {
        (from..=from + slack).find_map(|k| Some((k, object_at(seg, k)?)))
    };
    let first = find(ff + 1, 10);
    let second = first.and_then(|(k, _)| find(k + 8, 2));
    let mut refs = [first, second].into_iter().flatten().map(|(_, id)| id);
    let body = if flags & 0x04 != 0 { refs.next() } else { None };
    let view = if flags & 0x01 != 0 { refs.next() } else { None };
    (body, view)
}

/// Where each of `objs` was created, from the creation messages
/// `<obj> 00000000 <size u32> 617385fe <obj> 00000000 <8 bytes> <x f32>
/// <y f32> <z f32>`, in map coordinates. One pass over `body`.
pub fn spawn_positions(body: &[u8], objs: &[u32]) -> HashMap<u32, [f32; 3]> {
    let mut out = HashMap::new();
    if objs.is_empty() {
        return out;
    }
    for msg in memchr::memmem::find_iter(body, &CREATE) {
        let Some(obj) = msg.checked_sub(12).and_then(|at| object_at(body, at)) else {
            continue;
        };
        if !objs.contains(&obj) || out.contains_key(&obj) || object_at(body, msg + 4) != Some(obj) {
            continue;
        }
        let f = |i: usize| -> Option<f32> {
            let at = msg + 16 + 4 * i;
            Some(f32::from_le_bytes(body.get(at..at + 4)?.try_into().ok()?))
        };
        if let (Some(x), Some(y), Some(z)) = (f(0), f(1), f(2))
            && [x, y, z].iter().all(|v| v.is_finite() && v.abs() < 1e5)
        {
            out.insert(obj, [x, y, z]);
            if out.len() == objs.len() {
                break;
            }
        }
    }
    out
}

/// Class hash of a moving object's creation message.
const CREATE: [u8; 4] = [0x61, 0x73, 0x85, 0xFE];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tree_links_and_current_object() {
        let mut d = vec![];
        // 23 obj=1 name, 22 id
        d.extend([0x23, 1, 0, 0, 0xF0, 0, 0, 0, 0]);
        d.extend(NAME);
        d.extend([3, b'a', b'b', b'c']);
        d.push(0x22);
        d.extend(PLAYER_ID);
        d.push(8);
        d.extend(7u64.to_le_bytes());
        // 1a scoreboard child 2 of the current object
        d.push(0x1A);
        d.extend(SCOREBOARD_FIELD);
        d.extend([2, 0, 0, 0xF0, 0, 0, 0, 0]);
        d.extend([9, 9, 9, 9]);
        let p = players(&d);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].username, "abc");
        assert_eq!(p[0].player_id, 7);
        assert_eq!(p[0].scoreboard, Some(0xF000_0002));
    }

    /// `23 <obj> 00000000 <hash> <size> <value>`: a property that makes
    /// `obj` the current object.
    fn set(d: &mut Vec<u8>, obj: u32, hash: Hash, value: &[u8]) {
        d.push(0x23);
        d.extend(obj.to_le_bytes());
        d.extend([0; 4]);
        d.extend(hash);
        d.push(value.len() as u8);
        d.extend(value);
    }

    /// `22 <hash> <size> <value>`: a property of the current object.
    fn prop(d: &mut Vec<u8>, hash: Hash, value: &[u8]) {
        d.push(0x22);
        d.extend(hash);
        d.push(value.len() as u8);
        d.extend(value);
    }

    /// `1b <parent> 00000000 <field> <child> 00000000 <class>`.
    fn link(d: &mut Vec<u8>, parent: u32, field: Hash, child: u32) {
        d.push(0x1B);
        d.extend(parent.to_le_bytes());
        d.extend([0; 4]);
        d.extend(field);
        d.extend(child.to_le_bytes());
        d.extend([0; 4]);
        d.extend([7; 4]);
    }

    /// `1a <field> <child> 00000000 <class>`: a child of the current object.
    fn child(d: &mut Vec<u8>, field: Hash, child: u32) {
        d.push(0x1A);
        d.extend(field);
        d.extend(child.to_le_bytes());
        d.extend([0; 4]);
        d.extend([7; 4]);
    }

    /// `1e <field> <index> <child> 00000000 <class>`: an array slot of the
    /// current object.
    fn element(d: &mut Vec<u8>, field: Hash, index: u32, child: u32) {
        d.push(0x1E);
        d.extend(field);
        d.extend(index.to_le_bytes());
        d.extend(child.to_le_bytes());
        d.extend([0; 4]);
        d.extend([7; 4]);
    }

    /// A ban slot's side, team color, state and result, as the game writes
    /// them after the slot's operator.
    fn slot_props(d: &mut Vec<u8>, slot: u32, side: u32, state: u32, result: u32) {
        set(d, slot, HERO_TEAM, &side.to_le_bytes());
        prop(d, TEAM_COLOR, &1u32.to_le_bytes());
        prop(d, BAN_STATE, &state.to_le_bytes());
        prop(d, RESULT_TYPE, &result.to_le_bytes());
    }

    #[test]
    fn reads_ban_slots_in_order_with_their_operator_icons() {
        let (manager, banned, skipped, unused) =
            (0xF000_0100, 0xF000_0101, 0xF000_0102, 0xF000_0103);
        let (operator, info) = (0xF000_0110, 0xF000_0111);
        let mut d = vec![];
        set(&mut d, manager, BAN_SLOT_ARRAYS[0], &[1]);
        element(&mut d, BAN_SLOT_ARRAYS[0], 0, banned);
        element(&mut d, BAN_SLOT_ARRAYS[0], 1, skipped);
        element(&mut d, BAN_SLOT_ARRAYS[0], 2, unused);
        // Slot 0 banned Mira; slot 1's vote ended without a ban; slot 2 is
        // not used yet.
        link(&mut d, banned, OPERATOR, operator);
        set(&mut d, operator, [0x0E, 0x9E, 0xBE, 0x88], &[0; 8]);
        child(&mut d, OPERATOR_INFO, info);
        set(&mut d, info, BADGE_ICON, &39149215445u64.to_le_bytes());
        slot_props(&mut d, banned, 2, 3, 1);
        link(&mut d, skipped, OPERATOR, 0);
        slot_props(&mut d, skipped, 2, 3, 2);
        link(&mut d, unused, OPERATOR, 0);
        slot_props(&mut d, unused, 2, 0, 0);

        let slots = snapshot(&d).ban_slots;

        let slot = |index, state, result, icon| BanSlot {
            index,
            side: 2,
            color: 1,
            state,
            result,
            icon,
        };
        assert_eq!(
            slots,
            [
                slot(0, 3, 1, Some(39149215445)),
                slot(1, 3, 2, None),
                slot(2, 0, 0, None),
            ]
        );
    }

    #[test]
    fn a_resent_ban_slot_that_is_resolved_wins_over_an_unused_copy() {
        let (unused, resolved) = (0xF000_0101, 0xF000_0201);
        let (operator, info) = (0xF000_0210, 0xF000_0211);
        let mut d = vec![];
        set(&mut d, 0xF000_0100, BAN_SLOT_ARRAYS[0], &[1]);
        element(&mut d, BAN_SLOT_ARRAYS[0], 0, unused);
        link(&mut d, unused, OPERATOR, 0);
        slot_props(&mut d, unused, 2, 0, 0);
        // The same slot sent again with new object ids, now used.
        set(&mut d, 0xF000_0200, BAN_SLOT_ARRAYS[0], &[1]);
        element(&mut d, BAN_SLOT_ARRAYS[0], 0, resolved);
        link(&mut d, resolved, OPERATOR, operator);
        set(&mut d, operator, [0x0E, 0x9E, 0xBE, 0x88], &[0; 8]);
        child(&mut d, OPERATOR_INFO, info);
        set(&mut d, info, BADGE_ICON, &39149215445u64.to_le_bytes());
        slot_props(&mut d, resolved, 2, 3, 1);

        let slots = snapshot(&d).ban_slots;

        let expected = BanSlot {
            index: 0,
            side: 2,
            color: 1,
            state: 3,
            result: 1,
            icon: Some(39149215445),
        };
        assert_eq!(slots, [expected]);
    }

    #[test]
    fn a_stray_property_record_does_not_swallow_the_next_object() {
        let team: u32 = 0xF000_0010;
        let mut d = vec![];
        set(&mut d, 1, PLAYER_ID, &7u64.to_le_bytes());
        prop(&mut d, TEAM, &u64::from(team).to_le_bytes());
        // Bytes that read as a property whose value is the team object's
        // first record, ending where the team's color is written.
        let mut team_record = vec![];
        set(&mut team_record, team, [0x10, 0x9F, 0x20, 0x30], &[0]);
        prop(&mut d, [0xCA, 0x8C, 0xD8, 0x71], &team_record);
        prop(&mut d, TEAM_COLOR, &1u32.to_le_bytes());

        let p = players(&d);

        assert_eq!(p[0].team_color, Some(1));
    }

    #[test]
    fn a_stray_array_record_does_not_swallow_the_next_object() {
        let team: u32 = 0xF000_0010;
        let mut d = vec![];
        // A controller with its player id and team object.
        d.extend([0x23, 1, 0, 0, 0xF0, 0, 0, 0, 0]);
        d.extend(PLAYER_ID);
        d.push(8);
        d.extend(7u64.to_le_bytes());
        d.push(0x22);
        d.extend(TEAM);
        d.push(8);
        d.extend(u64::from(team).to_le_bytes());
        // Bytes that read as an array element with an impossible index, whose
        // value covers the team object's first record and ends where its
        // color is written (seen in a real round at offset 2774).
        let mut team_record = vec![0x23];
        team_record.extend(team.to_le_bytes());
        team_record.extend([0; 4]);
        team_record.extend([0x10, 0x9F, 0x20, 0x30, 1, 0]);
        d.push(0x26);
        d.extend([0xCA, 0x8C, 0xD8, 0x71]);
        d.extend(0x69A8_FC11u32.to_le_bytes());
        d.push(team_record.len() as u8);
        d.extend(&team_record);
        d.push(0x22);
        d.extend(TEAM_COLOR);
        d.push(4);
        d.extend(1u32.to_le_bytes());

        let p = players(&d);

        assert_eq!(p[0].team_color, Some(1));
    }

    #[test]
    fn entry_objects_tells_body_from_view() {
        let body = [
            0x39, 0x04, 0x04, 0x00, 0xFF, 0xEF, 0x8A, 0x2B, 0xF0, 0, 0, 0, 0,
        ];
        assert_eq!(entry_objects(&body), (Some(0xF02B_8AEF), None));
        let view = [
            0xE1, 0x01, 0xFF, 0x0B, 0x04, 0xF3, 0x8B, 0x2B, 0xF0, 0, 0, 0, 0,
        ];
        assert_eq!(entry_objects(&view), (None, Some(0xF02B_8BF3)));
        let floats = [
            0x39, 0x14, 0x04, 0x00, 0xFF, 0xB6, 0x2C, 0x36, 0xBF, 0x8B, 0xD0, 0x8A, 0xBE, 0xB8,
            0xB6, 0x28, 0xF0, 0, 0, 0, 0,
        ];
        assert_eq!(entry_objects(&floats), (Some(0xF028_B6B8), None));
        assert_eq!(entry_objects(&[0x21, 0x00, 0xFF]), (None, None));
        // A view switch without the `04` marker is still a view.
        let switch = [0x61, 0x01, 0xFF, 0x0A, 0xC0, 0x08, 0x37, 0xF0, 0, 0, 0, 0];
        assert_eq!(entry_objects(&switch), (None, Some(0xF037_08C0)));
        // A following message's object is not this entry's.
        let mut tail = body.to_vec();
        tail.extend([
            0x96, 0x79, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0x7C, 0x36, 0x2A, 0xF0, 0, 0, 0, 0,
        ]);
        assert_eq!(entry_objects(&tail), (Some(0xF02B_8AEF), None));
    }
}
