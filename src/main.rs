mod config;
mod env;
mod docker;
mod tasks;
mod ui;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let start_dir = std::env::current_dir()?;
    let root = config::find_project_root(&start_dir);

    // Preload base .env (profile-specific env is loaded after profile selection)
    env::load_env(&root, None)?;

    let docker_bin = config::resolve_docker_binary();
    let docker_meta = docker::DockerMeta::detect(&root, &docker_bin).await;

    ui::run(ui::RunOpts {
        root,
        docker_bin,
        docker_meta,
    })
    .await
}
