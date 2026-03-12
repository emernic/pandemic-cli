use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::{BoardPersonality, BoardRole, GameState, TICKS_PER_DAY};
use crate::format_number;

/// Maximum selection index for the board panel.
pub fn selection_max(state: &GameState) -> usize {
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

pub fn render(f: &mut Frame, area: Rect, state: &GameState) {
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
            Span::styled("  Board mood: ", Style::default().fg(Color::DarkGray)),
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

            // Name line
            lines.push(Line::from(Span::styled(
                format!("{}{}", marker, member.name),
                style,
            )));

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
            if let Some(personality) = &member.personality {
                connections.push(format!("[{}]", personality.label()));
            }
            if state.contracts.iter().any(|c| c.board_member_idx == i) {
                connections.push("[Contract]".to_string());
            }
            if !connections.is_empty() {
                detail_spans.push(Span::styled(
                    format!("  {}", connections.join(" ")),
                    Style::default().fg(Color::Cyan),
                ));
            }

            lines.push(Line::from(detail_spans));

            // Active demand summary (if board satisfaction is low enough that demands fire)
            if member.satisfaction < 0.5 {
                let demand_text = match &member.role {
                    BoardRole::CorporateLeader { corp_idx } => {
                        let corp_name = state.corporations.get(*corp_idx)
                            .map(|c| c.name.as_str()).unwrap_or("corporation");
                        let bankrupt = state.corporations.get(*corp_idx)
                            .map_or(false, |c| c.bankrupt);
                        if bankrupt {
                            Some(format!("Demands: Restore {} operations", corp_name))
                        } else {
                            Some(match member.personality {
                                Some(BoardPersonality::Technocrat) =>
                                    "Demands: Staff research programs".to_string(),
                                Some(BoardPersonality::Humanitarian) =>
                                    "Demands: Prioritize disease containment".to_string(),
                                Some(BoardPersonality::Dealmaker) =>
                                    format!("Demands: Invest in {}", corp_name),
                                Some(BoardPersonality::Profiteer) | None =>
                                    "Demands: Roll back restrictive policies".to_string(),
                            })
                        }
                    }
                    BoardRole::RegionGovernor { region_idx } => {
                        state.regions.get(*region_idx)
                            .map(|r| if r.collapsed {
                                format!("Demands: Rebuild {}", r.name)
                            } else if r.gdp_fraction() < 0.6 {
                                format!("Demands: Restore {} economy", r.name)
                            } else {
                                format!("Demands: Protect {} economy", r.name)
                            })
                    }
                    BoardRole::IndependentAdvisor => {
                        Some("Demands: Reduce global death toll".to_string())
                    }
                };
                if let Some(text) = demand_text {
                    lines.push(Line::from(Span::styled(
                        format!("    {}", text),
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

fn render_member_detail(lines: &mut Vec<Line<'static>>, state: &GameState, member_idx: usize) {
    let member = &state.board_members[member_idx];
    let hdr = Style::default().fg(Color::DarkGray);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "    \u{2500}\u{2500} Interests \u{2500}\u{2500}",
        Style::default().fg(Color::Cyan),
    )));

    // Role-specific detail
    match &member.role {
        BoardRole::CorporateLeader { corp_idx } => {
            if let Some(corp) = state.corporations.get(*corp_idx) {
                let region_name = state.regions.get(corp.region_idx)
                    .map(|r| r.name.as_str()).unwrap_or("?");

                lines.push(Line::from(vec![
                    Span::styled("    Corporation: ", hdr),
                    Span::styled(
                        corp.name.clone(),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        format!("  ({})", region_name),
                        hdr,
                    ),
                ]));

                // Stock price with trend arrow
                let change_pct = corp.price_change_pct();
                let trend_arrow = if corp.bankrupt {
                    ""
                } else if change_pct > 0.5 {
                    " ▲"
                } else if change_pct < -0.5 {
                    " ▼"
                } else {
                    ""
                };
                let price_color = if corp.bankrupt {
                    Color::Red
                } else if corp.share_price >= corp.ipo_price * 0.8 {
                    Color::Green
                } else if corp.share_price >= corp.ipo_price * 0.5 {
                    Color::Yellow
                } else {
                    Color::LightRed
                };

                let price_str = if corp.bankrupt {
                    "BANKRUPT".to_string()
                } else {
                    format!("¥{:.0}{} ({:+.1}%)", corp.share_price, trend_arrow, change_pct)
                };
                lines.push(Line::from(vec![
                    Span::styled("    Stock: ", hdr),
                    Span::styled(price_str, Style::default().fg(price_color)),
                    Span::styled(
                        format!("  IPO: ¥{:.0}", corp.ipo_price),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));

                // Revenue and profit (scaled to ~10x funding for corporate scale feel)
                let profit = corp.daily_profit();
                let profit_color = if profit >= 0.0 { Color::Green } else { Color::Red };
                lines.push(Line::from(vec![
                    Span::styled("    Revenue: ", hdr),
                    Span::styled(
                        format!("¥{:.0}/day", corp.revenue * 10.0),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled("  Profit: ", hdr),
                    Span::styled(
                        format!("{:+.0}/day", profit * 10.0),
                        Style::default().fg(profit_color),
                    ),
                ]));

                // Satisfaction driver / personality interests
                let interests = match &member.personality {
                    Some(p) => p.interests(&corp.name),
                    None => "Tracks stock performance".to_string(),
                };
                lines.push(Line::from(Span::styled(
                    format!("    {}", interests),
                    Style::default().fg(Color::DarkGray),
                )));

                // Chairman effect (personality-specific power)
                if member.is_chairman {
                    if let Some(personality) = &member.personality {
                        lines.push(Line::from(Span::styled(
                            format!("    {}", personality.chairman_effect_description()),
                            Style::default().fg(Color::Magenta),
                        )));
                    }
                }
            }
        }
        BoardRole::RegionGovernor { region_idx } => {
            if let Some(region) = state.regions.get(*region_idx) {
                let gdp_frac = region.gdp_fraction();

                lines.push(Line::from(vec![
                    Span::styled("    Region: ", hdr),
                    Span::styled(
                        region.name.clone(),
                        Style::default().fg(Color::White),
                    ),
                ]));

                let status = if region.collapsed {
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
                lines.push(Line::from(vec![
                    Span::styled("    GDP: ", hdr),
                    Span::styled(
                        format!("{:.0}k", region.gdp),
                        Style::default().fg(status.1),
                    ),
                    Span::styled(
                        format!("  ({})", status.0),
                        hdr,
                    ),
                ]));

                // Infection summary
                let total_infected: f64 = region.infections.iter()
                    .map(|inf| inf.infected)
                    .sum();
                let total_dead: f64 = region.infections.iter()
                    .map(|inf| inf.dead)
                    .sum();
                if total_infected > 0.0 || total_dead > 0.0 {
                    lines.push(Line::from(vec![
                        Span::styled("    Infected: ", hdr),
                        Span::styled(
                            format_number(total_infected),
                            Style::default().fg(Color::LightRed),
                        ),
                        Span::styled("  Dead: ", hdr),
                        Span::styled(
                            format_number(total_dead),
                            Style::default().fg(Color::Red),
                        ),
                    ]));
                }

                // Governor info
                if region.governor.is_dead() {
                    lines.push(Line::from(vec![
                        Span::styled("    Governor: ", hdr),
                        Span::styled("LEADERLESS", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                        Span::styled(
                            format!("  (policies {:.0}%)", region.policy_effectiveness() * 100.0),
                            Style::default().fg(Color::Red),
                        ),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("    Governor: ", hdr),
                        Span::styled(
                            region.governor.name.clone(),
                            Style::default().fg(Color::White),
                        ),
                        Span::styled(
                            format!("  ({})  Co-Op: {:.0}",
                                region.governor.personality.label(),
                                region.governor.cooperation),
                            hdr,
                        ),
                        {
                            let eff = region.policy_effectiveness();
                            if eff < 1.0 {
                                Span::styled(
                                    format!("  (policies {:.0}%)", eff * 100.0),
                                    Style::default().fg(Color::Red),
                                )
                            } else {
                                Span::raw("")
                            }
                        },
                    ]));
                }

                lines.push(Line::from(Span::styled(
                    format!("    Tracks regional GDP (base: {:.0}k)", region.base_gdp),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
        BoardRole::IndependentAdvisor => {
            let total_alive: f64 = state.regions.iter().map(|r| r.alive()).sum();
            let initial = state.initial_population();
            let survival_pct = if initial > 0.0 { (total_alive / initial) * 100.0 } else { 0.0 };

            lines.push(Line::from(vec![
                Span::styled("    Role: ", hdr),
                Span::styled(
                    "Independent advisor",
                    Style::default().fg(Color::White),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("    Global survival: ", hdr),
                Span::styled(
                    format!("{:.1}%", survival_pct),
                    Style::default().fg(if survival_pct > 90.0 { Color::Green }
                        else if survival_pct > 75.0 { Color::Yellow }
                        else { Color::Red }),
                ),
            ]));
            lines.push(Line::from(Span::styled(
                "    Tracks global death rate",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    // Connected corporation (for governor-members who also have a corp)
    if matches!(member.role, BoardRole::RegionGovernor { .. }) {
        if let Some(corp_idx) = member.corp_idx {
            if let Some(corp) = state.corporations.get(corp_idx) {
                let change = corp.price_change_pct();
                let arrow = if corp.bankrupt { "" }
                    else if change > 0.5 { " ▲" }
                    else if change < -0.5 { " ▼" }
                    else { "" };
                let stock_str = if corp.bankrupt {
                    "BANKRUPT".to_string()
                } else {
                    format!("¥{:.0}{}", corp.share_price, arrow)
                };
                let stock_color = if corp.bankrupt { Color::Red }
                    else if corp.share_price >= corp.ipo_price * 0.8 { Color::Green }
                    else if corp.share_price >= corp.ipo_price * 0.5 { Color::Yellow }
                    else { Color::LightRed };
                lines.push(Line::from(vec![
                    Span::styled("    Corp connection: ", hdr),
                    Span::styled(
                        corp.name.clone(),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        format!("  Stock: "),
                        hdr,
                    ),
                    Span::styled(stock_str, Style::default().fg(stock_color)),
                ]));
            }
        }
    }


}
