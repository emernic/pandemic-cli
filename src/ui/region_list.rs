use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::state::{map_grid_pos, GameState, Region, MAP_GRID_LEN,
    COINFECTION_LETHALITY_PER_DISEASE, COINFECTION_THRESHOLD};

use crate::format_number;

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

            let has_spread = state.regions[conn.a].detected_infected(&state.diseases) > 0.0
                || state.regions[conn.b].detected_infected(&state.diseases) > 0.0;
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
        render_region_box(f, rect, region, selected, &state.diseases, visibility, shows_immune);
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
) {
    let border_color = if selected {
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

    let infected = region.screened_infected(diseases, visibility);
    let immune = if shows_immune { region.detected_immune(diseases) } else { 0.0 };
    let dead = region.detected_dead(diseases);
    let pop = region.population as f64;

    let threat = if region.collapsed {
        ("FELL", Color::Red)
    } else if infected > 100_000.0 {
        ("CRIT", Color::Red)
    } else if infected > 10_000.0 {
        ("HIGH", Color::LightRed)
    } else if infected > 1_000.0 {
        ("MOD", Color::Yellow)
    } else if infected > 0.0 {
        ("LOW", Color::Green)
    } else {
        ("OK", Color::DarkGray)
    };

    let name_style = if selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let iw = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    // Line 1: Name + threat level
    let name = &region.name;
    let threat_len = threat.0.len();
    let max_name = iw.saturating_sub(threat_len + 1);
    let display_name: &str = if name.len() > max_name {
        &name[..max_name]
    } else {
        name
    };
    let padding = iw.saturating_sub(display_name.len() + threat_len);
    lines.push(Line::from(vec![
        Span::styled(display_name.to_string(), name_style),
        Span::raw(" ".repeat(padding)),
        Span::styled(
            threat.0,
            Style::default()
                .fg(threat.1)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Line 2: Key stats
    if inner.height >= 2 {
        if infected == 0.0 && dead == 0.0 {
            lines.push(Line::from(Span::styled(
                format!("Pop: {}", format_number(pop)),
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            let mut stats = Vec::new();
            stats.push(Span::styled("Inf ", Style::default().fg(Color::Red)));
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

        let segments: [(usize, Color); 4] = [
            (sus_w, Color::Cyan),
            (inf_w, Color::Red),
            (imm_w, Color::Green),
            (dead_w, Color::DarkGray),
        ];

        let mut spans = Vec::new();
        for (width, color) in segments {
            if width > 0 {
                spans.push(Span::styled("█".repeat(width), Style::default().fg(color)));
            }
        }
        // Fill remaining with healthy (if rounding left gaps)
        let total: usize = segments.iter().map(|(w, _)| w).sum();
        if total < bar_w {
            let remaining = bar_w - total;
            spans.push(Span::styled("█".repeat(remaining), Style::default().fg(Color::Cyan)));
        }
        lines.push(Line::from(spans));

        // Line 4: Collapse threshold indicator below the bar
        if inner.height >= 4 && !region.collapsed {
            let death_fraction_at_collapse = 1.0 - region.collapse_threshold;
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
    let infected = region.screened_infected(&state.diseases, visibility);
    let shows_immune = state.screening_shows_immune(idx);
    let immune = if shows_immune { region.detected_immune(&state.diseases) } else { 0.0 };
    let dead = region.detected_dead(&state.diseases);
    let alive = pop - dead; // alive based on detected deaths only

    let label = Style::default().fg(Color::DarkGray);
    let val = Style::default().fg(Color::White);

    let mut lines: Vec<Line> = Vec::new();

    // Collapse banner
    if region.collapsed {
        lines.push(Line::from(Span::styled(
            "  ██ COLLAPSED — society has broken down ██",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
    }

    // Population summary line
    let mut summary_spans = vec![
        Span::styled("Pop ", label),
        Span::styled(format_number(pop), val),
        Span::styled("  Alive ", label),
        Span::styled(format_number(alive), Style::default().fg(Color::Green)),
        Span::styled("  Infected ", label),
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
        let collapse_death_pct = (1.0 - region.collapse_threshold) * 100.0;
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
        let income_label = if region.income_modifier >= 1.5 {
            ("High", Color::Green)
        } else if region.income_modifier >= 1.0 {
            ("Moderate", Color::Yellow)
        } else {
            ("Low", Color::Red)
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
        lines.push(Line::from(vec![
            Span::styled("Economy: ", label),
            Span::styled(
                format!("{} ({:.1}x)", income_label.0, region.income_modifier),
                Style::default().fg(income_label.1),
            ),
            Span::styled("  Healthcare: ", label),
            Span::styled(
                healthcare_label.0,
                Style::default().fg(healthcare_label.1),
            ),
        ]));
        // Regional traits
        if !region.traits.is_empty() {
            let trait_labels: Vec<&str> = region.traits.iter().map(|t| t.label()).collect();
            lines.push(Line::from(vec![
                Span::styled("Traits: ", label),
                Span::styled(
                    trait_labels.join(", "),
                    Style::default().fg(Color::Yellow),
                ),
            ]));
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
                let screened_inf = inf.infected * visibility;
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

    // Active policies
    if let Some(policy) = state.policies.get(idx) {
        if policy.any_active() && lines.len() < inner.height as usize {
            let mut policy_parts: Vec<Span> = vec![
                Span::styled("  Policies: ", label),
            ];
            let policy_style = Style::default().fg(Color::Yellow);
            if policy.travel_ban {
                policy_parts.push(Span::styled("Travel Ban ", policy_style));
            }
            if policy.quarantine {
                policy_parts.push(Span::styled("Quarantine ", policy_style));
            }
            if policy.hospital_surge {
                policy_parts.push(Span::styled("Hospital Surge ", policy_style));
            }
            if policy.border_controls {
                policy_parts.push(Span::styled("Border Ctrl ", policy_style));
            }
            if policy.water_sanitation {
                policy_parts.push(Span::styled("Water Sanit. ", policy_style));
            }
            match policy.screening {
                crate::state::ScreeningLevel::None => {}
                crate::state::ScreeningLevel::Basic => {
                    policy_parts.push(Span::styled("Screen:Basic ", policy_style));
                }
                crate::state::ScreeningLevel::Antigen => {
                    policy_parts.push(Span::styled("Screen:Antigen ", policy_style));
                }
                crate::state::ScreeningLevel::MassRapid => {
                    policy_parts.push(Span::styled("Screen:Rapid ", policy_style));
                }
            }
            lines.push(Line::from(policy_parts));
        }
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
