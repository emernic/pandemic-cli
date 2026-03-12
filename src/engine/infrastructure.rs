use crate::state::{
    GameEvent, GameState, InfraSystem, INFRA_CRITICAL, INFRA_STRESSED,
    SEVERITY_CRIT_THRESHOLD, SEVERITY_HIGH_THRESHOLD, SEVERITY_MOD_THRESHOLD,
};

/// Tick infrastructure degradation for all regions. Called once per tick.
///
/// Each infrastructure system degrades based on regional conditions:
/// - Healthcare: degrades from infection load (hospitals overwhelmed)
/// - Supply lines: degrades from death rate and travel bans
/// - Civil order: degrades from deaths, restrictive policies, and low healthcare
pub(super) fn tick_infrastructure(state: &mut GameState) {
    let num_regions = state.regions.len();
    for i in 0..num_regions {
        if state.regions[i].collapsed {
            // Collapsed regions have no infrastructure
            state.regions[i].healthcare_capacity = 0.0;
            state.regions[i].supply_lines = 0.0;
            state.regions[i].civil_order = 0.0;
            continue;
        }

        let infected = state.regions[i].total_infected();
        let pop = state.regions[i].population as f64;
        let death_frac = if pop > 0.0 { state.regions[i].dead / pop } else { 0.0 };

        // --- Healthcare Capacity ---
        // Degrades from infection load (absolute thresholds matching severity levels)
        let healthcare_drain = if infected > SEVERITY_CRIT_THRESHOLD {
            -0.0008 // ~0.096/day at CRIT — hits 50% in ~5 days from full
        } else if infected > SEVERITY_HIGH_THRESHOLD {
            -0.0003 // ~0.036/day at HIGH
        } else if infected > SEVERITY_MOD_THRESHOLD {
            -0.00005 // ~0.006/day at MOD — very slow
        } else {
            0.0
        };
        // Hospital surge provides passive recovery
        let hospital_recovery = if state.policies[i].hospital_surge {
            0.0002 // ~0.024/day — slows degradation, doesn't fully counter CRIT
        } else {
            0.0
        };
        // Field hospital provides small passive recovery
        let hospital_building_recovery = match state.regions[i].hospital_level {
            2 => 0.00015, // Medical Center: ~0.018/day
            1 => 0.00008, // Field Hospital: ~0.01/day
            _ => 0.0,
        };
        // Natural recovery when not under pressure
        let natural_healthcare_recovery = if infected <= SEVERITY_MOD_THRESHOLD {
            0.00008 // ~0.01/day — very slow natural healing
        } else {
            0.0
        };

        let old_healthcare = state.regions[i].healthcare_capacity;
        let new_healthcare = (old_healthcare + healthcare_drain + hospital_recovery
            + hospital_building_recovery + natural_healthcare_recovery)
            .clamp(0.0, 1.0);
        state.regions[i].healthcare_capacity = new_healthcare;
        emit_breakpoint_events(state, i, InfraSystem::Healthcare, old_healthcare, new_healthcare);

        // --- Supply Lines ---
        // Degrades from death rate and travel bans
        let death_drain = if death_frac > 0.05 {
            -0.0006 // ~0.072/day when >5% dead
        } else if death_frac > 0.01 {
            -0.0003 // ~0.036/day when >1% dead
        } else if death_frac > 0.001 {
            -0.00008 // ~0.01/day when >0.1% dead
        } else {
            0.0
        };
        let travel_ban_drain = if state.policies[i].travel_ban {
            -0.0002 // ~0.024/day — trade disruption hurts logistics
        } else {
            0.0
        };
        // Natural recovery when deaths are low
        let natural_supply_recovery = if death_frac < 0.001 && !state.policies[i].travel_ban {
            0.00005 // ~0.006/day
        } else {
            0.0
        };

        let old_supply = state.regions[i].supply_lines;
        let supply_drain = death_drain + travel_ban_drain;
        let new_supply = (old_supply + supply_drain + natural_supply_recovery)
            .clamp(0.0, 1.0);
        state.regions[i].supply_lines = new_supply;
        emit_breakpoint_events(state, i, InfraSystem::SupplyLines, old_supply, new_supply);

        // --- Civil Order ---
        // Degrades from deaths, restrictive policies, and healthcare collapse
        let policy = &state.policies[i];
        let restrictive_count = [
            policy.travel_ban,
            policy.quarantine,
            policy.martial_law,
            policy.border_controls,
        ]
        .iter()
        .filter(|&&b| b)
        .count() as f64;

        let civil_death_drain = if death_frac > 0.05 {
            -0.0005 // ~0.06/day when >5% dead — panic
        } else if death_frac > 0.01 {
            -0.0002 // ~0.024/day when >1% dead
        } else {
            0.0
        };
        let restriction_drain = -restrictive_count * 0.00008; // ~0.01/day per policy
        // Healthcare collapse accelerates civil breakdown
        let healthcare_cascade = if new_healthcare < INFRA_CRITICAL {
            -0.0003 // ~0.036/day — people see hospitals failing
        } else if new_healthcare < INFRA_STRESSED {
            -0.0001 // ~0.012/day
        } else {
            0.0
        };
        // Natural recovery when things are calm
        let natural_civil_recovery = if death_frac < 0.001 && restrictive_count == 0.0 {
            0.00008 // ~0.01/day
        } else {
            0.0
        };

        let old_civil = state.regions[i].civil_order;
        let civil_drain = civil_death_drain + restriction_drain + healthcare_cascade;
        let new_civil = (old_civil + civil_drain + natural_civil_recovery)
            .clamp(0.0, 1.0);
        state.regions[i].civil_order = new_civil;
        emit_breakpoint_events(state, i, InfraSystem::CivilOrder, old_civil, new_civil);
    }
}

/// Emit GameEvent when infrastructure crosses a breakpoint threshold.
fn emit_breakpoint_events(
    state: &mut GameState,
    region_idx: usize,
    system: InfraSystem,
    old: f64,
    new: f64,
) {
    let thresholds = [INFRA_STRESSED, INFRA_CRITICAL, 0.0];
    for &threshold in &thresholds {
        if old > threshold && new <= threshold {
            state.events.push(GameEvent::InfrastructureBreakpoint {
                region_idx,
                system,
                threshold,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::GameState;

    #[test]
    fn healthcare_degrades_under_crit_infections() {
        let mut state = GameState::new_default(42);
        state.regions[0].get_or_create_infection(0).infected = SEVERITY_CRIT_THRESHOLD + 1.0;
        assert_eq!(state.regions[0].healthcare_capacity, 1.0);

        // Tick many times (5 days)
        for _ in 0..(120 * 5) {
            tick_infrastructure(&mut state);
        }
        assert!(state.regions[0].healthcare_capacity < 0.6,
            "HC should degrade under CRIT: {}", state.regions[0].healthcare_capacity);
    }

    #[test]
    fn healthcare_stable_without_infections() {
        let mut state = GameState::new_default(42);
        // Clear all infections
        for r in &mut state.regions {
            r.infections.clear();
        }
        for _ in 0..(120 * 10) {
            tick_infrastructure(&mut state);
        }
        assert!((state.regions[0].healthcare_capacity - 1.0).abs() < 0.01,
            "HC should stay at 1.0 without infections: {}", state.regions[0].healthcare_capacity);
    }

    #[test]
    fn supply_lines_degrade_with_deaths() {
        let mut state = GameState::new_default(42);
        let pop = state.regions[0].population as f64;
        state.regions[0].dead = pop * 0.06; // >5% dead

        for _ in 0..(120 * 10) {
            tick_infrastructure(&mut state);
        }
        assert!(state.regions[0].supply_lines < 0.95,
            "SL should degrade with high deaths: {}", state.regions[0].supply_lines);
    }

    #[test]
    fn civil_order_degrades_with_deaths_and_restrictions() {
        let mut state = GameState::new_default(42);
        let pop = state.regions[0].population as f64;
        state.regions[0].dead = pop * 0.06;
        state.policies[0].quarantine = true;
        state.policies[0].travel_ban = true;

        for _ in 0..(120 * 10) {
            tick_infrastructure(&mut state);
        }
        assert!(state.regions[0].civil_order < 0.95,
            "CO should degrade with deaths and restrictions: {}", state.regions[0].civil_order);
    }

    #[test]
    fn healthcare_cascade_accelerates_civil_order() {
        let mut state = GameState::new_default(42);
        // Manually set healthcare to critical
        state.regions[0].healthcare_capacity = 0.20;
        let pop = state.regions[0].population as f64;
        state.regions[0].dead = pop * 0.02;

        let initial_civil = state.regions[0].civil_order;
        for _ in 0..(120 * 5) {
            tick_infrastructure(&mut state);
        }
        let civil_with_cascade = state.regions[0].civil_order;

        // Reset and try without healthcare failure
        state.regions[0].civil_order = initial_civil;
        state.regions[0].healthcare_capacity = 1.0;
        for _ in 0..(120 * 5) {
            tick_infrastructure(&mut state);
        }
        let civil_without_cascade = state.regions[0].civil_order;

        assert!(civil_with_cascade < civil_without_cascade,
            "Civil order should degrade faster when healthcare is critical: {} vs {}",
            civil_with_cascade, civil_without_cascade);
    }

    #[test]
    fn breakpoint_events_fire() {
        let mut state = GameState::new_default(42);
        state.regions[0].healthcare_capacity = 0.51;
        state.regions[0].get_or_create_infection(0).infected = SEVERITY_CRIT_THRESHOLD + 1.0;

        // Tick until HC crosses 0.50
        for _ in 0..200 {
            state.events.clear();
            tick_infrastructure(&mut state);
            if state.regions[0].healthcare_capacity <= 0.50 {
                break;
            }
        }

        assert!(state.events.iter().any(|e| matches!(e,
            GameEvent::InfrastructureBreakpoint { system: InfraSystem::Healthcare, threshold, .. }
            if (*threshold - 0.50).abs() < 0.01
        )), "should fire STRESSED breakpoint event");
    }

    #[test]
    fn collapsed_region_has_zero_infrastructure() {
        let mut state = GameState::new_default(42);
        state.regions[0].collapsed = true;
        state.regions[0].healthcare_capacity = 0.50;

        tick_infrastructure(&mut state);
        assert_eq!(state.regions[0].healthcare_capacity, 0.0);
        assert_eq!(state.regions[0].supply_lines, 0.0);
        assert_eq!(state.regions[0].civil_order, 0.0);
    }

    #[test]
    fn field_ops_appear_when_infra_degraded() {
        use crate::state::{ResearchKind, INFRA_STRESSED};
        let mut state = GameState::new_default(42);
        // Infrastructure at full — no field ops should appear
        let projects = state.available_field_projects();
        assert!(!projects.iter().any(|p| matches!(p, ResearchKind::FieldOperations { .. })),
            "field ops should not appear when all infra >= 50%");

        // Degrade NA healthcare below stressed threshold
        state.regions[0].healthcare_capacity = INFRA_STRESSED - 0.01;
        let projects = state.available_field_projects();
        let ops: Vec<_> = projects.iter()
            .filter(|p| matches!(p, ResearchKind::FieldOperations { .. }))
            .collect();
        assert_eq!(ops.len(), 1, "should have exactly 1 field ops (HC in NA)");
        assert!(matches!(ops[0],
            ResearchKind::FieldOperations { region_idx: 0, system: InfraSystem::Healthcare }));
    }

    #[test]
    fn field_ops_completion_restores_infrastructure() {
        use crate::state::{ResearchKind, ResearchProject};
        use crate::engine::research;
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;
        let mut state = GameState::new_default(42);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        state.regions[0].healthcare_capacity = 0.10;
        // Create a completed field ops project
        state.field_research.push(ResearchProject {
            kind: ResearchKind::FieldOperations {
                region_idx: 0,
                system: InfraSystem::Healthcare,
            },
            progress: 300.0, // well past required
            required_ticks: 240.0,
            personnel_assigned: 3,
        });
        research::tick_research(&mut state, &mut rng);
        // Project should have completed and restored HC
        assert!(state.field_research.is_empty(), "project should be consumed");
        assert!((state.regions[0].healthcare_capacity - 0.40).abs() < 0.01,
            "HC should be 0.10 + FIELD_OPS_RESTORE = 0.40, got {}", state.regions[0].healthcare_capacity);
        assert!(state.events.iter().any(|e| matches!(e,
            GameEvent::InfrastructureStabilized { region_idx: 0, system: InfraSystem::Healthcare })));
    }

}
