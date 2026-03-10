use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use pandemic_cli_lib::action::{key_to_action, Action};
use pandemic_cli_lib::apply_action;
use pandemic_cli_lib::engine::tick;
use pandemic_cli_lib::snapshot;
use pandemic_cli_lib::state::GameState;
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

    /// RNG seed for new games
    #[arg(long, default_value = "42")]
    seed: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let mut snapshot_autosave_notice = None;
    // Use explicit path, or default to ./save.json for interactive mode.
    // Snapshot mode now also auto-creates a local save file so runs are resumable.
    let save_file = if let Some(path) = cli.save_file.clone() {
        Some(path)
    } else if cli.snapshot {
        let path = auto_snapshot_save_path();
        snapshot_autosave_notice = Some(path.clone());
        Some(path)
    } else {
        Some("save.json".into())
    };

    // Load or create state
    let mut state: GameState = if let Some(ref path) = save_file {
        if std::path::Path::new(path).exists() {
            let data = fs::read_to_string(path)?;
            if data.trim().is_empty() {
                GameState::new_default(cli.seed)
            } else {
                let mut s: GameState = serde_json::from_str(&data)
                    .map_err(|e| format!(
                        "Failed to load save file '{}': {}\nThe file may be corrupted. Delete it to start a new game.",
                        path, e
                    ))?;
                s.migrate();
                s
            }
        } else {
            GameState::new_default(cli.seed)
        }
    } else {
        GameState::new_default(cli.seed)
    };

    // Generate corporations for new games (loaded saves already have them)
    if state.corporations.is_empty() {
        pandemic_cli_lib::engine::corporations::generate_corporations(&mut state);
    }

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
            if let Some(parent) = std::path::Path::new(path).parent() {
                fs::create_dir_all(parent)?;
            }
            let json = serde_json::to_string_pretty(&result.state)?;
            fs::write(path, json)?;
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

fn auto_snapshot_save_path() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    let mut path = PathBuf::from("saves");
    path.push(format!("playtest-{}-{}.json", std::process::id(), nanos));
    path.to_string_lossy().into_owned()
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
        if let Some(parent) = std::path::Path::new(&path).parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&state)?;
        fs::write(&path, json)?;
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

        let effective_tick = tick_duration / state.ui.speed_multiplier.max(1) as u32;
        let timeout = if !state.sim_state.is_running() {
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
                        let was_stopped = !state.sim_state.is_running();
                        state = apply_action(&state, &action);
                        // Reset tick timer on unpause to avoid burst of catch-up ticks
                        if was_stopped && state.sim_state.is_running() {
                            last_tick = Instant::now();
                        }
                    }
                }
            }
        }

        // Auto-tick when unpaused
        if state.sim_state.is_running() && last_tick.elapsed() >= effective_tick {
            let had_crisis = state.active_crisis.is_some();
            state = tick(&state);
            ui::process_events(&mut state);
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
    use super::auto_snapshot_save_path;

    #[test]
    fn auto_snapshot_save_uses_gitignored_saves_directory() {
        let path = auto_snapshot_save_path();
        assert!(path.starts_with("saves/"));
        assert!(path.ends_with(".json"));
        assert!(path.contains("playtest-"));
    }
}
