use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use bollard::query_parameters::{ListVolumesOptions, RemoveVolumeOptions};
use crate::docker::DockerMeta;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerVolume {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Driver")]
    pub driver: String,
    #[serde(rename = "Size")]
    pub size: Option<String>,
}

pub async fn get_volumes(meta: &DockerMeta, _cwd: &Path) -> Result<Vec<DockerVolume>> {
    if let Some(client) = &meta.client {
        let options = Some(ListVolumesOptions {
            ..Default::default()
        });
        let response = client.list_volumes(options).await?;
        let mut results = Vec::new();
        if let Some(volumes) = response.volumes {
            for vol in volumes {
                results.push(DockerVolume {
                    name: vol.name,
                    driver: vol.driver,
                    size: None, // Bollard's Volume doesn't always provide size in summary
                });
            }
        }
        Ok(results)
    } else {
        let out = crate::docker::cmd_out(&meta.docker_bin, _cwd, &["volume", "ls", "--format", "{{json .}}"]).await?;
        let mut results = Vec::new();
        for line in out.lines().filter(|l| !l.trim().is_empty()) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                results.push(DockerVolume {
                    name: v.get("Name").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    driver: v.get("Driver").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    size: v.get("Size").and_then(|x| x.as_str()).map(|s| s.to_string()),
                });
            }
        }
        Ok(results)
    }
}

pub async fn rm_volume(meta: &DockerMeta, _cwd: &Path, name: &str, force: bool) -> Result<()> {
    if let Some(client) = &meta.client {
        let options = Some(RemoveVolumeOptions {
            force,
        });
        client.remove_volume(name, options).await?;
        Ok(())
    } else {
        let mut args = vec!["volume", "rm"];
        if force { args.push("-f"); }
        args.push(name);
        
        let status = tokio::process::Command::new(&meta.docker_bin)
            .current_dir(_cwd)
            .args(args)
            .status()
            .await?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow!("docker volume rm failed"))
        }
    }
}

