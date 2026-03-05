use crate::ui::app::App;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub fn draw_logs(f: &mut Frame, app: &mut App, area: Rect) {
    let color_primary = Color::Rgb(0, 255, 255); // Cyan
    let color_secondary = Color::Rgb(255, 170, 0); // Gold
    let color_dim = Color::Rgb(100, 100, 120);

    let border_style_logs = if app.focus_on_list {
        Style::default().fg(color_dim)
    } else {
        Style::default().fg(color_primary)
    };

    let (title, border_style_logs_actual) = if app.shell_active {
        (
            format!(" 🐚 SHELL — {t} (Esc to exit) ", t = app.current_target),
            Style::default().fg(color_secondary).add_modifier(Modifier::BOLD)
        )
    } else {
        let stats_info = app.container_stats.as_ref().map(|s| {
            let cpu_gauge = crate::docker::ContainerStats::gauge_char(s.cpu_percent);
            let mem_gauge = crate::docker::ContainerStats::gauge_char(s.mem_percent);
            
            let mem = if s.mem_usage_mb >= 1024.0 {
                format!("{:.1}G/{:.1}G", s.mem_usage_mb / 1024.0, s.mem_limit_mb / 1024.0)
            } else {
                format!("{:.0}M/{:.0}M", s.mem_usage_mb, s.mem_limit_mb)
            };
            let net = format!("↓{:.1}M ↑{:.1}M", s.net_rx_mb, s.net_tx_mb);
            let block = format!("R{:.1}M W{:.1}M", s.block_read_mb, s.block_write_mb);
            format!("  CPU {} {:.1}%  RAM {} {}  NET {}  IO {}", cpu_gauge, s.cpu_percent, mem_gauge, mem, net, block)
        }).unwrap_or_default();
        
        let follow_status = if app.follow_mode { "[FOLLOWING]" } else { "[PAUSED]" };
        let t = if app.current_target.is_empty() {
            format!(" 📑 Logs {} ", follow_status)
        } else {
            format!(" 📑 Logs {} — {t}{stats} ", follow_status, t = app.current_target, stats = stats_info)
        };
        (t, border_style_logs)
    };

    let (right_pane_history, right_pane_input) = if app.shell_active {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)].as_ref())
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let query_lower = app.log_filter_query.to_lowercase();
    let is_active_filter = !query_lower.is_empty();

    let mut filtered_lines = Vec::new();
    for l in app.log_lines.iter() {
        if !is_active_filter || l.to_lowercase().contains(&query_lower) {
            filtered_lines.push(l.clone());
        }
    }

    let log_text_lines: Vec<Line> = filtered_lines
        .into_iter()
        .map(|l| {
            if l.starts_with('❯') {
                Line::from(vec![
                    Span::styled(" ❯ ", Style::default().fg(color_secondary).add_modifier(Modifier::BOLD)),
                    Span::styled(l.trim_start_matches('❯').trim().to_string(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                ])
            } else if is_active_filter {
                // Highlight matches
                let mut spans = Vec::new();
                let lower_l = l.to_lowercase();
                let mut last_idx = 0;
                for (idx, _) in lower_l.match_indices(&query_lower) {
                    if idx > last_idx {
                        spans.push(Span::raw(l[last_idx..idx].to_string()));
                    }
                    spans.push(Span::styled(
                        l[idx..idx + query_lower.len()].to_string(),
                        Style::default().bg(Color::Yellow).fg(Color::Black).add_modifier(Modifier::BOLD),
                    ));
                    last_idx = idx + query_lower.len();
                }
                if last_idx < l.len() {
                    spans.push(Span::raw(l[last_idx..].to_string()));
                }
                Line::from(spans)
            } else {
                Line::from(Span::raw(l))
            }
        })
        .collect();

    let log_text: Text = log_text_lines.into();

    let log_height = right_pane_history.height.saturating_sub(2);
    app.last_log_height = log_height;

    let total_lines = app.log_lines.len() as u16;
    if app.stick_to_bottom {
        app.log_scroll = total_lines.saturating_sub(log_height);
    } else {
        let max_scroll = total_lines.saturating_sub(log_height);
        if app.log_scroll > max_scroll {
            app.log_scroll = max_scroll;
        }
    }

    let logs = Paragraph::new(log_text)
        .block(Block::default().borders(Borders::ALL).border_style(border_style_logs_actual).title(title))
        .wrap(Wrap { trim: false })
        .scroll((app.log_scroll, 0));
    f.render_widget(logs, right_pane_history);

    if let Some(input_area) = right_pane_input {
        let input_text = Line::from(vec![
            Span::styled(" ❯ ", Style::default().fg(color_secondary).add_modifier(Modifier::BOLD)),
            Span::raw(format!("{}█", app.shell_input)),
        ]);
        let input_widget = Paragraph::new(input_text)
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(color_primary)).title(" Command Input "));
        f.render_widget(input_widget, input_area);
    } else if app.is_filtering_logs {
        let bar_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)].as_ref())
            .split(area)[1];
            
        let filter_text = Line::from(vec![
            Span::styled(" 🔍 Filter Logs: ", Style::default().fg(color_secondary).add_modifier(Modifier::BOLD)),
            Span::raw(format!("{}█", app.log_filter_query)),
        ]);
        let filter_widget = Paragraph::new(filter_text)
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(color_secondary)).title(" Filtering Mode "));
        f.render_widget(filter_widget, bar_area);
    }
}
