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
    SelectLeft,
    SelectRight,
    Confirm,
    SpeedUp,
    ToggleAutoResolve,
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
        KeyCode::Left | KeyCode::Char('h') => Some(Action::SelectLeft),
        KeyCode::Right | KeyCode::Char('l') => Some(Action::SelectRight),
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Char('z') | KeyCode::Char('Z') => Some(Action::SpeedUp),
        KeyCode::Char('x') | KeyCode::Char('X') => Some(Action::ToggleAutoResolve),
        KeyCode::Char('q') | KeyCode::Char('Q') => Some(Action::Quit),
        _ => None,
    }
}

/// Map a string key name (from --key flag) to an Action.
/// Case-insensitive for named keys (space, esc, enter, etc.).
pub fn string_to_action(s: &str) -> Option<Action> {
    let lower = s.to_lowercase();
    match lower.as_str() {
        " " | "space" => Some(Action::TogglePause),
        "t" => Some(Action::OpenThreats),
        "r" => Some(Action::OpenResearch),
        "m" => Some(Action::OpenMedicines),
        "p" => Some(Action::OpenPolicy),
        "?" => Some(Action::OpenHelp),
        "esc" => Some(Action::ClosePanel),
        "down" | "j" => Some(Action::SelectNext),
        "up" | "k" => Some(Action::SelectPrev),
        "left" | "h" => Some(Action::SelectLeft),
        "right" | "l" => Some(Action::SelectRight),
        "enter" => Some(Action::Confirm),
        "z" => Some(Action::SpeedUp),
        "x" => Some(Action::ToggleAutoResolve),
        "q" => Some(Action::Quit),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_to_action_case_insensitive() {
        assert_eq!(string_to_action("space"), Some(Action::TogglePause));
        assert_eq!(string_to_action("Space"), Some(Action::TogglePause));
        assert_eq!(string_to_action("SPACE"), Some(Action::TogglePause));
        assert_eq!(string_to_action(" "), Some(Action::TogglePause));

        assert_eq!(string_to_action("esc"), Some(Action::ClosePanel));
        assert_eq!(string_to_action("Esc"), Some(Action::ClosePanel));

        assert_eq!(string_to_action("enter"), Some(Action::Confirm));
        assert_eq!(string_to_action("Enter"), Some(Action::Confirm));

        assert_eq!(string_to_action("Down"), Some(Action::SelectNext));
        assert_eq!(string_to_action("UP"), Some(Action::SelectPrev));

        assert_eq!(string_to_action("T"), Some(Action::OpenThreats));
        assert_eq!(string_to_action("M"), Some(Action::OpenMedicines));
        assert_eq!(string_to_action("Q"), Some(Action::Quit));
    }
}
