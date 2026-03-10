use anyhow::Result;
use std::path::Path;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use std::process::Stdio;

use crate::docker::{DockerMeta, LogStream};

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

pub async fn compose_group_down(meta: &DockerMeta, cwd: &Path, project: &str) -> Result<Vec<String>> {
    let output = Command::new(&meta.docker_bin)
        .current_dir(cwd)
        .args(["compose", "-p", project, "down"])
        .output()
        .await?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let mut lines: Vec<String> = stdout.lines().map(|l| l.to_string()).collect();
    lines.extend(stderr.lines().map(|l| l.to_string()));
    Ok(lines)
}


pub fn spawn_compose_logs(meta: &DockerMeta, cwd: &Path, project: &str, tail: usize) -> Result<(LogStream, tokio::sync::mpsc::UnboundedReceiver<String>)> {
    let mut child = Command::new(&meta.docker_bin)
        .current_dir(cwd)
        .args(["compose", "-p", project, "logs", "-f", "--tail", &tail.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    fn stream<R: AsyncBufReadExt + Unpin + Send + 'static>(mut reader: R, tx: tokio::sync::mpsc::UnboundedSender<String>) {
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

    Ok((LogStream::Child(child), rx))
}
