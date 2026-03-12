use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::state::{
    CrisisKind, FundingCondition, FundingContract, GameEvent, GameState,
    CONTRACT_FIRST_OFFER_TICK, CONTRACT_OFFER_INTERVAL, MAX_CONTRACTS, TICKS_PER_DAY,
    CONTRACT_CONDITION_WARN, CONTRACT_CONDITION_REVOKE,
    CONTRACT_DEGRADE_RATE, CONTRACT_RECOVER_RATE, CONTRACT_DEMAND_COOLDOWN,
    SEVERITY_HIGH_THRESHOLD,
};

/// When a contract template becomes contextually relevant (worth offering).
/// Lives on the template so relevance rules can't drift out of sync with templates.
enum RelevanceCheck {
    /// Always relevant once the condition is met.
    Always,
    /// Spread has reached multiple regions or a large total count.
    MultiRegionalSpread,
    /// At least one non-collapsed region has significant infection prevalence (≥0.5%).
    SignificantInfectionRate,
    /// The specified emergency decree is unlocked.
    DecreeUnlocked(usize),
    /// At least one region has a high-severity infection.
    HighSeverityRisk,
    /// Total deaths have reached a threshold.
    MinDeaths(f64),
}

impl RelevanceCheck {
    fn is_relevant(&self, state: &GameState) -> bool {
        match self {
            RelevanceCheck::Always => true,
            RelevanceCheck::MultiRegionalSpread => {
                let infected_regions = state.regions.iter()
                    .filter(|r| !r.collapsed)
                    .filter(|r| r.infections.iter().any(|i| i.infected > 0.0))
                    .count();
                infected_regions >= 2 || state.total_infected() >= 10_000.0
            }
            RelevanceCheck::SignificantInfectionRate => {
                state.regions.iter().any(|r| {
                    !r.collapsed && {
                        let infected: f64 = r.infections.iter().map(|i| i.infected).sum();
                        r.population > 0 && infected / r.population as f64 >= 0.005
                    }
                })
            }
            RelevanceCheck::DecreeUnlocked(idx) => state.decree_unlocked(*idx),
            RelevanceCheck::HighSeverityRisk => {
                state.regions.iter().any(|r| {
                    !r.collapsed && r.infections.iter().any(|i| i.infected >= SEVERITY_HIGH_THRESHOLD)
                })
            }
            RelevanceCheck::MinDeaths(threshold) => state.total_dead() >= *threshold,
        }
    }
}

/// Contract template — conditions with attached income. Board member is assigned dynamically.
struct Template {
    name: &'static str,
    income: f64,
    condition: FundingCondition,
    relevance: RelevanceCheck,
}

/// Contract templates. Each has a unique template_id (0-based index).
/// Income values are per-tick (multiply by 120 for per-day).
/// The offering board member is chosen at runtime from the current board.
const TEMPLATES: &[Template] = &[
    Template {
        name: "Shipping Lane Guarantee",
        income: 2.5,
        condition: FundingCondition::ForbidPolicy { policy_idx: 0 }, // No travel bans
        relevance: RelevanceCheck::MultiRegionalSpread,
    },
    Template {
        name: "Hospitality Protection Fund",
        income: 2.0,
        condition: FundingCondition::ForbidPolicy { policy_idx: 1 }, // No quarantine
        relevance: RelevanceCheck::SignificantInfectionRate,
    },
    Template {
        name: "Research Independence Pact",
        income: 2.0,
        condition: FundingCondition::ForbidDecree { decree_idx: 0 }, // No Conscript Researchers
        relevance: RelevanceCheck::DecreeUnlocked(0),
    },
    Template {
        name: "Stability Assurance Fund",
        income: 2.0,
        condition: FundingCondition::NoCollapse,
        relevance: RelevanceCheck::HighSeverityRisk,
    },
    Template {
        name: "Confidence Fund",
        income: 1.8,
        condition: FundingCondition::MaxDeaths { threshold: 50_000_000.0 }, // under 50M dead
        relevance: RelevanceCheck::MinDeaths(250_000.0),
    },
    Template {
        name: "Actuarial Pact",
        income: 1.5,
        condition: FundingCondition::MaxDeaths { threshold: 500_000_000.0 },
        relevance: RelevanceCheck::MinDeaths(2_500_000.0),
    },
    Template {
        name: "Equipment Lease",
        income: 1.5,
        condition: FundingCondition::ForbidPolicy { policy_idx: 2 }, // Discourage Hospitalization
        relevance: RelevanceCheck::Always,
    },
    Template {
        name: "Border Security Contract",
        income: 1.5,
        condition: FundingCondition::RequirePolicy { policy_idx: 3 }, // Border Controls
        relevance: RelevanceCheck::Always,
    },
    Template {
        name: "Ethics Protocols Grant",
        income: 2.0,
        condition: FundingCondition::ForbidDecree { decree_idx: 1 }, // No Authorize Human Trials
        relevance: RelevanceCheck::DecreeUnlocked(1),
    },
];

/// Satisfaction boost given to the offering board member when a contract is accepted.
const ACCEPT_OFFERER_BOOST: f64 = 0.15;
/// Satisfaction penalty applied to every OTHER board member when a contract is accepted.
const ACCEPT_OTHERS_PENALTY: f64 = 0.08;
/// Satisfaction penalty applied to the offering board member when a contract is refused.
const REFUSE_OFFERER_PENALTY: f64 = 0.12;
/// Satisfaction penalty applied to the offering board member when an active contract is canceled.
const CANCEL_PENALTY: f64 = 0.20;

/// Per-decline price escalation: each prior decline of this template adds 20% to base price.
const DECLINE_ESCALATION: f64 = 0.20;
/// Time-based price escalation: price increases ~2% per game day.
const TIME_ESCALATION_PER_DAY: f64 = 0.02;

/// Build a FundingContract from a template index, assigned to a specific board member.
/// Prices escalate based on game day and how many times this template was previously declined.
fn build_contract(template_id: u8, board_member_idx: usize, state: &GameState, rng: &mut ChaCha8Rng) -> FundingContract {
    let t = &TEMPLATES[template_id as usize];
    let variance = 0.8 + rng.r#gen::<f64>() * 0.4; // 0.8 to 1.2

    let game_day = state.tick as f64 / TICKS_PER_DAY;
    let time_mult = 1.0 + game_day * TIME_ESCALATION_PER_DAY;
    let decline_count = state.contract_decline_counts
        .get(template_id as usize)
        .copied()
        .unwrap_or(0) as f64;
    let decline_mult = 1.0 + decline_count * DECLINE_ESCALATION;

    FundingContract {
        name: t.name.to_string(),
        board_member_idx,
        income: t.income * variance * time_mult * decline_mult,
        condition: t.condition.clone(),
        template_id,
        satisfaction: 1.0,
        warned: false,
        last_demand_tick: 0,
        accepted_tick: 0,
        loyalty_raise_offered: false,
    }
}

/// Whether a template is contextually relevant given the current game state.
/// Delegates to the template's own relevance check — no magic index mapping needed.
fn is_contextually_relevant(template_id: usize, state: &GameState) -> bool {
    TEMPLATES.get(template_id)
        .map(|t| t.relevance.is_relevant(state))
        .unwrap_or(true)
}

/// Minimum days a contract must be held before a loyalty raise can be offered.
const LOYALTY_RAISE_MIN_DAYS: f64 = 10.0;
/// Per-tick probability of a loyalty raise firing once eligible (~1.5%/tick ≈ ~84% within a day).
const LOYALTY_RAISE_CHANCE: f64 = 0.015;
/// Loyalty raise multiplier — income increases by this fraction (e.g. 0.15 = 15% raise).
pub(super) const LOYALTY_RAISE_FRACTION: f64 = 0.15;

/// Check contracts held long enough to trigger a loyalty raise offer.
pub(super) fn tick_loyalty_raises(state: &mut GameState, rng: &mut ChaCha8Rng) {
    let min_ticks = (LOYALTY_RAISE_MIN_DAYS * TICKS_PER_DAY) as u64;

    for contract in &mut state.contracts {
        if contract.loyalty_raise_offered {
            continue;
        }
        if contract.accepted_tick == 0 {
            continue; // Legacy contracts without accepted_tick set
        }
        let held_ticks = state.tick.saturating_sub(contract.accepted_tick);
        if held_ticks < min_ticks {
            continue;
        }
        if rng.r#gen::<f64>() < LOYALTY_RAISE_CHANCE {
            contract.loyalty_raise_offered = true;
            state.pending_crises.push((
                state.tick,
                CrisisKind::LoyaltyRaise { template_id: contract.template_id },
            ));
        }
    }
}

/// Tick contract condition satisfaction and revoke contracts when it bottoms out.
pub(super) fn tick_check_contracts(state: &mut GameState) {
    // First pass: compute satisfaction changes (need immutable borrow for is_met)
    let updates: Vec<(usize, bool)> = state.contracts.iter().enumerate()
        .map(|(i, c)| (i, c.condition.is_met(state)))
        .collect();

    // Second pass: apply satisfaction drift, warnings, and revocations
    let mut to_revoke: Vec<(usize, String, usize, String)> = Vec::new();

    for (i, met) in &updates {
        let c = &mut state.contracts[*i];
        if *met {
            c.satisfaction = (c.satisfaction + CONTRACT_RECOVER_RATE).min(1.0);
            // Reset warning flag when satisfaction recovers above threshold
            if c.satisfaction > CONTRACT_CONDITION_WARN + 0.1 {
                c.warned = false;
            }
        } else {
            c.satisfaction = (c.satisfaction - CONTRACT_DEGRADE_RATE).max(0.0);

            // Fire warning when crossing threshold
            if c.satisfaction <= CONTRACT_CONDITION_WARN && !c.warned {
                c.warned = true;
                let member_name = state.board_members.get(c.board_member_idx)
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| "Board member".to_string());
                state.events.push(GameEvent::ContractWarning {
                    member_name,
                    reason: c.condition.description(),
                });

                // Queue a contract demand crisis if cooldown has passed
                let cooldown_ok = c.last_demand_tick == 0
                    || state.tick.saturating_sub(c.last_demand_tick) >= CONTRACT_DEMAND_COOLDOWN;
                if cooldown_ok {
                    c.last_demand_tick = state.tick;
                    state.pending_crises.push((
                        state.tick,
                        CrisisKind::ContractDemand { template_id: c.template_id },
                    ));
                }
            }

            // Revoke when satisfaction bottoms out
            if c.satisfaction <= CONTRACT_CONDITION_REVOKE {
                to_revoke.push((
                    *i,
                    c.name.clone(),
                    c.board_member_idx,
                    c.condition.description(),
                ));
            }
        }
    }

    // Remove in reverse order to preserve indices
    for (i, name, member_idx, reason) in to_revoke.iter().rev() {
        state.contracts.remove(*i);
        let member_name = state.board_members.get(*member_idx)
            .map(|m| m.name.as_str())
            .unwrap_or("Board member");
        state.events.push(GameEvent::ContractRevoked {
            name: format!("{} pulled out: {}", member_name, name),
            reason: reason.clone(),
        });
    }
}

/// Generate a new contract offer if enough time has passed and slots are available.
/// The offering board member is chosen randomly from members who don't already have
/// an active contract.
pub(super) fn tick_offer_contracts(state: &mut GameState, rng: &mut ChaCha8Rng) {
    // Don't offer if already have a pending offer
    if state.contract_offer.is_some() {
        return;
    }
    // Don't offer if at max contracts
    if state.contracts.len() >= MAX_CONTRACTS {
        return;
    }
    // Need board members to source contracts from
    if state.board_members.is_empty() {
        return;
    }
    // Timing: first offer at day ~1, then every ~5 days
    let min_tick = if state.contracts.is_empty() && state.last_contract_offer_tick == 0 {
        CONTRACT_FIRST_OFFER_TICK
    } else {
        state.last_contract_offer_tick + CONTRACT_OFFER_INTERVAL
    };
    if state.tick < min_tick {
        return;
    }

    // Pick a template that isn't already active, whose condition type isn't already held
    // (type exclusivity), whose condition is met, and that is contextually relevant.
    let active_ids: Vec<u8> = state.contracts.iter().map(|c| c.template_id).collect();
    let active_types: Vec<&str> = state.contracts.iter()
        .map(|c| c.condition.condition_type())
        .collect();
    let eligible: Vec<u8> = (0..TEMPLATES.len() as u8)
        .filter(|id| !active_ids.contains(id))
        .filter(|id| !active_types.contains(&TEMPLATES[*id as usize].condition.condition_type()))
        .filter(|id| TEMPLATES[*id as usize].condition.is_met(state))
        .filter(|id| is_contextually_relevant(*id as usize, state))
        .collect();

    if eligible.is_empty() {
        return;
    }

    // Pick a board member who doesn't already have an active contract
    let members_with_contracts: Vec<usize> = state.contracts.iter()
        .map(|c| c.board_member_idx)
        .collect();
    let eligible_members: Vec<usize> = (0..state.board_members.len())
        .filter(|idx| !members_with_contracts.contains(idx))
        .collect();
    if eligible_members.is_empty() {
        return;
    }

    let pick = eligible[rng.r#gen::<usize>() % eligible.len()];
    let member_idx = eligible_members[rng.r#gen::<usize>() % eligible_members.len()];
    let contract = build_contract(pick, member_idx, state, rng);

    let template_id = contract.template_id;
    state.events.push(GameEvent::ContractOffered {
        name: contract.name.clone(),
    });
    state.contract_offer = Some(contract);
    state.last_contract_offer_tick = state.tick;

    // Queue a crisis-style interrupt so the player must respond to the offer.
    // Fires on the same tick (pending_crises check runs after this function).
    state.pending_crises.push((state.tick, CrisisKind::ContractOffer { template_id }));
}

/// Accept the current contract offer. Returns (success, message).
/// Accepting boosts the offering board member's satisfaction and penalizes all others.
/// Called from crisis resolution (ContractOffer) and unit tests.
pub(super) fn accept_contract(state: &mut GameState) -> (bool, Option<String>) {
    if let Some(contract) = state.contract_offer.take() {
        if state.contracts.len() >= MAX_CONTRACTS {
            state.contract_offer = Some(contract);
            return (false, Some("Maximum contracts reached.".to_string()));
        }
        let income_per_day = contract.income * TICKS_PER_DAY;
        let offerer_idx = contract.board_member_idx;
        let offerer_name = state.board_members.get(offerer_idx)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "Board member".to_string());

        // Board politics: accepting one member's contract angers the rest.
        // Uses satisfaction_modifier so the effect persists across entity-driven updates.
        for (i, member) in state.board_members.iter_mut().enumerate() {
            if i == offerer_idx {
                member.satisfaction_modifier += ACCEPT_OFFERER_BOOST;
            } else {
                member.satisfaction_modifier -= ACCEPT_OTHERS_PENALTY;
            }
        }

        let msg = format!(
            "Accepted {}'s {}: +¥{:.0}/day. Other board members are displeased.",
            offerer_name, contract.name, income_per_day,
        );
        let mut contract = contract;
        contract.accepted_tick = state.tick;
        state.contracts.push(contract);
        (true, Some(msg))
    } else {
        (false, Some("No contract offer available.".to_string()))
    }
}

/// Reject (dismiss) the current contract offer.
/// Refusing penalizes the offering board member's satisfaction and records the decline
/// so future re-offers of the same template come at a higher price.
pub(super) fn reject_contract(state: &mut GameState) -> (bool, Option<String>) {
    if let Some(contract) = state.contract_offer.take() {
        let offerer_idx = contract.board_member_idx;
        let offerer_name = state.board_members.get(offerer_idx)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "Board member".to_string());

        // Refusing angers the offering board member
        if let Some(member) = state.board_members.get_mut(offerer_idx) {
            member.satisfaction_modifier -= REFUSE_OFFERER_PENALTY;
        }

        // Track the decline for price escalation on re-offers
        let tid = contract.template_id as usize;
        if state.contract_decline_counts.len() <= tid {
            state.contract_decline_counts.resize(tid + 1, 0);
        }
        state.contract_decline_counts[tid] = state.contract_decline_counts[tid].saturating_add(1);

        let msg = format!("{} is displeased — contract refused.", offerer_name);
        (true, Some(msg))
    } else {
        (false, Some("No contract offer to reject.".to_string()))
    }
}

/// Cancel an active contract by board member index.
/// Removes the contract, frees the slot, and penalizes the offering member's satisfaction.
pub(super) fn cancel_contract(state: &mut GameState, board_member_idx: usize) -> (bool, Option<String>) {
    let pos = state.contracts.iter().position(|c| c.board_member_idx == board_member_idx);
    if let Some(idx) = pos {
        let contract = state.contracts.remove(idx);
        let member_name = state.board_members.get(board_member_idx)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "Board member".to_string());

        // Penalize the offering member for breaking the deal
        if let Some(member) = state.board_members.get_mut(board_member_idx) {
            member.satisfaction_modifier -= CANCEL_PENALTY;
        }

        let msg = format!(
            "Canceled {}. {} is upset.",
            contract.name, member_name,
        );
        (true, Some(msg))
    } else {
        (false, Some("No active contract with this board member.".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn make_test_contract(condition: FundingCondition) -> FundingContract {
        FundingContract {
            name: "Test Contract".to_string(),
            board_member_idx: 0,
            income: 2.0,
            condition,
            template_id: 4,
            satisfaction: 1.0,
            warned: false,
            last_demand_tick: 0,
            accepted_tick: 0,
            loyalty_raise_offered: false,
        }
    }

    fn make_offer(state: &mut GameState) {
        state.contract_offer = Some(make_test_contract(
            FundingCondition::MaxDeaths { threshold: 50_000_000.0 },
        ));
    }

    #[test]
    fn accept_contract_adds_income_and_clears_offer() {
        let mut state = GameState::new_default(42);
        let income_before = state.funding_income_rate();
        make_offer(&mut state);

        let (ok, msg) = accept_contract(&mut state);
        assert!(ok);
        assert!(msg.unwrap().contains("Accepted"));
        assert!(state.contract_offer.is_none());
        assert_eq!(state.contracts.len(), 1);

        let income_after = state.funding_income_rate();
        assert!(income_after > income_before,
            "Income should increase: {income_before} -> {income_after}");
    }

    #[test]
    fn reject_contract_clears_offer_without_adding() {
        let mut state = GameState::new_default(42);
        make_offer(&mut state);

        let (ok, _) = reject_contract(&mut state);
        assert!(ok);
        assert!(state.contract_offer.is_none());
        assert!(state.contracts.is_empty());
    }

    #[test]
    fn accept_fails_when_no_offer() {
        let mut state = GameState::new_default(42);
        let (ok, _) = accept_contract(&mut state);
        assert!(!ok);
    }

    #[test]
    fn accept_blocked_at_max_contracts() {
        let mut state = GameState::new_default(42);
        for i in 0..MAX_CONTRACTS {
            state.contracts.push(FundingContract {
                name: format!("Contract {i}"),
                board_member_idx: 0,
                income: 1.0,
                condition: FundingCondition::NoCollapse,
                template_id: i as u8,
                satisfaction: 1.0,
                warned: false,
                last_demand_tick: 0,
                accepted_tick: 0,
                loyalty_raise_offered: false,
            });
        }
        make_offer(&mut state);

        let (ok, msg) = accept_contract(&mut state);
        assert!(!ok);
        assert!(msg.unwrap().contains("Maximum"));
        assert!(state.contract_offer.is_some());
    }

    #[test]
    fn satisfaction_degrades_when_condition_violated() {
        let mut state = GameState::new_default(42);
        let mut c = make_test_contract(FundingCondition::MaxDeaths { threshold: 50_000_000.0 });
        c.satisfaction = 1.0;
        state.contracts.push(c);

        // Escalate threat beyond level 3
        state.regions[0].dead = 60_000_000.0; // exceed 50M death threshold

        // Run several ticks — satisfaction should degrade but not instant-revoke
        for _ in 0..100 {
            tick_check_contracts(&mut state);
        }
        assert_eq!(state.contracts.len(), 1, "Should not be revoked after only 100 ticks");
        assert!(state.contracts[0].satisfaction < 1.0, "Satisfaction should have dropped");
        assert!(state.contracts[0].satisfaction > CONTRACT_CONDITION_REVOKE,
            "Should not have hit revocation yet");
    }

    #[test]
    fn satisfaction_recovers_when_condition_restored() {
        let mut state = GameState::new_default(42);
        let mut c = make_test_contract(FundingCondition::NoCollapse);
        c.satisfaction = 0.5; // Start at warning level
        state.contracts.push(c);

        // Condition is met (no collapses) — satisfaction should recover
        for _ in 0..600 {
            tick_check_contracts(&mut state);
        }
        assert!(state.contracts[0].satisfaction > 0.5, "Satisfaction should recover");
    }

    #[test]
    fn warning_fires_at_threshold() {
        let mut state = GameState::new_default(42);
        let mut c = make_test_contract(FundingCondition::MaxDeaths { threshold: 50_000_000.0 });
        c.satisfaction = CONTRACT_CONDITION_WARN + CONTRACT_DEGRADE_RATE * 0.5; // Just above warning
        state.contracts.push(c);
        state.regions[0].dead = 60_000_000.0; // exceed 50M death threshold

        // One tick should push below warning threshold
        tick_check_contracts(&mut state);

        assert!(state.contracts[0].warned);
        assert!(state.events.iter().any(|e|
            matches!(e, GameEvent::ContractWarning { .. })
        ));
    }

    #[test]
    fn revocation_at_low_satisfaction() {
        let mut state = GameState::new_default(42);
        let mut c = make_test_contract(FundingCondition::MaxDeaths { threshold: 50_000_000.0 });
        c.satisfaction = CONTRACT_CONDITION_REVOKE + 0.0001; // Just above revocation
        c.warned = true;
        state.contracts.push(c);
        state.regions[0].dead = 60_000_000.0; // exceed 50M death threshold

        tick_check_contracts(&mut state);

        assert!(state.contracts.is_empty(), "Contract should be revoked");
        assert!(state.events.iter().any(|e| matches!(e, GameEvent::ContractRevoked { .. })));
    }

    #[test]
    fn satisfied_contract_survives_check() {
        let mut state = GameState::new_default(42);
        state.contracts.push(FundingContract {
            name: "Stable Deal".to_string(),
            board_member_idx: 0,
            income: 2.0,
            condition: FundingCondition::NoCollapse,
            template_id: 3,
            satisfaction: 1.0,
            warned: false,
            last_demand_tick: 0,
            accepted_tick: 0,
            loyalty_raise_offered: false,
        });
        tick_check_contracts(&mut state);
        assert_eq!(state.contracts.len(), 1);
        // Satisfaction should stay at 1.0 (capped)
        assert!((state.contracts[0].satisfaction - 1.0).abs() < 0.001);
    }

    /// Set up board members for tests that need them (tick_offer_contracts requires board members)
    fn setup_board(state: &mut GameState) {
        crate::engine::corporations::generate_corporations(state);
        crate::engine::board::generate_board_members(state);
    }

    #[test]
    fn offer_generated_at_first_offer_tick() {
        let mut state = GameState::new_default(42);
        setup_board(&mut state);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        // Set deaths so MaxDeaths templates are contextually relevant
        state.regions[0].dead = 3_000_000.0;

        state.tick = CONTRACT_FIRST_OFFER_TICK - 1;
        tick_offer_contracts(&mut state, &mut rng);
        assert!(state.contract_offer.is_none());

        state.tick = CONTRACT_FIRST_OFFER_TICK;
        tick_offer_contracts(&mut state, &mut rng);
        assert!(state.contract_offer.is_some());
        // Verify offer is linked to a valid board member
        let offer = state.contract_offer.as_ref().unwrap();
        assert!(offer.board_member_idx < state.board_members.len(),
            "Offer should reference a valid board member");
    }

    #[test]
    fn no_offer_when_at_max_contracts() {
        let mut state = GameState::new_default(42);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        state.tick = CONTRACT_FIRST_OFFER_TICK;

        for i in 0..MAX_CONTRACTS {
            state.contracts.push(FundingContract {
                name: format!("C{i}"),
                board_member_idx: 0,
                income: 1.0,
                condition: FundingCondition::NoCollapse,
                template_id: i as u8,
                satisfaction: 1.0,
                warned: false,
                last_demand_tick: 0,
                accepted_tick: 0,
                loyalty_raise_offered: false,
            });
        }

        tick_offer_contracts(&mut state, &mut rng);
        assert!(state.contract_offer.is_none());
    }

    #[test]
    fn no_duplicate_offer_while_pending() {
        let mut state = GameState::new_default(42);
        setup_board(&mut state);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        state.tick = CONTRACT_FIRST_OFFER_TICK;
        // Set deaths so MaxDeaths templates are contextually relevant
        state.regions[0].dead = 3_000_000.0;

        tick_offer_contracts(&mut state, &mut rng);
        let first_offer = state.contract_offer.as_ref().unwrap().name.clone();

        state.tick += CONTRACT_OFFER_INTERVAL;
        tick_offer_contracts(&mut state, &mut rng);
        assert_eq!(state.contract_offer.as_ref().unwrap().name, first_offer);
    }

    #[test]
    fn warning_queues_contract_demand_crisis() {
        let mut state = GameState::new_default(42);
        state.tick = 500;
        let mut c = make_test_contract(FundingCondition::MaxDeaths { threshold: 50_000_000.0 });
        c.satisfaction = CONTRACT_CONDITION_WARN + CONTRACT_DEGRADE_RATE * 0.5;
        state.contracts.push(c);
        state.regions[0].dead = 60_000_000.0; // exceed 50M death threshold

        assert!(state.pending_crises.is_empty());
        tick_check_contracts(&mut state);

        // Warning should fire AND a contract demand crisis should be queued
        assert!(state.contracts[0].warned);
        assert_eq!(state.pending_crises.len(), 1);
        assert!(matches!(
            state.pending_crises[0].1,
            CrisisKind::ContractDemand { template_id: 4 }
        ));
        assert_eq!(state.contracts[0].last_demand_tick, 500);
    }

    #[test]
    fn contract_demand_cooldown_prevents_repeat() {
        let mut state = GameState::new_default(42);
        state.tick = 1000;
        let mut c = make_test_contract(FundingCondition::MaxDeaths { threshold: 50_000_000.0 });
        // Already warned recently, satisfaction recovered and dropped again
        c.satisfaction = CONTRACT_CONDITION_WARN + CONTRACT_DEGRADE_RATE * 0.5;
        c.last_demand_tick = 800; // Only 200 ticks ago, cooldown is 600
        state.contracts.push(c);
        state.regions[0].dead = 60_000_000.0; // exceed 50M death threshold

        tick_check_contracts(&mut state);

        // Warning fires, but no demand crisis due to cooldown
        assert!(state.contracts[0].warned);
        assert!(state.pending_crises.is_empty(),
            "Contract demand should not fire within cooldown period");
    }

    #[test]
    fn contract_demand_fires_after_cooldown_expires() {
        let mut state = GameState::new_default(42);
        state.tick = 1000;
        let mut c = make_test_contract(FundingCondition::MaxDeaths { threshold: 50_000_000.0 });
        c.satisfaction = CONTRACT_CONDITION_WARN + CONTRACT_DEGRADE_RATE * 0.5;
        c.last_demand_tick = 300; // 700 ticks ago, cooldown is 600
        state.contracts.push(c);
        state.regions[0].dead = 60_000_000.0; // exceed 50M death threshold

        tick_check_contracts(&mut state);

        assert_eq!(state.pending_crises.len(), 1);
        assert_eq!(state.contracts[0].last_demand_tick, 1000);
    }

    #[test]
    fn offer_queues_crisis_interrupt() {
        let mut state = GameState::new_default(42);
        setup_board(&mut state);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        state.tick = CONTRACT_FIRST_OFFER_TICK;
        // Set deaths so MaxDeaths templates are contextually relevant
        state.regions[0].dead = 3_000_000.0;

        assert!(state.pending_crises.is_empty());
        tick_offer_contracts(&mut state, &mut rng);

        assert!(state.contract_offer.is_some(), "offer should be stored");
        assert_eq!(state.pending_crises.len(), 1, "should queue a crisis");
        assert!(matches!(
            state.pending_crises[0].1,
            CrisisKind::ContractOffer { .. }
        ));
    }

    #[test]
    fn contract_crisis_accept_adds_to_contracts() {
        use crate::engine::crisis;
        let mut state = GameState::new_default(42);
        make_offer(&mut state);
        let offer_name = state.contract_offer.as_ref().unwrap().name.clone();

        // Build and activate the crisis
        let crisis_event = crisis::build_crisis_event(
            &state,
            CrisisKind::ContractOffer { template_id: 4 },
        );
        state.active_crisis = Some(crisis_event);

        // Resolve with option 0 (accept)
        let msg = crisis::resolve_crisis(&mut state, 0);
        assert!(msg.contains("Accepted"), "msg: {msg}");
        assert!(state.contract_offer.is_none(), "offer should be consumed");
        assert_eq!(state.contracts.len(), 1);
        assert_eq!(state.contracts[0].name, offer_name);
    }

    #[test]
    fn contract_crisis_decline_clears_offer() {
        use crate::engine::crisis;
        let mut state = GameState::new_default(42);
        make_offer(&mut state);

        let crisis_event = crisis::build_crisis_event(
            &state,
            CrisisKind::ContractOffer { template_id: 4 },
        );
        state.active_crisis = Some(crisis_event);

        let msg = crisis::resolve_crisis(&mut state, 1);
        assert!(msg.contains("displeased") || msg.contains("refused"), "msg: {msg}");
        assert!(state.contract_offer.is_none(), "offer should be cleared");
        assert!(state.contracts.is_empty());
    }

    #[test]
    fn accept_boosts_offerer_penalizes_others() {
        let mut state = GameState::new_default(42);
        setup_board(&mut state);

        // Create offer from board member 0
        let mut offer = make_test_contract(FundingCondition::NoCollapse);
        offer.board_member_idx = 0;
        state.contract_offer = Some(offer);

        let (ok, msg) = accept_contract(&mut state);
        assert!(ok, "should accept");
        assert!(msg.unwrap().contains("displeased"), "should mention others are displeased");

        // Offerer should have positive modifier
        assert!((state.board_members[0].satisfaction_modifier - ACCEPT_OFFERER_BOOST).abs() < 0.01,
            "offerer modifier should be +{}", ACCEPT_OFFERER_BOOST);
        // Others should have negative modifier
        for (i, m) in state.board_members.iter().enumerate() {
            if i != 0 {
                assert!((m.satisfaction_modifier - (-ACCEPT_OTHERS_PENALTY)).abs() < 0.01,
                    "member {} modifier should be -{}, got {}", i, ACCEPT_OTHERS_PENALTY, m.satisfaction_modifier);
            }
        }
    }

    #[test]
    fn refuse_penalizes_offerer_only() {
        let mut state = GameState::new_default(42);
        setup_board(&mut state);

        let mut offer = make_test_contract(FundingCondition::NoCollapse);
        offer.board_member_idx = 1;
        state.contract_offer = Some(offer);

        let (ok, _) = reject_contract(&mut state);
        assert!(ok);

        // Offerer (idx 1) should have negative modifier
        assert!((state.board_members[1].satisfaction_modifier - (-REFUSE_OFFERER_PENALTY)).abs() < 0.01);

        // Others should have zero modifier
        assert!((state.board_members[0].satisfaction_modifier).abs() < 0.01);
        assert!((state.board_members[2].satisfaction_modifier).abs() < 0.01);
    }

    #[test]
    fn decline_records_template_for_escalation() {
        let mut state = GameState::new_default(42);
        make_offer(&mut state); // template_id = 4
        assert!(state.contract_decline_counts.is_empty());

        reject_contract(&mut state);
        assert_eq!(state.contract_decline_counts.get(4).copied(), Some(1));

        // Decline again — count should increment
        make_offer(&mut state);
        reject_contract(&mut state);
        assert_eq!(state.contract_decline_counts[4], 2);
    }

    #[test]
    fn escalating_prices_increase_with_declines() {
        let mut state = GameState::new_default(42);
        let mut rng = ChaCha8Rng::seed_from_u64(99);

        // Build contract with no declines
        let c1 = build_contract(4, 0, &state, &mut rng);
        let income1 = c1.income;

        // Record 2 declines
        state.contract_decline_counts.resize(9, 0);
        state.contract_decline_counts[4] = 2;
        let mut rng2 = ChaCha8Rng::seed_from_u64(99); // same seed for same variance
        let c2 = build_contract(4, 0, &state, &mut rng2);
        let income2 = c2.income;

        assert!(income2 > income1,
            "Income should increase after declines: {income1} vs {income2}");
    }

    #[test]
    fn escalating_prices_increase_with_game_day() {
        let mut state = GameState::new_default(42);

        // Day 0
        let mut rng1 = ChaCha8Rng::seed_from_u64(99);
        let c1 = build_contract(4, 0, &state, &mut rng1);

        // Day 30
        state.tick = (30.0 * TICKS_PER_DAY) as u64;
        let mut rng2 = ChaCha8Rng::seed_from_u64(99);
        let c2 = build_contract(4, 0, &state, &mut rng2);

        assert!(c2.income > c1.income,
            "Day 30 income should exceed day 0: {} vs {}", c1.income, c2.income);
    }

    #[test]
    fn type_exclusivity_blocks_same_condition_type() {
        let mut state = GameState::new_default(42);
        setup_board(&mut state);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        state.tick = CONTRACT_FIRST_OFFER_TICK;

        // Accept a ForbidPolicy contract (template 0 = Shipping Lane Guarantee)
        state.contracts.push(FundingContract {
            name: "Shipping Lane Guarantee".to_string(),
            board_member_idx: 0,
            income: 2.5,
            condition: FundingCondition::ForbidPolicy { policy_idx: 0 },
            template_id: 0,
            satisfaction: 1.0,
            warned: false,
            last_demand_tick: 0,
            accepted_tick: 0,
            loyalty_raise_offered: false,
        });

        // Set deaths so MaxDeaths templates are relevant
        state.regions[0].dead = 3_000_000.0;

        // Try to generate an offer — it should NOT offer any other ForbidPolicy templates
        // (templates 1, 6 are also ForbidPolicy)
        for _ in 0..50 {
            state.contract_offer = None;
            state.last_contract_offer_tick = 0;
            tick_offer_contracts(&mut state, &mut rng);
            if let Some(ref offer) = state.contract_offer {
                let offer_type = TEMPLATES[offer.template_id as usize].condition.condition_type();
                assert_ne!(offer_type, "forbid_policy",
                    "Should not offer ForbidPolicy when one is already active (got template {})",
                    offer.template_id);
            }
        }
    }

    #[test]
    fn cancel_contract_removes_and_penalizes() {
        let mut state = GameState::new_default(42);
        setup_board(&mut state);

        // Add a contract from board member 0
        state.contracts.push(FundingContract {
            name: "Test Lease".to_string(),
            board_member_idx: 0,
            income: 2.0,
            condition: FundingCondition::NoCollapse,
            template_id: 3,
            satisfaction: 1.0,
            warned: false,
            last_demand_tick: 0,
            accepted_tick: 0,
            loyalty_raise_offered: false,
        });
        assert_eq!(state.contracts.len(), 1);

        let (ok, msg) = cancel_contract(&mut state, 0);
        assert!(ok);
        assert!(msg.unwrap().contains("Canceled"));
        assert_eq!(state.contracts.len(), 0);
        assert!((state.board_members[0].satisfaction_modifier - (-CANCEL_PENALTY)).abs() < 0.01,
            "offerer should receive cancel penalty");
    }

    #[test]
    fn cancel_contract_fails_without_contract() {
        let mut state = GameState::new_default(42);
        setup_board(&mut state);

        let (ok, msg) = cancel_contract(&mut state, 0);
        assert!(!ok);
        assert!(msg.unwrap().contains("No active contract"));
    }

    #[test]
    fn loyalty_raise_not_offered_before_min_days() {
        let mut state = GameState::new_default(42);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let accepted = 100;
        state.contracts.push(FundingContract {
            name: "Test".to_string(),
            board_member_idx: 0,
            income: 2.0,
            condition: FundingCondition::NoCollapse,
            template_id: 0,
            satisfaction: 1.0,
            warned: false,
            last_demand_tick: 0,
            accepted_tick: accepted,
            loyalty_raise_offered: false,
        });

        // Set tick to just before eligibility (10 days = 10 * TICKS_PER_DAY)
        state.tick = accepted + (LOYALTY_RAISE_MIN_DAYS * TICKS_PER_DAY) as u64 - 1;
        for _ in 0..100 {
            tick_loyalty_raises(&mut state, &mut rng);
        }
        assert!(!state.contracts[0].loyalty_raise_offered);
        assert!(state.pending_crises.is_empty());
    }

    #[test]
    fn loyalty_raise_offered_after_min_days() {
        let mut state = GameState::new_default(42);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let accepted = 100;
        state.contracts.push(FundingContract {
            name: "Test".to_string(),
            board_member_idx: 0,
            income: 2.0,
            condition: FundingCondition::NoCollapse,
            template_id: 0,
            satisfaction: 1.0,
            warned: false,
            last_demand_tick: 0,
            accepted_tick: accepted,
            loyalty_raise_offered: false,
        });

        // Set tick well past eligibility
        state.tick = accepted + (LOYALTY_RAISE_MIN_DAYS * TICKS_PER_DAY) as u64 + 1000;
        // Run enough ticks that the probability check should fire at least once
        for _ in 0..200 {
            tick_loyalty_raises(&mut state, &mut rng);
        }
        assert!(state.contracts[0].loyalty_raise_offered,
            "Loyalty raise should have been offered after enough ticks past eligibility");
        assert!(state.pending_crises.iter().any(|(_, k)|
            matches!(k, CrisisKind::LoyaltyRaise { template_id: 0 })
        ));
    }

    #[test]
    fn loyalty_raise_only_fires_once_per_contract() {
        let mut state = GameState::new_default(42);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        state.contracts.push(FundingContract {
            name: "Test".to_string(),
            board_member_idx: 0,
            income: 2.0,
            condition: FundingCondition::NoCollapse,
            template_id: 0,
            satisfaction: 1.0,
            warned: false,
            last_demand_tick: 0,
            accepted_tick: 100,
            loyalty_raise_offered: true, // Already offered
        });
        state.tick = 100 + (LOYALTY_RAISE_MIN_DAYS * TICKS_PER_DAY) as u64 + 5000;

        for _ in 0..200 {
            tick_loyalty_raises(&mut state, &mut rng);
        }
        assert!(state.pending_crises.is_empty(),
            "Should not offer a second loyalty raise");
    }

    #[test]
    fn loyalty_raise_crisis_accept_increases_income() {
        use crate::engine::crisis;
        let mut state = GameState::new_default(42);
        setup_board(&mut state);
        state.contracts.push(FundingContract {
            name: "Test".to_string(),
            board_member_idx: 0,
            income: 2.0,
            condition: FundingCondition::NoCollapse,
            template_id: 0,
            satisfaction: 1.0,
            warned: false,
            last_demand_tick: 0,
            accepted_tick: 100,
            loyalty_raise_offered: true,
        });

        let income_before = state.contracts[0].income;

        let crisis_event = crisis::build_crisis_event(
            &state,
            CrisisKind::LoyaltyRaise { template_id: 0 },
        );
        state.active_crisis = Some(crisis_event);

        let msg = crisis::resolve_crisis(&mut state, 0);
        assert!(msg.contains("raises") || msg.contains("increased"), "msg: {msg}");
        let expected = income_before * (1.0 + LOYALTY_RAISE_FRACTION);
        assert!((state.contracts[0].income - expected).abs() < 0.01,
            "Income should increase by {}%: {income_before} -> expected {expected}, got {}",
            LOYALTY_RAISE_FRACTION * 100.0, state.contracts[0].income);
    }

    #[test]
    fn loyalty_raise_crisis_decline_no_change() {
        use crate::engine::crisis;
        let mut state = GameState::new_default(42);
        setup_board(&mut state);
        state.contracts.push(FundingContract {
            name: "Test".to_string(),
            board_member_idx: 0,
            income: 2.0,
            condition: FundingCondition::NoCollapse,
            template_id: 0,
            satisfaction: 1.0,
            warned: false,
            last_demand_tick: 0,
            accepted_tick: 100,
            loyalty_raise_offered: true,
        });

        let income_before = state.contracts[0].income;

        let crisis_event = crisis::build_crisis_event(
            &state,
            CrisisKind::LoyaltyRaise { template_id: 0 },
        );
        state.active_crisis = Some(crisis_event);

        let msg = crisis::resolve_crisis(&mut state, 1);
        assert!(msg.contains("unchanged") || msg.contains("nods"), "msg: {msg}");
        assert!((state.contracts[0].income - income_before).abs() < 0.001,
            "Income should be unchanged after declining");
    }
}
