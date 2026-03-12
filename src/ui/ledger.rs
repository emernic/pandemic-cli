use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{GameState, LedgerUiState};
use crate::format_number;

/// Render a tiny sparkline from price history using Unicode block chars.
fn sparkline(history: &[f64], width: usize) -> String {
    if history.is_empty() {
        return String::new();
    }
    let bars = [' ', '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}'];
    let min = history.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = history.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = (max - min).max(0.01);

    // Sample last `width` points, or all if fewer
    let start = history.len().saturating_sub(width);
    let slice = &history[start..];
    slice.iter().map(|v| {
        let normalized = ((v - min) / range * 7.0).round() as usize;
        bars[normalized.min(8)]
    }).collect()
}

/// Price change as percentage since previous close.
fn daily_change(history: &[f64], current: f64) -> (f64, Color) {
    let prev = if history.len() >= 2 {
        history[history.len() - 2]
    } else {
        history.first().copied().unwrap_or(current)
    };
    if prev <= 0.0 {
        return (0.0, Color::DarkGray);
    }
    let pct = (current - prev) / prev * 100.0;
    let color = if pct > 0.5 {
        Color::Green
    } else if pct < -0.5 {
        Color::Red
    } else {
        Color::DarkGray
    };
    (pct, color)
}

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line: Option<usize> = None;
    let hdr = Style::default().fg(Color::DarkGray);

    // Portfolio summary header
    let total_value: f64 = state.corporations.iter().enumerate().map(|(i, c)| {
        let held = state.portfolio.get(i).copied().unwrap_or(0);
        c.share_price * held as f64
    }).sum();
    let total_shares: u32 = state.portfolio.iter().sum();

    lines.push(Line::from(vec![
        Span::styled("  Shenzhen Private Ledger", hdr),
    ]));
    if total_shares > 0 {
        lines.push(Line::from(vec![
            Span::styled("  Portfolio: ", hdr),
            Span::styled(
                format!("{} shares", total_shares),
                Style::default().fg(Color::White),
            ),
            Span::styled("  Value: ", hdr),
            Span::styled(
                format!("\u{00a5}{}", format_number(total_value)),
                Style::default().fg(Color::Green),
            ),
        ]));
    }

    // Column headers
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            format!("  {:<22} {:>6} {:>8} {:>7} {:>6} {:>4}  {}",
                "CORP", "SECTOR", "PRICE", "CHG", "HELD", "P/L", "30D"),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
        ),
    ]));

    // Corporation listing
    let mut last_region_idx: Option<usize> = None;
    for (c_idx, corp) in state.corporations.iter().enumerate() {
        let region_idx = corp.region_idx;

        // Region separator
        if last_region_idx != Some(region_idx) {
            if last_region_idx.is_some() {
                lines.push(Line::from(Span::styled(
                    "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
                    Style::default().fg(Color::DarkGray),
                )));
            }
            let region_name = state.regions.get(region_idx)
                .map(|r| r.name.as_str())
                .unwrap_or("???");
            lines.push(Line::from(Span::styled(
                format!("  {}", region_name),
                Style::default().fg(Color::White).add_modifier(Modifier::DIM),
            )));
            last_region_idx = Some(region_idx);
        }

        let selected = state.ui.panel_selection == c_idx;
        if selected {
            selected_line = Some(lines.len());
        }
        let marker = if selected { "\u{25b6} " } else { "  " };

        let (change_pct, change_color) = daily_change(&corp.price_history, corp.share_price);
        let change_str = if change_pct.abs() < 0.01 {
            "  0.0%".to_string()
        } else {
            format!("{:+5.1}%", change_pct)
        };

        let held = state.portfolio.get(c_idx).copied().unwrap_or(0);
        let held_str = if held > 0 { format!("{:>5}", held) } else { "    -".to_string() };

        // P/L for held shares (vs IPO price)
        let pl_str = if held > 0 {
            let pl = (corp.share_price - corp.ipo_price) * held as f64;
            if pl >= 0.0 { format!("+{}", format_number(pl)) } else { format_number(pl) }
        } else {
            "-".to_string()
        };

        let spark = sparkline(&corp.price_history, 12);

        let name_style = if corp.bankrupt {
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT)
        } else if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let price_str = if corp.bankrupt {
            "  BUST".to_string()
        } else {
            format!("{:>6.1}", corp.share_price)
        };

        let price_color = if corp.bankrupt { Color::Red } else { Color::White };

        let board_marker = if corp.board_seat { "\u{2605}" } else { " " };

        // Truncate name to 20 chars
        let display_name = if corp.name.len() > 20 {
            format!("{:.20}", corp.name)
        } else {
            corp.name.clone()
        };

        lines.push(Line::from(vec![
            Span::styled(marker, name_style),
            Span::styled(format!("{:<20}{}", display_name, board_marker), name_style),
            Span::styled(format!(" {:>6}", corp.sector.label()), hdr),
            Span::styled(format!(" {:>8}", price_str), Style::default().fg(price_color)),
            Span::styled(format!(" {:>7}", change_str), Style::default().fg(change_color)),
            Span::styled(format!(" {:>5}", held_str), Style::default().fg(if held > 0 { Color::Cyan } else { Color::DarkGray })),
            Span::styled(format!(" {:>5}", pl_str), Style::default().fg(if held > 0 { change_color } else { Color::DarkGray })),
            Span::raw("  "),
            Span::styled(spark, Style::default().fg(if corp.bankrupt { Color::DarkGray } else { Color::Green })),
        ]));
    }

    // Buy/Sell confirmation overlay
    match &state.ui.ledger_ui {
        Some(LedgerUiState::ConfirmBuy { corp_idx }) => {
            if let Some(corp) = state.corporations.get(*corp_idx) {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("  BUY ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::styled(
                        format!(" 10 shares of {} at \u{00a5}{:.1}/share = \u{00a5}{:.0}",
                            corp.name, corp.share_price, corp.share_price * 10.0),
                        Style::default().fg(Color::White),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  [Enter] Confirm  [X] Switch to Sell  [Esc] Cancel", hdr),
                ]));
            }
        }
        Some(LedgerUiState::ConfirmSell { corp_idx }) => {
            let held = state.portfolio.get(*corp_idx).copied().unwrap_or(0);
            if let Some(corp) = state.corporations.get(*corp_idx) {
                let qty = held.min(10);
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("  SELL ", Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::styled(
                        format!(" {} shares of {} at \u{00a5}{:.1}/share = \u{00a5}{:.0}  (hold: {})",
                            qty, corp.name, corp.share_price, corp.share_price * qty as f64, held),
                        Style::default().fg(Color::White),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  [Enter] Confirm  [X] Switch to Buy  [Esc] Cancel", hdr),
                ]));
            }
        }
        _ => {
            // Hints at bottom
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  [Enter] Buy  [X] Sell  [\u{2605}] Board seat", hdr),
            ]));
        }
    }

    let block = Block::default()
        .title(" S.P.L. Ledger ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let inner_height = area.height.saturating_sub(2);
    let scroll_offset = selected_line.map(|line| {
        if line as u16 >= inner_height {
            (line as u16).saturating_sub(inner_height * 2 / 3)
        } else {
            0
        }
    }).unwrap_or(0);

    let widget = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset, 0));
    f.render_widget(widget, area);
}
