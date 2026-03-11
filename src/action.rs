use crossterm::event::KeyCode;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    TogglePause,
    OpenThreats,
    OpenResearch,
    OpenMedicines,
    OpenPolicy,
    OpenOperations,
    OpenHelp,
    ClosePanel,
    SelectNext,
    SelectPrev,
    SelectLeft,
    SelectRight,
    Confirm,
    ToggleExtra,
    SpeedUp,
    Quit,
    /// Jump directly to item N in the current panel list (0-based).
    /// Keys 1–9 → index 0–8, key 0 → index 9.
    JumpToItem { index: usize },
    /// Close all panels and return to the main dashboard in one keypress.
    /// Unlike ClosePanel (Esc), which goes back one step, GoHome goes all the way.
    GoHome,
}

/// Map a crossterm KeyCode to an Action.
pub fn key_to_action(key: KeyCode) -> Option<Action> {
    match key {
        KeyCode::Char(' ') => Some(Action::TogglePause),
        KeyCode::Char('t') | KeyCode::Char('T') => Some(Action::OpenThreats),
        KeyCode::Char('r') | KeyCode::Char('R') => Some(Action::OpenResearch),
        KeyCode::Char('m') | KeyCode::Char('M') => Some(Action::OpenMedicines),
        KeyCode::Char('p') | KeyCode::Char('P') => Some(Action::OpenPolicy),
        KeyCode::Char('o') | KeyCode::Char('O') => Some(Action::OpenOperations),
        KeyCode::Char('?') => Some(Action::OpenHelp),
        KeyCode::Esc => Some(Action::ClosePanel),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectNext),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectPrev),
        KeyCode::Left | KeyCode::Char('h') => Some(Action::SelectLeft),
        KeyCode::Right | KeyCode::Char('l') => Some(Action::SelectRight),
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Char('x') | KeyCode::Char('X') => Some(Action::ToggleExtra),
        KeyCode::Char('z') | KeyCode::Char('Z') => Some(Action::SpeedUp),
        KeyCode::Char('q') | KeyCode::Char('Q') => Some(Action::Quit),
        KeyCode::Char('1') => Some(Action::JumpToItem { index: 0 }),
        KeyCode::Char('2') => Some(Action::JumpToItem { index: 1 }),
        KeyCode::Char('3') => Some(Action::JumpToItem { index: 2 }),
        KeyCode::Char('4') => Some(Action::JumpToItem { index: 3 }),
        KeyCode::Char('5') => Some(Action::JumpToItem { index: 4 }),
        KeyCode::Char('6') => Some(Action::JumpToItem { index: 5 }),
        KeyCode::Char('7') => Some(Action::JumpToItem { index: 6 }),
        KeyCode::Char('8') => Some(Action::JumpToItem { index: 7 }),
        KeyCode::Char('9') => Some(Action::JumpToItem { index: 8 }),
        KeyCode::Char('0') => Some(Action::JumpToItem { index: 9 }),
        KeyCode::Home => Some(Action::GoHome),
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
        "o" => Some(Action::OpenOperations),
        "?" => Some(Action::OpenHelp),
        "esc" => Some(Action::ClosePanel),
        "down" | "j" => Some(Action::SelectNext),
        "up" | "k" => Some(Action::SelectPrev),
        "left" | "h" => Some(Action::SelectLeft),
        "right" | "l" => Some(Action::SelectRight),
        "enter" => Some(Action::Confirm),
        "x" => Some(Action::ToggleExtra),
        "z" => Some(Action::SpeedUp),
        "q" => Some(Action::Quit),
        "1" => Some(Action::JumpToItem { index: 0 }),
        "2" => Some(Action::JumpToItem { index: 1 }),
        "3" => Some(Action::JumpToItem { index: 2 }),
        "4" => Some(Action::JumpToItem { index: 3 }),
        "5" => Some(Action::JumpToItem { index: 4 }),
        "6" => Some(Action::JumpToItem { index: 5 }),
        "7" => Some(Action::JumpToItem { index: 6 }),
        "8" => Some(Action::JumpToItem { index: 7 }),
        "9" => Some(Action::JumpToItem { index: 8 }),
        "0" => Some(Action::JumpToItem { index: 9 }),
        "home" => Some(Action::GoHome),
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

        assert_eq!(string_to_action("home"), Some(Action::GoHome));
        assert_eq!(string_to_action("Home"), Some(Action::GoHome));
        assert_eq!(string_to_action("HOME"), Some(Action::GoHome));
    }

    #[test]
    fn digit_keys_map_to_jump_to_item() {
        assert_eq!(string_to_action("1"), Some(Action::JumpToItem { index: 0 }));
        assert_eq!(string_to_action("5"), Some(Action::JumpToItem { index: 4 }));
        assert_eq!(string_to_action("9"), Some(Action::JumpToItem { index: 8 }));
        assert_eq!(string_to_action("0"), Some(Action::JumpToItem { index: 9 }));
        assert_eq!(string_to_action("a"), None);
    }
}
