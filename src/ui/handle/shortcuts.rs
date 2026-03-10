use crate::ui::app::App;
use crate::ui::types::{SidebarKind, Popup};
use crate::docker;
use crossterm::event::{KeyCode, KeyModifiers, EnableMouseCapture, DisableMouseCapture};
use tokio::io::AsyncWriteExt;
use std::io;
pub async fn handle_shortcut(app: &mut App, k: KeyCode, modifiers: KeyModifiers) -> bool {
    // ── Integrated Shell Mode ──
    if app.shell_active {
        if k == KeyCode::Esc {
            app.stop_shell().await;
        } else {
            let data_to_send = match k {
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => Some("\x03".to_string()),
                KeyCode::Char(c) => Some(c.to_string()),
                KeyCode::Enter => Some("\r".to_string()),
                KeyCode::Backspace => Some("\x7f".to_string()),
                KeyCode::Tab => Some("\t".to_string()),
                KeyCode::Up => Some("\x1b[A".to_string()),
                KeyCode::Down => Some("\x1b[B".to_string()),
                KeyCode::Right => Some("\x1b[C".to_string()),
                KeyCode::Left => Some("\x1b[D".to_string()),
                _ => None,
            };

            // Update visual input buffer
            match k {
                KeyCode::Char(c) => { app.shell_input.push(c); }
                KeyCode::Backspace => { app.shell_input.pop(); }
                KeyCode::Enter => { app.shell_input.clear(); }
                _ => {}
            }

            if let Some(data) = data_to_send {
                let mut res = Ok(());
                if let Some(stdin) = app.shell_stdin.as_mut() {
                    res = stdin.write_all(data.as_bytes()).await;
                    if res.is_ok() {
                        let _ = stdin.flush().await;
                    }
                }
                if let Err(e) = res {
                    app.push_current_log(&format!("❌ Shell stdin error: {e}"));
                }
            }
        }
        return false;
    }

    // Global quit
    if (k == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL))
        || k == KeyCode::Char('q')
    {
        return true;
    }

    if k == KeyCode::Char('H') && app.popup.is_none() && !app.is_filtering {
        let _ = app.show_system_health().await;
        return false;
    }

    // Help popup
    if k == KeyCode::Char('?') && app.popup.is_none() && !app.is_filtering {
        app.popup = Some(Popup::Help);
        return false;
    }

    // ── Filtering Mode ──
    if app.is_filtering {
        match k {
            KeyCode::Enter | KeyCode::Esc => {
                app.is_filtering = false;
                if k == KeyCode::Esc {
                    app.filter_query.clear();
                    app.rebuild_items();
                }
            }
            KeyCode::Char(c) => {
                app.filter_query.push(c);
                app.rebuild_items();
            }
            KeyCode::Backspace => {
                app.filter_query.pop();
                app.rebuild_items();
            }
            _ => {}
        }
        return false;
    }

    // ── Log Filtering Mode ──
    if app.is_filtering_logs {
        match k {
            KeyCode::Enter | KeyCode::Esc => {
                app.is_filtering_logs = false;
                if k == KeyCode::Esc {
                    app.log_filter_query.clear();
                }
            }
            KeyCode::Char(c) => {
                app.log_filter_query.push(c);
            }
            KeyCode::Backspace => {
                app.log_filter_query.pop();
            }
            _ => {}
        }
        return false;
    }

    // / to start filtering
    if k == KeyCode::Char('/') && app.popup.is_none() {
        if app.focus_on_list {
            app.is_filtering = true;
        } else {
            app.is_filtering_logs = true;
        }
        return false;
    }


    // ── Copy mode ──
    if app.copy_mode {
        app.copy_mode = false;
        crossterm::execute!(io::stdout(), EnableMouseCapture).ok();
        return false;
    }

    // popup mode
    if let Some(p) = app.popup.clone() {
        match p {
            Popup::Inspect { id, name, json, tab } => {
                match k {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?') => {
                        app.popup = None;
                    }
                    KeyCode::Tab | KeyCode::Right => {
                        app.popup = Some(Popup::Inspect { id, name, json, tab: (tab + 1) % 3 });
                    }
                    KeyCode::Left => {
                        app.popup = Some(Popup::Inspect { id, name, json, tab: (tab + 2) % 3 });
                    }
                    _ => {}
                }
                return false;
            }
            Popup::SystemHealth { .. } => {
                match k {
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        app.popup = Some(Popup::ConfirmPrune);
                    }
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
                        app.popup = None;
                    }
                    _ => {}
                }
                return false;
            }
            Popup::ConfirmBulkRemove { ids } => {
                match k {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        app.popup = None;
                        app.push_current_log(&format!("🗑️ Removing {} containers...", ids.len()));
                        for id in ids {
                            let _ = crate::docker::container_rm_force(&app.docker, &app.cfg.cwd, &id).await;
                        }
                        let _ = app.refresh_containers().await;
                        app.rebuild_items();
                    }
                    KeyCode::Esc | KeyCode::Char('n') => {
                        app.popup = None;
                    }
                    _ => {}
                }
                return false;
            }
            Popup::ConfirmPrune => {
                match k {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        app.popup = None;
                        app.push_current_log("🧹 Pruning system...");
                        app.stats_refreshing = true;
                        let _ = app.trigger_prune().await;
                    }
                    KeyCode::Esc | KeyCode::Char('n') => {
                        app.popup = None;
                    }
                    _ => {}
                }
                return false;
            }
            Popup::Help => {
                if matches!(k, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?')) {
                    app.popup = None;
                }
                return false;
            }
            Popup::FileExplorer { id, name, path, files, selected } => {
                match k {
                    KeyCode::Up => {
                        let new_sel = if selected == 0 { files.len().saturating_sub(1) } else { selected - 1 };
                        app.popup = Some(Popup::FileExplorer { id, name, path, files, selected: new_sel });
                    }
                    KeyCode::Down => {
                        let new_sel = if files.is_empty() { 0 } else { (selected + 1) % files.len() };
                        app.popup = Some(Popup::FileExplorer { id, name, path, files, selected: new_sel });
                    }
                    KeyCode::Enter | KeyCode::Right => {
                        if let Some((fname, is_dir)) = files.get(selected) {
                            if *is_dir {
                                let new_path = if path == "/" { format!("/{}", fname) } else { format!("{}/{}", path, fname) };
                                if let Ok(new_files) = docker::list_container_files(&app.docker, &id, &new_path).await {
                                    app.popup = Some(Popup::FileExplorer { id, name, path: new_path, files: new_files, selected: 0 });
                                }
                            }
                        }
                    }
                    KeyCode::Backspace | KeyCode::Left => {
                        if path != "/" {
                            let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
                            let new_path = if parts.len() <= 1 { "/".to_string() } else { format!("/{}", parts[..parts.len()-1].join("/")) };
                            if let Ok(new_files) = docker::list_container_files(&app.docker, &id, &new_path).await {
                                app.popup = Some(Popup::FileExplorer { id, name, path: new_path, files: new_files, selected: 0 });
                            }
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {
                        app.popup = None;
                    }
                    _ => {}
                }
                return false;
            }
            Popup::Volumes { volumes, selected } => {
                match k {
                    KeyCode::Up => {
                        let new_sel = if selected == 0 { volumes.len().saturating_sub(1) } else { selected - 1 };
                        app.popup = Some(Popup::Volumes { volumes, selected: new_sel });
                    }
                    KeyCode::Down => {
                        let new_sel = if selected + 1 >= volumes.len() { 0 } else { selected + 1 };
                        app.popup = Some(Popup::Volumes { volumes, selected: new_sel });
                    }
                    KeyCode::Char('d') => {
                        if !volumes.is_empty() {
                            let vol = &volumes[selected];
                            app.push_current_log(&format!("🗑️ Removing volume {}...", vol.name));
                            if let Err(e) = docker::rm_volume(&app.docker, &app.cfg.cwd, &vol.name, false).await {
                                app.notify(format!("❌ Remove failed: {e}"), ratatui::style::Color::Red);
                            } else {
                                app.notify(format!("✅ Removed volume {}", vol.name), ratatui::style::Color::Green);
                                if let Ok(new_vols) = docker::get_volumes(&app.docker, &app.cfg.cwd).await {
                                    let new_sel = if selected >= new_vols.len() { new_vols.len().saturating_sub(1) } else { selected };
                                    app.popup = Some(Popup::Volumes { volumes: new_vols, selected: new_sel });
                                } else {
                                    app.popup = None;
                                }
                            }
                        }
                    }
                    KeyCode::Char('D') => {
                        if !volumes.is_empty() {
                            let vol = &volumes[selected];
                            app.push_current_log(&format!("🗑️ Force removing volume {}...", vol.name));
                            if let Err(e) = docker::rm_volume(&app.docker, &app.cfg.cwd, &vol.name, true).await {
                                app.notify(format!("❌ Force remove failed: {e}"), ratatui::style::Color::Red);
                            } else {
                                app.notify(format!("✅ Force removed volume {}", vol.name), ratatui::style::Color::Green);
                                if let Ok(new_vols) = docker::get_volumes(&app.docker, &app.cfg.cwd).await {
                                    let new_sel = if selected >= new_vols.len() { new_vols.len().saturating_sub(1) } else { selected };
                                    app.popup = Some(Popup::Volumes { volumes: new_vols, selected: new_sel });
                                } else {
                                    app.popup = None;
                                }
                            }
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
                        app.popup = None;
                    }
                    _ => {}
                }
                return false;
            }
            Popup::Networks { networks, selected } => {
                match k {
                    KeyCode::Up => {
                        let new_sel = if selected == 0 { networks.len().saturating_sub(1) } else { selected - 1 };
                        app.popup = Some(Popup::Networks { networks, selected: new_sel });
                    }
                    KeyCode::Down => {
                        let new_sel = if selected + 1 >= networks.len() { 0 } else { selected + 1 };
                        app.popup = Some(Popup::Networks { networks, selected: new_sel });
                    }
                    KeyCode::Char('d') => {
                        if !networks.is_empty() {
                            let net = &networks[selected];
                            app.push_current_log(&format!("🗑️ Removing network {}...", net.name));
                            if let Err(e) = docker::rm_network(&app.docker, &app.cfg.cwd, &net.id).await {
                                app.notify(format!("❌ Remove failed: {e}"), ratatui::style::Color::Red);
                            } else {
                                app.notify(format!("✅ Removed network {}", net.name), ratatui::style::Color::Green);
                                if let Ok(new_nets) = docker::get_networks(&app.docker, &app.cfg.cwd).await {
                                    let new_sel = if selected >= new_nets.len() { new_nets.len().saturating_sub(1) } else { selected };
                                    app.popup = Some(Popup::Networks { networks: new_nets, selected: new_sel });
                                } else {
                                    app.popup = None;
                                }
                            }
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
                        app.popup = None;
                    }
                    _ => {}
                }
                return false;
            }
            Popup::ConfirmReset { id, name } => {
                match k {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        app.popup = None;
                        app.push_current_log(&format!("🔥 RESETTING {name} (Stop+Rm+VolRm)..."));
                        let _ = docker::reset_container(&app.docker, &app.cfg.cwd, &id).await.map(|msgs| {
                            for m in msgs {
                                app.push_current_log(&m);
                            }
                        });
                        let _ = app.refresh_containers().await;
                        app.rebuild_items();
                        let _ = app.select(app.selected).await;
                    }
                    KeyCode::Esc | KeyCode::Char('n') => {
                        app.popup = None;
                    }
                    _ => {}
                }
                return false;
            }
            Popup::ConfirmComposeRestart { infra_running } => {
                match k {
                    KeyCode::Char('r') | KeyCode::Enter => {
                        app.popup = None;
                        app.compose_up_or_restart(infra_running).await;
                    }
                    KeyCode::Char('k') => {
                        app.popup = None;
                    }
                    KeyCode::Esc => {
                        app.popup = None;
                    }
                    _ => {}
                }
                return false;
            }
            Popup::ScaleService { id, name, current, mut input } => {
                match k {
                    KeyCode::Enter => {
                        let replicas: u64 = input.parse().unwrap_or(current);
                        app.popup = None;
                        app.push_current_log(&format!("⚖️ Scaling service {name} to {replicas}..."));
                        if let Err(e) = docker::service_scale(&app.docker, &app.cfg.cwd, &id, replicas as usize).await {
                            app.push_current_log(&format!("❌ Scaling failed: {e}"));
                        }
                        let _ = app.refresh_swarm().await;
                        app.rebuild_items();
                    }
                    KeyCode::Char(c) if c.is_digit(10) => {
                        input.push(c);
                        app.popup = Some(Popup::ScaleService { id, name, current, input });
                    }
                    KeyCode::Backspace => {
                        input.pop();
                        app.popup = Some(Popup::ScaleService { id, name, current, input });
                    }
                    KeyCode::Esc => {
                        app.popup = None;
                    }
                    _ => {}
                }
                return false;
            }
            Popup::ContextSwitch { contexts, selected } => {
                match k {
                    KeyCode::Up => {
                        let new_sel = if selected == 0 { contexts.len().saturating_sub(1) } else { selected - 1 };
                        app.popup = Some(Popup::ContextSwitch { contexts, selected: new_sel });
                    }
                    KeyCode::Down => {
                        let new_sel = if selected + 1 >= contexts.len() { 0 } else { selected + 1 };
                        app.popup = Some(Popup::ContextSwitch { contexts, selected: new_sel });
                    }
                    KeyCode::Enter => {
                        let ctx_name = contexts[selected].name.clone();
                        app.popup = None;
                        app.push_current_log(&format!("🔌 Switching to Docker context: {}...", ctx_name));
                        let _ = app.switch_context_and_refresh(ctx_name).await;
                    }
                    KeyCode::Esc => {
                        app.popup = None;
                    }
                    _ => {}
                }
                return false;
            }
            Popup::ImageExplorer { images, selected } => {
                match k {
                    KeyCode::Up => {
                        let new_sel = if selected == 0 { images.len().saturating_sub(1) } else { selected - 1 };
                        app.popup = Some(Popup::ImageExplorer { images, selected: new_sel });
                    }
                    KeyCode::Down => {
                        let new_sel = if selected + 1 >= images.len() { 0 } else { selected + 1 };
                        app.popup = Some(Popup::ImageExplorer { images, selected: new_sel });
                    }
                    KeyCode::Char('d') => {
                        if !images.is_empty() {
                            let img = &images[selected];
                            app.push_current_log(&format!("🗑️ Removing image {}...", img.repository));
                            if let Err(e) = docker::rm_image(&app.docker, &app.cfg.cwd, &img.id, false).await {
                                app.notify(format!("❌ Remove failed: {e}"), ratatui::style::Color::Red);
                            } else {
                                app.notify(format!("✅ Removed image {}", img.repository), ratatui::style::Color::Green);
                                if let Ok(new_images) = docker::get_images(&app.docker, &app.cfg.cwd).await {
                                    let new_sel = if selected >= new_images.len() { new_images.len().saturating_sub(1) } else { selected };
                                    app.popup = Some(Popup::ImageExplorer { images: new_images, selected: new_sel });
                                } else {
                                    app.popup = None;
                                }
                            }
                        }
                    }
                    KeyCode::Char('D') => {
                        if !images.is_empty() {
                            let img = &images[selected];
                            app.push_current_log(&format!("🗑️ Force removing image {}...", img.repository));
                            if let Err(e) = docker::rm_image(&app.docker, &app.cfg.cwd, &img.id, true).await {
                                app.notify(format!("❌ Force remove failed: {e}"), ratatui::style::Color::Red);
                            } else {
                                app.notify(format!("✅ Force removed image {}", img.repository), ratatui::style::Color::Green);
                                if let Ok(new_images) = docker::get_images(&app.docker, &app.cfg.cwd).await {
                                    let new_sel = if selected >= new_images.len() { new_images.len().saturating_sub(1) } else { selected };
                                    app.popup = Some(Popup::ImageExplorer { images: new_images, selected: new_sel });
                                } else {
                                    app.popup = None;
                                }
                            }
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
                        app.popup = None;
                    }
                    _ => {}
                }
                return false;
            }
        }
    }

    // I: Image Explorer
    if k == KeyCode::Char('I') && modifiers.contains(KeyModifiers::SHIFT) && app.popup.is_none() && !app.is_filtering {
        if let Ok(images) = docker::get_images(&app.docker, &app.cfg.cwd).await {
            app.popup = Some(Popup::ImageExplorer { images, selected: 0 });
        } else {
            app.notify("❌ Failed to fetch images".to_string(), ratatui::style::Color::Red);
        }
        return false;
    }

    // V: Volumes Explorer
    if k == KeyCode::Char('V') && modifiers.contains(KeyModifiers::SHIFT) && app.popup.is_none() && !app.is_filtering {
        if let Ok(volumes) = docker::get_volumes(&app.docker, &app.cfg.cwd).await {
            app.popup = Some(Popup::Volumes { volumes, selected: 0 });
        } else {
            app.notify("❌ Failed to fetch volumes".to_string(), ratatui::style::Color::Red);
        }
        return false;
    }

    // N: Networks Explorer
    if k == KeyCode::Char('N') && modifiers.contains(KeyModifiers::SHIFT) && app.popup.is_none() && !app.is_filtering {
        if let Ok(networks) = docker::get_networks(&app.docker, &app.cfg.cwd).await {
            app.popup = Some(Popup::Networks { networks, selected: 0 });
        } else {
            app.notify("❌ Failed to fetch networks".to_string(), ratatui::style::Color::Red);
        }
        return false;
    }

    // s: Toggle sort cycle (Name -> Status -> Id)
    if k == KeyCode::Char('s') && app.popup.is_none() && !app.is_filtering {
        let next = match app.sort_by {
            crate::ui::app::SortBy::Name => crate::ui::app::SortBy::Status,
            crate::ui::app::SortBy::Status => crate::ui::app::SortBy::Id,
            crate::ui::app::SortBy::Id => crate::ui::app::SortBy::Name,
            _ => crate::ui::app::SortBy::Name,
        };
        app.toggle_sort(next);
        app.rebuild_items();
        let order = if app.sort_order == crate::ui::app::SortOrder::Asc { "↑" } else { "↓" };
        app.notify(format!("🔃 Sort: {:?} {}", app.sort_by, order), ratatui::style::Color::Blue);
        return false;
    }

    // p: Sort by Project
    if k == KeyCode::Char('p') && app.popup.is_none() && !app.is_filtering {
        app.toggle_sort(crate::ui::app::SortBy::Project);
        app.rebuild_items();
        let order = if app.sort_order == crate::ui::app::SortOrder::Asc { "↑" } else { "↓" };
        app.notify(format!("📂 Sort: Project {}", order), ratatui::style::Color::Blue);
        return false;
    }

    // Context Switch
    if k == KeyCode::Char('C') && app.popup.is_none() && !app.is_filtering {
        let contexts = docker::list_contexts(&app.docker, &app.cfg.cwd).await;
        let current_idx = contexts.iter().position(|c| c.current).unwrap_or(0);
        app.popup = Some(Popup::ContextSwitch { contexts, selected: current_idx });
        return false;
    }

    // Bulk Actions (Shift+S, Shift+X, Shift+D)
    if app.popup.is_none() && !app.is_filtering && !app.multi_selected.is_empty() {
        match k {
            KeyCode::Char('S') => {
                let ids: Vec<String> = app.multi_selected.iter().cloned().collect();
                app.notify(format!("🚀 Bulk Start: {} items", ids.len()), ratatui::style::Color::Cyan);
                for id in ids {
                    let _ = docker::container_action(&app.docker, &app.cfg.cwd, "start", &id).await;
                }
                let _ = app.refresh_containers().await;
                app.rebuild_items();
                return false;
            }
            KeyCode::Char('X') => {
                let ids: Vec<String> = app.multi_selected.iter().cloned().collect();
                app.notify(format!("🛑 Bulk Stop: {} items", ids.len()), ratatui::style::Color::Yellow);
                for id in ids {
                    let _ = docker::container_action(&app.docker, &app.cfg.cwd, "stop", &id).await;
                }
                let _ = app.refresh_containers().await;
                app.rebuild_items();
                return false;
            }
            KeyCode::Char('D') => {
                app.popup = Some(Popup::ConfirmBulkRemove { ids: app.multi_selected.iter().cloned().collect() });
                return false;
            }
            _ => {}
        }
    }

    // Shift+F: Container File Explorer
    if k == KeyCode::Char('F') && app.focus_on_list && app.popup.is_none() {
        if let Some(it) = app.items.get(app.selected) {
            if it.kind == SidebarKind::Container {
                if let Ok(files) = docker::list_container_files(&app.docker, &it.id, "/").await {
                    app.popup = Some(Popup::FileExplorer { id: it.id.clone(), name: it.name.clone(), path: "/".to_string(), files, selected: 0 });
                }
                return false;
            }
        }
    }

    // focus toggle
    if k == KeyCode::Tab {
        app.set_focus(!app.focus_on_list);
        return false;
    }

    // m: enter copy mode
    if k == KeyCode::Char('m') {
        app.copy_mode = true;
        crossterm::execute!(io::stdout(), DisableMouseCapture).ok();
        return false;
    }

    // v: toggle multi-select
    if k == KeyCode::Char('v') && app.focus_on_list && app.popup.is_none() {
        if !app.items.is_empty() {
            let id = app.items[app.selected].id.clone();
            let kind = &app.items[app.selected].kind;
            if *kind == SidebarKind::Container || *kind == SidebarKind::SwarmService || *kind == SidebarKind::Task {
                app.toggle_select(&id);
                app.rebuild_items();
            }
        }
        return false;
    }

    // f: toggle follow mode
    if k == KeyCode::Char('f') && app.popup.is_none() && !app.is_filtering {
        app.follow_mode = !app.follow_mode;
        if app.follow_mode {
            app.stick_to_bottom = true;
            app.notify("▶️ Follow mode: ON".to_string(), ratatui::style::Color::Green);
        } else {
            app.stick_to_bottom = false;
            app.notify("⏸️ Follow mode: OFF".to_string(), ratatui::style::Color::Yellow);
        }
        return false;
    }

    // Space: toggle group collapse
    if k == KeyCode::Char(' ') && app.focus_on_list {
        if !app.items.is_empty() {
            let item = app.items[app.selected].clone();
            if item.kind == SidebarKind::GroupHeader {
                let project = item.id.clone();
                app.toggle_group_collapse(&project);
                app.rebuild_items();
                return false;
            }
        }
    }

    // L: Multi-container log streaming
    if k == KeyCode::Char('L') && app.focus_on_list {
            if !app.items.is_empty() {
            let item = app.items[app.selected].clone();
            if item.kind == SidebarKind::GroupHeader && !item.id.starts_with("stack:") && item.id != "__pins__" {
                let _ = app.start_compose_logs(item.id.clone());
                return false;
            }
        }
    }

    // E: Export logs
    if k == KeyCode::Char('E') && app.popup.is_none() && !app.is_filtering {
        match app.export_logs().await {
            Ok(file) => app.notify(format!("📂 Logs exported to {file}"), ratatui::style::Color::Cyan),
            Err(e) => app.notify(format!("❌ Export failed: {e}"), ratatui::style::Color::Red),
        }
        return false;
    }

    false
}
