use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use bollard::query_parameters::{ListImagesOptions, RemoveImageOptions};
use crate::docker::DockerMeta;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerImage {
    pub id: String,
    pub repository: String,
    pub tag: String,
    pub size: String,
    pub created_since: String,
}

pub async fn get_images(meta: &DockerMeta, _cwd: &Path) -> Result<Vec<DockerImage>> {
    if let Some(client) = &meta.client {
        let options = Some(ListImagesOptions {
            all: true,
            ..Default::default()
        });
        let images = client.list_images(options).await?;
        let mut results = Vec::new();
        for img in images {
            let id = img.id.replace("sha256:", "");
            let id_short = if id.len() > 12 { &id[..12] } else { &id }.to_string();
            
            let (repo, tag) = if let Some(tags) = img.repo_tags.first() {
                let parts: Vec<&str> = tags.split(':').collect();
                if parts.len() >= 2 {
                    (parts[0].to_string(), parts[1].to_string())
                } else {
                    (tags.to_string(), "<none>".to_string())
                }
            } else {
                ("<none>".to_string(), "<none>".to_string())
            };

            let size_mb = img.size as f64 / 1_048_576.0;
            
            // For created_since, we'd ideally calculate human-readable time.
            // For now, let's just use the timestamp or a placeholder if we don't want to add chrono yet.
            // The existing app showed "CreatedSince".
            let created_since = format!("{}s ago", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() - img.created as u64);

            results.push(DockerImage {
                id: id_short,
                repository: repo,
                tag,
                size: format!("{:.2} MB", size_mb),
                created_since,
            });
        }
        Ok(results)
    } else {
        let out = crate::docker::cmd_out(&meta.docker_bin, _cwd, &["image", "ls", "--format", "{{json .}}"]).await?;
        let mut results = Vec::new();
        for line in out.lines().filter(|l| !l.trim().is_empty()) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                let id = v.get("ID").and_then(|x| x.as_str()).unwrap_or_default().to_string();
                results.push(DockerImage {
                    id: if id.starts_with("sha256:") { id[7..19].to_string() } else { id[..id.len().min(12)].to_string() },
                    repository: v.get("Repository").and_then(|x| x.as_str()).unwrap_or("<none>").to_string(),
                    tag: v.get("Tag").and_then(|x| x.as_str()).unwrap_or("<none>").to_string(),
                    size: v.get("Size").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    created_since: v.get("CreatedSince").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                });
            }
        }
        Ok(results)
    }
}

pub async fn rm_image(meta: &DockerMeta, _cwd: &Path, id: &str, force: bool) -> Result<()> {
    if let Some(client) = &meta.client {
        let options = Some(RemoveImageOptions {
            force,
            ..Default::default()
        });
        client.remove_image(id, options, None).await?;
        Ok(())
    } else {
        let mut args = vec!["image", "rm"];
        if force { args.push("-f"); }
        args.push(id);
        
        let status = tokio::process::Command::new(&meta.docker_bin)
            .current_dir(_cwd)
            .args(args)
            .status()
            .await?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow!("docker image rm failed"))
        }
    }
}
