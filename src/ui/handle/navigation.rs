use crate::ui::app::App;
use crate::ui::types::SidebarKind;
use crossterm::event::{KeyCode, MouseEventKind};

pub async fn handle_navigation(app: &mut App, k: KeyCode) {
    if app.focus_on_list {
        match k {
            KeyCode::Up => {
                let mut next = app.selected;
                loop {
                    if next == 0 { break; }
                    next -= 1;
                    if app.items.get(next).map(|i| i.kind != SidebarKind::Separator).unwrap_or(false) {
                        app.selected = next;
                        break;
                    }
                }
                let _ = app.select(app.selected).await;
            }
            KeyCode::Down => {
                let mut next = app.selected;
                loop {
                    next += 1;
                    if next >= app.items.len() { break; }
                    if app.items.get(next).map(|i| i.kind != SidebarKind::Separator).unwrap_or(false) {
                        app.selected = next;
                        break;
                    }
                }
                let _ = app.select(app.selected).await;
            }
            KeyCode::Enter => {
                let _ = app.select(app.selected).await;
            }
            _ => {}
        }
    } else {
        // Logs focus navigation
        let height = app.last_log_height;
        match k {
            KeyCode::Up => {
                app.stick_to_bottom = false;
                app.log_scroll = app.log_scroll.saturating_sub(1);
            }
            KeyCode::Down => {
                app.log_scroll = app.log_scroll.saturating_add(1);
                if app.log_scroll + height >= app.log_lines.len() as u16 {
                    app.stick_to_bottom = true;
                }
            }
            KeyCode::PageUp => {
                app.stick_to_bottom = false;
                app.log_scroll = app.log_scroll.saturating_sub(height);
            }
            KeyCode::PageDown => {
                app.log_scroll = app.log_scroll.saturating_add(height);
                if app.log_scroll + height >= app.log_lines.len() as u16 {
                    app.stick_to_bottom = true;
                }
            }
            KeyCode::Home => {
                app.stick_to_bottom = false;
                app.log_scroll = 0;
            }
            KeyCode::End => {
                app.stick_to_bottom = true;
            }
            _ => {}
        }
    }
}

pub async fn handle_mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
    let height = app.last_log_height;
    match kind {
        MouseEventKind::ScrollDown => {
            app.log_scroll = app.log_scroll.saturating_add(3);
            if app.log_scroll + height >= app.log_lines.len() as u16 {
                app.stick_to_bottom = true;
            }
        }
        MouseEventKind::ScrollUp => {
            app.stick_to_bottom = false;
            app.log_scroll = app.log_scroll.saturating_sub(3);
        }
        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
            let area = app.list_area;
            if column >= area.x && column < area.x + area.width && row >= area.y && row < area.y + area.height {
                if row > area.y && row < area.y + area.height - 1 {
                    let relative_row = row - area.y - 1;
                    let index = app.list_state.offset() + relative_row as usize;
                    if index < app.items.len() {
                        let _ = app.select(index).await;
                    }
                }
                app.set_focus(true);
            } else {
                app.set_focus(false);
            }
        }
        _ => {}
    }
}
