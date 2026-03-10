use anyhow::{anyhow, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::docker::{cmd_out, DockerMeta, LogStream};

#[derive(Debug, Clone)]
pub struct SwarmService {
    pub id: String,
    pub name: String,
    pub mode: String,
    pub replicas: String,
    pub image: String,
    pub stack: Option<String>,
    pub ports: Vec<crate::docker::Port>,
}

pub async fn list_swarm_services(meta: &DockerMeta, cwd: &Path) -> Vec<SwarmService> {
    let out = cmd_out(
        &meta.docker_bin,
        cwd,
        &["service", "ls", "--format", "{{json .}}"],
    )
    .await
    .unwrap_or_default();

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
            let id       = field(&v, &["ID", "Id", "id"]);
            let name     = field(&v, &["Name", "name"]);
            let mode     = field(&v, &["Mode", "mode"]);
            let replicas = field(&v, &["Replicas", "replicas", "ReplicaCount"]);
            let image    = field(&v, &["Image", "image"]);
            let labels   = field(&v, &["Labels", "labels"]);
            let ports_raw = field(&v, &["Ports", "ports"]);
            let ports    = crate::docker::parse_port_string(&ports_raw);
            
            let mut stack = None;
            if !labels.is_empty() {
                for part in labels.split(',') {
                    if part.starts_with("com.docker.stack.namespace=") {
                        stack = Some(part.replace("com.docker.stack.namespace=", ""));
                        break;
                    }
                }
            }
            
            if stack.is_none() && name.contains('_') {
                stack = Some(name.split('_').next().unwrap().to_string());
            }

            if !id.is_empty() || !name.is_empty() {
                services.push(SwarmService {
                    id: if id.is_empty() { name.clone() } else { id },
                    name, mode, replicas, image, stack, ports,
                });
            }
        }
    }

    if services.is_empty() && !out.is_empty() {
        if let Ok(txt) = cmd_out(&meta.docker_bin, cwd, &["service", "ls"]).await {
            for line in txt.lines().skip(1) {
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() >= 4 {
                    let name = cols[1].to_string();
                    let stack = if name.contains('_') {
                        Some(name.split('_').next().unwrap().to_string())
                    } else {
                        None
                    };
                    let ports_raw = if cols.len() >= 6 { cols[5] } else { "" };
                    services.push(SwarmService {
                        id: cols[0].to_string(),
                        name,
                        mode: cols[2].to_string(),
                        replicas: cols[3].to_string(),
                        image: cols.get(4).unwrap_or(&"").to_string(),
                        stack,
                        ports: crate::docker::parse_port_string(ports_raw),
                    });
                }
            }
        }
    }

    services
}

pub async fn cmd_inspect_service(meta: &DockerMeta, cwd: &Path, id: &str) -> Result<String> {
    cmd_out(&meta.docker_bin, cwd, &["service", "inspect", "--pretty", id]).await
}

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

pub async fn stream_service_logs(
    meta: &DockerMeta,
    cwd: &Path,
    id: &str,
    tail: usize,
) -> Result<(LogStream, tokio::sync::mpsc::UnboundedReceiver<String>)> {
    let mut child = Command::new(&meta.docker_bin)
        .current_dir(cwd)
        .args(["service", "logs", "-f", "--tail", &tail.to_string(), id])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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

    Ok((LogStream::Child(child), rx))
}

pub fn spawn_service_logs_follow(
    meta: &DockerMeta,
    cwd: &std::path::Path,
    id: &str,
    tail: usize,
) -> Result<(LogStream, tokio::sync::mpsc::UnboundedReceiver<String>)> {
    let rt = tokio::runtime::Handle::current();
    tokio::task::block_in_place(|| {
        rt.block_on(async {
            let (s, rx) = stream_service_logs(meta, cwd, id, tail).await?;
            Ok((s, rx))
        })
    })
}

pub async fn find_service_task_container(
    meta: &DockerMeta,
    cwd: &Path,
    service_id: &str,
) -> Option<String> {
    // Try filtering by service ID label first
    let by_id = cmd_out(
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
    .ok()
    .and_then(|out| out.lines().next().map(|s| s.trim().to_string()))
    .filter(|s| !s.is_empty());

    if by_id.is_some() {
        return by_id;
    }

    // Fallback: filter by service name label
    let by_name = cmd_out(
        &meta.docker_bin,
        cwd,
        &[
            "ps",
            "--filter",
            &format!("label=com.docker.swarm.service.name={}", service_id),
            "--filter",
            "status=running",
            "--format",
            "{{.ID}}",
        ],
    )
    .await
    .ok()
    .and_then(|out| out.lines().next().map(|s| s.trim().to_string()))
    .filter(|s| !s.is_empty());

    if by_name.is_some() {
        return by_name;
    }

    // Last fallback: match by container name prefix (Swarm names containers as "<service>.<n>.<id>")
    // Also handles stack services named "<stack>_<service>.<n>.<id>"
    let all = cmd_out(
        &meta.docker_bin,
        cwd,
        &["ps", "--filter", "status=running", "--format", "{{.ID}} {{.Names}}"],
    )
    .await
    .ok()
    .unwrap_or_default();

    for line in all.lines() {
        let mut parts = line.splitn(2, ' ');
        let cid = parts.next().unwrap_or("").trim();
        let names = parts.next().unwrap_or("").trim().to_lowercase();
        let svc = service_id.to_lowercase();
        // Match: service_name.N.xxx  OR  stack_service.N.xxx
        if names.starts_with(&format!("{}.", svc))
            || names.contains(&format!("_{}", svc))
            || names.contains(&format!("_{}_", svc))
        {
            return Some(cid.to_string());
        }
    }

    None
}
