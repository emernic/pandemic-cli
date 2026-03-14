use rand::Rng;
use rand::seq::SliceRandom;

use crate::state::{BoardMember, BoardPersonality, BoardRole, GameState,
    GovernorPersonality, ModifierSource, SatisfactionModifier};

/// Generate board members from existing game entities at game start.
/// Called after corporations and regions are initialized.
///
/// Board composition:
/// - 4 corporate leaders (randomly chosen — may stack in the same region)
/// - 2 governors who also sit on the board (dual-role creates strategic tension)
/// - Total: 6 members
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
                format!("Dir., {}", corp.name)
            },
            role: BoardRole::CorporateLeader { corp_idx },
            corp_idx: Some(corp_idx),
            region_idx: Some(corp.region_idx),
            satisfaction: 1.0,
            modifiers: vec![SatisfactionModifier {
                source: ModifierSource::InitialSkepticism,
                value: -INITIAL_SKEPTICISM,
            }],
            is_chairman,
            personality: Some(personality),
            dead: false,
        });
    }

    // Governor-members: randomly select 2-3 governors as dual-role board members.
    let mut region_indices: Vec<usize> = (0..state.regions.len()).collect();
    region_indices.shuffle(&mut state.rng_misc);

    let max_governor_members = 2;
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
            modifiers: vec![SatisfactionModifier {
                source: ModifierSource::InitialSkepticism,
                value: -INITIAL_SKEPTICISM,
            }],
            is_chairman: false,
            personality: None,
            dead: false,
        });
        governor_count += 1;
    }

    state.board_members = members;

    // Compute initial satisfaction so modifiers are populated from tick 0
    update_board_satisfaction(state);

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

/// Initial skepticism penalty applied to all board members at game start.
/// Represents the board not yet trusting the player (~30% satisfaction hit).
const INITIAL_SKEPTICISM: f64 = 0.30;

/// Rate at which initial skepticism wanes: ~1%/day, gone by ~day 30.
const SKEPTICISM_DECAY_RATE: f64 = 0.01 / crate::state::TICKS_PER_DAY;

/// Chairman satisfaction threshold below which the hostility timer starts.
const CHAIRMAN_HOSTILE_THRESHOLD: f64 = 0.20;

/// Update each board member's satisfaction based on named modifiers.
/// Continuous modifiers (Base, Stock, GDP, etc.) are cleared and recomputed.
/// Event-driven modifiers (trades, contracts, crises) decay toward 0.
/// Called once per tick from the main tick loop.
pub(super) fn update_board_satisfaction(state: &mut GameState) {
    // Pre-compute shared values outside the member loop
    let research_util = research_utilization(state);
    let survival_rate = global_survival_rate(state);

    for i in 0..state.board_members.len() {
        if state.board_members[i].dead {
            continue;
        }
        // 1. Remove continuous modifiers (they'll be recomputed fresh)
        state.board_members[i].modifiers.retain(|m| !m.source.is_continuous());

        // 2. Decay event-driven modifiers toward 0
        for m in state.board_members[i].modifiers.iter_mut() {
            let rate = if m.source == ModifierSource::InitialSkepticism {
                SKEPTICISM_DECAY_RATE
            } else {
                MODIFIER_DECAY_RATE
            };
            if m.value.abs() > rate {
                m.value -= m.value.signum() * rate;
            } else {
                m.value = 0.0;
            }
        }
        // Remove near-zero modifiers
        state.board_members[i].modifiers.retain(|m| m.value.abs() > 0.001);

        // 3. Add continuous modifiers based on role and personality
        let member = &state.board_members[i];
        let mut continuous = vec![SatisfactionModifier {
            source: ModifierSource::Base,
            value: 0.50,
        }];

        match &member.role {
            BoardRole::CorporateLeader { corp_idx } => {
                let stock = stock_performance(state, *corp_idx);
                match member.personality {
                    Some(BoardPersonality::Profiteer) | None => {
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::StockPerformance,
                            value: stock - 0.50,
                        });
                    }
                    Some(BoardPersonality::Technocrat) => {
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::StockPerformance,
                            value: 0.6 * stock - 0.30,
                        });
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::ResearchUtilization,
                            value: 0.4 * research_util - 0.20,
                        });
                    }
                    Some(BoardPersonality::Humanitarian) => {
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::StockPerformance,
                            value: 0.5 * stock - 0.25,
                        });
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::GlobalSurvival,
                            value: 0.5 * survival_rate - 0.25,
                        });
                    }
                    Some(BoardPersonality::Dealmaker) => {
                        let owns_shares = state.portfolio.get(*corp_idx)
                            .map_or(0.0, |&shares| if shares > 0 { 1.0 } else { 0.0 });
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::StockPerformance,
                            value: 0.7 * stock - 0.35,
                        });
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::PlayerInvestment,
                            value: 0.3 * owns_shares - 0.15,
                        });
                    }
                }
            }
            BoardRole::RegionGovernor { region_idx } => {
                let region = state.regions.get(*region_idx);
                let gdp = region
                    .map(|r| if r.collapsed { 0.0 } else { r.gdp_fraction() })
                    .unwrap_or(0.0);
                let gov_personality = region.map(|r| r.governor.personality);

                match gov_personality {
                    Some(GovernorPersonality::Blowhard) => {
                        // Blowhard: GDP matters less, hates restrictive policies.
                        // 60% GDP weight + 40% policy freedom weight.
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::RegionalGdp,
                            value: 0.6 * gdp - 0.30,
                        });
                        let restrictive = state.policies.get(*region_idx)
                            .map(|p| [p.travel_ban, p.quarantine, p.martial_law, p.border_controls]
                                .iter().filter(|&&b| b).count() as f64)
                            .unwrap_or(0.0);
                        // 0 policies = +0.20, 4 policies = -0.20
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::RestrictivePolicies,
                            value: 0.20 - restrictive * 0.10,
                        });
                    }
                    Some(GovernorPersonality::Hardliner) => {
                        // Zero-sum nationalist. Cares about their region's GDP
                        // relative to other regions. Pleased when competitors suffer.
                        // 50% own GDP + 50% relative standing vs others.
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::RegionalGdp,
                            value: 0.5 * gdp - 0.25,
                        });
                        // Compare own GDP fraction to average of other regions
                        let other_gdps: Vec<f64> = state.regions.iter().enumerate()
                            .filter(|(j, _)| *j != *region_idx)
                            .map(|(_, r)| if r.collapsed { 0.0 } else { r.gdp_fraction() })
                            .collect();
                        let avg_other = if other_gdps.is_empty() { 0.5 } else {
                            other_gdps.iter().sum::<f64>() / other_gdps.len() as f64
                        };
                        // Positive when own GDP > average, negative when below
                        let standing = (gdp - avg_other).clamp(-0.25, 0.25);
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::RegionalStanding,
                            value: standing,
                        });
                    }
                    Some(GovernorPersonality::Operative) => {
                        // Operative: GDP matters, also likes active contracts (deal flow).
                        // 70% GDP + 30% contract activity.
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::RegionalGdp,
                            value: 0.7 * gdp - 0.35,
                        });
                        let active_contracts = state.contracts.len() as f64;
                        // 0 contracts = -0.15, 1 = 0.0, 2+ = +0.15
                        let contract_score = (active_contracts * 0.15 - 0.15).clamp(-0.15, 0.15);
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::ActiveContracts,
                            value: contract_score,
                        });
                    }
                    Some(GovernorPersonality::Mobster) => {
                        // Mobster: GDP matters less, wants to see fat funding reserves.
                        // 50% GDP + 50% funding level.
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::RegionalGdp,
                            value: 0.5 * gdp - 0.25,
                        });
                        let daily_income = state.funding_income_rate() * crate::state::TICKS_PER_DAY;
                        let reserve_ratio = if daily_income > 0.0 {
                            (state.resources.funding / daily_income).clamp(0.0, 10.0)
                        } else {
                            0.0
                        };
                        // 0 days reserve = -0.25, 5+ days = +0.25
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::FundingReserves,
                            value: (reserve_ratio / 10.0 * 0.50 - 0.25).clamp(-0.25, 0.25),
                        });
                    }
                    _ => {
                        // Buffoon, Recluse, or unknown: pure GDP driver
                        continuous.push(SatisfactionModifier {
                            source: ModifierSource::RegionalGdp,
                            value: gdp - 0.50,
                        });
                    }
                }
            }
        }

        // Insert continuous modifiers at the front so they display first
        let event_modifiers: Vec<SatisfactionModifier> =
            state.board_members[i].modifiers.drain(..).collect();
        state.board_members[i].modifiers = continuous;
        state.board_members[i].modifiers.extend(event_modifiers);

        // 4. Compute satisfaction as sum of all modifiers
        let total: f64 = state.board_members[i].modifiers.iter().map(|m| m.value).sum();
        state.board_members[i].satisfaction = total.clamp(0.0, 1.0);
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

/// Research pipeline utilization: fraction of available research being pursued.
/// Computed as active / (active + available). If the player is researching
/// everything they can, this returns 1.0. If nothing is researchable yet, 1.0.
fn research_utilization(state: &GameState) -> f64 {
    let active = state.active_research.len() as f64;
    let available = state.all_available_projects().len() as f64;
    let total = active + available;
    if total == 0.0 {
        1.0 // nothing researchable — no reason for the board to complain
    } else {
        active / total
    }
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
            member.add_modifier(ModifierSource::BoughtShares, boost * dealmaker_mult);
            pleased = Some(member.name.clone());
        } else if let Some(member_corp) = state.corporations.get(member_corp_idx) {
            if member_corp.sector == bought_sector {
                member.add_modifier(ModifierSource::RivalInvestment, -INVEST_RIVAL_PENALTY * dealmaker_mult);
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
            member.add_modifier(ModifierSource::SoldShares, -SELL_OWN_CORP_PENALTY * dealmaker_mult);
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
            member.add_modifier(ModifierSource::PolicyEnacted, -PROFITEER_POLICY_PENALTY);
        }
    }
}

/// Apply satisfaction boost to Technocrat board members when research completes.
pub(super) fn on_research_completed(state: &mut GameState) {
    for member in state.board_members.iter_mut() {
        if member.personality == Some(BoardPersonality::Technocrat) {
            member.add_modifier(ModifierSource::ResearchCompleted, TECHNOCRAT_RESEARCH_BOOST);
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

        // Should have 4 corporate leaders + 2 governor members = 6
        assert!(!state.board_members.is_empty());
        let corp_leaders: Vec<_> = state.board_members.iter()
            .filter(|m| matches!(m.role, BoardRole::CorporateLeader { .. }))
            .collect();
        let gov_members: Vec<_> = state.board_members.iter()
            .filter(|m| matches!(m.role, BoardRole::RegionGovernor { .. }))
            .collect();

        assert_eq!(corp_leaders.len(), 4, "should have 4 corporate leaders");
        assert_eq!(gov_members.len(), 2, "should have 2 governor members");
    }

    #[test]
    fn board_satisfaction_matches_old_aggregate() {
        // After generation, corporate leaders should have satisfaction 1.0 minus
        // initial skepticism (0.30), so board_satisfaction() should return ~0.70
        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        generate_board_members(&mut state);
        // Run one tick to compute satisfaction with skepticism
        update_board_satisfaction(&mut state);

        let sat = state.board_satisfaction();
        // Governor personality modifiers can shift aggregate slightly from 0.70;
        // allow wider tolerance since governor personalities now have secondary drivers
        assert!((sat - 0.70).abs() < 0.10, "initial board satisfaction should be ~0.70 with skepticism, got {sat}");
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

        // Record satisfaction at full GDP
        update_board_satisfaction(&mut state);
        let sat_full = state.board_members[gov_idx].satisfaction;

        // Set GDP to 50% of base (simulating economic damage from disease + policies)
        let base = state.regions[region_idx].base_gdp;
        state.regions[region_idx].gdp = base * 0.5;
        update_board_satisfaction(&mut state);

        let sat_half = state.board_members[gov_idx].satisfaction;
        // Regardless of personality weighting, lower GDP should reduce satisfaction
        assert!(sat_half < sat_full, "governor satisfaction should drop with GDP: full={sat_full}, half={sat_half}");
        // And it should be noticeably lower (GDP is a significant driver for all governor personalities)
        assert!(sat_full - sat_half > 0.10, "GDP drop should meaningfully reduce satisfaction: full={sat_full}, half={sat_half}");
    }

    #[test]
    fn hardliner_governor_board_member_reacts_to_relative_standing() {
        use crate::state::GovernorPersonality;
        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);

        // Force a specific governor to be Hardliner and place them on the board
        state.regions[0].governor.personality = GovernorPersonality::Hardliner;
        generate_board_members(&mut state);

        // Find the governor member for region 0 (if they were selected)
        let gov_idx = state.board_members.iter().position(|m| {
            matches!(m.role, BoardRole::RegionGovernor { region_idx: 0 })
        });
        if gov_idx.is_none() {
            return;
        }
        let gov_idx = gov_idx.unwrap();

        // Should have a RegionalStanding modifier
        update_board_satisfaction(&mut state);
        let has_standing = state.board_members[gov_idx].modifiers.iter()
            .any(|m| m.source == ModifierSource::RegionalStanding);
        assert!(has_standing, "Hardliner should have RegionalStanding modifier");
        let sat_even = state.board_members[gov_idx].satisfaction;

        // Trash other regions' GDP — Hardliner should be happier (their region looks better)
        for i in 1..state.regions.len() {
            state.regions[i].gdp = state.regions[i].base_gdp * 0.3;
        }
        update_board_satisfaction(&mut state);
        let sat_others_down = state.board_members[gov_idx].satisfaction;

        assert!(sat_others_down > sat_even,
            "Hardliner should be happier when competing regions suffer: even={sat_even}, others_down={sat_others_down}");
    }

    #[test]
    fn blowhard_governor_board_member_reacts_to_policies() {
        use crate::state::GovernorPersonality;
        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);

        state.regions[0].governor.personality = GovernorPersonality::Blowhard;
        generate_board_members(&mut state);

        let gov_idx = state.board_members.iter().position(|m| {
            matches!(m.role, BoardRole::RegionGovernor { region_idx: 0 })
        });
        if gov_idx.is_none() {
            return;
        }
        let gov_idx = gov_idx.unwrap();

        // No policies
        update_board_satisfaction(&mut state);
        let sat_free = state.board_members[gov_idx].satisfaction;

        // Activate restrictive policies
        state.policies[0].quarantine = true;
        state.policies[0].travel_ban = true;
        state.policies[0].border_controls = true;
        update_board_satisfaction(&mut state);
        let sat_restricted = state.board_members[gov_idx].satisfaction;

        assert!(sat_restricted < sat_free, "Blowhard satisfaction should drop with restrictions: free={sat_free}, restricted={sat_restricted}");
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

        on_buy_shares(&mut state, corp_idx);
        let total = state.board_members[member_idx].modifier_total(&ModifierSource::BoughtShares);

        assert!((total - INVEST_OWN_CORP_BOOST).abs() < 0.001,
            "buying own corp should add BoughtShares modifier of {INVEST_OWN_CORP_BOOST}, got {}",
            total);
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
            on_buy_shares(&mut state, rival_idx);
            let total = state.board_members[member_idx].modifier_total(&ModifierSource::RivalInvestment);

            assert!((total + INVEST_RIVAL_PENALTY).abs() < 0.001,
                "buying rival sector corp should add RivalInvestment modifier of -{INVEST_RIVAL_PENALTY}, got {}",
                total);
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

        on_sell_shares(&mut state, corp_idx);
        let total = state.board_members[member_idx].modifier_total(&ModifierSource::SoldShares);

        assert!((total + SELL_OWN_CORP_PENALTY).abs() < 0.001,
            "selling own corp should add SoldShares modifier of -{SELL_OWN_CORP_PENALTY}, got {}",
            total);
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
            let before_count = state.board_members[member_idx].modifiers.len();
            on_buy_shares(&mut state, unrelated_idx);
            let after_count = state.board_members[member_idx].modifiers.len();

            assert_eq!(before_count, after_count,
                "buying unrelated sector should not add any modifier");
        }
    }
}
