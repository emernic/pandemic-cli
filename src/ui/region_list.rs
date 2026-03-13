use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::state::{map_grid_pos, GameState, Region, MAP_GRID_LEN,
    COINFECTION_LETHALITY_PER_DISEASE, COINFECTION_THRESHOLD,
    SEVERITY_CRIT_THRESHOLD, SEVERITY_HIGH_THRESHOLD, SEVERITY_MOD_THRESHOLD};

use crate::format_number;
use super::sparkline;

#[derive(Clone, Copy)]
enum ConnKind {
    Horizontal,
    Vertical,
    Diagonal,
}

struct MapConnection {
    a: usize,
    b: usize,
    kind: ConnKind,
}

/// Classify a connection between two regions by grid adjacency.
/// Pair must be (smaller_idx, larger_idx). Returns None if not drawable.
fn classify_connection(a: usize, b: usize) -> Option<ConnKind> {
    let (ca, ra) = map_grid_pos(a)?;
    let (cb, rb) = map_grid_pos(b)?;
    if ra == rb && cb == ca + 1 {
        Some(ConnKind::Horizontal)
    } else if ca == cb && rb == ra + 1 {
        Some(ConnKind::Vertical)
    } else if cb == ca + 1 && ra == rb + 1 {
        Some(ConnKind::Diagonal)
    } else {
        None
    }
}

/// Build drawable connections from region topology, classifying each by kind.
fn drawable_connections(state: &GameState) -> Vec<MapConnection> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for (i, region) in state.regions.iter().enumerate() {
        for &j in &region.connections {
            let pair = if i < j { (i, j) } else { (j, i) };
            if !seen.insert(pair) {
                continue;
            }
            if let Some(kind) = classify_connection(pair.0, pair.1) {
                result.push(MapConnection { a: pair.0, b: pair.1, kind });
            }
        }
    }
    result
}

/// Find connections for a region that can't be drawn on the grid.
fn non_drawable_connections(state: &GameState, region_idx: usize) -> Vec<usize> {
    state.regions[region_idx]
        .connections
        .iter()
        .filter(|&&j| {
            let (a, b) = if region_idx < j { (region_idx, j) } else { (j, region_idx) };
            classify_connection(a, b).is_none()
        })
        .copied()
        .collect()
}

/// Human-readable label for a region's specialization bonus.
fn specialization_label(region: &crate::state::Region) -> &'static str {
    match region.specialization {
        Some(spec) => spec.label(),
        None => "None",
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
    let block = Block::default()
        .title(" World Map ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 20 || inner.height < 6 || state.regions.len() < MAP_GRID_LEN {
        return;
    }

    let gap_col: u16 = 3;
    let gap_row: u16 = 1;
    let region_width = ((inner.width.saturating_sub(2 * gap_col)) / 3).min(30);
    // Condensed boxes: 4 content lines + 2 border = 6 (name, stats, bar, collapse indicator)
    let region_height = ((inner.height.saturating_sub(gap_row)) / 2).min(6);

    // Draw connections in gap areas
    let connections = drawable_connections(state);
    {
        let buf = f.buffer_mut();
        let buf_area = buf.area;
        for conn in &connections {
            let (ca, ra) = map_grid_pos(conn.a).unwrap();
            let (_cb, rb) = map_grid_pos(conn.b).unwrap();

            let has_spread = state.regions[conn.a].screened_infected() > 0.0
                || state.regions[conn.b].screened_infected() > 0.0;
            let color = if has_spread {
                Color::Red
            } else {
                Color::DarkGray
            };
            let style = Style::default().fg(color);

            match conn.kind {
                ConnKind::Horizontal => {
                    let x_start = inner.x + ca * (region_width + gap_col) + region_width;
                    let y = inner.y + ra * (region_height + gap_row) + region_height / 2;
                    for x in x_start..x_start + gap_col {
                        if x < buf_area.x + buf_area.width && y < buf_area.y + buf_area.height {
                            let cell = &mut buf[(x, y)];
                            cell.set_symbol("─");
                            cell.set_style(style);
                        }
                    }
                }
                ConnKind::Vertical => {
                    let x = inner.x + ca * (region_width + gap_col) + region_width / 2;
                    let y_start = inner.y + ra * (region_height + gap_row) + region_height;
                    for y in y_start..y_start + gap_row {
                        if x < buf_area.x + buf_area.width && y < buf_area.y + buf_area.height {
                            let cell = &mut buf[(x, y)];
                            cell.set_symbol("│");
                            cell.set_style(style);
                        }
                    }
                }
                ConnKind::Diagonal => {
                    let x = inner.x + ca * (region_width + gap_col) + region_width + gap_col / 2;
                    let y = inner.y + rb * (region_height + gap_row) + region_height;
                    if x < buf_area.x + buf_area.width && y < buf_area.y + buf_area.height {
                        let cell = &mut buf[(x, y)];
                        cell.set_symbol("╱");
                        cell.set_style(style);
                    }
                }
            }
        }
    }

    // Render each region box (condensed: name, stats, health bar only)
    for (idx, region) in state.regions.iter().enumerate() {
        let (col, row) = match map_grid_pos(idx) {
            Some(p) => p,
            None => break,
        };
        let x = inner.x + col * (region_width + gap_col);
        let y = inner.y + row * (region_height + gap_row);
        let rect = Rect::new(x, y, region_width, region_height);
        let selected = idx == state.ui.map_selection;
        let visibility = state.screening_visibility(idx);
        let shows_immune = state.screening_shows_immune(idx);
        let is_ark = state.ark_protocol == Some(idx);
        let is_abandoned = state.is_abandoned(idx);
        let board_count = state.board_members.iter().filter(|bm| bm.region_idx == Some(idx)).count();
        let martial_law = state.policies.get(idx).is_some_and(|p| p.martial_law);
        render_region_box(f, rect, region, selected, &state.diseases, visibility, shows_immune, is_ark, is_abandoned, board_count, martial_law);
    }

    // Detail panel below the grid for the selected region
    let grid_bottom = inner.y + 2 * region_height + gap_row;
    let detail_height = (inner.y + inner.height).saturating_sub(grid_bottom);
    if detail_height >= 2 {
        let detail_area = Rect::new(inner.x, grid_bottom, inner.width, detail_height);
        render_detail_panel(f, detail_area, state);
    }
}

fn render_region_box(
    f: &mut Frame,
    area: Rect,
    region: &Region,
    selected: bool,
    diseases: &[crate::state::Disease],
    visibility: f64,
    shows_immune: bool,
    is_ark: bool,
    is_abandoned: bool,
    board_count: usize,
    martial_law: bool,
) {
    let border_color = if is_ark {
        Color::Cyan
    } else if selected {
        Color::Yellow
    } else {
        Color::DarkGray
    };
    let border_mod = if selected {
        Modifier::BOLD
    } else {
        Modifier::empty()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if selected { BorderType::Double } else { BorderType::Plain })
        .border_style(Style::default().fg(border_color).add_modifier(border_mod));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 2 || inner.height < 1 {
        return;
    }

    let infected = region.screened_infected();
    let immune = if shows_immune { region.detected_immune(diseases) } else { 0.0 };
    let dead = region.detected_dead(diseases);
    let pop = region.population as f64;

    let threat = if is_ark {
        ("HQ", Color::Cyan)
    } else if is_abandoned {
        ("GONE", Color::DarkGray)
    } else if region.collapsed {
        ("FELL", Color::Red)
    } else if infected > SEVERITY_CRIT_THRESHOLD {
        ("CRIT", Color::Red)
    } else if infected > SEVERITY_HIGH_THRESHOLD {
        ("HIGH", Color::LightRed)
    } else if infected > SEVERITY_MOD_THRESHOLD {
        ("MOD", Color::Yellow)
    } else if infected > 0.0 {
        ("LOW", Color::Green)
    } else {
        ("OK", Color::DarkGray)
    };

    // Information blackout: collapsed (non-HQ) or abandoned (GONE) regions
    let info_blackout = is_abandoned || (region.collapsed && !is_ark);

    let name_style = if info_blackout {
        // Greyed out — collapsed regions are information blackouts
        Style::default().fg(Color::DarkGray)
    } else if selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let iw = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    // Line 1: Name + board stars + threat level
    let name = &region.name;
    let stars = "★".repeat(board_count);
    let threat_len = threat.0.len();
    let stars_with_space = if board_count > 0 { board_count + 1 } else { 0 }; // space + ★s
    let max_name = iw.saturating_sub(threat_len + stars_with_space + 1);
    let display_name: &str = if name.len() > max_name {
        &name[..max_name]
    } else {
        name
    };
    let used = display_name.len() + stars_with_space + threat_len;
    let padding = iw.saturating_sub(used);
    let mut name_spans = vec![
        Span::styled(display_name.to_string(), name_style),
    ];
    if board_count > 0 {
        name_spans.push(Span::styled(format!(" {}", stars), Style::default().fg(Color::Yellow)));
    }
    name_spans.push(Span::raw(" ".repeat(padding)));
    name_spans.push(Span::styled(
        threat.0,
        Style::default()
            .fg(threat.1)
            .add_modifier(Modifier::BOLD),
    ));
    lines.push(Line::from(name_spans));

    // Line 2: Key stats
    if inner.height >= 2 {
        if info_blackout {
            // Information blackout: infected count unknown, only dead is visible
            let mut stats = vec![
                Span::styled("Inf ", Style::default().fg(Color::DarkGray)),
                Span::styled("?", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
            ];
            if dead >= 0.5 {
                stats.push(Span::styled("  Dead ", Style::default().fg(Color::DarkGray)));
                stats.push(Span::styled(
                    format_number(dead),
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
                ));
            }
            lines.push(Line::from(stats));
        } else if infected == 0.0 && dead == 0.0 {
            lines.push(Line::from(Span::styled(
                format!("Pop: {}", format_number(pop)),
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            let mut stats = Vec::new();
            let inf_label = if visibility < 1.0 { "Inf~ " } else { "Inf " };
            stats.push(Span::styled(inf_label, Style::default().fg(Color::Red)));
            stats.push(Span::styled(
                format_number(infected),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
            if dead >= 0.5 {
                stats.push(Span::styled("  Dead ", Style::default().fg(Color::DarkGray)));
                stats.push(Span::styled(
                    format_number(dead),
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
                ));
            }
            lines.push(Line::from(stats));
        }
    }

    // Line 3: Health bar (rendered intact, no markers embedded)
    if inner.height >= 3 && pop > 0.0 {
        let bar_w = iw;
        let mut inf_w = if infected > 0.0 {
            ((infected / pop) * bar_w as f64).round().max(1.0) as usize
        } else {
            0
        };
        let mut imm_w = if immune > 0.0 {
            ((immune / pop) * bar_w as f64).round().max(1.0) as usize
        } else {
            0
        };
        let mut dead_w = if dead > 0.0 {
            ((dead / pop) * bar_w as f64).round().max(1.0) as usize
        } else {
            0
        };
        let used = inf_w + imm_w + dead_w;
        if used > bar_w {
            let excess = used - bar_w;
            for _ in 0..excess {
                if dead_w > 1 {
                    dead_w -= 1;
                } else if imm_w > 1 {
                    imm_w -= 1;
                } else if inf_w > 1 {
                    inf_w -= 1;
                }
            }
        }
        let sus_w = bar_w.saturating_sub(inf_w + imm_w + dead_w);

        // Collapsed regions (not consolidated HQ): all grey — information blackout, keep texture only
        let (sus_color, inf_color, imm_color) = if info_blackout {
            (Color::DarkGray, Color::DarkGray, Color::DarkGray)
        } else {
            (Color::Cyan, Color::Red, Color::Green)
        };
        let segments: [(usize, Color, &str); 4] = [
            (sus_w, sus_color, "█"),
            (inf_w, inf_color, "▓"),
            (imm_w, imm_color, "▒"),
            (dead_w, Color::DarkGray, "░"),
        ];

        let mut spans = Vec::new();
        for (width, color, ch) in segments {
            if width > 0 {
                spans.push(Span::styled(ch.repeat(width), Style::default().fg(color)));
            }
        }
        // Fill remaining with healthy (if rounding left gaps)
        let total: usize = segments.iter().map(|(w, _, _)| w).sum();
        if total < bar_w {
            let remaining = bar_w - total;
            spans.push(Span::styled("█".repeat(remaining), Style::default().fg(sus_color)));
        }
        lines.push(Line::from(spans));

        // Line 4: Collapse threshold indicator below the bar
        if inner.height >= 4 && !region.collapsed {
            let death_fraction_at_collapse = 1.0 - region.effective_collapse_threshold(martial_law);
            let collapse_pos = bar_w.saturating_sub(
                (death_fraction_at_collapse * bar_w as f64).round() as usize
            );
            if collapse_pos > 0 && collapse_pos < bar_w {
                let mut indicator_spans = Vec::new();
                if collapse_pos > 0 {
                    indicator_spans.push(Span::raw(" ".repeat(collapse_pos)));
                }
                indicator_spans.push(Span::styled("▲", Style::default().fg(Color::Red)));
                lines.push(Line::from(indicator_spans));
            }
        }
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

/// Detail panel below the region grid showing full info for the selected region.
fn render_detail_panel(f: &mut Frame, area: Rect, state: &GameState) {
    let idx = state.ui.map_selection;
    let region = match state.regions.get(idx) {
        Some(r) => r,
        None => return,
    };

    let border_color = Color::Yellow;
    let block = Block::default()
        .title(format!(" {} ", region.name))
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(border_color).add_modifier(Modifier::BOLD));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 10 || inner.height < 1 {
        return;
    }

    let pop = region.population as f64;
    let visibility = state.screening_visibility(idx);
    let infected = region.screened_infected();
    let shows_immune = state.screening_shows_immune(idx);
    let shows_exposed = state.screening_shows_exposed(idx);
    let immune = if shows_immune { region.detected_immune(&state.diseases) } else { 0.0 };
    let dead = region.detected_dead(&state.diseases);
    let alive = pop - dead; // alive based on detected deaths only

    let label = Style::default().fg(Color::DarkGray);
    let val = Style::default().fg(Color::White);

    let mut lines: Vec<Line> = Vec::new();

    let is_ark = state.ark_protocol == Some(idx);
    let is_abandoned = state.is_abandoned(idx);

    // Abandoned region (Ark Protocol active, not HQ, not collapsed): minimal info
    if is_abandoned {
        lines.push(Line::from(Span::styled(
            "  ██ ABANDONED ██",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled("Pop ", label),
            Span::styled(format_number(pop), Style::default().fg(Color::DarkGray)),
            Span::styled("  Dead ", label),
            Span::styled(format_number(dead), Style::default().fg(Color::DarkGray)),
        ]));
        let spec_label = specialization_label(region);
        lines.push(Line::from(vec![
            Span::styled("Specialization: ", label),
            Span::styled(
                format!("{} (ABANDONED)", spec_label),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        let paragraph = Paragraph::new(lines);
        f.render_widget(paragraph, inner);
        return;
    }

    // Collapse banner (always shown for collapsed regions)
    if region.collapsed {
        lines.push(Line::from(Span::styled(
            "  ██ COLLAPSED ██",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
        if !is_ark {
            // Information blackout: only alive vs dead — all epidemiological data is unknown
            lines.push(Line::from(vec![
                Span::styled("Pop ", label),
                Span::styled(format_number(pop), val),
                Span::styled("  Alive ", label),
                Span::styled(format_number(alive), Style::default().fg(Color::DarkGray)),
                Span::styled("  Dead ", label),
                Span::styled(format_number(dead), Style::default().fg(Color::DarkGray)),
            ]));
            // Show lost specialization even during information blackout
            let spec_label = specialization_label(region);
            lines.push(Line::from(vec![
                Span::styled("Specialization: ", label),
                Span::styled(
                    format!("{} (LOST)", spec_label),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            let paragraph = Paragraph::new(lines);
            f.render_widget(paragraph, inner);
            return;
        }
    }

    // Population summary line
    let mut summary_spans = vec![
        Span::styled("Pop ", label),
        Span::styled(format_number(pop), val),
        Span::styled("  Alive ", label),
        Span::styled(format_number(alive), Style::default().fg(Color::Green)),
        Span::styled(
            if visibility < 0.99 {
                format!("  Infected~{:.0}% ", visibility * 100.0)
            } else {
                "  Infected ".to_string()
            },
            label,
        ),
        Span::styled(format_number(infected), Style::default().fg(Color::Red)),
    ];
    if shows_immune {
        summary_spans.push(Span::styled("  Immune ", label));
        summary_spans.push(Span::styled(
            format_number(immune),
            Style::default().fg(Color::Cyan),
        ));
    }
    summary_spans.push(Span::styled("  Dead ", label));
    summary_spans.push(Span::styled(format_number(dead), Style::default().fg(if dead > 0.0 { Color::Red } else { Color::DarkGray })));
    lines.push(Line::from(summary_spans));

    // Collapse threshold line
    if !region.collapsed {
        let death_pct = if pop > 0.0 { (dead / pop * 100.0).abs() } else { 0.0 };
        let martial_law = state.policies.get(idx).is_some_and(|p| p.martial_law);
        let collapse_death_pct = (1.0 - region.effective_collapse_threshold(martial_law)) * 100.0;
        let proximity = if collapse_death_pct > 0.0 { death_pct / collapse_death_pct } else { 1.0 };
        let threshold_color = if proximity >= 0.75 {
            Color::Red
        } else if proximity >= 0.40 {
            Color::Yellow
        } else {
            Color::DarkGray
        };
        let mut collapse_spans = vec![
            Span::styled("Collapse at ", label),
            Span::styled(
                format!("{:.0}% deaths", collapse_death_pct),
                Style::default().fg(threshold_color),
            ),
            Span::styled("  (currently ", label),
            Span::styled(
                format!("{:.1}%", death_pct),
                Style::default().fg(threshold_color),
            ),
            Span::styled(")", label),
        ];
        // Show estimated time to collapse when the region is meaningfully threatened
        if proximity >= 0.05 {
            let martial_law = state.policies.get(idx).is_some_and(|p| p.martial_law);
            if let Some(days) = region.days_to_collapse(martial_law) {
                let eta_color = if days < 5.0 {
                    Color::Red
                } else if days < 15.0 {
                    Color::Yellow
                } else {
                    Color::DarkGray
                };
                let label = if days < 1.0 {
                    format!("  ~{:.0}h left", days * 24.0)
                } else {
                    format!("  ~{:.1} days left", days)
                };
                collapse_spans.push(Span::styled(label, Style::default().fg(eta_color)));
            }
        }
        lines.push(Line::from(collapse_spans));
    }

    // Region traits (income and healthcare modifiers)
    {
        let gdp_frac = region.gdp_fraction();
        let gdp_status = if region.collapsed {
            ("COLLAPSED", Color::Red)
        } else if gdp_frac < 0.40 {
            ("Depression", Color::Red)
        } else if gdp_frac < 0.60 {
            ("Recession", Color::LightRed)
        } else if gdp_frac < 0.80 {
            ("Strained", Color::Yellow)
        } else {
            ("Stable", Color::Green)
        };
        let healthcare_label = if region.healthcare_modifier <= 0.80 {
            ("Excellent", Color::Green)
        } else if region.healthcare_modifier <= 0.95 {
            ("Good", Color::Cyan)
        } else if region.healthcare_modifier <= 1.0 {
            ("Average", Color::Yellow)
        } else {
            ("Strained", Color::Red)
        };
        let mut econ_spans = vec![
            Span::styled("GDP: ", label),
            Span::styled(
                format!("{:.0}k", region.gdp),
                Style::default().fg(gdp_status.1),
            ),
            Span::styled(
                format!(" ({})", gdp_status.0),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("  Healthcare: ", label),
            Span::styled(
                healthcare_label.0,
                Style::default().fg(healthcare_label.1),
            ),
        ];
        if region.hospital_level > 0 {
            let hospital_name = if region.hospital_level >= 2 { "Med Center" } else { "Field Hospital" };
            econ_spans.push(Span::styled(
                format!("  [{}]", hospital_name),
                Style::default().fg(Color::Green),
            ));
        }
        if region.intel_level > 0 {
            let intel_name = if region.intel_level >= 2 { "Adv Intel" } else { "Intel Stn" };
            econ_spans.push(Span::styled(
                format!("  [{}]", intel_name),
                Style::default().fg(Color::Cyan),
            ));
        }
        if region.is_disrupted(state.tick) {
            econ_spans.push(Span::styled(
                "  [DISRUPTED]",
                Style::default().fg(Color::Red),
            ));
        }
        lines.push(Line::from(econ_spans));
        // Regional specialization
        let spec_label = specialization_label(region);
        let spec_color = if region.collapsed { Color::DarkGray } else { Color::Cyan };
        let spec_status = if region.collapsed { " (LOST)" } else { "" };
        lines.push(Line::from(vec![
            Span::styled("Specialization: ", label),
            Span::styled(
                format!("{}{}", spec_label, spec_status),
                Style::default().fg(spec_color),
            ),
        ]));
        // Regional traits with effects
        for t in &region.traits {
            lines.push(Line::from(vec![
                Span::styled(format!("{}: ", t.label()), label),
                Span::styled(t.effect(), Style::default().fg(Color::Yellow)),
            ]));
        }
    }

    // Infrastructure status
    if !region.collapsed {
        fn infra_color(val: f64) -> Color {
            if val <= 0.0 || val < crate::state::INFRA_CRITICAL {
                Color::Red
            } else if val < crate::state::INFRA_STRESSED {
                Color::Yellow
            } else {
                Color::Green
            }
        }
        let hc = region.healthcare_capacity;
        let sl = region.supply_lines;
        let co = region.civil_order;
        // Only show infrastructure when at least one system is degraded
        if hc < 1.0 || sl < 1.0 || co < 1.0 {
            lines.push(Line::from(vec![
                Span::styled("Infra: ", label),
                Span::styled("HC ", label),
                Span::styled(
                    format!("{}%", (hc * 100.0) as u32),
                    Style::default().fg(infra_color(hc)),
                ),
                Span::styled("  SL ", label),
                Span::styled(
                    format!("{}%", (sl * 100.0) as u32),
                    Style::default().fg(infra_color(sl)),
                ),
                Span::styled("  CO ", label),
                Span::styled(
                    format!("{}%", (co * 100.0) as u32),
                    Style::default().fg(infra_color(co)),
                ),
            ]));
            // Show effect warnings for stressed/critical systems
            let mut effects: Vec<Span> = Vec::new();
            if hc < crate::state::INFRA_CRITICAL {
                effects.push(Span::styled(
                    format!("  HC: {}x lethality", crate::state::HEALTHCARE_CRITICAL_LETHALITY as u32),
                    Style::default().fg(Color::Red),
                ));
            } else if hc < crate::state::INFRA_STRESSED {
                effects.push(Span::styled(
                    format!("  HC: {}x lethality", crate::state::HEALTHCARE_STRESSED_LETHALITY as u32),
                    Style::default().fg(Color::Yellow),
                ));
            }
            if sl < crate::state::INFRA_CRITICAL {
                effects.push(Span::styled(
                    "  SL: 2x deploy time, 1.5x policy cost",
                    Style::default().fg(Color::Red),
                ));
            } else if sl < crate::state::INFRA_STRESSED {
                effects.push(Span::styled(
                    "  SL: 1.5x policy cost",
                    Style::default().fg(Color::Yellow),
                ));
            }
            if !effects.is_empty() {
                let mut effect_line = vec![Span::styled("Effects:", label)];
                effect_line.extend(effects);
                lines.push(Line::from(effect_line));
            }
            // Show delivery efficiency when impaired
            let eff = region.delivery_efficiency();
            if eff < 0.95 {
                lines.push(Line::from(vec![
                    Span::styled("Delivery efficiency: ", label),
                    Span::styled(
                        format!("{:.0}%", eff * 100.0),
                        Style::default().fg(if eff < 0.5 { Color::Red } else { Color::Yellow }),
                    ),
                    Span::styled(
                        " (HC × SL)",
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        }
    }

    // Governor cooperation
    {
        let gov = &region.governor;
        if gov.is_dead() {
            // Leaderless state: show succession countdown
            let succession_info = if let Some(succ_tick) = gov.succession_tick {
                let ticks_left = succ_tick.saturating_sub(state.tick);
                let days_left = ticks_left as f64 / crate::state::TICKS_PER_DAY;
                format!(" (successor in {:.0} days)", days_left.ceil())
            } else {
                String::new()
            };
            lines.push(Line::from(vec![
                Span::styled("LEADERLESS", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::styled(succession_info, Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!(" (policies {:.0}% effective)", gov.policy_effectiveness() * 100.0),
                    Style::default().fg(Color::Red),
                ),
            ]));
        } else {
            let cooperation_color = if gov.is_defiant() {
                Color::Red
            } else if gov.is_cooperative() {
                Color::Green
            } else {
                Color::Yellow
            };
            let status = if gov.is_defiant() {
                " DEFIANT"
            } else if gov.is_cooperative() {
                " cooperative"
            } else {
                ""
            };
            let gov_is_board = state.board_members.iter().any(|bm| {
                matches!(bm.role, crate::state::BoardRole::RegionGovernor { region_idx: ri } if ri == idx)
            });
            let gov_board_marker = if gov_is_board { " ★" } else { "" };
            lines.push(Line::from(vec![
                Span::styled(&gov.name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(gov_board_marker, Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!(" ({}) ", gov.personality.label()),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled("Co-Op: ", label),
                Span::styled(
                    format!("{:.0}", gov.cooperation),
                    Style::default().fg(cooperation_color),
                ),
                Span::styled(
                    status,
                    Style::default().fg(cooperation_color).add_modifier(Modifier::BOLD),
                ),
                {
                    let eff = gov.policy_effectiveness();
                    if eff < 1.0 {
                        Span::styled(
                            format!(" (policies {:.0}% effective)", eff * 100.0),
                            Style::default().fg(Color::Red),
                        )
                    } else {
                        Span::raw("")
                    }
                },
            ]));
        }
    }

    // Local market — corporations headquartered in this region
    {
        let corps = state.region_corporations(idx);
        if !corps.is_empty() && lines.len() + 2 < inner.height as usize {
            lines.push(Line::from(Span::styled(
                "─── Local Market ───",
                Style::default().fg(Color::DarkGray),
            )));
            for corp in &corps {
                if lines.len() >= inner.height as usize { break; }
                let change = corp.price_change_pct();
                let (ticker_str, price_color) = if corp.bankrupt {
                    ("  BUST".to_string(), Color::Red)
                } else {
                    let color = if corp.share_price >= corp.ipo_price * 0.8 { Color::Green }
                        else if corp.share_price >= corp.ipo_price * 0.5 { Color::Yellow }
                        else { Color::LightRed };
                    (format!("¥{:.0}", corp.share_price), color)
                };
                let change_color = if change > 0.5 { Color::Green }
                    else if change < -0.5 { Color::Red }
                    else { Color::DarkGray };
                let board_marker = if corp.board_seat { " ★" } else { "" };
                let spark = sparkline(&corp.price_history, 8);
                // Truncate long names to fit the column, then pad to fixed width.
                let max_name_len = 22;
                let display_name: String = corp.name.chars().take(max_name_len).collect();
                // Build change string with fixed-width arrow column.
                // Arrows (▲/▼) are 1 display column each, so format! padding works.
                let padded_change = if corp.bankrupt {
                    format!("{:<10}", "")
                } else if change > 0.5 {
                    format!(" ▲{:>+5.1}%", change)
                } else if change < -0.5 {
                    format!(" ▼{:>+5.1}%", change)
                } else {
                    format!(" {:<9}", "──")
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!(" {:<width$}", display_name, width = max_name_len),
                        Style::default().fg(if corp.bankrupt { Color::DarkGray } else { Color::White }),
                    ),
                    Span::styled(
                        format!("{:>7} ", ticker_str),
                        Style::default().fg(price_color),
                    ),
                    Span::styled(
                        padded_change,
                        Style::default().fg(change_color),
                    ),
                    Span::styled(
                        spark,
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        board_marker,
                        Style::default().fg(Color::Yellow),
                    ),
                ]));
            }
        }
    }

    // Sector bonuses from local corporations
    {
        let bonuses = state.active_sector_bonuses(idx);
        if !bonuses.is_empty() && lines.len() + 1 < inner.height as usize {
            let mut bonus_spans: Vec<Span> = vec![
                Span::styled("Sector: ", label),
            ];
            for (j, (sector, strength)) in bonuses.iter().enumerate() {
                if j > 0 {
                    bonus_spans.push(Span::styled("  ", label));
                }
                let color = if *strength > 0.7 { Color::Green }
                    else if *strength > 0.3 { Color::Yellow }
                    else { Color::DarkGray };
                bonus_spans.push(Span::styled(
                    sector.bonus_text(*strength),
                    Style::default().fg(color),
                ));
            }
            lines.push(Line::from(bonus_spans));
        }
    }

    // Co-infection warning (only show detected diseases — don't leak undetected info)
    {
        let coinfection_count = region.infections.iter()
            .filter(|inf| inf.infected >= COINFECTION_THRESHOLD
                && state.diseases.get(inf.disease_idx).is_some_and(|d| d.detected))
            .count();
        if coinfection_count >= 2 {
            let pct = (COINFECTION_LETHALITY_PER_DISEASE * (coinfection_count as f64 - 1.0) * 100.0) as u32;
            lines.push(Line::from(vec![
                Span::styled("Co-infection: ", label),
                Span::styled(
                    format!("+{}% lethality ({} diseases)", pct, coinfection_count),
                    Style::default().fg(Color::Red),
                ),
            ]));
        }
    }

    // Per-disease breakdown (detected diseases only)
    if !region.infections.is_empty() {
        for inf in &region.infections {
            if lines.len() >= inner.height as usize {
                break;
            }
            if let Some(disease) = state.diseases.get(inf.disease_idx) {
                if !disease.detected {
                    continue;
                }
                let dname = disease.display_name(inf.disease_idx);
                // Distribute region's total estimate proportionally across diseases.
                // Without antigen screening, exposed (incubating) people are invisible.
                let total_real = if shows_exposed {
                    region.detected_infected(&state.diseases)
                } else {
                    region.detected_symptomatic(&state.diseases)
                };
                let this_disease_total = if shows_exposed { inf.exposed + inf.infected } else { inf.infected };
                let proportion = if total_real > 0.0 { this_disease_total / total_real } else { 0.0 };
                let screened_inf = region.estimated_infected * proportion;
                let shown_immune = if shows_immune { inf.immune } else { 0.0 };
                let susceptible = pop - screened_inf - dead - shown_immune;
                let mut spans = vec![
                    Span::styled(
                        format!("  {:<20}", dname),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::styled("Inf ", label),
                    Span::styled(
                        format!("{:<10}", format_number(screened_inf)),
                        Style::default().fg(Color::Red),
                    ),
                ];
                if shows_immune {
                    spans.push(Span::styled("Immune ", label));
                    spans.push(Span::styled(
                        format!("{:<10}", format_number(shown_immune)),
                        Style::default().fg(Color::Cyan),
                    ));
                }
                spans.extend([
                    Span::styled("Dead ", label),
                    Span::styled(
                        format!("{:<10}", format_number(inf.dead)),
                        Style::default().fg(if inf.dead > 0.0 { Color::Red } else { Color::DarkGray }),
                    ),
                ]);
                if susceptible > 0.0 {
                    spans.push(Span::styled("Susceptible ", label));
                    spans.push(Span::styled(
                        format_number(susceptible.max(0.0)),
                        Style::default().fg(Color::Cyan),
                    ));
                }
                lines.push(Line::from(spans));
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  No infections",
            Style::default().fg(Color::Green),
        )));
    }


    // Non-drawable connection hint
    let hidden = non_drawable_connections(state, idx);
    if !hidden.is_empty() && lines.len() < inner.height as usize {
        let names: Vec<&str> = hidden
            .iter()
            .filter_map(|&j| state.regions.get(j).map(|r| r.name.as_str()))
            .collect();
        lines.push(Line::from(Span::styled(
            format!("  Connected to: {}", names.join(", ")),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}
