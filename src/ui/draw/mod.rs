pub mod logs;
pub mod popups;
pub mod sidebar;
pub mod utils;

use crate::ui::app::App;
use crate::ui::types::SidebarKind;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Paragraph, Block, Borders};
use ratatui::Frame;
use ratatui::layout::Rect;

pub fn draw_ui(f: &mut Frame, app: &mut App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)].as_ref())
        .split(f.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(33), Constraint::Percentage(67)].as_ref())
        .split(root[0]);

    // Draw main list
    sidebar::draw_sidebar(f, app, body[0]);

    // Draw right pane (logs + input)
    logs::draw_logs(f, app, body[1]);

    // Draw footer (help or status)
    if app.copy_mode {
        let banner = Paragraph::new(" 📋 COPY MODE — sélectionne avec la souris, appuie sur n'importe quelle touche pour quitter  ")
            .style(Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD));
        f.render_widget(banner, root[1]);
    } else if app.is_filtering {
        let bar = Paragraph::new(format!(" 🔍 Filter: {}█ ", app.filter_query))
            .style(Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD));
        f.render_widget(bar, root[1]);
    } else {
        let help = help_for_selected(app);
        let help_bar = Paragraph::new(help).style(Style::default().fg(Color::Black).bg(Color::White));
        f.render_widget(help_bar, root[1]);
    }

    // Draw active popups
    if let Some(p) = &app.popup {
        popups::draw_popup(f, app, p);
    }

    // Draw toast notification
    if let Some((msg, time, color)) = &app.toast {
        if time.elapsed().as_secs() > 3 {
            app.toast = None;
        } else {
            let text_len = msg.chars().count() as u16;
            let width = text_len + 4; // padding + borders
            if f.area().width >= width && f.area().height >= 3 {
                let x = f.area().width - width - 1;
                let y = 1;
                let toast_area = Rect::new(x, y, width, 3);
                
                let p = Paragraph::new(msg.as_str())
                    .style(Style::default().fg(*color).bg(Color::Black).add_modifier(Modifier::BOLD))
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(*color)));
                f.render_widget(p, toast_area);
            }
        }
    }
}

pub fn help_for_selected(app: &App) -> String {
    if app.shell_active {
        return " Interagissez avec le shell...  Esc:Quitter le shell".to_string();
    }
    if app.is_filtering {
        return format!(" 🔍 Filter: {}  [Enter]:Confirm  [Esc]:Clear", app.filter_query);
    }
    if app.items.is_empty() {
        return " /:Filter  C:Context  V:Volumes  N:Networks  q:Quit  ?:Help".to_string();
    }
    let item = &app.items[app.selected];
    let scroll = if !app.focus_on_list { " ↑/↓:Scroll" } else { "" };
    let filtered_status = if !app.filter_query.is_empty() { " (Filtered)" } else { "" };
    let v_status = if !app.multi_selected.is_empty() { format!(" ({})", app.multi_selected.len()) } else { "".to_string() };
    match item.kind {
        SidebarKind::Container => format!(
            " /:Filter{f}  v:Select{v}  C:Context  V:Volumes  N:Networks  e:Shell  P:Pin  m:CopyMode  y:Copy  r:Restart  s:Stop  t:Start  p:Pause  u:Unpause  k:Kill  d:Rm  i:Inspect  o:Web  tab:Focus  q:Quit  ?:Help{scroll}",
            f = filtered_status, v = v_status
        ),
        SidebarKind::Task => format!(
            " /:Filter{f}  v:Select{v}  C:Context  V:Volumes  N:Networks  r:Run  s:Stop  y:Copy  tab:Focus  q:Quit  ?:Help{scroll}",
            f = filtered_status, v = v_status
        ),
        SidebarKind::GroupHeader => if item.id == "__pins__" || item.id.starts_with("stack:") {
                format!(" /:Filter{f}  C:Context  V:Volumes  N:Networks  Spc:Expand/Collapse  ↑/↓:Nav  q:Quit  ?:Help", f = filtered_status)
            } else {
                format!(" /:Filter{f}  C:Ctx H:Health V:Vol N:Net  L:Logs  Spc:Collapse  t:StartAll  R:RestartAll  q:Quit  ?:Help", f = filtered_status)
            }
        SidebarKind::SwarmService => format!(
            " /:Filter{f}  v:Select{v}  C:Context  V:Volumes  N:Networks  e:Shell  S:Scale  r:Restart  s:Stop  t:Start  d:Rm  i:Inspect  y:Copy  tab:Focus  q:Quit  ?:Help{scroll}",
            f = filtered_status, v = v_status
        ),
        SidebarKind::Separator => format!(" /:Filter{f}  C:Context  V:Volumes  N:Networks  q:Quit  ?:Help", f = filtered_status),
        SidebarKind::Image => format!(" /:Filter{f}  q:Quit  ?:Help", f = filtered_status),
    }
}
