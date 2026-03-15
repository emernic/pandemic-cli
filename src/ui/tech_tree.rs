use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders},
    Frame,
};

use crate::state::{AppState, BasicTech};

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
        VaccinePlatform => "Vaccine Platform",
        ResistanceSurveillance => "Resistance Surv.",
        CombinationTherapy => "Combination Therapy",
        CompetitiveDisplacement => "Competitive Displ.",
        DirectedAttenuation => "Directed Attenuation",
        GeneDriveContainment => "Gene Drive Contain.",
        AutomatedSynthesis => "Automated Synthesis",
        StabilizedFormulation => "Stabilized Formula.",
        ResilientGrids => "Resilient Grids",
        EpidemiologicalForecasting => "Epi. Forecasting",
    }
}

/// Layout: every edge connects row N to row N+1 only. No skipping rows.
/// Three columns, each a mostly independent vertical chain.
fn tree_layout() -> Vec<TechNode> {
    use BasicTech::*;
    vec![
        // Row 0: Foundation roots
        TechNode { tech: TargetedDrugDesign,      row: 0, col: 0 },
        TechNode { tech: RapidSequencing,          row: 0, col: 1 },
        TechNode { tech: AutomatedSynthesis,       row: 0, col: 2 },

        // Row 1
        TechNode { tech: MonoclonalAntibodies,     row: 1, col: 0 },
        TechNode { tech: ResistanceSurveillance,   row: 1, col: 1 },
        TechNode { tech: StabilizedFormulation,    row: 1, col: 2 },

        // Row 2
        TechNode { tech: PhageTherapy,             row: 2, col: 0 },
        TechNode { tech: MetagenomicSurveillance,  row: 2, col: 1 },

        // Row 3
        TechNode { tech: VaccinePlatform,          row: 3, col: 0 },
        TechNode { tech: EpidemiologicalForecasting, row: 3, col: 1 },

        // Row 4
        TechNode { tech: ResilientGrids,           row: 4, col: 0 },
        TechNode { tech: CombinationTherapy,       row: 4, col: 1 },

        // Row 5
        TechNode { tech: CompetitiveDisplacement,  row: 5, col: 0 },

        // Row 6
        TechNode { tech: DirectedAttenuation,      row: 6, col: 0 },

        // Row 7
        TechNode { tech: GeneDriveContainment,     row: 7, col: 0 },
    ]
}

/// Visual edges for the tree. Every edge goes from row N to row N+1.
/// These represent the visual layout only — actual game prereqs are in state.rs.
fn tree_edges() -> Vec<TechEdge> {
    use BasicTech::*;
    vec![
        // Col 0 chain: TDD → Mono → Phage → VaccPlatform → Resilient → CompDispl → ...
        TechEdge { from: TargetedDrugDesign,      to: MonoclonalAntibodies },
        TechEdge { from: MonoclonalAntibodies,     to: PhageTherapy },
        TechEdge { from: PhageTherapy,             to: VaccinePlatform },
        TechEdge { from: VaccinePlatform,          to: ResilientGrids },
        TechEdge { from: ResilientGrids,           to: CompetitiveDisplacement },
        TechEdge { from: CompetitiveDisplacement,  to: DirectedAttenuation },
        TechEdge { from: DirectedAttenuation,      to: GeneDriveContainment },

        // Col 1 chain: RapidSeq → ResistSurv → MetaSurv → EpiForecasting → CombTherapy
        TechEdge { from: RapidSequencing,          to: ResistanceSurveillance },
        TechEdge { from: ResistanceSurveillance,   to: MetagenomicSurveillance },
        TechEdge { from: MetagenomicSurveillance,  to: EpidemiologicalForecasting },
        TechEdge { from: EpidemiologicalForecasting, to: CombinationTherapy },

        // Col 2 chain: AutoSynth → StabForm
        TechEdge { from: AutomatedSynthesis,       to: StabilizedFormulation },

        // Cross-column (adjacent only, row N to row N+1):
        // CombTherapy (row 4, col 1) → CompDisplacement (row 5, col 0)
        TechEdge { from: CombinationTherapy,       to: CompetitiveDisplacement },
    ]
}

fn find_node<'a>(layout: &'a [TechNode], tech: BasicTech) -> Option<&'a TechNode> {
    layout.iter().find(|n| n.tech == tech)
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
        .title(" Tech Tree ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 30 || inner.height < 10 {
        return;
    }

    let layout = tree_layout();
    let edges = tree_edges();

    let gap_h: u16 = 2;  // horizontal gap between columns
    let gap_v: u16 = 3;  // vertical gap: departure row, highway row, arrival row
    let box_w: u16 = ((inner.width.saturating_sub(2 * gap_h)) / 3).min(30);
    let box_h: u16 = 3;

    let detail_h: u16 = 5;
    let tree_area_h = inner.height.saturating_sub(detail_h);

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

    // Pixel position helpers (before scroll)
    let box_left = |col: u16| -> u16 { inner.x + col * (box_w + gap_h) };
    let box_top_raw = |row: u16| -> u16 { inner.y + row * (box_h + gap_v) };

    // Scrolled y (returns None if off-screen above)
    let scrolled = |raw_y: u16| -> Option<u16> { raw_y.checked_sub(scroll_y) };

    // --- Compute attachment points for each edge ---

    // For a given tech, return edge indices sorted by the OTHER end's column.
    let outgoing_sorted = |tech: BasicTech| -> Vec<usize> {
        let mut v: Vec<usize> = edges.iter().enumerate()
            .filter(|(_, e)| e.from == tech).map(|(i, _)| i).collect();
        v.sort_by_key(|&i| find_node(&layout, edges[i].to).map(|n| n.col).unwrap_or(0));
        v
    };
    let incoming_sorted = |tech: BasicTech| -> Vec<usize> {
        let mut v: Vec<usize> = edges.iter().enumerate()
            .filter(|(_, e)| e.to == tech).map(|(i, _)| i).collect();
        v.sort_by_key(|&i| find_node(&layout, edges[i].from).map(|n| n.col).unwrap_or(0));
        v
    };

    // Evenly space N points across a box width. Point k (0-indexed) of N total:
    // box_left + box_w * (k+1) / (N+1)
    let spaced_x = |bx: u16, k: usize, n: usize| -> u16 {
        bx + (box_w * (k as u16 + 1)) / (n as u16 + 1)
    };

    // --- Draw edges ---
    // Each edge is routed as:
    //   [source box bottom]
    //   dep_y:  │ at exit_x           (vertical stub down from source)
    //   hw_y:   ╰────────╮ or │       (elbows + horizontal, or straight vertical)
    //   arr_y:            │ at entry_x (vertical stub, continues down to target)
    //   ... more │ at entry_x through any intermediate rows/gaps ...
    //   [target box top]

    for (i, edge) in edges.iter().enumerate() {
        let from_node = match find_node(&layout, edge.from) { Some(n) => n, None => continue };
        let to_node = match find_node(&layout, edge.to) { Some(n) => n, None => continue };

        // Exit x: evenly spaced among source's outgoing edges
        let out = outgoing_sorted(edge.from);
        let out_k = out.iter().position(|&ei| ei == i).unwrap_or(0);
        let exit_x = spaced_x(box_left(from_node.col), out_k, out.len());

        // Entry x: evenly spaced among target's incoming edges
        let inc = incoming_sorted(edge.to);
        let in_k = inc.iter().position(|&ei| ei == i).unwrap_or(0);
        let entry_x = spaced_x(box_left(to_node.col), in_k, inc.len());

        // Raw y positions (before scroll)
        let dep_y_raw = box_top_raw(from_node.row) + box_h;     // first gap row below source
        let hw_y_raw = dep_y_raw + 1;                            // middle gap row
        let arr_y_raw = dep_y_raw + 2;                           // last gap row
        let target_top_raw = box_top_raw(to_node.row);           // top of target box

        // Edge color
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
            // Same column: draw straight vertical at entry_x (the target's attachment point).
            // Even if exit_x differs slightly due to spacing, we go straight down —
            // the departure stub silently shifts to entry_x rather than elbowing.
            for raw_y in dep_y_raw..target_top_raw {
                if let Some(y) = scrolled(raw_y) {
                    buf_set(f, entry_x, y, "│", style, inner);
                }
            }
        } else {
            // Layer 1: departure stub (vertical at exit_x)
            if let Some(y) = scrolled(dep_y_raw) {
                buf_set(f, exit_x, y, "│", style, inner);
            }

            // Layer 2: highway (elbows + horizontal)
            if let Some(y) = scrolled(hw_y_raw) {
                let (left_x, right_x) = if exit_x < entry_x {
                    (exit_x, entry_x)
                } else {
                    (entry_x, exit_x)
                };

                // Left elbow
                if exit_x < entry_x {
                    buf_set(f, exit_x, y, "╰", style, inner);   // comes from above, turns right
                    buf_set(f, entry_x, y, "╮", style, inner);  // comes from left, turns down
                } else {
                    buf_set(f, exit_x, y, "╯", style, inner);   // comes from above, turns left
                    buf_set(f, entry_x, y, "╭", style, inner);  // comes from right, turns down
                }

                // Horizontal between the two elbows
                for x in (left_x + 1)..right_x {
                    buf_set(f, x, y, "─", style, inner);
                }
            }

            // Layer 3 + continuation: vertical at entry_x from arr_y to target top
            for raw_y in arr_y_raw..target_top_raw {
                if let Some(y) = scrolled(raw_y) {
                    buf_set(f, entry_x, y, "│", style, inner);
                }
            }
        }
    }

    // --- Draw boxes on top of connections ---
    for (idx, node) in layout.iter().enumerate() {
        let x = box_left(node.col);
        let y = match scrolled(box_top_raw(node.row)) {
            Some(y) if y + box_h <= inner.y + tree_area_h && y >= inner.y => y,
            _ => continue,
        };

        let rect = Rect::new(x, y, box_w, box_h);
        let is_selected = selected_idx == idx;
        let is_unlocked = state.unlocked_techs.contains(&node.tech);
        let is_available = node.tech.prerequisites_met(&state.world);
        let is_researching = state.active_research.iter().any(|r| {
            matches!(r.kind, crate::state::ResearchKind::BasicResearch { tech } if tech == node.tech)
        });

        let (border_color, name_color) = if is_unlocked {
            (Color::Green, Color::Green)
        } else if is_researching {
            (Color::Yellow, Color::Yellow)
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
                    buf_set(f, bx, by, " ", Style::default(), inner);
                }
            }
            // Dotted borders
            for bx in (x + 1)..(x + box_w - 1) {
                buf_set(f, bx, y, "┄", border_style, inner);
                buf_set(f, bx, y + box_h - 1, "┄", border_style, inner);
            }
            for by in (y + 1)..(y + box_h - 1) {
                buf_set(f, x, by, "┊", border_style, inner);
                buf_set(f, x + box_w - 1, by, "┊", border_style, inner);
            }
            if is_selected {
                buf_set(f, x, y, "╔", border_style, inner);
                buf_set(f, x + box_w - 1, y, "╗", border_style, inner);
                buf_set(f, x, y + box_h - 1, "╚", border_style, inner);
                buf_set(f, x + box_w - 1, y + box_h - 1, "╝", border_style, inner);
            } else {
                buf_set(f, x, y, "┌", border_style, inner);
                buf_set(f, x + box_w - 1, y, "┐", border_style, inner);
                buf_set(f, x, y + box_h - 1, "└", border_style, inner);
                buf_set(f, x + box_w - 1, y + box_h - 1, "┘", border_style, inner);
            }
        } else {
            let border_type = if is_selected { BorderType::Double } else { BorderType::Plain };
            let box_block = Block::default()
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(border_color).add_modifier(border_mod));
            f.render_widget(box_block, rect);
        }

        let name = short_name(node.tech);
        let max_inner = (box_w as usize).saturating_sub(2);
        let name_style = Style::default().fg(name_color).add_modifier(
            if is_selected { Modifier::BOLD } else { Modifier::empty() }
        );
        buf_write(f, x + 1, y + 1, name, name_style, max_inner as u16);
    }

    // --- Detail panel at bottom ---
    if let Some(node) = layout.get(selected_idx) {
        let detail_y = inner.y + tree_area_h;
        if detail_y + 3 < inner.y + inner.height {
            let max_w = inner.width.saturating_sub(2);

            for sx in inner.x..inner.x + inner.width {
                buf_set(f, sx, detail_y, "─", Style::default().fg(Color::DarkGray), inner);
            }

            buf_write(
                f, inner.x + 1, detail_y + 1, node.tech.name(),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                max_w,
            );

            buf_write(
                f, inner.x + 1, detail_y + 2, node.tech.description(),
                Style::default().fg(Color::DarkGray),
                max_w,
            );

            let prereq = format!("Requires: {}", node.tech.prereq_description());
            let tech_unlocked = state.unlocked_techs.contains(&node.tech);
            let tech_available = node.tech.prerequisites_met(&state.world);
            let prereq_color = if tech_unlocked || tech_available { Color::Green } else { Color::Yellow };
            buf_write(
                f, inner.x + 1, detail_y + 3, &prereq,
                Style::default().fg(prereq_color),
                max_w,
            );
        }
    }
}

pub fn node_count() -> usize {
    tree_layout().len()
}
