# docker-cli (Rust)

This tool allows you to manage your Docker stack and local tasks (scripts from `package.json`, etc.) through a fast and fluid Terminal User Interface (TUI).

<img width="914" height="695" alt="Capture d’écran 2026-02-07 à 23 37 14" src="https://github.com/user-attachments/assets/aece013d-c674-4179-be35-302dda7f36f7" />

## Features

- **Project Auto-detection**:
  - Automatically detects project root via `docker-compose.yml` or `package.json`.
  - Loads scripts from `package.json` as executable tasks.
- **Docker Management**:
  - Docker profile support (`DOCKER_PROFILE` / `COMPOSE_PROFILE`, default: `local`).
  - Automatic `docker compose up -d` on launch if needed.
  - Live log streaming.
  - Quick actions: Restart, Stop, Start, Pause, Kill, Remove, Inspect, Reset (Delete + Volumes).
- **User Interface**:
  - Mouse support (click to select, wheel to scroll logs).
  - Fluid keyboard navigation.
  - Open exposed ports in browser (`o`).

## Installation (Add to commands)

To use `docker-cli` as a global command on your machine:

### Option 1: Install via Cargo (Recommended)

If you have Rust installed:

```bash
# Inside efora-dev-rs folder
cargo install --path .
```

This installs the `docker-cli` binary to `~/.cargo/bin`. Ensure this directory is in your `PATH`.
You can then run the command anywhere:

```bash
cd my-project
docker-cli
```

### Option 2: Manual Build and Symlink

If you prefer managing the binary manually:

```bash
# 1. Build release
cargo build --release

# 2. Create a symlink to a PATH directory (e.g., /usr/local/bin)
sudo ln -s $(pwd)/target/release/docker-cli /usr/local/bin/docker-cli
```

## Usage

Simply navigate to your project root (where `package.json` or `docker-compose.yml` is located) and run:

```bash
docker-cli
```

The tool will automatically detect the environment and load available tasks.

## Shortcuts (Keyboard & Mouse)

- **Mouse**:
  - **Left Click**: Select a container or task.
  - **Scroll Wheel**: Scroll through logs.
- **Keyboard**:
  - `Tab`/`Shift+Tab`: Switch focus between list and logs.
  - `Enter`: Attach/View logs (if list is focused).
  - `c`: Docker Compose Up / Restart (with confirmation).
  - `q` / `Ctrl+C`: Quit.
- **Actions on Selection**:
  - `r`: Restart
  - `s`: Stop
  - `t`: Start
  - `p` / `u`: Pause / Unpause (Docker)
  - `k`: Kill (Docker)
  - `d`: Remove (Docker)
  - `x`: Reset (Stop + Rm + Volumes) (Docker)
  - `i`: Inspect (Popup JSON)
  - `o`: Open in browser

## Configuration (Advanced)

The tool works out-of-the-box, but you can customize it via environment variables (or `.env` file):

- `DOCKER_BIN` (default: `docker`)
- `DOCKER_PROFILE` / `COMPOSE_PROFILE` (default: `local`)
- `DB_CONTAINER` (default: `supabase-db`)
- `STORAGE_CONTAINER` (default: `supabase-storage`)
- `MAX_LOG_LINES` (default: `1200`)
- `REFRESH_MS` (default: `1000`)
- `POST_UP_TASKS_<PROFILE>`: Additional manual tasks (format: `name::command` per line).
