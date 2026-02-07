use crate::config::TaskSpec;
use anyhow::Result;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Load .env and optional .env.<profile> into process env.
/// Also performs a few passes of ${VAR} and ${VAR:-default} expansion.
pub fn load_env(cwd: &Path, profile: Option<&str>) -> Result<Vec<String>> {
    let mut loaded: Vec<String> = Vec::new();

    let base = cwd.join(".env");
    if base.exists() {
        dotenvy::from_path(&base).ok();
        loaded.push(".env".to_string());
    }

    if let Some(profile) = profile {
        let prof = profile.trim();
        if !prof.is_empty() {
            let pf = cwd.join(format!(".env.{prof}"));
            if pf.exists() {
                dotenvy::from_path_override(&pf).ok();
                loaded.push(format!(".env.{prof}"));
            } else {
                // JS implementation pushes even if missing; we don't.
            }
        }
    }

    // Multi-pass expansion for ${VAR} and ${VAR:-default}
    for _pass in 0..5 {
        let keys: Vec<String> = std::env::vars().map(|(k, _)| k).collect();
        let mut changes = 0;

        for key in keys {
            if let Ok(val) = std::env::var(&key) {
                if !val.contains("${") {
                    continue;
                }
                let new_val = expand_value(&key, &val);
                if new_val != val {
                    std::env::set_var(&key, new_val);
                    changes += 1;
                }
            }
        }
        if changes == 0 {
            break;
        }
    }

    let uniq: Vec<String> = loaded.into_iter().collect::<HashSet<_>>().into_iter().collect();
    Ok(uniq)
}

fn expand_value(current_key: &str, input: &str) -> String {
    // Regex-free small parser: replace occurrences of ${NAME} or ${NAME:-default}
    let mut out = String::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '{' {
            // find closing }
            let mut j = i + 2;
            while j < chars.len() && chars[j] != '}' {
                j += 1;
            }
            if j >= chars.len() {
                out.push(chars[i]);
                i += 1;
                continue;
            }
            let inner: String = chars[i + 2..j].iter().collect();
            let (name, def) = if let Some(pos) = inner.find(":-") {
                (inner[..pos].to_string(), Some(inner[pos + 2..].to_string()))
            } else {
                (inner, None)
            };

            let mut resolved: Option<String> = None;
            if name != current_key {
                if let Ok(v) = std::env::var(&name) {
                    if !v.is_empty() {
                        resolved = Some(v);
                    }
                }
            }

            if let Some(v) = resolved {
                out.push_str(&v);
            } else if let Some(d) = def {
                out.push_str(&d);
            }

            i = j + 1;
            continue;
        }

        out.push(chars[i]);
        i += 1;
    }
    out
}

pub fn key_for_profile(base_key: &str, profile: &str) -> String {
    let p = profile
        .trim()
        .to_uppercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>();
    if p.is_empty() {
        base_key.to_string()
    } else {
        format!("{base_key}_{p}")
    }
}

pub fn get_profile_value(base_key: &str, profile: &str) -> String {
    let k = key_for_profile(base_key, profile);
    std::env::var(&k)
        .ok()
        .or_else(|| std::env::var(base_key).ok())
        .unwrap_or_default()
}

/// Parse POST_UP_TASKS_<PROFILE>
/// Format (one per line):
///   name::command
/// Ignores empty lines and comments (#).
pub fn parse_post_up_tasks(raw: &str) -> Vec<TaskSpec> {
    let text = raw.trim();
    if text.is_empty() {
        return vec![];
    }

    let mut tasks = Vec::new();
    for line in text.lines().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        if line.starts_with('#') {
            continue;
        }
        if let Some(idx) = line.find("::") {
            let name = line[..idx].trim();
            let cmd = line[idx + 2..].trim();
            if !cmd.is_empty() {
                tasks.push(TaskSpec {
                    name: if name.is_empty() { "task".to_string() } else { name.to_string() },
                    cmd: cmd.to_string(),
                });
            }
        } else {
            tasks.push(TaskSpec {
                name: "task".to_string(),
                cmd: line.to_string(),
            });
        }
    }
    tasks
}

#[allow(dead_code)]
pub fn read_file_if_exists(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok()
}
