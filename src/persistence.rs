//! Centralized save/load for game state.
//!
//! Both interactive and snapshot modes go through this module for all
//! file I/O, deserialization, migration, and bootstrap. Saves use the
//! `SaveFile` format (world + UI state); session state is reconstructed
//! fresh on every load.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::engine;
use crate::state::{AppState, SaveFile, SessionState, UiState, WorldState};

/// A ready-to-run game state loaded (or created) from a persistence path.
pub struct LoadedGame {
    pub state: AppState,
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
/// This is the single entry point for obtaining a ready-to-run `AppState`.
/// It handles: file I/O, deserialization, migration, and game-system bootstrap
/// (`initialize_game`). Both interactive and snapshot callers use this.
pub fn load_or_create(path: Option<&str>, seed: u64) -> Result<LoadedGame, PersistenceError> {
    let (mut world, ui) = match path {
        Some(p) if Path::new(p).exists() => {
            let data = fs::read_to_string(p)?;
            if data.trim().is_empty() {
                (WorldState::new_default(seed), UiState::default())
            } else {
                let mut sf: SaveFile = serde_json::from_str(&data).map_err(|e| {
                    PersistenceError::Corrupt {
                        path: p.to_string(),
                        detail: e.to_string(),
                    }
                })?;
                sf.world.migrate();
                (sf.world, sf.ui)
            }
        }
        _ => (WorldState::new_default(seed), UiState::default()),
    };

    // Bootstrap game systems (corporations, board) for new games.
    // Loaded saves already have this data; initialize_game checks and skips.
    engine::initialize_game(&mut world);

    Ok(LoadedGame {
        state: AppState {
            world,
            ui,
            session: SessionState::default(),
        },
    })
}

/// Save game state to `path`, creating parent directories as needed.
///
/// Uses atomic write (temp file + rename) so an interrupted save never
/// leaves a truncated or empty file at the target path.
pub fn save(state: &AppState, path: &str) -> Result<(), PersistenceError> {
    let target = Path::new(path);
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let sf = SaveFile {
        world: state.world.clone(),
        ui: state.ui.clone(),
    };
    let json = serde_json::to_string_pretty(&sf)
        .map_err(|e| PersistenceError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

    // Write to a sibling temp file, then atomically rename.
    let tmp_path = target.with_extension("tmp");
    fs::write(&tmp_path, &json)?;
    fs::rename(&tmp_path, target)?;
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
