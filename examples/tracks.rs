//! Prints how each player's movement read: `cargo run --example tracks -- R01.rec`.

use replay_analyzer::{ReadMode, ReadOptions, Round};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for path in std::env::args().skip(1) {
        let options = ReadOptions {
            mode: ReadMode::Full,
            census: false,
            movement: true,
        };
        let round = Round::open(&path, options)?;
        println!("{path}");
        let Some(m) = &round.movement else { continue };
        for t in &m.players {
            let top = t.speed.iter().copied().fold(0.0, f32::max);
            println!(
                "  {:<16} samples {:>5} unread {} top {:.2} m/s stance {} lean {} aim {} gait {} doing {:?}",
                t.username,
                t.time.len(),
                t.unread,
                top,
                t.stance.len(),
                t.lean.len(),
                t.aiming.len(),
                t.gait.len(),
                t.doing.iter().map(|c| c.value).collect::<Vec<_>>(),
            );
        }
        let owned = m.views.iter().filter(|v| v.owner.is_some() || v.fixed);
        println!(
            "  views {} with an owner or fixed {}",
            m.views.len(),
            owned.count()
        );
        for v in m
            .views
            .iter()
            .filter(|v| v.owner.is_none() && !v.fixed)
            .take(3)
        {
            println!("    {v:?}");
        }
        let kinds = |k| m.placements.iter().filter(|p| p.kind == k).count();
        use replay_analyzer::movement::PlacedKind::*;
        println!(
            "  placed: {} reinforcements, {} barricades, {} gadgets",
            kinds(Reinforcement),
            kinds(Barricade),
            kinds(Gadget)
        );
    }
    Ok(())
}
