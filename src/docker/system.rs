use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::process::Command;
use bollard::query_parameters::{PruneImagesOptions, PruneContainersOptions, PruneNetworksOptions, PruneVolumesOptions};

use crate::docker::{cmd_out, DockerMeta};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerContext {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Current")]
    pub current: bool,
    #[serde(rename = "DockerEndpoint")]
    pub endpoint: String,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemDfRow {
    pub kind: String,
    pub total: String,
    pub active: String,
    pub size: String,
    pub reclaimable: String,
    pub reclaimable_percent: f64,
}

pub async fn get_system_df(meta: &DockerMeta, _cwd: &Path) -> Result<Vec<SystemDfRow>> {
    if let Some(client) = &meta.client {
        let df = client.df(None).await?;
        let mut rows = Vec::new();
        
        if let Some(imgs) = df.images_disk_usage {
            let total = imgs.total_count;
            let size = imgs.items.unwrap_or_default().iter().map(|img| {
                img.get("Size").and_then(|v| v.as_i64()).unwrap_or(0)
            }).sum::<i64>();
            let rec = imgs.reclaimable.unwrap_or(0);
            let percent = if size > 0 { (rec as f64 / size as f64) * 100.0 } else { 0.0 };
            rows.push(SystemDfRow {
                kind: "Images".to_string(),
                total: total.unwrap_or(0).to_string(),
                active: imgs.active_count.unwrap_or(0).to_string(),
                size: format!("{:.2} MB", size as f64 / 1_048_576.0),
                reclaimable: format!("{:.2} MB", rec as f64 / 1_048_576.0),
                reclaimable_percent: percent,
            });
        }

        if let Some(conts) = df.containers_disk_usage {
            let rec = conts.reclaimable.unwrap_or(0);
            rows.push(SystemDfRow {
                kind: "Containers".to_string(),
                total: conts.total_count.unwrap_or(0).to_string(),
                active: conts.active_count.unwrap_or(0).to_string(),
                size: "-".to_string(),
                reclaimable: format!("{:.2} MB", rec as f64 / 1_048_576.0),
                reclaimable_percent: 0.0, // Hard to tell total size of containers easily
            });
        }

        if let Some(vols) = df.volumes_disk_usage {
            let rec = vols.reclaimable.unwrap_or(0);
            rows.push(SystemDfRow {
                kind: "Volumes".to_string(),
                total: vols.total_count.unwrap_or(0).to_string(),
                active: vols.active_count.unwrap_or(0).to_string(),
                size: "-".to_string(),
                reclaimable: format!("{:.2} MB", rec as f64 / 1_048_576.0),
                reclaimable_percent: 0.0,
            });
        }

        Ok(rows)
    } else {
        // Fallback for system df: try to get something, but it's hard to parse without bollard
        // We'll just return an empty vec for now to avoid errors, or try to parse 'docker system df'
        let out = crate::docker::cmd_out(&meta.docker_bin, _cwd, &["system", "df"]).await.unwrap_or_default();
        let mut rows = Vec::new();
        // Very basic parsing
        for line in out.lines() {
            if line.starts_with("Images") || line.starts_with("Containers") || line.starts_with("Local Volumes") {
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() >= 4 {
                    rows.push(SystemDfRow {
                        kind: cols[0].to_string(),
                        total: cols[1].to_string(),
                        active: cols[2].to_string(),
                        size: cols[3].to_string(),
                        reclaimable: cols.get(4).unwrap_or(&"-").to_string(),
                        reclaimable_percent: 0.0,
                    });
                }
            }
        }
        Ok(rows)
    }
}

pub async fn system_prune(meta: &DockerMeta, _cwd: &Path) -> Result<String> {
    if let Some(client) = &meta.client {
        let mut out = String::new();
        
        if let Ok(p) = client.prune_containers(None::<PruneContainersOptions>).await {
            out.push_str(&format!("Pruned {} containers. Reclaimed {} bytes.\n", p.containers_deleted.unwrap_or_default().len(), p.space_reclaimed.unwrap_or(0)));
        }
        if let Ok(p) = client.prune_images(None::<PruneImagesOptions>).await {
            out.push_str(&format!("Pruned {} images. Reclaimed {} bytes.\n", p.images_deleted.unwrap_or_default().len(), p.space_reclaimed.unwrap_or(0)));
        }
        if let Ok(p) = client.prune_networks(None::<PruneNetworksOptions>).await {
            out.push_str(&format!("Pruned {} networks.\n", p.networks_deleted.unwrap_or_default().len()));
        }
        if let Ok(p) = client.prune_volumes(None::<PruneVolumesOptions>).await {
            out.push_str(&format!("Pruned {} volumes. Reclaimed {} bytes.\n", p.volumes_deleted.unwrap_or_default().len(), p.space_reclaimed.unwrap_or(0)));
        }
        
        Ok(out)
    } else {
        let out = crate::docker::cmd_out(&meta.docker_bin, _cwd, &["system", "prune", "-f"]).await?;
        Ok(out)
    }
}
