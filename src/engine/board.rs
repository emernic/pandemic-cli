use rand::Rng;
use rand::seq::SliceRandom;

use crate::state::{BoardMember, BoardPersonality, BoardRole, GameState, MAX_FIELD_RESEARCH};

/// Generate board members from existing game entities at game start.
/// Called after corporations and regions are initialized.
///
/// Board composition:
/// - 6 corporate leaders (randomly chosen — may stack in the same region)
/// - 2-3 governors who also sit on the board (dual-role creates strategic tension)
/// - Total: 8-9 members
pub(super) fn generate_board_members(state: &mut GameState) {
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
        let personality_idx = state.rng_misc.r#gen::<usize>() % BoardPersonality::ALL.len();
        let personality = BoardPersonality::ALL[personality_idx];
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
            personality: Some(personality),
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
            personality: None,
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

    // Initialize board budget from corporate tax revenue at current satisfaction.
    if state.board_budget_per_tick == 0.0 {
        let board_sat = state.board_satisfaction();
        state.board_budget_per_tick =
            crate::engine::crisis::compute_board_budget_per_tick(state, board_sat);
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

/// Stock performance component: share_price / ipo_price, clamped 0–1.
fn stock_performance(state: &GameState, corp_idx: usize) -> f64 {
    state.corporations.get(corp_idx)
        .map(|c| if c.bankrupt { 0.0 } else {
            (c.share_price / c.ipo_price).clamp(0.0, 1.0)
        })
        .unwrap_or(0.0)
}

/// Research pipeline utilization: fraction of research slots actively in use
/// across field, applied, and basic tracks.
fn research_utilization(state: &GameState) -> f64 {
    let field_active = state.field_research.len() as f64;
    let field_max = MAX_FIELD_RESEARCH as f64;
    let applied_active = if state.applied_research.is_some() { 1.0 } else { 0.0 };
    let basic_active = if state.basic_research.is_some() { 1.0 } else { 0.0 };
    let total_active = field_active + applied_active + basic_active;
    let total_max = field_max + 1.0 + 1.0;
    (total_active / total_max).clamp(0.0, 1.0)
}

/// Global survival rate: fraction of initial population still alive.
fn global_survival_rate(state: &GameState) -> f64 {
    let initial_pop = state.initial_population();
    if initial_pop <= 0.0 {
        0.0
    } else {
        let alive = initial_pop - state.total_dead();
        (alive / initial_pop).clamp(0.0, 1.0)
    }
}

/// Compute satisfaction for a single board member from game state.
/// Corporate leaders blend stock performance with personality-specific factors.
fn compute_member_satisfaction(state: &GameState, member_idx: usize) -> f64 {
    let member = &state.board_members[member_idx];
    match &member.role {
        BoardRole::CorporateLeader { corp_idx } => {
            let stock = stock_performance(state, *corp_idx);
            match member.personality {
                Some(BoardPersonality::Profiteer) | None => {
                    stock
                }
                Some(BoardPersonality::Technocrat) => {
                    0.6 * stock + 0.4 * research_utilization(state)
                }
                Some(BoardPersonality::Humanitarian) => {
                    0.5 * stock + 0.5 * global_survival_rate(state)
                }
                Some(BoardPersonality::Dealmaker) => {
                    let owns_shares = state.portfolio.get(*corp_idx)
                        .map_or(0.0, |&shares| if shares > 0 { 1.0 } else { 0.0 });
                    0.7 * stock + 0.3 * owns_shares
                }
            }
        }
        BoardRole::RegionGovernor { region_idx } => {
            state.regions.get(*region_idx)
                .map(|r| if r.collapsed { 0.0 } else { r.gdp_fraction() })
                .unwrap_or(0.0)
        }
        BoardRole::IndependentAdvisor => {
            global_survival_rate(state)
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

/// Amplified satisfaction boost for Dealmaker personality when player buys their corp's shares.
/// 2x the normal boost — Dealmakers care deeply about direct investment.
const INVEST_DEALMAKER_BOOST: f64 = 0.10;

/// Satisfaction penalty for Profiteer personality when GDP-hurting policy is enacted
/// in their corporation's region.
const PROFITEER_POLICY_PENALTY: f64 = 0.05;

/// Satisfaction boost for Technocrat personality when research completes
/// (medicine developed or basic research unlocked).
const TECHNOCRAT_RESEARCH_BOOST: f64 = 0.03;

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

    // Dealmaker chairman doubles stock trade reactions for ALL board members
    let dealmaker_mult = if state.chairman_personality() == Some(BoardPersonality::Dealmaker) {
        2.0
    } else {
        1.0
    };

    let mut pleased: Option<String> = None;
    let mut displeased_count = 0usize;

    for member in state.board_members.iter_mut() {
        let member_corp_idx = match member.role {
            BoardRole::CorporateLeader { corp_idx } => corp_idx,
            _ => continue,
        };
        if member_corp_idx == corp_idx {
            let boost = if member.personality == Some(BoardPersonality::Dealmaker) {
                INVEST_DEALMAKER_BOOST
            } else {
                INVEST_OWN_CORP_BOOST
            };
            member.satisfaction_modifier += boost * dealmaker_mult;
            pleased = Some(member.name.clone());
        } else if let Some(member_corp) = state.corporations.get(member_corp_idx) {
            if member_corp.sector == bought_sector {
                member.satisfaction_modifier -= INVEST_RIVAL_PENALTY * dealmaker_mult;
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
    // Dealmaker chairman doubles stock trade reactions for ALL board members
    let dealmaker_mult = if state.chairman_personality() == Some(BoardPersonality::Dealmaker) {
        2.0
    } else {
        1.0
    };
    let mut displeased: Option<String> = None;
    for member in state.board_members.iter_mut() {
        let member_corp_idx = match member.role {
            BoardRole::CorporateLeader { corp_idx } => corp_idx,
            _ => continue,
        };
        if member_corp_idx == corp_idx {
            member.satisfaction_modifier -= SELL_OWN_CORP_PENALTY * dealmaker_mult;
            displeased = Some(member.name.clone());
        }
    }
    displeased.map(|name| format!("{} disapproves.", name))
}

/// Apply satisfaction penalty to Profiteer board members when a GDP-hurting policy
/// is enacted in their corporation's region.
/// GDP-hurting policies: travel ban (0), quarantine (1), martial law (8).
pub(super) fn on_gdp_policy_enacted(state: &mut GameState, region_idx: usize) {
    for member in state.board_members.iter_mut() {
        if member.personality != Some(BoardPersonality::Profiteer) {
            continue;
        }
        if member.region_idx == Some(region_idx) {
            member.satisfaction_modifier -= PROFITEER_POLICY_PENALTY;
        }
    }
}

/// Apply satisfaction boost to Technocrat board members when research completes.
pub(super) fn on_research_completed(state: &mut GameState) {
    for member in state.board_members.iter_mut() {
        if member.personality == Some(BoardPersonality::Technocrat) {
            member.satisfaction_modifier += TECHNOCRAT_RESEARCH_BOOST;
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
        // Profiteers get 0.0, others blend stock (0.0 from bankruptcy) with other factors
        match leader.personality {
            Some(BoardPersonality::Profiteer) | None => {
                assert_eq!(leader.satisfaction, 0.0, "bankrupt Profiteer should have 0 satisfaction");
            }
            _ => {
                assert!(leader.satisfaction < 0.7, "bankrupt corp leader should have reduced satisfaction, got {}", leader.satisfaction);
            }
        }

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

        // Set GDP to 50% of base (simulating economic damage from disease + policies)
        let base = state.regions[region_idx].base_gdp;
        state.regions[region_idx].gdp = base * 0.5;
        update_board_satisfaction(&mut state);

        let sat = state.board_members[gov_idx].satisfaction;
        assert!((sat - 0.5).abs() < 0.01, "governor satisfaction should track GDP fraction ~0.5, got {sat}");
    }

    #[test]
    fn gdp_target_drops_with_containment_policies() {
        let state = GameState::new_default(42);
        let base_gdp = state.regions[0].base_gdp;
        // No policies: GDP target should be close to base_gdp (no disease yet)
        let target = state.gdp_target(0);
        assert!(target > base_gdp * 0.9, "baseline GDP target should be near base_gdp, got {target}");

        // With quarantine + travel ban: GDP target should drop significantly
        let mut state2 = state.clone();
        state2.policies[0].quarantine = true;
        state2.policies[0].travel_ban = true;
        let reduced = state2.gdp_target(0);
        // quarantine 0.80 × travel_ban 0.70 = 0.56 of base
        assert!(reduced < target * 0.60, "GDP target with containment should drop, got {reduced}");
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
