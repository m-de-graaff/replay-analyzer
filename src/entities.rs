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
//! in a player table, a stream of its own in Y11S3
//! ([`PLAYER_TABLE_STREAM`]): its snapshot is the table that opens the round
//! and each of its frame records is a table of what changed since.
//!
//! ```text
//! table:  <count u8>, count x entry
//! entry:  <playerid u64> <mask u8> <flags u8>
//!         one byte for each of the mask bits 08 10 20 40 80
//!         flags 01:  <object u64>              what the player looks through
//!         flags 10:  <f32> <f32>
//!         flags 08:  <playerid u64>            another player, or ff..ff
//!         flags 04:  <object u64>              the body the player moves
//!         flags 20:  <n u32>, n x { <playerid u64> <u8> }
//! ```
//!
//! An object of 0 takes the body or view away. The `playerid` is the one the
//! controller holds, and every player in the round has an entry: the
//! recorder, players the header gives no usable id for, and players who join
//! after the header was written. A table is as long as its record says, to
//! the byte, which is how the layout above was checked against real rounds.

use std::collections::HashMap;

use aho_corasick::AhoCorasick;

/// A property hash as it appears in the stream.
pub(crate) type Hash = [u8; 4];

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
/// Controller: the platform the player is on (`PlayerPlatform`): 0 PC,
/// 5 PlayStation, 7 Xbox. 12 marks a seat whose player left.
const PLATFORM: Hash = [0x7D, 0xD4, 0xFC, 0x18];
/// Profile: 1 when the name shown is a nickname standing in for the
/// player's own (`UsesNickname`).
const USES_NICKNAME: Hash = [0xFE, 0xAC, 0xF1, 0x7B];
/// Profile: the player's name, as an array whose first element holds it.
/// Written when the profile object is created, never on a rename.
const PROFILE_NAMES: Hash = [0xEE, 0xB0, 0xB9, 0xBD];
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

pub(crate) fn u32_at(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

fn hash_at(d: &[u8], i: usize) -> Option<Hash> {
    d.get(i..i + 4)?.try_into().ok()
}

fn zero4(d: &[u8], i: usize) -> bool {
    d.get(i..i + 4) == Some(&[0; 4])
}

pub(crate) enum Record {
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
pub(crate) fn record(d: &[u8], i: usize) -> Option<(Record, usize)> {
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

/// Calls `visit` with the offset of every record in `data`, in order.
/// Between runs of records sit binary blocks; a run counts when it has two
/// records or starts by naming its object.
pub(crate) fn for_each_record(data: &[u8], mut visit: impl FnMut(usize, Record)) {
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
            visit(at, r);
        }
        i = next;
    }
}

/// Reads the records in `data` into a tree.
fn tree(data: &[u8]) -> Tree<'_> {
    let mut t = Tree {
        data,
        objects: HashMap::new(),
    };
    let mut current: Option<u32> = None;
    for_each_record(data, |at, r| match r {
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
    });
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
    /// The team object's `HeroTeam`: 1 attack, 2 defense.
    pub team_side: Option<u32>,
    pub scoreboard: Option<u32>,
    pub health: Option<u32>,
    /// Raw relation to the recorder: 1 opponent, 2 teammate, 3 in the
    /// recorder's party (every lobby member in custom games), 5 the recorder.
    pub relation: Option<u32>,
    /// Raw party role: 0 none, 1 member, 2 leader.
    pub party_role: Option<u32>,
    /// Weapon-ready flag at the start of the recording.
    pub weapon_ready: Option<bool>,
    /// Raw `PlayerPlatform`: 0 PC, 5 PlayStation, 7 Xbox.
    pub platform: Option<u32>,
    /// The name is a nickname the game shows in place of the player's own
    /// (`UsesNickname`).
    pub uses_nickname: bool,
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
                    team_side: team_object.and_then(|team| t.u32(team, HERO_TEAM)),
                    scoreboard: t.child(obj, SCOREBOARD_FIELD),
                    health: t.child(obj, HEALTH_FIELD),
                    relation: profile.and_then(|p| t.u32(p, RELATION)),
                    party_role: profile.and_then(|p| t.u32(p, PARTY_ROLE)),
                    weapon_ready: t.prop(obj, WEAPON_READY).and_then(|v| match v {
                        [0] => Some(false),
                        [1] => Some(true),
                        _ => None,
                    }),
                    platform: t.u32(obj, PLATFORM),
                    uses_nickname: profile.and_then(|p| t.prop(p, USES_NICKNAME)) == Some(&[1][..]),
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

/// Name hash, in file byte order, of the stream that holds the player table
/// (Y11S3).
pub const PLAYER_TABLE_STREAM: [u8; 4] = [0xAC, 0xA4, 0xC4, 0x35];

/// One change of the object a player controls.
#[derive(Clone, Debug, PartialEq)]
pub struct Possession {
    pub player_id: u64,
    /// Frame of the table's record; `None` for the table that opens the
    /// stream, which is its snapshot.
    pub frame: Option<u32>,
    /// The body the player moves: their operator.
    pub body: Option<u32>,
    /// What the player looks through instead, when not their body: a drone
    /// or camera.
    pub view: Option<u32>,
}

/// A player starting to look through something, or going back to their own
/// eyes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewChange {
    pub player_id: u64,
    /// Frame of the table's record; `None` for the opening table.
    pub frame: Option<u32>,
    /// The object looked through; 0 for the player's own eyes. Fixed map
    /// cameras have ids longer than 32 bits.
    pub view: u64,
    /// The kind in force: 0 own eyes, 1 camera, 2 the view of a teammate
    /// the dead player follows, 4 drone. Sent only when it changes.
    pub kind: u8,
}

/// What a round's player tables say.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerTables {
    /// Each body or view a player was given, in stream order.
    pub changes: Vec<Possession>,
    /// Each change of what a player looks through, in stream order.
    pub views: Vec<ViewChange>,
    /// The kind of view each player last had.
    pub(crate) kinds: HashMap<u64, u8>,
    /// Tables read.
    pub tables: usize,
    /// Records of the table stream that did not read as a table.
    pub unread: usize,
    /// Every `playerid` the tables list, in the order first seen.
    pub players: Vec<u64>,
}

impl PlayerTables {
    /// Reads one table: the stream's snapshot or the payload of one of its
    /// frame records. A record can be empty.
    pub fn read(&mut self, frame: Option<u32>, payload: &[u8]) {
        if payload.is_empty() {
            return;
        }
        let Some(entries) = table_entries(payload) else {
            tracing::debug!(?frame, size = payload.len(), "unreadable player table");
            self.unread += 1;
            return;
        };
        self.tables += 1;
        for e in entries {
            if !self.players.contains(&e.player_id) {
                self.players.push(e.player_id);
            }
            if let Some(kind) = e.kind {
                self.kinds.insert(e.player_id, kind);
            }
            if let Some(view) = e.view {
                self.views.push(ViewChange {
                    player_id: e.player_id,
                    frame,
                    view,
                    kind: self.kinds.get(&e.player_id).copied().unwrap_or(0),
                });
            }
            let (body, view) = (e.body.and_then(object), e.view.and_then(object));
            if body.is_some() || view.is_some() {
                self.changes.push(Possession {
                    player_id: e.player_id,
                    frame,
                    body,
                    view,
                });
            }
        }
    }

    /// Finds tables without the stream list, for data whose streams could
    /// not be told apart: a table follows `<size u32> 00000000` (the end of
    /// a record's header, or a snapshot's length) and starts with its count
    /// and a player's id. `ids` are the `playerid`s known from the header
    /// and the controllers. The frame of a table found this way is not
    /// known.
    pub fn scan(body: &[u8], ids: &[u64]) -> Self {
        let mut out = Self::default();
        let ids: Vec<[u8; 8]> = ids
            .iter()
            .filter(|&&i| i != 0)
            .map(|i| i.to_le_bytes())
            .collect();
        // Tables come before the movement stream starts; without one, they
        // are in the opening part of the stream if anywhere.
        let body = &body[..first_movement(body).unwrap_or(body.len().min(SNAPSHOT_BYTES))];
        let Ok(ac) = AhoCorasick::new(&ids) else {
            return out;
        };
        // Where the last table read ends: ids before it are its entries.
        let mut read_to = 0;
        for hit in ac.find_iter(body) {
            let Some(count_at) = hit.start().checked_sub(1).filter(|&at| at >= read_to) else {
                continue;
            };
            let size = count_at
                .checked_sub(8)
                .filter(|&at| zero4(body, at + 4))
                .and_then(|at| u32_at(body, at))
                .map_or(0, |size| size as usize);
            let Some(payload) = body.get(count_at..count_at + size) else {
                continue;
            };
            if size > 0 && table_entries(payload).is_some() {
                out.read(None, payload);
                read_to = count_at + size;
            }
        }
        out
    }
}

/// One player's entry in a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TableEntry {
    pub player_id: u64,
    /// The body reference, when the entry sets it; 0 takes the body away.
    pub body: Option<u64>,
    /// The view reference, when the entry sets it; 0 for none.
    pub view: Option<u64>,
    /// The kind of the view, when the entry sets it (see [`ViewChange`]).
    kind: Option<u8>,
}

/// Mask bits that each add one byte to an entry. `08` and `10` come with
/// the player's place in their team and the team; `20` with `ff` for every
/// player but the recorder; `40` and `80` with the slot and kind of the
/// view.
const ENTRY_BYTES: u8 = 0xF8;
/// Mask bits that add nothing: values 0 to 3 have been seen in them.
const ENTRY_STATE: u8 = 0x03;
const VIEW_SET: u8 = 0x01;
const BODY_SET: u8 = 0x04;
const PLAYER_SET: u8 = 0x08;
const FLOATS_SET: u8 = 0x10;
const LIST_SET: u8 = 0x20;
/// A list names other players; more than a lobby holds is not a list.
const MAX_LIST: usize = 16;

/// The entries of a table, when `payload` is one to its last byte.
pub(crate) fn table_entries(payload: &[u8]) -> Option<Vec<TableEntry>> {
    let (&count, mut rest) = payload.split_first()?;
    let mut out = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let (entry, tail) = table_entry(rest)?;
        out.push(entry);
        rest = tail;
    }
    rest.is_empty().then_some(out)
}

/// The entry at the start of `d` and what follows it. `None` for bits never
/// seen, whose fields would have an unknown size.
fn table_entry(d: &[u8]) -> Option<(TableEntry, &[u8])> {
    let mut at = 0;
    let mut take = |n: usize| {
        let bytes = d.get(at..at + n)?;
        at += n;
        Some(bytes)
    };
    let u64_of = |bytes: &[u8]| bytes.try_into().ok().map(u64::from_le_bytes);
    let player_id = u64_of(take(8)?)?;
    let (mask, flags) = (take(1)?[0], take(1)?[0]);
    if mask & !(ENTRY_BYTES | ENTRY_STATE) != 0
        || flags & !(VIEW_SET | BODY_SET | PLAYER_SET | FLOATS_SET | LIST_SET) != 0
    {
        return None;
    }
    // One byte per bit, in bit order; the last is the kind of the view.
    let bytes = take((mask & ENTRY_BYTES).count_ones() as usize)?;
    let mut entry = TableEntry {
        player_id,
        body: None,
        view: None,
        kind: bytes.last().copied().filter(|_| mask & 0x80 != 0),
    };
    if flags & VIEW_SET != 0 {
        entry.view = u64_of(take(8)?);
    }
    if flags & FLOATS_SET != 0 {
        take(8)?;
    }
    if flags & PLAYER_SET != 0 {
        take(8)?;
    }
    if flags & BODY_SET != 0 {
        entry.body = u64_of(take(8)?);
    }
    if flags & LIST_SET != 0 {
        let n = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
        if n > MAX_LIST {
            return None;
        }
        take(9 * n)?;
    }
    Some((entry, &d[at..]))
}

/// The object a table reference names: an id in the `f0xxxxxx` range
/// objects are numbered in. Not 0 (none), and not the longer ids some views
/// carry.
fn object(reference: u64) -> Option<u32> {
    u32::try_from(reference).ok().filter(|id| id >> 24 == 0xF0)
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

/// An object reference: `<id u32> 00000000` with the id in the `f0xxxxxx`
/// range objects are numbered in.
fn object_at(seg: &[u8], k: usize) -> Option<u32> {
    let id = u32_at(seg, k)?;
    (id >> 24 == 0xF0 && zero4(seg, k + 4)).then_some(id)
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

/// A creation message of the movement stream: a body, or an item a body
/// carries or has placed.
///
/// ```text
/// <obj u64> <size u32> 617385fe <obj u64> 00000000
/// <position 3 x f32> <rotation 4 x f32>
/// <flag u8>                  3 for an item a body carries
/// <u64> <n u32>, n x <class hash>
/// <asset u64> 00000000       what the object is: an operator, a weapon
/// <count u32>, count x { <value u64> <slot hash> <type u32> }
/// 17 more bytes
/// ```
///
/// The size counts from the class hash. A slot's hash is the CRC-32 of its
/// name (`Uniform`, `WeaponSkin`), its value an asset id, 0 when empty;
/// slots come in no fixed order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spawn {
    pub object: u32,
    /// Carried by a body: its weapons and gadgets.
    pub carried: bool,
    pub asset: u64,
    /// `(slot hash, value)`, as written.
    pub slots: Vec<([u8; 4], u64)>,
}

impl Spawn {
    /// The value of a slot; `None` when the slot is absent or empty.
    pub fn slot(&self, hash: [u8; 4]) -> Option<u64> {
        let value = self.slots.iter().find(|s| s.0 == hash)?.1;
        (value != 0).then_some(value)
    }
}

/// Bytes after a creation message's slots.
const SPAWN_TAIL: usize = 17;
/// More slots or classes than any object has (23 and 5 seen).
const MAX_SLOTS: usize = 64;

/// Every creation message in `body` that reads to its last byte.
pub fn spawns(body: &[u8]) -> Vec<Spawn> {
    memchr::memmem::find_iter(body, &CREATE)
        .filter_map(|msg| spawn_at(body, msg))
        .collect()
}

fn spawn_at(body: &[u8], msg: usize) -> Option<Spawn> {
    let object = object_at(body, msg.checked_sub(12)?)?;
    if object_at(body, msg + 4) != Some(object) {
        return None;
    }
    let end = msg + u32_at(body, msg - 4)? as usize;
    // Past the repeated id, a zero, the position and the rotation.
    let mut at = msg + 4 + 8 + 4 + 28;
    let carried = *body.get(at)? == 3;
    let classes = u32_at(body, at + 9)? as usize;
    if classes > MAX_SLOTS {
        return None;
    }
    at += 13 + 4 * classes;
    let asset = u64::from_le_bytes(body.get(at..at + 8)?.try_into().ok()?);
    let count = u32_at(body, at + 12)? as usize;
    at += 16;
    if count > MAX_SLOTS || at + 16 * count + SPAWN_TAIL != end {
        return None;
    }
    let (slots, _) = body.get(at..at + 16 * count)?.as_chunks::<16>();
    let slots = slots
        .iter()
        .map(|s| {
            let value = u64::from_le_bytes(s[..8].try_into().expect("8 bytes"));
            ([s[8], s[9], s[10], s[11]], value)
        })
        .collect();
    Some(Spawn {
        object,
        carried,
        asset,
        slots,
    })
}

/// Whether `body` names `item` right after `holder`, as the list of what a
/// body carries does.
pub fn lists_together(body: &[u8], holder: u32, item: u32) -> bool {
    let mut pair = [0u8; 16];
    pair[..4].copy_from_slice(&holder.to_le_bytes());
    pair[8..12].copy_from_slice(&item.to_le_bytes());
    memchr::memmem::find(body, &pair).is_some()
}

/// Names written to `controllers` on their own: the controller and the new
/// name, in stream order. A name that comes with a player being introduced
/// is not a new name for the player before, and is left out:
///
/// - the whole controller, name and profile id together, as in the opening
///   snapshot and when a seat is filled again;
/// - a name after the seat was emptied (an empty name), or one followed by
///   the names of a new profile object, when another player takes the seat.
pub fn renames(body: &[u8], controllers: &[u32]) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let mut vacated: Vec<u32> = Vec::new();
    for hash in memchr::memmem::find_iter(body, &NAME) {
        let Some(at) = hash.checked_sub(9) else {
            continue;
        };
        let Some((Record::Set(obj, _, from, to), mut next)) = record(body, at) else {
            continue;
        };
        if !controllers.contains(&obj) || vacated.contains(&obj) {
            continue;
        }
        if from == to {
            vacated.push(obj);
            continue;
        }
        let mut introduced = false;
        while let Some((r, n)) = record(body, next) {
            match r {
                Record::Prop(PROFILE_ID, ..) => introduced = true,
                Record::Set(_, PROFILE_NAMES, ..) => {
                    introduced = true;
                    break;
                }
                Record::Set(..) | Record::ParentChild(..) => break,
                _ => {}
            }
            next = n;
        }
        if !introduced {
            out.push((obj, String::from_utf8_lossy(&body[from..to]).into_owned()));
        }
    }
    out
}

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
    fn a_name_written_alone_is_a_rename_and_one_with_a_new_player_is_not() {
        let (renamed, refilled, replaced, emptied) =
            (0xF000_0001, 0xF000_0002, 0xF000_0003, 0xF000_0004);
        let mut d = vec![];
        // The opening snapshot: the whole controller.
        set(&mut d, renamed, NAME, b"alias");
        prop(&mut d, PROFILE_ID, &[b'a'; 36]);
        // Later, the name alone.
        set(&mut d, 0xF000_0100, HERO_TEAM, &[1, 0, 0, 0]);
        set(&mut d, renamed, NAME, b"real name");
        set(&mut d, 0xF000_0100, HERO_TEAM, &[2, 0, 0, 0]);
        // A seat filled again: name and profile id together.
        set(&mut d, refilled, NAME, b"newcomer");
        prop(&mut d, PROFILE_ID, &[b'b'; 36]);
        // Another player takes a seat: the name, then a new profile object.
        set(&mut d, replaced, NAME, b"backfill");
        set(&mut d, 0xF000_0200, PROFILE_NAMES, &[0xFF; 8]);
        // A seat emptied, then named again.
        set(&mut d, emptied, NAME, b"");
        set(&mut d, 0xF000_0100, HERO_TEAM, &[1, 0, 0, 0]);
        set(&mut d, emptied, NAME, b"someone else");
        // A name on an object that is no player's controller.
        set(&mut d, 0xF000_0300, NAME, b"not a player");

        let all = [renamed, refilled, replaced, emptied];
        assert_eq!(renames(&d, &all), [(renamed, "real name".to_owned())]);
    }

    /// A creation message with `slots`, as the movement stream writes it.
    fn creation(object: u32, flag: u8, classes: u32, asset: u64, slots: &[(Hash, u64)]) -> Vec<u8> {
        let mut body = vec![];
        body.extend(CREATE);
        body.extend(u64::from(object).to_le_bytes());
        body.extend([0; 4]);
        body.extend([0; 28]);
        body.push(flag);
        body.extend([0; 8]);
        body.extend(classes.to_le_bytes());
        body.extend(vec![0xCC; 4 * classes as usize]);
        body.extend(asset.to_le_bytes());
        body.extend([0; 4]);
        body.extend((slots.len() as u32).to_le_bytes());
        for (hash, value) in slots {
            body.extend(value.to_le_bytes());
            body.extend(hash);
            body.extend([0; 4]);
        }
        body.extend([0; SPAWN_TAIL]);
        let mut d = vec![];
        d.extend(u64::from(object).to_le_bytes());
        d.extend((body.len() as u32).to_le_bytes());
        d.extend(body);
        d
    }

    #[test]
    fn reads_the_slots_of_creation_messages() {
        let (uniform, skin) = ([1, 2, 3, 4], [5, 6, 7, 8]);
        let mut d = vec![0xAB; 5];
        d.extend(creation(
            0xF000_0001,
            0,
            5,
            77,
            &[(uniform, 100), (skin, 0)],
        ));
        d.extend(creation(0xF000_0002, 3, 4, 88, &[(skin, 200)]));
        let s = spawns(&d);
        assert_eq!(s.len(), 2);
        assert_eq!(
            (s[0].object, s[0].carried, s[0].asset),
            (0xF000_0001, false, 77)
        );
        assert_eq!(s[0].slot(uniform), Some(100));
        assert_eq!(s[0].slot(skin), None, "an empty slot");
        assert_eq!((s[1].carried, s[1].slot(skin)), (true, Some(200)));
        // A message that does not end where its size says is not read.
        let mut short = creation(0xF000_0003, 0, 0, 1, &[(skin, 1)]);
        short[8] += 1;
        assert!(spawns(&short).is_empty());
    }

    #[test]
    fn reads_platform_and_nickname_of_a_player() {
        let (controller, profile) = (0xF000_0001, 0xF000_0002);
        let mut d = vec![];
        set(&mut d, controller, NAME, b"abc");
        prop(&mut d, PLAYER_ID, &7u64.to_le_bytes());
        prop(&mut d, PLATFORM, &5u32.to_le_bytes());
        child(&mut d, PROFILE_FIELD, profile);
        set(&mut d, profile, USES_NICKNAME, &[1]);
        // A later copy of the controller does not replace the first value.
        set(&mut d, controller, PLATFORM, &12u32.to_le_bytes());
        let p = players(&d);
        assert_eq!((p[0].platform, p[0].uses_nickname), (Some(5), true));
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

    /// A table entry: the player's id, then `rest` (mask, flags, fields).
    fn entry(d: &mut Vec<u8>, id: u64, rest: &[u8]) {
        d.extend(id.to_le_bytes());
        d.extend(rest);
    }

    /// An object reference as a table writes it.
    fn reference(obj: u32) -> [u8; 8] {
        u64::from(obj).to_le_bytes()
    }

    fn change(player_id: u64, body: Option<u32>, view: Option<u32>) -> Possession {
        Possession {
            player_id,
            frame: Some(7),
            body,
            view,
        }
    }

    #[test]
    fn a_table_gives_bodies_and_views_by_player_id() {
        let (body, drone) = (0xF02B_8AEF, 0xF02B_8BF3);
        let mut d = vec![4];
        // A defender in the opening table: place, team, `ff`, then the body.
        entry(&mut d, 1, &[0x39, 0x04, 0x04, 0x00, 0xFF]);
        d.extend(reference(body));
        // An attacker on their drone: place, team, `ff`, slot and kind of
        // the view, then the view.
        entry(&mut d, 2, &[0xF9, 0x01, 0x00, 0x01, 0xFF, 0x08, 0x04]);
        d.extend(reference(drone));
        // Two floats come before a body.
        entry(&mut d, 3, &[0x39, 0x14, 0x03, 0x00, 0xFF]);
        d.extend([0xB6, 0x2C, 0x36, 0xBF, 0x8B, 0xD0, 0x8A, 0xBE]);
        d.extend(reference(0xF028_B6B8));
        // Nothing changed.
        entry(&mut d, 4, &[0x21, 0x00, 0xFF]);

        let mut t = PlayerTables::default();
        t.read(Some(7), &d);

        assert_eq!((t.tables, t.unread), (1, 0));
        assert_eq!(t.players, [1, 2, 3, 4]);
        assert_eq!(
            t.changes,
            [
                change(1, Some(body), None),
                change(2, None, Some(drone)),
                change(3, Some(0xF028_B6B8), None),
            ]
        );
    }

    /// The recorder's entry has no `ff` byte (mask bit `20` unset), and it
    /// can hold a list of other players' ids. Splitting a table where ids
    /// are found takes a listed id for an entry and loses the table's last
    /// one.
    #[test]
    fn the_recorders_entry_gives_their_body_and_can_name_other_players() {
        let mut d = vec![3];
        entry(&mut d, 1, &[0x01, 0x04]);
        d.extend(reference(0xF008_CDED));
        entry(&mut d, 2, &[0x21, 0x00, 0xFF]);
        entry(&mut d, 3, &[0x21, 0x00, 0xFF]);
        let mut listing = vec![3];
        // A list of two players, each with a byte.
        entry(&mut listing, 1, &[0x01, 0x20, 2, 0, 0, 0]);
        for (id, byte) in [(2u64, 1), (3, 0)] {
            listing.extend(id.to_le_bytes());
            listing.push(byte);
        }
        entry(&mut listing, 2, &[0x21, 0x00, 0xFF]);
        entry(&mut listing, 3, &[0x21, 0x04, 0xFF]);
        listing.extend(reference(0xF008_CCFA));

        let mut t = PlayerTables::default();
        t.read(Some(7), &d);
        t.read(Some(7), &listing);

        assert_eq!((t.tables, t.unread), (2, 0));
        assert_eq!(
            t.changes,
            [
                change(1, Some(0xF008_CDED), None),
                change(3, Some(0xF008_CCFA), None),
            ]
        );
    }

    #[test]
    fn a_table_lists_players_the_header_does_not() {
        let mut d = vec![2];
        entry(&mut d, 0x46BC_CA3F_3788_21E6, &[0x21, 0x04, 0xFF]);
        d.extend(reference(0xF006_4B2D));
        // A view kind without a view: mask bit `80` adds its byte alone.
        entry(&mut d, 9, &[0xA0, 0x00, 0xFF, 0x02]);

        let mut t = PlayerTables::default();
        t.read(None, &d);

        assert_eq!(t.players, [0x46BC_CA3F_3788_21E6, 9]);
        assert_eq!(t.changes.len(), 1);
        assert_eq!(t.changes[0].frame, None);
    }

    #[test]
    fn a_reference_of_zero_or_a_longer_id_is_no_object() {
        let mut d = vec![3];
        // The body taken away.
        entry(&mut d, 1, &[0x20, 0x04, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0]);
        // A view by slot, with an id outside the object range.
        entry(&mut d, 2, &[0x61, 0x01, 0xFF, 0x04]);
        d.extend([0xF0, 0xFD, 0x1B, 0x5B, 0x60, 0, 0, 0]);
        // A view, two floats, then the player being watched.
        entry(&mut d, 3, &[0xC0, 0x19, 0x08, 0x04]);
        d.extend(reference(0xF008_EB8C));
        d.extend([0x9D, 0x80, 0x1C, 0xBD, 0x88, 0x00, 0x88, 0xBB]);
        d.extend([0xFF; 8]);

        let mut t = PlayerTables::default();
        t.read(Some(7), &d);

        assert_eq!((t.tables, t.unread), (1, 0));
        assert_eq!(t.players, [1, 2, 3]);
        assert_eq!(t.changes, [change(3, None, Some(0xF008_EB8C))]);
    }

    #[test]
    fn a_record_that_is_not_a_table_to_its_last_byte_is_counted_unread() {
        let mut d = vec![1];
        entry(&mut d, 1, &[0x21, 0x04, 0xFF]);
        d.extend(reference(0xF008_CCFA));
        let mut t = PlayerTables::default();

        // A byte too many, a byte too few, a flag never seen, and an empty
        // record (which the stream has, and which is not a fault).
        let mut longer = d.clone();
        longer.push(0);
        t.read(Some(1), &longer);
        t.read(Some(2), &d[..d.len() - 1]);
        let mut unknown = d.clone();
        unknown[10] |= 0x40;
        t.read(Some(3), &unknown);
        t.read(Some(4), &[]);

        assert_eq!((t.tables, t.unread), (0, 3));
        assert!(t.changes.is_empty() && t.players.is_empty());
    }

    #[test]
    fn scan_finds_tables_by_their_framing() {
        let mut table = vec![2];
        entry(&mut table, 5, &[0x01, 0x04]);
        table.extend(reference(0xF008_CDED));
        // Player 6 is in neither the header nor the snapshot.
        entry(&mut table, 6, &[0x21, 0x04, 0xFF]);
        table.extend(reference(0xF008_CCFA));
        // A snapshot: `<length u64>` then the table.
        let mut d = vec![0xAB; 16];
        d.extend((table.len() as u64).to_le_bytes());
        d.extend(&table);
        // A frame record: `<frame u32> <size u32> 00000000` then the table.
        d.extend(900u32.to_le_bytes());
        d.extend((table.len() as u32).to_le_bytes());
        d.extend([0; 4]);
        d.extend(&table);
        // The id again, not in a table.
        d.extend([0x22, 0x33]);
        d.extend(5u64.to_le_bytes());
        d.extend([0x21, 0x00, 0xFF]);

        let t = PlayerTables::scan(&d, &[5, 0]);

        assert_eq!((t.tables, t.unread), (2, 0));
        assert_eq!(t.players, [5, 6]);
        assert_eq!(t.changes.len(), 4);
        assert_eq!(t.changes[1].body, Some(0xF008_CCFA));
    }
}
