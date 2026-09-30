//! Match-wide breakdowns built from every round: side records, site and spawn
//! win rates, operator records, how rounds ended, prep swaps and rounds that
//! started a player down. Rates are wins over rounds played, 0 to 1.

use serde::Serialize;

use crate::feedback::MatchUpdateType;
use crate::outcome::RoundInfo;
use crate::round::Round;
use crate::types::{Operator, TeamRole, WinCondition};

/// Rounds played and won.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub played: u32,
    pub won: u32,
    pub win_rate: f64,
}

impl Record {
    fn add(&mut self, won: bool) {
        self.played += 1;
        self.won += u32::from(won);
        self.win_rate = f64::from(self.won) / f64::from(self.played);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamAnalytics {
    pub name: String,
    pub attack: Record,
    pub defense: Record,
    /// Rounds that started with this team a player down, and how they went.
    pub started_down: Record,
    pub plants: u32,
    pub disables: u32,
    /// Attacker operator swaps during prep, and how many came in its last
    /// 10 seconds.
    pub swaps: u32,
    pub late_swaps: u32,
}

/// A defended site.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteStats {
    pub site: String,
    /// From the defenders' side.
    pub defense: Record,
    /// Per team, the rounds it defended this site.
    pub defended_by: [Record; 2],
    /// Per team, the rounds it attacked this site.
    pub attacked_by: [Record; 2],
}

/// An attacker spawn, per team: one pick per player per round.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnStats {
    pub spawn: String,
    pub team: usize,
    /// Player-rounds that spawned here, and how many of those rounds the
    /// team won.
    pub picks: Record,
    /// Rounds at least one player of the team spawned here.
    pub rounds: Record,
}

/// An operator, per team, as played once prep ended.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatorStats {
    pub operator: Operator,
    pub team: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<TeamRole>,
    pub rounds: Record,
    pub kills: u32,
    pub deaths: u32,
    pub headshots: u32,
    /// Rounds this operator was swapped to during prep.
    pub swapped_to: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndReasonCount {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<WinCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner_side: Option<TeamRole>,
    pub rounds: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchAnalytics {
    pub teams: [TeamAnalytics; 2],
    pub sites: Vec<SiteStats>,
    pub spawns: Vec<SpawnStats>,
    pub operators: Vec<OperatorStats>,
    pub end_reasons: Vec<EndReasonCount>,
}

impl MatchAnalytics {
    /// Builds the breakdowns from rounds in play order. Rounds without a
    /// known winner are skipped.
    pub fn new(rounds: &[Round]) -> Self {
        let mut a = MatchAnalytics::default();
        if let Some(first) = rounds.first() {
            for t in 0..2 {
                a.teams[t].name = first.header.teams[t].name.clone();
            }
        }
        for round in rounds {
            let info = round.info();
            let Some(winner) = info.winner else { continue };
            a.add(round, &info, winner);
        }
        a.sites.sort_by(|x, y| x.site.cmp(&y.site));
        a.spawns
            .sort_by(|x, y| (x.team, &x.spawn).cmp(&(y.team, &y.spawn)));
        a.operators.sort_by(|x, y| {
            (x.team, std::cmp::Reverse(x.rounds.played), x.operator).cmp(&(
                y.team,
                std::cmp::Reverse(y.rounds.played),
                y.operator,
            ))
        });
        a
    }

    fn add(&mut self, round: &Round, info: &RoundInfo, winner: usize) {
        for t in 0..2 {
            let won = t == winner;
            let team = &mut self.teams[t];
            match info.sides[t] {
                Some(TeamRole::Attack) => team.attack.add(won),
                Some(TeamRole::Defense) => team.defense.add(won),
                None => {}
            }
            if info.started_down[t] {
                team.started_down.add(won);
            }
            team.swaps += info.swaps.iter().filter(|s| s.team == t).count() as u32;
            team.late_swaps += info.swaps.iter().filter(|s| s.team == t && s.late).count() as u32;
        }
        for u in &round.match_feedback {
            let Some(t) = u.team.or_else(|| team_of(info, &u.username)) else {
                continue;
            };
            match u.kind {
                MatchUpdateType::DefuserPlantComplete => self.teams[t].plants += 1,
                MatchUpdateType::DefuserDisableComplete => self.teams[t].disables += 1,
                _ => {}
            }
        }

        let defender = info
            .sides
            .iter()
            .position(|s| *s == Some(TeamRole::Defense));
        if let Some(d) = defender.filter(|_| !info.site.is_empty()) {
            let i = match self.sites.iter().position(|s| s.site == info.site) {
                Some(i) => i,
                None => {
                    self.sites.push(SiteStats {
                        site: info.site.clone(),
                        ..SiteStats::default()
                    });
                    self.sites.len() - 1
                }
            };
            let site = &mut self.sites[i];
            site.defense.add(winner == d);
            site.defended_by[d].add(winner == d);
            site.attacked_by[d ^ 1].add(winner != d);
        }

        // Spawns: attackers only; defenders' `spawn` is the site.
        let mut seen: Vec<(usize, &str)> = Vec::new();
        for p in info
            .lineup
            .iter()
            .filter(|p| p.side == Some(TeamRole::Attack) && !p.spawn.is_empty())
        {
            let won = p.team == winner;
            let i = self.spawn_index(p.team, &p.spawn);
            self.spawns[i].picks.add(won);
            if !seen.contains(&(p.team, p.spawn.as_str())) {
                seen.push((p.team, p.spawn.as_str()));
                self.spawns[i].rounds.add(won);
            }
        }

        for p in &info.lineup {
            if p.operator.is_empty() {
                continue;
            }
            let i = self.operator_index(p.team, p.operator, p.side);
            let o = &mut self.operators[i];
            o.rounds.add(p.team == winner);
            if !p.swapped_from.is_empty() {
                o.swapped_to += 1;
            }
            for u in &round.match_feedback {
                match u.kind {
                    MatchUpdateType::Kill if u.username == p.username => {
                        o.kills += 1;
                        o.headshots += u32::from(u.headshot == Some(true));
                    }
                    MatchUpdateType::Kill if u.target == p.username => o.deaths += 1,
                    MatchUpdateType::Death if u.username == p.username => o.deaths += 1,
                    _ => {}
                }
            }
        }

        let side = info.sides[winner];
        match self
            .end_reasons
            .iter_mut()
            .find(|e| e.reason == info.end_reason && e.winner_side == side)
        {
            Some(e) => e.rounds += 1,
            None => self.end_reasons.push(EndReasonCount {
                reason: info.end_reason,
                winner_side: side,
                rounds: 1,
            }),
        }
    }

    fn spawn_index(&mut self, team: usize, spawn: &str) -> usize {
        if let Some(i) = self
            .spawns
            .iter()
            .position(|s| s.team == team && s.spawn == spawn)
        {
            return i;
        }
        self.spawns.push(SpawnStats {
            spawn: spawn.to_owned(),
            team,
            ..SpawnStats::default()
        });
        self.spawns.len() - 1
    }

    fn operator_index(&mut self, team: usize, operator: Operator, side: Option<TeamRole>) -> usize {
        if let Some(i) = self
            .operators
            .iter()
            .position(|o| o.team == team && o.operator == operator)
        {
            return i;
        }
        self.operators.push(OperatorStats {
            operator,
            team,
            side: operator.role().or(side),
            ..OperatorStats::default()
        });
        self.operators.len() - 1
    }
}

fn team_of(info: &RoundInfo, username: &str) -> Option<usize> {
    info.lineup
        .iter()
        .find(|p| p.username == username)
        .map(|p| p.team)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_keep_a_win_rate() {
        let mut r = Record::default();
        r.add(true);
        r.add(false);
        r.add(true);
        assert_eq!((r.played, r.won), (3, 2));
        assert!((r.win_rate - 2.0 / 3.0).abs() < 1e-9);
    }
}
