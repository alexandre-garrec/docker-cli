use crate::ui::app::App;
use crate::ui::types::{SidebarKind, Popup};
use crate::docker;
use crate::pins;
use anyhow::Result;
use arboard::Clipboard;

pub async fn handle_action(app: &mut App, c: char) -> Result<()> {
    match c {
        'o' => {
            app.open_selected_in_browser().await;
        }
        'c' => {
            if app.docker.available {
                app.popup = Some(Popup::ConfirmComposeRestart { infra_running: app.infra_already_up() });
            }
        }
        'i' => {
            if app.items.is_empty() { return Ok(()); }
            let item = app.items[app.selected].clone();
            match item.kind {
                SidebarKind::Container if app.docker.available => {
                    if let Ok(v) = docker::container_inspect(&app.docker, &app.cfg.cwd, &item.id).await {
                        let content = docker::format_container_info(&v);
                        app.popup = Some(Popup::Inspect { title: format!("Inspect: {}", item.name), content });
                    }
                }
                SidebarKind::SwarmService if app.docker.available => {
                    let out = docker::cmd_inspect_service(&app.docker, &app.cfg.cwd, &item.id).await
                        .unwrap_or_else(|e| format!("Error: {e}"));
                    app.popup = Some(Popup::Inspect { title: format!("Service: {}", item.name), content: out });
                }
                _ => {}
            }
        }
        'x' => {
            if app.items.is_empty() { return Ok(()); }
            let item = app.items[app.selected].clone();
            if item.kind == SidebarKind::Container && app.docker.available {
                app.popup = Some(Popup::ConfirmReset { id: item.id, name: item.name });
            }
        }
        'r' => {
            if app.items.is_empty() { return Ok(()); }
            let ids = if !app.multi_selected.is_empty() {
                app.multi_selected.iter().cloned().collect::<Vec<_>>()
            } else {
                vec![app.items[app.selected].id.clone()]
            };

            for id in ids {
                if let Some(item) = app.items.iter().find(|i| i.id == id).cloned() {
                    match item.kind {
                        SidebarKind::Task => {
                            let _ = app.run_task(&item.id).await;
                        }
                        SidebarKind::Container if app.docker.available => {
                            app.push_current_log(&format!("Restarting container {}...", item.name));
                            if let Err(e) = docker::container_action(&app.docker, &app.cfg.cwd, "restart", &item.id).await {
                                app.notify(format!("❌ Restart failed: {e}"), ratatui::style::Color::Red);
                            } else {
                                app.notify(format!("✅ Restarted {}", item.name), ratatui::style::Color::Green);
                            }
                        }
                        SidebarKind::SwarmService if app.docker.available => {
                            app.push_current_log(&format!("Rolling restart service {}...", item.name));
                            if let Err(e) = docker::service_rolling_restart(&app.docker, &app.cfg.cwd, &item.id).await {
                                app.notify(format!("❌ Rolling restart failed: {e}"), ratatui::style::Color::Red);
                            } else {
                                app.notify(format!("✅ Rolling restart issued for {}", item.name), ratatui::style::Color::Green);
                            }
                        }
                        _ => {}
                    }
                }
            }
            app.multi_selected.clear();
            let _ = app.refresh_containers().await;
            let _ = app.refresh_swarm().await;
            app.rebuild_items();
            let _ = app.select(app.selected).await;
        }
        's' => {
            if app.items.is_empty() { return Ok(()); }
            let ids = if !app.multi_selected.is_empty() {
                app.multi_selected.iter().cloned().collect::<Vec<_>>()
            } else {
                vec![app.items[app.selected].id.clone()]
            };

            for id in ids {
                if let Some(item) = app.items.iter().find(|i| i.id == id).cloned() {
                    match item.kind {
                        SidebarKind::Task => {
                            app.stop_task(&item.id).await;
                        }
                        SidebarKind::Container if app.docker.available => {
                            app.push_current_log(&format!("Stopping container {}...", item.name));
                            if let Err(e) = docker::container_action(&app.docker, &app.cfg.cwd, "stop", &item.id).await {
                                app.notify(format!("❌ Stop failed: {e}"), ratatui::style::Color::Red);
                            } else {
                                app.notify(format!("🛑 Stopped {}", item.name), ratatui::style::Color::Yellow);
                            }
                        }
                        SidebarKind::SwarmService if app.docker.available => {
                            app.push_current_log(&format!("Stopping Swarm service {} (scaling to 0)...", item.name));
                            if let Err(e) = docker::service_scale(&app.docker, &app.cfg.cwd, &item.id, 0).await {
                                app.notify(format!("❌ Stop failed: {e}"), ratatui::style::Color::Red);
                            } else {
                                app.notify(format!("🛑 Stopped service {}", item.name), ratatui::style::Color::Yellow);
                            }
                        }
                        _ => {}
                    }
                }
            }
            app.multi_selected.clear();
            let _ = app.refresh_containers().await;
            let _ = app.refresh_swarm().await;
            app.rebuild_items();
            let _ = app.select(app.selected).await;
        }
        't' => {
            if app.items.is_empty() { return Ok(()); }
            let ids = if !app.multi_selected.is_empty() {
                app.multi_selected.iter().cloned().collect::<Vec<_>>()
            } else {
                vec![app.items[app.selected].id.clone()]
            };

            for id in ids {
                if let Some(item) = app.items.iter().find(|i| i.id == id).cloned() {
                    match item.kind {
                        SidebarKind::Task => {
                            let _ = app.run_task(&item.id).await;
                        }
                        SidebarKind::Container if app.docker.available => {
                            app.push_current_log(&format!("Starting container {}...", item.name));
                            if let Err(e) = docker::container_action(&app.docker, &app.cfg.cwd, "start", &item.id).await {
                                app.notify(format!("❌ Start failed: {e}"), ratatui::style::Color::Red);
                            } else {
                                app.notify(format!("🚀 Started {}", item.name), ratatui::style::Color::Green);
                            }
                        }
                        SidebarKind::SwarmService if app.docker.available => {
                            app.push_current_log(&format!("Starting Swarm service {} (scaling to 1)...", item.name));
                            if let Err(e) = docker::service_scale(&app.docker, &app.cfg.cwd, &item.id, 1).await {
                                app.notify(format!("❌ Start failed: {e}"), ratatui::style::Color::Red);
                            } else {
                                app.notify(format!("🚀 Started service {}", item.name), ratatui::style::Color::Green);
                            }
                        }
                        SidebarKind::GroupHeader if item.id != "__pins__" && !item.id.starts_with("stack:") && app.docker.available => {
                            let project = item.id.clone();
                            app.push_current_log(&format!("🚀 Starting compose project {}...", project));
                            match docker::compose_group_up(&app.docker, &app.cfg.cwd, &project).await {
                                Ok(lines) => {
                                    for l in lines { app.push_current_log(&l); }
                                    app.notify(format!("✅ Compose up done: {}", project), ratatui::style::Color::Green);
                                }
                                Err(e) => app.notify(format!("❌ Start failed: {e}"), ratatui::style::Color::Red),
                            }
                        }
                        _ => {}
                    }
                }
            }
            app.multi_selected.clear();
            let _ = app.refresh_containers().await;
            let _ = app.refresh_swarm().await;
            app.rebuild_items();
            let _ = app.select(app.selected).await;
        }
        'S' => {
            if app.items.is_empty() { return Ok(()); }
            let item = app.items[app.selected].clone();
            if item.kind == SidebarKind::SwarmService && app.docker.available {
                let current = app.swarm_services.iter()
                    .find(|s| s.id == item.id)
                    .and_then(|s| s.replicas.split('/').next())
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .unwrap_or(0);
                app.popup = Some(Popup::ScaleService { id: item.id, name: item.name, current, input: current.to_string() });
            }
        }
        'p' => {
            if app.items.is_empty() { return Ok(()); }
            let item = app.items[app.selected].clone();
            if item.kind == SidebarKind::Container && app.docker.available {
                app.push_current_log(&format!("Pausing container {}...", item.name));
                if let Err(e) = docker::container_action(&app.docker, &app.cfg.cwd, "pause", &item.id).await {
                    app.push_current_log(&format!("❌ Pause failed: {e}"));
                }
                let _ = app.refresh_containers().await;
                app.rebuild_items();
            }
        }
        'u' => {
            if app.items.is_empty() { return Ok(()); }
            let item = app.items[app.selected].clone();
            if item.kind == SidebarKind::Container && app.docker.available {
                app.push_current_log(&format!("Unpausing container {}...", item.name));
                if let Err(e) = docker::container_action(&app.docker, &app.cfg.cwd, "unpause", &item.id).await {
                    app.push_current_log(&format!("❌ Unpause failed: {e}"));
                }
                let _ = app.refresh_containers().await;
                app.rebuild_items();
            }
        }
        'k' => {
            if app.items.is_empty() { return Ok(()); }
            let item = app.items[app.selected].clone();
            if item.kind == SidebarKind::Container && app.docker.available {
                app.push_current_log(&format!("Killing container {}...", item.name));
                if let Err(e) = docker::container_action(&app.docker, &app.cfg.cwd, "kill", &item.id).await {
                    app.push_current_log(&format!("❌ Kill failed: {e}"));
                }
                let _ = app.refresh_containers().await;
                app.rebuild_items();
            }
        }
        'd' => {
            if app.items.is_empty() { return Ok(()); }
            let ids = if !app.multi_selected.is_empty() {
                app.multi_selected.iter().cloned().collect::<Vec<_>>()
            } else {
                vec![app.items[app.selected].id.clone()]
            };

            for id in ids {
                if let Some(item) = app.items.iter().find(|i| i.id == id).cloned() {
                    if app.docker.available {
                        match item.kind {
                            SidebarKind::Container => {
                                app.push_current_log(&format!("Removing container {} (force)...", item.name));
                                if let Err(e) = docker::container_rm_force(&app.docker, &app.cfg.cwd, &item.id).await {
                                    app.notify(format!("❌ Remove failed: {e}"), ratatui::style::Color::Red);
                                } else {
                                    app.notify(format!("🗑️ Removed {}", item.name), ratatui::style::Color::Green);
                                }
                            }
                            SidebarKind::SwarmService => {
                                app.push_current_log(&format!("Removing Swarm service {}...", item.name));
                                if let Err(e) = docker::service_rm(&app.docker, &app.cfg.cwd, &item.id).await {
                                    app.notify(format!("❌ Remove failed: {e}"), ratatui::style::Color::Red);
                                } else {
                                    app.notify(format!("🗑️ Removed service {}", item.name), ratatui::style::Color::Green);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            app.multi_selected.clear();
            let _ = app.refresh_containers().await;
            let _ = app.refresh_swarm().await;
            app.rebuild_items();
            let _ = app.select(app.selected).await;
        }
        'y' => {
            let text = app.log_lines.iter().cloned().collect::<Vec<_>>().join("\n");
            match Clipboard::new().and_then(|mut cb| { cb.set_text(text)?; Ok(()) }) {
                Ok(()) => app.push_current_log("📋 Logs copied to clipboard."),
                Err(_)  => app.push_current_log("⚠️  Clipboard unavailable."),
            }
        }
        'e' => {
            if app.items.is_empty() { return Ok(()); }
            let item = app.items[app.selected].clone();
            if (item.kind == SidebarKind::Container || item.kind == SidebarKind::SwarmService) && app.docker.available {
                app.start_shell(&item.id, item.kind).await?;
            }
        }
        'P' => {
            if app.items.is_empty() { return Ok(()); }
            let item = app.items[app.selected].clone();
            if item.kind == SidebarKind::Container {
                if app.pins.contains(&item.name) {
                    app.pins.remove(&item.name);
                    app.push_current_log(&format!("📌 Unpinned {}.", item.name));
                } else {
                    app.pins.insert(item.name.clone());
                    app.push_current_log(&format!("📌 Pinned {}.", item.name));
                }
                pins::save_pins(&app.pins);
                app.rebuild_items();
            }
        }
        'R' => {
            if app.items.is_empty() { return Ok(()); }
            let item = app.items[app.selected].clone();
            if item.kind == SidebarKind::GroupHeader && item.id != "__pins__" && !item.id.starts_with("stack:") && app.docker.available {
                let project = item.id.clone();
                app.push_current_log(&format!("🔄 Restarting compose project {}...", project));
                match docker::compose_group_restart(&app.docker, &app.cfg.cwd, &project).await {
                    Ok(lines) => {
                        for l in lines { app.push_current_log(&l); }
                        app.push_current_log("✅ Compose restart done.");
                    }
                    Err(e) => app.push_current_log(&format!("❌ Restart failed: {e}")),
                }
            }
        }
        _ => {}
    }
    Ok(())
}
