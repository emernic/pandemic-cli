use crate::state::{
    FieldOpKind, FieldOperation, GameEvent, GameOutcome, GameState, InfraSystem,
    KNOWLEDGE_NAME, OP_EMERGENCY_EFFECT_TICKS, OP_EMERGENCY_LETHALITY_MULT,
    OP_RECON_KNOWLEDGE, OP_SURVEY_REPAIR, OP_SUPPLY_RESTORE, OP_CIVIL_RESTORE,
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
            let old_val = state.regions.get(r_idx)
                .map(|r| r.supply_lines).unwrap_or(1.0);
            if let Some(region) = state.regions.get_mut(r_idx) {
                region.supply_lines = (region.supply_lines + OP_SUPPLY_RESTORE).min(1.0);
            }
            let new_pct = ((old_val + OP_SUPPLY_RESTORE).min(1.0) * 100.0) as u32;
            let name = state.regions.get(r_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            ("Supply Reinforcement".to_string(), format!("{}: supply lines restored to {}%", name, new_pct))
        }
        FieldOpKind::CivilOrderStabilization { region_idx } => {
            let r_idx = *region_idx;
            let old_val = state.regions.get(r_idx)
                .map(|r| r.civil_order).unwrap_or(1.0);
            if let Some(region) = state.regions.get_mut(r_idx) {
                region.civil_order = (region.civil_order + OP_CIVIL_RESTORE).min(1.0);
            }
            let new_pct = ((old_val + OP_CIVIL_RESTORE).min(1.0) * 100.0) as u32;
            let name = state.regions.get(r_idx)
                .map(|r| r.name.as_str()).unwrap_or("Unknown");
            ("Civil Stabilization".to_string(), format!("{}: civil order restored to {}%", name, new_pct))
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
    fn supply_reinforcement_restores_supply_lines_and_costs_funding() {
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
    }

    #[test]
    fn civil_stabilization_restores_civil_order() {
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
}
