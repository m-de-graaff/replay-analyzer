//! Derived per-round and per-match statistics.

use std::collections::HashMap;

use serde::Serialize;

use crate::details::LifeEventType;
use crate::feedback::{MatchUpdate, MatchUpdateType};
use crate::round::Round;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRoundStats {
    pub username: String,
    #[serde(skip)]
    pub team_index: usize,
    pub score: u32,
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
    /// Health lost during the action phase.
    pub damage_taken: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub downs: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub revives: u32,
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
    pub downs: u32,
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

        for h in &self.health {
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
        }
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
            s.downs += p.downs;
            s.drone_seconds += p.drone_seconds;
            s.camera_seconds += p.camera_seconds;
        }
    }
    stats
}
