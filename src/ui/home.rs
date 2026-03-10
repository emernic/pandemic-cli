use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::format_number;
use crate::state::{GameState, TICKS_PER_DAY, SEVERITY_CRIT_THRESHOLD, SEVERITY_HIGH_THRESHOLD, format_days};

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

    // Find the initial outbreak region
    let outbreak_region = state.regions.iter()
        .find(|r| r.infections.iter().any(|inf| inf.disease_idx == 0 && inf.infected > 0.0))
        .map(|r| r.name.as_str())
        .unwrap_or("an unknown region");

    segments.push(("  ── BRIEFING ──\n".to_string(), cyan));
    segments.push(("\n".to_string(), dim));
    segments.push((format!("  An unidentified pathogen is spreading in {}.\n", outbreak_region), white));
    segments.push(("  Casualties are mounting. No treatment protocol exists.\n".to_string(), dim));
    segments.push(("  The former director of the N.W.H.O. has been removed\n".to_string(), dim));
    segments.push(("  for inaction. You are the replacement.\n".to_string(), dim));
    segments.push(("\n".to_string(), dim));
    segments.push(("  Your mandate: assess and defend humanity against\n".to_string(), white));
    segments.push(("  all biological threats — foreign and domestic.\n".to_string(), white));
    segments.push(("\n".to_string(), dim));
    segments.push(("  Your first priority: send a field research team\n".to_string(), white));
    segments.push(("  to identify what we're dealing with.\n".to_string(), white));
    segments.push(("\n".to_string(), dim));
    segments.push(("  → Press [R] to open Research and begin\n".to_string(), yellow));
    segments.push(("    identification.\n".to_string(), yellow));
    segments.push(("\n".to_string(), dim));
    segments.push(("  [T] Threats   [R] Research   [M] Medicines\n".to_string(), dim));
    segments.push(("  [P] Policy    [?] Help       [Space] Pause\n".to_string(), dim));

    segments
}

fn render_truncated(segments: &[(String, Style)], max_lines: usize) -> Vec<Line<'static>> {
    let cursor_style = Style::default().fg(Color::White);
    let mut lines: Vec<Line> = Vec::new();
    let mut current_spans: Vec<Span> = Vec::new();
    let mut lines_seen = 0;

    for (text, style) in segments {
        for ch in text.chars() {
            if ch == '\n' {
                lines_seen += 1;
                if lines_seen > max_lines {
                    // Add cursor at end of last line and stop
                    current_spans.push(Span::styled("█", cursor_style));
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                    return lines;
                }
                lines.push(Line::from(std::mem::take(&mut current_spans)));
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

fn render_splash(f: &mut Frame, area: Rect, state: &GameState) {
    let segments = build_splash_content(state);

    // One full line per tick for a snappy typewriter effect.
    // usize::MAX means "show everything" (typewriter done).
    let lines_to_show = state.tick as usize;
    let lines = render_truncated(&segments, lines_to_show);

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
            let death_frac = if pop > 0.0 { (r.total_dead() / pop).min(1.0) } else { 0.0 };
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
                let proximity = |r: &crate::state::Region| {
                    let pop = r.population as f64;
                    let death_frac = if pop > 0.0 { (r.total_dead() / pop).min(1.0) } else { 0.0 };
                    let collapse_death_frac = 1.0 - r.collapse_threshold;
                    if collapse_death_frac > 0.0 { death_frac / collapse_death_frac } else { 1.0 }
                };
                proximity(a).partial_cmp(&proximity(b)).unwrap_or(std::cmp::Ordering::Equal)
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

    let inf_data: Vec<f64> = history.iter().map(|h| h.screened_infected).collect();
    let dead_data: Vec<f64> = history.iter().map(|h| h.detected_dead).collect();

    let any_estimated = state.regions.iter().enumerate()
        .any(|(i, _)| state.screening_visibility(i) < 1.0);
    let inf_label = if any_estimated { "Infected~" } else { "Infected" };
    lines.extend(sparkline(
        &inf_data,
        chart_width,
        Color::Yellow,
        inf_label,
        &format_number(state.total_infected_screened()),
    ));
    lines.extend(sparkline(
        &dead_data,
        chart_width,
        Color::Red,
        "Deaths",
        &format_number(state.total_dead_detected()),
    ));

    // ── Budget breakdown ──
    {
        let gross = state.funding_income_rate() * TICKS_PER_DAY;
        let upkeep = state.personnel_upkeep_rate() * TICKS_PER_DAY;
        let policy = state.total_policy_funding_cost() * TICKS_PER_DAY;
        let net = gross - upkeep - policy;

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  ── BUDGET ──", cyan)));
        lines.push(Line::from(""));

        let green = Style::default().fg(Color::Green);
        let red_style = Style::default().fg(Color::Red);

        // Income line — show travel ban penalty separately when active
        let alive_regions = state.regions.iter().filter(|r| !r.collapsed).count();
        let ban_penalty = state.travel_ban_income_penalty() * TICKS_PER_DAY;
        if ban_penalty > 0.0 {
            // Show the pre-penalty income so the player can see the true cost
            let base_income = gross + ban_penalty;
            lines.push(Line::from(vec![
                Span::styled("  Income:   ", dim),
                Span::styled(format!("+${:.0}/day", base_income), green),
                Span::styled(format!("  ({} regions)", alive_regions), dim),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  Ban cost: ", dim),
                Span::styled(format!("-${:.0}/day", ban_penalty), red_style),
                Span::styled("  (halved income)", dim),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled("  Income:   ", dim),
                Span::styled(format!("+${:.0}/day", gross), green),
                Span::styled(format!("  ({} regions)", alive_regions), dim),
            ]));
        }

        // Upkeep line
        if upkeep > 0.0 {
            lines.push(Line::from(vec![
                Span::styled("  Upkeep:   ", dim),
                Span::styled(format!("-${:.0}/day", upkeep), red_style),
                Span::styled(format!("  ({} personnel)", state.resources.personnel), dim),
            ]));
        }

        // Policy line
        if policy > 0.0 {
            let active_count: usize = state.policies.iter()
                .filter(|p| p.any_active())
                .count();
            lines.push(Line::from(vec![
                Span::styled("  Policies: ", dim),
                Span::styled(format!("-${:.0}/day", policy), red_style),
                Span::styled(format!("  (in {} region{})", active_count, if active_count == 1 { "" } else { "s" }), dim),
            ]));
        }

        // Net line
        let (net_str, net_color) = if net >= 0.0 {
            (format!("+${:.0}/day", net), Color::Green)
        } else {
            (format!("-${:.0}/day", net.abs()), Color::Red)
        };
        lines.push(Line::from(vec![
            Span::styled("  ────────────────────", dim),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Net:      ", dim),
            Span::styled(net_str, Style::default().fg(net_color)),
        ]));
    }

    // ── Active diseases ──
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  ── ACTIVE THREATS ──", cyan)));
    lines.push(Line::from(""));

    for (i, disease) in state.diseases.iter().enumerate() {
        // Skip undetected diseases — player shouldn't see them
        if !disease.detected {
            continue;
        }

        let name = disease.display_name(i);
        let total_inf: f64 = state.regions.iter().enumerate()
            .map(|(ri, r)| {
                let vis = state.screening_visibility(ri);
                r.infections.iter()
                    .filter(|inf| inf.disease_idx == i)
                    .map(|inf| inf.infected * vis)
                    .sum::<f64>()
            })
            .sum();

        let severity_color = if total_inf > SEVERITY_CRIT_THRESHOLD { Color::Red }
            else if total_inf > SEVERITY_HIGH_THRESHOLD { Color::Yellow }
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
    if !state.field_research.is_empty() || state.applied_research.is_some() || state.basic_research.is_some() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  ── RESEARCH ──", cyan)));
        lines.push(Line::from(""));

        let research_bar_w = (area.width as usize).saturating_sub(30).min(20);

        // Field research: show all active projects
        for (i, proj) in state.field_research.iter().enumerate() {
            let label = if state.field_research.len() > 1 {
                format!("Field {}", i + 1)
            } else {
                "Field".to_string()
            };
            let pct = if proj.required_ticks > 0.0 {
                proj.progress / proj.required_ticks
            } else { 1.0 };
            let remaining = proj.required_ticks - proj.progress;
            let speed = proj.speed(&state.medicines);
            let effective_remaining = if speed > 0.0 { remaining / speed } else { remaining };

            let mut spans = vec![
                Span::styled(format!("  {}: ", label), dim),
            ];
            spans.extend(bar(pct, research_bar_w, Color::Green));
            spans.push(Span::styled(
                format!(" {:.0}% ({} left)", pct * 100.0, format_days(effective_remaining)),
                yellow,
            ));
            lines.push(Line::from(spans));
        }

        // Applied and Basic: single-slot
        for (label, project) in [
            ("Applied", &state.applied_research),
            ("Basic", &state.basic_research),
        ] {
            if let Some(proj) = project {
                let pct = if proj.required_ticks > 0.0 {
                    proj.progress / proj.required_ticks
                } else { 1.0 };
                let remaining = proj.required_ticks - proj.progress;
                let speed = proj.speed(&state.medicines);
                let effective_remaining = if speed > 0.0 { remaining / speed } else { remaining };

                let mut spans = vec![
                    Span::styled(format!("  {}: ", label), dim),
                ];
                spans.extend(bar(pct, research_bar_w, Color::Green));
                spans.push(Span::styled(
                    format!(" {:.0}% ({} left)", pct * 100.0, format_days(effective_remaining)),
                    yellow,
                ));
                lines.push(Line::from(spans));
            }
        }
    }

    // ── Event log ──
    if !state.event_log.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  ── RECENT EVENTS ──", cyan)));
        lines.push(Line::from(""));

        // Show as many recent events as will fit — estimate available space
        let used_lines = lines.len();
        let available = (area.height as usize).saturating_sub(used_lines + 2); // 2 for border
        let show_count = available.min(state.event_log.len()).min(15);

        let skip = state.event_log.len().saturating_sub(show_count);
        for (day, msg) in state.event_log.iter().skip(skip) {
            lines.push(Line::from(vec![
                Span::styled(format!("  Day {:<5.1} ", day), dim),
                Span::styled(msg.clone(), Style::default().fg(Color::White)),
            ]));
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
