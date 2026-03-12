use rand::Rng;
use rand::seq::SliceRandom;

use crate::state::{BoardMember, BoardRole, GameState};

/// Generate board members from existing game entities at game start.
/// Called after corporations and regions are initialized.
///
/// Board composition:
/// - 6 corporate leaders (one per board-seat corporation)
/// - 2-3 governors who also sit on the board (dual-role creates strategic tension)
/// - Total: 8-9 members
pub fn generate_board_members(state: &mut GameState) {
    let mut members = Vec::new();

    // Corporate leaders: one per board-seat corporation
    for (corp_idx, corp) in state.corporations.iter().enumerate() {
        if !corp.board_seat {
            continue;
        }
        let region_name = state.regions.get(corp.region_idx)
            .map(|r| r.name.as_str())
            .unwrap_or("Unknown");
        members.push(BoardMember {
            name: format!("Dir. {}", corp_short_surname(&corp.name)),
            title: format!("{} CEO, {} rep.", corp.name, region_name),
            role: BoardRole::CorporateLeader { corp_idx },
            corp_idx: Some(corp_idx),
            region_idx: Some(corp.region_idx),
            satisfaction: 1.0,
            last_demand_tick: 0,
        });
    }

    // Governor-members: randomly select 2-3 governors as dual-role board members.
    let mut region_indices: Vec<usize> = (0..state.regions.len()).collect();
    region_indices.shuffle(&mut state.rng_misc);

    let max_governor_members = 2 + (state.rng_misc.r#gen::<usize>() % 2); // 2 or 3
    let mut governor_count = 0;
    for &region_idx in &region_indices {
        if governor_count >= max_governor_members {
            break;
        }
        let region = &state.regions[region_idx];
        if region.collapsed {
            continue;
        }
        let gov_name = region.governor.name.clone();
        let region_name = region.name.clone();
        members.push(BoardMember {
            name: gov_name,
            title: format!("{} governor, board liaison", region_name),
            role: BoardRole::RegionGovernor { region_idx },
            corp_idx: None,
            region_idx: Some(region_idx),
            satisfaction: 1.0,
            last_demand_tick: 0,
        });
        governor_count += 1;
    }

    state.board_members = members;
}

/// Extract a short surname from a corporation name for the board member display name.
/// "Helion Power" -> "Helion", "Seraph Genomics" -> "Seraph", etc.
fn corp_short_surname(corp_name: &str) -> &str {
    corp_name.split_whitespace().next().unwrap_or(corp_name)
}

/// Update each board member's satisfaction based on their connected entities.
/// Called once per tick from the main tick loop.
pub(super) fn update_board_satisfaction(state: &mut GameState) {
    for i in 0..state.board_members.len() {
        let new_sat = compute_member_satisfaction(&state, i);
        state.board_members[i].satisfaction = new_sat;
    }
}

/// Compute satisfaction for a single board member from game state.
fn compute_member_satisfaction(state: &GameState, member_idx: usize) -> f64 {
    let member = &state.board_members[member_idx];
    match &member.role {
        BoardRole::CorporateLeader { corp_idx } => {
            // Satisfaction tracks corporation reserve health.
            // Bankrupt corps contribute 0.0.
            state.corporations.get(*corp_idx)
                .map(|c| if c.bankrupt { 0.0 } else { c.reserves_fraction() })
                .unwrap_or(0.0)
        }
        BoardRole::RegionGovernor { region_idx } => {
            // Satisfaction tracks region population health (alive fraction).
            // A collapsed region contributes 0.0.
            state.regions.get(*region_idx)
                .map(|r| {
                    if r.collapsed {
                        0.0
                    } else {
                        let pop = r.population as f64;
                        if pop <= 0.0 { 0.0 } else { (r.alive() / pop).clamp(0.0, 1.0) }
                    }
                })
                .unwrap_or(0.0)
        }
        BoardRole::IndependentAdvisor => {
            // Satisfaction tracks global survival: fraction of total population alive.
            let initial_pop = state.initial_population();
            if initial_pop <= 0.0 {
                0.0
            } else {
                let alive = initial_pop - state.total_dead();
                (alive / initial_pop).clamp(0.0, 1.0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::GameState;

    #[test]
    fn board_members_generated_from_corporations_and_governors() {
        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        generate_board_members(&mut state);

        // Should have 6 corporate leaders + 3 governor members = 9
        assert!(!state.board_members.is_empty());
        let corp_leaders: Vec<_> = state.board_members.iter()
            .filter(|m| matches!(m.role, BoardRole::CorporateLeader { .. }))
            .collect();
        let gov_members: Vec<_> = state.board_members.iter()
            .filter(|m| matches!(m.role, BoardRole::RegionGovernor { .. }))
            .collect();

        assert_eq!(corp_leaders.len(), 6, "should have 6 corporate leaders");
        assert!(gov_members.len() >= 2 && gov_members.len() <= 3,
            "should have 2-3 governor members, got {}", gov_members.len());
    }

    #[test]
    fn board_satisfaction_matches_old_aggregate() {
        // After generation, corporate leaders should have satisfaction 1.0
        // (full reserves), so board_satisfaction() should return ~1.0
        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        generate_board_members(&mut state);

        let sat = state.board_satisfaction();
        assert!(sat > 0.9, "initial board satisfaction should be high, got {sat}");
    }

    #[test]
    fn bankrupt_corp_reduces_member_satisfaction() {
        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        generate_board_members(&mut state);

        // Bankrupt the first board-seat corp
        let board_corp_idx = state.corporations.iter().position(|c| c.board_seat)
            .expect("should have a board-seat corp");
        state.corporations[board_corp_idx].bankrupt = true;
        update_board_satisfaction(&mut state);

        let leader = state.board_members.iter()
            .find(|m| matches!(m.role, BoardRole::CorporateLeader { corp_idx } if corp_idx == board_corp_idx))
            .expect("should have a board member for the bankrupted corp");
        assert_eq!(leader.satisfaction, 0.0, "bankrupt corp leader should have 0 satisfaction");

        // Overall board satisfaction should drop
        let sat = state.board_satisfaction();
        assert!(sat < 1.0, "board satisfaction should drop with bankrupt corp");
    }

    #[test]
    fn governor_member_satisfaction_tracks_region_health() {
        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        generate_board_members(&mut state);

        // Find a governor member
        let gov_idx = state.board_members.iter().position(|m| {
            matches!(m.role, BoardRole::RegionGovernor { .. })
        }).expect("should have governor members");

        let region_idx = match &state.board_members[gov_idx].role {
            BoardRole::RegionGovernor { region_idx } => *region_idx,
            _ => unreachable!(),
        };

        // Kill half the population
        state.regions[region_idx].dead = state.regions[region_idx].population as f64 / 2.0;
        update_board_satisfaction(&mut state);

        let sat = state.board_members[gov_idx].satisfaction;
        assert!((sat - 0.5).abs() < 0.01, "governor satisfaction should be ~0.5, got {sat}");
    }
}
