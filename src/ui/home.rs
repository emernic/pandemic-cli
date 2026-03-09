use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::GameState;

/// Build the full splash content as a flat string (for typewriter truncation)
/// and as styled lines (for final rendering). Returns (plain_text, styled_lines).
fn build_splash_content(state: &GameState) -> Vec<(String, Style)> {
    let red = Style::default().fg(Color::Red);
    let white = Style::default().fg(Color::White);
    let dim = Style::default().fg(Color::DarkGray);
    let cyan = Style::default().fg(Color::Cyan);
    let yellow = Style::default().fg(Color::Yellow);

    let mut segments: Vec<(String, Style)> = Vec::new();

    // ASCII art biohazard + title
    let art_lines = [
        "",
        "       ██████████       ",
        "     ██░░░░░░░░░░██     ",
        "    █░░  ██████  ░░█    ",
        "   █░░ ██      ██ ░░█   ",
        "   █░░██  ████  ██░░█   ",
        "   █░░██ ██  ██ ██░░█   ",
        "    █░░ ██ ░░ ██ ░░█    ",
        "     ██  ██░░██  ██     ",
        "       ████░░████       ",
        "           ░░           ",
        "       ████████████     ",
        "",
    ];
    for line in &art_lines {
        segments.push((format!("{}\n", line), red));
    }

    segments.push(("    P A N D E M I C  C.L.I.\n".to_string(), white));
    segments.push(("\n".to_string(), dim));

    segments.push(("  ── Getting Started ──\n".to_string(), cyan));
    segments.push(("\n".to_string(), dim));

    // Tailor guidance to game state
    let has_unknown = state.diseases.iter().enumerate().any(|(i, d)| {
        d.display_name(i).starts_with("Unknown")
    });
    let has_identified = state.diseases.iter().any(|d| d.knowledge >= 0.33);
    let has_unlocked_medicine = state.medicines.iter().any(|m| m.unlocked);
    let any_policy = state.policies.iter().any(|p| p.any_active());

    if has_unknown && state.field_research.is_none() {
        segments.push(("  → Press [R] to start Research\n".to_string(), yellow));
        segments.push(("    Identify unknown threats first!\n".to_string(), dim));
    } else if state.field_research.is_some() || state.bench_research.is_some() {
        segments.push(("  → Research in progress...\n".to_string(), dim));
        segments.push(("    Press [R] to check status or boost\n".to_string(), dim));
    }

    if has_identified && !has_unlocked_medicine {
        segments.push(("  → Develop medicines in [R] Research\n".to_string(), yellow));
        segments.push(("    Bench research → Develop Medicine\n".to_string(), dim));
    }

    if has_unlocked_medicine {
        segments.push(("  → Deploy medicines with [M]\n".to_string(), yellow));
    }

    if !any_policy {
        segments.push(("  → Set policies with [P]\n".to_string(), yellow));
        segments.push(("    Quarantine, travel bans, and more\n".to_string(), dim));
    }

    segments.push(("\n".to_string(), dim));
    segments.push(("  ── Panels ──\n".to_string(), cyan));
    segments.push(("\n".to_string(), dim));
    segments.push(("  [T] Threats   — view diseases\n".to_string(), dim));
    segments.push(("  [R] Research  — identify & develop\n".to_string(), dim));
    segments.push(("  [M] Medicines — deploy treatments\n".to_string(), dim));
    segments.push(("  [P] Policy    — contain outbreaks\n".to_string(), dim));
    segments.push(("  [?] Help      — full controls\n".to_string(), dim));
    segments.push(("\n".to_string(), dim));
    segments.push(("  [Space] Pause  [←/→] Regions\n".to_string(), dim));

    segments
}

/// Render styled segments truncated to `max_chars` characters, with a cursor at the end.
fn render_truncated(segments: &[(String, Style)], max_chars: usize) -> Vec<Line<'static>> {
    let cursor_style = Style::default().fg(Color::Green);
    let mut lines: Vec<Line> = Vec::new();
    let mut current_spans: Vec<Span> = Vec::new();
    let mut chars_shown = 0;

    let mut done = false;
    for (text, style) in segments {
        if done { break; }
        for ch in text.chars() {
            if chars_shown >= max_chars {
                // Add cursor at the truncation point
                current_spans.push(Span::styled("█", cursor_style));
                lines.push(Line::from(std::mem::take(&mut current_spans)));
                done = true;
                break;
            }
            if ch == '\n' {
                lines.push(Line::from(std::mem::take(&mut current_spans)));
            } else {
                current_spans.push(Span::styled(ch.to_string(), *style));
            }
            chars_shown += 1;
        }
    }

    // Flush remaining spans if we didn't hit the limit
    if !done && !current_spans.is_empty() {
        lines.push(Line::from(current_spans));
    }

    lines
}

/// Render styled segments fully (no truncation).
fn render_full(segments: &[(String, Style)]) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();
    let mut current_spans: Vec<Span> = Vec::new();

    for (text, style) in segments {
        for ch in text.chars() {
            if ch == '\n' {
                lines.push(Line::from(current_spans));
                current_spans = Vec::new();
            } else {
                current_spans.push(Span::styled(ch.to_string(), *style));
            }
        }
    }

    if !current_spans.is_empty() {
        lines.push(Line::from(current_spans));
    }

    lines
}

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let segments = build_splash_content(state);

    let lines = if state.ui.home_splash_done {
        render_full(&segments)
    } else {
        // Animation: reveal ~2 characters per tick for a nice pace
        let chars_to_show = (state.tick as usize) * 2;
        let total_chars: usize = segments.iter().map(|(s, _)| s.len()).sum();
        if chars_to_show >= total_chars {
            render_full(&segments)
        } else {
            render_truncated(&segments, chars_to_show)
        }
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}
