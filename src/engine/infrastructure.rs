use crate::state::{
    BasicTech, CorporationSector, GameEvent, GameState, InfraSystem, PathogenType, RegionSpecialization,
    INFRA_CRITICAL, INFRA_STRESSED,
    SEVERITY_CRIT_THRESHOLD, SEVERITY_HIGH_THRESHOLD, SEVERITY_MOD_THRESHOLD,
    TROPICAL_MEDICINE_HC_DRAIN_MULT, COMMUNITY_NETWORKS_CO_DRAIN_MULT, LOGISTICS_HUB_SL_DRAIN_MULT,
};

/// Tick infrastructure degradation for all regions. Called once per tick.
///
/// Each infrastructure system degrades based on regional conditions:
/// - Healthcare: degrades from infection load (hospitals overwhelmed)
/// - Supply lines: degrades from death rate and travel bans
/// - Civil order: degrades from deaths, restrictive policies, and low healthcare
pub(super) fn tick_infrastructure(state: &mut GameState) {
    // ResilientGrids tech: disease-caused drains are 20% slower.
    let resilience_mult = if state.unlocked_techs.contains(&BasicTech::ResilientGrids) {
        0.80
    } else {
        1.0
    };
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

        // Energy sector bonus: infrastructure drains slower
        let energy_bonus = state.sector_bonus(i, CorporationSector::Energy);
        let energy_drain_mult = 1.0 - CorporationSector::Energy.max_bonus_pct() / 100.0 * energy_bonus;
        // Mining sector bonus: natural recovery rates faster
        let mining_bonus = state.sector_bonus(i, CorporationSector::Mining);
        let mining_recovery_mult = 1.0 + CorporationSector::Mining.max_bonus_pct() / 100.0 * mining_bonus;

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
        // Fungal infections contaminate hospital environments (spores on surfaces,
        // HVAC systems, equipment). Even at low infection counts, active fungal
        // presence slowly degrades healthcare capacity — a "contained" fungus
        // is still silently eroding the region's ability to treat other diseases.
        let fungal_count: usize = state.regions[i].infections.iter()
            .filter(|inf| {
                inf.infected > 0.0
                    && state.diseases.get(inf.disease_idx)
                        .is_some_and(|d| d.pathogen_type == PathogenType::Fungus)
            })
            .count();
        let fungal_drain = -(fungal_count as f64) * 0.0002; // ~0.024/day per fungus — slow but persistent
        // Baseline: hospitals provide small passive healthcare recovery.
        // Discourage Hospitalization removes this benefit.
        let hospital_recovery = if state.policies[i].discourage_hosp {
            0.0
        } else {
            0.0001 // ~0.012/day — slows degradation slightly
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
        // TropicalMedicine specialization: healthcare degrades 40% slower
        let hc_spec_mult = if state.regions[i].has_specialization(RegionSpecialization::TropicalMedicine) {
            TROPICAL_MEDICINE_HC_DRAIN_MULT
        } else {
            1.0
        };
        let new_healthcare = (old_healthcare + (healthcare_drain + fungal_drain) * resilience_mult * hc_spec_mult * energy_drain_mult
            + (hospital_recovery + hospital_building_recovery + natural_healthcare_recovery) * mining_recovery_mult)
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
            -0.0001 // ~0.012/day — trade disruption hurts logistics
        } else {
            0.0
        };
        // Natural recovery when deaths are low (travel ban no longer blocks this)
        let natural_supply_recovery = if death_frac < 0.001 {
            0.00005 // ~0.006/day
        } else {
            0.0
        };

        let old_supply = state.regions[i].supply_lines;
        let supply_drain = death_drain * resilience_mult + travel_ban_drain;
        // LogisticsHub specialization: supply lines degrade 40% slower
        let sl_spec_mult = if state.regions[i].has_specialization(RegionSpecialization::LogisticsHub) {
            LOGISTICS_HUB_SL_DRAIN_MULT
        } else {
            1.0
        };
        let new_supply = (old_supply + supply_drain * sl_spec_mult * energy_drain_mult + natural_supply_recovery * mining_recovery_mult)
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
        // Quarantine fatigue: sustained quarantine erodes civil order much faster than
        // generic restriction drain. Creates a ~22-day window before anarchy kicks in,
        // forcing players to cycle quarantine on/off rather than set-and-forget.
        let quarantine_drain = if policy.quarantine { -0.0003 } else { 0.0 }; // ~0.036/day
        // RNA virus panic: visible large-scale RNA outbreaks cause extra civil
        // order degradation from social panic (rapid spread is visibly alarming).
        // Only counts detected diseases — undetected spread doesn't cause visible panic.
        let rna_infected: f64 = state.regions[i].infections.iter()
            .filter(|inf| {
                state.diseases.get(inf.disease_idx)
                    .is_some_and(|d| d.detected && d.pathogen_type == PathogenType::RnaVirus)
            })
            .map(|inf| inf.infected)
            .sum();
        let rna_panic_drain = if rna_infected > SEVERITY_CRIT_THRESHOLD {
            -0.0003 // ~0.036/day — visible mass casualties from fast-moving RNA virus
        } else if rna_infected > SEVERITY_HIGH_THRESHOLD {
            -0.00012 // ~0.014/day — growing unrest from RNA outbreak
        } else {
            0.0
        };

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
        let civil_drain = civil_death_drain * resilience_mult + restriction_drain + quarantine_drain + healthcare_cascade + rna_panic_drain * resilience_mult;
        // CommunityNetworks specialization: civil order degrades 40% slower
        let co_spec_mult = if state.regions[i].has_specialization(RegionSpecialization::CommunityNetworks) {
            COMMUNITY_NETWORKS_CO_DRAIN_MULT
        } else {
            1.0
        };
        let new_civil = (old_civil + civil_drain * co_spec_mult * energy_drain_mult + natural_civil_recovery * mining_recovery_mult)
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

        // Tick many times (7 days — baseline hospital recovery partially counters drain)
        for _ in 0..(120 * 7) {
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
    fn quarantine_drains_civil_order_faster_than_other_policies() {
        let mut state = GameState::new_default(42);
        // No deaths — isolate quarantine drain effect
        state.regions[0].dead = 0.0;
        for r in &mut state.regions {
            r.infections.clear();
        }

        // Run with quarantine only
        state.policies[0].quarantine = true;
        let initial = state.regions[0].civil_order;
        for _ in 0..(120 * 15) {
            tick_infrastructure(&mut state);
        }
        let co_with_quarantine = state.regions[0].civil_order;

        // Reset and run with travel_ban only (same generic restriction weight)
        state.regions[0].civil_order = initial;
        state.policies[0].quarantine = false;
        state.policies[0].travel_ban = true;
        for _ in 0..(120 * 15) {
            tick_infrastructure(&mut state);
        }
        let co_with_travel_ban = state.regions[0].civil_order;

        assert!(co_with_quarantine < co_with_travel_ban,
            "Quarantine should drain civil order faster than travel ban: {} vs {}",
            co_with_quarantine, co_with_travel_ban);
        // 15 days of quarantine should cause meaningful drain (at least 0.3 drop)
        assert!(initial - co_with_quarantine > 0.3,
            "15 days of quarantine should drain civil order significantly: {} -> {}",
            initial, co_with_quarantine);
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
        state.active_research.push(ResearchProject {
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
        assert!(state.active_research.is_empty(), "project should be consumed");
        assert!((state.regions[0].healthcare_capacity - 0.40).abs() < 0.01,
            "HC should be 0.10 + FIELD_OPS_RESTORE = 0.40, got {}", state.regions[0].healthcare_capacity);
        assert!(state.events.iter().any(|e| matches!(e,
            GameEvent::InfrastructureStabilized { region_idx: 0, system: InfraSystem::Healthcare })));
    }

    #[test]
    fn resilient_grids_prereq_requires_targeted_drug_design() {
        use crate::state::BasicTech;
        let state = GameState::new_default(42);
        // Without TargetedDrugDesign, prereq not met
        assert!(!BasicTech::ResilientGrids.prerequisites_met(&state));

        let mut state2 = GameState::new_default(42);
        state2.unlocked_techs.push(BasicTech::TargetedDrugDesign);
        assert!(BasicTech::ResilientGrids.prerequisites_met(&state2));
    }

    #[test]
    fn resilient_grids_slows_healthcare_degradation() {
        use crate::state::BasicTech;
        // Without tech
        let mut state_no_tech = GameState::new_default(42);
        state_no_tech.regions[0].get_or_create_infection(0).infected = SEVERITY_CRIT_THRESHOLD + 1.0;
        for _ in 0..(120 * 7) {
            tick_infrastructure(&mut state_no_tech);
        }
        let hc_no_tech = state_no_tech.regions[0].healthcare_capacity;

        // With tech
        let mut state_tech = GameState::new_default(42);
        state_tech.unlocked_techs.push(BasicTech::ResilientGrids);
        state_tech.regions[0].get_or_create_infection(0).infected = SEVERITY_CRIT_THRESHOLD + 1.0;
        for _ in 0..(120 * 7) {
            tick_infrastructure(&mut state_tech);
        }
        let hc_tech = state_tech.regions[0].healthcare_capacity;

        assert!(hc_tech > hc_no_tech,
            "ResilientGrids should slow HC degradation: with={} without={}", hc_tech, hc_no_tech);
    }

    #[test]
    fn resilient_grids_slows_supply_line_degradation() {
        use crate::state::BasicTech;
        let pop = {
            let s = GameState::new_default(42);
            s.regions[0].population as f64
        };

        // Without tech
        let mut state_no_tech = GameState::new_default(42);
        state_no_tech.regions[0].dead = pop * 0.06;
        for _ in 0..(120 * 10) {
            tick_infrastructure(&mut state_no_tech);
        }
        let sl_no_tech = state_no_tech.regions[0].supply_lines;

        // With tech
        let mut state_tech = GameState::new_default(42);
        state_tech.unlocked_techs.push(BasicTech::ResilientGrids);
        state_tech.regions[0].dead = pop * 0.06;
        for _ in 0..(120 * 10) {
            tick_infrastructure(&mut state_tech);
        }
        let sl_tech = state_tech.regions[0].supply_lines;

        assert!(sl_tech > sl_no_tech,
            "ResilientGrids should slow SL degradation: with={} without={}", sl_tech, sl_no_tech);
    }

    #[test]
    fn resilient_grids_slows_civil_order_degradation() {
        use crate::state::BasicTech;
        let pop = {
            let s = GameState::new_default(42);
            s.regions[0].population as f64
        };

        // Without tech
        let mut state_no_tech = GameState::new_default(42);
        state_no_tech.regions[0].dead = pop * 0.06;
        for _ in 0..(120 * 10) {
            tick_infrastructure(&mut state_no_tech);
        }
        let co_no_tech = state_no_tech.regions[0].civil_order;

        // With tech
        let mut state_tech = GameState::new_default(42);
        state_tech.unlocked_techs.push(BasicTech::ResilientGrids);
        state_tech.regions[0].dead = pop * 0.06;
        for _ in 0..(120 * 10) {
            tick_infrastructure(&mut state_tech);
        }
        let co_tech = state_tech.regions[0].civil_order;

        assert!(co_tech > co_no_tech,
            "ResilientGrids should slow CO degradation: with={} without={}", co_tech, co_no_tech);
    }

    #[test]
    fn rna_virus_causes_extra_civil_order_drain() {
        use crate::state::PathogenType;
        // Set up two identical states: one with an RNA virus, one with a bacterium
        let mut state_rna = GameState::new_default(42);
        for r in &mut state_rna.regions { r.infections.clear(); r.dead = 0.0; }
        state_rna.diseases[0].pathogen_type = PathogenType::RnaVirus;
        state_rna.diseases[0].detected = true;
        state_rna.regions[0].get_or_create_infection(0).infected = SEVERITY_CRIT_THRESHOLD + 1.0;

        let mut state_bact = GameState::new_default(42);
        for r in &mut state_bact.regions { r.infections.clear(); r.dead = 0.0; }
        state_bact.diseases[0].pathogen_type = PathogenType::Bacterium;
        state_bact.diseases[0].detected = true;
        state_bact.regions[0].get_or_create_infection(0).infected = SEVERITY_CRIT_THRESHOLD + 1.0;

        for _ in 0..(120 * 10) {
            tick_infrastructure(&mut state_rna);
            tick_infrastructure(&mut state_bact);
        }

        assert!(state_rna.regions[0].civil_order < state_bact.regions[0].civil_order,
            "RNA virus should cause more civil order drain than bacterium: rna={} bact={}",
            state_rna.regions[0].civil_order, state_bact.regions[0].civil_order);
    }

    #[test]
    fn rna_panic_only_affects_detected_diseases() {
        use crate::state::PathogenType;
        let mut state_detected = GameState::new_default(42);
        for r in &mut state_detected.regions { r.infections.clear(); r.dead = 0.0; }
        state_detected.diseases[0].pathogen_type = PathogenType::RnaVirus;
        state_detected.diseases[0].detected = true;
        state_detected.regions[0].get_or_create_infection(0).infected = SEVERITY_CRIT_THRESHOLD + 1.0;

        let mut state_undetected = GameState::new_default(42);
        for r in &mut state_undetected.regions { r.infections.clear(); r.dead = 0.0; }
        state_undetected.diseases[0].pathogen_type = PathogenType::RnaVirus;
        state_undetected.diseases[0].detected = false;
        state_undetected.regions[0].get_or_create_infection(0).infected = SEVERITY_CRIT_THRESHOLD + 1.0;

        for _ in 0..(120 * 10) {
            tick_infrastructure(&mut state_detected);
            tick_infrastructure(&mut state_undetected);
        }

        assert!(state_detected.regions[0].civil_order < state_undetected.regions[0].civil_order,
            "Detected RNA virus should drain more civil order: detected={} undetected={}",
            state_detected.regions[0].civil_order, state_undetected.regions[0].civil_order);
    }

    #[test]
    fn fungal_infection_drains_healthcare_below_severity_threshold() {
        use crate::state::PathogenType;
        // A small fungal infection (below SEVERITY_MOD_THRESHOLD) should still drain healthcare
        let mut state = GameState::new_default(42);
        for r in &mut state.regions { r.infections.clear(); r.dead = 0.0; }
        state.diseases[0].pathogen_type = PathogenType::Fungus;
        // Set infected well below SEVERITY_MOD_THRESHOLD (1000)
        state.regions[0].get_or_create_infection(0).infected = 100.0;

        let initial_hc = state.regions[0].healthcare_capacity;
        // Run for 30 days — natural recovery and hospital recovery will fight the drain
        for _ in 0..(120 * 30) {
            tick_infrastructure(&mut state);
        }
        // HC should be lower than initial despite being below normal drain thresholds
        assert!(state.regions[0].healthcare_capacity < initial_hc,
            "Fungal infection should drain HC even below MOD threshold: initial={} final={}",
            initial_hc, state.regions[0].healthcare_capacity);
    }

    #[test]
    fn fungal_drain_stacks_with_multiple_infections() {
        use crate::state::PathogenType;
        // One fungal infection
        let mut state_one = GameState::new_default(42);
        for r in &mut state_one.regions { r.infections.clear(); r.dead = 0.0; }
        state_one.diseases[0].pathogen_type = PathogenType::Fungus;
        state_one.regions[0].get_or_create_infection(0).infected = 100.0;

        // Two fungal infections
        let mut state_two = GameState::new_default(42);
        for r in &mut state_two.regions { r.infections.clear(); r.dead = 0.0; }
        state_two.diseases[0].pathogen_type = PathogenType::Fungus;
        state_two.regions[0].get_or_create_infection(0).infected = 100.0;
        // Need a second disease entry
        if state_two.diseases.len() < 2 {
            state_two.diseases.push(state_two.diseases[0].clone());
        }
        state_two.diseases[1].pathogen_type = PathogenType::Fungus;
        state_two.regions[0].get_or_create_infection(1).infected = 100.0;

        for _ in 0..(120 * 20) {
            tick_infrastructure(&mut state_one);
            tick_infrastructure(&mut state_two);
        }
        assert!(state_two.regions[0].healthcare_capacity < state_one.regions[0].healthcare_capacity,
            "Two fungal infections should drain more than one: two={} one={}",
            state_two.regions[0].healthcare_capacity, state_one.regions[0].healthcare_capacity);
    }

    #[test]
    fn non_fungal_has_no_drain_below_severity_threshold() {
        use crate::state::PathogenType;
        let mut state = GameState::new_default(42);
        for r in &mut state.regions { r.infections.clear(); r.dead = 0.0; }
        state.diseases[0].pathogen_type = PathogenType::Bacterium;
        state.regions[0].get_or_create_infection(0).infected = 100.0;

        for _ in 0..(120 * 30) {
            tick_infrastructure(&mut state);
        }
        // HC should be at or above initial (natural recovery with no real drain)
        assert!(state.regions[0].healthcare_capacity >= 1.0 - 0.01,
            "Non-fungal below MOD should not drain HC: {}",
            state.regions[0].healthcare_capacity);
    }

    #[test]
    fn resilient_grids_does_not_affect_policy_drain() {
        use crate::state::BasicTech;
        // Travel ban drain on supply lines should be unaffected
        let mut state_no_tech = GameState::new_default(42);
        // Clear infections so only policy drain acts
        for r in &mut state_no_tech.regions {
            r.infections.clear();
            r.dead = 0.0;
        }
        state_no_tech.policies[0].travel_ban = true;
        for _ in 0..(120 * 10) {
            tick_infrastructure(&mut state_no_tech);
        }
        let sl_no_tech = state_no_tech.regions[0].supply_lines;

        let mut state_tech = GameState::new_default(42);
        state_tech.unlocked_techs.push(BasicTech::ResilientGrids);
        for r in &mut state_tech.regions {
            r.infections.clear();
            r.dead = 0.0;
        }
        state_tech.policies[0].travel_ban = true;
        for _ in 0..(120 * 10) {
            tick_infrastructure(&mut state_tech);
        }
        let sl_tech = state_tech.regions[0].supply_lines;

        assert!((sl_tech - sl_no_tech).abs() < 0.001,
            "ResilientGrids should NOT affect policy drain: with={} without={}", sl_tech, sl_no_tech);
    }

}
