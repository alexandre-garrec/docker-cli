use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use futures_util::stream::StreamExt;

use crate::docker::{DockerMeta, Port, LogStream};
use bollard::query_parameters::{ListContainersOptions, StatsOptions, LogsOptions, StartContainerOptions, KillContainerOptions, RemoveContainerOptions, RemoveVolumeOptions};
use bollard::exec::CreateExecOptions;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use std::process::Stdio;

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
    #[serde(skip)]
    pub compose_project: Option<String>,
}

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
    #[allow(dead_code)]
    pub fn gauge_char(pct: f64) -> char {
        match pct as u64 {
            0..=20  => '░',
            21..=40 => '▒',
            41..=60 => '▓',
            _       => '█',
        }
    }

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

pub fn container_name(raw_names: &str) -> String {
    raw_names
        .split(',')
        .next()
        .unwrap_or("")
        .trim()
        .trim_start_matches('/')
        .to_string()
}

fn parse_size(s: &str) -> f64 {
    let s = s.to_lowercase();
    let val = s.chars().take_while(|c| c.is_digit(10) || *c == '.').collect::<String>().parse::<f64>().unwrap_or(0.0);
    if s.contains("gib") || s.contains("gb") { val * 1024.0 }
    else if s.contains("mib") || s.contains("mb") { val }
    else if s.contains("kib") || s.contains("kb") { val / 1024.0 }
    else if s.contains("b") { val / 1_048_576.0 }
    else { val }
}

pub async fn list_containers_all(meta: &DockerMeta, cwd: &Path) -> Result<Vec<(ContainerSummary, Vec<Port>)>> {
    if let Some(client) = &meta.client {
        let options = Some(ListContainersOptions {
            all: true,
            ..Default::default()
        });
        let containers = client.list_containers(options).await.unwrap_or_default();
        let mut res = Vec::new();
        for mut c in containers {
            let id = c.id.unwrap_or_default();
            let names = c.names.unwrap_or_default().join(",");
            let state = c.state.map(|s| s.to_string()).unwrap_or_default();
            let status = c.status.unwrap_or_default();
            
            let compose_project = c.labels.as_mut().and_then(|l| l.remove("com.docker.compose.project"));

            let mut parsed_ports = Vec::new();
            if let Some(cports) = c.ports {
                for p in cports {
                    parsed_ports.push(Port {
                        ip: p.ip.clone(),
                        private_port: if p.private_port == 0 { None } else { Some(p.private_port as u16) },
                        public_port: p.public_port.map(|x| x as u16),
                        port_type: p.typ.map(|t| t.to_string()),
                    });
                }
            }

            let summary = ContainerSummary {
                id,
                names,
                state,
                status,
                ports: String::new(),
                compose_project,
            };
            res.push((summary, parsed_ports));
        }
        res.sort_by_key(|(c, _)| container_name(&c.names).to_lowercase());
        return Ok(res);
    }

    // Fallback for SSH / no-bollard contexts: use docker binary
    let out = crate::docker::cmd_out(
        &meta.docker_bin,
        cwd,
        &["ps", "--all", "--format", "{{json .}}"],
    )
    .await?;

    let mut res = Vec::new();
    for line in out.lines().filter(|l| !l.trim().is_empty()) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            let id = v.get("ID").and_then(|x| x.as_str()).unwrap_or_default().to_string();
            let names = v.get("Names").and_then(|x| x.as_str()).unwrap_or_default().to_string();
            let state = v.get("State").and_then(|x| x.as_str()).unwrap_or_default().to_string();
            let status = v.get("Status").and_then(|x| x.as_str()).unwrap_or_default().to_string();
            
            // Labels for compose project
            let labels_raw = v.get("Labels").and_then(|x| x.as_str()).unwrap_or("");
            let mut compose_project = None;
            for part in labels_raw.split(',') {
                if part.starts_with("com.docker.compose.project=") {
                    compose_project = Some(part.replace("com.docker.compose.project=", ""));
                    break;
                }
            }

            let mut parsed_ports = Vec::new();
            let ports_raw = v.get("Ports").and_then(|x| x.as_str()).unwrap_or("");
            if !ports_raw.is_empty() {
                parsed_ports = parse_port_string(ports_raw);
            }

            let summary = ContainerSummary {
                id,
                names,
                state,
                status,
                ports: String::new(),
                compose_project,
            };
            res.push((summary, parsed_ports));
        }
    }
    res.sort_by_key(|(c, _)| container_name(&c.names).to_lowercase());
    Ok(res)
}

pub async fn container_action(meta: &DockerMeta, _cwd: &Path, verb: &str, id: &str) -> Result<()> {
    if let Some(client) = &meta.client {
        match verb {
            "start" => client.start_container(id, None::<StartContainerOptions>).await?,
            "stop" => client.stop_container(id, None).await?,
            "restart" => client.restart_container(id, None).await?,
            "pause" => client.pause_container(id).await?,
            "unpause" => client.unpause_container(id).await?,
            "kill" => client.kill_container(id, None::<KillContainerOptions>).await?,
            _ => return Err(anyhow!("Unsupported verb: {}", verb)),
        }
        Ok(())
    } else {
        let status = tokio::process::Command::new(&meta.docker_bin)
            .current_dir(_cwd)
            .args([verb, id])
            .status()
            .await?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow!("docker {} failed", verb))
        }
    }
}

pub async fn container_rm_force(meta: &DockerMeta, _cwd: &Path, id: &str) -> Result<()> {
    if let Some(client) = &meta.client {
        let options = Some(RemoveContainerOptions {
            force: true,
            v: false,
            link: false,
        });
        client.remove_container(id, options).await?;
        Ok(())
    } else {
        let status = tokio::process::Command::new(&meta.docker_bin)
            .current_dir(_cwd)
            .args(["rm", "-f", id])
            .status()
            .await?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow!("docker rm -f failed"))
        }
    }
}

pub async fn container_inspect(meta: &DockerMeta, _cwd: &Path, id: &str) -> Result<Value> {
    if let Some(client) = &meta.client {
        let info = client.inspect_container(id, None).await?;
        Ok(serde_json::to_value(info)?)
    } else {
        let out = crate::docker::cmd_out(&meta.docker_bin, _cwd, &["inspect", id]).await?;
        let v: Value = serde_json::from_str(&out)?;
        // Inspect outputs an array, take the first element
        if let Some(first) = v.get(0) {
            Ok(first.clone())
        } else {
            Ok(v)
        }
    }
}

pub async fn list_container_files(meta: &DockerMeta, id: &str, path: &str) -> Result<Vec<(String, bool)>> {
    if let Some(client) = &meta.client {
        let exec = client.create_exec(id, CreateExecOptions {
            cmd: Some(vec!["ls", "-a", "-p", path]),
            attach_stdout: Some(true),
            ..Default::default()
        }).await?;

        let mut out = String::new();
        if let bollard::exec::StartExecResults::Attached { mut output, .. } = client.start_exec(&exec.id, None).await? {
            while let Some(Ok(msg)) = output.next().await {
                out.push_str(&msg.to_string());
            }
        }

        let mut files = Vec::new();
        for line in out.lines() {
            let name = line.trim();
            if name.is_empty() || name == "." || name == "./" { continue; }
            let is_dir = name.ends_with('/');
            let clean_name = if is_dir { &name[..name.len()-1] } else { name };
            files.push((clean_name.to_string(), is_dir));
        }
        files.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0))); // Dirs first, then alpha
        Ok(files)
    } else {
        let out = crate::docker::cmd_out(&meta.docker_bin, Path::new("."), &["exec", id, "ls", "-a", "-p", path]).await?;
        let mut files = Vec::new();
        for line in out.lines() {
            let name = line.trim();
            if name.is_empty() || name == "." || name == "./" { continue; }
            let is_dir = name.ends_with('/');
            let clean_name = if is_dir { &name[..name.len()-1] } else { name };
            files.push((clean_name.to_string(), is_dir));
        }
        files.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(files)
    }
}

pub async fn reset_container(meta: &DockerMeta, _cwd: &Path, id: &str) -> Result<Vec<String>> {
    let mut log = Vec::new();
    if let Some(client) = &meta.client {
        log.push(format!("Inspecting {}...", id));
        let info = client.inspect_container(id, None).await.map_err(|e| anyhow!("Failed to inspect: {}", e))?;
        
        let mut volumes = Vec::new();
        if let Some(mounts) = info.mounts {
            for m in mounts {
                if let Some(bollard::models::MountPointTypeEnum::VOLUME) = m.typ {
                    if let Some(name) = m.name {
                        volumes.push(name);
                    }
                }
            }
        }

        let _ = client.stop_container(id, None).await;
        log.push(format!("Removing container {}...", id));
        let _ = client.remove_container(id, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;

        for v in volumes {
            log.push(format!("Removing volume {}...", v));
            if let Err(e) = client.remove_volume(&v, None::<RemoveVolumeOptions>).await {
                log.push(format!("Failed to remove volume {}: {}", v, e));
            }
        }
        log.push(format!("✅ Reset complete for {}", id));

    } else {
        log.push(format!("Inspecting {} via CLI...", id));
        let out = crate::docker::cmd_out(&meta.docker_bin, _cwd, &["inspect", id]).await?;
        let v: Value = serde_json::from_str(&out)?;
        let info = v.get(0).unwrap_or(&v);
        
        let mut volumes = Vec::new();
        if let Some(mounts) = info.get("Mounts").and_then(|m| m.as_array()) {
            for m in mounts {
                if m.get("Type").and_then(|t| t.as_str()) == Some("volume") {
                    if let Some(name) = m.get("Name").and_then(|n| n.as_str()) {
                        volumes.push(name.to_string());
                    }
                }
            }
        }

        log.push(format!("Stopping container {}...", id));
        let _ = tokio::process::Command::new(&meta.docker_bin).current_dir(_cwd).args(["stop", id]).status().await;
        
        log.push(format!("Removing container {}...", id));
        let _ = tokio::process::Command::new(&meta.docker_bin).current_dir(_cwd).args(["rm", "-f", id]).status().await;

        for v in volumes {
            log.push(format!("Removing volume {}...", v));
            let _ = tokio::process::Command::new(&meta.docker_bin).current_dir(_cwd).args(["volume", "rm", &v]).status().await;
        }
        log.push(format!("✅ Reset complete for {}", id));
    }
    Ok(log)
}

pub async fn fetch_stats(meta: &DockerMeta, _cwd: &Path, id: &str) -> Result<ContainerStats> {
    if let Some(client) = &meta.client {
        let options = Some(StatsOptions {
            stream: false,
            one_shot: true,
        });
        let mut stream = client.stats(id, options);
        if let Some(Ok(stats)) = stream.next().await {
            let cpu_usage = match stats.cpu_stats.as_ref().and_then(|c| c.cpu_usage.as_ref()) {
                Some(cu) => cu.total_usage.unwrap_or(0) as f64,
                None => 0.0,
            };
            let precpu_usage = match stats.precpu_stats.as_ref().and_then(|c| c.cpu_usage.as_ref()) {
                Some(cu) => cu.total_usage.unwrap_or(0) as f64,
                None => 0.0,
            };
            let cpu_delta = cpu_usage - precpu_usage;
            
            let system_cpu_usage = stats.cpu_stats.as_ref().and_then(|c| c.system_cpu_usage).unwrap_or(0) as f64;
            let presystem_cpu_usage = stats.precpu_stats.as_ref().and_then(|c| c.system_cpu_usage).unwrap_or(0) as f64;
            let system_cpu_delta = system_cpu_usage - presystem_cpu_usage;

            let online_cpus = stats.cpu_stats.as_ref().and_then(|c| c.online_cpus).unwrap_or(1) as f64;
            let cpu_percent = if system_cpu_delta > 0.0 && cpu_delta > 0.0 {
                (cpu_delta / system_cpu_delta) * online_cpus * 100.0
            } else {
                0.0
            };

            let mem_usage = stats.memory_stats.as_ref().and_then(|m| m.usage).unwrap_or(0) as f64;
            let mem_limit = stats.memory_stats.as_ref().and_then(|m| m.limit).unwrap_or(0) as f64;
            let mem_percent = if mem_limit > 0.0 { (mem_usage / mem_limit) * 100.0 } else { 0.0 };

            let mut net_rx = 0.0;
            let mut net_tx = 0.0;
            if let Some(networks) = stats.networks {
                for (_, net) in networks {
                    net_rx += net.rx_bytes.unwrap_or(0) as f64;
                    net_tx += net.tx_bytes.unwrap_or(0) as f64;
                }
            }

            let mut block_read = 0.0;
            let mut block_write = 0.0;
            if let Some(blkio) = stats.blkio_stats {
                if let Some(recursive) = blkio.io_service_bytes_recursive {
                    for stat in recursive {
                        let op = stat.op.as_deref().unwrap_or("").to_lowercase();
                        if op == "read" { block_read += stat.value.unwrap_or(0) as f64; }
                        else if op == "write" { block_write += stat.value.unwrap_or(0) as f64; }
                    }
                }
            }

            return Ok(ContainerStats {
                cpu_percent,
                mem_usage_mb: mem_usage / 1_048_576.0,
                mem_limit_mb: mem_limit / 1_048_576.0,
                net_rx_mb: net_rx / 1_048_576.0,
                net_tx_mb: net_tx / 1_048_576.0,
                block_read_mb: block_read / 1_048_576.0,
                block_write_mb: block_write / 1_048_576.0,
                mem_percent,
            });
        }
        Err(anyhow!("No stats returned from bollard"))
    } else {
        let out = crate::docker::cmd_out(&meta.docker_bin, _cwd, &["stats", "--no-stream", "--format", "{{json .}}", id]).await.unwrap_or_default();
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&out) {
            let cpu_str = v.get("CPUPerc").and_then(|x| x.as_str()).unwrap_or("0%");
            let cpu_percent = cpu_str.trim_end_matches('%').parse::<f64>().unwrap_or(0.0);
            
            let mem_str = v.get("MemUsage").and_then(|x| x.as_str()).unwrap_or("0 / 0");
            let mem_parts: Vec<&str> = mem_str.split('/').collect();
            let mem_usage_mb = if let Some(p) = mem_parts.get(0) { parse_size(p.trim()) } else { 0.0 };
            let mem_limit_mb = if let Some(p) = mem_parts.get(1) { parse_size(p.trim()) } else { 0.0 };
            let mem_percent = v.get("MemPerc").and_then(|x| x.as_str()).unwrap_or("0%").trim_end_matches('%').parse::<f64>().unwrap_or(0.0);

            let net_str = v.get("NetIO").and_then(|x| x.as_str()).unwrap_or("0 / 0");
            let net_parts: Vec<&str> = net_str.split('/').collect();
            let net_rx_mb = if let Some(p) = net_parts.get(0) { parse_size(p.trim()) } else { 0.0 };
            let net_tx_mb = if let Some(p) = net_parts.get(1) { parse_size(p.trim()) } else { 0.0 };

            let block_str = v.get("BlockIO").and_then(|x| x.as_str()).unwrap_or("0 / 0");
            let block_parts: Vec<&str> = block_str.split('/').collect();
            let block_read_mb = if let Some(p) = block_parts.get(0) { parse_size(p.trim()) } else { 0.0 };
            let block_write_mb = if let Some(p) = block_parts.get(1) { parse_size(p.trim()) } else { 0.0 };

            Ok(ContainerStats {
                cpu_percent,
                mem_usage_mb,
                mem_limit_mb,
                net_rx_mb,
                net_tx_mb,
                block_read_mb,
                block_write_mb,
                mem_percent,
            })
        } else {
            Err(anyhow!("Failed to parse stats"))
        }
    }
}

pub async fn stream_container_logs(
    meta: &DockerMeta,
    _cwd: &Path,
    id: &str,
    tail: usize,
) -> Result<(LogStream, tokio::sync::mpsc::UnboundedReceiver<String>)> {
    if let Some(client) = &meta.client {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let options = Some(LogsOptions {
            follow: true,
            stdout: true,
            stderr: true,
            tail: tail.to_string(),
            ..Default::default()
        });
        
        let mut stream = client.logs(id, options);
        let task = tokio::spawn(async move {
            while let Some(Ok(log)) = stream.next().await {
                let msg = match log {
                    bollard::container::LogOutput::StdOut { message } => String::from_utf8_lossy(&message).to_string(),
                    bollard::container::LogOutput::StdErr { message } => String::from_utf8_lossy(&message).to_string(),
                    bollard::container::LogOutput::Console { message } => String::from_utf8_lossy(&message).to_string(),
                    bollard::container::LogOutput::StdIn { message: _ } => continue,
                };
                let _ = tx.send(msg);
            }
        });

        Ok((LogStream::Task(task), rx))
    } else {
        let mut child = Command::new(&meta.docker_bin)
            .current_dir(_cwd)
            .args(["logs", "-f", "--tail", &tail.to_string(), id])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        if let Some(mut out) = stdout {
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                while let Ok(n) = out.read(&mut buf).await {
                    if n == 0 { break; }
                    let _ = tx2.send(String::from_utf8_lossy(&buf[..n]).to_string());
                }
            });
        }
        if let Some(mut err) = stderr {
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                while let Ok(n) = err.read(&mut buf).await {
                    if n == 0 { break; }
                    let _ = tx2.send(String::from_utf8_lossy(&buf[..n]).to_string());
                }
            });
        }

        Ok((LogStream::Child(child), rx))
    }
}

pub fn spawn_logs_follow(
    meta: &DockerMeta,
    cwd: &std::path::Path,
    id: &str,
    tail: usize,
) -> Result<(LogStream, tokio::sync::mpsc::UnboundedReceiver<String>)> {
    let rt = tokio::runtime::Handle::current();
    tokio::task::block_in_place(|| {
        rt.block_on(async {
            stream_container_logs(meta, cwd, id, tail).await
        })
    })
}

pub async fn spawn_shell(
    meta: &DockerMeta,
    _cwd: &Path,
    id: &str,
) -> Result<(LogStream, std::pin::Pin<Box<dyn tokio::io::AsyncWrite + Send>>, tokio::sync::mpsc::UnboundedReceiver<String>)> {
    if let Some(client) = &meta.client {
        use bollard::exec::{CreateExecOptions, StartExecOptions};

        let config = CreateExecOptions {
            attach_stdin: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            tty: Some(true),
            cmd: Some(vec!["sh"]),
            ..Default::default()
        };

        let exec_instance = client.create_exec(id, config).await?;
        let start_res = client.start_exec(&exec_instance.id, None::<StartExecOptions>).await?;

        if let bollard::exec::StartExecResults::Attached { mut output, input } = start_res {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            
            let handle = tokio::spawn(async move {
                while let Some(Ok(log)) = output.next().await {
                    let s = match log {
                        bollard::container::LogOutput::StdOut { message } => String::from_utf8_lossy(&message).to_string(),
                        bollard::container::LogOutput::StdErr { message } => String::from_utf8_lossy(&message).to_string(),
                        bollard::container::LogOutput::Console { message } => String::from_utf8_lossy(&message).to_string(),
                        _ => String::new(),
                    };
                    if !s.is_empty() {
                        let _ = tx.send(s);
                    }
                }
            });

            Ok((LogStream::Task(handle), input, rx))
        } else {
            Err(anyhow!("Failed to attach to exec session"))
        }
    } else {
        let mut child = Command::new(&meta.docker_bin)
            .current_dir(_cwd)
            .args(["exec", "-i", id, "sh"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = Box::pin(child.stdin.take().unwrap());
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let tx_out = tx.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            while let Ok(n) = stdout.read(&mut buf).await {
                if n == 0 { break; }
                let _ = tx_out.send(String::from_utf8_lossy(&buf[..n]).to_string());
            }
        });
        let tx_err = tx;
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            while let Ok(n) = stderr.read(&mut buf).await {
                if n == 0 { break; }
                let s = String::from_utf8_lossy(&buf[..n]).to_string();
                // Filter out the harmless "can't access tty" warning from sh
                if !s.contains("can't access tty") {
                    let _ = tx_err.send(s);
                }
            }
        });

        Ok((LogStream::Child(child), stdin, rx))
    }
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

pub fn parse_port_string(raw: &str) -> Vec<Port> {
    let mut results = Vec::new();
    // Example: "0.0.0.0:80->80/tcp, :::80->80/tcp, 443/tcp"
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() { continue; }

        let mut p = Port {
            ip: None,
            private_port: None,
            public_port: None,
            port_type: None,
        };

        // Split "0.0.0.0:80->80/tcp" or "80/tcp"
        if part.contains("->") {
            let sides: Vec<&str> = part.split("->").collect();
            if sides.len() == 2 {
                // Public side: "0.0.0.0:80" or "[::]:80"
                let host_side = sides[0];
                if let Some(last_colon) = host_side.rfind(':') {
                    let ip = &host_side[..last_colon];
                    let pub_port = &host_side[last_colon+1..];
                    p.ip = Some(ip.to_string());
                    p.public_port = pub_port.parse::<u16>().ok();
                }

                // Private side: "80/tcp"
                let container_side = sides[1];
                let bits: Vec<&str> = container_side.split('/').collect();
                p.private_port = bits[0].parse::<u16>().ok();
                if bits.len() > 1 {
                    p.port_type = Some(bits[1].to_string());
                }
            }
        } else {
            // Just "80/tcp"
            let bits: Vec<&str> = part.split('/').collect();
            p.private_port = bits[0].parse::<u16>().ok();
            if bits.len() > 1 {
                p.port_type = Some(bits[1].to_string());
            }
        }
        results.push(p);
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_port_string() {
        let ports = parse_port_string("0.0.0.0:80->80/tcp, :::80->80/tcp, 443/tcp");
        assert_eq!(ports.len(), 3);
        
        assert_eq!(ports[0].ip, Some("0.0.0.0".to_string()));
        assert_eq!(ports[0].public_port, Some(80));
        assert_eq!(ports[0].private_port, Some(80));
        assert_eq!(ports[0].port_type, Some("tcp".to_string()));

        assert_eq!(ports[1].ip, Some("::".to_string()));
        assert_eq!(ports[1].public_port, Some(80));
        assert_eq!(ports[1].private_port, Some(80));
        assert_eq!(ports[1].port_type, Some("tcp".to_string()));

        assert_eq!(ports[2].ip, None);
        assert_eq!(ports[2].public_port, None);
        assert_eq!(ports[2].private_port, Some(443));
        assert_eq!(ports[2].port_type, Some("tcp".to_string()));
    }

    #[test]
    fn test_parse_port_string_simple() {
        let ports = parse_port_string("8080/tcp");
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].private_port, Some(8080));
        assert_eq!(ports[0].public_port, None);
    }
}

