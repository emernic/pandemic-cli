use rand::Rng;

use crate::state::{
    FieldOpKind, FieldOperation, GameEvent, GameOutcome, GameState, InfraSystem,
    KNOWLEDGE_NAME, MAX_INFRA_RESILIENCE, OP_EMERGENCY_EFFECT_TICKS,
    OP_EMERGENCY_LETHALITY_MULT, OP_RECON_KNOWLEDGE, OP_SURVEY_REPAIR,
    OP_SUPPLY_RESTORE, OP_SUPPLY_RESILIENCE, OP_CIVIL_RESTORE, OP_CIVIL_RESILIENCE,
    OP_EVAC_FRACTION, OP_EVAC_SEED_RATE_FACTOR, format_number,
};

/// Start a field operation. Validates personnel availability and target.
/// Returns (success, message).
pub(super) fn start_field_op(
    state: &mut GameState,
    kind: FieldOpKind,
) -> (bool, Option<String>) {
    if state.outcome != GameOutcome::Playing {
        return (false, None);
    }

    let personnel = kind.personnel();
    if state.personnel_available() < personnel {
        return (false, Some(format!(
            "Need {} personnel ({} available)",
            personnel, state.personnel_available()
        )));
    }

    // Check funding cost
    if let Some(cost) = kind.cost() {
        if state.resources.funding < cost {
            return (false, Some(format!(
                "Need ¥{:.0} ({:.0} available)",
                cost, state.resources.funding
            )));
        }
    }

    // Validate targets
    match &kind {
        FieldOpKind::Recon { disease_idx } => {
            let disease = state.diseases.get(*disease_idx);
            if disease.is_none() {
                return (false, Some("Invalid disease target".to_string()));
            }
            if disease.is_some_and(|d| d.knowledge >= KNOWLEDGE_NAME) {
                return (false, Some("Pathogen already identified".to_string()));
            }
            // Check no duplicate recon on same disease
            if state.field_operations.iter().any(|op| matches!(&op.kind, FieldOpKind::Recon { disease_idx: d } if d == disease_idx)) {
                return (false, Some("Recon already in progress for this pathogen".to_string()));
            }
        }
        FieldOpKind::EmergencyResponse { region_idx } => {
            if state.regions.get(*region_idx).is_some_and(|r| r.collapsed) {
                return (false, Some("Region has collapsed".to_string()));
            }
            // Check no duplicate emergency response to same region
            if state.field_operations.iter().any(|op| matches!(&op.kind, FieldOpKind::EmergencyResponse { region_idx: r } if r == region_idx)) {
                return (false, Some("Emergency response already in progress".to_string()));
            }
        }
        FieldOpKind::InfraSurvey { region_idx } => {
            if state.regions.get(*region_idx).is_some_and(|r| r.collapsed) {
                return (false, Some("Region has collapsed".to_string()));
            }
            // Check no duplicate survey to same region
            if state.field_operations.iter().any(|op| matches!(&op.kind, FieldOpKind::InfraSurvey { region_idx: r } if r == region_idx)) {
                return (false, Some("Infrastructure survey already in progress".to_string()));
            }
        }
        FieldOpKind::SupplyChainReinforcement { region_idx } => {
            if state.regions.get(*region_idx).is_some_and(|r| r.collapsed) {
                return (false, Some("Region has collapsed".to_string()));
            }
            if state.field_operations.iter().any(|op| matches!(&op.kind, FieldOpKind::SupplyChainReinforcement { region_idx: r } if r == region_idx)) {
                return (false, Some("Supply reinforcement already in progress".to_string()));
            }
        }
        FieldOpKind::CivilOrderStabilization { region_idx } => {
            if state.regions.get(*region_idx).is_some_and(|r| r.collapsed) {
                return (false, Some("Region has collapsed".to_string()));
            }
            if state.field_operations.iter().any(|op| matches!(&op.kind, FieldOpKind::CivilOrderStabilization { region_idx: r } if r == region_idx)) {
                return (false, Some("Civil stabilization already in progress".to_string()));
            }
        }
        FieldOpKind::EvacuationCorridor { source_idx, dest_idx } => {
            if state.regions.get(*source_idx).is_some_and(|r| r.collapsed) {
                return (false, Some("Source region has collapsed".to_string()));
            }
            if state.regions.get(*dest_idx).is_some_and(|r| r.collapsed) {
                return (false, Some("Destination region has collapsed".to_string()));
            }
            if source_idx == dest_idx {
                return (false, Some("Source and destination must differ".to_string()));
            }
            if state.field_operations.iter().any(|op| matches!(&op.kind, FieldOpKind::EvacuationCorridor { source_idx: s, .. } if s == source_idx)) {
                return (false, Some("Evacuation already in progress from this region".to_string()));
            }
        }
    }

    let label = kind.label();
    let cost = kind.cost();
    let duration_ticks = kind.duration_ticks();

    // Deduct funding if this op has a cost
    if let Some(c) = cost {
        state.resources.funding -= c;
    }

    state.field_operations.push(FieldOperation {
        kind,
        personnel,
        ticks_remaining: duration_ticks,
        total_ticks: duration_ticks,
    });

    let cost_note = cost.map(|c| format!(", ¥{:.0}", c)).unwrap_or_default();
    (true, Some(format!("{} dispatched ({} personnel{})", label, personnel, cost_note)))
}

/// Tick all active field operations. Complete ones that finish.
pub(super) fn tick_field_operations(state: &mut GameState) {
    let mut i = 0;
    while i < state.field_operations.len() {
        state.field_operations[i].ticks_remaining -= 1.0;
        if state.field_operations[i].ticks_remaining <= 0.0 {
            let op = state.field_operations.remove(i);
            complete_operation(state, &op);
        } else {
            i += 1;
        }
    }
}

/// Apply the effects of a completed field operation.
fn complete_operation(state: &mut GameState, op: &FieldOperation) {
    let (label, result) = match &op.kind {
        FieldOpKind::Recon { disease_idx } => {
            let d_idx = *disease_idx;
            let was_unknown = state.diseases.get(d_idx)
                .is_some_and(|d| d.knowledge < KNOWLEDGE_NAME);
            if let Some(disease) = state.diseases.get_mut(d_idx) {
                disease.knowledge = (disease.knowledge + OP_RECON_KNOWLEDGE).min(1.0);
            }
            if was_unknown && state.diseases.get(d_idx)
                .is_some_and(|d| d.knowledge >= KNOWLEDGE_NAME)
            {
                state.events.push(GameEvent::PathogenIdentified { disease_idx: d_idx });
            }
            let name = state.diseases.get(d_idx)
                .map(|d| d.display_name(d_idx))
                .unwrap_or_else(|| "Unknown".to_string());
            let ptype = state.diseases.get(d_idx)
                .map(|d| d.pathogen_type.label())
                .unwrap_or("Unknown");
            ("Recon Mission".to_string(), format!("{}: pathogen type is {}", name, ptype))
        }
        FieldOpKind::EmergencyResponse { region_idx } => {
            let r_idx = *region_idx;
            if let Some(region) = state.regions.get_mut(r_idx) {
                region.emergency_response_until = Some(state.tick + OP_EMERGENCY_EFFECT_TICKS);
            }
            let name = state.regions.get(r_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            let effect_pct = ((1.0 - OP_EMERGENCY_LETHALITY_MULT) * 100.0) as u32;
            ("Emergency Response".to_string(), format!("{}: lethality reduced {}% for 3 days", name, effect_pct))
        }
        FieldOpKind::InfraSurvey { region_idx } => {
            let r_idx = *region_idx;
            // Find the worst infrastructure system
            let (worst_sys, worst_val) = if let Some(region) = state.regions.get(r_idx) {
                let systems = [
                    (InfraSystem::Healthcare, region.healthcare_capacity),
                    (InfraSystem::SupplyLines, region.supply_lines),
                    (InfraSystem::CivilOrder, region.civil_order),
                ];
                systems.into_iter()
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                    .unwrap()
            } else {
                (InfraSystem::Healthcare, 1.0)
            };
            if let Some(region) = state.regions.get_mut(r_idx) {
                let target = match worst_sys {
                    InfraSystem::Healthcare => &mut region.healthcare_capacity,
                    InfraSystem::SupplyLines => &mut region.supply_lines,
                    InfraSystem::CivilOrder => &mut region.civil_order,
                };
                *target = (*target + OP_SURVEY_REPAIR).min(1.0);
            }
            let new_pct = ((worst_val + OP_SURVEY_REPAIR).min(1.0) * 100.0) as u32;
            let name = state.regions.get(r_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            ("Infra Survey".to_string(), format!("{}: {} repaired to {}%", name, worst_sys.label(), new_pct))
        }
        FieldOpKind::SupplyChainReinforcement { region_idx } => {
            let r_idx = *region_idx;
            if let Some(region) = state.regions.get_mut(r_idx) {
                region.supply_lines = (region.supply_lines + OP_SUPPLY_RESTORE).min(1.0);
                region.supply_resilience = (region.supply_resilience + OP_SUPPLY_RESILIENCE).min(MAX_INFRA_RESILIENCE);
            }
            let new_pct = state.regions.get(r_idx)
                .map(|r| (r.supply_lines * 100.0) as u32).unwrap_or(0);
            let res_pct = state.regions.get(r_idx)
                .map(|r| (r.supply_resilience * 100.0) as u32).unwrap_or(0);
            let name = state.regions.get(r_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            ("Supply Reinforcement".to_string(), format!("{}: supply lines {}%, resilience {}%", name, new_pct, res_pct))
        }
        FieldOpKind::CivilOrderStabilization { region_idx } => {
            let r_idx = *region_idx;
            if let Some(region) = state.regions.get_mut(r_idx) {
                region.civil_order = (region.civil_order + OP_CIVIL_RESTORE).min(1.0);
                region.civil_resilience = (region.civil_resilience + OP_CIVIL_RESILIENCE).min(MAX_INFRA_RESILIENCE);
            }
            let new_pct = state.regions.get(r_idx)
                .map(|r| (r.civil_order * 100.0) as u32).unwrap_or(0);
            let res_pct = state.regions.get(r_idx)
                .map(|r| (r.civil_resilience * 100.0) as u32).unwrap_or(0);
            let name = state.regions.get(r_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            ("Civil Stabilization".to_string(), format!("{}: civil order {}%, resilience {}%", name, new_pct, res_pct))
        }
        FieldOpKind::EvacuationCorridor { source_idx, dest_idx } => {
            let s_idx = *source_idx;
            let d_idx = *dest_idx;

            // Calculate susceptibles in source: alive people who aren't infected
            let source_pop = state.regions.get(s_idx).map(|r| r.population as f64).unwrap_or(0.0);
            let source_dead = state.regions.get(s_idx).map(|r| r.dead).unwrap_or(0.0);
            let source_infected = state.regions.get(s_idx).map(|r| r.total_infected()).unwrap_or(0.0);
            let susceptibles = (source_pop - source_dead - source_infected).max(0.0);
            let to_move = (susceptibles * OP_EVAC_FRACTION) as u64;

            let source_name = state.regions.get(s_idx).map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".to_string());
            let dest_name = state.regions.get(d_idx).map(|r| r.name.clone()).unwrap_or_else(|| "Unknown".to_string());

            // Transfer population
            if to_move > 0 {
                if let Some(src) = state.regions.get_mut(s_idx) {
                    src.population = src.population.saturating_sub(to_move);
                }
                if let Some(dst) = state.regions.get_mut(d_idx) {
                    dst.population = dst.population.saturating_add(to_move);
                }
            }

            // Seeding risk: evacuees may carry infection
            // Seed chance scales with infection rate in source (capped at 80%)
            let infection_rate = if source_pop > 0.0 { source_infected / source_pop } else { 0.0 };
            let seed_chance = (infection_rate * OP_EVAC_SEED_RATE_FACTOR).min(0.80);
            let seeded = if seed_chance > 0.0 {
                let roll: f64 = state.rng.r#gen();
                roll < seed_chance
            } else {
                false
            };

            let result = if seeded && source_infected > 0.0 {
                // Seed destination with a fraction of evacuees proportional to infection rate
                let seed_count = (to_move as f64 * infection_rate * 0.5).max(1.0);
                // Find the most prevalent disease in source to seed with
                if let Some(disease_idx) = state.regions.get(s_idx)
                    .and_then(|r| r.infections.iter().max_by(|a, b| a.infected.partial_cmp(&b.infected).unwrap()))
                    .map(|inf| inf.disease_idx)
                {
                    let dest_inf = state.regions[d_idx].get_or_create_infection(disease_idx);
                    dest_inf.infected += seed_count;
                    format!(
                        "{} evacuated from {} to {}. {:.0} carriers arrived at destination.",
                        format_number(to_move as f64),
                        source_name, dest_name,
                        seed_count
                    )
                } else {
                    format!(
                        "{} evacuated from {} to {}",
                        format_number(to_move as f64), source_name, dest_name
                    )
                }
            } else {
                format!(
                    "{} evacuated from {} to {}. No carriers detected.",
                    format_number(to_move as f64), source_name, dest_name
                )
            };

            ("Evacuation Corridor".to_string(), result)
        }
    };

    state.events.push(GameEvent::FieldOpCompleted { label, result });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{GameState, OP_SUPPLY_COST, OP_CIVIL_COST};

    #[test]
    fn recon_adds_knowledge() {
        let mut state = GameState::new_default(42);
        assert!(state.diseases[0].knowledge < KNOWLEDGE_NAME);

        let kind = FieldOpKind::Recon { disease_idx: 0 };
        let (ok, _) = start_field_op(&mut state, kind);
        assert!(ok);
        assert_eq!(state.field_operations.len(), 1);
        assert_eq!(state.personnel_available(), 20 - 2); // 2 personnel on recon

        // Tick until complete
        for _ in 0..200 {
            tick_field_operations(&mut state);
        }
        assert_eq!(state.field_operations.len(), 0);
        assert!(state.diseases[0].knowledge >= OP_RECON_KNOWLEDGE);
    }

    #[test]
    fn emergency_response_sets_timer() {
        let mut state = GameState::new_default(42);
        let kind = FieldOpKind::EmergencyResponse { region_idx: 0 };
        let (ok, _) = start_field_op(&mut state, kind);
        assert!(ok);

        // Tick until complete
        for _ in 0..130 {
            tick_field_operations(&mut state);
        }
        assert!(state.regions[0].emergency_response_until.is_some());
    }

    #[test]
    fn infra_survey_repairs_worst() {
        let mut state = GameState::new_default(42);
        state.regions[0].healthcare_capacity = 0.30;
        state.regions[0].supply_lines = 0.50;
        state.regions[0].civil_order = 0.80;

        let kind = FieldOpKind::InfraSurvey { region_idx: 0 };
        let (ok, _) = start_field_op(&mut state, kind);
        assert!(ok);

        for _ in 0..250 {
            tick_field_operations(&mut state);
        }
        // Healthcare was worst (0.30), should be repaired to 0.45
        assert!((state.regions[0].healthcare_capacity - 0.45).abs() < 0.01);
        // Others unchanged
        assert!((state.regions[0].supply_lines - 0.50).abs() < 0.01);
    }

    #[test]
    fn blocks_duplicate_recon() {
        let mut state = GameState::new_default(42);
        let kind = FieldOpKind::Recon { disease_idx: 0 };
        let (ok1, _) = start_field_op(&mut state, kind.clone());
        assert!(ok1);
        let (ok2, _) = start_field_op(&mut state, kind);
        assert!(!ok2); // duplicate blocked
    }

    #[test]
    fn blocks_when_insufficient_personnel() {
        let mut state = GameState::new_default(42);
        state.resources.personnel = 1;
        let kind = FieldOpKind::Recon { disease_idx: 0 };
        let (ok, msg) = start_field_op(&mut state, kind);
        assert!(!ok);
        assert!(msg.unwrap().contains("personnel"));
    }

    #[test]
    fn supply_reinforcement_restores_and_adds_resilience() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 2000.0;
        state.regions[0].supply_lines = 0.50;
        let funding_before = state.resources.funding;

        let kind = FieldOpKind::SupplyChainReinforcement { region_idx: 0 };
        let (ok, msg) = start_field_op(&mut state, kind);
        assert!(ok);
        assert!(msg.unwrap().contains("¥800"));
        assert!((state.resources.funding - (funding_before - OP_SUPPLY_COST)).abs() < 0.01,
            "funding should be deducted on dispatch");

        // Tick until complete (360 ticks)
        for _ in 0..370 {
            tick_field_operations(&mut state);
        }
        assert_eq!(state.field_operations.len(), 0);
        assert!((state.regions[0].supply_lines - 0.70).abs() < 0.01,
            "supply lines should be restored by 20%: got {}", state.regions[0].supply_lines);
        assert!((state.regions[0].supply_resilience - 0.25).abs() < 0.01,
            "should gain 25% supply resilience: got {}", state.regions[0].supply_resilience);
    }

    #[test]
    fn civil_stabilization_restores_and_adds_resilience() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 2000.0;
        state.regions[0].civil_order = 0.40;
        let funding_before = state.resources.funding;

        let kind = FieldOpKind::CivilOrderStabilization { region_idx: 0 };
        let (ok, _) = start_field_op(&mut state, kind);
        assert!(ok);
        assert!((state.resources.funding - (funding_before - OP_CIVIL_COST)).abs() < 0.01);
        assert_eq!(state.personnel_available(), 20 - 1); // 1 personnel

        for _ in 0..250 {
            tick_field_operations(&mut state);
        }
        assert_eq!(state.field_operations.len(), 0);
        assert!((state.regions[0].civil_order - 0.55).abs() < 0.01,
            "civil order should be restored by 15%: got {}", state.regions[0].civil_order);
        assert!((state.regions[0].civil_resilience - 0.25).abs() < 0.01,
            "should gain 25% civil resilience: got {}", state.regions[0].civil_resilience);
    }

    #[test]
    fn funded_op_blocked_when_insufficient_funding() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 100.0; // Not enough for ¥800
        let kind = FieldOpKind::SupplyChainReinforcement { region_idx: 0 };
        let (ok, msg) = start_field_op(&mut state, kind);
        assert!(!ok);
        assert!(msg.unwrap().contains("¥800"));
    }

    #[test]
    fn blocks_duplicate_supply_reinforcement() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 2000.0;
        let kind = FieldOpKind::SupplyChainReinforcement { region_idx: 0 };
        let (ok1, _) = start_field_op(&mut state, kind.clone());
        assert!(ok1);
        let (ok2, _) = start_field_op(&mut state, kind);
        assert!(!ok2);
    }

    #[test]
    fn evacuation_corridor_transfers_population() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 2000.0;
        let src_pop = state.regions[0].population;
        let dst_pop = state.regions[1].population;

        let kind = FieldOpKind::EvacuationCorridor { source_idx: 0, dest_idx: 1 };
        let (ok, msg) = start_field_op(&mut state, kind);
        assert!(ok);
        assert!(msg.unwrap().contains("¥600"));
        assert_eq!(state.personnel_available(), 20 - 2);

        for _ in 0..130 {
            tick_field_operations(&mut state);
        }
        assert_eq!(state.field_operations.len(), 0, "op should complete");

        // Source loses ~10% susceptibles; destination gains them
        assert!(state.regions[0].population < src_pop, "source population should decrease");
        assert!(state.regions[1].population > dst_pop, "dest population should increase");
        // Transfer should be roughly 10% of source population (no deaths at start)
        let expected_transfer = (src_pop as f64 * 0.10) as u64;
        let actual_transfer = src_pop - state.regions[0].population;
        assert!((actual_transfer as i64 - expected_transfer as i64).abs() < 1_000_000,
            "transferred ~{} but expected ~{}", actual_transfer, expected_transfer);
    }

    #[test]
    fn evacuation_corridor_blocks_same_source_dest() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 2000.0;
        let kind = FieldOpKind::EvacuationCorridor { source_idx: 0, dest_idx: 0 };
        let (ok, msg) = start_field_op(&mut state, kind);
        assert!(!ok);
        assert!(msg.unwrap().contains("differ"));
    }

    #[test]
    fn evacuation_corridor_blocks_duplicate_from_same_source() {
        let mut state = GameState::new_default(42);
        state.resources.funding = 2000.0;
        let kind = FieldOpKind::EvacuationCorridor { source_idx: 0, dest_idx: 1 };
        let (ok1, _) = start_field_op(&mut state, kind.clone());
        assert!(ok1);
        let (ok2, msg) = start_field_op(&mut state, kind);
        assert!(!ok2);
        assert!(msg.unwrap().contains("already in progress"));
    }
}
