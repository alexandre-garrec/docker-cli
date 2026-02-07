use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

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

    // Batch inspect all IDs at once
    let ports_map = if !ids.is_empty() {
        inspect_ports_batch(meta, cwd, &ids).await.unwrap_or_default()
    } else {
        HashMap::new()
    };

    let mut res = Vec::new();
    for c in summaries {
        let ports = ports_map.get(&c.id).cloned().unwrap_or_default();
        res.push((c, ports));
    }

    res.sort_by_key(|(c, _)| container_name(&c.names).to_lowercase());
    Ok(res)
}

async fn inspect_ports_batch(meta: &DockerMeta, cwd: &Path, ids: &[String]) -> Result<HashMap<String, Vec<Port>>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut args = vec!["inspect"];
    for id in ids {
        args.push(id);
    }

    let out = cmd_out(&meta.docker_bin, cwd, &args).await?;
    let v: Value = serde_json::from_str(&out)?;
    
    let mut result = HashMap::new();

    if let Value::Array(arr) = v {
        for item in arr {
            if let Some(id) = item.get("Id").and_then(|s| s.as_str()) {
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
                result.insert(id.to_string(), ports);
            }
        }
    }

    Ok(result)
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

