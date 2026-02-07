use crate::env::{get_profile_value, parse_post_up_tasks};
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct TaskSpec {
    pub name: String,
    pub cmd: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub cwd: PathBuf,
    pub profile: String,
    #[allow(dead_code)]
    pub docker_bin: String,

    pub auto_compose_up: bool,
    pub compose_profile: String,

    pub db_container: String,
    pub storage_container: String,

    pub post_up_tasks: Vec<TaskSpec>,

    pub max_log_lines: usize,
    pub refresh_ms: u64,
}

pub fn resolve_docker_binary() -> String {
    std::env::var("DOCKER_BIN").unwrap_or_else(|_| "docker".to_string())
}

pub fn find_project_root(start_dir: &Path) -> PathBuf {
    // Walk up until we find docker-compose.yml (preferred). If we only find package.json,
    // keep it as fallback but continue searching for docker-compose.yml.
    let mut dir = start_dir.to_path_buf();
    let mut fallback: Option<PathBuf> = None;

    for _ in 0..12 {
        let compose = dir.join("docker-compose.yml");
        let pkg = dir.join("package.json");

        if compose.exists() {
            return dir;
        }
        if pkg.exists() && fallback.is_none() {
            fallback = Some(dir.clone());
        }

        if let Some(parent) = dir.parent() {
            let parent = parent.to_path_buf();
            if parent == dir {
                break;
            }
            dir = parent;
        } else {
            break;
        }
    }

    fallback.unwrap_or_else(|| start_dir.to_path_buf())
}

fn load_tasks_from_package_json(root: &Path) -> Vec<TaskSpec> {
    let pkg_path = root.join("package.json");
    if !pkg_path.exists() {
        return vec![];
    }

    let file = match File::open(pkg_path) {
        Ok(f) => f,
        Err(_) => return vec![],
    };
    let reader = BufReader::new(file);
    let v: serde_json::Value = match serde_json::from_reader(reader) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let mut tasks = Vec::new();

    if let Some(scripts) = v.get("scripts").and_then(|s| s.as_object()) {
        for (name, cmd) in scripts {
            if let Some(cmd_str) = cmd.as_str() {
                tasks.push(TaskSpec {
                    name: name.clone(),
                    cmd: cmd_str.to_string(),
                });
            }
        }
    }
    // Sort tasks alphabetically for better UI consistency
    tasks.sort_by(|a, b| a.name.cmp(&b.name));
    tasks
}

pub fn get_config(profile: &str) -> Config {
    let prof = if profile.trim().is_empty() {
        "local".to_string()
    } else {
        profile.trim().to_string()
    };

    let tasks_raw = get_profile_value("POST_UP_TASKS", &prof);
    let tasks = parse_post_up_tasks(&tasks_raw);

    // Back-compat: old single command
    let single = get_profile_value("POST_UP_CMD", &prof);
    let mut post_up_tasks = if !tasks.is_empty() {
        tasks
    } else if !single.trim().is_empty() {
        vec![TaskSpec {
            name: "postup".to_string(),
            cmd: single,
        }]
    } else {
        vec![]
    };

    let cwd = find_project_root(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Merge tasks from package.json
    let pkg_tasks = load_tasks_from_package_json(&cwd);
    // Avoid duplicates, prefer env/profile tasks
    for pt in pkg_tasks {
        if !post_up_tasks.iter().any(|t| t.name == pt.name) {
            post_up_tasks.push(pt);
        }
    }
    // Sort combined tasks
    post_up_tasks.sort_by(|a, b| a.name.cmp(&b.name));

    let max_log_lines = std::env::var("MAX_LOG_LINES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1200);

    let refresh_ms = std::env::var("REFRESH_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(1000);

    Config {
        cwd,
        profile: prof.clone(),
        docker_bin: resolve_docker_binary(),
        auto_compose_up: true,
        compose_profile: prof,
        db_container: std::env::var("DB_CONTAINER").unwrap_or_else(|_| "supabase-db".to_string()),
        storage_container: std::env::var("STORAGE_CONTAINER")
            .unwrap_or_else(|_| "supabase-storage".to_string()),
        post_up_tasks,
        max_log_lines,
        refresh_ms,
    }
}

#[allow(dead_code)]
fn _compose_files_for_profile(cwd: &Path, profile: &str) -> Vec<PathBuf> {
    // Kept for parity with JS code; the current implementation does not pass -f flags.
    let mut files = vec![cwd.join("docker-compose.yml")];
    let prof = if profile.trim().is_empty() { "local" } else { profile.trim() };
    let candidate = PathBuf::from("docker").join(prof).join("docker-compose.yml");
    if cwd.join(&candidate).exists() {
        files.push(cwd.join(candidate));
    }
    files
}

#[allow(dead_code)]
pub fn ensure_dir_exists(p: &Path) -> anyhow::Result<()> {
    if !p.exists() {
        fs::create_dir_all(p)?;
    }
    Ok(())
}
