pub mod types;
pub mod app;
pub mod draw;
pub mod handle;

pub use app::{App, RunOpts};

use crate::docker;
use crate::env;
use crate::config::{get_config};
use crate::ui::draw::draw_ui;
use crate::ui::handle::handle_event;
use anyhow::Result;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, Stdout};
use tokio::sync::mpsc;
use tokio::time::{self, Duration};

pub async fn run(opts: RunOpts) -> Result<()> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let chosen = std::env::var("DOCKER_PROFILE")
        .ok()
        .or_else(|| std::env::var("COMPOSE_PROFILE").ok())
        .unwrap_or_else(|| "local".to_string());

    std::env::set_var("DOCKER_PROFILE", &chosen);
    std::env::set_var("COMPOSE_PROFILES", &chosen);
    env::load_env(&opts.root, Some(&chosen))?;

    let cfg = get_config(&chosen);
    let mut app = App::new(cfg, opts.docker_meta);

    if app.docker.available {
        let _ = app.refresh_containers().await;
        app.rebuild_items();
        let _ = app.select(0).await;
        
        // Suppress auto-popup if we are in screenshot mode
        let screenshot_file = std::fs::read_to_string("screenshot_backdoor.txt").is_ok();
        let screenshot_env = std::env::var("SCREENSHOT_VIEW").is_ok();
        
        if app.cfg.auto_compose_up && !screenshot_file && !screenshot_env {
            let infra_running = app.infra_already_up();
            app.popup = Some(types::Popup::ConfirmComposeRestart { infra_running });
        }
    } else {
        app.rebuild_items();
    }

    if let Ok(view) = std::fs::read_to_string("screenshot_backdoor.txt") {
        let view = view.trim();
        match view {
            "health" => {
                if let Ok(df) = docker::get_system_df(&app.docker, &app.cfg.cwd).await {
                    app.popup = Some(types::Popup::SystemHealth { data: df });
                }
            }
            "images" => {
                if let Ok(imgs) = docker::get_images(&app.docker, &app.cfg.cwd).await {
                    app.popup = Some(types::Popup::ImageExplorer { images: imgs, selected: 0 });
                }
            }
            "volumes" => {
                if let Ok(vols) = docker::get_volumes(&app.docker, &app.cfg.cwd).await {
                    app.popup = Some(types::Popup::Volumes { volumes: vols, selected: 0 });
                }
            }
            "networks" => {
                if let Ok(nets) = docker::get_networks(&app.docker, &app.cfg.cwd).await {
                    app.popup = Some(types::Popup::Networks { networks: nets, selected: 0 });
                }
            }
            _ => {}
        }
    }

    let mut ticker = time::interval(Duration::from_millis(app.cfg.refresh_ms));



    let mut stats_ticker = time::interval(Duration::from_secs(2));
    let (tx_refresh, mut rx_refresh) = mpsc::unbounded_channel();
    let (tx_swarm, mut rx_swarm) = mpsc::unbounded_channel::<Vec<docker::SwarmService>>();
    let (tx_stats, mut rx_stats) = mpsc::unbounded_channel::<Option<docker::ContainerStats>>();

    loop {
        terminal.draw(|f| draw_ui(f, &mut app))?;

        app.pump_background().await;

        tokio::select! {
            _ = ticker.tick() => {
                if app.docker.available && app.popup.is_none() {
                    if !app.refreshing {
                        app.refreshing = true;
                        let tx = tx_refresh.clone();
                        let docker = app.docker.clone();
                        let cwd = app.cfg.cwd.clone();
                        tokio::spawn(async move {
                            let res = docker::list_containers_all(&docker, &cwd).await;
                            let _ = tx.send(res);
                        });
                    }
                    if !app.swarm_refreshing {
                        app.swarm_refreshing = true;
                        let tx = tx_swarm.clone();
                        let docker = app.docker.clone();
                        let cwd = app.cfg.cwd.clone();
                        tokio::spawn(async move {
                            let svcs = docker::list_swarm_services(&docker, &cwd).await;
                            let _ = tx.send(svcs);
                        });
                    }
                }
            }
            _ = stats_ticker.tick() => {
                if app.docker.available && !app.items.is_empty() && !app.stats_refreshing {
                    let item = app.items[app.selected].clone();
                    if item.kind == types::SidebarKind::Container {
                        app.stats_refreshing = true;
                        let tx = tx_stats.clone();
                        let docker = app.docker.clone();
                        let cwd = app.cfg.cwd.clone();
                        let id = item.id.clone();
                        tokio::spawn(async move {
                            let s = docker::fetch_stats(&docker, &cwd, &id).await.ok();
                            let _ = tx.send(s);
                        });
                    }
                }
            }
            Some(res) = rx_refresh.recv() => {
                app.refreshing = false;
                if let Ok(containers) = res {
                    app.containers = containers;
                    app.rebuild_items();
                }
            }
            Some(svcs) = rx_swarm.recv() => {
                app.swarm_refreshing = false;
                app.swarm_services = svcs;
                app.rebuild_items();
            }
            Some(stats) = rx_stats.recv() => {
                app.stats_refreshing = false;
                app.container_stats = stats.clone();
                if let Some(s) = stats {
                    if !app.items.is_empty() {
                        let item = &app.items[app.selected];
                        if item.kind == crate::ui::types::SidebarKind::Container {
                            let history = app.stats_history.entry(item.id.clone()).or_default();
                            history.push_back((s.cpu_percent, s.mem_usage_mb));
                            if history.len() > 50 {
                                history.pop_front();
                            }
                        }
                    }
                }
                app.rebuild_items();
            }
            ev = read_event() => {
                if let Some(ev) = ev {
                    let should_quit = handle_event(&mut app, ev).await?;
                    if should_quit { break; }
                }
            }
        }
    }

    disable_raw_mode()?;
    let mut stdout: Stdout = io::stdout();
    stdout.execute(DisableMouseCapture)?;
    stdout.execute(LeaveAlternateScreen)?;
    Ok(())
}

async fn read_event() -> Option<Event> {
    if event::poll(Duration::from_millis(50)).ok()? {
        event::read().ok()
    } else {
        None
    }
}
