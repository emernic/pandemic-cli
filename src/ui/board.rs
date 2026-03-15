use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{BoardRole, AppState, ModifierSource, TICKS_PER_DAY};


/// Maximum selection index for the board panel.
pub fn selection_max(state: &AppState) -> usize {
    state.board_members.len().saturating_sub(1)
}

/// Satisfaction word and color for a satisfaction value (0.0-1.0).
fn satisfaction_display(satisfaction: f64) -> (&'static str, Color) {
    if satisfaction > 0.7 {
        ("Content", Color::Green)
    } else if satisfaction > 0.5 {
        ("Wary", Color::Yellow)
    } else if satisfaction > 0.3 {
        ("Displeased", Color::LightRed)
    } else {
        ("Hostile", Color::Red)
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &AppState) {
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line: Option<usize> = None;

    if state.board_members.is_empty() {
        lines.push(Line::from(Span::styled(
            "No board members.",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        // Overall board satisfaction header
        let board_sat = state.board_satisfaction();
        let (overall_word, overall_color) = satisfaction_display(board_sat);
        lines.push(Line::from(vec![
            Span::styled("  Mood: ", Style::default().fg(Color::DarkGray)),
            Span::styled(overall_word, Style::default().fg(overall_color).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("  ({} members)", state.board_members.len()),
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        // Next board meeting countdown
        if state.next_board_meeting_tick > state.tick {
            let ticks_remaining = state.next_board_meeting_tick - state.tick;
            let days_remaining = ticks_remaining as f64 / TICKS_PER_DAY;
            lines.push(Line::from(vec![
                Span::styled("  Next meeting in: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:.1} days", days_remaining),
                    Style::default().fg(if days_remaining < 2.0 { Color::Yellow } else { Color::White }),
                ),
            ]));
        }

        // Board budget display
        let budget_day = state.board_budget_per_tick * TICKS_PER_DAY;
        let base_day = state.base_board_budget_per_tick() * TICKS_PER_DAY;
        let budget_color = if budget_day > base_day * 1.05 {
            Color::Green
        } else if budget_day < base_day * 0.95 {
            Color::Red
        } else {
            Color::White
        };
        lines.push(Line::from(Span::styled(
            format!("  Budget: ¥{:.0}/day", budget_day),
            Style::default().fg(budget_color),
        )));

        lines.push(Line::from(""));

        for (i, member) in state.board_members.iter().enumerate() {
            let selected = state.ui.panel_selection == i;
            if selected {
                selected_line = Some(lines.len());
            }
            let marker = if selected { "\u{25b6} " } else { "  " };
            let style = if selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            // Dead members: show greyed out with death indicator
            if member.dead {
                lines.push(Line::from(vec![
                    Span::styled(format!("{}{}", marker, member.name), Style::default().fg(Color::DarkGray)),
                    Span::styled(" [DEAD]", Style::default().fg(Color::Red)),
                ]));
                lines.push(Line::from(Span::styled(
                    format!("    {}", member.title),
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::from(""));
                continue;
            }

            // Name line (with personality in gray parentheses, like governor personalities)
            let mut name_spans: Vec<Span> = vec![
                Span::styled(format!("{}{}", marker, member.name), style),
            ];
            if let Some(personality) = &member.personality {
                name_spans.push(Span::styled(
                    format!(" ({})", personality.label()),
                    Style::default().fg(Color::DarkGray),
                ));
            } else if let BoardRole::RegionGovernor { region_idx } = &member.role {
                if let Some(region) = state.regions.get(*region_idx) {
                    name_spans.push(Span::styled(
                        format!(" ({})", region.governor.personality.label()),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
            }
            lines.push(Line::from(name_spans));

            // Title + satisfaction + connection indicators
            let (sat_word, sat_color) = satisfaction_display(member.satisfaction);
            let mut detail_spans: Vec<Span> = vec![
                Span::styled(
                    format!("    {}", member.title),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw("  "),
                Span::styled(sat_word, Style::default().fg(sat_color)),
            ];

            // Connection indicators
            let mut connections: Vec<String> = Vec::new();
            if let Some(corp_idx) = member.corp_idx {
                if let Some(corp) = state.corporations.get(corp_idx) {
                    connections.push(format!("[{}]", corp.sector.label()));
                }
            }
            if let Some(region_idx) = member.region_idx {
                if let Some(region) = state.regions.get(region_idx) {
                    if matches!(member.role, BoardRole::RegionGovernor { .. }) {
                        connections.push(format!("[Gov: {}]", region.name));
                    }
                }
            }

            if !connections.is_empty() {
                detail_spans.push(Span::styled(
                    format!("  {}", connections.join(" ")),
                    Style::default().fg(Color::Cyan),
                ));
            }

            lines.push(Line::from(detail_spans));

            // Active demand summary — show the worst modifier driving unhappiness
            if member.satisfaction < 0.5 {
                let demand_text = worst_modifier_demand(state, i);
                if let Some(text) = demand_text {
                    lines.push(Line::from(Span::styled(
                        format!("    Unhappy: {}", text),
                        Style::default().fg(Color::LightRed),
                    )));
                }
            }

            // Detail view for selected member
            if selected {
                render_member_detail(&mut lines, state, i);
            }

            lines.push(Line::from(""));
        }
    }

    let block = Block::default()
        .title(" Board ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner_height = area.height.saturating_sub(2);
    let scroll_offset = crate::ui::scroll_offset_for_selection(&lines, selected_line, inner_height);

    let widget = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset, 0));
    f.render_widget(widget, area);
}

fn render_member_detail(lines: &mut Vec<Line<'static>>, state: &AppState, member_idx: usize) {
    let member = &state.board_members[member_idx];
    let hdr = Style::default().fg(Color::DarkGray);

    // Chairman effect (personality-specific power)
    if member.is_chairman {
        if let Some(personality) = &member.personality {
            lines.push(Line::from(Span::styled(
                format!("    {}", personality.chairman_effect_description()),
                Style::default().fg(Color::Magenta),
            )));
        }
    }

    // Unified Approval section — shows all named modifiers
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "    \u{2500}\u{2500} Approval \u{2500}\u{2500}",
        Style::default().fg(Color::Cyan),
    )));

    let modifiers = state.member_satisfaction_modifiers(member_idx);
    for m in modifiers {
        // Build context string for certain modifiers
        let context = modifier_context(state, member, &m.source);
        let label = m.source.label();
        let pct = m.value * 100.0;
        let val_color = if pct > 1.0 { Color::Green }
            else if pct > -1.0 { Color::DarkGray }
            else { Color::Red };

        let mut spans = vec![
            Span::styled(format!("    {}: ", label), hdr),
            Span::styled(
                format!("{:+.0}%", pct),
                Style::default().fg(val_color),
            ),
        ];
        if !context.is_empty() {
            spans.push(Span::styled(format!("  {}", context), hdr));
        }
        lines.push(Line::from(spans));
    }

    // Show final satisfaction
    let (sat_word, sat_color) = satisfaction_display(member.satisfaction);
    lines.push(Line::from(vec![
        Span::styled("    Total: ", hdr),
        Span::styled(
            format!("{:.0}%", member.satisfaction * 100.0),
            Style::default().fg(sat_color),
        ),
        Span::styled(
            format!("  ({})", sat_word),
            Style::default().fg(sat_color),
        ),
    ]));

    // Active contract with this board member (if any)
    if let Some(contract) = state.contracts.iter().find(|c| c.board_member_idx == member_idx) {
        let income_per_day = contract.income * TICKS_PER_DAY;
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "    \u{2500}\u{2500} Contract \u{2500}\u{2500}",
            Style::default().fg(Color::Cyan),
        )));
        lines.push(Line::from(vec![
            Span::styled("    Income: ", hdr),
            Span::styled(
                format!("+\u{00a5}{:.0}/day", income_per_day),
                Style::default().fg(Color::Green),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    Condition: ", hdr),
            Span::styled(
                contract.condition.description(),
                Style::default().fg(Color::White),
            ),
        ]));
        lines.push(Line::from(Span::styled(
            "    [X] Cancel contract",
            Style::default().fg(Color::Yellow),
        )));
    }
}

/// Build a short context string for a modifier line (e.g., stock price, GDP status).
fn modifier_context(
    state: &AppState,
    member: &crate::state::BoardMember,
    source: &ModifierSource,
) -> String {
    match source {
        ModifierSource::StockPerformance => {
            let corp_idx = match &member.role {
                BoardRole::CorporateLeader { corp_idx } => Some(*corp_idx),
                _ => member.corp_idx,
            };
            if let Some(idx) = corp_idx {
                if let Some(corp) = state.corporations.get(idx) {
                    if corp.bankrupt {
                        return "BANKRUPT".to_string();
                    }
                    let change = corp.price_change_pct();
                    let arrow = if change > 0.5 { "\u{25b2}" }
                        else if change < -0.5 { "\u{25bc}" }
                        else { "" };
                    return format!("\u{00a5}{:.0}{}", corp.share_price, arrow);
                }
            }
            String::new()
        }
        ModifierSource::RegionalGdp => {
            if let Some(idx) = member.region_idx {
                if let Some(region) = state.regions.get(idx) {
                    return format!("{:.0}k ({})", region.gdp, region.gdp_status());
                }
            }
            String::new()
        }
        ModifierSource::GlobalSurvival => {
            let initial = state.initial_population();
            if initial > 0.0 {
                let alive: f64 = state.regions.iter().map(|r| r.alive()).sum();
                let pct = (alive / initial) * 100.0;
                return format!("{:.1}%", pct);
            }
            String::new()
        }
        _ => String::new(),
    }
}

/// Find the most negative satisfaction modifier and return a human-readable
/// demand string explaining why this board member is unhappy.
fn worst_modifier_demand(state: &AppState, member_idx: usize) -> Option<String> {
    let modifiers = state.member_satisfaction_modifiers(member_idx);
    // Find the most negative non-Base modifier
    let worst = modifiers.iter()
        .filter(|m| m.source != ModifierSource::Base)
        .min_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal))?;

    if worst.value >= 0.0 {
        return None; // No negative modifiers — shouldn't happen at <50% but be safe
    }

    let member = &state.board_members[member_idx];
    Some(match &worst.source {
        ModifierSource::StockPerformance => {
            let corp_name = member.corp_idx
                .and_then(|idx| state.corporations.get(idx))
                .map(|c| c.name.as_str())
                .unwrap_or("corporation");
            format!("{} stock is down", corp_name)
        }
        ModifierSource::RegionalGdp => {
            let region_name = member.region_idx
                .and_then(|idx| state.regions.get(idx))
                .map(|r| r.name.as_str())
                .unwrap_or("region");
            format!("{} GDP declining", region_name)
        }
        ModifierSource::ResearchUtilization => "Research capacity underused".to_string(),
        ModifierSource::GlobalSurvival => "Too many lives lost".to_string(),
        ModifierSource::PlayerInvestment => {
            let corp_name = member.corp_idx
                .and_then(|idx| state.corporations.get(idx))
                .map(|c| c.name.as_str())
                .unwrap_or("corporation");
            format!("Wants investment in {}", corp_name)
        }
        ModifierSource::InitialSkepticism => "Doesn't trust you yet".to_string(),
        ModifierSource::RestrictivePolicies => {
            let region_name = member.region_idx
                .and_then(|idx| state.regions.get(idx))
                .map(|r| r.name.as_str())
                .unwrap_or("region");
            format!("Too many restrictions in {}", region_name)
        }
        ModifierSource::RegionalStanding => {
            let region_name = member.region_idx
                .and_then(|idx| state.regions.get(idx))
                .map(|r| r.name.as_str())
                .unwrap_or("region");
            format!("{} falling behind other regions", region_name)
        }
        ModifierSource::GovernorDysfunction => "Governors too cooperative".to_string(),
        ModifierSource::FundingReserves => "Funding reserves too low".to_string(),
        ModifierSource::PersonnelDeployment => "Not enough personnel deployed".to_string(),
        ModifierSource::RegionalSurvival => {
            let region_name = member.region_idx
                .and_then(|idx| state.regions.get(idx))
                .map(|r| r.name.as_str())
                .unwrap_or("region");
            format!("{} losing population", region_name)
        }
        other => format!("{}", other.label()),
    })
}
