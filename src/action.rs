use crossterm::event::KeyCode;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    TogglePause,
    OpenThreats,
    OpenResearch,
    OpenMedicines,
    OpenPolicy,
    OpenHelp,
    ClosePanel,
    SelectNext,
    SelectPrev,
    Quit,
}

/// Map a crossterm KeyCode to an Action.
pub fn key_to_action(key: KeyCode) -> Option<Action> {
    match key {
        KeyCode::Char(' ') => Some(Action::TogglePause),
        KeyCode::Char('t') | KeyCode::Char('T') => Some(Action::OpenThreats),
        KeyCode::Char('r') | KeyCode::Char('R') => Some(Action::OpenResearch),
        KeyCode::Char('m') | KeyCode::Char('M') => Some(Action::OpenMedicines),
        KeyCode::Char('p') | KeyCode::Char('P') => Some(Action::OpenPolicy),
        KeyCode::Char('?') => Some(Action::OpenHelp),
        KeyCode::Esc => Some(Action::ClosePanel),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectNext),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectPrev),
        KeyCode::Char('q') | KeyCode::Char('Q') => Some(Action::Quit),
        _ => None,
    }
}

/// Map a string key name (from --key flag) to an Action.
pub fn string_to_action(s: &str) -> Option<Action> {
    match s {
        " " | "space" => Some(Action::TogglePause),
        "t" | "T" => Some(Action::OpenThreats),
        "r" | "R" => Some(Action::OpenResearch),
        "m" | "M" => Some(Action::OpenMedicines),
        "p" | "P" => Some(Action::OpenPolicy),
        "?" => Some(Action::OpenHelp),
        "esc" | "Esc" => Some(Action::ClosePanel),
        "down" | "Down" | "j" => Some(Action::SelectNext),
        "up" | "Up" | "k" => Some(Action::SelectPrev),
        "q" | "Q" => Some(Action::Quit),
        _ => None,
    }
}
