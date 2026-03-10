use crate::config::Config;
use crate::docker;
use crate::pins;
use crate::tasks::{self, TaskStatus};
use crate::ui::types::{SidebarKind, UiItem, TaskRuntime, Popup};
use anyhow::{Result};
use ratatui::layout::{Rect};
use std::collections::{HashMap, HashSet, VecDeque};
use tokio::sync::mpsc;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Clone)]
pub enum SortBy {
    Name,
    Status,
    Id,
    Project,
}

#[derive(Debug, PartialEq, Clone)]
pub enum SortOrder {
    Asc,
    Desc,
}

#[derive(Debug)]
pub struct RunOpts {
    pub root: PathBuf,
    #[allow(dead_code)]
    pub docker_bin: String,
    pub docker_meta: docker::DockerMeta,
}

pub struct App {
    pub cfg: Config,
    pub docker: docker::DockerMeta,

    pub items: Vec<UiItem>,
    pub selected: usize,
    pub focus_on_list: bool,

    pub current_target: String,
    pub log_lines: VecDeque<String>,
    pub log_scroll: u16,
    pub stick_to_bottom: bool,
    pub follow_mode: bool,
    pub last_log_height: u16,

    pub docker_log_child: Option<crate::docker::LogStream>,
    pub docker_log_rx: Option<mpsc::UnboundedReceiver<String>>,

    pub tasks: HashMap<String, TaskRuntime>,
    pub containers: Vec<(docker::ContainerSummary, Vec<docker::Port>)>,
    pub expanded_groups: HashSet<String>,
    pub refreshing: bool,

    pub swarm_services: Vec<docker::SwarmService>,
    pub swarm_refreshing: bool,

    pub list_state: ratatui::widgets::ListState,
    pub list_area: Rect,

    pub popup: Option<Popup>,
    pub copy_mode: bool,

    pub container_stats: Option<docker::ContainerStats>,
    pub stats_history: HashMap<String, VecDeque<(f64, f64)>>,
    pub stats_refreshing: bool,
    pub pins: HashSet<String>,

    pub shell_stdin: Option<std::pin::Pin<Box<dyn tokio::io::AsyncWrite + Send>>>,
    pub shell_process: Option<crate::docker::LogStream>,
    pub shell_active: bool,
    pub shell_input: String,
    
    pub sort_by: SortBy,
    pub sort_order: SortOrder,

    pub filter_query: String,
    pub is_filtering: bool,
    pub log_filter_query: String,
    pub is_filtering_logs: bool,

    pub multi_selected: HashSet<String>,
    pub toast: Option<(String, std::time::Instant, ratatui::style::Color)>,
}

impl App {
    pub fn new(cfg: Config, docker: docker::DockerMeta) -> Self {
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
            follow_mode: true,
            last_log_height: 0,
            docker_log_child: None,
            docker_log_rx: None,
            tasks: tasks_map,
            containers: Vec::new(),
            expanded_groups: HashSet::new(),
            refreshing: false,
            swarm_services: Vec::new(),
            swarm_refreshing: false,
            list_state: ratatui::widgets::ListState::default(),
            list_area: Rect::default(),
            popup: None,
            copy_mode: false,
            container_stats: None,
            stats_history: HashMap::new(),
            stats_refreshing: false,
            pins: pins::load_pins(),
            shell_stdin: None,
            shell_process: None,
            shell_active: false,
            shell_input: String::new(),
            filter_query: String::new(),
            is_filtering: false,
            log_filter_query: String::new(),
            is_filtering_logs: false,
            multi_selected: HashSet::new(),
            toast: None,
            sort_by: SortBy::Name,
            sort_order: SortOrder::Asc,
        }
    }

    pub fn notify(&mut self, msg: String, color: ratatui::style::Color) {
        self.toast = Some((msg, std::time::Instant::now(), color));
    }

    pub fn infra_already_up(&self) -> bool {
        let names: std::collections::HashSet<String> = self
            .containers
            .iter()
            .map(|(c, _)| docker::container_name(&c.names))
            .collect();
        names.contains(&self.cfg.db_container) || names.contains(&self.cfg.storage_container)
    }

    pub async fn refresh_containers(&mut self) -> Result<()> {
        self.containers = docker::list_containers_all(&self.docker, &self.cfg.cwd).await?;
        Ok(())
    }

    pub async fn refresh_swarm(&mut self) -> Result<()> {
        self.swarm_services = docker::list_swarm_services(&self.docker, &self.cfg.cwd).await;
        Ok(())
    }

    fn get_sparkline(&self, data: impl Iterator<Item = f64>, max: f64, len: usize) -> String {
        let chars = [" ", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
        let vals: Vec<f64> = data.collect();
        let mut out = String::new();
        let start = vals.len().saturating_sub(len);
        for &v in &vals[start..] {
            let idx = ((v / max) * (chars.len() - 1) as f64).round() as usize;
            out.push_str(chars[idx.min(chars.len() - 1)]);
        }
        if out.chars().count() < len {
            format!("{}{}", " ".repeat(len - out.chars().count()), out)
        } else {
            out
        }
    }

    pub fn rebuild_items(&mut self) {
        let mut sorted_containers = self.containers.clone();
        let mut sorted_swarm = self.swarm_services.clone();

        match self.sort_by {
            SortBy::Name => {
                sorted_containers.sort_by(|a, b| {
                    let res = docker::container_name(&a.0.names).cmp(&docker::container_name(&b.0.names));
                    if self.sort_order == SortOrder::Asc { res } else { res.reverse() }
                });
                sorted_swarm.sort_by(|a, b| {
                    let res = a.name.cmp(&b.name);
                    if self.sort_order == SortOrder::Asc { res } else { res.reverse() }
                });
            }
            SortBy::Status => {
                sorted_containers.sort_by(|a, b| {
                    let res = a.0.state.cmp(&b.0.state);
                    if self.sort_order == SortOrder::Asc { res } else { res.reverse() }
                });
            }
            SortBy::Id => {
                sorted_containers.sort_by(|a, b| {
                    let res = a.0.id.cmp(&b.0.id);
                    if self.sort_order == SortOrder::Asc { res } else { res.reverse() }
                });
                sorted_swarm.sort_by(|a, b| {
                    let res = a.id.cmp(&b.id);
                    if self.sort_order == SortOrder::Asc { res } else { res.reverse() }
                });
            }
            SortBy::Project => {
                sorted_containers.sort_by(|a, b| {
                    let p_a = a.0.compose_project.as_deref().unwrap_or_default();
                    let p_b = b.0.compose_project.as_deref().unwrap_or_default();
                    let res = p_a.cmp(p_b);
                    if self.sort_order == SortOrder::Asc { res } else { res.reverse() }
                });
            }
        }

        let mut items: Vec<UiItem> = Vec::new();
        let query = self.filter_query.to_lowercase();

        if !query.is_empty() {
            let re = regex::RegexBuilder::new(&query)
                .case_insensitive(true)
                .build().ok();

            items.push(UiItem {
                kind: SidebarKind::Separator,
                id: "__filter_results__".to_string(),
                name: "Results".to_string(),
                label: format!("── 🔍 Filter: {query} ──"),
                ports: vec![],
                selected: false,
                depth: 0,
            });

            // Match containers
            for (c, ports) in &sorted_containers {
                let name = docker::container_name(&c.names);
                let is_match = if let Some(ref r) = re {
                    r.is_match(&name) || r.is_match(&c.id)
                } else {
                    name.to_lowercase().contains(&query) || c.id.contains(&query)
                };

                if is_match {
                    let state = c.state.to_lowercase();
                    let badge = if state == "running" { "🟢" }
                        else if state == "paused" { "🟡" }
                        else if state == "restarting" { "🔵" }
                        else if state == "exited" || state == "dead" { "🔴" }
                        else { "⚪️" };
                    
                    let mut label = format!(" {badge} {name}");
                    if let Some(history) = self.stats_history.get(&c.id) {
                        let cpu_spark = self.get_sparkline(history.iter().map(|h| h.0), 100.0, 5);
                        label.push_str(&format!("  [C:{}]", cpu_spark));
                    }

                    items.push(UiItem {
                        kind: SidebarKind::Container,
                        id: c.id.clone(),
                        name: name.clone(),
                        label,
                        ports: ports.clone(),
                        selected: self.multi_selected.contains(&c.id),
                        depth: 0,
                    });
                }
            }
            // Match services
            for svc in &sorted_swarm {
                let is_match = if let Some(ref r) = re {
                    r.is_match(&svc.name) || r.is_match(&svc.id)
                } else {
                    svc.name.to_lowercase().contains(&query) || svc.id.to_lowercase().contains(&query)
                };

                if is_match {
                    items.push(UiItem {
                        kind: SidebarKind::SwarmService,
                        id: svc.id.clone(),
                        name: svc.name.clone(),
                        label: format!(" 🐳 {}", svc.name),
                        ports: svc.ports.clone(),
                        selected: self.multi_selected.contains(&svc.id),
                        depth: 0,
                    });
                }
            }

            self.items = items;
            if self.selected >= self.items.len() {
                self.selected = self.items.len().saturating_sub(1);
            }
            return;
        }

        // -- Pinned containers --
        let pinned: Vec<&(docker::ContainerSummary, Vec<docker::Port>)> = sorted_containers
            .iter()
            .filter(|(c, _)| self.pins.contains(&docker::container_name(&c.names)))
            .collect();
        if !pinned.is_empty() {
            items.push(UiItem {
                kind: SidebarKind::GroupHeader,
                id: "__pins__".to_string(),
                name: "__pins__".to_string(),
                label: format!("  📌 Pinned  ({} containers)", pinned.len()),
                ports: vec![],
                selected: false,
                depth: 0,
            });
            for (c, ports) in &pinned {
                let name = docker::container_name(&c.names);
                let state_icon = match c.state.as_str() {
                    "running" => "🟢", "paused" => "🟡",
                    "exited" | "dead" => "🔴", _ => "⚪️",
                };
                let stats_suffix = self.container_stats.as_ref()
                    .map(|s| s.sidebar_label()).unwrap_or_default();
                let label = format!("    {state_icon} {name}{stats_suffix}");
                items.push(UiItem {
                    kind: SidebarKind::Container,
                    id: c.id.clone(),
                    name: name.clone(),
                    label,
                    ports: ports.clone(),
                    selected: self.multi_selected.contains(&c.id),
                    depth: 1,
                });
            }
        }

        // -- Tasks section --
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
                kind: SidebarKind::Task,
                id: t.name.clone(),
                name: t.name.clone(),
                label,
                ports: vec![],
                selected: self.multi_selected.contains(&t.name),
                depth: 0,
            });
        }

        // -- Containers grouped by compose project --
        let mut project_order: Vec<String> = Vec::new();
        let mut project_containers: HashMap<String, Vec<&(docker::ContainerSummary, Vec<docker::Port>)>> = HashMap::new();
        for entry in &sorted_containers {
            let key = entry.0.compose_project.clone().unwrap_or_else(|| "(ungrouped)".to_string());
            project_containers.entry(key.clone()).or_default().push(entry);
            if !project_order.contains(&key) {
                project_order.push(key);
            }
        }
        project_order.sort_by(|a, b| {
            match (a.as_str(), b.as_str()) {
                ("(ungrouped)", _) => std::cmp::Ordering::Greater,
                (_, "(ungrouped)") => std::cmp::Ordering::Less,
                _ => a.cmp(b),
            }
        });

        for project in &project_order {
            let collapsed = !self.expanded_groups.contains(project);
            let group_containers = project_containers.get(project).map(|v| v.as_slice()).unwrap_or(&[]);
            let running_count = group_containers.iter().filter(|(c, _)| c.state.to_lowercase() == "running").count();
            let total_count = group_containers.len();
            let arrow = if collapsed { "▶" } else { "▼" };
            let header_label = format!("{arrow} 📦 {}  ({}/{})", project, running_count, total_count);

            items.push(UiItem {
                kind: SidebarKind::GroupHeader,
                id: project.clone(),
                name: project.clone(),
                label: header_label,
                ports: vec![],
                selected: false,
                depth: 0,
            });

            if !collapsed {
                for (c, ports) in group_containers {
                    let name = docker::container_name(&c.names);
                    let state = c.state.to_lowercase();
                    let badge = if state == "running" { "🟢" }
                        else if state == "paused" { "🟡" }
                        else if state == "restarting" { "🔵" }
                        else if state == "created" { "⚪️" }
                        else if state == "exited" || state == "dead" { "🔴" }
                        else { "⚪️" };

                    let status_txt = c.status.split_whitespace().collect::<Vec<_>>().join(" ");
                    let mut label = format!("  {badge} {:<20} {status_txt}", name);
                    if let Some(history) = self.stats_history.get(&c.id) {
                        let cpu_spark = self.get_sparkline(history.iter().map(|h| h.0), 100.0, 5);
                        label.push_str(&format!("  [C:{}]", cpu_spark));
                    }
                    items.push(UiItem {
                        kind: SidebarKind::Container,
                        id: c.id.clone(),
                        name: name.clone(),
                        label,
                        ports: ports.to_vec(),
                        selected: self.multi_selected.contains(&c.id),
                        depth: 1,
                    });
                }
            }
        }

        // -- Swarm services section --
        if !sorted_swarm.is_empty() {
            items.push(UiItem {
                kind: SidebarKind::Separator,
                id: "__swarm_sep__".to_string(),
                name: "── Swarm Services ──".to_string(),
                label: "── 🐳 Swarm Services ──".to_string(),
                ports: vec![],
                selected: false,
                depth: 0,
            });

            let mut stack_order: Vec<String> = Vec::new();
            let mut stack_services: HashMap<String, Vec<&docker::SwarmService>> = HashMap::new();
            for svc in &sorted_swarm {
                let key = svc.stack.clone().unwrap_or_else(|| "(no stack)".to_string());
                stack_services.entry(key.clone()).or_default().push(svc);
                if !stack_order.contains(&key) {
                    stack_order.push(key);
                }
            }
            stack_order.sort();

            for stack in stack_order {
                let group_id = format!("stack:{}", stack);
                let collapsed = !self.expanded_groups.contains(&group_id);
                let group_services = stack_services.get(&stack).unwrap();
                let arrow = if collapsed { "▶" } else { "▼" };
                let header_label = format!("{} 🌊 Stack: {}  ({})", arrow, stack, group_services.len());
                
                items.push(UiItem {
                    kind: SidebarKind::GroupHeader,
                    id: group_id,
                    name: stack.clone(),
                    label: header_label,
                    ports: vec![],
                    selected: false,
                    depth: 0,
                });

                if !collapsed {
                    for svc in group_services {
                        let label = format!("    🐳 {:<22} {} ({}) {}", svc.name, svc.replicas, svc.mode, svc.image);
                        items.push(UiItem {
                            kind: SidebarKind::SwarmService,
                            id: svc.id.clone(),
                            name: svc.name.clone(),
                            label,
                            ports: svc.ports.clone(),
                            selected: self.multi_selected.contains(&svc.id),
                            depth: 1,
                        });
                    }
                }
            }
        }

        self.items = items;
        if self.selected >= self.items.len() {
            self.selected = self.items.len().saturating_sub(1);
        }
        while self.selected < self.items.len()
            && self.items[self.selected].kind == SidebarKind::Separator
        {
            self.selected += 1;
        }
    }

    pub fn toggle_select(&mut self, id: &str) {
        if self.multi_selected.contains(id) {
            self.multi_selected.remove(id);
        } else {
            self.multi_selected.insert(id.to_string());
        }
    }

    pub fn set_focus(&mut self, list: bool) {
        self.focus_on_list = list;
    }

    pub fn push_current_log(&mut self, line: &str) {
        self.log_lines.push_back(line.to_string());
        while self.log_lines.len() > self.cfg.max_log_lines {
            self.log_lines.pop_front();
        }
        if self.follow_mode {
            self.stick_to_bottom = true;
        }
    }

    pub fn push_partial_log(&mut self, data: &str) {
        for c in data.chars() {
            match c {
                '\n' => {
                    self.log_lines.push_back(String::new());
                }
                '\r' => {
                    if let Some(last) = self.log_lines.back_mut() {
                        last.clear();
                    }
                }
                '\x08' | '\x7f' => {
                    if let Some(last) = self.log_lines.back_mut() {
                        if !last.is_empty() {
                            last.pop();
                        }
                    }
                }
                _ => {
                    if let Some(last) = self.log_lines.back_mut() {
                        last.push(c);
                    } else {
                        self.log_lines.push_back(c.to_string());
                    }
                }
            }
        }

        while self.log_lines.len() > self.cfg.max_log_lines {
            self.log_lines.pop_front();
        }
        if self.follow_mode {
            self.stick_to_bottom = true;
        }
    }

    pub fn replace_current_logs(&mut self, all: Vec<String>) {
        self.log_lines.clear();
        for l in all {
            self.log_lines.push_back(l);
        }
        while self.log_lines.len() > self.cfg.max_log_lines {
            self.log_lines.pop_front();
        }
        if self.follow_mode {
            self.stick_to_bottom = true;
        }
    }

    pub async fn select(&mut self, idx: usize) -> Result<()> {
        if self.items.is_empty() {
            return Ok(());
        }
        self.selected = idx.min(self.items.len() - 1);
        while self.selected < self.items.len()
            && self.items[self.selected].kind == SidebarKind::Separator
        {
            if self.selected + 1 < self.items.len() {
                self.selected += 1;
            } else {
                break;
            }
        }
        self.list_state.select(Some(self.selected));
        let item = self.items[self.selected].clone();

        if let Some(mut c) = self.docker_log_child.take() {
            c.kill();
        }
        self.docker_log_rx = None;

        self.current_target = item.id.clone();

        match item.kind {
            SidebarKind::Task => {
                let rt = self.tasks.get(&item.id);
                let lines = rt
                    .map(|r| r.lines.iter().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                self.replace_current_logs(lines);
            }
            SidebarKind::Container => {
                self.replace_current_logs(vec![format!("--- streaming logs for {} ---", item.name)]);
                let (child, rx) = docker::spawn_logs_follow(&self.docker, &self.cfg.cwd, &item.id, 200)?;
                self.docker_log_child = Some(child);
                self.docker_log_rx = Some(rx);
            }
            SidebarKind::GroupHeader => {
                self.replace_current_logs(vec![
                    format!("Compose project: {}", item.name),
                    "Press [Space] to expand/collapse this group.".to_string(),
                ]);
            }
            SidebarKind::SwarmService => {
                self.replace_current_logs(vec![format!("--- streaming logs for service {} ---", item.name)]);
                let (child, rx) = docker::spawn_service_logs_follow(&self.docker, &self.cfg.cwd, &item.id, 200)?;
                self.docker_log_child = Some(child);
                self.docker_log_rx = Some(rx);
            }
            SidebarKind::Separator => {}
        }

        Ok(())
    }

    pub fn toggle_group_collapse(&mut self, project: &str) {
        if self.expanded_groups.contains(project) {
            self.expanded_groups.remove(project);
        } else {
            self.expanded_groups.insert(project.to_string());
        }
    }

    pub async fn stop_shell(&mut self) {
        if let Some(mut child) = self.shell_process.take() {
            let _ = child.kill();
        }
        self.shell_stdin = None;
        self.shell_active = false;
        self.shell_input.clear();
    }

    pub async fn start_shell(&mut self, id: &str, kind: SidebarKind) -> Result<()> {
        if let Some(mut c) = self.docker_log_child.take() {
            c.kill();
        }
        self.docker_log_rx = None;
        self.stop_shell().await;

        let target_id = if kind == SidebarKind::SwarmService {
            self.push_current_log(&format!("🔍 Looking for tasks of service: {}", id));
            match docker::find_service_task_container(&self.docker, &self.cfg.cwd, id).await {
                Some(cid) => {
                    self.push_current_log(&format!("✅ Found task container: {}", cid));
                    cid
                }
                None => {
                    // Show what's actually running for debug
                    let running = crate::docker::cmd_out(&self.docker.docker_bin, &self.cfg.cwd, &["ps", "--filter", "status=running", "--format", "{{.Names}}"]).await.unwrap_or_default();
                    self.push_current_log("❌ No running tasks found. Running containers:");
                    for name in running.lines().take(10) {
                        self.push_current_log(&format!("  - {}", name));
                    }
                    return Ok(());
                }
            }
        } else {
            id.to_string()
        };

        match docker::spawn_shell(&self.docker, &self.cfg.cwd, &target_id).await {
            Ok((child, stdin, rx)) => {
                self.shell_process = Some(child);
                self.shell_stdin = Some(stdin);
                self.docker_log_rx = Some(rx);
                self.shell_active = true;
                self.replace_current_logs(vec![
                    format!("🐚 Shell started in container {}", id),
                    "Type to send commands, Esc to exit.".to_string(),
                    "------------------------------------------------".to_string(),
                    "".to_string(), // New line for input
                ]);
            }
            Err(e) => {
                self.push_current_log(&format!("❌ Failed to start shell: {}", e));
            }
        }
        Ok(())
    }

    pub fn start_compose_logs(&mut self, project: String) -> Result<()> {
        if let Some(mut c) = self.docker_log_child.take() {
            let _ = tokio::spawn(async move { c.kill() });
        }
        self.docker_log_rx = None;
        self.current_target = format!("project:{}", project);
        self.replace_current_logs(vec![format!("📡 Aggregating logs for project: {}", project)]);

        let (child, rx) = docker::spawn_compose_logs(&self.docker, &self.cfg.cwd, &project, 100)?;
        self.docker_log_child = Some(child);
        self.docker_log_rx = Some(rx);
        Ok(())
    }

    pub async fn show_system_health(&mut self) -> Result<()> {
        let df = docker::get_system_df(&self.docker, &self.cfg.cwd).await?;
        self.popup = Some(Popup::SystemHealth { data: df });
        Ok(())
    }

    pub async fn trigger_prune(&mut self) -> Result<()> {
        self.push_current_log("🧹 Starting System Prune...");
        let out = docker::system_prune(&self.docker, &self.cfg.cwd).await?;
        self.push_current_log("✨ System Prune completed.");
        for line in out.lines() {
             self.push_current_log(&format!("  {}", line));
        }
        self.popup = None;
        Ok(())
    }

    pub async fn switch_context_and_refresh(&mut self, name: String) -> Result<()> {
        docker::use_context(&self.docker, &self.cfg.cwd, &name).await?;
        self.refresh_all_after_context_switch().await
    }

    pub async fn refresh_all_after_context_switch(&mut self) -> Result<()> {
        let new_meta = docker::DockerMeta::detect(&self.cfg.cwd, &self.cfg.docker_bin).await;
        self.docker = new_meta;
        
        self.containers.clear();
        self.swarm_services.clear();
        self.container_stats = None;
        self.multi_selected.clear();
        self.expanded_groups.clear();
        self.selected = 0;
        
        let _ = self.refresh_containers().await;
        let _ = self.refresh_swarm().await;
        self.rebuild_items();
        let _ = self.select(0).await;
        
        self.push_current_log(&format!("✅ Switched to context: {}", self.docker.context_name));
        Ok(())
    }

    pub async fn pump_background(&mut self) {
        let mut shell_data = Vec::new();
        if let Some(rx) = self.docker_log_rx.as_mut() {
            while let Ok(data) = rx.try_recv() {
                shell_data.push(data);
            }
        }
        for data in shell_data {
            self.push_partial_log(&data);
        }

        let current = self.current_target.clone();
        let max_lines = self.cfg.max_log_lines;

        let mut ui_append: Vec<String> = Vec::new();
        let mut ui_replace: Option<Vec<String>> = None;

        for (name, rt) in self.tasks.iter_mut() {
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

        if let Some(lines) = ui_replace {
            self.replace_current_logs(lines);
        } else {
            for l in ui_append {
                self.push_current_log(&l);
            }
        }
    }

    pub async fn run_task(&mut self, task_name: &str) -> Result<()> {
        let mut snapshot_for_ui: Option<Vec<String>> = None;
        if let Some(rt) = self.tasks.get_mut(task_name) {
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

    pub async fn stop_task(&mut self, task_name: &str) {
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

    pub async fn compose_up_or_restart(&mut self, restart: bool) {
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

    pub async fn open_selected_in_browser(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let item = self.items[self.selected].clone();
        if item.kind != SidebarKind::Container && item.kind != SidebarKind::SwarmService {
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

    pub async fn export_logs(&self) -> Result<String> {
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
        let filename = format!("logs_{}_{}.txt", self.current_target.replace("/", "_"), timestamp);
        let path = self.cfg.cwd.join(&filename);
        let content = self.log_lines.iter().map(|s| s.to_owned()).collect::<Vec<String>>().join("\n");
        std::fs::write(&path, content)?;
        Ok(filename)
    }

    pub fn toggle_sort(&mut self, next: SortBy) {
        if self.sort_by == next {
            self.sort_order = match self.sort_order {
                SortOrder::Asc => SortOrder::Desc,
                SortOrder::Desc => SortOrder::Asc,
            };
        } else {
            self.sort_by = next;
            self.sort_order = SortOrder::Asc;
        }
    }
}
