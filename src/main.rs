mod config;
mod env;
mod docker;
mod tasks;
mod pins;
mod ui;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Install a panic hook that restores the terminal BEFORE printing the
    // panic message, so it doesn't disappear inside the alternate screen.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Best-effort terminal restore
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stderr(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
        );
        default_hook(info);
    }));

    let start_dir = std::env::current_dir()?;
    let root = config::find_project_root(&start_dir);

    // Preload base .env (profile-specific env is loaded after profile selection)
    // Ignore errors when outside a project — .env is optional
    let _ = env::load_env(&root, None);

    let docker_bin = config::resolve_docker_binary();
    let docker_meta = docker::DockerMeta::detect(&root, &docker_bin).await;

    if let Err(e) = ui::run(ui::RunOpts {
        root,
        docker_bin,
        docker_meta,
    })
    .await
    {
        // Ensure terminal is restored then print the error
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stderr(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
        );
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }

    Ok(())
}
