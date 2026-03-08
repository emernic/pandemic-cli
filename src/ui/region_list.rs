use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{map_grid_pos, GameState, Region, MAP_GRID_LEN};
use crate::ui::research::disease_display_name;

use super::format_number;

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
    let region_height = ((inner.height.saturating_sub(gap_row)) / 2).min(8);

    // Draw connections in gap areas
    let connections = drawable_connections(state);
    {
        let buf = f.buffer_mut();
        let buf_area = buf.area;
        for conn in &connections {
            // Grid positions are guaranteed valid by drawable_connections
            let (ca, ra) = map_grid_pos(conn.a).unwrap();
            let (_cb, rb) = map_grid_pos(conn.b).unwrap();

            let has_spread = state.regions[conn.a].total_infected() > 0.0
                || state.regions[conn.b].total_infected() > 0.0;
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

    // Render each region box
    for (idx, region) in state.regions.iter().enumerate() {
        let (col, row) = match map_grid_pos(idx) {
            Some(p) => p,
            None => break,
        };
        let x = inner.x + col * (region_width + gap_col);
        let y = inner.y + row * (region_height + gap_row);
        let rect = Rect::new(x, y, region_width, region_height);
        let selected = idx == state.ui.map_selection;
        render_region_box(f, rect, region, selected, state);
    }

    // Show hints for connections that can't be drawn on the grid
    let hidden = non_drawable_connections(state, state.ui.map_selection);
    if !hidden.is_empty() {
        let names: Vec<&str> = hidden
            .iter()
            .filter_map(|&j| state.regions.get(j).map(|r| r.name.as_str()))
            .collect();
        if !names.is_empty() {
            let (col, row) = map_grid_pos(state.ui.map_selection).unwrap();
            let box_x = inner.x + col * (region_width + gap_col);
            let box_y = inner.y + row * (region_height + gap_row) + region_height;
            if box_y < inner.y + inner.height {
                let hint = format!("↔ {}", names.join(", "));
                let hint_line = Line::from(Span::styled(
                    hint,
                    Style::default().fg(Color::DarkGray),
                ));
                let hint_area = Rect::new(box_x, box_y, region_width, 1);
                f.render_widget(Paragraph::new(vec![hint_line]), hint_area);
            }
        }
    }
}

fn render_region_box(
    f: &mut Frame,
    area: Rect,
    region: &Region,
    selected: bool,
    state: &GameState,
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
        .border_style(Style::default().fg(border_color).add_modifier(border_mod));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 2 || inner.height < 1 {
        return;
    }

    let infected = region.total_infected();
    let immune = region.total_immune();
    let dead = region.total_dead();
    let pop = region.population as f64;

    let threat = if infected > 100_000.0 {
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

    // Line 2: Population
    if inner.height >= 2 {
        lines.push(Line::from(Span::styled(
            format!("Pop: {}", format_number(pop)),
            Style::default().fg(Color::DarkGray),
        )));
    }

    // Line 3: Health bar — nonzero values get at least 1 char
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
        // Clamp so minimums don't exceed bar width
        let used = inf_w + imm_w + dead_w;
        if used > bar_w {
            // Scale down proportionally, but keep at least 1 for each
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

        let mut spans = Vec::new();
        if sus_w > 0 {
            spans.push(Span::styled(
                "█".repeat(sus_w),
                Style::default().fg(Color::Cyan),
            ));
        }
        if inf_w > 0 {
            spans.push(Span::styled(
                "█".repeat(inf_w),
                Style::default().fg(Color::Red),
            ));
        }
        if imm_w > 0 {
            spans.push(Span::styled(
                "█".repeat(imm_w),
                Style::default().fg(Color::Green),
            ));
        }
        if dead_w > 0 {
            spans.push(Span::styled(
                "█".repeat(dead_w),
                Style::default().fg(Color::DarkGray),
            ));
        }
        if spans.is_empty() {
            spans.push(Span::styled(
                "█".repeat(bar_w),
                Style::default().fg(Color::Cyan),
            ));
        }
        lines.push(Line::from(spans));
    }

    // Lines 4+: Per-disease detail (selected regions only)
    if selected && inner.height >= 4 {
        if region.infections.is_empty() {
            lines.push(Line::from(Span::styled(
                "No infections",
                Style::default().fg(Color::Green),
            )));
        } else {
            for inf in &region.infections {
                if lines.len() >= inner.height as usize {
                    break;
                }
                if let Some(disease) = state.diseases.get(inf.disease_idx) {
                    let dname = disease_display_name(disease, inf.disease_idx);
                    let max_dname = iw.saturating_sub(12);
                    let display_dname = if dname.len() > max_dname {
                        &dname[..max_dname]
                    } else {
                        dname.as_str()
                    };
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{}: ", display_dname),
                            Style::default().fg(Color::Yellow),
                        ),
                        Span::styled(format_number(inf.infected), Style::default().fg(Color::Red)),
                        Span::raw(" inf"),
                    ]));
                }
            }
        }
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}
