use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::GameState;

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let mut lines: Vec<Line> = Vec::new();

    // ASCII art biohazard + title
    let red = Style::default().fg(Color::Red);
    let white = Style::default().fg(Color::White);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("       ██████████       ", red)));
    lines.push(Line::from(Span::styled("     ██░░░░░░░░░░██     ", red)));
    lines.push(Line::from(Span::styled("    █░░  ██████  ░░█    ", red)));
    lines.push(Line::from(Span::styled("   █░░ ██      ██ ░░█   ", red)));
    lines.push(Line::from(Span::styled("   █░░██  ████  ██░░█   ", red)));
    lines.push(Line::from(Span::styled("   █░░██ ██  ██ ██░░█   ", red)));
    lines.push(Line::from(Span::styled("    █░░ ██ ░░ ██ ░░█    ", red)));
    lines.push(Line::from(Span::styled("     ██  ██░░██  ██     ", red)));
    lines.push(Line::from(Span::styled("       ████░░████       ", red)));
    lines.push(Line::from(Span::styled("           ░░           ", red)));
    lines.push(Line::from(Span::styled("       ████████████     ", red)));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "    P A N D E M I C  C.L.I.",
        white,
    )));
    lines.push(Line::from(""));

    // Getting started guide
    let dim = Style::default().fg(Color::DarkGray);
    let cyan = Style::default().fg(Color::Cyan);
    let yellow = Style::default().fg(Color::Yellow);

    lines.push(Line::from(Span::styled("  ── Getting Started ──", cyan)));
    lines.push(Line::from(""));

    // Tailor guidance to game state
    let has_unknown = state.diseases.iter().enumerate().any(|(i, d)| {
        d.display_name(i).starts_with("Unknown")
    });
    let has_identified = state.diseases.iter().any(|d| d.knowledge >= 0.33);
    let has_unlocked_medicine = state.medicines.iter().any(|m| m.unlocked);
    let any_policy = state.policies.iter().any(|p| p.any_active());

    if has_unknown && state.field_research.is_none() {
        lines.push(Line::from(Span::styled(
            "  → Press [R] to start Research",
            yellow,
        )));
        lines.push(Line::from(Span::styled(
            "    Identify unknown threats first!",
            dim,
        )));
    } else if state.field_research.is_some() || state.bench_research.is_some() {
        lines.push(Line::from(Span::styled(
            "  → Research in progress...",
            dim,
        )));
        lines.push(Line::from(Span::styled(
            "    Press [R] to check status or boost",
            dim,
        )));
    }

    if has_identified && !has_unlocked_medicine {
        lines.push(Line::from(Span::styled(
            "  → Develop medicines in [R] Research",
            yellow,
        )));
        lines.push(Line::from(Span::styled(
            "    Bench research → Develop Medicine",
            dim,
        )));
    }

    if has_unlocked_medicine {
        lines.push(Line::from(Span::styled(
            "  → Deploy medicines with [M]",
            yellow,
        )));
    }

    if !any_policy {
        lines.push(Line::from(Span::styled(
            "  → Set policies with [P]",
            yellow,
        )));
        lines.push(Line::from(Span::styled(
            "    Quarantine, travel bans, and more",
            dim,
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  ── Panels ──", cyan)));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  [T] Threats   — view diseases", dim)));
    lines.push(Line::from(Span::styled("  [R] Research  — identify & develop", dim)));
    lines.push(Line::from(Span::styled("  [M] Medicines — deploy treatments", dim)));
    lines.push(Line::from(Span::styled("  [P] Policy    — contain outbreaks", dim)));
    lines.push(Line::from(Span::styled("  [?] Help      — full controls", dim)));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  [Space] Pause  [←/→] Regions", dim)));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}
