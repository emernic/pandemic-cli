use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::state::{
    FundingCondition, FundingContract, GameEvent, GameState,
    CONTRACT_FIRST_OFFER_TICK, CONTRACT_OFFER_INTERVAL, MAX_CONTRACTS, TICKS_PER_DAY,
    PATRON_SATISFACTION_WARN, PATRON_SATISFACTION_REVOKE,
    PATRON_DEGRADE_RATE, PATRON_RECOVER_RATE,
};

/// Contract template with named patron.
struct Template {
    name: &'static str,
    patron: &'static str,
    income: f64,
    condition: FundingCondition,
    source: &'static str,
}

/// Contract templates. Each has a unique template_id (0-based index).
/// Income values are per-tick (multiply by 120 for per-day).
const TEMPLATES: &[Template] = &[
    // High-value: forbid powerful tools
    Template {
        name: "Shipping Alliance",
        patron: "Elena Vasquez, Logistics Magnate",
        income: 2.5,
        condition: FundingCondition::ForbidPolicy { policy_idx: 0 }, // No travel bans
        source: "Open borders are non-negotiable.",
    },
    Template {
        name: "Civil Liberties Fund",
        patron: "Dr. Amara Osei, Human Rights Director",
        income: 2.0,
        condition: FundingCondition::ForbidPolicy { policy_idx: 1 }, // No quarantine
        source: "Quarantine is imprisonment by another name.",
    },
    // Medium-value: require commitments
    Template {
        name: "Pharma Research Grant",
        patron: "Dr. Henrik Lindqvist, Pharma Consortium",
        income: 1.8,
        condition: FundingCondition::ActiveResearch,
        source: "We fund results, not bureaucracy.",
    },
    Template {
        name: "Stability Investment Pact",
        patron: "James Chen, Global Investment Fund",
        income: 2.0,
        condition: FundingCondition::NoCollapse,
        source: "One region collapses, markets follow.",
    },
    // Threshold-based: lost when situation deteriorates
    Template {
        name: "Media Transparency Pledge",
        patron: "Sarah Kowalski, World Press Group",
        income: 1.8,
        condition: FundingCondition::MaxThreatLevel { level: 3 }, // DEFCON 3 or better
        source: "We cover crises, not catastrophes.",
    },
    Template {
        name: "Population Welfare Fund",
        patron: "Dr. Fatima Al-Rashidi, Humanitarian Aid",
        income: 1.5,
        condition: FundingCondition::MaxDeaths { threshold: 500_000_000.0 },
        source: "We exist to limit casualties. So do you.",
    },
    // Policy-requiring: force spending
    Template {
        name: "Medical Workers' Compact",
        patron: "Roberto Silva, Healthcare Union",
        income: 1.5,
        condition: FundingCondition::RequirePolicy { policy_idx: 2 }, // Hospital Surge
        source: "Our people need proper surge facilities.",
    },
    Template {
        name: "Border Security Contract",
        patron: "Gen. Klaus Weber, Security Services",
        income: 1.5,
        condition: FundingCondition::RequirePolicy { policy_idx: 3 }, // Border Controls
        source: "Secure borders save lives.",
    },
];

/// Build a FundingContract from a template index, with ±20% income variance.
fn build_contract(template_id: u8, rng: &mut ChaCha8Rng) -> FundingContract {
    let t = &TEMPLATES[template_id as usize];
    let variance = 0.8 + rng.r#gen::<f64>() * 0.4; // 0.8 to 1.2
    FundingContract {
        name: t.name.to_string(),
        patron: t.patron.to_string(),
        income: t.income * variance,
        condition: t.condition.clone(),
        source: t.source.to_string(),
        template_id,
        satisfaction: 1.0,
        warned: false,
    }
}

/// Tick patron satisfaction and revoke contracts when satisfaction bottoms out.
pub fn tick_check_contracts(state: &mut GameState) {
    // First pass: compute satisfaction changes (need immutable borrow for is_met)
    let updates: Vec<(usize, bool)> = state.contracts.iter().enumerate()
        .map(|(i, c)| (i, c.condition.is_met(state)))
        .collect();

    // Second pass: apply satisfaction drift, warnings, and revocations
    let mut to_revoke: Vec<(usize, String, String, String)> = Vec::new();

    for (i, met) in &updates {
        let c = &mut state.contracts[*i];
        if *met {
            c.satisfaction = (c.satisfaction + PATRON_RECOVER_RATE).min(1.0);
            // Reset warning flag when satisfaction recovers above threshold
            if c.satisfaction > PATRON_SATISFACTION_WARN + 0.1 {
                c.warned = false;
            }
        } else {
            c.satisfaction = (c.satisfaction - PATRON_DEGRADE_RATE).max(0.0);

            // Fire warning when crossing threshold
            if c.satisfaction <= PATRON_SATISFACTION_WARN && !c.warned {
                c.warned = true;
                state.events.push(GameEvent::ContractWarning {
                    patron: c.patron.clone(),
                    reason: c.condition.description(),
                });
            }

            // Revoke when satisfaction bottoms out
            if c.satisfaction <= PATRON_SATISFACTION_REVOKE {
                to_revoke.push((
                    *i,
                    c.name.clone(),
                    c.patron.clone(),
                    c.condition.description(),
                ));
            }
        }
    }

    // Remove in reverse order to preserve indices
    for (i, name, patron, reason) in to_revoke.iter().rev() {
        state.contracts.remove(*i);
        state.events.push(GameEvent::ContractRevoked {
            name: format!("{} pulled out: {}", patron, name),
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

    fn make_test_contract(condition: FundingCondition) -> FundingContract {
        FundingContract {
            name: "Test Contract".to_string(),
            patron: "Test Patron, Testing Dept".to_string(),
            income: 2.0,
            condition,
            source: "For testing.".to_string(),
            template_id: 4,
            satisfaction: 1.0,
            warned: false,
        }
    }

    fn make_offer(state: &mut GameState) {
        state.contract_offer = Some(make_test_contract(
            FundingCondition::MaxThreatLevel { level: 3 },
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
                patron: format!("Patron {i}"),
                income: 1.0,
                condition: FundingCondition::NoCollapse,
                source: String::new(),
                template_id: i as u8,
                satisfaction: 1.0,
                warned: false,
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
        let mut c = make_test_contract(FundingCondition::MaxThreatLevel { level: 3 });
        c.satisfaction = 1.0;
        state.contracts.push(c);

        // Escalate threat beyond level 3
        state.threat_level = crate::state::ThreatLevel::Catastrophe;

        // Run several ticks — satisfaction should degrade but not instant-revoke
        for _ in 0..100 {
            tick_check_contracts(&mut state);
        }
        assert_eq!(state.contracts.len(), 1, "Should not be revoked after only 100 ticks");
        assert!(state.contracts[0].satisfaction < 1.0, "Satisfaction should have dropped");
        assert!(state.contracts[0].satisfaction > PATRON_SATISFACTION_REVOKE,
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
        let mut c = make_test_contract(FundingCondition::MaxThreatLevel { level: 3 });
        c.satisfaction = PATRON_SATISFACTION_WARN + PATRON_DEGRADE_RATE * 0.5; // Just above warning
        state.contracts.push(c);
        state.threat_level = crate::state::ThreatLevel::Catastrophe;

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
        let mut c = make_test_contract(FundingCondition::MaxThreatLevel { level: 3 });
        c.satisfaction = PATRON_SATISFACTION_REVOKE + 0.0001; // Just above revocation
        c.warned = true;
        state.contracts.push(c);
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
            patron: "Stable Patron".to_string(),
            income: 2.0,
            condition: FundingCondition::NoCollapse,
            source: String::new(),
            template_id: 3,
            satisfaction: 1.0,
            warned: false,
        });
        tick_check_contracts(&mut state);
        assert_eq!(state.contracts.len(), 1);
        // Satisfaction should stay at 1.0 (capped)
        assert!((state.contracts[0].satisfaction - 1.0).abs() < 0.001);
    }

    #[test]
    fn offer_generated_at_first_offer_tick() {
        let mut state = GameState::new_default(42);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        state.tick = CONTRACT_FIRST_OFFER_TICK - 1;
        tick_offer_contracts(&mut state, &mut rng);
        assert!(state.contract_offer.is_none());

        state.tick = CONTRACT_FIRST_OFFER_TICK;
        tick_offer_contracts(&mut state, &mut rng);
        assert!(state.contract_offer.is_some());
        // Verify offer has patron name
        let offer = state.contract_offer.as_ref().unwrap();
        assert!(!offer.patron.is_empty(), "Offer should have a patron name");
    }

    #[test]
    fn no_offer_when_at_max_contracts() {
        let mut state = GameState::new_default(42);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        state.tick = CONTRACT_FIRST_OFFER_TICK;

        for i in 0..MAX_CONTRACTS {
            state.contracts.push(FundingContract {
                name: format!("C{i}"),
                patron: format!("Patron {i}"),
                income: 1.0,
                condition: FundingCondition::NoCollapse,
                source: String::new(),
                template_id: i as u8,
                satisfaction: 1.0,
                warned: false,
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

        state.tick += CONTRACT_OFFER_INTERVAL;
        tick_offer_contracts(&mut state, &mut rng);
        assert_eq!(state.contract_offer.as_ref().unwrap().name, first_offer);
    }
}
