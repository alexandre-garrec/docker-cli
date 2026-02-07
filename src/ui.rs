use crate::config::{get_config, Config, TaskSpec};
use crate::docker;
use crate::env;
use crate::tasks::{self, TaskStatus};
use anyhow::Result;
use chrono::Local;
use crossterm::event::{self, Event, KeyCode, KeyModifiers, MouseEvent, MouseEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Stdout};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::time::{self, Duration};

#[derive(Debug, Clone)]
pub struct RunOpts {
    pub root: PathBuf,
    #[allow(dead_code)]
    pub docker_bin: String,
    pub docker_meta: docker::DockerMeta,
}

#[derive(Debug, Clone)]
enum Kind {
    Task,
    Container,
}

#[derive(Debug, Clone)]
struct UiItem {
    kind: Kind,
    id: String, // container id OR task name
    name: String,
    label: String,
    ports: Vec<docker::Port>,
    #[allow(dead_code)]
    task_cmd: Option<String>,
}

struct TaskRuntime {
    spec: TaskSpec,
    status: TaskStatus,
    lines: VecDeque<String>,
    child: Option<tokio::process::Child>,
    rx: Option<mpsc::UnboundedReceiver<String>>,
}

#[derive(Clone)]
enum Popup {
    Inspect { title: String, content: String },
    ConfirmReset { id: String, name: String },
    ConfirmComposeRestart { infra_running: bool },
}

struct App {
    cfg: Config,
    docker: docker::DockerMeta,

    items: Vec<UiItem>,
    selected: usize,
    focus_on_list: bool,

    // current logs target: container id OR task name
    current_target: String,

    // log buffer for current target
    log_lines: VecDeque<String>,
    log_scroll: u16,
    stick_to_bottom: bool,
    last_log_height: u16,

    // docker logs follower
    docker_log_child: Option<tokio::process::Child>,
    docker_log_rx: Option<mpsc::UnboundedReceiver<String>>,

    tasks: HashMap<String, TaskRuntime>,

    // cached containers
    // cached containers
    containers: Vec<(docker::ContainerSummary, Vec<docker::Port>)>,
    refreshing: bool,

    // For mouse handling
    list_state: ratatui::widgets::ListState,
    list_area: Rect,

    popup: Option<Popup>,
}

pub async fn run(opts: RunOpts) -> Result<()> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Profile select (parity with JS) - REMOVED
    // Default to "local" or env var, skipping the selection UI
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
        if app.cfg.auto_compose_up {
            let infra_running = app.infra_already_up();
            app.popup = Some(Popup::ConfirmComposeRestart { infra_running });
        }
    } else {
        app.push_current_log("Docker unavailable. Colima: colima start ; docker context use colima ; docker ps");
        app.rebuild_items();
    }

    let mut ticker = time::interval(Duration::from_millis(app.cfg.refresh_ms));
    let (tx_refresh, mut rx_refresh) = mpsc::unbounded_channel();

    loop {
        terminal.draw(|f| draw_ui(f, &mut app))?;

        // pump background logs into current view
        app.pump_background().await;

        tokio::select! {
            _ = ticker.tick() => {
                if app.docker.available && app.popup.is_none() && !app.refreshing {
                    app.refreshing = true;
                    let tx = tx_refresh.clone();
                    let docker = app.docker.clone();
                    let cwd = app.cfg.cwd.clone();
                    tokio::spawn(async move {
                        let res = docker::list_containers_all(&docker, &cwd).await;
                        let _ = tx.send(res);
                    });
                }
            }
            Some(res) = rx_refresh.recv() => {
                app.refreshing = false;
                if let Ok(containers) = res {
                    app.containers = containers;
                    app.rebuild_items();
                }
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

impl App {
    fn new(cfg: Config, docker: docker::DockerMeta) -> Self {
        let mut tasks_map = HashMap::new();
        for t in &cfg.post_up_tasks {
            tasks_map.insert(
                t.name.clone(),
                TaskRuntime {
                    spec: t.clone(),
                    status: TaskStatus::Pending,
                    lines: VecDeque::new(),
                    child: None,
                    rx: None,
                },
            );
        }

        let mut log_lines = VecDeque::new();
        log_lines.push_back(format!("Profile: {}", cfg.profile));
        log_lines.push_back(format!(
            "Docker backend: {} | Context: {}",
            docker.backend, docker.context_name
        ));
        let tasks = if cfg.post_up_tasks.is_empty() {
            "(none)".to_string()
        } else {
            cfg.post_up_tasks
                .iter()
                .map(|t| t.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        };
        log_lines.push_back(format!("Post-up tasks: {tasks}"));

        Self {
            cfg,
            docker,
            items: Vec::new(),
            selected: 0,
            focus_on_list: true,
            current_target: "".to_string(),
            log_lines,
            log_scroll: 0,
            stick_to_bottom: true,
            last_log_height: 0,
            docker_log_child: None,
            docker_log_rx: None,
            tasks: tasks_map,
            containers: Vec::new(),
            refreshing: false,
            list_state: ratatui::widgets::ListState::default(),
            list_area: Rect::default(),
            popup: None,
        }
    }

    fn infra_already_up(&self) -> bool {
        let names: std::collections::HashSet<String> = self
            .containers
            .iter()
            .map(|(c, _)| docker::container_name(&c.names))
            .collect();
        names.contains(&self.cfg.db_container) || names.contains(&self.cfg.storage_container)
    }

    async fn refresh_containers(&mut self) -> Result<()> {
        // synchronous refresh kept for initial load or manual actions
        self.containers = docker::list_containers_all(&self.docker, &self.cfg.cwd).await?;
        Ok(())
    }

    fn rebuild_items(&mut self) {
        let mut items = Vec::new();

        // tasks first
        for t in &self.cfg.post_up_tasks {
            let rt = self.tasks.get(&t.name);
            let (status, lines) = rt
                .map(|r| (r.status, r.lines.len()))
                .unwrap_or((TaskStatus::Pending, 0));
            let badge = match status {
                TaskStatus::Run => "🟢",
                TaskStatus::Ok => "⚪️",
                TaskStatus::Fail => "🔴",
                TaskStatus::Stop => "⚪️",
                TaskStatus::Pending => "⚪️",
            };
            let label = format!(
                "{badge} task: {:<14}  [{:<4}]  logs:{:>4}",
                t.name,
                status.as_str(),
                lines
            );

            items.push(UiItem {
                kind: Kind::Task,
                id: t.name.clone(),
                name: t.name.clone(),
                label,
                ports: vec![],
                task_cmd: Some(t.cmd.clone()),
            });
        }

        // containers
        for (c, ports) in &self.containers {
            let name = docker::container_name(&c.names);
            let state = c.state.to_lowercase();
            let badge = if state == "running" {
                "🟢"
            } else if state == "paused" {
                "🟡"
            } else if state == "restarting" {
                "🔵"
            } else if state == "created" {
                "⚪️"
            } else if state == "exited" || state == "dead" {
                "🔴"
            } else {
                "⚪️"
            };

            let status_txt = c.status.split_whitespace().collect::<Vec<_>>().join(" ");

            items.push(UiItem {
                kind: Kind::Container,
                id: c.id.clone(),
                name: name.clone(),
                label: format!("{badge} {:<22} {status_txt}", name),
                ports: ports.clone(),
                task_cmd: None,
            });
        }

        self.items = items;
        if self.selected >= self.items.len() {
            self.selected = self.items.len().saturating_sub(1);
        }
    }

    fn set_focus(&mut self, list: bool) {
        self.focus_on_list = list;
    }

    fn push_current_log(&mut self, line: &str) {
        self.log_lines.push_back(line.to_string());
        while self.log_lines.len() > self.cfg.max_log_lines {
            self.log_lines.pop_front();
        }
        // Don't reset scroll here; let stick_to_bottom handle it in draw_ui
    }

    fn replace_current_logs(&mut self, all: Vec<String>) {
        self.log_lines.clear();
        for l in all {
            self.log_lines.push_back(l);
        }
        while self.log_lines.len() > self.cfg.max_log_lines {
            self.log_lines.pop_front();
        }
        self.stick_to_bottom = true;
    }

    async fn select(&mut self, idx: usize) -> Result<()> {
        if self.items.is_empty() {
            return Ok(());
        }
        self.selected = idx.min(self.items.len() - 1);
        self.list_state.select(Some(self.selected));
        let item = self.items[self.selected].clone();

        // stop docker log follower
        if let Some(mut c) = self.docker_log_child.take() {
            let _ = c.kill().await;
        }
        self.docker_log_rx = None;

        self.current_target = item.id.clone();

        match item.kind {
            Kind::Task => {
                let rt = self.tasks.get(&item.id);
                let lines = rt
                    .map(|r| r.lines.iter().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                self.replace_current_logs(lines);
            }
            Kind::Container => {
                self.replace_current_logs(vec![format!("--- streaming logs for {} ---", item.name)]);
                let (child, rx) = docker::spawn_logs_follow(&self.docker, &self.cfg.cwd, &item.id, 200)?;
                self.docker_log_child = Some(child);
                self.docker_log_rx = Some(rx);
            }
        }

        Ok(())
    }

    async fn pump_background(&mut self) {
        // -------------------------
        // docker logs: drain first
        // -------------------------
        let mut docker_drained: Vec<String> = Vec::new();
        if let Some(rx) = self.docker_log_rx.as_mut() {
            while let Ok(line) = rx.try_recv() {
                docker_drained.push(line);
            }
        }
        for line in docker_drained {
            self.push_current_log(&line);
        }

        // -------------------------
        // task logs: collect UI ops, then apply
        // -------------------------
        let current = self.current_target.clone();
        let max_lines = self.cfg.max_log_lines;

        let mut ui_append: Vec<String> = Vec::new();
        let mut ui_replace: Option<Vec<String>> = None;

        for (name, rt) in self.tasks.iter_mut() {
            // drain task rx
            if let Some(rx) = rt.rx.as_mut() {
                loop {
                    match rx.try_recv() {
                        Ok(line) => {
                            let full = format!("[{name}] {line}");
                            rt.lines.push_back(full.clone());
                            while rt.lines.len() > max_lines {
                                rt.lines.pop_front();
                            }
                            if current == *name {
                                ui_append.push(full);
                            }
                        }
                        Err(_) => break,
                    }
                }
            }

            // exit detection
            if let Some(child) = rt.child.as_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let code = status.code().unwrap_or(if status.success() { 0 } else { 1 });

                        rt.child = None;
                        rt.rx = None;

                        if code == 0 {
                            rt.status = TaskStatus::Ok;
                            rt.lines.push_back("==> OK".to_string());
                        } else {
                            rt.status = TaskStatus::Fail;
                            rt.lines.push_back(format!("==> FAIL (exit {code})"));
                        }
                        while rt.lines.len() > max_lines {
                            rt.lines.pop_front();
                        }

                        if current == *name {
                            ui_replace = Some(rt.lines.iter().cloned().collect::<Vec<_>>());
                        }
                    }
                    _ => {}
                }
            }
        }

        // Apply UI mutations after the loop (avoids &mut self borrow conflicts)
        if let Some(lines) = ui_replace {
            self.replace_current_logs(lines);
        } else {
            for l in ui_append {
                self.push_current_log(&l);
            }
        }
    }

    async fn run_task(&mut self, task_name: &str) -> Result<()> {
        // Do all mutations on the task, then snapshot logs, then update UI.
        let mut snapshot_for_ui: Option<Vec<String>> = None;

        if let Some(rt) = self.tasks.get_mut(task_name) {
            // stop if running
            if let Some(child) = &rt.child {
                tasks::kill_process_group(child);
            }
            rt.child = None;
            rt.rx = None;

            rt.lines.clear();
            rt.status = TaskStatus::Run;
            rt.lines.push_back(format!("==> RESTART: {}", rt.spec.cmd));

            let (child, rx) = tasks::spawn_task(&rt.spec.cmd, &self.cfg.cwd)?;
            rt.child = Some(child);
            rt.rx = Some(rx);

            if self.current_target == task_name {
                snapshot_for_ui = Some(rt.lines.iter().cloned().collect::<Vec<_>>());
            }
        }

        if let Some(lines) = snapshot_for_ui {
            self.replace_current_logs(lines);
        }
        Ok(())
    }

    async fn stop_task(&mut self, task_name: &str) {
        let mut snapshot_for_ui: Option<Vec<String>> = None;

        if let Some(rt) = self.tasks.get_mut(task_name) {
            if let Some(child) = &rt.child {
                tasks::kill_process_group(child);
                rt.status = TaskStatus::Stop;
                rt.lines.push_back("==> STOPPED (user)".to_string());
            }
            rt.child = None;
            rt.rx = None;

            while rt.lines.len() > self.cfg.max_log_lines {
                rt.lines.pop_front();
            }

            if self.current_target == task_name {
                snapshot_for_ui = Some(rt.lines.iter().cloned().collect::<Vec<_>>());
            }
        }

        if let Some(lines) = snapshot_for_ui {
            self.replace_current_logs(lines);
        }
    }

    async fn compose_up_or_restart(&mut self, restart: bool) {
        let profile = self.cfg.compose_profile.clone();
        let cwd = self.cfg.cwd.clone();
        if restart {
            self.push_current_log(&format!("Restarting services (profile: {profile})..."));
            let code = docker::docker_compose(&self.docker, &cwd, &profile, &["restart"]).await.unwrap_or(1);
            if code != 0 {
                self.push_current_log(&format!("Restart failed (exit {code}) → fallback: up -d"));
                let code2 = docker::docker_compose(&self.docker, &cwd, &profile, &["up", "-d"]).await.unwrap_or(1);
                if code2 != 0 {
                    self.push_current_log(&format!("Compose up failed (exit {code2})"));
                } else {
                    self.push_current_log("Compose up OK");
                }
            } else {
                self.push_current_log("Compose restart OK");
            }
        } else {
            self.push_current_log(&format!("Starting services (profile: {profile})..."));
            let code = docker::docker_compose(&self.docker, &cwd, &profile, &["up", "-d"]).await.unwrap_or(1);
            if code != 0 {
                self.push_current_log(&format!("Compose up FAILED (exit {code})"));
            } else {
                self.push_current_log("Compose up OK");
            }
        }

        let _ = self.refresh_containers().await;
        self.rebuild_items();
        let _ = self.select(self.selected).await;
    }

    async fn open_selected_in_browser(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let item = self.items[self.selected].clone();
        if !matches!(item.kind, Kind::Container) {
            return;
        }
        let port = docker::pick_best_public_port(&item.ports);
        if let Some(pubp) = port {
            let host = if self.docker.remote_host.is_empty() { "localhost" } else { &self.docker.remote_host };
            let url = format!("http://{host}:{pubp}");
            let _ = open::that(url);
        } else {
            self.push_current_log(&format!("No public tcp port for {}", item.name));
        }
    }
}

async fn handle_event(app: &mut App, ev: Event) -> Result<bool> {
    if let Event::Key(k) = ev {
        // Global quit
        if (k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL))
            || k.code == KeyCode::Char('q')
        {
            return Ok(true);
        }

        // popup mode
        if let Some(p) = app.popup.clone() {
            match p {
                Popup::Inspect { .. } => {
                    // close on Esc / Enter / q
                    if matches!(k.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                        app.popup = None;
                    }
                    return Ok(false);
                }
                Popup::ConfirmReset { id, name } => {
                    match k.code {
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
                    return Ok(false);
                }
                Popup::ConfirmComposeRestart { infra_running } => {
                    match k.code {
                        KeyCode::Char('r') | KeyCode::Enter => {
                            app.popup = None;
                            app.compose_up_or_restart(infra_running).await;
                        }
                        KeyCode::Char('k') => {
                            // Keep
                            app.popup = None;
                        }
                        KeyCode::Esc => {
                            app.popup = None;
                        }
                        _ => {}
                    }
                    return Ok(false);
                }
            }
        }

        // focus toggle
        if k.code == KeyCode::Tab {
            app.set_focus(!app.focus_on_list);
            return Ok(false);
        }

        // selection navigation (list focus only)
        if app.focus_on_list {
            match k.code {
                KeyCode::Up => {
                    if app.selected > 0 {
                        app.selected -= 1;
                    }
                }
                KeyCode::Down => {
                    if app.selected + 1 < app.items.len() {
                        app.selected += 1;
                    }
                }
                KeyCode::Enter => {
                    let _ = app.select(app.selected).await;
                }
                _ => {}
            }
        } else {
            // Logs focus navigation
            let height = app.last_log_height; // Use last known height
            match k.code {
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

        // actions
        match k.code {
            KeyCode::Char('o') => {
                app.open_selected_in_browser().await;
            }
            KeyCode::Char('c') => {
                if app.docker.available {
                    app.popup = Some(Popup::ConfirmComposeRestart { infra_running: app.infra_already_up() });
                }
            }
            KeyCode::Char('i') => {
                if app.items.is_empty() {
                    return Ok(false);
                }
                let item = app.items[app.selected].clone();
                if matches!(item.kind, Kind::Container) && app.docker.available {
                    if let Ok(v) = docker::container_inspect(&app.docker, &app.cfg.cwd, &item.id).await {
                        let content = docker::format_container_info(&v);
                        app.popup = Some(Popup::Inspect { title: format!("Inspect: {}", item.name), content });
                    }
                }
            }
            KeyCode::Char('x') => {
                if app.items.is_empty() {
                    return Ok(false);
                }
                let item = app.items[app.selected].clone();
                if matches!(item.kind, Kind::Container) && app.docker.available {
                    app.popup = Some(Popup::ConfirmReset { id: item.id, name: item.name });
                }
            }
            KeyCode::Char('r') => {
                if app.items.is_empty() {
                    return Ok(false);
                }
                let item = app.items[app.selected].clone();
                match item.kind {
                    Kind::Task => {
                        let _ = app.run_task(&item.id).await;
                        app.rebuild_items();
                        let _ = app.select(app.selected).await;
                    }
                    Kind::Container => {
                        if app.docker.available {
                            app.push_current_log(&format!("Restarting container {}...", item.name));
                            let _ = docker::container_action(&app.docker, &app.cfg.cwd, "restart", &item.id).await;
                            let _ = app.refresh_containers().await;
                            app.rebuild_items();
                            let _ = app.select(app.selected).await;
                        }
                    }
                }
            }
            KeyCode::Char('s') => {
                if app.items.is_empty() {
                    return Ok(false);
                }
                let item = app.items[app.selected].clone();
                match item.kind {
                    Kind::Task => {
                        app.stop_task(&item.id).await;
                        app.rebuild_items();
                        let _ = app.select(app.selected).await;
                    }
                    Kind::Container => {
                        if app.docker.available {
                            app.push_current_log(&format!("Stopping container {}...", item.name));
                            let _ = docker::container_action(&app.docker, &app.cfg.cwd, "stop", &item.id).await;
                            let _ = app.refresh_containers().await;
                            app.rebuild_items();
                        }
                    }
                }
            }
            KeyCode::Char('t') => {
                if app.items.is_empty() {
                    return Ok(false);
                }
                let item = app.items[app.selected].clone();
                match item.kind {
                    Kind::Task => {
                        let _ = app.run_task(&item.id).await;
                        app.rebuild_items();
                        let _ = app.select(app.selected).await;
                    }
                    Kind::Container => {
                        if app.docker.available {
                            app.push_current_log(&format!("Starting container {}...", item.name));
                            let _ = docker::container_action(&app.docker, &app.cfg.cwd, "start", &item.id).await;
                            let _ = app.refresh_containers().await;
                            app.rebuild_items();
                            let _ = app.select(app.selected).await;
                        }
                    }
                }
            }
            KeyCode::Char('p') => {
                if app.items.is_empty() {
                    return Ok(false);
                }
                let item = app.items[app.selected].clone();
                if matches!(item.kind, Kind::Container) && app.docker.available {
                    app.push_current_log(&format!("Pausing container {}...", item.name));
                    let _ = docker::container_action(&app.docker, &app.cfg.cwd, "pause", &item.id).await;
                    let _ = app.refresh_containers().await;
                    app.rebuild_items();
                }
            }
            KeyCode::Char('u') => {
                if app.items.is_empty() {
                    return Ok(false);
                }
                let item = app.items[app.selected].clone();
                if matches!(item.kind, Kind::Container) && app.docker.available {
                    app.push_current_log(&format!("Unpausing container {}...", item.name));
                    let _ = docker::container_action(&app.docker, &app.cfg.cwd, "unpause", &item.id).await;
                    let _ = app.refresh_containers().await;
                    app.rebuild_items();
                }
            }
            KeyCode::Char('k') => {
                if app.items.is_empty() {
                    return Ok(false);
                }
                let item = app.items[app.selected].clone();
                if matches!(item.kind, Kind::Container) && app.docker.available {
                    app.push_current_log(&format!("Killing container {}...", item.name));
                    let _ = docker::container_action(&app.docker, &app.cfg.cwd, "kill", &item.id).await;
                    let _ = app.refresh_containers().await;
                    app.rebuild_items();
                }
            }
            KeyCode::Char('d') => {
                if app.items.is_empty() {
                    return Ok(false);
                }
                let item = app.items[app.selected].clone();
                if matches!(item.kind, Kind::Container) && app.docker.available {
                    app.push_current_log(&format!("Removing container {} (force)...", item.name));
                    let _ = docker::container_rm_force(&app.docker, &app.cfg.cwd, &item.id).await;
                    let _ = app.refresh_containers().await;
                    app.rebuild_items();
                }
            }
            _ => {}
        }
    }

    if let Event::Mouse(MouseEvent { kind, column, row, .. }) = ev {
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
                // Check if click is inside list area
                let area = app.list_area;
                if column >= area.x && column < area.x + area.width && row >= area.y && row < area.y + area.height {
                    // 1 for border
                    if row > area.y && row < area.y + area.height - 1 {
                        let relative_row = row - area.y - 1;
                        let index = app.list_state.offset() + relative_row as usize;
                        if index < app.items.len() {
                            let _ = app.select(index).await;
                        }
                    }
                    app.set_focus(true);
                } else {
                    // assumes logs are the other part
                    app.set_focus(false);
                }
            }
            _ => {}
        }
    }

    Ok(false)
}

fn draw_ui(f: &mut Frame, app: &mut App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)].as_ref())
        .split(f.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(33), Constraint::Percentage(67)].as_ref())
        .split(root[0]);

    let border_style_list = if app.focus_on_list {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::White)
    };
    let border_style_logs = if app.focus_on_list {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::Green)
    };

    // left list
    let items: Vec<ListItem> = if app.docker.available {
        app.items.iter().map(|it| ListItem::new(it.label.clone())).collect()
    } else {
        vec![ListItem::new("(docker not available)")]
    };

    let updated = Local::now().format("%H:%M:%S").to_string();
    let left_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style_list)
        .title(format!(" Containers + Tasks (upd: {updated}) "));

    let list = List::new(items)
        .block(left_block)
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Blue))
        .highlight_symbol("▶ ");
    
    app.list_area = body[0];
    f.render_stateful_widget(list, app.list_area, &mut app.list_state);

    // logs
    let title = if app.current_target.is_empty() {
        " Logs ".to_string()
    } else {
        format!(" Logs — {t} ", t = app.current_target)
    };

    let log_text: Text = app
        .log_lines
        .iter()
        .map(|l| Line::from(Span::raw(l.clone())))
        .collect::<Vec<_>>()
        .into();

    let log_height = body[1].height.saturating_sub(2); // borders
    app.last_log_height = log_height;

    let total_lines = app.log_lines.len() as u16;
    if app.stick_to_bottom {
        app.log_scroll = total_lines.saturating_sub(log_height);
    } else {
        // Clamp scroll so we don't scroll past infinity
        let max_scroll = total_lines.saturating_sub(log_height);
        if app.log_scroll > max_scroll {
            app.log_scroll = max_scroll;
        }
    }

    let logs = Paragraph::new(log_text)
        .block(Block::default().borders(Borders::ALL).border_style(border_style_logs).title(title))
        .wrap(Wrap { trim: false })
        .scroll((app.log_scroll, 0));
    f.render_widget(logs, body[1]);

    // help bar
    let help = help_for_selected(app);
    let help_bar = Paragraph::new(help).style(Style::default().fg(Color::Black).bg(Color::White));
    f.render_widget(help_bar, root[1]);

    // popup overlay
    if let Some(p) = &app.popup {
        match p {
            Popup::Inspect { title, content } => {
                let area = centered_rect(80, 80, f.area());
                f.render_widget(Clear, area);
                let text: Text = content
                    .lines()
                    .map(|l| Line::from(Span::raw(l.to_string())))
                    .collect::<Vec<_>>()
                    .into();
                let w = Paragraph::new(text)
                    .block(Block::default().borders(Borders::ALL).title(title.clone()))
                    .wrap(Wrap { trim: false });
                f.render_widget(w, area);
            }
            Popup::ConfirmReset { name, .. } => {
                let area = centered_rect(60, 25, f.area());
                f.render_widget(Clear, area);
                let msg = format!(
                    "RESET {name}?\nThis will STOP it, REMOVE it, and DELETE its volumes.\n\n[y/Enter]=Reset, [n/Esc]=Cancel"
                );
                let w = Paragraph::new(msg)
                    .block(Block::default().borders(Borders::ALL).title(" ⚠️  RESET CONTAINER "))
                    .wrap(Wrap { trim: false });
                f.render_widget(w, area);
            }
            Popup::ConfirmComposeRestart { infra_running } => {
                let area = centered_rect(60, 25, f.area());
                f.render_widget(Clear, area);
                let msg = if *infra_running {
                    "Detected running stack containers.\n\n[r/Enter]=Restart services, [k]=Keep, [Esc]=Cancel".to_string()
                } else {
                    "Start services now?\n\n[r/Enter]=docker compose up -d, [Esc]=Cancel".to_string()
                };
                let w = Paragraph::new(msg)
                    .block(Block::default().borders(Borders::ALL).title(" Docker compose "))
                    .wrap(Wrap { trim: false });
                f.render_widget(w, area);
            }
        }
    }
}

fn help_for_selected(app: &App) -> String {
    if app.items.is_empty() {
        return " q:Quit ".to_string();
    }
    let item = &app.items[app.selected];
    let common = "Ent:Log r:Rest s:Stop t:Start tab:Focus q:Quit";
    let extra = if !app.focus_on_list { " ↑/↓:Scroll " } else { "" };
    match item.kind {
        Kind::Container => format!("{} p:Paus u:Unp k:Kill d:Rm i:Insp o:Web x:Reset c:Compose{}", common, extra),
        Kind::Task => format!("{}{}", common, extra),
    }
}



fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ]
            .as_ref(),
        )
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1])[1]
}

async fn read_event() -> Option<Event> {
    // poll at 50ms so ticker can run
    if event::poll(Duration::from_millis(50)).ok()? {
        event::read().ok()
    } else {
        None
    }
}


