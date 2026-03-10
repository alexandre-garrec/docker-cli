use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use bollard::query_parameters::ListNetworksOptions;
use crate::docker::DockerMeta;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerNetwork {
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Driver")]
    pub driver: String,
    #[serde(rename = "Scope")]
    pub scope: String,
}

pub async fn get_networks(meta: &DockerMeta, _cwd: &Path) -> Result<Vec<DockerNetwork>> {
    if let Some(client) = &meta.client {
        let options = Some(ListNetworksOptions {
            ..Default::default()
        });
        let networks = client.list_networks(options).await?;
        let mut results = Vec::new();
        for net in networks {
            let id = net.id.unwrap_or_default();
            let id_short = if id.len() > 12 { &id[..12] } else { &id }.to_string();
            results.push(DockerNetwork {
                id: id_short,
                name: net.name.unwrap_or_default(),
                driver: net.driver.unwrap_or_default(),
                scope: net.scope.unwrap_or_default(),
            });
        }
        Ok(results)
    } else {
        let out = crate::docker::cmd_out(&meta.docker_bin, _cwd, &["network", "ls", "--format", "{{json .}}"]).await?;
        let mut results = Vec::new();
        for line in out.lines().filter(|l| !l.trim().is_empty()) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                let id = v.get("ID").and_then(|x| x.as_str()).unwrap_or_default().to_string();
                results.push(DockerNetwork {
                    id: if id.len() > 12 { id[..12].to_string() } else { id },
                    name: v.get("Name").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    driver: v.get("Driver").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    scope: v.get("Scope").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                });
            }
        }
        Ok(results)
    }
}

pub async fn rm_network(meta: &DockerMeta, _cwd: &Path, id: &str) -> Result<()> {
    if let Some(client) = &meta.client {
        client.remove_network(id).await?;
        Ok(())
    } else {
        let status = tokio::process::Command::new(&meta.docker_bin)
            .current_dir(_cwd)
            .args(["network", "rm", id])
            .status()
            .await?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow!("docker network rm failed"))
        }
    }
}

