//! Derived per-round and per-match statistics.

use std::collections::HashMap;

use serde::Serialize;

use crate::details::LifeEventType;
use crate::devices::{DeviceEventType, EndKind};
use crate::feedback::{MatchUpdate, MatchUpdateType};
use crate::round::Round;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRoundStats {
    pub username: String,
    #[serde(skip)]
    pub team_index: usize,
    /// The player's match score when the recording ends: a total, not the
    /// round's points (those are `scoreboard[].round.score`). From Y11S3
    /// it can be below zero.
    pub score: i32,
    #[serde(skip)]
    pub operator: String,
    pub kills: u32,
    pub died: bool,
    pub assists: u32,
    pub headshots: u32,
    pub headshot_percentage: f64,
    /// Kills made as the last player standing on the winning team.
    #[serde(rename = "1vX", skip_serializing_if = "is_zero")]
    pub one_vx: u32,
    /// Health lost during the action phase. From Y11S3 the damage of every
    /// hit taken, the killing blow included, so an overheal wearing off
    /// does not count.
    pub damage_taken: u32,
    /// Y11S3: damage of the hits on opponents this player is named for.
    /// The attacker of a hit is inferred unless it downed or killed (see
    /// [`crate::combat`]), and hits nobody is named for count for nobody,
    /// so this is an estimate. A hit that downs or kills counts the health
    /// the victim had left.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damage_dealt: Option<u32>,
    /// Y11S3: the same for hits on teammates.
    #[serde(skip_serializing_if = "is_zero")]
    pub team_damage: u32,
    /// Times the player went down.
    #[serde(skip_serializing_if = "is_zero")]
    pub downs: u32,
    /// Times the player was revived.
    #[serde(skip_serializing_if = "is_zero")]
    pub revives: u32,
    /// Y11S3: opponents this player downed.
    #[serde(skip_serializing_if = "is_zero")]
    pub downs_dealt: u32,
    /// Y11S3: kills of players who were down.
    #[serde(skip_serializing_if = "is_zero")]
    pub finishes: u32,
    /// Y11S3: other players this player revived.
    #[serde(skip_serializing_if = "is_zero")]
    pub revives_given: u32,
    /// Y11S3: kills of teammates.
    #[serde(skip_serializing_if = "is_zero")]
    pub team_kills: u32,
    /// Y11S3: health this player's heals gave, to themselves included. The
    /// giver of a heal is inferred (see [`crate::vitals`]).
    #[serde(skip_serializing_if = "is_zero")]
    pub healing_given: u32,
    /// Y11S3: health heals gave this player.
    #[serde(skip_serializing_if = "is_zero")]
    pub healing_received: u32,
    /// Y11S3: pings the player put on the map.
    #[serde(skip_serializing_if = "is_zero")]
    pub pings: u32,
    /// Y11S3: spots of this player the other team saw.
    #[serde(skip_serializing_if = "is_zero")]
    pub times_spotted: u32,
    /// Y11S3: spots this player is named for. Who spotted is inferred (see
    /// [`crate::joins`]), and a spot nobody is named for counts for
    /// nobody, so this is an estimate.
    #[serde(skip_serializing_if = "is_zero")]
    pub spots_made: u32,
    /// Y11S3: times the player got the points of a spot assist (inferred).
    #[serde(skip_serializing_if = "is_zero")]
    pub spot_assists: u32,
    /// Y11S3: drones and cameras of the other team this player is named
    /// for destroying. Who destroyed a device is inferred (see
    /// [`crate::devices`]).
    #[serde(skip_serializing_if = "is_zero")]
    pub devices_destroyed: u32,
    /// Y11S3: drones of this player that were destroyed.
    #[serde(skip_serializing_if = "is_zero")]
    pub drones_lost: u32,
    /// Y11S3: times a jammer disabled a drone of this player.
    #[serde(skip_serializing_if = "is_zero")]
    pub times_jammed: u32,
    /// Y11S3: 1 when this player found the objective.
    #[serde(skip_serializing_if = "is_zero")]
    pub objective_found: u32,
    /// Y11S3: gadgets of `gadgets` this player put out that are in one of
    /// their loadout slots. What a launcher fires and what a gadget leaves
    /// behind (a post, a pellet) is not counted.
    #[serde(skip_serializing_if = "is_zero")]
    pub gadgets_deployed: u32,
    /// Y11S3: gadgets, drones and cameras of the other team this player is
    /// named for destroying. Who destroyed a gadget is inferred from the
    /// scoreboard (see [`crate::gadget_events`]).
    #[serde(skip_serializing_if = "is_zero")]
    pub gadgets_destroyed: u32,
    /// Y11S3: this player's gadgets, drones and cameras that were
    /// destroyed, by anyone.
    #[serde(skip_serializing_if = "is_zero")]
    pub gadgets_lost: u32,
    /// Y11S3: reinforcements and barricades this player put up.
    #[serde(skip_serializing_if = "is_zero")]
    pub reinforcements: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub barricades: u32,
    /// Y11S3: breach devices this player used, and how many of them
    /// opened a reinforcement. A soft wall has no flag that says it was
    /// opened, so a soft breach counts in `breaches` only.
    #[serde(skip_serializing_if = "is_zero")]
    pub breaches: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub breaches_opened: u32,
    /// Y11S3: times a trap of this player went off.
    #[serde(skip_serializing_if = "is_zero")]
    pub traps_triggered: u32,
    /// Seconds spent on drones (any phase, own or a teammate's).
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub drone_seconds: f64,
    /// Seconds spent on cameras.
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub camera_seconds: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerMatchStats {
    pub username: String,
    #[serde(skip)]
    pub team_index: usize,
    pub rounds: u32,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub headshots: u32,
    pub headshot_percentage: f64,
    pub damage_taken: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damage_dealt: Option<u32>,
    pub downs: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub downs_dealt: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub finishes: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub revives_given: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub team_kills: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub healing_given: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub pings: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub times_spotted: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub spots_made: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub spot_assists: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub devices_destroyed: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub drones_lost: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub times_jammed: u32,
    /// Rounds in which this player found the objective.
    #[serde(skip_serializing_if = "is_zero")]
    pub objectives_found: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub gadgets_deployed: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub gadgets_destroyed: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub gadgets_lost: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub reinforcements: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub barricades: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub breaches: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub breaches_opened: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub traps_triggered: u32,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub drone_seconds: f64,
    #[serde(serialize_with = "crate::feedback::whole_number_as_int")]
    pub camera_seconds: f64,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

pub fn headshot_percentage(headshots: u32, kills: u32) -> f64 {
    if kills == 0 {
        0.0
    } else {
        f64::from(headshots) / f64::from(kills) * 100.0
    }
}

impl Round {
    /// The first kill of the round.
    pub fn opening_kill(&self) -> Option<&MatchUpdate> {
        self.match_feedback
            .iter()
            .find(|u| u.kind == MatchUpdateType::Kill)
    }

    /// The first death of the round, by kill or otherwise.
    pub fn opening_death(&self) -> Option<&MatchUpdate> {
        self.match_feedback.iter().find(|u| u.is_kill_or_death())
    }

    pub fn kills_and_deaths(&self) -> impl Iterator<Item = &MatchUpdate> {
        self.match_feedback.iter().filter(|u| u.is_kill_or_death())
    }

    /// Pairs of kills where the second avenges the first within 3 seconds.
    pub fn trades(&self) -> Vec<(&MatchUpdate, &MatchUpdate)> {
        self.match_feedback
            .windows(2)
            .filter_map(|w| {
                let (prev, cur) = (&w[0], &w[1]);
                let same_players = prev.target == cur.username || prev.username == cur.target;
                // The clock counts down.
                let close = prev.time_in_seconds - cur.time_in_seconds <= 3.0;
                (cur.kind == MatchUpdateType::Kill && same_players && close).then_some((prev, cur))
            })
            .collect()
    }

    pub fn team_size(&self, team: usize) -> usize {
        self.header
            .players
            .iter()
            .filter(|p| p.team_index == team)
            .count()
    }

    /// Y11S3: the health `username`'s HUD last showed above zero before
    /// `time`, with a margin for the HUD writing a hit a frame early.
    fn health_before(&self, username: &str, time: Option<f64>) -> Option<u32> {
        let (vitals, time) = (self.vitals.as_ref()?, time?);
        let player = vitals.players.iter().find(|p| p.username == username)?;
        let earlier = |s: &&crate::vitals::VitalSample| {
            s.health > 0 && s.recording_time.is_some_and(|t| t < time - 0.05)
        };
        player.samples.iter().rev().find(earlier).map(|s| s.health)
    }

    /// Y11S3: what each player put out, put up, broke and lost, from the
    /// gadgets, panels and breaches of the round. `find` gives a player's
    /// index in `stats`.
    fn world_stats(&self, stats: &mut [PlayerRoundStats], find: &dyn Fn(&str) -> Option<usize>) {
        use crate::gadget_events::{Cause, Verdict};
        use crate::gadgets::How;
        let mut count = |username: Option<&str>, field: fn(&mut PlayerRoundStats) -> &mut u32| {
            if let Some(s) = username.and_then(find).and_then(|i| stats.get_mut(i)) {
                *field(s) += 1;
            }
        };
        // Who broke something of the other team's.
        let breaker = |v: &Verdict| v.by.clone().filter(|_| !v.friendly);
        for g in &self.gadgets {
            let owner = g.username.as_deref();
            if g.slot.is_some() && g.parent.is_none() {
                count(owner, |s| &mut s.gadgets_deployed);
            }
            if g.end.how == How::Destroyed {
                count(owner, |s| &mut s.gadgets_lost);
            }
            let by = g.end.verdict.as_ref().and_then(breaker);
            count(by.as_deref(), |s| &mut s.gadgets_destroyed);
            for _ in &g.triggers {
                count(owner, |s| &mut s.traps_triggered);
            }
        }
        if let Some(events) = &self.gadget_events {
            for r in &events.removals {
                if matches!(r.verdict.cause, Cause::Destroyed | Cause::Intercepted) {
                    count(r.subject.username.as_deref(), |s| &mut s.gadgets_lost);
                }
                let by = breaker(&r.verdict);
                count(by.as_deref(), |s| &mut s.gadgets_destroyed);
            }
            for t in &events.traps {
                count(t.username.as_deref(), |s| &mut s.traps_triggered);
            }
        }
        for r in self.reinforcements.iter().filter(|r| r.completed.is_some()) {
            count(r.username.as_deref(), |s| &mut s.reinforcements);
        }
        for b in self.barricades.iter().filter(|b| b.completed.is_some()) {
            count(b.username.as_deref(), |s| &mut s.barricades);
        }
        for b in &self.breaches {
            count(b.username.as_deref(), |s| &mut s.breaches);
            if b.opened_reinforcement == Some(true) {
                count(b.username.as_deref(), |s| &mut s.breaches_opened);
            }
        }
    }

    pub fn winning_team(&self) -> usize {
        usize::from(self.header.teams[1].won)
    }

    pub fn player_stats(&self) -> Vec<PlayerRoundStats> {
        let players = &self.header.players;
        let mut stats: Vec<PlayerRoundStats> = players
            .iter()
            .map(|p| {
                let board = self.scoreboard_for(p);
                PlayerRoundStats {
                    username: p.username.clone(),
                    team_index: p.team_index,
                    operator: p.operator.to_string(),
                    assists: board.assists_from_round,
                    score: board.score,
                    ..Default::default()
                }
            })
            .collect();
        let index: HashMap<&str, usize> = players
            .iter()
            .enumerate()
            .map(|(i, p)| (p.username.as_str(), i))
            .collect();
        let find = |name: &str| index.get(name).copied();

        // From Y11S3 the hits themselves say what a player took; before,
        // only the health the HUD showed does, which misses the last blow
        // of most deaths.
        for h in self.health.iter().filter(|_| self.combat.is_none()) {
            if let Some(i) = find(&h.username) {
                stats[i].damage_taken += h.change.min(0).unsigned_abs();
            }
        }
        for e in &self.life_events {
            if let Some(i) = find(&e.username) {
                match e.kind {
                    LifeEventType::Down => stats[i].downs += 1,
                    LifeEventType::Revive => stats[i].revives += 1,
                }
            }
            let by = e.by.as_deref().filter(|by| *by != e.username);
            if let Some(i) = by.and_then(find) {
                match e.kind {
                    LifeEventType::Down => stats[i].downs_dealt += 1,
                    LifeEventType::Revive => stats[i].revives_given += 1,
                }
            }
        }
        if let Some(combat) = &self.combat {
            for s in &mut stats {
                s.damage_dealt = Some(0);
            }
            for h in &combat.hits {
                let Some(victim) = find(&h.username) else {
                    continue;
                };
                // A hit that downs or kills holds no amount: it took what
                // health the victim's HUD last showed before it.
                let damage = h
                    .damage
                    .or_else(|| self.health_before(&h.username, h.recording_time));
                let damage = damage.unwrap_or(0);
                stats[victim].damage_taken += damage;
                let Some(by) = h.by.as_deref().and_then(find).filter(|&by| by != victim) else {
                    continue;
                };
                if stats[by].team_index == stats[victim].team_index {
                    stats[by].team_damage += damage;
                } else {
                    *stats[by].damage_dealt.get_or_insert(0) += damage;
                }
            }
        }
        if let Some(vitals) = &self.vitals {
            for h in &vitals.heals {
                if let Some(i) = find(&h.username) {
                    stats[i].healing_received += h.amount;
                }
                if let Some(i) = h.by.as_deref().and_then(find) {
                    stats[i].healing_given += h.amount;
                }
            }
        }
        for p in &self.pings {
            if let Some(i) = find(&p.username) {
                stats[i].pings += 1;
            }
        }
        for s in &self.spots {
            if let Some(i) = find(&s.username) {
                stats[i].times_spotted += 1;
            }
            if let Some(i) = s.by.as_deref().and_then(find) {
                stats[i].spots_made += 1;
            }
        }
        for a in &self.spot_assists {
            if let Some(i) = find(&a.username) {
                stats[i].spot_assists += 1;
            }
        }
        // A device of the destroyer's own team is not one to their credit.
        let drones = self.drones.iter().map(|d| (&d.end, true, &d.owner));
        let cameras = self.cameras.iter().map(|c| (&c.end, false, &c.owner));
        for (end, drone, owner) in drones.chain(cameras) {
            let Some(end) = end.as_ref().filter(|e| e.kind == EndKind::Destroyed) else {
                continue;
            };
            if let Some(i) = end.by.as_deref().and_then(find)
                && !end.team_kill
            {
                stats[i].devices_destroyed += 1;
            }
            if let Some(i) = owner.as_deref().and_then(find).filter(|_| drone) {
                stats[i].drones_lost += 1;
            }
        }
        for e in &self.device_events {
            let jammed = |d: &&crate::devices::Drone| d.entity == e.device;
            let drone = (self.drones.iter()).find(jammed);
            if let Some(i) = drone.and_then(|d| d.owner.as_deref()).and_then(find)
                && e.kind == DeviceEventType::Jam
            {
                stats[i].times_jammed += 1;
            }
        }
        let finder = self.objective.as_ref().and_then(|o| o.by.as_deref());
        if let Some(i) = finder.and_then(find) {
            stats[i].objective_found = 1;
        }
        self.world_stats(&mut stats, &find);
        for o in &self.observation {
            if let Some(i) = find(&o.username) {
                if o.tool.is_drone() {
                    stats[i].drone_seconds += o.seconds;
                } else if o.tool.is_camera() {
                    stats[i].camera_seconds += o.seconds;
                }
            }
        }

        let mut last_death = None;
        for u in &self.match_feedback {
            match u.kind {
                MatchUpdateType::Kill => {
                    if let Some(i) = find(&u.username) {
                        let s = &mut stats[i];
                        s.kills += 1;
                        s.headshots += u32::from(u.headshot == Some(true));
                        s.finishes += u32::from(u.finish);
                        s.team_kills += u32::from(u.team_kill);
                        s.headshot_percentage = headshot_percentage(s.headshots, s.kills);
                    }
                    if let Some(t) = find(&u.target) {
                        stats[t].died = true;
                        last_death = Some(t);
                    }
                }
                MatchUpdateType::Death => {
                    if let Some(i) = find(&u.username) {
                        stats[i].died = true;
                        last_death = Some(i);
                    }
                }
                _ => {}
            }
        }

        // 1vX: the winning team's last player standing.
        let winner = self.winning_team();
        let alive: Vec<usize> = (0..players.len())
            .filter(|&i| players[i].team_index == winner && !stats[i].died)
            .collect();
        let last_death_was_winner = last_death.is_some_and(|i| players[i].team_index == winner);
        let clutcher = match alive.as_slice() {
            [only] => Some(*only),
            [] if last_death_was_winner => last_death,
            _ => None,
        };
        if let Some(clutcher) = clutcher {
            let username = stats[clutcher].username.clone();
            let on_winning_team =
                |name: &str| find(name).is_some_and(|i| stats[i].team_index == winner);
            let mut team_left = self.team_size(winner) as i64;
            let mut one_vx = 0;
            for u in &self.match_feedback {
                let lost_teammate = match u.kind {
                    MatchUpdateType::Kill => on_winning_team(&u.target),
                    MatchUpdateType::Death | MatchUpdateType::PlayerLeave => {
                        on_winning_team(&u.username)
                    }
                    _ => false,
                };
                if lost_teammate {
                    team_left -= 1;
                }
                if u.username == username && u.kind == MatchUpdateType::Kill && team_left < 2 {
                    one_vx += 1;
                }
            }
            one_vx += stats
                .iter()
                .filter(|s| s.team_index != winner && !s.died)
                .count() as u32;
            stats[clutcher].one_vx = one_vx;
        }
        stats
    }
}

/// Sums round stats per player across a match, in order of first appearance.
pub fn match_stats<'a>(rounds: impl IntoIterator<Item = &'a Round>) -> Vec<PlayerMatchStats> {
    let mut stats: Vec<PlayerMatchStats> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for round in rounds {
        for p in round.player_stats() {
            let i = *index.entry(p.username.clone()).or_insert_with(|| {
                stats.push(PlayerMatchStats {
                    username: p.username.clone(),
                    team_index: p.team_index,
                    ..Default::default()
                });
                stats.len() - 1
            });
            let s = &mut stats[i];
            s.rounds += 1;
            s.kills += p.kills;
            s.deaths += u32::from(p.died);
            s.assists += p.assists;
            s.headshots += p.headshots;
            s.headshot_percentage = headshot_percentage(s.headshots, s.kills);
            s.damage_taken += p.damage_taken;
            if let Some(dealt) = p.damage_dealt {
                *s.damage_dealt.get_or_insert(0) += dealt;
            }
            s.downs += p.downs;
            s.downs_dealt += p.downs_dealt;
            s.finishes += p.finishes;
            s.revives_given += p.revives_given;
            s.team_kills += p.team_kills;
            s.healing_given += p.healing_given;
            s.pings += p.pings;
            s.times_spotted += p.times_spotted;
            s.spots_made += p.spots_made;
            s.spot_assists += p.spot_assists;
            s.devices_destroyed += p.devices_destroyed;
            s.drones_lost += p.drones_lost;
            s.times_jammed += p.times_jammed;
            s.objectives_found += p.objective_found;
            s.gadgets_deployed += p.gadgets_deployed;
            s.gadgets_destroyed += p.gadgets_destroyed;
            s.gadgets_lost += p.gadgets_lost;
            s.reinforcements += p.reinforcements;
            s.barricades += p.barricades;
            s.breaches += p.breaches;
            s.breaches_opened += p.breaches_opened;
            s.traps_triggered += p.traps_triggered;
            s.drone_seconds += p.drone_seconds;
            s.camera_seconds += p.camera_seconds;
        }
    }
    stats
}
