use crate::docker;
use crate::tasks::TaskStatus;
use crate::config::{TaskSpec};
use std::collections::{VecDeque};
use tokio::process::{Child};
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub enum SidebarKind {
    Task,
    GroupHeader,
    Container,
    SwarmService,
    Image,
    Separator,
}

#[derive(Debug, Clone)]
pub struct UiItem {
    pub kind: SidebarKind,
    pub id: String,
    pub name: String,
    pub label: String,
    pub ports: Vec<docker::Port>,
    pub selected: bool,
    pub depth: usize,
}

pub struct TaskRuntime {
    pub spec: TaskSpec,
    pub status: TaskStatus,
    pub lines: VecDeque<String>,
    pub child: Option<Child>,
    pub rx: Option<mpsc::UnboundedReceiver<String>>,
}

#[derive(Clone)]
pub enum Popup {
    Inspect { title: String, content: String },
    ConfirmReset { id: String, name: String },
    ConfirmComposeRestart { infra_running: bool },
    ScaleService { id: String, name: String, current: u64, input: String },
    Volumes { volumes: Vec<crate::docker::DockerVolume>, selected: usize },
    Networks { networks: Vec<crate::docker::DockerNetwork>, selected: usize },
    ContextSwitch { contexts: Vec<crate::docker::DockerContext>, selected: usize },
    SystemHealth { data: Vec<docker::SystemDfRow> },
    ImageExplorer { images: Vec<crate::docker::DockerImage>, selected: usize },
    ConfirmPrune,
    Help,
}
