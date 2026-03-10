use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::process::Command;

pub mod containers;
pub mod images;
pub mod networks;
pub mod swarm;
pub mod volumes;
pub mod compose;
pub mod system;

pub use containers::*;
pub use images::*;
pub use networks::*;
pub use swarm::*;
pub use volumes::*;
pub use compose::*;
pub use system::*;

#[derive(Debug, Clone)]
pub struct DockerMeta {
    pub backend: String,
    pub context_name: String,
    #[allow(dead_code)]
    pub socket_path: String,
    pub remote_host: String,
    pub available: bool,
    pub docker_bin: String,
    pub client: Option<bollard::Docker>,
}

impl DockerMeta {
    pub async fn detect(cwd: &Path, docker_bin: &str) -> Self {
        let docker_bin = docker_bin.to_string();
        let cwd_buf = cwd.to_path_buf();

        let mut available = false;
        let mut ctx_name = "default".to_string();
        let mut backend = "unknown".to_string();
        let mut socket_path = "".to_string();
        let mut remote_host = "localhost".to_string();
        let mut host_raw = "".to_string();

        // 1. Primary check: docker context show
        // If this works, the binary is found and functional.
        if let Ok(ctx_out) = cmd_out(&docker_bin, &cwd_buf, &["context", "show"]).await {
            available = true;
            let ctx = ctx_out.trim().to_string();
            ctx_name = ctx.clone();
            
            // docker context inspect <ctx>
            if let Ok(info) = cmd_out(&docker_bin, &cwd_buf, &["context", "inspect", &ctx]).await {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&info) {
                    let host = v
                        .get(0)
                        .and_then(|x| x.get("Endpoints"))
                        .and_then(|x| x.get("docker"))
                        .and_then(|x| x.get("Host"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    socket_path = host.clone();
                    host_raw = host.clone();
                    remote_host = if host.starts_with("ssh://") || host.starts_with("tcp://") {
                        host.split("//").nth(1).and_then(|s| s.split('@').last()).and_then(|s| s.split(':').next()).unwrap_or("localhost").to_string()
                    } else {
                        "localhost".to_string()
                    };
                    backend = classify(&ctx, &host);
                }
            }
        } else {
            // 2. Fallback: just check if docker binary exists/works at all
            if let Ok(o) = Command::new(&docker_bin).args(["--version"]).output().await {
                if o.status.success() {
                    available = true;
                }
            }
        }

        // Build bollard client depending on the transport type
        let client = if available {
            if host_raw.starts_with("ssh://") {
                // bollard does not support SSH — set env var so the docker binary works,
                // but leave the bollard client as None. CLI commands will still function.
                std::env::set_var("DOCKER_HOST", &host_raw);
                None
            } else if host_raw.starts_with("tcp://") || host_raw.starts_with("http://") {
                std::env::set_var("DOCKER_HOST", &host_raw);
                let addr = host_raw.trim_start_matches("tcp://").trim_start_matches("http://");
                bollard::Docker::connect_with_http(addr, 120, bollard::API_DEFAULT_VERSION)
                    .ok()
            } else if !host_raw.is_empty() {
                // unix socket path, e.g. unix:///var/run/docker.sock
                std::env::set_var("DOCKER_HOST", &host_raw);
                let mut d = bollard::Docker::connect_with_defaults().ok();
                if let Some(dock) = d {
                    d = dock.negotiate_version().await.ok();
                }
                d
            } else {
                // local default socket
                let mut d = bollard::Docker::connect_with_defaults().ok();
                if let Some(dock) = d {
                    d = dock.negotiate_version().await.ok();
                }
                d
            }
        } else {
            None
        };

        DockerMeta {
            backend,
            context_name: ctx_name,
            socket_path,
            remote_host,
            available,
            docker_bin,
            client,
        }
    }
}

pub(crate) fn classify(context_name: &str, socket_path: &str) -> String {
    let s = format!("{context_name} {socket_path}").to_lowercase();
    if s.contains("colima") {
        "colima".to_string()
    } else {
        "docker".to_string()
    }
}

pub(crate) async fn cmd_out(bin: &str, cwd: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new(bin).current_dir(cwd).args(args).output().await?;
    if !out.status.success() {
        return Err(anyhow!("command failed: {bin} {:?}", args));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Port {
    #[serde(rename = "IP")]
    pub ip: Option<String>,
    #[serde(rename = "PrivatePort")]
    pub private_port: Option<u16>,
    #[serde(rename = "PublicPort")]
    pub public_port: Option<u16>,
    #[serde(rename = "Type")]
    pub port_type: Option<String>,
}

pub enum LogStream {
    Child(tokio::process::Child),
    Task(tokio::task::JoinHandle<()>),
}

impl LogStream {
    #[allow(dead_code)]
    pub fn kill(&mut self) {
        match self {
            LogStream::Child(c) => { let _ = c.start_kill(); }
            LogStream::Task(t) => { t.abort(); }
        }
    }
}
