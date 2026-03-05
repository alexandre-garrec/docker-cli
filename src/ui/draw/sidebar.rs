use crate::ui::app::App;
use crate::ui::types::SidebarKind;
use chrono::Local;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

pub fn draw_sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    let color_primary = Color::Rgb(0, 255, 255); // Cyan
    let color_secondary = Color::Rgb(255, 170, 0); // Gold
    let color_dim = Color::Rgb(100, 100, 120);

    let border_style_list = if app.focus_on_list {
        Style::default().fg(color_primary)
    } else {
        Style::default().fg(color_dim)
    };

    let items: Vec<ListItem> = if app.docker.available {
        app.items.iter().map(|it| {
            let (raw_label, style) = match it.kind {
                SidebarKind::GroupHeader => (it.label.clone(), Style::default().fg(color_secondary).add_modifier(Modifier::BOLD)),
                SidebarKind::Separator => (it.label.clone(), Style::default().fg(color_dim).add_modifier(Modifier::DIM)),
                SidebarKind::SwarmService => {
                    let prefix = if it.selected { "● " } else { "○ " };
                    (format!("{prefix}{}", it.label), Style::default().fg(Color::Rgb(80, 180, 255)))
                }
                SidebarKind::Task => {
                    let prefix = if it.selected { "● " } else { "○ " };
                    (format!("{prefix}{}", it.label), Style::default().fg(Color::Rgb(255, 255, 80)))
                }
                SidebarKind::Container => {
                    let prefix = if it.selected { "● " } else { "○ " };
                    (format!("{prefix}{}", it.label), Style::default())
                }
                SidebarKind::Image => (String::new(), Style::default()),
            };
            
            let label = if it.depth > 0 {
                format!("{}└─ {}", "  ".repeat(it.depth - 1), raw_label)
            } else {
                raw_label
            };

            let mut final_style = style;
            if it.selected && it.kind != SidebarKind::GroupHeader && it.kind != SidebarKind::Separator {
                final_style = final_style.add_modifier(Modifier::BOLD);
            }
            ListItem::new(label).style(final_style)
        }).collect()
    } else {
        vec![ListItem::new("(docker not available)")]
    };

    let updated = Local::now().format("%H:%M:%S").to_string();
    let left_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style_list)
        .title(format!(" 🐳 Containers + Tasks [upd: {updated}] "));

    let list = List::new(items)
        .block(left_block)
        .highlight_style(Style::default().fg(Color::Black).bg(color_primary).add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");
    
    app.list_area = area;
    f.render_stateful_widget(list, area, &mut app.list_state);
}
