pub mod actions;
pub mod navigation;
pub mod shortcuts;

use crate::ui::app::App;
use anyhow::Result;
use crossterm::event::{Event, KeyCode};

pub async fn handle_event(app: &mut App, ev: Event) -> Result<bool> {
    if let Event::Key(k) = ev {
        // 1. Check if it's a general global shortcut
        let should_quit = shortcuts::handle_shortcut(app, k.code, k.modifiers).await;
        if should_quit {
            return Ok(true);
        }

        // 2. Check if it's navigation (arrows, enter, page up/down)
        if matches!(
            k.code,
            KeyCode::Up
                | KeyCode::Down
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Enter
        ) && !app.is_filtering && !app.shell_active && app.popup.is_none() {
            navigation::handle_navigation(app, k.code).await;
            return Ok(false);
        }

        // 3. Check if it's a specific resource action ('t', 's', 'r', 'd', 'e', 'o', etc.)
        if let KeyCode::Char(c) = k.code {
            if !app.is_filtering && !app.shell_active && app.popup.is_none() && !k.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
                // If the shortcut module already handled it (like 'C', 'V', 'N', 'm', 'v', 'H', '?'), 
                // it might have changed app state (like opening a popup) but return false. 
                // We shouldn't process it as an action if it was meant to be a shortcut.
                // However, our action list ('t', 's', 'p', 'd', 'e', etc.) doesn't overlap with shortcuts.
                if !matches!(c, 'C' | 'V' | 'N' | 'm' | 'v' | ' ' | 'L' | 'H' | '?' | '/') {
                    actions::handle_action(app, c).await?;
                }
            }
        }
    } else if let Event::Mouse(m) = ev {
        navigation::handle_mouse(app, m.kind, m.column, m.row).await;
    }

    Ok(false)
}
