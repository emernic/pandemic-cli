//! Centralized save/load for game state.
//!
//! Both interactive and snapshot modes go through this module for all
//! file I/O, deserialization, migration, and bootstrap. The later
//! state-split cutover can change this one seam instead of every caller.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::engine;
use crate::state::GameState;

/// A ready-to-run game state loaded (or created) from a persistence path.
///
/// Callers receive this after `load_or_create` and can start running
/// the game immediately. The later state-split work will add fresh
/// `SessionState` construction here without changing callers.
pub struct LoadedGame {
    pub state: GameState,
}

/// Errors from persistence operations.
#[derive(Debug)]
pub enum PersistenceError {
    Io(std::io::Error),
    Corrupt { path: String, detail: String },
}

impl std::fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistenceError::Io(e) => write!(f, "{}", e),
            PersistenceError::Corrupt { path, detail } => write!(
                f,
                "Failed to load save file '{}': {}\nThe file may be corrupted. Delete it to start a new game.",
                path, detail
            ),
        }
    }
}

impl std::error::Error for PersistenceError {}

impl From<std::io::Error> for PersistenceError {
    fn from(e: std::io::Error) -> Self {
        PersistenceError::Io(e)
    }
}

/// Load a game from `path`, or create a new one if the file is missing/empty.
///
/// This is the single entry point for obtaining a ready-to-run `GameState`.
/// It handles: file I/O, deserialization, migration, and game-system bootstrap
/// (`initialize_game`). Both interactive and snapshot callers use this.
pub fn load_or_create(path: Option<&str>, seed: u64) -> Result<LoadedGame, PersistenceError> {
    let mut state = match path {
        Some(p) if Path::new(p).exists() => {
            let data = fs::read_to_string(p)?;
            if data.trim().is_empty() {
                GameState::new_default(seed)
            } else {
                let mut s: GameState = serde_json::from_str(&data).map_err(|e| {
                    PersistenceError::Corrupt {
                        path: p.to_string(),
                        detail: e.to_string(),
                    }
                })?;
                s.migrate();
                s
            }
        }
        _ => GameState::new_default(seed),
    };

    // Bootstrap game systems (corporations, board) for new games.
    // Loaded saves already have this data; initialize_game checks and skips.
    engine::initialize_game(&mut state);

    Ok(LoadedGame { state })
}

/// Save game state to `path`, creating parent directories as needed.
///
/// Used by both snapshot (save-before-print) and interactive (save-on-quit).
pub fn save(state: &GameState, path: &str) -> Result<(), PersistenceError> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| PersistenceError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
    fs::write(path, json)?;
    Ok(())
}

/// Generate an auto-save path for snapshot mode under `saves/`.
///
/// Uses PID + nanosecond timestamp to avoid collisions across parallel agents.
pub fn auto_snapshot_save_path() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    let mut path = PathBuf::from("saves");
    path.push(format!("playtest-{}-{}.json", std::process::id(), nanos));
    path.to_string_lossy().into_owned()
}
