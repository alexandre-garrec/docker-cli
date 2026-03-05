use crate::ui::app::App;
use crate::ui::types::{SidebarKind, Popup};
use crate::docker;
use crossterm::event::{KeyCode, KeyModifiers, EnableMouseCapture, DisableMouseCapture};
use tokio::io::AsyncWriteExt;
use std::io;

pub async fn handle_shortcut(app: &mut App, k: KeyCode, modifiers: KeyModifiers) -> bool {
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

    // ── Integrated Shell Mode ──
    if app.shell_active {
        if k == KeyCode::Esc {
            app.stop_shell().await;
        } else {
            let mut data_to_send = None;
            
            match k {
                KeyCode::Char(c) => {
                    app.shell_input.push(c);
                }
                KeyCode::Enter => {
                    let cmd = app.shell_input.clone();
                    app.push_partial_log(&format!("❯ {}\n", cmd));
                    data_to_send = Some(cmd + "\n");
                    app.shell_input.clear();
                }
                KeyCode::Backspace => {
                    app.shell_input.pop();
                }
                KeyCode::Tab => {
                    app.shell_input.push('\t');
                }
                _ => {}
            }

            if let Some(data) = data_to_send {
                if let Some(stdin) = app.shell_stdin.as_mut() {
                    let _ = stdin.write_all(data.as_bytes()).await;
                    let _ = stdin.flush().await;
                }
            }
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
            Popup::Inspect { .. } | Popup::Help => {
                if matches!(k, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?')) {
                    app.popup = None;
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
            Popup::ConfirmPrune => {
                match k {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        let _ = app.trigger_prune().await;
                    }
                    KeyCode::Esc | KeyCode::Char('n') => {
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

    // C: switch context
    if k == KeyCode::Char('C') && app.popup.is_none() && !app.is_filtering {
        let contexts = docker::list_contexts(&app.docker, &app.cfg.cwd).await;
        let current_idx = contexts.iter().position(|c| c.current).unwrap_or(0);
        app.popup = Some(Popup::ContextSwitch { contexts, selected: current_idx });
        return false;
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

    false
}
