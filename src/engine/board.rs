use rand::Rng;
use rand::seq::SliceRandom;

use crate::state::{BoardMember, BoardRole, GameState};

/// Generate board members from existing game entities at game start.
/// Called after corporations and regions are initialized.
///
/// Board composition:
/// - 6 corporate leaders (randomly chosen — may stack in the same region)
/// - 2-3 governors who also sit on the board (dual-role creates strategic tension)
/// - Total: 8-9 members
pub fn generate_board_members(state: &mut GameState) {
    let mut members = Vec::new();
    let mut chairman_assigned = false;

    // Corporate leaders: one per board-seat corporation.
    // The first corporate leader becomes Chairman of the Board.
    for (corp_idx, corp) in state.corporations.iter().enumerate() {
        if !corp.board_seat {
            continue;
        }
        let is_chairman = !chairman_assigned;
        if is_chairman {
            chairman_assigned = true;
        }
        members.push(BoardMember {
            name: if is_chairman {
                format!("Chairman {}", corp.director_surname)
            } else {
                format!("Dir. {}", corp.director_surname)
            },
            title: if is_chairman {
                format!("Chairman of the Board, {}", corp.name)
            } else {
                format!("CEO, {}", corp.name)
            },
            role: BoardRole::CorporateLeader { corp_idx },
            corp_idx: Some(corp_idx),
            region_idx: Some(corp.region_idx),
            satisfaction: 1.0,
            satisfaction_modifier: 0.0,
            is_chairman,
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
            satisfaction_modifier: 0.0,
            is_chairman: false,
        });
        governor_count += 1;
    }

    state.board_members = members;

    // Schedule the first board meeting around day 7 ± 1 day of jitter.
    if state.next_board_meeting_tick == 0 {
        let base = (7.0 * crate::state::TICKS_PER_DAY) as u64;
        let jitter_range = (1.0 * crate::state::TICKS_PER_DAY) as u64;
        let jitter = state.rng_misc.r#gen::<u64>() % (jitter_range * 2 + 1);
        state.next_board_meeting_tick = base.saturating_sub(jitter_range) + jitter;
    }
}


/// Decay rate for satisfaction modifier: ~0.02/day = modifier halves in ~35 days.
const MODIFIER_DECAY_RATE: f64 = 0.02 / crate::state::TICKS_PER_DAY;

/// Chairman satisfaction threshold below which the hostility timer starts.
const CHAIRMAN_HOSTILE_THRESHOLD: f64 = 0.20;

/// Update each board member's satisfaction based on their connected entities
/// plus any relationship modifier from contract decisions.
/// Called once per tick from the main tick loop.
pub(super) fn update_board_satisfaction(state: &mut GameState) {
    for i in 0..state.board_members.len() {
        let base_sat = compute_member_satisfaction(&state, i);
        // Decay modifier toward 0
        let modifier = state.board_members[i].satisfaction_modifier;
        if modifier.abs() > 0.001 {
            let decay = modifier.signum() * MODIFIER_DECAY_RATE;
            state.board_members[i].satisfaction_modifier =
                if modifier.abs() <= MODIFIER_DECAY_RATE { 0.0 }
                else { modifier - decay };
        }
        state.board_members[i].satisfaction =
            (base_sat + state.board_members[i].satisfaction_modifier).clamp(0.0, 1.0);
    }

    // Track chairman hostility duration for Vote of No Confidence
    let chairman_sat = state.board_members.iter()
        .find(|m| m.is_chairman)
        .map(|m| m.satisfaction);
    match chairman_sat {
        Some(sat) if sat < CHAIRMAN_HOSTILE_THRESHOLD => {
            if state.chairman_hostile_since.is_none() {
                state.chairman_hostile_since = Some(state.tick);
            }
        }
        _ => {
            state.chairman_hostile_since = None;
        }
    }
}

/// Compute satisfaction for a single board member from game state.
fn compute_member_satisfaction(state: &GameState, member_idx: usize) -> f64 {
    let member = &state.board_members[member_idx];
    match &member.role {
        BoardRole::CorporateLeader { corp_idx } => {
            // Satisfaction tracks stock price relative to IPO price.
            // Stock price already incorporates revenue and reserves via fair
            // value calculation, with natural mean-reversion lag.
            state.corporations.get(*corp_idx)
                .map(|c| if c.bankrupt { 0.0 } else {
                    (c.share_price / c.ipo_price).clamp(0.0, 1.0)
                })
                .unwrap_or(0.0)
        }
        BoardRole::RegionGovernor { region_idx } => {
            // Satisfaction tracks regional GDP (0.0–1.0).
            // GDP accounts for disease burden, deaths, and active containment
            // policies — creating tension where containment saves lives but
            // tanks the economy and makes governors unhappy.
            state.regions.get(*region_idx)
                .map(|r| if r.collapsed { 0.0 } else { r.gdp.clamp(0.0, 1.0) })
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

/// Satisfaction boost when player buys shares in a board member's own corporation.
/// Scaled per 10-share block — a single buy gives +0.05, comparable to a minor contract favor.
const INVEST_OWN_CORP_BOOST: f64 = 0.05;

/// Satisfaction penalty when player buys shares in a rival corp (same sector, different company).
/// Smaller than the boost — board members notice slights less than favors.
const INVEST_RIVAL_PENALTY: f64 = 0.03;

/// Satisfaction penalty when player sells shares of a board member's own corporation.
/// A mild rebuke — selling is less provocative than investing in a rival.
const SELL_OWN_CORP_PENALTY: f64 = 0.03;

/// Apply board member satisfaction modifiers when player buys shares.
/// Board members with CorporateLeader role react:
/// - Positive when you invest in their corp
/// - Negative when you invest in a same-sector rival
/// Returns a short reaction hint for the transaction message.
pub(super) fn on_buy_shares(state: &mut GameState, corp_idx: usize) -> Option<String> {
    let bought_sector = match state.corporations.get(corp_idx) {
        Some(c) => c.sector,
        None => return None,
    };

    let mut pleased: Option<String> = None;
    let mut displeased_count = 0usize;

    for member in state.board_members.iter_mut() {
        let member_corp_idx = match member.role {
            BoardRole::CorporateLeader { corp_idx } => corp_idx,
            _ => continue,
        };
        if member_corp_idx == corp_idx {
            member.satisfaction_modifier += INVEST_OWN_CORP_BOOST;
            pleased = Some(member.name.clone());
        } else if let Some(member_corp) = state.corporations.get(member_corp_idx) {
            if member_corp.sector == bought_sector {
                member.satisfaction_modifier -= INVEST_RIVAL_PENALTY;
                displeased_count += 1;
            }
        }
    }

    match (pleased, displeased_count) {
        (Some(name), 0) => Some(format!("{} approves.", name)),
        (Some(name), n) => Some(format!("{} approves; {} rival{} displeased.", name, n, if n == 1 { "" } else { "s" })),
        (None, n) if n > 0 => Some(format!("{} sector rival{} displeased.", n, if n == 1 { "" } else { "s" })),
        _ => None,
    }
}

/// Apply board member satisfaction modifiers when player sells shares.
/// Only the member whose corp was sold reacts (negatively).
/// Returns a short reaction hint for the transaction message.
pub(super) fn on_sell_shares(state: &mut GameState, corp_idx: usize) -> Option<String> {
    let mut displeased: Option<String> = None;
    for member in state.board_members.iter_mut() {
        let member_corp_idx = match member.role {
            BoardRole::CorporateLeader { corp_idx } => corp_idx,
            _ => continue,
        };
        if member_corp_idx == corp_idx {
            member.satisfaction_modifier -= SELL_OWN_CORP_PENALTY;
            displeased = Some(member.name.clone());
        }
    }
    displeased.map(|name| format!("{} disapproves.", name))
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
    fn governor_member_satisfaction_tracks_gdp() {
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

        // Set GDP to 0.5 (simulating economic damage from disease + policies)
        state.regions[region_idx].gdp = 0.5;
        update_board_satisfaction(&mut state);

        let sat = state.board_members[gov_idx].satisfaction;
        assert!((sat - 0.5).abs() < 0.01, "governor satisfaction should track GDP ~0.5, got {sat}");
    }

    #[test]
    fn gdp_target_drops_with_containment_policies() {
        let state = GameState::new_default(42);
        // No policies: GDP target should be close to 1.0 (no disease yet)
        let base = state.gdp_target(0);
        assert!(base > 0.9, "baseline GDP target should be high, got {base}");

        // With quarantine + travel ban: GDP target should drop significantly
        let mut state2 = state.clone();
        state2.policies[0].quarantine = true;
        state2.policies[0].travel_ban = true;
        let reduced = state2.gdp_target(0);
        // quarantine 0.80 × travel_ban 0.70 = 0.56 of base
        assert!(reduced < base * 0.60, "GDP target with containment should drop, got {reduced}");
        assert!(reduced > 0.0, "GDP target should not be zero");
    }

    #[test]
    fn board_meeting_scheduled_around_day_7() {
        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        generate_board_members(&mut state);

        let ticks_per_day = crate::state::TICKS_PER_DAY as u64;
        let meeting_tick = state.next_board_meeting_tick;
        // Should be scheduled between day 6 and day 8
        assert!(meeting_tick >= 6 * ticks_per_day, "meeting too early: tick {meeting_tick}");
        assert!(meeting_tick <= 8 * ticks_per_day, "meeting too late: tick {meeting_tick}");
    }

    #[test]
    fn board_meeting_fires_on_schedule() {
        use crate::engine::tick;

        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        generate_board_members(&mut state);

        let meeting_tick = state.next_board_meeting_tick;
        let original_next = state.next_board_meeting_tick;

        // Advance until a board meeting fires or we pass the scheduled tick
        let max_tick = meeting_tick + 100; // small buffer
        let mut found_meeting = false;
        while state.tick <= max_tick {
            state = tick(&state);
            if let Some(ref crisis) = state.active_crisis {
                if crisis.kind.tag() == "board_meeting" {
                    found_meeting = true;
                    // Board meetings are now single-option communiqués
                    assert_eq!(crisis.options.len(), 1, "board communiqué should have 1 option");
                    assert_eq!(crisis.options[0].label, "Acknowledged");
                    assert_eq!(crisis.title, "Board Communiqué");
                    // Next meeting should have been rescheduled
                    assert!(state.next_board_meeting_tick > original_next,
                        "next meeting should be rescheduled after firing");
                    break;
                }
                // Auto-resolve non-meeting crises to keep advancing
                state.active_crisis = None;
                state.last_crisis_resolved_tick = state.tick;
            }
        }
        assert!(found_meeting, "board meeting should have fired by tick {max_tick}");
    }

    #[test]
    fn buying_own_corp_boosts_member_satisfaction() {
        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        generate_board_members(&mut state);

        // Find a corporate leader and their corp_idx
        let (member_idx, corp_idx) = state.board_members.iter().enumerate()
            .find_map(|(i, m)| match m.role {
                BoardRole::CorporateLeader { corp_idx } => Some((i, corp_idx)),
                _ => None,
            }).expect("should have a corporate leader");

        let before = state.board_members[member_idx].satisfaction_modifier;
        on_buy_shares(&mut state, corp_idx);
        let after = state.board_members[member_idx].satisfaction_modifier;

        assert!((after - before - INVEST_OWN_CORP_BOOST).abs() < 0.001,
            "buying own corp should boost modifier by {INVEST_OWN_CORP_BOOST}, got delta {}",
            after - before);
    }

    #[test]
    fn buying_rival_sector_penalizes_member() {
        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        generate_board_members(&mut state);

        // Find a corporate leader
        let (member_idx, member_corp_idx) = state.board_members.iter().enumerate()
            .find_map(|(i, m)| match m.role {
                BoardRole::CorporateLeader { corp_idx } => Some((i, corp_idx)),
                _ => None,
            }).expect("should have a corporate leader");

        let member_sector = state.corporations[member_corp_idx].sector;

        // Find a different corp in the same sector
        let rival_idx = state.corporations.iter().enumerate()
            .find(|(idx, c)| *idx != member_corp_idx && c.sector == member_sector)
            .map(|(idx, _)| idx);

        if let Some(rival_idx) = rival_idx {
            let before = state.board_members[member_idx].satisfaction_modifier;
            on_buy_shares(&mut state, rival_idx);
            let after = state.board_members[member_idx].satisfaction_modifier;

            assert!((after - before + INVEST_RIVAL_PENALTY).abs() < 0.001,
                "buying rival sector corp should penalize by {INVEST_RIVAL_PENALTY}, got delta {}",
                after - before);
        }
        // If no same-sector rival exists for this seed, the test passes vacuously
    }

    #[test]
    fn selling_own_corp_penalizes_member() {
        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        generate_board_members(&mut state);

        let (member_idx, corp_idx) = state.board_members.iter().enumerate()
            .find_map(|(i, m)| match m.role {
                BoardRole::CorporateLeader { corp_idx } => Some((i, corp_idx)),
                _ => None,
            }).expect("should have a corporate leader");

        let before = state.board_members[member_idx].satisfaction_modifier;
        on_sell_shares(&mut state, corp_idx);
        let after = state.board_members[member_idx].satisfaction_modifier;

        assert!((after - before + SELL_OWN_CORP_PENALTY).abs() < 0.001,
            "selling own corp should penalize by {SELL_OWN_CORP_PENALTY}, got delta {}",
            after - before);
    }

    #[test]
    fn buying_unrelated_sector_no_effect() {
        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        generate_board_members(&mut state);

        // Find a corporate leader
        let (member_idx, member_corp_idx) = state.board_members.iter().enumerate()
            .find_map(|(i, m)| match m.role {
                BoardRole::CorporateLeader { corp_idx } => Some((i, corp_idx)),
                _ => None,
            }).expect("should have a corporate leader");

        let member_sector = state.corporations[member_corp_idx].sector;

        // Find a corp in a DIFFERENT sector that also isn't this member's corp
        let unrelated_idx = state.corporations.iter().enumerate()
            .find(|(idx, c)| *idx != member_corp_idx && c.sector != member_sector)
            .map(|(idx, _)| idx);

        if let Some(unrelated_idx) = unrelated_idx {
            let before = state.board_members[member_idx].satisfaction_modifier;
            on_buy_shares(&mut state, unrelated_idx);
            let after = state.board_members[member_idx].satisfaction_modifier;

            assert!((after - before).abs() < 0.001,
                "buying unrelated sector should not affect this member, got delta {}",
                after - before);
        }
    }
}
