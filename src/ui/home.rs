use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::format_number;
use crate::state::{GameState, ticks_to_days};

// ── Splash (first visit) ──────────────────────────────────────────────

fn build_splash_content(state: &GameState) -> Vec<(String, Style)> {
    let red = Style::default().fg(Color::Red);
    let white = Style::default().fg(Color::White);
    let dim = Style::default().fg(Color::DarkGray);
    let cyan = Style::default().fg(Color::Cyan);
    let yellow = Style::default().fg(Color::Yellow);

    let mut segments: Vec<(String, Style)> = Vec::new();

    let pandemic_full = [
        " ████  █████ █   █ ████  █████ █   █ █  ████ ",
        " █   █ █   █ ██  █ █   █ █     ██ ██ █ █     ",
        " ████  █████ █ █ █ █   █ ████  █ █ █ █ █     ",
        " █     █   █ █  ██ █   █ █     █   █ █ █     ",
        " █     █   █ █   █ ████  █████ █   █ █  ████ ",
    ];
    segments.push(("\n".to_string(), red));
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
        // ~50 chars/tick ≈ one line per tick for a zippy typewriter effect
        let chars_to_show = (state.tick as usize) * 50;
        let total_chars: usize = segments.iter().map(|(s, _)| s.len()).sum();
        if chars_to_show >= total_chars {
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

// ── Sparkline rendering ───────────────────────────────────────────────

/// Braille-based sparkline characters. Each braille char is 2 columns × 4 rows
/// of dots, giving us 4 vertical levels per character position.
/// We use a simpler approach: map values to rows and use block characters.
const SPARK_CHARS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Render a sparkline from data points into a single Line.
/// `width` is how many characters wide the sparkline should be.
/// Data is resampled/binned to fit the width.
fn sparkline(
    data: &[f64],
    width: usize,
    color: Color,
    label: &str,
    current_value: &str,
) -> Vec<Line<'static>> {
    let dim = Style::default().fg(Color::DarkGray);
    let spark_style = Style::default().fg(color);

    if data.is_empty() || width == 0 {
        return vec![Line::from(vec![
            Span::styled(format!("  {:<12}", label), dim),
            Span::styled("  (no data yet)", dim),
        ])];
    }

    // Find the max value for scaling
    let max_val = data.iter().cloned().fold(0.0_f64, f64::max);
    let max_val = if max_val < 1.0 { 1.0 } else { max_val };

    // Resample data to fit width
    let mut chart = String::with_capacity(width);
    for i in 0..width {
        let data_idx = if width > 1 {
            (i * (data.len() - 1)) / (width - 1)
        } else {
            data.len() - 1
        };
        let val = data.get(data_idx).copied().unwrap_or(0.0);
        let normalized = (val / max_val * 8.0).round() as usize;
        chart.push(SPARK_CHARS[normalized.min(8)]);
    }

    vec![Line::from(vec![
        Span::styled(format!("  {:<12}", label), dim),
        Span::styled(chart, spark_style),
        Span::styled(format!(" {}", current_value), Style::default().fg(color)),
    ])]
}

// ── Dashboard ─────────────────────────────────────────────────────────

fn bar(filled: f64, width: usize, fill_color: Color) -> Vec<Span<'static>> {
    let fill_chars = (filled.clamp(0.0, 1.0) * width as f64).round() as usize;
    let empty_chars = width.saturating_sub(fill_chars);
    vec![
        Span::styled("█".repeat(fill_chars), Style::default().fg(fill_color)),
        Span::styled("░".repeat(empty_chars), Style::default().fg(Color::DarkGray)),
    ]
}

fn render_dashboard(f: &mut Frame, area: Rect, state: &GameState) {
    let dim = Style::default().fg(Color::DarkGray);
    let cyan = Style::default().fg(Color::Cyan);
    let yellow = Style::default().fg(Color::Yellow);

    let mut lines: Vec<Line> = Vec::new();

    // ── Global threat meter ──
    // Shows how close the worst-off region is to collapse (0% = safe, 100% = collapse).
    // Once regions start falling, switches to showing collapse count.
    let total_regions = state.regions.len();
    let collapsed_count = state.regions.iter().filter(|r| r.collapsed).count();
    let bar_width = (area.width as usize).saturating_sub(6).min(40);

    // Collapse proximity: fraction of the way to collapse for the worst-off region
    let max_proximity = state.regions.iter()
        .filter(|r| !r.collapsed)
        .map(|r| {
            let pop = r.population as f64;
            let death_frac = if pop > 0.0 { r.total_dead() / pop } else { 0.0 };
            let collapse_death_frac = 1.0 - r.collapse_threshold;
            if collapse_death_frac > 0.0 { (death_frac / collapse_death_frac).min(1.0) } else { 1.0 }
        })
        .fold(0.0_f64, f64::max);

    // Blend: if regions have collapsed, show that; otherwise show proximity
    let (threat_fill, threat_pct, threat_label, threat_col) = if collapsed_count == total_regions {
        (1.0, 100.0, "TOTAL COLLAPSE", Color::LightRed)
    } else if collapsed_count > 0 {
        let frac = collapsed_count as f64 / total_regions as f64;
        let pct = frac * 100.0;
        let label = if collapsed_count == 1 { "REGION FALLEN" } else { "CASCADING" };
        (frac.max(max_proximity), pct, label, Color::Red)
    } else if max_proximity >= 0.75 {
        (max_proximity, max_proximity * 100.0, "CRITICAL", Color::Red)
    } else if max_proximity >= 0.40 {
        (max_proximity, max_proximity * 100.0, "SEVERE", Color::Yellow)
    } else if max_proximity >= 0.10 {
        (max_proximity, max_proximity * 100.0, "ELEVATED", Color::Yellow)
    } else {
        (max_proximity, max_proximity * 100.0, "STABLE", Color::Green)
    };

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  ── GLOBAL THREAT ──", cyan)));
    lines.push(Line::from(""));

    let mut threat_spans = vec![Span::styled("  ", dim)];
    threat_spans.extend(bar(threat_fill, bar_width, threat_col));
    threat_spans.push(Span::styled(format!(" {:.0}%", threat_pct), Style::default().fg(threat_col)));
    lines.push(Line::from(threat_spans));

    let detail = if collapsed_count > 0 {
        format!("  Regions fallen: {} / {}", collapsed_count, total_regions)
    } else {
        let worst_region = state.regions.iter()
            .filter(|r| !r.collapsed)
            .max_by(|a, b| {
                let a_prox = a.total_dead() / a.population as f64;
                let b_prox = b.total_dead() / b.population as f64;
                a_prox.partial_cmp(&b_prox).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|r| r.name.as_str())
            .unwrap_or("Unknown");
        format!("  Most at risk: {}", worst_region)
    };
    lines.push(Line::from(vec![
        Span::styled("  Status: ", dim),
        Span::styled(threat_label, Style::default().fg(threat_col)),
        Span::styled(detail, dim),
    ]));

    // ── Infection & death sparklines ──
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  ── TRENDS ──", cyan)));
    lines.push(Line::from(""));

    let chart_width = (area.width as usize).saturating_sub(22).min(50);
    let history = &state.history;

    let inf_data: Vec<f64> = history.iter().map(|h| h.total_infected).collect();
    let dead_data: Vec<f64> = history.iter().map(|h| h.total_dead).collect();

    lines.extend(sparkline(
        &inf_data,
        chart_width,
        Color::Yellow,
        "Infected",
        &format_number(state.total_infected()),
    ));
    lines.extend(sparkline(
        &dead_data,
        chart_width,
        Color::Red,
        "Deaths",
        &format_number(state.total_dead()),
    ));

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
