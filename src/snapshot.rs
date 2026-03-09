use ratatui::{backend::TestBackend, Terminal};

use crate::action::{string_to_action, Action};
use crate::apply_action;
use crate::engine::tick;
use crate::state::{GameEvent, GameState, GameOutcome, SimState};
use crate::ui;

/// Result of running snapshot mode: the rendered screen and the updated state.
#[derive(Debug)]
pub struct SnapshotResult {
    pub screen: String,
    pub state: GameState,
}

/// Parse a snapshot step string. Returns either a tick count or a key string.
/// `d<N>` means N days (converted to ticks). `t<N>` means N raw ticks (legacy).
fn parse_step(s: &str) -> Result<SnapshotStep, String> {
    // d<N> — days (primary user-facing unit)
    if let Some(rest) = s.strip_prefix('d') {
        if let Ok(days) = rest.parse::<f64>() {
            let ticks = (days * crate::state::TICKS_PER_DAY) as u64;
            return Ok(SnapshotStep::Ticks(ticks));
        }
    }
    // t<N> — raw ticks (legacy/internal use)
    if let Some(rest) = s.strip_prefix('t') {
        if let Ok(n) = rest.parse::<u64>() {
            return Ok(SnapshotStep::Ticks(n));
        }
    }
    // "t" alone is a valid key (opens Threats panel), "d" alone is not a key
    if s == "d" {
        return Err("Invalid step: 'd' — did you mean 'd1' (1 day)?".to_string());
    }
    Ok(SnapshotStep::Key(s.to_string()))
}

enum SnapshotStep {
    Ticks(u64),
    Key(String),
}

/// Advance simulation by `n` ticks.
///
/// Crises are auto-resolved inline (picks cheapest affordable option).
/// Stops early on game over.
fn advance_ticks(state: &mut GameState, n: u64) {
    state.sim_state = SimState::Running;
    for _ in 0..n {
        *state = tick(state);
        ui::process_events(state);
        if state.active_crisis.is_some() {
            auto_resolve_crisis(state);
        }
        if state.outcome != GameOutcome::Playing {
            return;
        }
        // Auto-pause events (DiseaseDetected, RegionCollapsed) set SimState::Paused.
        // In snapshot mode, log the event and auto-resume rather than blocking.
        if state.sim_state == SimState::Paused {
            for event in &state.events {
                match event {
                    GameEvent::DiseaseDetected { disease_idx } => {
                        let name = state.diseases.get(*disease_idx)
                            .map(|d| d.display_name(*disease_idx))
                            .unwrap_or_else(|| format!("Unknown Pathogen #{}", disease_idx + 1));
                        eprintln!("⚠ NEW THREAT detected: {}", name);
                    }
                    GameEvent::RegionCollapsed { region_idx } => {
                        let name = state.regions.get(*region_idx)
                            .map(|r| r.name.as_str())
                            .unwrap_or("Unknown");
                        eprintln!("⚠ REGION COLLAPSED: {}", name);
                    }
                    _ => {}
                }
            }
            state.sim_state = SimState::Running;
        }
    }
}

/// Auto-resolve a crisis by picking the cheapest affordable option.
/// Tries option A first, then option B if A is unaffordable.
/// Prints a summary line to stderr so playtesters can see what happened.
fn auto_resolve_crisis(state: &mut GameState) {
    // Try option A (index 0) first, fall back to option B (index 1)
    let (choice, title, option_label) = if let Some(crisis) = &state.active_crisis {
        if crisis.option_a.cost.as_ref().map_or(true, |c| c.affordable(state)) {
            (0, crisis.title.clone(), crisis.option_a.label.clone())
        } else {
            (1, crisis.title.clone(), crisis.option_b.label.clone())
        }
    } else {
        return;
    };

    let day = state.tick as f64 / crate::state::TICKS_PER_DAY;
    eprintln!("[Day {day:.1}] Crisis auto-resolved: {title} → {option_label}");

    // Use apply_action(Confirm) which handles affordability checks,
    // sim state restoration, and all crisis resolution logic.
    state.ui.crisis_selection = choice;
    *state = apply_action(state, &Action::Confirm);
}

/// Run snapshot mode: process an ordered sequence of steps, then render.
/// Each step is either a key action (e.g. "r", "enter") or ticks (e.g. "t10").
///
/// Crisis events are always auto-resolved during tick advancement (picks
/// cheapest affordable option). Game over stops execution immediately.
///
/// Returns both the rendered screen and the updated state.
pub fn run_snapshot(
    mut state: GameState,
    steps: &[String],
) -> Result<SnapshotResult, String> {
    for step_str in steps.iter() {
        if state.outcome != GameOutcome::Playing {
            break;
        }

        match parse_step(step_str)? {
            SnapshotStep::Key(key_str) => {
                match string_to_action(&key_str) {
                    Some(action) => {
                        state = apply_action(&state, &action);
                    }
                    None => {
                        return Err(format!(
                            "Unknown key: {:?}. Valid keys: space, t, r, m, p, ?, esc, up, down, left, right, h, l, enter, z, x, q",
                            key_str
                        ));
                    }
                }
            }
            SnapshotStep::Ticks(n) => {
                advance_ticks(&mut state, n);
            }
        }
    }

    let screen = render_to_string(&state);
    Ok(SnapshotResult { screen, state })
}

pub fn render_to_string(state: &GameState) -> String {
    let backend = TestBackend::new(200, 48);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            ui::render(f, state);
        })
        .unwrap();

    // Extract the buffer as a string
    let backend = terminal.backend();
    let buffer = backend.buffer();
    let mut output = String::new();

    for y in 0..buffer.area.height {
        let mut line = String::new();
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            line.push_str(cell.symbol());
        }
        // Trim trailing whitespace from each line
        let trimmed = line.trim_end();
        output.push_str(trimmed);
        output.push('\n');
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_default_renders() {
        let state = GameState::new_default(42);
        let output = render_to_string(&state);
        assert!(output.contains("RUNNING"));
        assert!(output.contains("Asia"));
    }

    #[test]
    fn snapshot_with_days() {
        let state = GameState::new_default(42);
        // d1 = 1 day = 120 ticks
        let result = run_snapshot(state, &["d1".to_string()]).unwrap();
        assert!(result.screen.contains("Day: 1.0"));
        assert_eq!(result.state.tick, 120);
    }

    #[test]
    fn snapshot_with_raw_ticks() {
        let state = GameState::new_default(42);
        // Legacy: t10 = 10 raw ticks
        let result = run_snapshot(state, &["t10".to_string()]).unwrap();
        assert!(result.screen.contains("Day: 0.1"));
        assert_eq!(result.state.tick, 10);
    }

    #[test]
    fn snapshot_with_key() {
        let state = GameState::new_default(42);
        let result = run_snapshot(state, &["t".to_string()]).unwrap();
        assert!(result.screen.contains("Threats"));
        // Diseases start undetected — shown as "?" until detection threshold is reached
        assert!(result.screen.contains("?"));
    }

    #[test]
    fn snapshot_with_multiple_keys() {
        let state = GameState::new_default(42);
        // Navigate to Threats then press down to select second item
        let result = run_snapshot(state, &["t".to_string(), "down".to_string()]).unwrap();
        assert!(result.screen.contains("Threats"));
    }

    #[test]
    fn snapshot_invalid_key() {
        let state = GameState::new_default(42);
        let result = run_snapshot(state, &["!".to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown key"));
    }

    #[test]
    fn snapshot_interleaved_days_and_keys() {
        let state = GameState::new_default(42);
        // Advance 0.5 days (60 ticks), open threats panel, advance 0.5 more days
        let result = run_snapshot(
            state,
            &["d0.5".to_string(), "t".to_string(), "d0.5".to_string()],
        ).unwrap();
        assert_eq!(result.state.tick, 120);
        assert!(result.screen.contains("Threats"));
        assert!(result.screen.contains("Day: 1.0"));
    }

    #[test]
    fn snapshot_auto_resolves_crises() {
        let state = GameState::new_default(42);
        // d60 will hit crises. They should be auto-resolved inline.
        let result = run_snapshot(state, &["d60".to_string()]).unwrap();
        assert!(result.state.active_crisis.is_none(),
            "crises should be auto-resolved during tick advancement");
        // Should have advanced well past the first crisis point.
        // (May not reach full 60 days if game over occurs.)
        assert!(result.state.tick > 360,
            "should advance past first crisis point (tick {})", result.state.tick);
    }

    #[test]
    fn snapshot_key_not_eaten_by_crisis() {
        let state = GameState::new_default(42);
        // Advance enough days to trigger a crisis, then open threats panel.
        // Previously 't' would be eaten by the crisis. Now the crisis is
        // auto-resolved before 't' is processed.
        let result = run_snapshot(
            state,
            &["d60".to_string(), "t".to_string()],
        ).unwrap();
        // Threats panel should be open (key was not eaten).
        assert!(result.screen.contains("Threats"),
            "threats panel should be open — key should not be eaten by crisis");
    }

    #[test]
    fn snapshot_stops_on_game_over() {
        let mut state = GameState::new_default(42);
        // Force game over state
        state.outcome = GameOutcome::Lost;
        let tick_before = state.tick;
        let result = run_snapshot(state, &["d10".to_string()]).unwrap();
        // tick() returns early when not Playing, so tick shouldn't advance
        assert_eq!(result.state.outcome, GameOutcome::Lost);
        assert_eq!(result.state.tick, tick_before,
            "tick should not advance after game over");
    }

    #[test]
    fn snapshot_auto_resumes_on_disease_detection_pause() {
        let mut state = GameState::new_default(42);
        // Set first disease just below detection threshold so it triggers during ticks.
        // Detection threshold is 10,000 total infected across all regions.
        state.diseases[0].detected = false;
        let near_threshold = 9_900.0;
        state.regions[0].get_or_create_infection(0).infected = near_threshold;

        // With the disease growing, detection should trigger during these ticks.
        let result = run_snapshot(state, &["d1".to_string()]).unwrap();

        // The disease should now be detected
        assert!(result.state.diseases[0].detected,
            "disease should be detected after crossing threshold");
        // All ticks should have completed (pause was auto-resumed, not blocking)
        assert_eq!(result.state.tick, 120,
            "all 120 ticks should complete — pause should auto-resume (got {})", result.state.tick);
    }

    #[test]
    fn defeat_screen_shows_collapse_timeline() {
        let mut state = GameState::new_default(42);
        state.outcome = GameOutcome::Lost;
        // Simulate collapse at different times
        state.regions[0].collapsed = true;
        state.regions[0].collapsed_at_tick = Some(600);
        state.regions[2].collapsed = true;
        state.regions[2].collapsed_at_tick = Some(1200);
        let screen = render_to_string(&state);
        assert!(screen.contains("Collapse Timeline"),
            "defeat screen should show collapse timeline");
        assert!(screen.contains("FELL"),
            "collapsed regions should show FELL");
        assert!(screen.contains("held"),
            "standing regions should show 'held'");
    }

    #[test]
    fn policy_panel_shows_decrees() {
        let state = GameState::new_default(42);
        let result = run_snapshot(state, &["p".to_string()]).unwrap();
        assert!(result.screen.contains("EMERGENCY DECREES"),
            "policy panel should show decrees section");
        assert!(result.screen.contains("Conscript"),
            "should show Conscript Researchers decree");
        assert!(result.screen.contains("Human Trials"),
            "should show Authorize Human Trials decree");
        assert!(result.screen.contains("Sacrifice"),
            "should show Sacrifice Region decree");
    }

    #[test]
    fn region_detail_shows_governor() {
        let state = GameState::new_default(42);
        let screen = render_to_string(&state);
        // Governor name and personality should be visible in the selected region's detail
        assert!(screen.contains("Gov."),
            "region detail should show governor name");
        assert!(screen.contains("Loyalty:"),
            "region detail should show governor loyalty");
    }

    #[test]
    fn policy_panel_shows_appease() {
        let state = GameState::new_default(42);
        // Open policy panel, enter first region's management
        let result = run_snapshot(state, &["p".to_string(), "enter".to_string()]).unwrap();
        assert!(result.screen.contains("Appease Gov."),
            "policy management should show Appease option");
    }

    #[test]
    fn defeat_screen_shows_pathogen_report_and_score() {
        let mut state = GameState::new_default(42);
        state.outcome = GameOutcome::Lost;
        state.tick = 2400; // 20 days
        // Give disease 0 some deaths
        state.regions[0].get_or_create_infection(0).dead = 50_000.0;
        state.regions[0].dead = 50_000.0;
        let screen = render_to_string(&state);
        assert!(screen.contains("Pathogen Report"),
            "defeat screen should show pathogen report");
        assert!(screen.contains("Score"),
            "defeat screen should show score");
        // Disease names are revealed on defeat (even unidentified ones)
        assert!(screen.contains(&state.diseases[0].name),
            "defeat screen should reveal true disease name");
    }

}
