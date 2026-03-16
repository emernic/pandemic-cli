use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders},
    Frame,
};

use crate::state::{AppState, BasicTech, ResearchKind, ticks_to_days};

struct TechNode {
    tech: BasicTech,
    row: u16,
    col: u16,
}

struct TechEdge {
    from: BasicTech,
    to: BasicTech,
}

fn short_name(tech: BasicTech) -> &'static str {
    use BasicTech::*;
    match tech {
        TargetedDrugDesign => "Targeted Drug Design",
        MonoclonalAntibodies => "Monoclonal Antibodies",
        PhageTherapy => "Phage Therapy",
        RapidSequencing => "Rapid Sequencing",
        MetagenomicSurveillance => "Metagenomic Surv.",
        ResistanceSurveillance => "Resistance Surv.",
        CombinationTherapy => "Combination Therapy",
        ResilientGrids => "Resilient Grids",
        EpidemiologicalForecasting => "Epi. Forecasting",
    }
}

/// Layout: every edge connects row N to row N+1 only. No skipping rows.
fn tree_layout() -> Vec<TechNode> {
    use BasicTech::*;
    vec![
        TechNode { tech: TargetedDrugDesign,         row: 0, col: 0 },
        TechNode { tech: RapidSequencing,             row: 0, col: 1 },

        TechNode { tech: MonoclonalAntibodies,        row: 1, col: 0 },
        TechNode { tech: ResistanceSurveillance,      row: 1, col: 1 },

        TechNode { tech: PhageTherapy,                row: 2, col: 0 },
        TechNode { tech: MetagenomicSurveillance,     row: 2, col: 1 },

        TechNode { tech: ResilientGrids,              row: 3, col: 0 },
        TechNode { tech: EpidemiologicalForecasting,  row: 3, col: 1 },

        TechNode { tech: CombinationTherapy,          row: 4, col: 1 },

    ]
}

/// Visual edges derived from `BasicTech::tech_prereqs()` — single source of truth.
fn tree_edges() -> Vec<TechEdge> {
    let layout = tree_layout();
    let mut edges = Vec::new();
    for node in &layout {
        for &prereq in node.tech.tech_prereqs() {
            edges.push(TechEdge { from: prereq, to: node.tech });
        }
    }
    edges
}

fn find_node(layout: &[TechNode], tech: BasicTech) -> Option<&TechNode> {
    layout.iter().find(|n| n.tech == tech)
}

/// Precomputed edge indices for each tech's outgoing and incoming edges,
/// sorted by the other endpoint's column.
struct EdgeIndex {
    /// For each tech in layout order: sorted outgoing edge indices.
    outgoing: Vec<Vec<usize>>,
    /// For each tech in layout order: sorted incoming edge indices.
    incoming: Vec<Vec<usize>>,
}

impl EdgeIndex {
    fn build(layout: &[TechNode], edges: &[TechEdge]) -> Self {
        let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); layout.len()];
        let mut incoming: Vec<Vec<usize>> = vec![Vec::new(); layout.len()];

        for (ei, edge) in edges.iter().enumerate() {
            if let Some(from_pos) = layout.iter().position(|n| n.tech == edge.from) {
                outgoing[from_pos].push(ei);
            }
            if let Some(to_pos) = layout.iter().position(|n| n.tech == edge.to) {
                incoming[to_pos].push(ei);
            }
        }

        // Sort outgoing by target column, incoming by source column
        for outs in outgoing.iter_mut() {
            outs.sort_by_key(|&ei| {
                find_node(layout, edges[ei].to).map(|n| n.col).unwrap_or(0)
            });
        }
        for incs in incoming.iter_mut() {
            incs.sort_by_key(|&ei| {
                find_node(layout, edges[ei].from).map(|n| n.col).unwrap_or(0)
            });
        }

        EdgeIndex { outgoing, incoming }
    }
}

fn buf_write(f: &mut Frame, x: u16, y: u16, text: &str, style: Style, max_len: u16) {
    let buf = f.buffer_mut();
    let buf_area = buf.area;
    for (i, ch) in text.chars().enumerate() {
        let cx = x + i as u16;
        if cx >= x + max_len || cx >= buf_area.x + buf_area.width || y >= buf_area.y + buf_area.height {
            break;
        }
        let cell = &mut buf[(cx, y)];
        cell.set_symbol(&ch.to_string());
        cell.set_style(style);
    }
}

fn buf_set(f: &mut Frame, x: u16, y: u16, sym: &str, style: Style, inner: Rect) {
    if x >= inner.x && x < inner.x + inner.width && y >= inner.y && y < inner.y + inner.height {
        let buf = f.buffer_mut();
        let buf_area = buf.area;
        if x < buf_area.x + buf_area.width && y < buf_area.y + buf_area.height {
            let cell = &mut buf[(x, y)];
            cell.set_symbol(sym);
            cell.set_style(style);
        }
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &AppState, selected_idx: usize) {
    let block = Block::default()
        .title(" Research ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 30 || inner.height < 10 {
        return;
    }

    // Full-screen: split into tree (left) and detail (right).
    // Narrow panels (< 80 wide) keep the old bottom-detail layout.
    let is_wide = inner.width >= 80;

    let (tree_rect, detail_rect) = if is_wide {
        // 60% tree, 40% detail
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(inner);
        (split[0], Some(split[1]))
    } else {
        (inner, None)
    };

    render_tree(f, tree_rect, state, selected_idx, !is_wide);

    if let Some(detail_area) = detail_rect {
        if let Some(node) = tree_layout().get(selected_idx) {
            render_detail_panel(f, detail_area, state, node.tech);
        }
    }
}

/// Render the tech tree graph into the given area.
/// If `show_bottom_detail` is true, reserves 5 lines at the bottom for a compact detail strip
/// (used when the panel is narrow and there's no side detail panel).
fn render_tree(f: &mut Frame, area: Rect, state: &AppState, selected_idx: usize, show_bottom_detail: bool) {
    let layout = tree_layout();
    let edges = tree_edges();
    let edge_idx = EdgeIndex::build(&layout, &edges);

    let gap_h: u16 = 2;  // horizontal gap between columns
    let gap_v: u16 = 3;  // vertical gap: departure row, highway row, arrival row
    let box_w: u16 = ((area.width.saturating_sub(2 * gap_h)) / 3).min(30);
    let box_h: u16 = 3;

    let detail_h: u16 = if show_bottom_detail { 5 } else { 0 };
    let tree_area_h = area.height.saturating_sub(detail_h);

    let selected_row = layout.get(selected_idx).map(|n| n.row).unwrap_or(0);
    let total_rows = layout.iter().map(|n| n.row).max().unwrap_or(0) + 1;
    let total_pixel_h = total_rows * (box_h + gap_v);
    let scroll_y: u16 = if total_pixel_h > tree_area_h {
        let selected_pixel_y = selected_row * (box_h + gap_v);
        let center = tree_area_h / 2;
        if selected_pixel_y > center {
            (selected_pixel_y - center).min(total_pixel_h.saturating_sub(tree_area_h))
        } else {
            0
        }
    } else {
        0
    };

    let box_left = |col: u16| -> u16 { area.x + col * (box_w + gap_h) };
    let box_top_raw = |row: u16| -> u16 { area.y + row * (box_h + gap_v) };
    let scrolled = |raw_y: u16| -> Option<u16> { raw_y.checked_sub(scroll_y) };

    let spaced_x = |bx: u16, k: usize, n: usize| -> u16 {
        bx + (box_w * (k as u16 + 1)) / (n as u16 + 1)
    };

    // Clip rect for the tree area (excludes bottom detail strip)
    let tree_clip = Rect::new(area.x, area.y, area.width, tree_area_h);

    // --- Draw edges (connections rendered BEFORE boxes so boxes cover overlaps) ---
    for (i, edge) in edges.iter().enumerate() {
        let from_node = match find_node(&layout, edge.from) { Some(n) => n, None => continue };
        let to_node = match find_node(&layout, edge.to) { Some(n) => n, None => continue };

        let from_pos = layout.iter().position(|n| n.tech == edge.from).unwrap_or(0);
        let to_pos = layout.iter().position(|n| n.tech == edge.to).unwrap_or(0);

        let out = &edge_idx.outgoing[from_pos];
        let out_k = out.iter().position(|&ei| ei == i).unwrap_or(0);
        let exit_x = spaced_x(box_left(from_node.col), out_k, out.len());

        let inc = &edge_idx.incoming[to_pos];
        let in_k = inc.iter().position(|&ei| ei == i).unwrap_or(0);
        let entry_x = spaced_x(box_left(to_node.col), in_k, inc.len());

        let dep_y_raw = box_top_raw(from_node.row) + box_h;
        let hw_y_raw = dep_y_raw + 1;
        let arr_y_raw = dep_y_raw + 2;
        let target_top_raw = box_top_raw(to_node.row);

        let from_unlocked = state.unlocked_techs.contains(&edge.from);
        let to_unlocked = state.unlocked_techs.contains(&edge.to);
        let color = if to_unlocked {
            Color::Green
        } else if from_unlocked {
            Color::Yellow
        } else {
            Color::DarkGray
        };
        let style = Style::default().fg(color);

        let same_column = from_node.col == to_node.col;

        if same_column {
            // Same column: straight vertical at entry_x
            for raw_y in dep_y_raw..target_top_raw {
                if let Some(y) = scrolled(raw_y) {
                    buf_set(f, entry_x, y, "│", style, tree_clip);
                }
            }
        } else {
            // Cross-column: 3-layer routing
            if let Some(y) = scrolled(dep_y_raw) {
                buf_set(f, exit_x, y, "│", style, tree_clip);
            }

            if let Some(y) = scrolled(hw_y_raw) {
                let (left_x, right_x) = if exit_x < entry_x {
                    (exit_x, entry_x)
                } else {
                    (entry_x, exit_x)
                };

                if exit_x < entry_x {
                    buf_set(f, exit_x, y, "╰", style, tree_clip);
                    buf_set(f, entry_x, y, "╮", style, tree_clip);
                } else {
                    buf_set(f, exit_x, y, "╯", style, tree_clip);
                    buf_set(f, entry_x, y, "╭", style, tree_clip);
                }

                for x in (left_x + 1)..right_x {
                    buf_set(f, x, y, "─", style, tree_clip);
                }
            }

            for raw_y in arr_y_raw..target_top_raw {
                if let Some(y) = scrolled(raw_y) {
                    buf_set(f, entry_x, y, "│", style, tree_clip);
                }
            }
        }
    }

    // --- Draw boxes on top of connections ---
    for (idx, node) in layout.iter().enumerate() {
        let x = box_left(node.col);
        let y = match scrolled(box_top_raw(node.row)) {
            Some(y) if y + box_h <= area.y + tree_area_h && y >= area.y => y,
            _ => continue,
        };

        let rect = Rect::new(x, y, box_w, box_h);
        let is_selected = selected_idx == idx;
        let is_unlocked = state.unlocked_techs.contains(&node.tech);
        let is_available = node.tech.prerequisites_met(&state.world);
        let is_researching = state.active_research.iter().any(|r| {
            matches!(r.kind, ResearchKind::BasicResearch { tech } if tech == node.tech)
        });

        let is_queued = state.world.queued_techs.contains(&node.tech);

        let (border_color, name_color) = if is_unlocked {
            (Color::Green, Color::Green)
        } else if is_researching {
            (Color::Yellow, Color::Yellow)
        } else if is_queued {
            (Color::Rgb(210, 180, 140), Color::Rgb(210, 180, 140))
        } else if is_available {
            (Color::White, Color::White)
        } else {
            (Color::DarkGray, Color::DarkGray)
        };

        let border_mod = if is_selected { Modifier::BOLD } else { Modifier::empty() };
        let is_locked = !is_unlocked && !is_available && !is_researching;

        if is_locked {
            let border_style = Style::default().fg(border_color).add_modifier(border_mod);
            // Clear interior (edges may pass through)
            for by in (y + 1)..(y + box_h - 1) {
                for bx in (x + 1)..(x + box_w - 1) {
                    buf_set(f, bx, by, " ", Style::default(), tree_clip);
                }
            }
            // Dotted borders for locked techs
            for bx in (x + 1)..(x + box_w - 1) {
                buf_set(f, bx, y, "┄", border_style, tree_clip);
                buf_set(f, bx, y + box_h - 1, "┄", border_style, tree_clip);
            }
            for by in (y + 1)..(y + box_h - 1) {
                buf_set(f, x, by, "┊", border_style, tree_clip);
                buf_set(f, x + box_w - 1, by, "┊", border_style, tree_clip);
            }
            if is_selected {
                let corner_style = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
                buf_set(f, x, y, "╔", corner_style, tree_clip);
                buf_set(f, x + box_w - 1, y, "╗", corner_style, tree_clip);
                buf_set(f, x, y + box_h - 1, "╚", corner_style, tree_clip);
                buf_set(f, x + box_w - 1, y + box_h - 1, "╝", corner_style, tree_clip);
            } else {
                buf_set(f, x, y, "┌", border_style, tree_clip);
                buf_set(f, x + box_w - 1, y, "┐", border_style, tree_clip);
                buf_set(f, x, y + box_h - 1, "└", border_style, tree_clip);
                buf_set(f, x + box_w - 1, y + box_h - 1, "┘", border_style, tree_clip);
            }
        } else {
            let border_type = if is_selected { BorderType::Double } else { BorderType::Plain };
            let sel_border_color = if is_selected { Color::White } else { border_color };
            let box_block = Block::default()
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(sel_border_color).add_modifier(border_mod));
            f.render_widget(box_block, rect);
        }

        let name = short_name(node.tech);
        let max_inner = (box_w as usize).saturating_sub(2);
        let name_style = Style::default().fg(name_color).add_modifier(
            if is_selected { Modifier::BOLD } else { Modifier::empty() }
        );

        // Show progress percentage inline for actively researching techs
        let research = if is_researching {
            state.active_research.iter().find(|r| {
                matches!(r.kind, ResearchKind::BasicResearch { tech } if tech == node.tech)
            })
        } else {
            None
        };

        if let Some(r) = research {
            let pct = (r.progress / r.required_ticks * 100.0).min(100.0) as u8;
            let pct_str = format!(" {}%", pct);
            let name_budget = max_inner.saturating_sub(pct_str.len());
            let truncated: String = name.chars().take(name_budget).collect();
            buf_write(f, x + 1, y + 1, &truncated, name_style, name_budget as u16);
            let pct_x = x + 1 + truncated.len() as u16;
            buf_write(f, pct_x, y + 1, &pct_str, Style::default().fg(Color::Yellow), max_inner as u16);
        } else {
            buf_write(f, x + 1, y + 1, name, name_style, max_inner as u16);
        }
    }

    // --- Bottom detail strip (only when no side detail panel) ---
    if show_bottom_detail {
        if let Some(node) = layout.get(selected_idx) {
            let detail_y = area.y + tree_area_h;
            if detail_y + 3 < area.y + area.height {
                let max_w = area.width.saturating_sub(2);

                for sx in area.x..area.x + area.width {
                    buf_set(f, sx, detail_y, "─", Style::default().fg(Color::DarkGray), area);
                }

                buf_write(
                    f, area.x + 1, detail_y + 1, node.tech.name(),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                    max_w,
                );
                buf_write(
                    f, area.x + 1, detail_y + 2, node.tech.description(),
                    Style::default().fg(Color::DarkGray),
                    max_w,
                );
                let prereq = format!("Requires: {}", node.tech.prereq_description());
                let tech_unlocked = state.unlocked_techs.contains(&node.tech);
                let tech_available = node.tech.prerequisites_met(&state.world);
                let prereq_color = if tech_unlocked || tech_available { Color::Green } else { Color::Yellow };
                buf_write(
                    f, area.x + 1, detail_y + 3, &prereq,
                    Style::default().fg(prereq_color),
                    max_w,
                );
            }
        }
    }
}

/// Render the detail panel for the selected tech on the right side of the full-screen layout.
fn render_detail_panel(f: &mut Frame, area: Rect, state: &AppState, tech: BasicTech) {
    // Draw a left-side vertical separator
    for y in area.y..area.y + area.height {
        buf_set(f, area.x, y, "│", Style::default().fg(Color::DarkGray), area);
    }

    // Content area (inset from separator)
    let cx = area.x + 2;
    let max_w = area.width.saturating_sub(4);
    if max_w < 10 {
        return;
    }
    let mut y = area.y + 1;
    let y_max = area.y + area.height;

    let bold = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::DarkGray);
    let normal = Style::default().fg(Color::White);

    // --- Status ---
    let is_unlocked = state.unlocked_techs.contains(&tech);
    let is_available = tech.prerequisites_met(&state.world);
    let active_research = state.active_research.iter().find(|r| {
        matches!(r.kind, ResearchKind::BasicResearch { tech: t } if t == tech)
    });

    let is_queued = state.world.queued_techs.contains(&tech);

    let (status_text, status_color) = if is_unlocked {
        ("UNLOCKED", Color::Green)
    } else if active_research.is_some() {
        ("RESEARCHING", Color::Yellow)
    } else if is_queued {
        ("QUEUED", Color::Rgb(210, 180, 140))
    } else if is_available {
        ("AVAILABLE", Color::Cyan)
    } else {
        ("LOCKED", Color::DarkGray)
    };

    // --- Name ---
    if y < y_max {
        buf_write(f, cx, y, tech.name(), bold, max_w);
        y += 1;
    }

    // Status tag + progress bar (inline)
    if y < y_max {
        buf_write(f, cx, y, status_text, Style::default().fg(status_color).add_modifier(Modifier::BOLD), max_w);
        y += 1;
    }
    if let Some(research) = active_research {
        if y < y_max {
            let pct = (research.progress / research.required_ticks * 100.0).min(100.0);
            let bar_w = (max_w as usize).saturating_sub(10).min(30);
            let filled = ((pct / 100.0) * bar_w as f64).round() as usize;
            let empty = bar_w.saturating_sub(filled);
            let bar = format!("  [{}{}] {:.0}%", "█".repeat(filled), "░".repeat(empty), pct);
            buf_write(f, cx, y, &bar, Style::default().fg(Color::Yellow), max_w);
            y += 1;
        }
        if y < y_max {
            let remaining_ticks = research.required_ticks - research.progress;
            let remaining_days = ticks_to_days(remaining_ticks.max(0.0));
            let eta = format!("  ~{:.1} days remaining", remaining_days);
            buf_write(f, cx, y, &eta, dim, max_w);
            y += 1;
        }
    }

    y += 1; // blank line

    // --- Description (word-wrapped) ---
    if y < y_max {
        let desc = tech.description();
        let lines = word_wrap(desc, max_w as usize);
        for line in &lines {
            if y >= y_max { break; }
            buf_write(f, cx, y, line, normal, max_w);
            y += 1;
        }
    }

    y += 1; // blank line

    // --- Prerequisites ---
    if y < y_max {
        buf_write(f, cx, y, "Prerequisites", dim.add_modifier(Modifier::BOLD), max_w);
        y += 1;
    }
    if y < y_max {
        let prereq_met = is_unlocked || is_available;
        let prereq_color = if prereq_met { Color::Green } else { Color::Yellow };
        let check = if prereq_met { "+" } else { "-" };
        let prereq_text = format!(" {} {}", check, tech.prereq_description());
        buf_write(f, cx, y, &prereq_text, Style::default().fg(prereq_color), max_w);
        y += 1;
    }

    y += 1; // blank line

    // --- Costs ---
    let research_kind = ResearchKind::BasicResearch { tech };
    let (personnel, ticks, funding) = research_kind.costs(&state.world.medicines);
    let (eff_personnel, eff_ticks, eff_funding) = state.effective_costs(&research_kind);

    if y < y_max {
        buf_write(f, cx, y, "Costs", dim.add_modifier(Modifier::BOLD), max_w);
        y += 1;
    }

    let days = ticks_to_days(eff_ticks);
    let red = Style::default().fg(Color::Red);
    let personnel_unmet = state.world.personnel_available() < eff_personnel;
    let funding_unmet = state.world.resources.funding < eff_funding;

    let cost_lines: [(String, Style); 3] = [
        (format!("  Personnel:  {}", eff_personnel), if personnel_unmet { red } else { normal }),
        (format!("  Duration:   {:.1} days", days), normal),
        (format!("  Funding:    {} \u{00a5}", eff_funding as i64), if funding_unmet { red } else { normal }),
    ];

    // Show base vs effective if tech modifiers are active
    let has_modifier = eff_personnel != personnel || (eff_ticks - ticks).abs() > 0.1 || (eff_funding - funding).abs() > 0.1;

    for (line, style) in &cost_lines {
        if y >= y_max { break; }
        buf_write(f, cx, y, line, *style, max_w);
        y += 1;
    }

    if has_modifier && y < y_max {
        let base_days = ticks_to_days(ticks);
        let base_text = format!("  (base: {} pers, {:.1}d, {}\u{00a5})", personnel, base_days, funding as i64);
        buf_write(f, cx, y, &base_text, dim, max_w);
        y += 1;
    }

    // --- Hint line at bottom ---
    let hint_y = area.y + area.height - 1;
    if hint_y > y && hint_y < y_max {
        let hint = if is_unlocked {
            "[Esc] Close"
        } else if active_research.is_some() {
            "[Esc] Close"
        } else if is_available {
            "[Enter] Start Research  [Esc] Close"
        } else {
            "[Esc] Close"
        };
        buf_write(f, cx, hint_y, hint, dim, max_w);
    }
}

/// Simple word-wrap: breaks text into lines no wider than `width`.
fn word_wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current = word.to_string();
        } else if current.len() + 1 + word.len() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

pub fn node_count() -> usize {
    tree_layout().len()
}

/// Returns the techs in tree layout order (for mapping selection index to tech).
pub fn layout_techs() -> Vec<BasicTech> {
    tree_layout().iter().map(|n| n.tech).collect()
}

/// Navigate the tech tree spatially. Returns the new selection index.
pub fn navigate(current_idx: usize, direction: TreeDirection) -> usize {
    let layout = tree_layout();
    let current = match layout.get(current_idx) {
        Some(n) => n,
        None => return current_idx,
    };

    match direction {
        TreeDirection::Up => {
            // Find nearest node in a lower row, preferring same column
            layout.iter().enumerate()
                .filter(|(_, n)| n.row < current.row)
                .max_by_key(|(_, n)| (n.row, -(n.col as i16 - current.col as i16).abs() as i16))
                .map(|(i, _)| i)
                .unwrap_or(current_idx)
        }
        TreeDirection::Down => {
            // Find nearest node in a higher row, preferring same column
            layout.iter().enumerate()
                .filter(|(_, n)| n.row > current.row)
                .min_by_key(|(_, n)| (n.row, (n.col as i16 - current.col as i16).abs()))
                .map(|(i, _)| i)
                .unwrap_or(current_idx)
        }
        TreeDirection::Left => {
            // Find nearest node in same row with lower column
            layout.iter().enumerate()
                .filter(|(_, n)| n.row == current.row && n.col < current.col)
                .max_by_key(|(_, n)| n.col)
                .map(|(i, _)| i)
                .unwrap_or(current_idx)
        }
        TreeDirection::Right => {
            // Find nearest node in same row with higher column
            layout.iter().enumerate()
                .filter(|(_, n)| n.row == current.row && n.col > current.col)
                .min_by_key(|(_, n)| n.col)
                .map(|(i, _)| i)
                .unwrap_or(current_idx)
        }
    }
}

pub enum TreeDirection {
    Up,
    Down,
    Left,
    Right,
}
