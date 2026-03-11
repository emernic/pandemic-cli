use crate::state::{
    CrisisKind, GameState, LoanLender,
    LOAN_CORP_INTEREST_RATE, LOAN_GOVERNOR_INTEREST_RATE,
    LOAN_MAX_SIMULTANEOUS, LOAN_OFFER_COOLDOWN, TICKS_PER_DAY,
};

/// Accrue daily interest on all active loans, and queue hostile follow-up
/// crises when loans become overdue. Called each tick.
pub(super) fn tick_loans(state: &mut GameState) {
    if state.loans.is_empty() {
        return;
    }
    let current_day = state.tick as f64 / TICKS_PER_DAY;

    for loan in &mut state.loans {
        // Accrue interest each tick
        loan.outstanding += loan.interest_per_tick();
    }

    // Queue LoanCallIn for overdue loans that haven't been actioned yet
    for i in 0..state.loans.len() {
        if state.loans[i].hostile_queued {
            continue;
        }
        if current_day <= state.loans[i].due_day {
            continue;
        }
        // Only queue if no LoanCallIn is already pending for this lender
        let already_pending = state.pending_crises.iter().any(|(_, k)| {
            matches!(k, CrisisKind::LoanCallIn { lender, .. } if *lender == state.loans[i].lender)
        });
        if !already_pending {
            let kind = CrisisKind::LoanCallIn {
                lender_name: state.loans[i].lender_name.clone(),
                lender: state.loans[i].lender.clone(),
                outstanding: state.loans[i].outstanding,
            };
            state.pending_crises.push((state.tick, kind));
            state.loans[i].hostile_queued = true;
        }
    }
}

/// Repay a loan in full. Returns the amount repaid (or 0.0 if out of funds or bad index).
/// Removes the loan from state.loans on success.
pub(super) fn repay_loan(state: &mut GameState, loan_idx: usize) -> f64 {
    if loan_idx >= state.loans.len() {
        return 0.0;
    }
    let amount = state.loans[loan_idx].outstanding;
    if state.resources.funding < amount {
        return 0.0; // can't afford it
    }
    state.resources.funding -= amount;
    state.loans.remove(loan_idx);
    amount
}

/// Check whether a loan offer should fire. Called from `tick_enforce_costs` context
/// (after a policy has been suspended due to insufficient funds).
///
/// Selects the best available lender: prefers a high-loyalty governor; falls back
/// to a healthy corporation. Returns a CrisisKind if a loan offer should be queued.
pub(super) fn maybe_queue_loan_offer(state: &mut GameState) {
    // Rate limit
    if state.tick.saturating_sub(state.resources.last_loan_offer_tick) < LOAN_OFFER_COOLDOWN {
        return;
    }
    // Don't offer if already at loan cap
    if state.loans.len() >= LOAN_MAX_SIMULTANEOUS {
        return;
    }
    // Don't offer if already have a pending loan offer
    let already_pending = state.pending_crises.iter().any(|(_, k)| {
        matches!(k, CrisisKind::LoanOffer { .. })
    });
    if already_pending {
        return;
    }
    if state.active_crisis.as_ref().is_some_and(|c| matches!(c.kind, CrisisKind::LoanOffer { .. })) {
        return;
    }

    let daily_income = state.funding_income_rate() * TICKS_PER_DAY;

    // Loan amount: enough to cover ~3 days of net burn, clamped to sensible range
    let policy_cost = state.total_policy_funding_cost() * TICKS_PER_DAY;
    let net_burn = (policy_cost + state.personnel_upkeep_rate() * TICKS_PER_DAY - daily_income).max(50.0);
    let amount = (net_burn * 3.0).clamp(100.0, 600.0);
    let amount = (amount / 10.0).round() * 10.0; // round to nearest ¥10

    // Try governor lender first (prefers highest loyalty among non-collapsed regions)
    let governor_lender = state.regions.iter().enumerate()
        .filter(|(_, r)| !r.collapsed)
        .filter(|(i, _)| {
            // Don't offer from a governor who already gave us a loan
            !state.loans.iter().any(|l| matches!(l.lender, LoanLender::Governor { region_idx } if region_idx == *i))
        })
        .max_by(|(_, a), (_, b)| {
            a.governor.loyalty.partial_cmp(&b.governor.loyalty).unwrap_or(std::cmp::Ordering::Equal)
        })
        .filter(|(_, r)| r.governor.loyalty >= 40.0) // Only willing governors
        .map(|(i, r)| (i, r.governor.name.clone()));

    // Try corporation lender (prefers most financially healthy, non-bankrupt)
    let corp_lender = state.corporations.iter().enumerate()
        .filter(|(_, c)| !c.bankrupt)
        .filter(|(ci, _)| {
            !state.loans.iter().any(|l| matches!(l.lender, LoanLender::Corporation { corp_idx } if corp_idx == *ci))
        })
        .max_by(|(_, a), (_, b)| {
            a.reserves_fraction().partial_cmp(&b.reserves_fraction()).unwrap_or(std::cmp::Ordering::Equal)
        })
        .filter(|(_, c)| c.reserves_fraction() >= 0.3) // Only solvent corps
        .map(|(ci, c)| (ci, c.name.clone()));

    // Pick a lender (governor preferred for drama; corp as fallback)
    let (lender, lender_name, interest_rate) = if let Some((region_idx, gov_name)) = governor_lender {
        (LoanLender::Governor { region_idx }, gov_name, LOAN_GOVERNOR_INTEREST_RATE)
    } else if let Some((corp_idx, corp_name)) = corp_lender {
        (LoanLender::Corporation { corp_idx }, corp_name, LOAN_CORP_INTEREST_RATE)
    } else {
        return; // No lender available
    };

    let kind = CrisisKind::LoanOffer {
        lender_name,
        lender,
        amount,
        daily_interest_rate: interest_rate,
    };

    state.pending_crises.push((state.tick, kind));
    state.resources.last_loan_offer_tick = state.tick;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ActiveLoan, GameState, LoanLender, TICKS_PER_DAY};

    fn make_loan(outstanding: f64, due_day: f64) -> ActiveLoan {
        ActiveLoan {
            lender_name: "Test Governor".to_string(),
            lender: LoanLender::Governor { region_idx: 0 },
            principal: outstanding,
            outstanding,
            daily_interest_rate: 0.10,
            due_day,
            hostile_queued: false,
        }
    }

    #[test]
    fn interest_accrues_on_active_loan() {
        let mut state = GameState::new_default(42);
        let initial = 200.0;
        state.loans.push(make_loan(initial, 100.0)); // due_day well in future
        let before = state.loans[0].outstanding;

        tick_loans(&mut state);

        let after = state.loans[0].outstanding;
        let expected_increase = initial * 0.10 / TICKS_PER_DAY;
        assert!(
            (after - before - expected_increase).abs() < 0.001,
            "expected interest ~{expected_increase:.4}, got {:.4}",
            after - before
        );
    }

    #[test]
    fn overdue_loan_queues_hostile_crisis() {
        let mut state = GameState::new_default(42);
        state.tick = (15.0 * TICKS_PER_DAY) as u64; // day 15
        // Loan due on day 10 — already overdue
        state.loans.push(make_loan(200.0, 10.0));

        tick_loans(&mut state);

        assert!(state.loans[0].hostile_queued, "should mark hostile as queued");
        let has_call_in = state.pending_crises.iter().any(|(_, k)| {
            matches!(k, CrisisKind::LoanCallIn { .. })
        });
        assert!(has_call_in, "should queue LoanCallIn crisis");
    }

    #[test]
    fn loan_offer_queued_when_policies_suspended() {
        let mut state = GameState::new_default(42);
        crate::engine::corporations::generate_corporations(&mut state);
        // Advance tick past the cooldown window
        state.tick = LOAN_OFFER_COOLDOWN + 100;
        state.resources.last_loan_offer_tick = 0; // ensure cooldown has elapsed
        // Ensure some governors have sufficient loyalty
        for r in &mut state.regions {
            r.governor.loyalty = 70.0;
        }

        maybe_queue_loan_offer(&mut state);

        let has_offer = state.pending_crises.iter().any(|(_, k)| {
            matches!(k, CrisisKind::LoanOffer { .. })
        });
        assert!(has_offer, "should queue LoanOffer crisis when lender is available");
    }

    #[test]
    fn repay_loan_removes_loan_and_deducts_funds() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 500.0;
        state.loans.push(make_loan(200.0, 100.0));

        let repaid = repay_loan(&mut state, 0);

        assert_eq!(repaid, 200.0, "should repay 200");
        assert!(state.loans.is_empty(), "loan should be removed");
        assert_eq!(state.resources.funding, 300.0, "funds should be deducted");
    }

    #[test]
    fn repay_loan_fails_if_insufficient_funds() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 100.0;
        state.loans.push(make_loan(200.0, 100.0));

        let repaid = repay_loan(&mut state, 0);

        assert_eq!(repaid, 0.0, "should not repay when insufficient funds");
        assert!(!state.loans.is_empty(), "loan should remain");
        assert_eq!(state.resources.funding, 100.0, "funds should not change");
    }
}
