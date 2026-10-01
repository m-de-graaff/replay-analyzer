//! Players across many matches: who they are, what they were called when,
//! and who plays with you.
//!
//! Usernames change; profile ids do not. Players are keyed by
//! [`PlayerSummary::key`], the profile id when the replay has one.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Serialize, Serializer};

use crate::entities::Relation;
use crate::header::Platform;
use crate::summary::MatchSummary;

/// Every player seen in a set of matches.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerDirectory {
    /// Keys of the accounts that recorded, most matches first. Usually one:
    /// you.
    pub you: Vec<String>,
    /// Matches read.
    pub matches: usize,
    /// Most matches first.
    pub players: Vec<KnownPlayer>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownPlayer {
    pub key: String,
    #[serde(rename = "profileID", skip_serializing_if = "String::is_empty")]
    pub profile_id: String,
    /// The name in the latest match: the one the game gave as that match
    /// ended when the recording has it, since the name shown during a match
    /// can be a nickname.
    pub username: String,
    /// Every name used, in the order first seen. More than one means the
    /// player renamed, plays behind a nickname, or is on a console (or,
    /// without a profile id, never happens: the name is the key).
    pub names: Vec<NameUse>,
    /// `pc`, `playstation` or `xbox`, in the latest match that says
    /// (Y11S3+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
    pub matches: u32,
    /// Matches this player recorded.
    #[serde(skip_serializing_if = "is_zero")]
    pub recorded: u32,
    /// Matches on the recording player's team, and against it.
    pub with_you: u32,
    pub against_you: u32,
    /// Matches in the recording player's party (Y8S1+, full or partial
    /// reads).
    #[serde(skip_serializing_if = "is_zero")]
    pub party_with_you: u32,
    /// Queued with you at least once, or on your team in two or more
    /// matches: a likely premade.
    pub queue_mate: bool,
    #[serde(serialize_with = "rfc3339")]
    pub first_seen: DateTime<Utc>,
    #[serde(serialize_with = "rfc3339")]
    pub last_seen: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NameUse {
    pub username: String,
    /// A nickname the game showed in place of the player's own name.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub nickname: bool,
    /// The game gave the player this name as a match ended: their own name
    /// for a player behind a nickname. Console players get one too.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub at_match_end: bool,
    pub matches: u32,
    #[serde(serialize_with = "rfc3339")]
    pub first_seen: DateTime<Utc>,
    #[serde(serialize_with = "rfc3339")]
    pub last_seen: DateTime<Utc>,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

fn rfc3339<S: Serializer>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(&t.format("%Y-%m-%dT%H:%M:%SZ"))
}

impl PlayerDirectory {
    /// Builds the directory from match summaries in any order. A match seen
    /// twice (the same `matchID`, e.g. imported from two players) counts
    /// once.
    pub fn new(summaries: &[MatchSummary]) -> Self {
        let mut order: Vec<&MatchSummary> = Vec::new();
        for s in summaries {
            if s.match_id.is_empty() || !order.iter().any(|o| o.match_id == s.match_id) {
                order.push(s);
            }
        }
        order.sort_by_key(|s| s.start_time);

        let mut players: HashMap<String, KnownPlayer> = HashMap::new();
        let mut recorders: HashMap<String, u32> = HashMap::new();
        for s in &order {
            let when = s.start_time;
            for p in s.teams.iter().flat_map(|t| &t.players) {
                let e = players.entry(p.key.clone()).or_insert_with(|| KnownPlayer {
                    key: p.key.clone(),
                    first_seen: when,
                    ..KnownPlayer::default()
                });
                e.matches += 1;
                e.last_seen = when;
                e.username = p.renamed_to.as_ref().unwrap_or(&p.username).clone();
                if !p.profile_id.is_empty() {
                    e.profile_id = p.profile_id.clone();
                }
                if p.platform.is_some() {
                    e.platform = p.platform;
                }
                let shown = (&p.username, p.uses_nickname, false);
                let given = p.renamed_to.as_ref().map(|name| (name, false, true));
                for (name, nickname, at_match_end) in [Some(shown), given].into_iter().flatten() {
                    match e.names.iter_mut().find(|n| n.username == *name) {
                        Some(n) => {
                            n.matches += 1;
                            n.last_seen = when;
                            n.nickname |= nickname;
                            n.at_match_end |= at_match_end;
                        }
                        None => e.names.push(NameUse {
                            username: name.clone(),
                            nickname,
                            at_match_end,
                            matches: 1,
                            first_seen: when,
                            last_seen: when,
                        }),
                    }
                }
                match p.relation {
                    Some(Relation::You) => {
                        e.recorded += 1;
                        *recorders.entry(p.key.clone()).or_default() += 1;
                    }
                    Some(Relation::Teammate) => e.with_you += 1,
                    Some(Relation::Opponent) => e.against_you += 1,
                    None => {}
                }
                if p.party.is_some() && p.relation != Some(Relation::You) {
                    e.party_with_you += 1;
                }
            }
        }
        let mut players: Vec<KnownPlayer> = players
            .into_values()
            .map(|mut p| {
                p.queue_mate = p.recorded == 0 && (p.party_with_you > 0 || p.with_you >= 2);
                p
            })
            .collect();
        players.sort_by(|a, b| b.matches.cmp(&a.matches).then_with(|| a.key.cmp(&b.key)));
        let mut you: Vec<(String, u32)> = recorders.into_iter().collect();
        you.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Self {
            you: you.into_iter().map(|(k, _)| k).collect(),
            matches: order.len(),
            players,
        }
    }

    pub fn get(&self, key: &str) -> Option<&KnownPlayer> {
        self.players.iter().find(|p| p.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::PartyRole;
    use crate::summary::{PlayerSummary, TeamSummary};

    fn player(key: &str, name: &str, relation: Relation, party: bool) -> PlayerSummary {
        PlayerSummary {
            username: name.into(),
            key: key.into(),
            profile_id: key.into(),
            relation: Some(relation),
            party: party.then_some(PartyRole::Member),
            ..PlayerSummary::default()
        }
    }

    fn game(id: &str, day: u32, players: Vec<PlayerSummary>) -> MatchSummary {
        let mut s = MatchSummary {
            match_id: id.into(),
            start_time: format!("2026-09-{day:02}T12:00:00Z").parse().unwrap(),
            ..MatchSummary::default()
        };
        s.teams[0] = TeamSummary {
            players,
            ..TeamSummary::default()
        };
        s
    }

    #[test]
    fn tracks_renames_and_queue_mates() {
        let a = game(
            "m1",
            1,
            vec![
                player("me", "Me", Relation::You, true),
                player("p1", "OldName", Relation::Teammate, false),
                player("p2", "Mate", Relation::Teammate, true),
                player("p3", "Enemy", Relation::Opponent, false),
            ],
        );
        let b = game(
            "m2",
            5,
            vec![
                player("me", "Me", Relation::You, false),
                player("p1", "NewName", Relation::Teammate, false),
            ],
        );
        // The same match imported twice counts once.
        let d = PlayerDirectory::new(&[b.clone(), a, b]);
        assert_eq!(d.matches, 2);
        assert_eq!(d.you, vec!["me".to_owned()]);
        let p1 = d.get("p1").unwrap();
        assert_eq!(p1.username, "NewName");
        let names: Vec<_> = p1.names.iter().map(|n| n.username.as_str()).collect();
        assert_eq!(names, ["OldName", "NewName"]);
        assert!(p1.queue_mate, "on your team twice");
        assert!(d.get("p2").unwrap().queue_mate, "in your party");
        assert!(!d.get("p3").unwrap().queue_mate);
        assert!(!d.get("me").unwrap().queue_mate);
        assert_eq!(d.get("p3").unwrap().against_you, 1);
    }

    #[test]
    fn a_nickname_gives_way_to_the_name_given_at_match_end() {
        let hidden = PlayerSummary {
            uses_nickname: true,
            renamed_to: Some("RealName".into()),
            platform: Some(Platform::Pc),
            ..player("p1", "CalmOtter", Relation::Opponent, false)
        };
        // The recording of the second match stops before its end.
        let unrevealed = PlayerSummary {
            uses_nickname: true,
            ..player("p1", "BoldHeron", Relation::Opponent, false)
        };
        let d =
            PlayerDirectory::new(&[game("m1", 1, vec![hidden]), game("m2", 2, vec![unrevealed])]);
        let p = d.get("p1").unwrap();
        assert_eq!(
            p.username, "BoldHeron",
            "the latest match has no other name"
        );
        assert_eq!(p.platform, Some(Platform::Pc));
        let names: Vec<_> = (p.names.iter())
            .map(|n| (n.username.as_str(), n.nickname, n.at_match_end))
            .collect();
        assert_eq!(
            names,
            [
                ("CalmOtter", true, false),
                ("RealName", false, true),
                ("BoldHeron", true, false)
            ]
        );
        let d = PlayerDirectory::new(&d_only_first());
        assert_eq!(d.get("p1").unwrap().username, "RealName");
    }

    fn d_only_first() -> Vec<MatchSummary> {
        let hidden = PlayerSummary {
            uses_nickname: true,
            renamed_to: Some("RealName".into()),
            ..player("p1", "CalmOtter", Relation::Opponent, false)
        };
        vec![game("m1", 1, vec![hidden])]
    }
}
