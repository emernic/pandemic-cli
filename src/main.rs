use std::io;
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use pandemic_cli_lib::action::{key_to_action, Action};
use pandemic_cli_lib::apply_action;
use pandemic_cli_lib::persistence;
use pandemic_cli_lib::snapshot;
use pandemic_cli_lib::state::GameState;
use pandemic_cli_lib::tick_and_process;
use pandemic_cli_lib::ui;

#[derive(Parser)]
#[command(name = "pandemic-cli", about = "Defend humanity against disease outbreaks")]
struct Cli {
    /// Save file to load/save game state
    save_file: Option<String>,

    /// Run in snapshot mode (non-interactive, render to stdout)
    #[arg(long)]
    snapshot: bool,

    /// Apply key action(s) before rendering (snapshot mode, repeatable)
    #[arg(long)]
    key: Vec<String>,

    /// Advance this many days (snapshot mode). 1 day = 120 ticks.
    #[arg(long)]
    days: Option<f64>,

    /// Ordered sequence of steps (snapshot mode, repeatable).
    /// Use d<N> for days (e.g. d1 = 1 day = 120 ticks), anything else is a key action.
    /// Example: --do d1 --do r --do enter --do d2.5
    #[arg(long = "do")]
    steps: Vec<String>,

    /// RNG seed for new games (random if not specified)
    #[arg(long)]
    seed: Option<u64>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let seed = cli.seed.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
    });

    let mut snapshot_autosave_notice = None;
    // Use explicit path, or default to ./save.json for interactive mode.
    // Snapshot mode now also auto-creates a local save file so runs are resumable.
    let save_file = if let Some(path) = cli.save_file.clone() {
        Some(path)
    } else if cli.snapshot {
        let path = persistence::auto_snapshot_save_path();
        snapshot_autosave_notice = Some(path.clone());
        Some(path)
    } else {
        Some("save.json".into())
    };

    // Load or create state through the centralized persistence seam
    let loaded = persistence::load_or_create(save_file.as_deref(), seed)
        .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })?;
    let state = loaded.state;

    if cli.snapshot {
        // Build step sequence: --do takes priority if provided, otherwise
        // fall back to legacy --key/--ticks args
        let steps = if !cli.steps.is_empty() {
            cli.steps
        } else {
            let mut s: Vec<String> = cli.key.into_iter().collect();
            if let Some(d) = cli.days {
                s.push(format!("d{d}"));
            }
            s
        };
        let result = snapshot::run_snapshot(state, &steps)
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        // Write save file BEFORE printing to stdout — if output is piped
        // through `head` or similar, SIGPIPE kills the process on print,
        // so the save must happen first.
        if let Some(ref path) = save_file {
            persistence::save(&result.state, path)
                .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })?;
        }
        if let Some(ref path) = snapshot_autosave_notice {
            println!("No save file passed in. Creating {}.", path);
            println!(
                "Run `cargo run -- {} --snapshot` to continue this playthrough.",
                path
            );
            println!();
        }
        print!("{}", result.screen);
        Ok(())
    } else {
        run_interactive(state, save_file)
    }
}

fn install_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Best-effort terminal cleanup
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));
}

fn run_interactive(
    state: GameState,
    save_path: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    install_panic_hook();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = game_loop(&mut terminal, state);

    // Always cleanup, even if game_loop errored
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    let state = result?; // Now propagate error

    // Save state on quit
    if let Some(path) = save_path {
        persistence::save(&state, &path)
            .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })?;
        eprintln!("Game saved to {}", path);
    }

    Ok(())
}

/// Minimum time after a crisis popup before Enter is accepted,
/// preventing accidental confirmation from keypresses meant for other UI.
const EVENT_INPUT_LOCKOUT: Duration = Duration::from_millis(500);

fn game_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut state: GameState,
) -> Result<GameState, Box<dyn std::error::Error>> {
    let tick_duration = Duration::from_millis(500);
    let mut last_tick = Instant::now();
    // Track when a crisis event appeared so we can block Enter briefly
    let mut event_appeared_at: Option<Instant> = if state.active_crisis.is_some() {
        Some(Instant::now())
    } else {
        None
    };

    loop {
        terminal.draw(|f| {
            ui::render(f, &state);
        })?;

        let effective_tick = tick_duration / state.session.speed_multiplier.max(1) as u32;
        let timeout = if !state.is_effectively_running() {
            Duration::from_millis(100)
        } else {
            effective_tick
                .checked_sub(last_tick.elapsed())
                .unwrap_or(Duration::ZERO)
        };

        if event::poll(timeout)? {
            if let Event::Key(key_event) = event::read()? {
                // Only handle key press events (not release/repeat)
                if key_event.kind == KeyEventKind::Press {
                    // When the size warning overlay is showing, only X/Esc/Q work
                    let term_size = terminal.size().unwrap_or_default();
                    if ui::is_size_warning_active(&state, term_size.width, term_size.height) {
                        match key_event.code {
                            KeyCode::Char('x') | KeyCode::Char('X') | KeyCode::Esc => {
                                state.session.size_warning_dismissed = true;
                            }
                            KeyCode::Char('q') | KeyCode::Char('Q') => {
                                return Ok(state);
                            }
                            _ => {} // swallow all other keys
                        }
                        continue;
                    }
                    if let Some(action) = key_to_action(key_event.code) {
                        if action == Action::Quit {
                            return Ok(state);
                        }
                        // Block Enter during crisis input lockout to prevent
                        // accidental confirmation from keypresses aimed at other UI
                        if action == Action::Confirm {
                            if let Some(appeared) = event_appeared_at {
                                if appeared.elapsed() < EVENT_INPUT_LOCKOUT {
                                    continue;
                                }
                            }
                        }
                        let was_stopped = !state.is_effectively_running();
                        state = apply_action(&state, &action);
                        // Reset tick timer on unpause to avoid burst of catch-up ticks
                        if was_stopped && state.is_effectively_running() {
                            last_tick = Instant::now();
                        }
                    }
                }
            }
        }

        // Auto-tick when effectively running (not paused, no crisis, not game over)
        if state.is_effectively_running() && last_tick.elapsed() >= effective_tick {
            let had_crisis = state.active_crisis.is_some();
            state = tick_and_process(&state);
            last_tick += effective_tick;
            // Detect when a new crisis appears and start the input lockout
            if !had_crisis && state.active_crisis.is_some() {
                event_appeared_at = Some(Instant::now());
            }
        }

        // Clear lockout tracking when crisis is resolved
        if state.active_crisis.is_none() {
            event_appeared_at = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use pandemic_cli_lib::persistence;

    #[test]
    fn auto_snapshot_save_uses_gitignored_saves_directory() {
        let path = persistence::auto_snapshot_save_path();
        assert!(path.starts_with("saves/"));
        assert!(path.ends_with(".json"));
        assert!(path.contains("playtest-"));
    }
}
