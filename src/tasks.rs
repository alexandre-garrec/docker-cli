use anyhow::Result;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Run,
    Ok,
    Fail,
    Stop,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "…",
            TaskStatus::Run => "RUN",
            TaskStatus::Ok => "OK",
            TaskStatus::Fail => "FAIL",
            TaskStatus::Stop => "STOP",
        }
    }
}

#[cfg(unix)]
fn spawn_shell(cmd: &str, cwd: &std::path::Path) -> Result<Child> {


    let mut c = Command::new("sh");
    c.arg("-lc")
        .arg(cmd)
        .current_dir(cwd)
        .envs(std::env::vars())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // Start a new process group so we can terminate the group (parity with JS: detached: true)
    unsafe {
        c.pre_exec(|| {
            nix::unistd::setpgid(nix::unistd::Pid::from_raw(0), nix::unistd::Pid::from_raw(0))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            Ok(())
        });
    }

    Ok(c.spawn()?)
}

#[cfg(not(unix))]
fn spawn_shell(cmd: &str, cwd: &std::path::Path) -> Result<Child> {
    let mut c = Command::new("cmd");
    c.arg("/C")
        .arg(cmd)
        .current_dir(cwd)
        .envs(std::env::vars())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    Ok(c.spawn()?)
}

#[cfg(unix)]
pub fn kill_process_group(child: &Child) {
    if let Some(pid) = child.id() {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(-(pid as i32)),
            nix::sys::signal::Signal::SIGTERM,
        );
    }
}

#[cfg(not(unix))]
pub fn kill_process_group(_child: &Child) {
    // Best-effort: Windows needs Job Objects to reliably terminate process trees.
}

/// Spawn a task and return (child, receiver of output lines).
/// Lines are tagged with [OUT] / [ERR] to mirror the JS UI.
pub fn spawn_task(cmd: &str, cwd: &std::path::Path) -> Result<(Child, mpsc::UnboundedReceiver<String>)> {
    let mut child = spawn_shell(cmd, cwd)?;

    let (tx, rx) = mpsc::unbounded_channel::<String>();

    if let Some(stdout) = child.stdout.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut r = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = r.next_line().await {
                let line = line.trim_end().to_string();
                if !line.trim().is_empty() {
                    let _ = tx.send(format!("[OUT] {line}"));
                }
            }
        });
    }

    if let Some(stderr) = child.stderr.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut r = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = r.next_line().await {
                let line = line.trim_end().to_string();
                if !line.trim().is_empty() {
                    let _ = tx.send(format!("[ERR] {line}"));
                }
            }
        });
    }

    Ok((child, rx))
}
