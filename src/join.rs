//! The join of what the world decoders found (Y11S3): each of them reads
//! one thing from the bytes, and what a round says about a gadget, a
//! reinforcement or a kill is spread over several of them. Nothing here
//! reads the stream. What is put together:
//!
//! - **A gadget's end.** [`crate::gadgets`] reads what an entity showed
//!   as it left play (`end.signals`); [`crate::gadget_events`] works out
//!   why, by whom and with what (its removals). A removal belongs to the
//!   gadget of its entity that had started by then, the last of them when
//!   the game used the entity again. Its verdict goes into the gadget's
//!   `end`, and `end.how` follows the cause where the signals alone said
//!   `removed` or guessed `pickedUp`: `destroyed` and `intercepted` make
//!   it `destroyed`; `triggered`, `detonated` and `used` make it
//!   `wentOff`; `pickedUp` makes it `pickedUp`. `end.source` is then
//!   `inferred` unless the cause was read from the signals. When an
//!   entity showed two removals in one life (a Deployable Shield broken,
//!   then tidied away seven seconds later), the one that names a player
//!   is kept, else the first with a cause, else the first.
//! - **Statuses and trap triggers** go to the gadget they are on, as
//!   `gadgets[].statuses` and `gadgets[].triggers`.
//! - **What is left.** Drones and the cameras of the map are no gadgets
//!   of `gadgets[]`: their removals stay in a list of their own
//!   (`deviceRemovals`). A removal of an entity that is no gadget and has
//!   no known cause is left out and counted: pooled objects, swarms,
//!   panes. Statuses and triggers with no gadget to go to stay in
//!   `gadgetStatuses` and `trapTriggers`: a camera's mount, a mine of
//!   Fenrir's HUD list no entity was matched to.
//! - **Reinforcements.** `opened` is the moment
//!   [`crate::destruction`] saw the panel's intact flag cleared. A breach
//!   on a reinforcement names who put the reinforcement up
//!   (`reinforcedBy`), and a gadget fixed to a panel says what kind of
//!   panel (`hostKind`).
//! - **Areas.** A kill, a death and a fire or gas hit name the fire, gas
//!   or swarm area the victim's body was in (`inArea`, by
//!   [`crate::areas::inside`]); a shot says whether it passed through a
//!   smoke cloud (`throughSmoke`, by [`crate::areas::through_smoke`]).
//!   Both are derived and say so.
//! - **Jams and wire.** The effect of a jammed player takes the owner of
//!   the nearest Signal Disruptor (`jammer`), a barbed wire hit the owner
//!   of the nearest wire (`gadgetOwner`), both with a source of `nearest`.

use std::collections::HashMap;

use crate::areas::{self, Area, AreaKind, InArea};
use crate::combat::Hit;
use crate::destruction::Breach;
use crate::feedback::MatchUpdate;
use crate::gadget_events::{Cause, GadgetEvents, Verdict};
use crate::gadgets::{Gadget, How, Source};
use crate::header::Player;
use crate::loadout::When;
use crate::panels::{Barricade, Reinforcement, ReinforcementKind};
use crate::shots::Shot;
use crate::vitals::Effect;
use crate::world::World;

/// `hits[].type.id` of fire and of gas.
const FIRE_DAMAGE: u32 = 36;
const GAS_DAMAGE: u32 = 9;
/// `effects[].type` of a jammed player.
const JAMMED: u32 = 8;
/// A jam belongs to the effect that started at its time, this exactly
/// (seconds).
const SAME_MOMENT: f64 = 0.0005;

/// What the join did, for `decodeStatus`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Removals that went to a gadget's `end`.
    pub removals_joined: usize,
    /// Second removals of a gadget, folded into the first.
    pub removals_folded: usize,
    /// Removals of entities that are no gadget and have no known cause,
    /// left out.
    pub removals_dropped: usize,
    /// Statuses and trap triggers that went to a gadget.
    pub statuses_joined: usize,
    pub triggers_joined: usize,
    /// Opened reinforcements with no reinforcement to go to.
    pub opened_unmatched: usize,
    /// Fire and gas hits, and how many of them were taken inside an area.
    pub area_hits: usize,
    pub area_hits_inside: usize,
}

/// The gadget of `entity` that had started by `frame`: the last of them
/// when the game used the entity again. `None` is the opening snapshot.
/// `index` lists each entity's gadgets in the order they started.
fn gadget_at(
    gadgets: &[Gadget],
    index: &HashMap<u64, Vec<usize>>,
    entity: u64,
    frame: Option<u32>,
) -> Option<usize> {
    let started = |i: &&usize| gadgets.get(**i).is_some_and(|g| g.frames.start <= frame);
    let i = *index.get(&entity)?.iter().rfind(started)?;
    // What an entity showed after it was gone is another use of it.
    let gone = gadgets.get(i)?.frames.gone.flatten();
    gone.is_none_or(|g| frame <= Some(g)).then_some(i)
}

/// What a cause makes of an end the signals left open.
fn refined(how: How, source: Source, verdict: &Verdict) -> (How, Source) {
    let open = how == How::Removed || (how == How::PickedUp && source == Source::Inferred);
    let to = match verdict.cause {
        Cause::Destroyed | Cause::Intercepted => How::Destroyed,
        Cause::Triggered | Cause::Detonated | Cause::Used => How::WentOff,
        Cause::PickedUp => How::PickedUp,
        Cause::Unknown | Cause::RoundEnd => return (how, source),
    };
    if !open {
        return (how, source);
    }
    let read = verdict.cause_source == Some("signals");
    (to, if read { Source::Read } else { Source::Inferred })
}

/// Puts the removals, statuses and trap triggers of `events` on the
/// gadgets they are about and leaves the rest in `events`. `team_of`
/// gives a player's team.
pub(crate) fn gadget_events(
    gadgets: &mut [Gadget],
    events: &mut GadgetEvents,
    team_of: impl Fn(&str) -> Option<usize>,
) -> Counts {
    let mut counts = Counts::default();
    let mut index: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, g) in gadgets.iter().enumerate() {
        index.entry(g.entity).or_default().push(i);
    }
    for list in index.values_mut() {
        list.sort_by_key(|&i| gadgets.get(i).map(|g| g.frames.start));
    }

    // How much a removal says: who, then why.
    let rank = |v: &Verdict| (v.by.is_some(), v.cause != Cause::Unknown);
    let mut taken: Vec<Option<Verdict>> = vec![None; gadgets.len()];
    let mut devices = Vec::new();
    for r in std::mem::take(&mut events.removals) {
        let Some(i) = gadget_at(gadgets, &index, r.subject.entity, Some(r.frame)) else {
            if r.verdict.cause == Cause::Unknown {
                counts.removals_dropped += 1;
            } else {
                devices.push(r);
            }
            continue;
        };
        let Some(slot) = taken.get_mut(i) else {
            continue;
        };
        match slot {
            Some(kept) => {
                counts.removals_folded += 1;
                if rank(&r.verdict) > rank(kept) {
                    *kept = r.verdict;
                }
            }
            None => {
                counts.removals_joined += 1;
                *slot = Some(r.verdict);
            }
        }
    }
    events.removals = devices;
    for (g, verdict) in gadgets.iter_mut().zip(taken) {
        let Some(mut verdict) = verdict else { continue };
        (g.end.how, g.end.source) = refined(g.end.how, g.end.source, &verdict);
        // The team that held it is worth saying when it is not the
        // owner's: a captured gadget.
        let owners = g.username.as_deref().and_then(&team_of);
        if verdict.owner_team == owners {
            verdict.owner_team = None;
        }
        g.end.verdict = Some(verdict);
    }

    let mut statuses = Vec::new();
    for s in std::mem::take(&mut events.statuses) {
        let at = gadget_at(gadgets, &index, s.subject.entity, Some(s.frame));
        match at.and_then(|i| gadgets.get_mut(i)) {
            Some(g) => {
                g.statuses.push(s.shown);
                counts.statuses_joined += 1;
            }
            None => statuses.push(s),
        }
    }
    events.statuses = statuses;

    let mut traps = Vec::new();
    for t in std::mem::take(&mut events.traps) {
        let at = t
            .entity
            .and_then(|e| gadget_at(gadgets, &index, e, t.frame));
        match at.and_then(|i| gadgets.get_mut(i)) {
            Some(g) => {
                g.triggers.push(t.trigger);
                counts.triggers_joined += 1;
            }
            None => traps.push(t),
        }
    }
    events.traps = traps;
    counts
}

fn seconds(when: Option<&When>) -> Option<f64> {
    when?.recording_time
}

/// The index of the reinforcement of `entity` that had started by
/// `seconds`: the last of them. One the map placed has no start.
fn reinforcement_at(list: &[Reinforcement], entity: u64, at: Option<f64>) -> Option<usize> {
    let started = |r: &Reinforcement| match (seconds(r.started.as_ref()), at) {
        (Some(start), Some(at)) => start <= at + SAME_MOMENT,
        _ => true,
    };
    list.iter()
        .rposition(|r| r.entity == entity && !r.cancelled && started(r))
}

/// Gives each opened reinforcement its `opened`, each breach on a
/// reinforcement and each reinforcement a breach device struck who put it
/// up, and each gadget fixed to a panel the kind of panel. `opened` is `(entity, when its intact flag was cleared)`.
/// Returns how many of `opened` found no reinforcement.
pub(crate) fn panels(
    reinforcements: &mut [Reinforcement],
    barricades: &[Barricade],
    breaches: &mut [Breach],
    gadgets: &mut [Gadget],
    opened: Vec<(u64, When)>,
) -> usize {
    let mut unmatched = 0;
    for (entity, when) in opened {
        let at = reinforcement_at(reinforcements, entity, when.recording_time);
        match at.and_then(|i| reinforcements.get_mut(i)) {
            Some(r) if r.opened.is_none() => r.opened = Some(when),
            Some(_) => {}
            None => unmatched += 1,
        }
    }
    // Who put up the reinforcement of entity `hex` that stood at `at`.
    let by = |hex: &str, at: Option<f64>| {
        let entity = u64::from_str_radix(hex, 16).ok()?;
        let index = reinforcement_at(reinforcements, entity, at)?;
        reinforcements.get(index)?.username.clone()
    };
    for b in breaches.iter_mut() {
        let at = b.when.recording_time;
        b.reinforced_by = b.reinforcement.as_deref().and_then(|hex| by(hex, at));
        let ended = seconds(b.ended.as_ref()).or(at);
        for a in b.affected.iter_mut().filter(|a| a.kind.reinforced()) {
            a.reinforced_by = by(&a.object, ended);
        }
    }
    let mut kinds: HashMap<u64, &'static str> = HashMap::new();
    for b in barricades {
        kinds.insert(b.entity, "barricade");
    }
    for r in reinforcements.iter() {
        let kind = match r.kind {
            ReinforcementKind::Wall => "reinforcedWall",
            ReinforcementKind::Hatch => "reinforcedHatch",
        };
        kinds.insert(r.entity, kind);
    }
    for g in gadgets.iter_mut() {
        g.host_kind = g.host.and_then(|h| kinds.get(&h).copied());
    }
    unmatched
}

/// Where the players' bodies were, for the joins that need a place.
pub(crate) struct Bodies<'a> {
    pub world: &'a World,
    pub players: &'a [Player],
    /// Seconds since the recording started, per frame.
    pub frame_times: &'a [f64],
}

impl Bodies<'_> {
    /// Where `username`'s body was `seconds` into the recording: the last
    /// position the world kept up to that frame. A dead player's body
    /// stays where it fell.
    fn at(&self, username: &str, seconds: f64) -> Option<[f32; 3]> {
        let index = self.players.iter().position(|p| p.username == username)?;
        let player = self.players.get(index)?;
        let named = player.entities.as_ref().and_then(|e| e.movement);
        let body = named.map(u64::from).or_else(|| self.world.body_of(index))?;
        let frames = self
            .frame_times
            .partition_point(|&t| t <= seconds + SAME_MOMENT);
        let frame = u32::try_from(frames.checked_sub(1)?).ok()?;
        self.world.body_at(body, frame)
    }

    /// The fire, gas or swarm area `username` stood in.
    fn area(&self, areas: &[Area], username: &str, seconds: Option<f64>) -> Option<InArea> {
        let seconds = seconds?;
        let index = areas::inside(areas, self.at(username, seconds)?, seconds)?;
        let area = areas.get(index)?;
        Some(InArea {
            area: index,
            kind: area.kind,
            source: area.source,
            username: area.username.clone(),
        })
    }
}

/// Names the area each victim of a kill or death and of a fire or gas hit
/// stood in, and marks the shots that passed through smoke. Returns the
/// fire and gas hits and how many of them were inside an area.
pub(crate) fn in_areas(
    areas: &[Area],
    bodies: &Bodies,
    feedback: &mut [MatchUpdate],
    hits: &mut [Hit],
    shots: &mut [Shot],
) -> (usize, usize) {
    for u in feedback.iter_mut() {
        let Some(victim) = u.victim().map(str::to_owned) else {
            continue;
        };
        u.in_area = bodies.area(areas, &victim, u.recording_time);
        u.in_area_source = u.in_area.as_ref().map(|_| "derived");
    }
    let (mut burned, mut inside) = (0, 0);
    for h in hits.iter_mut() {
        if ![FIRE_DAMAGE, GAS_DAMAGE].contains(&h.kind.id) {
            continue;
        }
        burned += 1;
        h.in_area = bodies.area(areas, &h.username, h.recording_time);
        h.in_area_source = h.in_area.as_ref().map(|_| "derived");
        inside += usize::from(h.in_area.is_some());
    }
    if areas.iter().any(|a| a.kind == AreaKind::Smoke) {
        for s in shots.iter_mut() {
            let Some(at) = s.when.recording_time else {
                continue;
            };
            let (origin, direction) = (s.origin.map(f64::from), s.direction.map(f64::from));
            s.through_smoke =
                areas::through_smoke(areas, origin, direction, f64::from(s.distance), at);
        }
    }
    (burned, inside)
}

/// Names the jammer's owner on the effects of jammed players and the
/// wire's owner on barbed wire hits, from what
/// [`crate::gadget_events`] found nearest.
pub(crate) fn nearest_gadgets(events: &GadgetEvents, effects: &mut [Effect], hits: &mut [Hit]) {
    for jam in &events.jams {
        let Some(owner) = &jam.jammer_owner else {
            continue;
        };
        let same = |e: &&mut Effect| {
            let start = e.start.recording_time;
            e.kind == JAMMED
                && e.username == jam.username
                && start.is_some_and(|t| (t - jam.recording_time).abs() <= SAME_MOMENT)
        };
        if let Some(e) = effects.iter_mut().find(same) {
            e.jammer = Some(owner.clone());
            e.jammer_source = Some("nearest");
        }
    }
    for wire in &events.wire_hits {
        let (Some(owner), Some(h)) = (&wire.username, hits.get_mut(wire.hit)) else {
            continue;
        };
        h.gadget_owner = Some(owner.clone());
        h.gadget_owner_source = Some("nearest");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gadget_events::{
        Removal, Shown, Signal, Status, StatusKind, Subject, TrapTrigger, Trigger,
    };
    use crate::gadgets::{End, Frames};

    const SHIELD: u64 = 0xF000_0010;
    const DRONE: u64 = 0xF000_0020;

    fn gadget(entity: u64, start: u32, gone: Option<u32>) -> Gadget {
        Gadget {
            entity,
            username: Some("owner".to_owned()),
            end: End {
                how: How::Removed,
                ..End::default()
            },
            frames: Frames {
                start: Some(start),
                gone: gone.map(Some),
                ..Frames::default()
            },
            ..Gadget::default()
        }
    }

    fn removal(entity: u64, frame: u32, cause: Cause, by: Option<&str>) -> Removal {
        Removal {
            subject: Subject {
                entity,
                ..Subject::default()
            },
            position: None,
            signals: vec![Signal::NotLive],
            verdict: Verdict {
                owner_team: Some(1),
                cause,
                cause_source: by.map(|_| "score"),
                by: by.map(str::to_owned),
                by_source: by.map(|_| "score"),
                ..Verdict::default()
            },
            when: When::default(),
            frame,
        }
    }

    fn events(removals: Vec<Removal>) -> GadgetEvents {
        GadgetEvents {
            removals,
            ..GadgetEvents::default()
        }
    }

    fn team(username: &str) -> Option<usize> {
        Some(usize::from(username == "owner"))
    }

    #[test]
    fn a_removal_goes_to_the_gadget_of_its_entity_and_refines_how() {
        let mut gadgets = [gadget(SHIELD, 100, Some(900))];
        let mut found = events(vec![removal(SHIELD, 500, Cause::Destroyed, Some("enemy"))]);
        let counts = gadget_events(&mut gadgets, &mut found, team);
        assert_eq!(counts.removals_joined, 1);
        assert!(found.removals.is_empty());
        let end = &gadgets[0].end;
        assert_eq!((end.how, end.source), (How::Destroyed, Source::Inferred));
        let verdict = end.verdict.as_ref().unwrap();
        assert_eq!(verdict.by.as_deref(), Some("enemy"));
        assert_eq!(verdict.by_source, Some("score"));
        // The owner's own team held it: not worth saying.
        assert_eq!(verdict.owner_team, None);
    }

    #[test]
    fn a_gadget_held_by_the_other_team_says_so() {
        let mut gadgets = [gadget(SHIELD, 100, None)];
        let mut held = removal(SHIELD, 500, Cause::Destroyed, Some("owner"));
        held.verdict.owner_team = Some(0);
        gadget_events(&mut gadgets, &mut events(vec![held]), team);
        let verdict = gadgets[0].end.verdict.as_ref().unwrap();
        assert_eq!(verdict.owner_team, Some(0));
    }

    #[test]
    fn an_entity_used_again_gives_each_gadget_its_own_removal() {
        let mut gadgets = [gadget(SHIELD, 100, Some(300)), gadget(SHIELD, 400, None)];
        let mut found = events(vec![
            removal(SHIELD, 290, Cause::Detonated, None),
            removal(SHIELD, 800, Cause::Destroyed, Some("enemy")),
            // After the first was gone and before the second started.
            removal(SHIELD, 350, Cause::Destroyed, Some("enemy")),
        ]);
        let counts = gadget_events(&mut gadgets, &mut found, team);
        assert_eq!(counts.removals_joined, 2);
        assert_eq!(gadgets[0].end.how, How::WentOff);
        assert_eq!(gadgets[1].end.how, How::Destroyed);
        assert_eq!(found.removals.len(), 1);
        assert_eq!(found.removals[0].frame, 350);
    }

    #[test]
    fn of_two_removals_the_one_that_says_more_is_kept() {
        let mut gadgets = [gadget(SHIELD, 100, Some(900))];
        let mut found = events(vec![
            removal(SHIELD, 200, Cause::Unknown, None),
            removal(SHIELD, 500, Cause::Destroyed, Some("enemy")),
            removal(SHIELD, 900, Cause::RoundEnd, None),
        ]);
        let counts = gadget_events(&mut gadgets, &mut found, team);
        assert_eq!((counts.removals_joined, counts.removals_folded), (1, 2));
        let verdict = gadgets[0].end.verdict.as_ref().unwrap();
        assert_eq!(verdict.cause, Cause::Destroyed);
    }

    #[test]
    fn what_the_signals_read_is_not_overruled() {
        let mut gadgets = [gadget(SHIELD, 100, None)];
        gadgets[0].end.how = How::Destroyed;
        let mut found = events(vec![removal(SHIELD, 500, Cause::Triggered, None)]);
        gadget_events(&mut gadgets, &mut found, team);
        assert_eq!(
            (gadgets[0].end.how, gadgets[0].end.source),
            (How::Destroyed, Source::Read)
        );
        // A cause read from the signals keeps the end read.
        let mut gadgets = [gadget(SHIELD, 100, None)];
        let mut read = removal(SHIELD, 500, Cause::Destroyed, None);
        read.verdict.cause_source = Some("signals");
        gadget_events(&mut gadgets, &mut events(vec![read]), team);
        assert_eq!(
            (gadgets[0].end.how, gadgets[0].end.source),
            (How::Destroyed, Source::Read)
        );
        // A guess of the delay gives way to a cause.
        let mut gadgets = [gadget(SHIELD, 100, None)];
        (gadgets[0].end.how, gadgets[0].end.source) = (How::PickedUp, Source::Inferred);
        let mut found = events(vec![removal(SHIELD, 500, Cause::Destroyed, Some("enemy"))]);
        gadget_events(&mut gadgets, &mut found, team);
        assert_eq!(gadgets[0].end.how, How::Destroyed);
    }

    #[test]
    fn a_removal_with_no_gadget_stays_when_its_cause_is_known() {
        let mut gadgets = [gadget(SHIELD, 100, None)];
        let mut found = events(vec![
            removal(DRONE, 500, Cause::Destroyed, Some("enemy")),
            removal(DRONE + 1, 500, Cause::Unknown, None),
            // Before the gadget of its entity started.
            removal(SHIELD, 50, Cause::Unknown, None),
        ]);
        let counts = gadget_events(&mut gadgets, &mut found, team);
        assert_eq!((counts.removals_joined, counts.removals_dropped), (0, 2));
        assert_eq!(found.removals.len(), 1);
        assert_eq!(found.removals[0].subject.entity, DRONE);
        assert_eq!(gadgets[0].end.verdict, None);
    }

    #[test]
    fn statuses_and_triggers_go_to_their_gadget_or_stay() {
        let status = |entity: u64| Status {
            subject: Subject {
                entity,
                ..Subject::default()
            },
            shown: Shown {
                kind: StatusKind::Frozen,
                fx_asset: None,
                team: None,
                by: None,
                by_source: None,
                target: None,
                until: None,
                when: When::default(),
            },
            frame: 500,
        };
        let trap = |entity: Option<u64>| TrapTrigger {
            trap: "Welcome Mat".to_owned(),
            entity,
            asset: None,
            type_index: None,
            username: Some("owner".to_owned()),
            trigger: Trigger {
                marker: "inert".to_owned(),
                detonated_at: None,
                location: None,
                position: None,
                victims: Vec::new(),
                nearest_enemy: None,
                nearest_enemy_distance: None,
                nearest_enemy_source: None,
                points: None,
                when: When::default(),
            },
            frame: Some(500),
        };
        let mut gadgets = [gadget(SHIELD, 100, None)];
        let mut found = GadgetEvents {
            statuses: vec![status(SHIELD), status(DRONE)],
            traps: vec![trap(Some(SHIELD)), trap(None)],
            ..GadgetEvents::default()
        };
        let counts = gadget_events(&mut gadgets, &mut found, team);
        assert_eq!((counts.statuses_joined, counts.triggers_joined), (1, 1));
        assert_eq!(gadgets[0].statuses.len(), 1);
        assert_eq!(gadgets[0].triggers.len(), 1);
        assert_eq!(found.statuses.len(), 1);
        assert_eq!(found.statuses[0].subject.entity, DRONE);
        assert_eq!(found.traps.len(), 1);
        assert_eq!(found.traps[0].entity, None);
    }

    fn at(seconds: f64) -> When {
        When {
            recording_time: Some(seconds),
            ..When::default()
        }
    }

    fn reinforcement(entity: u64, by: &str, started: f64) -> Reinforcement {
        Reinforcement {
            entity,
            username: Some(by.to_owned()),
            started: Some(at(started)),
            completed: Some(at(started + 4.1)),
            ..Reinforcement::default()
        }
    }

    #[test]
    fn a_reinforcement_is_opened_and_names_who_put_it_up() {
        const PANEL: u64 = 0xF000_0300;
        // The entity was a reinforcement twice.
        let mut list = [
            reinforcement(PANEL, "first", 10.0),
            reinforcement(PANEL, "second", 60.0),
            reinforcement(PANEL + 1, "other", 20.0),
        ];
        let mut charge = gadget(SHIELD, 100, None);
        charge.host = Some(PANEL + 1);
        let mut gadgets = [charge, gadget(SHIELD + 1, 100, None)];
        let unmatched = panels(
            &mut list,
            &[],
            &mut [],
            &mut gadgets,
            vec![(PANEL, at(90.0)), (PANEL + 9, at(90.0))],
        );
        assert_eq!(unmatched, 1);
        assert_eq!(list[0].opened, None);
        assert_eq!(list[1].opened, Some(at(90.0)));
        assert_eq!(list[2].opened, None);
        assert_eq!(gadgets[0].host_kind, Some("reinforcedWall"));
        assert_eq!(gadgets[1].host_kind, None);
        assert_eq!(reinforcement_at(&list, PANEL, Some(30.0)), Some(0));
        assert_eq!(reinforcement_at(&list, PANEL, Some(5.0)), None);
    }

    #[test]
    fn a_called_off_reinforcement_is_not_opened() {
        const PANEL: u64 = 0xF000_0300;
        let mut called_off = reinforcement(PANEL, "second", 60.0);
        called_off.cancelled = true;
        let mut list = [reinforcement(PANEL, "first", 10.0), called_off];
        panels(&mut list, &[], &mut [], &mut [], vec![(PANEL, at(90.0))]);
        assert!(list[0].opened.is_some());
        assert_eq!(list[1].opened, None);
    }

    #[test]
    fn a_jam_and_a_wire_hit_name_the_nearest_gadgets_owner() {
        use crate::gadget_events::{Jam, WireHit};
        let effect = |username: &str, kind: u32, start: f64| Effect {
            username: username.to_owned(),
            kind,
            name: None,
            buff: false,
            start: crate::vitals::At {
                recording_time: Some(start),
                ..Default::default()
            },
            seconds: 2.0,
            open: false,
            jammer: None,
            jammer_source: None,
        };
        let mut effects = [
            effect("victim", JAMMED, 12.0),
            effect("victim", JAMMED, 40.0),
            effect("victim", 13, 40.0),
        ];
        let found = GadgetEvents {
            jams: vec![Jam {
                username: "victim".to_owned(),
                recording_time: 40.0,
                jammer: 7,
                jammer_owner: Some("mute".to_owned()),
                device: None,
                distance: 1.0,
            }],
            wire_hits: vec![WireHit {
                hit: 3,
                entity: 9,
                username: Some("owner".to_owned()),
                distance: 0.5,
            }],
            ..GadgetEvents::default()
        };
        // A hit index out of range is skipped.
        nearest_gadgets(&found, &mut effects, &mut []);
        assert_eq!(effects[0].jammer, None);
        assert_eq!(effects[1].jammer.as_deref(), Some("mute"));
        assert_eq!(effects[1].jammer_source, Some("nearest"));
        assert_eq!(effects[2].jammer, None);
    }
}
