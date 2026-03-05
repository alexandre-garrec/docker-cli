/// Persisted pinned container names — stored in ~/.config/docker-cli/pins.json
use std::collections::HashSet;
use std::path::PathBuf;

fn pins_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("docker-cli")
        .join("pins.json")
}

pub fn load_pins() -> HashSet<String> {
    let p = pins_path();
    if let Ok(data) = std::fs::read_to_string(&p) {
        if let Ok(list) = serde_json::from_str::<Vec<String>>(&data) {
            return list.into_iter().collect();
        }
    }
    HashSet::new()
}

pub fn save_pins(pins: &HashSet<String>) {
    let p = pins_path();
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let list: Vec<&String> = pins.iter().collect();
    if let Ok(json) = serde_json::to_string_pretty(&list) {
        let _ = std::fs::write(&p, json);
    }
}
