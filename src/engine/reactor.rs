use crate::state::{
    GameEvent, GameOutcome, WorldState, ResearchKind,
    REACTOR_COST, MAX_REACTORS, Reactor,
    personnel_speed,
};

/// Buy an additional production reactor.
pub(super) fn buy_reactor(state: &mut WorldState) -> (bool, Option<String>) {
    if state.outcome != GameOutcome::Playing {
        return (false, None);
    }
    if state.reactors.len() >= MAX_REACTORS {
        return (false, Some(format!("Maximum {} reactors reached.", MAX_REACTORS)));
    }
    if state.resources.funding < REACTOR_COST {
        return (false, Some(format!(
            "Insufficient funds: need ¥{:.0}, have ¥{:.0}",
            REACTOR_COST, state.resources.funding
        )));
    }
    state.resources.funding -= REACTOR_COST;
    state.reactors.push(Reactor {
        medicine_idx: None,
        auto_deploy: false,
        repeat: false,
        batch_progress: 0.0,
        batch_required: 0.0,
        personnel_assigned: 0,
        active: false,
    });
    (true, Some(format!("Reactor purchased. {} total.", state.reactors.len())))
}

/// Configure a reactor to produce a specific medicine (or clear it).
pub(super) fn configure_reactor(
    state: &mut WorldState,
    reactor_idx: usize,
    medicine_idx: Option<usize>,
) -> (bool, Option<String>) {
    let reactor = match state.reactors.get_mut(reactor_idx) {
        Some(r) => r,
        None => return (false, Some("Invalid reactor".to_string())),
    };
    if reactor.active {
        return (false, Some("Reactor is running a batch. Wait for it to finish.".to_string()));
    }
    if let Some(m_idx) = medicine_idx {
        if m_idx >= state.medicines.len() || !state.medicines[m_idx].unlocked {
            return (false, Some("Invalid or locked medicine.".to_string()));
        }
    }
    reactor.medicine_idx = medicine_idx;
    (true, None)
}

/// Start a manufacturing batch in a reactor.
pub(super) fn start_batch(state: &mut WorldState, reactor_idx: usize) -> (bool, Option<String>) {
    if state.outcome != GameOutcome::Playing {
        return (false, None);
    }
    let medicine_idx = match state.reactors.get(reactor_idx) {
        Some(r) if !r.active => match r.medicine_idx {
            Some(idx) => idx,
            None => return (false, Some("Reactor has no medicine assigned.".to_string())),
        },
        Some(_) => return (false, Some("Reactor is already running.".to_string())),
        None => return (false, Some("Invalid reactor.".to_string())),
    };

    // Check if stockpile is already full
    if let Some(med) = state.medicines.get(medicine_idx) {
        if med.doses >= med.max_doses * state.manufacturing_yield_bonus() {
            return (false, Some("Stockpile is full.".to_string()));
        }
    }

    let (personnel, duration, funding) = state.reactor_batch_costs(medicine_idx);
    if state.resources.funding < funding {
        return (false, Some(format!(
            "Insufficient funds: need ¥{:.0}, have ¥{:.0}",
            funding, state.resources.funding
        )));
    }
    if state.personnel_available() < personnel {
        return (false, Some(format!(
            "Need {} personnel, only {} available",
            personnel, state.personnel_available(),
        )));
    }

    state.resources.funding -= funding;
    let reactor = &mut state.reactors[reactor_idx];
    reactor.active = true;
    reactor.batch_progress = 0.0;
    reactor.batch_required = duration;
    reactor.personnel_assigned = personnel;
    (true, None)
}

/// Advance reactor batches by one tick and handle completions.
pub(super) fn tick_reactors(state: &mut WorldState, events: &mut Vec<GameEvent>) {
    let lab_mult = state.lab_speed_multiplier();
    let biotech_bonus = (0..state.regions.len())
        .map(|r| state.sector_bonus(r, crate::state::CorporationSector::Biotech))
        .fold(0.0_f64, f64::max);
    let biotech_mult = 1.0 + crate::state::CorporationSector::Biotech.max_bonus_pct() / 100.0 * biotech_bonus;
    let mfg_bonus = state.manufacturing_yield_bonus();
    let infra_mult = state.research_infra_multiplier();

    let reactor_count = state.reactors.len();
    for i in 0..reactor_count {
        if !state.reactors[i].active {
            // Auto-repeat: start a new batch if repeat is enabled and stockpile is low
            if state.reactors[i].repeat {
                if let Some(med_idx) = state.reactors[i].medicine_idx {
                    let dose_frac = state.medicines.get(med_idx)
                        .map(|m| if m.max_doses > 0.0 { m.doses / (m.max_doses * mfg_bonus) } else { 1.0 })
                        .unwrap_or(1.0);
                    if dose_frac < 1.0 {
                        let (ok, _) = start_batch(state, i);
                        if ok {
                            events.push(GameEvent::ResearchAutoRestarted {
                                kind: ResearchKind::ManufactureDoses { medicine_idx: med_idx },
                            });
                        }
                    }
                }
            }
            continue;
        }

        // Advance progress
        let base_personnel = {
            let kind = ResearchKind::ManufactureDoses {
                medicine_idx: state.reactors[i].medicine_idx.unwrap_or(0),
            };
            let (p, _, _) = kind.costs(&state.medicines);
            p
        };
        let speed = personnel_speed(state.reactors[i].personnel_assigned, base_personnel)
            * infra_mult * lab_mult * biotech_mult;
        state.reactors[i].batch_progress += speed;

        // Check completion
        if state.reactors[i].batch_progress >= state.reactors[i].batch_required {
            let reactor = &mut state.reactors[i];
            reactor.active = false;
            reactor.batch_progress = 0.0;
            reactor.personnel_assigned = 0;

            if let Some(med_idx) = reactor.medicine_idx {
                // Complete manufacturing — restore doses
                if let Some(medicine) = state.medicines.get_mut(med_idx) {
                    medicine.doses = medicine.max_doses * mfg_bonus;
                }

                // Manufacturer satisfaction boost (same as old ManufactureDoses completion)
                if let Some(corp_idx) = state.medicines.get(med_idx).and_then(|m| m.manufacturer_corp_idx) {
                    if let Some(corp) = state.corporations.get_mut(corp_idx) {
                        if corp.board_seat && !corp.bankrupt {
                            let boost = corp.max_reserves * 0.25;
                            corp.reserves = (corp.reserves + boost).min(corp.max_reserves);
                        }
                    }
                }

                // Auto-deploy: enable medicine auto-deployment when batch completes.
                // This sets deploy_enabled (Medicines panel toggle) so the existing
                // auto-deploy system dispatches doses to worst-affected regions.
                if state.reactors[i].auto_deploy {
                    while state.deploy_enabled.len() <= med_idx {
                        state.deploy_enabled.push(false);
                    }
                    state.deploy_enabled[med_idx] = true;
                }

                events.push(GameEvent::ReactorBatchComplete { medicine_idx: med_idx });
            }
        }
    }
}
