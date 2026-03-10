use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::state::{
    FundingCondition, FundingContract, GameEvent, GameState,
    CONTRACT_FIRST_OFFER_TICK, CONTRACT_OFFER_INTERVAL, MAX_CONTRACTS, TICKS_PER_DAY,
};

/// Contract templates. Each has a unique template_id (0-based).
/// Income values are per-tick (multiply by 120 for per-day).
const TEMPLATES: &[(&str, f64, FundingCondition, &str)] = &[
    // High-value: forbid powerful tools
    (
        "Global Trade Alliance",
        2.5,
        FundingCondition::ForbidPolicy { policy_idx: 0 }, // No travel bans
        "Shipping conglomerates require open borders.",
    ),
    (
        "Civil Liberties Coalition",
        2.0,
        FundingCondition::ForbidPolicy { policy_idx: 1 }, // No quarantine
        "Human rights groups oppose forced quarantine.",
    ),
    // Medium-value: require commitments
    (
        "Pharma Research Grant",
        1.8,
        FundingCondition::ActiveResearch,
        "Pharmaceutical consortium funds active research programs.",
    ),
    (
        "Regional Stability Pact",
        2.0,
        FundingCondition::NoCollapse,
        "International investors require all regions operational.",
    ),
    // Threshold-based: lost when situation deteriorates
    (
        "Media Transparency Pledge",
        1.8,
        FundingCondition::MaxThreatLevel { level: 3 }, // DEFCON 3 or better
        "News networks fund coverage while crisis appears manageable.",
    ),
    (
        "Population Welfare Fund",
        1.5,
        FundingCondition::MaxDeaths { threshold: 500_000_000.0 },
        "Humanitarian aid contingent on limiting casualties.",
    ),
    // Policy-requiring: force spending
    (
        "Hospital Workers Union",
        1.5,
        FundingCondition::RequirePolicy { policy_idx: 2 }, // Hospital Surge
        "Medical unions fund surge capacity programs.",
    ),
    (
        "Border Security Consortium",
        1.5,
        FundingCondition::RequirePolicy { policy_idx: 3 }, // Border Controls
        "Security firms subsidize border monitoring.",
    ),
];

/// Build a FundingContract from a template index, with ±20% income variance.
fn build_contract(template_id: u8, rng: &mut ChaCha8Rng) -> FundingContract {
    let t = &TEMPLATES[template_id as usize];
    let variance = 0.8 + rng.r#gen::<f64>() * 0.4; // 0.8 to 1.2
    FundingContract {
        name: t.0.to_string(),
        income: t.1 * variance,
        condition: t.2.clone(),
        source: t.3.to_string(),
        template_id,
    }
}

/// Check active contract conditions and revoke any that are violated.
pub fn tick_check_contracts(state: &mut GameState) {
    // Collect indices to revoke (can't borrow state mutably and immutably simultaneously)
    let to_revoke: Vec<(usize, String, String)> = state.contracts.iter().enumerate()
        .filter(|(_, c)| !c.condition.is_met(state))
        .map(|(i, c)| (i, c.name.clone(), c.condition.description()))
        .collect();

    // Remove in reverse order to preserve indices
    for (i, name, reason) in to_revoke.iter().rev() {
        state.contracts.remove(*i);
        state.events.push(GameEvent::ContractRevoked {
            name: name.clone(),
            reason: reason.clone(),
        });
    }
}

/// Generate a new contract offer if enough time has passed and slots are available.
pub fn tick_offer_contracts(state: &mut GameState, rng: &mut ChaCha8Rng) {
    // Don't offer if already have a pending offer
    if state.contract_offer.is_some() {
        return;
    }
    // Don't offer if at max contracts
    if state.contracts.len() >= MAX_CONTRACTS {
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

    // Pick a template that isn't already active and whose condition is currently met
    let active_ids: Vec<u8> = state.contracts.iter().map(|c| c.template_id).collect();
    let eligible: Vec<u8> = (0..TEMPLATES.len() as u8)
        .filter(|id| !active_ids.contains(id))
        .collect();

    if eligible.is_empty() {
        return;
    }

    let pick = eligible[rng.r#gen::<usize>() % eligible.len()];
    let contract = build_contract(pick, rng);

    // Only offer if condition is currently met (don't offer dead-on-arrival contracts)
    if !contract.condition.is_met(state) {
        return;
    }

    state.events.push(GameEvent::ContractOffered {
        name: contract.name.clone(),
    });
    state.contract_offer = Some(contract);
    state.last_contract_offer_tick = state.tick;
}

/// Accept the current contract offer. Returns (success, message).
pub fn accept_contract(state: &mut GameState) -> (bool, Option<String>) {
    if let Some(contract) = state.contract_offer.take() {
        if state.contracts.len() >= MAX_CONTRACTS {
            state.contract_offer = Some(contract);
            return (false, Some("Maximum contracts reached.".to_string()));
        }
        let income_per_day = contract.income * TICKS_PER_DAY;
        let msg = format!("Accepted: {} (+¥{:.0}/day)", contract.name, income_per_day);
        state.contracts.push(contract);
        (true, Some(msg))
    } else {
        (false, Some("No contract offer available.".to_string()))
    }
}

/// Reject (dismiss) the current contract offer.
pub fn reject_contract(state: &mut GameState) -> (bool, Option<String>) {
    if state.contract_offer.take().is_some() {
        (true, Some("Contract offer dismissed.".to_string()))
    } else {
        (false, Some("No contract offer to reject.".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn make_offer(state: &mut GameState) {
        let contract = FundingContract {
            name: "Test Contract".to_string(),
            income: 2.0,
            condition: FundingCondition::MaxThreatLevel { level: 3 },
            source: "Test source".to_string(),
            template_id: 4,
        };
        state.contract_offer = Some(contract);
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
        // Fill to max
        for i in 0..MAX_CONTRACTS {
            state.contracts.push(FundingContract {
                name: format!("Contract {i}"),
                income: 1.0,
                condition: FundingCondition::NoCollapse,
                source: String::new(),
                template_id: i as u8,
            });
        }
        make_offer(&mut state);

        let (ok, msg) = accept_contract(&mut state);
        assert!(!ok);
        assert!(msg.unwrap().contains("Maximum"));
        // Offer should still be there
        assert!(state.contract_offer.is_some());
    }

    #[test]
    fn violated_contract_gets_revoked() {
        let mut state = GameState::new_default(42);
        // Add a contract requiring DEFCON <= 3
        state.contracts.push(FundingContract {
            name: "Fragile Deal".to_string(),
            income: 2.0,
            condition: FundingCondition::MaxThreatLevel { level: 3 },
            source: String::new(),
            template_id: 4,
        });
        // Escalate threat beyond level 3 (DEFCON 2 = Catastrophe)
        state.threat_level = crate::state::ThreatLevel::Catastrophe;

        tick_check_contracts(&mut state);

        assert!(state.contracts.is_empty(), "Contract should be revoked");
        assert!(state.events.iter().any(|e| matches!(e, GameEvent::ContractRevoked { .. })));
    }

    #[test]
    fn satisfied_contract_survives_check() {
        let mut state = GameState::new_default(42);
        state.contracts.push(FundingContract {
            name: "Stable Deal".to_string(),
            income: 2.0,
            condition: FundingCondition::NoCollapse,
            source: String::new(),
            template_id: 3,
        });
        // No regions collapsed — condition is met
        tick_check_contracts(&mut state);
        assert_eq!(state.contracts.len(), 1);
    }

    #[test]
    fn offer_generated_at_first_offer_tick() {
        let mut state = GameState::new_default(42);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // Before threshold — no offer
        state.tick = CONTRACT_FIRST_OFFER_TICK - 1;
        tick_offer_contracts(&mut state, &mut rng);
        assert!(state.contract_offer.is_none());

        // At threshold — should get an offer
        state.tick = CONTRACT_FIRST_OFFER_TICK;
        tick_offer_contracts(&mut state, &mut rng);
        assert!(state.contract_offer.is_some(), "Should offer contract at tick {}", CONTRACT_FIRST_OFFER_TICK);
    }

    #[test]
    fn no_offer_when_at_max_contracts() {
        let mut state = GameState::new_default(42);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        state.tick = CONTRACT_FIRST_OFFER_TICK;

        for i in 0..MAX_CONTRACTS {
            state.contracts.push(FundingContract {
                name: format!("C{i}"),
                income: 1.0,
                condition: FundingCondition::NoCollapse,
                source: String::new(),
                template_id: i as u8,
            });
        }

        tick_offer_contracts(&mut state, &mut rng);
        assert!(state.contract_offer.is_none());
    }

    #[test]
    fn no_duplicate_offer_while_pending() {
        let mut state = GameState::new_default(42);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        state.tick = CONTRACT_FIRST_OFFER_TICK;

        tick_offer_contracts(&mut state, &mut rng);
        let first_offer = state.contract_offer.as_ref().unwrap().name.clone();

        // Try again — should not replace existing offer
        state.tick += CONTRACT_OFFER_INTERVAL;
        tick_offer_contracts(&mut state, &mut rng);
        assert_eq!(state.contract_offer.as_ref().unwrap().name, first_offer);
    }
}
