use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::format_number;
use crate::state::{GameState, LOSE_DEATH_FRACTION, ticks_to_days};

// ── Splash (first visit) ──────────────────────────────────────────────

fn build_splash_content(state: &GameState) -> Vec<(String, Style)> {
    let red = Style::default().fg(Color::Red);
    let white = Style::default().fg(Color::White);
    let dim = Style::default().fg(Color::DarkGray);
    let cyan = Style::default().fg(Color::Cyan);
    let yellow = Style::default().fg(Color::Yellow);

    let mut segments: Vec<(String, Style)> = Vec::new();

    // Block-letter "PANDEMIC" — fits in ~66 chars wide.
    // If terminal is very narrow, we split as "PAN" / "DEMIC" per user request.
    let pandemic_full = [
        " ████  █████ █   █ ████  █████ █   █ █  ████ ",
        " █   █ █   █ ██  █ █   █ █     ██ ██ █ █     ",
        " ████  █████ █ █ █ █   █ ████  █ █ █ █ █     ",
        " █     █   █ █  ██ █   █ █     █   █ █ █     ",
        " █     █   █ █   █ ████  █████ █   █ █  ████ ",
    ];
    segments.push(("\n".to_string(), red));
    // Full "PANDEMIC" is 46 chars — fits in the ~96-char panel
    for line in &pandemic_full {
        segments.push((format!("{}\n", line), red));
    }

    segments.push(("\n".to_string(), red));
    segments.push(("              C . L . I .\n".to_string(), white));
    segments.push(("\n".to_string(), dim));

    segments.push(("  ── Getting Started ──\n".to_string(), cyan));
    segments.push(("\n".to_string(), dim));

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

    if !done && !current_spans.is_empty() {
        lines.push(Line::from(current_spans));
    }

    lines
}

fn render_splash(f: &mut Frame, area: Rect, state: &GameState) {
    let segments = build_splash_content(state);

    let lines = {
        let chars_to_show = (state.tick as usize) * 2;
        let total_chars: usize = segments.iter().map(|(s, _)| s.len()).sum();
        if chars_to_show >= total_chars {
            // Animation complete — render full but keep splash style
            let mut full_lines: Vec<Line> = Vec::new();
            let mut current_spans: Vec<Span> = Vec::new();
            for (text, style) in &segments {
                for ch in text.chars() {
                    if ch == '\n' {
                        full_lines.push(Line::from(current_spans));
                        current_spans = Vec::new();
                    } else {
                        current_spans.push(Span::styled(ch.to_string(), *style));
                    }
                }
            }
            if !current_spans.is_empty() {
                full_lines.push(Line::from(current_spans));
            }
            full_lines
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

// ── Dashboard (subsequent visits) ─────────────────────────────────────

/// Build a horizontal bar using block characters.
/// `filled` is 0.0–1.0, `width` is the total bar width in chars.
fn bar(filled: f64, width: usize, fill_color: Color) -> Vec<Span<'static>> {
    let fill_chars = (filled.clamp(0.0, 1.0) * width as f64).round() as usize;
    let empty_chars = width.saturating_sub(fill_chars);
    vec![
        Span::styled("█".repeat(fill_chars), Style::default().fg(fill_color)),
        Span::styled("░".repeat(empty_chars), Style::default().fg(Color::DarkGray)),
    ]
}

fn threat_color(fraction: f64) -> Color {
    if fraction >= 0.07 { Color::Red }
    else if fraction >= 0.03 { Color::Yellow }
    else if fraction >= 0.01 { Color::Cyan }
    else { Color::Green }
}

fn render_dashboard(f: &mut Frame, area: Rect, state: &GameState) {
    let dim = Style::default().fg(Color::DarkGray);
    let white = Style::default().fg(Color::White);
    let cyan = Style::default().fg(Color::Cyan);
    let yellow = Style::default().fg(Color::Yellow);

    let mut lines: Vec<Line> = Vec::new();
    let initial_pop = state.initial_population();

    // ── Global threat meter ──
    let death_frac = if initial_pop > 0.0 { state.total_dead() / initial_pop } else { 0.0 };
    let threat_pct = (death_frac / LOSE_DEATH_FRACTION * 100.0).min(100.0);
    let threat_col = threat_color(death_frac);
    let bar_width = (area.width as usize).saturating_sub(6).min(40);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  ── GLOBAL THREAT ──", cyan)));
    lines.push(Line::from(""));

    let mut threat_spans = vec![Span::styled("  ", dim)];
    threat_spans.extend(bar(death_frac / LOSE_DEATH_FRACTION, bar_width, threat_col));
    threat_spans.push(Span::styled(format!(" {:.0}%", threat_pct), Style::default().fg(threat_col)));
    lines.push(Line::from(threat_spans));

    let threat_label = if death_frac < 0.01 { "CONTAINED" }
        else if death_frac < 0.03 { "MODERATE" }
        else if death_frac < 0.07 { "SEVERE" }
        else { "CRITICAL" };
    lines.push(Line::from(vec![
        Span::styled("  Status: ", dim),
        Span::styled(threat_label, Style::default().fg(threat_col)),
        Span::styled(format!("  Deaths: {} / {}", format_number(state.total_dead()), format_number(initial_pop * LOSE_DEATH_FRACTION)), dim),
    ]));

    // ── Region overview ──
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  ── REGIONS ──", cyan)));
    lines.push(Line::from(""));

    let region_bar_width = (area.width as usize).saturating_sub(22).min(20);
    for (i, region) in state.regions.iter().enumerate() {
        let pop = region.population as f64;
        let inf = region.total_infected();
        let dead = region.total_dead();
        let immune = region.total_immune();
        let healthy = (pop - inf - dead - immune).max(0.0);

        // Stacked mini-bar: green=healthy, cyan=immune, yellow=infected, red=dead
        let h_frac = healthy / pop;
        let im_frac = immune / pop;
        let inf_frac = inf / pop;
        let d_frac = dead / pop;

        let h_chars = (h_frac * region_bar_width as f64).round() as usize;
        let im_chars = (im_frac * region_bar_width as f64).round() as usize;
        let inf_chars = (inf_frac * region_bar_width as f64).round().max(if inf > 0.0 { 1.0 } else { 0.0 }) as usize;
        let d_chars = (d_frac * region_bar_width as f64).round().max(if dead > 0.0 { 1.0 } else { 0.0 }) as usize;
        // Remaining goes to healthy
        let total = h_chars + im_chars + inf_chars + d_chars;
        let h_chars = if total > region_bar_width { h_chars.saturating_sub(total - region_bar_width) }
            else { h_chars + (region_bar_width - total) };

        let selected = state.ui.map_selection == i;
        let name_style = if selected { white } else { dim };
        let name = format!("{:<14}", region.name);

        let mut spans = vec![
            Span::styled(if selected { "▶ " } else { "  " }, if selected { white } else { dim }),
            Span::styled(name, name_style),
        ];
        spans.push(Span::styled("█".repeat(h_chars), Style::default().fg(Color::Green)));
        spans.push(Span::styled("█".repeat(im_chars), Style::default().fg(Color::Cyan)));
        spans.push(Span::styled("█".repeat(inf_chars), Style::default().fg(Color::Yellow)));
        spans.push(Span::styled("█".repeat(d_chars), Style::default().fg(Color::Red)));

        // Compact stats
        if inf > 0.0 {
            spans.push(Span::styled(format!(" {}", format_number(inf)), Style::default().fg(Color::Yellow)));
        }

        lines.push(Line::from(spans));
    }

    // ── Active diseases ──
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  ── ACTIVE THREATS ──", cyan)));
    lines.push(Line::from(""));

    for (i, disease) in state.diseases.iter().enumerate() {
        let name = disease.display_name(i);
        let total_inf: f64 = state.regions.iter()
            .flat_map(|r| r.infections.iter())
            .filter(|inf| inf.disease_idx == i)
            .map(|inf| inf.infected)
            .sum();

        let severity_color = if total_inf > 100_000.0 { Color::Red }
            else if total_inf > 10_000.0 { Color::Yellow }
            else if total_inf > 0.0 { Color::Cyan }
            else { Color::DarkGray };

        let knowledge_bar_w: usize = 8;
        let knowledge_filled = (disease.knowledge * knowledge_bar_w as f64).round() as usize;

        let mut spans = vec![
            Span::styled("  ", dim),
            Span::styled("● ", Style::default().fg(severity_color)),
            Span::styled(format!("{:<24}", name), Style::default().fg(severity_color)),
        ];

        // Knowledge bar
        spans.push(Span::styled("K:", dim));
        spans.push(Span::styled("█".repeat(knowledge_filled), Style::default().fg(Color::Cyan)));
        spans.push(Span::styled("░".repeat(knowledge_bar_w.saturating_sub(knowledge_filled)), dim));

        if total_inf > 0.0 {
            spans.push(Span::styled(format!("  Inf:{}", format_number(total_inf)), Style::default().fg(severity_color)));
        } else {
            spans.push(Span::styled("  Eradicated", Style::default().fg(Color::Green)));
        }

        lines.push(Line::from(spans));
    }

    // ── Research status ──
    if state.field_research.is_some() || state.bench_research.is_some() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  ── RESEARCH ──", cyan)));
        lines.push(Line::from(""));

        let research_bar_w = (area.width as usize).saturating_sub(30).min(20);
        for (label, project) in [
            ("Field", &state.field_research),
            ("Bench", &state.bench_research),
        ] {
            if let Some(proj) = project {
                let pct = if proj.required_ticks > 0.0 {
                    proj.progress / proj.required_ticks
                } else { 1.0 };
                let remaining = proj.required_ticks - proj.progress;
                let remaining_days = ticks_to_days(remaining);

                let mut spans = vec![
                    Span::styled(format!("  {}: ", label), dim),
                ];
                spans.extend(bar(pct, research_bar_w, Color::Green));
                spans.push(Span::styled(
                    format!(" {:.0}% ({:.1}d left)", pct * 100.0, remaining_days),
                    yellow,
                ));
                lines.push(Line::from(spans));
            }
        }
    }

    // ── Footer ──
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [T]hreats [R]esearch [M]eds [P]olicy",
        dim,
    )));

    let block = Block::default()
        .title(" DASHBOARD ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

// ── Public entry point ────────────────────────────────────────────────

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    if state.ui.home_splash_done {
        render_dashboard(f, area, state);
    } else {
        render_splash(f, area, state);
    }
}
