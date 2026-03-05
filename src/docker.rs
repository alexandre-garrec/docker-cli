use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct DockerMeta {
    pub backend: String,
    pub context_name: String,
    #[allow(dead_code)]
    pub socket_path: String,
    pub remote_host: String,
    pub available: bool,
    pub docker_bin: String,
}

impl DockerMeta {
    pub async fn detect(cwd: &Path, docker_bin: &str) -> Self {
        let docker_bin = docker_bin.to_string();
        let cwd_buf = cwd.to_path_buf();

        let context_check = async {
            let mut ctx_name = "default".to_string();
            let mut backend = "unknown".to_string();
            let mut socket_path = "".to_string();
            let mut remote_host = "localhost".to_string();

            // docker context show
            if let Ok(ctx_out) = cmd_out(&docker_bin, &cwd_buf, &["context", "show"]).await {
                 let ctx = ctx_out.trim().to_string();
                 ctx_name = ctx.clone();
                 // docker context inspect <ctx>
                 if let Ok(info) = cmd_out(&docker_bin, &cwd_buf, &["context", "inspect", &ctx]).await {
                    if let Ok(v) = serde_json::from_str::<Value>(&info) {
                        let host = v
                            .get(0)
                            .and_then(|x| x.get("Endpoints"))
                            .and_then(|x| x.get("docker"))
                            .and_then(|x| x.get("Host"))
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                        socket_path = host.clone();
                        remote_host = if host.starts_with("ssh://") || host.starts_with("tcp://") {
                            host.split("//").nth(1).and_then(|s| s.split('@').last()).and_then(|s| s.split(':').next()).unwrap_or("localhost").to_string()
                        } else {
                            "localhost".to_string()
                        };
                        backend = classify(&ctx, &host);
                    }
                 }
            }
            (ctx_name, backend, socket_path, remote_host)
        };

        let availability_check = async {
            Command::new(&docker_bin)
                .current_dir(&cwd_buf)
                .args(["info"]) // cheaper than ping and works for ssh context
                .output()
                .await
                .map(|o| o.status.success())
                .unwrap_or(false)
        };

        let ((context_name, backend, socket_path, remote_host), available) = tokio::join!(context_check, availability_check);

        DockerMeta {
            backend,
            context_name,
            socket_path,
            remote_host,
            available,
            docker_bin,
        }
    }
}

fn classify(context_name: &str, socket_path: &str) -> String {
    let s = format!("{context_name} {socket_path}").to_lowercase();
    if s.contains("colima") {
        "colima".to_string()
    } else {
        "docker".to_string()
    }
}

async fn cmd_out(bin: &str, cwd: &Path, args: &[&str]) -> Result<String> {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerSummary {
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "Names")]
    pub names: String,
    #[serde(rename = "State")]
    pub state: String,
    #[serde(rename = "Status")]
    pub status: String,
    #[serde(rename = "Ports")]
    pub ports: String,
    /// Docker Compose project name (from label com.docker.compose.project), filled after inspect
    #[serde(skip)]
    pub compose_project: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerContext {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Current")]
    pub current: bool,
    #[serde(rename = "DockerEndpoint")]
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerImage {
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "Repository")]
    pub repository: String,
    #[serde(rename = "Tag")]
    pub tag: String,
    #[serde(rename = "Size")]
    pub size: String,
    #[serde(rename = "CreatedSince")]
    pub created_since: String,
}

pub async fn list_contexts(meta: &DockerMeta, cwd: &Path) -> Vec<DockerContext> {
    let out = cmd_out(&meta.docker_bin, cwd, &["context", "ls", "--format", "{{json .}}"]).await.unwrap_or_default();
    let mut results = Vec::new();
    for line in out.lines() {
        if let Ok(ctx) = serde_json::from_str::<DockerContext>(line) {
            results.push(ctx);
        }
    }
    results
}

pub async fn use_context(meta: &DockerMeta, cwd: &Path, name: &str) -> Result<()> {
    let out = Command::new(&meta.docker_bin).current_dir(cwd).args(["context", "use", name]).output().await?;
    if !out.status.success() {
        return Err(anyhow!("Failed to switch context: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(())
}

pub async fn get_images(meta: &DockerMeta, cwd: &Path) -> Result<Vec<DockerImage>> {
    let out = cmd_out(&meta.docker_bin, cwd, &["image", "ls", "--format", "{{json .}}"]).await.unwrap_or_default();
    let mut results = Vec::new();
    for line in out.lines() {
        if let Ok(img) = serde_json::from_str::<DockerImage>(line) {
            results.push(img);
        }
    }
    Ok(results)
}

// ── Volumes & Networks ───────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DockerVolume {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Driver")]
    pub driver: String,
    #[serde(rename = "Size")]
    pub size: Option<String>,
}

pub async fn get_volumes(meta: &DockerMeta, cwd: &Path) -> Result<Vec<DockerVolume>> {
    let out = cmd_out(&meta.docker_bin, cwd, &["volume", "ls", "--format", "{{json .}}"]).await.unwrap_or_default();
    let mut results = Vec::new();
    for line in out.lines() {
        if let Ok(vol) = serde_json::from_str::<DockerVolume>(line) {
            results.push(vol);
        }
    }
    Ok(results)
}

pub async fn rm_volume(meta: &DockerMeta, cwd: &Path, name: &str, force: bool) -> Result<()> {
    let mut args = vec!["volume", "rm"];
    if force {
        args.push("-f");
    }
    args.push(name);
    let status = Command::new(&meta.docker_bin).current_dir(cwd).args(&args).status().await?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("Failed to remove volume {}", name))
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DockerNetwork {
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Driver")]
    pub driver: String,
    #[serde(rename = "Scope")]
    pub scope: String,
}

pub async fn get_networks(meta: &DockerMeta, cwd: &Path) -> Result<Vec<DockerNetwork>> {
    let out = cmd_out(&meta.docker_bin, cwd, &["network", "ls", "--format", "{{json .}}"]).await.unwrap_or_default();
    let mut results = Vec::new();
    for line in out.lines() {
        if let Ok(net) = serde_json::from_str::<DockerNetwork>(line) {
            results.push(net);
        }
    }
    Ok(results)
}

pub async fn rm_network(meta: &DockerMeta, cwd: &Path, id: &str) -> Result<()> {
    // Network rm doesn't have a -f flag, but we keep the signature for consistency
    let status = Command::new(&meta.docker_bin).current_dir(cwd).args(&["network", "rm", id]).status().await?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("Failed to remove network {}", id))
    }
}

pub async fn rm_image(meta: &DockerMeta, cwd: &Path, id: &str, force: bool) -> Result<()> {
    let mut args = vec!["image", "rm"];
    if force {
        args.push("-f");
    }
    args.push(id);
    let status = Command::new(&meta.docker_bin).current_dir(cwd).args(&args).status().await?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("Failed to remove image {}", id))
    }
}

// ── Swarm service ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SwarmService {
    pub id: String,
    pub name: String,
    pub mode: String,
    pub replicas: String,
    pub image: String,
    pub stack: Option<String>,
}

/// List all swarm services. Returns an empty Vec when the daemon is not in swarm mode
/// or the command otherwise fails.
pub async fn list_swarm_services(meta: &DockerMeta, cwd: &Path) -> Vec<SwarmService> {
    // Try JSON format first (Docker 20+)
    let out = cmd_out(
        &meta.docker_bin,
        cwd,
        &["service", "ls", "--format", "{{json .}}"],
    )
    .await
    .unwrap_or_default();

    // Helper: pick first non-empty value from several key candidates
    fn field<'a>(v: &'a serde_json::Value, keys: &[&str]) -> String {
        for k in keys {
            if let Some(s) = v.get(k).and_then(|x| x.as_str()) {
                if !s.is_empty() { return s.to_string(); }
            }
        }
        String::new()
    }

    let mut services = Vec::new();
    for line in out.lines().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            // Docker uses different cases/names depending on version
            let id       = field(&v, &["ID", "Id", "id"]);
            let name     = field(&v, &["Name", "name"]);
            let mode     = field(&v, &["Mode", "mode"]);
            let replicas = field(&v, &["Replicas", "replicas", "ReplicaCount"]);
            let image    = field(&v, &["Image", "image"]);
            let labels   = field(&v, &["Labels", "labels"]);
            
            let mut stack = None;
            if !labels.is_empty() {
                // Try to find com.docker.stack.namespace in comma-separated labels
                for part in labels.split(',') {
                    if part.starts_with("com.docker.stack.namespace=") {
                        stack = Some(part.replace("com.docker.stack.namespace=", ""));
                        break;
                    }
                }
            }
            
            // Fallback: name contains underscore (stack_service)
            if stack.is_none() && name.contains('_') {
                stack = Some(name.split('_').next().unwrap().to_string());
            }

            if !id.is_empty() || !name.is_empty() {
                services.push(SwarmService {
                    id: if id.is_empty() { name.clone() } else { id },
                    name, mode, replicas, image, stack,
                });
            }
        }
    }

    // Fallback: plain-text parse of `docker service ls` (no --format)
    if services.is_empty() && !out.is_empty() {
        // The JSON lines may have been empty but the command succeeded
        // Try without format flag to get the table
        if let Ok(txt) = cmd_out(&meta.docker_bin, cwd, &["service", "ls"]).await {
            for line in txt.lines().skip(1) { // skip header
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() >= 4 {
                    let name = cols[1].to_string();
                    let stack = if name.contains('_') {
                        Some(name.split('_').next().unwrap().to_string())
                    } else {
                        None
                    };
                    services.push(SwarmService {
                        id: cols[0].to_string(),
                        name,
                        mode: cols[2].to_string(),
                        replicas: cols[3].to_string(),
                        image: cols.get(4).unwrap_or(&"").to_string(),
                        stack,
                    });
                }
            }
        }
    }

    services
}

pub fn container_name(raw_names: &str) -> String {
    // docker ps .Names is a single string (may include comma-separated)
    raw_names
        .split(',')
        .next()
        .unwrap_or("")
        .trim()
        .trim_start_matches('/')
        .to_string()
}


pub async fn list_containers_all(meta: &DockerMeta, cwd: &Path) -> Result<Vec<(ContainerSummary, Vec<Port>)>> {
    // One JSON object per line
    let out = cmd_out(
        &meta.docker_bin,
        cwd,
        &["ps", "-a", "--no-trunc", "--format", "{{json .}}"],
    )
    .await
    .unwrap_or_default();

    let mut summaries = Vec::new();
    let mut ids = Vec::new();

    for line in out.lines().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        if let Ok(mut c) = serde_json::from_str::<ContainerSummary>(line) {
            // Normalize state for UI parity
            if c.state.trim().is_empty() {
                c.state = "unknown".to_string();
            }
            ids.push(c.id.clone());
            summaries.push(c);
        }
    }

    // Batch inspect all IDs at once — also extracts compose project labels
    let (ports_map, project_map) = if !ids.is_empty() {
        inspect_batch(meta, cwd, &ids).await.unwrap_or_default()
    } else {
        (HashMap::new(), HashMap::new())
    };

    let mut res = Vec::new();
    for mut c in summaries {
        let ports = ports_map.get(&c.id).cloned().unwrap_or_default();
        c.compose_project = project_map.get(&c.id).cloned();
        res.push((c, ports));
    }

    res.sort_by_key(|(c, _)| container_name(&c.names).to_lowercase());
    Ok(res)
}

/// Returns (ports_map, compose_project_map) keyed by full container ID.
async fn inspect_batch(
    meta: &DockerMeta,
    cwd: &Path,
    ids: &[String],
) -> Result<(HashMap<String, Vec<Port>>, HashMap<String, String>)> {
    if ids.is_empty() {
        return Ok((HashMap::new(), HashMap::new()));
    }

    let mut args = vec!["inspect"];
    for id in ids {
        args.push(id);
    }

    let out = cmd_out(&meta.docker_bin, cwd, &args).await?;
    let v: Value = serde_json::from_str(&out)?;

    let mut ports_result: HashMap<String, Vec<Port>> = HashMap::new();
    let mut project_result: HashMap<String, String> = HashMap::new();

    if let Value::Array(arr) = v {
        for item in arr {
            if let Some(id) = item.get("Id").and_then(|s| s.as_str()) {
                // ── compose project label ────────────────────────────────
                if let Some(project) = item
                    .get("Config")
                    .and_then(|c| c.get("Labels"))
                    .and_then(|l| l.get("com.docker.compose.project"))
                    .and_then(|p| p.as_str())
                {
                    project_result.insert(id.to_string(), project.to_string());
                }

                // ── ports ────────────────────────────────────────────────
                let ports_obj = item
                    .get("NetworkSettings")
                    .and_then(|x| x.get("Ports"))
                    .cloned()
                    .unwrap_or(Value::Null);

                let mut ports = Vec::new();
                if let Value::Object(map) = ports_obj {
                    for (k, vv) in map {
                        // k like "3000/tcp"
                        let mut parts = k.split('/');
                        let priv_port = parts.next().and_then(|s| s.parse::<u16>().ok());
                        let typ = parts.next().map(|s| s.to_string()).or_else(|| Some("tcp".to_string()));
                        match vv {
                            Value::Null => {
                                ports.push(Port {
                                    ip: None,
                                    private_port: priv_port,
                                    public_port: None,
                                    port_type: typ.clone(),
                                });
                            }
                            Value::Array(arr) => {
                                for e in arr {
                                    ports.push(Port {
                                        ip: e.get("HostIp").and_then(|x| x.as_str()).map(|s| s.to_string()),
                                        private_port: priv_port,
                                        public_port: e
                                            .get("HostPort")
                                            .and_then(|x| x.as_str())
                                            .and_then(|s| s.parse::<u16>().ok()),
                                        port_type: typ.clone(),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
                ports_result.insert(id.to_string(), ports);
            }
        }
    }

    Ok((ports_result, project_result))
}




pub async fn docker_compose(meta: &DockerMeta, cfg_cwd: &Path, profile: &str, args: &[&str]) -> Result<i32> {
    let mut full: Vec<&str> = vec!["compose", "--profile", profile];
    full.extend_from_slice(args);

    let status = Command::new(&meta.docker_bin)
        .current_dir(cfg_cwd)
        .args(full)
        .envs(std::env::vars())
        .status()
        .await?;
    Ok(status.code().unwrap_or(if status.success() { 0 } else { 1 }))
}

pub async fn container_action(meta: &DockerMeta, cwd: &Path, verb: &str, id: &str) -> Result<()> {
    let status = Command::new(&meta.docker_bin)
        .current_dir(cwd)
        .args([verb, id])
        .status()
        .await?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("docker {verb} failed"))
    }
}

pub async fn container_rm_force(meta: &DockerMeta, cwd: &Path, id: &str) -> Result<()> {
    let status = Command::new(&meta.docker_bin)
        .current_dir(cwd)
        .args(["rm", "-f", id])
        .status()
        .await?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("docker rm -f failed"))
    }
}

pub async fn container_inspect(meta: &DockerMeta, cwd: &Path, id: &str) -> Result<Value> {
    let out = cmd_out(&meta.docker_bin, cwd, &["inspect", id]).await?;
    Ok(serde_json::from_str::<Value>(&out)?)
}

/// Fetch swarm service inspect as formatted text.
pub async fn cmd_inspect_service(meta: &DockerMeta, cwd: &Path, id: &str) -> Result<String> {
    let out = cmd_out(&meta.docker_bin, cwd, &["service", "inspect", "--pretty", id]).await?;
    Ok(out)
}

/// Rolling restart a swarm service (service update --force).
pub async fn service_rolling_restart(meta: &DockerMeta, cwd: &Path, id: &str) -> Result<()> {
    let status = Command::new(&meta.docker_bin)
        .current_dir(cwd)
        .args(["service", "update", "--force", id])
        .status()
        .await?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("docker service update --force failed"))
    }
}

pub async fn service_scale(meta: &DockerMeta, cwd: &Path, id: &str, replicas: usize) -> Result<()> {
    let status = Command::new(&meta.docker_bin)
        .current_dir(cwd)
        .args(["service", "scale", &format!("{}={}", id, replicas)])
        .status()
        .await?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("docker service scale failed"))
    }
}

pub async fn service_rm(meta: &DockerMeta, cwd: &Path, id: &str) -> Result<()> {
    let status = Command::new(&meta.docker_bin)
        .current_dir(cwd)
        .args(["service", "rm", id])
        .status()
        .await?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("docker service rm failed"))
    }
}

pub async fn reset_container(meta: &DockerMeta, cwd: &Path, id: &str) -> Result<Vec<String>> {
    let mut log = Vec::new();

    log.push(format!("Inspecting {id}..."));
    let info = container_inspect(meta, cwd, id).await?;
    let mounts = info
        .get(0)
        .and_then(|x| x.get("Mounts"))
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();

    let mut volumes = Vec::new();
    for m in mounts {
        if m.get("Type").and_then(|x| x.as_str()) == Some("volume") {
            if let Some(name) = m.get("Name").and_then(|x| x.as_str()) {
                volumes.push(name.to_string());
            }
        }
    }

    // stop (ignore errors)
    let _ = container_action(meta, cwd, "stop", id).await;
    log.push(format!("Removing container {id}..."));
    container_rm_force(meta, cwd, id).await?;

    for v in volumes {
        log.push(format!("Removing volume {v}..."));
        let status = Command::new(&meta.docker_bin)
            .current_dir(cwd)
            .args(["volume", "rm", &v])
            .status()
            .await;
        if let Err(e) = status {
            log.push(format!("Failed to remove volume {v}: {e}"));
        }
    }

    log.push(format!("✅ Reset complete for {id}"));
    Ok(log)
}

pub struct LogStream {
    pub child: Child,
}

impl LogStream {
    #[allow(dead_code)]
    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}

pub async fn stream_container_logs(
    meta: &DockerMeta,
    cwd: &Path,
    id: &str,
    tail: usize,
) -> Result<(LogStream, tokio::sync::mpsc::UnboundedReceiver<String>)> {
    let mut child = Command::new(&meta.docker_bin)
        .current_dir(cwd)
        .args(["logs", "-f", "--tail", &tail.to_string(), id])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    if let Some(out) = stdout {
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let mut r = BufReader::new(out).lines();
            while let Ok(Some(line)) = r.next_line().await {
                let _ = tx2.send(line);
            }
        });
    }

    if let Some(err) = stderr {
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let mut r = BufReader::new(err).lines();
            while let Ok(Some(line)) = r.next_line().await {
                let _ = tx2.send(line);
            }
        });
    }

    Ok((LogStream { child }, rx))
}

pub async fn stream_service_logs(
    meta: &DockerMeta,
    cwd: &Path,
    id: &str,
    tail: usize,
) -> Result<(LogStream, tokio::sync::mpsc::UnboundedReceiver<String>)> {
    let mut child = Command::new(&meta.docker_bin)
        .current_dir(cwd)
        .args(["service", "logs", "-f", "--tail", &tail.to_string(), id])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    if let Some(out) = stdout {
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let mut r = BufReader::new(out).lines();
            while let Ok(Some(line)) = r.next_line().await {
                let _ = tx2.send(line);
            }
        });
    }

    if let Some(err) = stderr {
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let mut r = BufReader::new(err).lines();
            while let Ok(Some(line)) = r.next_line().await {
                let _ = tx2.send(line);
            }
        });
    }

    Ok((LogStream { child }, rx))
}

pub fn pick_best_public_port(ports: &[Port]) -> Option<u16> {
    let preferred_private = [3000u16, 8025, 54323, 5678, 5173, 4173, 8080, 80, 1337];

    let tcp_ports: Vec<&Port> = ports
        .iter()
        .filter(|p| p.public_port.is_some() && p.port_type.as_deref().unwrap_or("tcp") == "tcp")
        .collect();

    for privp in preferred_private {
        if let Some(hit) = tcp_ports.iter().find(|p| p.private_port == Some(privp)) {
            return hit.public_port;
        }
    }

    let preferred_public = [80u16, 3000, 8080, 54324];
    for pubp in preferred_public {
        if let Some(hit) = tcp_ports.iter().find(|p| p.public_port == Some(pubp)) {
            return hit.public_port;
        }
    }

    tcp_ports.first().and_then(|p| p.public_port)
}

/// Convenience wrapper used by the TUI: returns (tokio::process::Child, rx)
/// so the UI can own/kill the follower process easily.
pub fn spawn_service_logs_follow(
    meta: &DockerMeta,
    cwd: &std::path::Path,
    id: &str,
    tail: usize,
) -> Result<(tokio::process::Child, tokio::sync::mpsc::UnboundedReceiver<String>)> {
    let rt = tokio::runtime::Handle::current();
    tokio::task::block_in_place(|| {
        rt.block_on(async {
            let (s, rx) = stream_service_logs(meta, cwd, id, tail).await?;
            Ok((s.child, rx))
        })
    })
}

/// Helper: for a swarm service, find one running container ID to exec into.
pub async fn find_service_task_container(
    meta: &DockerMeta,
    cwd: &Path,
    service_id: &str,
) -> Option<String> {
    let out = cmd_out(
        &meta.docker_bin,
        cwd,
        &[
            "ps",
            "--filter",
            &format!("label=com.docker.swarm.service.id={}", service_id),
            "--filter",
            "status=running",
            "--format",
            "{{.ID}}",
        ],
    )
    .await
    .ok()?;

    out.lines().next().map(|s| s.trim().to_string())
}

pub fn spawn_logs_follow(
    meta: &DockerMeta,
    cwd: &std::path::Path,
    id: &str,
    tail: usize,
) -> Result<(tokio::process::Child, tokio::sync::mpsc::UnboundedReceiver<String>)> {
    let rt = tokio::runtime::Handle::current();
    // We need to run the async stream_container_logs from a sync context.
    // This function is only called from async code, so we can block_in_place safely.
    tokio::task::block_in_place(|| {
        rt.block_on(async {
            let (s, rx) = stream_container_logs(meta, cwd, id, tail).await?;
            Ok((s.child, rx))
        })
    })
}

pub fn format_container_info(inspect_json: &serde_json::Value) -> String {
    let info = inspect_json.get(0).unwrap_or(inspect_json);

    let id = info.get("Id").and_then(|x| x.as_str()).unwrap_or("");
    let created = info.get("Created").and_then(|x| x.as_str()).unwrap_or("");

    let state = info.get("State").cloned().unwrap_or(serde_json::Value::Null);
    let status = state.get("Status").and_then(|x| x.as_str()).unwrap_or("unknown");
    let pid = state.get("Pid").and_then(|x| x.as_i64()).unwrap_or(0);
    let exit_code = state.get("ExitCode").and_then(|x| x.as_i64()).unwrap_or(0);

    let cfg = info.get("Config").cloned().unwrap_or(serde_json::Value::Null);
    let image = cfg.get("Image").and_then(|x| x.as_str()).unwrap_or("");
    let env = cfg
        .get("Env")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();

    let mounts = info
        .get("Mounts")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();

    let ports_obj = info
        .get("NetworkSettings")
        .and_then(|x| x.get("Ports"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let mut out = Vec::new();
    out.push(format!("ID: {}", &id.chars().take(12).collect::<String>()));
    out.push(format!("Image: {}", image));
    out.push(format!("Status: {} (Pid: {}, Exit: {})", status, pid, exit_code));
    out.push(format!("Created: {}", created));
    out.push(String::new());

    out.push("PORTS:".to_string());
    if let serde_json::Value::Object(map) = ports_obj {
        for (k, v) in map {
            match v {
                serde_json::Value::Null => out.push(format!("  {} (not exposed)", k)),
                serde_json::Value::Array(arr) => {
                    let mapped = arr
                        .iter()
                        .filter_map(|p| {
                            let hip = p.get("HostIp").and_then(|x| x.as_str()).unwrap_or("0.0.0.0");
                            let hp = p.get("HostPort").and_then(|x| x.as_str()).unwrap_or("");
                            if hp.is_empty() { None } else { Some(format!("{}:{}", hip, hp)) }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    out.push(format!("  {} -> {}", k, mapped));
                }
                _ => {}
            }
        }
    } else {
        out.push("  (none)".to_string());
    }

    out.push(String::new());
    out.push("MOUNTS:".to_string());
    if mounts.is_empty() {
        out.push("  (none)".to_string());
    } else {
        for m in mounts {
            let typ = m.get("Type").and_then(|x| x.as_str()).unwrap_or("");
            let src = m.get("Source").and_then(|x| x.as_str()).unwrap_or("");
            let dst = m.get("Destination").and_then(|x| x.as_str()).unwrap_or("");
            out.push(format!("  {}: {} -> {}", typ, src, dst));
        }
    }

    out.push(String::new());
    out.push("ENV VARIABLES:".to_string());
    if env.is_empty() {
        out.push("  (none)".to_string());
    } else {
        let mut envs = env
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect::<Vec<_>>();
        envs.sort();
        for e in envs {
            out.push(format!("  {}", e));
        }
    }

    out.join("\n")
}

// ── Container stats ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ContainerStats {
    pub cpu_percent: f64,
    pub mem_usage_mb: f64,
    pub mem_limit_mb: f64,
    pub net_rx_mb: f64,
    pub net_tx_mb: f64,
    pub block_read_mb: f64,
    pub block_write_mb: f64,
    pub mem_percent: f64,
}

impl ContainerStats {
    /// 0–4 block char for a percentage gauge
    #[allow(dead_code)]
    pub fn gauge_char(pct: f64) -> char {
        match pct as u64 {
            0..=20  => '░',
            21..=40 => '▒',
            41..=60 => '▓',
            _       => '█',
        }
    }

    /// Sidebar label suffix: "CPU █░░░ 12% MEM 256M"
    pub fn sidebar_label(&self) -> String {
        let cpu_pct = self.cpu_percent.min(100.0);
        let gauge: String = (0..4)
            .map(|i| if cpu_pct >= (i as f64 + 1.0) * 25.0 { '█' } else { '░' })
            .collect();
        let mem = if self.mem_usage_mb >= 1024.0 {
            format!("{:.1}G", self.mem_usage_mb / 1024.0)
        } else {
            format!("{:.0}M", self.mem_usage_mb)
        };
        format!(" {gauge} {cpu_pct:.0}% {mem}")
    }
}

fn parse_docker_bytes(s: &str) -> f64 {
    let s = s.trim();
    // e.g. "1.23GiB", "512MiB", "768kB", "1.2MB"
    let (num, unit) = s
        .find(|c: char| c.is_alphabetic())
        .map(|i| (&s[..i], &s[i..]))
        .unwrap_or((s, "B"));
    let n: f64 = num.parse().unwrap_or(0.0);
    match unit.to_lowercase().as_str() {
        "gib" | "gb" => n * 1024.0,
        "mib" | "mb" => n,
        "kib" | "kb" => n / 1024.0,
        _             => n / 1_048_576.0,
    }
}

/// Fetch a single snapshot of stats for one container.
/// Runs `docker stats --no-stream --format "{{json .}}" <id>`.
pub async fn fetch_stats(meta: &DockerMeta, cwd: &Path, id: &str) -> Result<ContainerStats> {
    let out = cmd_out(
        &meta.docker_bin,
        cwd,
        &["stats", "--no-stream", "--format", "{{json .}}", id],
    )
    .await?;

    let line = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let v: Value = serde_json::from_str(line)?;

    // CPUPerc: "12.34%"
    let cpu_str = v.get("CPUPerc").and_then(|x| x.as_str()).unwrap_or("0%");
    let cpu = cpu_str.trim_end_matches('%').parse::<f64>().unwrap_or(0.0);

    // MemUsage: "256MiB / 1GiB"
    let mem_str = v.get("MemUsage").and_then(|x| x.as_str()).unwrap_or("0MiB / 0MiB");
    let parts: Vec<&str> = mem_str.splitn(2, '/').collect();
    let mem_usage_mb = parse_docker_bytes(parts.first().unwrap_or(&"0"));
    let mem_limit_mb = parse_docker_bytes(parts.get(1).unwrap_or(&"0"));

    // NetIO: "1.23MB / 456kB"
    let net_str = v.get("NetIO").and_then(|x| x.as_str()).unwrap_or("0B / 0B");
    let nparts: Vec<&str> = net_str.splitn(2, '/').collect();
    let net_rx_mb = parse_docker_bytes(nparts.first().unwrap_or(&"0"));
    let net_tx_mb = parse_docker_bytes(nparts.get(1).unwrap_or(&"0"));

    // BlockIO: "0B / 0B"
    let block_str = v.get("BlockIO").and_then(|x| x.as_str()).unwrap_or("0B / 0B");
    let bparts: Vec<&str> = block_str.splitn(2, '/').collect();
    let block_read_mb = parse_docker_bytes(bparts.first().unwrap_or(&"0"));
    let block_write_mb = parse_docker_bytes(bparts.get(1).unwrap_or(&"0"));

    let mem_percent = if mem_limit_mb > 0.0 { (mem_usage_mb / mem_limit_mb) * 100.0 } else { 0.0 };

    Ok(ContainerStats { 
        cpu_percent: cpu, 
        mem_usage_mb, 
        mem_limit_mb, 
        net_rx_mb, 
        net_tx_mb, 
        block_read_mb, 
        block_write_mb,
        mem_percent
    })
}

pub async fn list_volumes(meta: &DockerMeta, cwd: &Path) -> Result<String> {
    cmd_out(&meta.docker_bin, cwd, &["volume", "ls", "--format", "{{.Name}}\t{{.Driver}}\t{{.Scope}}"]).await
}

pub async fn list_networks(meta: &DockerMeta, cwd: &Path) -> Result<String> {
    cmd_out(&meta.docker_bin, cwd, &["network", "ls", "--format", "{{.Name}}\t{{.Driver}}\t{{.Scope}}"]).await
}

// ── Compose group actions ─────────────────────────────────────────────────────

/// Run `docker compose -p <project> restart` and collect stdout lines.
pub async fn compose_group_restart(meta: &DockerMeta, cwd: &Path, project: &str) -> Result<Vec<String>> {
    let output = Command::new(&meta.docker_bin)
        .current_dir(cwd)
        .args(["compose", "-p", project, "restart"])
        .output()
        .await?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let mut lines: Vec<String> = stdout.lines().map(|l| l.to_string()).collect();
    lines.extend(stderr.lines().map(|l| l.to_string()));
    Ok(lines)
}

/// Run `docker compose -p <project> up -d` and collect stdout lines.
pub async fn compose_group_up(meta: &DockerMeta, cwd: &Path, project: &str) -> Result<Vec<String>> {
    let output = Command::new(&meta.docker_bin)
        .current_dir(cwd)
        .args(["compose", "-p", project, "up", "-d"])
        .output()
        .await?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let mut lines: Vec<String> = stdout.lines().map(|l| l.to_string()).collect();
    lines.extend(stderr.lines().map(|l| l.to_string()));
    Ok(lines)
}

/// Spawn an interactive shell in the container.
/// Returns (child, stdin, receiver) for integrated TUI terminal.
pub fn spawn_shell(
    meta: &DockerMeta,
    cwd: &Path,
    id: &str,
) -> Result<(Child, ChildStdin, mpsc::UnboundedReceiver<String>)> {
    let mut child = Command::new(&meta.docker_bin)
        .current_dir(cwd)
        .args(["exec", "-i", id, "sh"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdin = child.stdin.take().ok_or_else(|| anyhow!("Failed to take stdin"))?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("Failed to take stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("Failed to take stderr"))?;

    let (tx, rx) = mpsc::unbounded_channel();

    // Spawn reader task for stdout/stderr
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        async fn read_to_tx<R: AsyncReadExt + Unpin>(mut reader: R, tx: mpsc::UnboundedSender<String>) {
            let mut buf = [0u8; 1024];
            while let Ok(n) = reader.read(&mut buf).await {
                if n == 0 { break; }
                let s = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = tx.send(s);
            }
        }
        let t1 = tokio::spawn(read_to_tx(stdout, tx.clone()));
        let t2 = tokio::spawn(read_to_tx(stderr, tx.clone()));
        let _ = tokio::join!(t1, t2);
    });
    Ok((child, stdin, rx))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemDfRow {
    pub kind: String,
    pub total: String,
    pub active: String,
    pub size: String,
    pub reclaimable: String,
}

pub async fn get_system_df(meta: &DockerMeta, cwd: &Path) -> Result<Vec<SystemDfRow>> {
    let out = cmd_out(&meta.docker_bin, cwd, &["system", "df", "--format", "{{json .}}"]).await?;
    
    fn field<'a>(v: &'a serde_json::Value, keys: &[&str]) -> String {
        for k in keys {
            if let Some(s) = v.get(k).and_then(|x| x.as_str()) {
                if !s.is_empty() { return s.to_string(); }
            }
        }
        String::new()
    }

    let mut rows = Vec::new();
    for line in out.lines().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            rows.push(SystemDfRow {
                kind: field(&v, &["Type", "type"]),
                total: field(&v, &["TotalCount", "Count"]),
                active: field(&v, &["ActiveCount", "Active"]),
                size: field(&v, &["Size", "size"]),
                reclaimable: field(&v, &["Reclaimable", "reclaimable"]),
            });
        }
    }
    Ok(rows)
}

pub async fn system_prune(meta: &DockerMeta, cwd: &Path) -> Result<String> {
    cmd_out(&meta.docker_bin, cwd, &["system", "prune", "-f"]).await
}

pub fn spawn_compose_logs(meta: &DockerMeta, cwd: &Path, project: &str, tail: usize) -> Result<(Child, mpsc::UnboundedReceiver<String>)> {
    let mut child = Command::new(&meta.docker_bin)
        .current_dir(cwd)
        .args(["compose", "-p", project, "logs", "-f", "--tail", &tail.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let (tx, rx) = mpsc::unbounded_channel();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    fn stream<R: AsyncBufReadExt + Unpin + Send + 'static>(mut reader: R, tx: mpsc::UnboundedSender<String>) {
        tokio::spawn(async move {
            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line).await {
                if n == 0 { break; }
                let _ = tx.send(line.trim_end().to_string());
                line.clear();
            }
        });
    }

    stream(BufReader::new(stdout), tx.clone());
    stream(BufReader::new(stderr), tx);

    Ok((child, rx))
}
