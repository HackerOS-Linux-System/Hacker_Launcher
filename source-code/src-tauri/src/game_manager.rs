use crate::config_manager::Settings;
use anyhow::{bail, Context, Result};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

/// Maximum number of historical log files kept per game before the oldest
/// ones are rotated out.
const MAX_LOGS_PER_GAME: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    pub name: String,
    pub exe: String,
    pub runner: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub launch_options: String,
    #[serde(default)]
    pub fps_limit: Option<u32>,
    #[serde(default)]
    pub enable_dxvk: bool,
    #[serde(default)]
    pub enable_esync: bool,
    #[serde(default)]
    pub enable_fsync: bool,
    #[serde(default)]
    pub enable_dxvk_async: bool,
    #[serde(default)]
    pub app_id: String,
    /// Newline (or `;`) separated `KEY=VALUE` pairs the user wants injected
    /// into the launched process' environment, on top of the fixed
    /// WINEESYNC / DXVK ones already handled below.
    #[serde(default)]
    pub env_vars: String,
    /// Optional path to a custom icon/cover image shown in the games list.
    #[serde(default)]
    pub icon_path: String,
    /// Cumulative time (in seconds) this game has been played for.
    #[serde(default)]
    pub total_playtime_secs: u64,
    /// RFC3339 timestamp of the last time this game was launched.
    #[serde(default)]
    pub last_played: Option<String>,
    /// Comma-separated free-form tags/categories (e.g. "RPG, Co-op").
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub favorite: bool,
    /// If true, this game uses the shared prefix (see Settings) instead of
    /// its own per-game prefix in `self.prefix`.
    #[serde(default)]
    pub use_shared_prefix: bool,
    /// Sets `STEAM_COMPAT_DISABLE_STEAM_INPUT=1` for Proton runners.
    #[serde(default)]
    pub disable_steam_input: bool,
    /// Raw value for the `SDL_GAMECONTROLLERCONFIG` env var, for games that
    /// need a custom gamepad mapping.
    #[serde(default)]
    pub sdl_controller_config: String,
}

/// Lightweight, serializable info about a currently-running game, exposed to
/// the frontend so it can show a "Running" badge / elapsed time and offer a
/// "Stop" action.
#[derive(Debug, Clone, Serialize)]
pub struct RunningGameInfo {
    pub name: String,
    pub pid: u32,
    pub started_at: String,
}

/// Metadata about a single historical log file for a game.
#[derive(Debug, Clone, Serialize)]
pub struct GameLogEntry {
    pub file_name: String,
    pub path: String,
    pub modified: String,
}

struct RunningEntry {
    pid: u32,
    started_at: chrono::DateTime<Local>,
}

pub struct GameManager {
    games_file: PathBuf,
    prefixes_dir: PathBuf,
    logs_dir: PathBuf,
    protons_dir: PathBuf,
    settings: Settings,
    running: Arc<Mutex<HashMap<String, RunningEntry>>>,
}

impl GameManager {
    pub fn new(
        games_file: PathBuf,
        prefixes_dir: PathBuf,
        logs_dir: PathBuf,
        protons_dir: PathBuf,
        settings: Settings,
    ) -> Self {
        Self {
            games_file,
            prefixes_dir,
            logs_dir,
            protons_dir,
            settings,
            running: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Called after Settings are saved so global toggles (Esync/Fsync/DXVK
    /// Async, shared prefix path) take effect on the next launch without
    /// requiring an app restart.
    pub fn update_settings(&mut self, settings: Settings) {
        self.settings = settings;
    }

    pub fn load_games(&self) -> Result<Vec<Game>> {
        if !self.games_file.exists() {
            return Ok(vec![]);
        }
        let content = fs::read_to_string(&self.games_file)?;
        let games: Vec<Game> = serde_json::from_str(&content).unwrap_or_default();
        Ok(games)
    }

    fn save_games(&self, games: &[Game]) -> Result<()> {
        let content = serde_json::to_string_pretty(games)?;
        fs::write(&self.games_file, content)?;
        Ok(())
    }

    /// Adds a new game, rejecting the operation if a game with the same name
    /// (case-insensitive) already exists — previously a duplicate name would
    /// silently create two entries that collided on every by-name lookup
    /// (update/remove/launch/log-file naming).
    pub fn add_game(&self, game: Game) -> Result<()> {
        let mut games = self.load_games()?;
        if games
            .iter()
            .any(|g| g.name.eq_ignore_ascii_case(game.name.trim()))
        {
            bail!("A game named \"{}\" already exists", game.name);
        }
        games.push(game);
        self.save_games(&games)
    }

    pub fn remove_game(&self, name: &str) -> Result<()> {
        let mut games = self.load_games()?;
        games.retain(|g| g.name != name);
        self.save_games(&games)
    }

    /// Updates a game. `original_name` identifies which entry to update,
    /// since `updated.name` may itself have changed (a rename). Renaming
    /// onto an existing game's name is rejected to avoid two games merging
    /// into one on-disk record.
    pub fn update_game(&self, updated: Game, original_name: &str) -> Result<()> {
        let mut games = self.load_games()?;
        if !updated.name.eq_ignore_ascii_case(original_name)
            && games
                .iter()
                .any(|g| g.name.eq_ignore_ascii_case(updated.name.trim()))
        {
            bail!("A game named \"{}\" already exists", updated.name);
        }
        let mut found = false;
        for g in games.iter_mut() {
            if g.name == original_name {
                *g = updated.clone();
                found = true;
                break;
            }
        }
        if !found {
            bail!("Game not found: {}", original_name);
        }
        self.save_games(&games)
    }

    /// Internal variant used by launch_game to persist small in-place
    /// updates (auto-assigned prefix, playtime, last_played) without going
    /// through the duplicate-name / rename checks.
    fn save_single_game(&self, game: &Game) -> Result<()> {
        let mut games = self.load_games()?;
        for g in games.iter_mut() {
            if g.name == game.name {
                *g = game.clone();
                break;
            }
        }
        self.save_games(&games)
    }

    pub fn get_running_games(&self) -> Vec<RunningGameInfo> {
        let running = self.running.lock().unwrap();
        running
            .iter()
            .map(|(name, entry)| RunningGameInfo {
                name: name.clone(),
                pid: entry.pid,
                started_at: entry.started_at.to_rfc3339(),
            })
            .collect()
    }

    pub fn is_running(&self, name: &str) -> bool {
        self.running.lock().unwrap().contains_key(name)
    }

    /// Forcefully stops a running game by sending SIGKILL to its process.
    /// Only works for games this manager itself launched and is still
    /// tracking (i.e. present in `running`).
    pub fn stop_game(&self, name: &str) -> Result<()> {
        let pid = {
            let running = self.running.lock().unwrap();
            running
                .get(name)
                .map(|e| e.pid)
                .with_context(|| format!("Game is not currently running: {}", name))?
        };

        #[cfg(unix)]
        {
            let status = std::process::Command::new("kill")
                .arg("-9")
                .arg(pid.to_string())
                .status()
                .with_context(|| format!("Failed to invoke kill for pid {}", pid))?;
            if !status.success() {
                bail!("kill command failed for pid {}", pid);
            }
        }
        #[cfg(not(unix))]
        {
            bail!("Stopping games is only supported on Unix-like systems");
        }

        // The background watcher thread spawned in launch_game will notice
        // the process exit, update playtime and remove it from `running`.
        Ok(())
    }

    /// Lists historical log files for a game, most recent first.
    pub fn list_game_logs(&self, name: &str) -> Result<Vec<GameLogEntry>> {
        let prefix = format!("{}_", sanitize_name(name));
        let mut entries = vec![];
        if !self.logs_dir.exists() {
            return Ok(entries);
        }
        for entry in fs::read_dir(&self.logs_dir)? {
            let entry = entry?;
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with(&prefix) && file_name.ends_with(".log") {
                let modified = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .map(|t| chrono::DateTime::<Local>::from(t).to_rfc3339())
                    .unwrap_or_default();
                entries.push(GameLogEntry {
                    file_name: file_name.clone(),
                    path: entry.path().to_string_lossy().to_string(),
                    modified,
                });
            }
        }
        entries.sort_by(|a, b| b.file_name.cmp(&a.file_name));
        Ok(entries)
    }

    pub fn read_log_file(&self, path: &str) -> Result<String> {
        let p = PathBuf::from(path);
        // Only allow reading files that actually live inside our logs dir.
        let canonical_logs = fs::canonicalize(&self.logs_dir).unwrap_or(self.logs_dir.clone());
        let canonical_target = fs::canonicalize(&p).unwrap_or(p.clone());
        if !canonical_target.starts_with(&canonical_logs) {
            bail!("Refusing to read a log file outside the logs directory");
        }
        Ok(fs::read_to_string(&p)?)
    }

    pub fn launch_game(
        &self,
        mut game: Game,
        gamescope: bool,
        app_handle: tauri::AppHandle,
    ) -> Result<()> {
        use tauri::Emitter;

        if self.is_running(&game.name) {
            bail!("\"{}\" is already running", game.name);
        }

        // Validation
        if game.runner != "Steam" && !std::path::Path::new(&game.exe).exists() {
            bail!("Executable does not exist: {}", game.exe);
        }
        if game.runner == "Steam" && game.app_id.is_empty() {
            bail!("Steam App ID not set");
        }

        let mut env: HashMap<String, String> = std::env::vars().collect();

        // Quote-aware parsing: previously `split_whitespace()` broke any
        // path or argument containing spaces (e.g. `--config="C:\Program
        // Files\..."`). `shell_words` understands single/double quotes and
        // backslash escaping the way a shell would.
        let launch_options: Vec<String> = shell_words::split(&game.launch_options)
            .with_context(|| {
                format!(
                    "Could not parse launch options: {}",
                    game.launch_options
                )
            })?;

        let is_wine_or_proton = game.runner == "Wine" || game.runner.contains("Proton");
        // Held for the entire lifetime of the launched process (moved into
        // the watcher thread below) so a second game — or a Winetricks /
        // dependency-installer run — can't touch the same prefix while this
        // one is running and risk corrupting its Wine registry.
        let mut prefix_guard: Option<crate::prefix_lock::PrefixLockGuard> = None;

        if is_wine_or_proton {
            // Set up prefix: either the shared one (opt-in per game, path
            // configurable in Settings) or the game's own auto-created one.
            if game.use_shared_prefix {
                let shared = if self.settings.shared_prefix_path.trim().is_empty() {
                    self.prefixes_dir.join("shared")
                } else {
                    PathBuf::from(&self.settings.shared_prefix_path)
                };
                game.prefix = shared.to_string_lossy().to_string();
            } else if game.prefix.is_empty() {
                let default_prefix = self.prefixes_dir.join(game.name.replace(' ', "_"));
                game.prefix = default_prefix.to_string_lossy().to_string();
            }
            fs::create_dir_all(&game.prefix)?;

            prefix_guard = Some(crate::prefix_lock::lock_prefix(
                &game.prefix,
                &format!("game \"{}\"", game.name),
            )?);

            env.insert("WINEPREFIX".to_string(), game.prefix.clone());
            if game.enable_dxvk {
                env.insert(
                    "WINEDLLOVERRIDES".to_string(),
                    "d3d11=n,b;dxgi=n,b".to_string(),
                );
            }
            let esync = if game.enable_esync || self.settings.enable_esync {
                "1"
            } else {
                "0"
            };
            let fsync = if game.enable_fsync || self.settings.enable_fsync {
                "1"
            } else {
                "0"
            };
            let dxvk_async = if game.enable_dxvk_async || self.settings.enable_dxvk_async {
                "1"
            } else {
                "0"
            };
            env.insert("WINEESYNC".to_string(), esync.to_string());
            env.insert("WINEFSYNC".to_string(), fsync.to_string());
            env.insert("DXVK_ASYNC".to_string(), dxvk_async.to_string());

            if game.disable_steam_input {
                env.insert("STEAM_COMPAT_DISABLE_STEAM_INPUT".to_string(), "1".to_string());
            }
            if !game.sdl_controller_config.trim().is_empty() {
                env.insert(
                    "SDL_GAMECONTROLLERCONFIG".to_string(),
                    game.sdl_controller_config.trim().to_string(),
                );
            }
        }

        // Per-game custom environment variables. Applied after the fixed
        // ones above so the user can deliberately override them if needed.
        for (k, v) in parse_env_vars(&game.env_vars) {
            env.insert(k, v);
        }

        // Build command
        let mut cmd_parts: Vec<String> = vec![];
        let mut remaining_options = launch_options.clone();

        if gamescope {
            if which::which("gamescope").is_err() {
                bail!("Gamescope is not installed. Please install it via your package manager.");
            }
            cmd_parts.push("gamescope".to_string());
            let mut to_remove: Vec<String> = vec![];

            if remaining_options.contains(&"--adaptive-sync".to_string()) {
                cmd_parts.push("--adaptive-sync".to_string());
                to_remove.push("--adaptive-sync".to_string());
            }
            if remaining_options.contains(&"--force-grab-cursor".to_string()) {
                cmd_parts.push("--force-grab-cursor".to_string());
                to_remove.push("--force-grab-cursor".to_string());
            }
            if let Some(w) = remaining_options.iter().find(|o| o.starts_with("--width=")) {
                let val = w.split('=').nth(1).unwrap_or("1920");
                cmd_parts.push("-W".to_string());
                cmd_parts.push(val.to_string());
                to_remove.push(w.clone());
            }
            if let Some(h) = remaining_options.iter().find(|o| o.starts_with("--height=")) {
                let val = h.split('=').nth(1).unwrap_or("1080");
                cmd_parts.push("-H".to_string());
                cmd_parts.push(val.to_string());
                to_remove.push(h.clone());
            }
            if remaining_options.contains(&"--fullscreen".to_string()) {
                cmd_parts.push("-f".to_string());
                to_remove.push("--fullscreen".to_string());
            }
            if remaining_options.contains(&"--bigpicture".to_string()) {
                cmd_parts.extend(["-e".to_string(), "-f".to_string()]);
                to_remove.push("--bigpicture".to_string());
            }
            if let Some(fps) = game.fps_limit {
                cmd_parts.push("-r".to_string());
                cmd_parts.push(fps.to_string());
            }
            remaining_options.retain(|o| !to_remove.contains(o));
            cmd_parts.push("--".to_string());
        }

        // Runner-specific command
        match game.runner.as_str() {
            "Native" => {
                cmd_parts.push(game.exe.clone());
                cmd_parts.extend(remaining_options);
            }
            "Wine" => {
                if which::which("wine").is_err() {
                    bail!("Wine not installed. Please install it (e.g., dnf install wine).");
                }
                cmd_parts.push("wine".to_string());
                cmd_parts.push(game.exe.clone());
                cmd_parts.extend(remaining_options);
            }
            "Flatpak" => {
                if which::which("flatpak").is_err() {
                    bail!("Flatpak not installed.");
                }
                cmd_parts.extend(["flatpak".to_string(), "run".to_string()]);
                cmd_parts.push(game.exe.clone());
                cmd_parts.extend(remaining_options);
            }
            "Steam" => {
                if which::which("flatpak").is_ok() && which::which("steam").is_err() {
                    cmd_parts.extend([
                        "flatpak".to_string(),
                        "run".to_string(),
                        "com.valvesoftware.Steam".to_string(),
                        "-applaunch".to_string(),
                        game.app_id.clone(),
                    ]);
                } else if which::which("steam").is_ok() {
                    cmd_parts.extend([
                        "steam".to_string(),
                        "-applaunch".to_string(),
                        game.app_id.clone(),
                    ]);
                } else {
                    bail!("Steam or Flatpak not installed.");
                }
                cmd_parts.extend(remaining_options);
            }
            runner if runner.contains("Proton") => {
                // Find proton binary
                let proton_dir = self.protons_dir.join(runner);
                let proton_bin = find_proton_binary(&proton_dir)
                    .with_context(|| format!("Proton binary not found for {}", runner))?;

                let steam_dir = dirs::home_dir()
                    .unwrap_or_default()
                    .join(".local/share/Steam");
                fs::create_dir_all(steam_dir.join("steamapps/compatdata")).ok();

                env.insert(
                    "STEAM_COMPAT_CLIENT_INSTALL_PATH".to_string(),
                    steam_dir.to_string_lossy().to_string(),
                );
                env.insert("STEAM_COMPAT_DATA_PATH".to_string(), game.prefix.clone());
                env.insert(
                    "STEAM_RUNTIME".to_string(),
                    steam_dir
                        .join("ubuntu12_32/steam-runtime")
                        .to_string_lossy()
                        .to_string(),
                );
                let ld = format!(
                    "{}:{}:{}",
                    steam_dir.join("ubuntu12_32").to_string_lossy(),
                    steam_dir.join("ubuntu12_64").to_string_lossy(),
                    env.get("LD_LIBRARY_PATH").cloned().unwrap_or_default()
                );
                env.insert("LD_LIBRARY_PATH".to_string(), ld);

                cmd_parts.push(proton_bin.to_string_lossy().to_string());
                cmd_parts.push("waitforexitandrun".to_string());
                cmd_parts.push(game.exe.clone());
                cmd_parts.extend(remaining_options);
            }
            unknown => bail!("Unknown runner: {}", unknown),
        }

        // Rotate logs before writing a new one so we never keep more than
        // MAX_LOGS_PER_GAME historical runs on disk.
        fs::create_dir_all(&self.logs_dir).ok();
        rotate_logs(&self.logs_dir, &game.name, MAX_LOGS_PER_GAME)?;

        let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
        let log_file_path = self
            .logs_dir
            .join(format!("{}_{}.log", sanitize_name(&game.name), timestamp));
        let log_file = fs::File::create(&log_file_path)?;
        let stderr_log = log_file.try_clone()?;

        let mut command = std::process::Command::new(&cmd_parts[0]);
        command
            .args(&cmd_parts[1..])
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(stderr_log));

        // Run the game from its own directory: many titles look for assets
        // relative to the executable's folder rather than the launcher's
        // current working directory.
        if game.runner != "Steam" {
            if let Some(parent) = std::path::Path::new(&game.exe).parent() {
                if !parent.as_os_str().is_empty() {
                    command.current_dir(parent);
                }
            }
        }

        for (k, v) in &env {
            command.env(k, v);
        }

        let mut child = command.spawn().with_context(|| {
            format!("Failed to launch game: {} with cmd: {:?}", game.name, cmd_parts)
        })?;

        // Persist any auto-computed fields (prefix) picked up above, plus
        // last_played, without touching duplicate/rename validation.
        game.last_played = Some(Local::now().to_rfc3339());
        let _ = self.save_single_game(&game);

        let pid = child.id();
        let started_at = Local::now();
        {
            let mut running = self.running.lock().unwrap();
            running.insert(
                game.name.clone(),
                RunningEntry { pid, started_at },
            );
        }
        let _ = app_handle.emit(
            "game_started",
            serde_json::json!({ "name": game.name, "pid": pid, "started_at": started_at.to_rfc3339() }),
        );

        // Watcher thread: waits for the process to exit (whether it quit on
        // its own or was killed via stop_game), then records playtime and
        // clears the running-state entry. This is what makes "is it still
        // running" and "how long have I played this" possible at all.
        let running_map = self.running.clone();
        let games_file = self.games_file.clone();
        let game_name = game.name.clone();
        let app_handle_watch = app_handle.clone();

        std::thread::spawn(move || {
            let _ = child.wait();
            let elapsed = (Local::now() - started_at).num_seconds().max(0) as u64;

            if let Ok(content) = fs::read_to_string(&games_file) {
                if let Ok(mut games) = serde_json::from_str::<Vec<Game>>(&content) {
                    for g in games.iter_mut() {
                        if g.name == game_name {
                            g.total_playtime_secs = g.total_playtime_secs.saturating_add(elapsed);
                        }
                    }
                    if let Ok(new_content) = serde_json::to_string_pretty(&games) {
                        let _ = fs::write(&games_file, new_content);
                    }
                }
            }

            running_map.lock().unwrap().remove(&game_name);
            let _ = app_handle_watch.emit(
                "game_stopped",
                serde_json::json!({ "name": game_name, "elapsed_secs": elapsed }),
            );
            // Dropping the guard here (end of thread, after the process has
            // actually exited) is what releases the prefix lock — not the
            // return of `launch_game` above, which happens immediately
            // after spawning.
            drop(prefix_guard);
        });

        Ok(())
    }
}

/// Turns a `KEY=VALUE` per-line (or `;`-separated) block into pairs, skipping
/// blank lines and `#`-prefixed comments.
fn parse_env_vars(raw: &str) -> Vec<(String, String)> {
    raw.split(['\n', ';'])
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut parts = l.splitn(2, '=');
            let key = parts.next()?.trim().to_string();
            let value = parts.next().unwrap_or("").trim().to_string();
            if key.is_empty() {
                None
            } else {
                Some((key, value))
            }
        })
        .collect()
}

/// Filesystem-safe stand-in for a game's display name, used to build log
/// file names.
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Deletes the oldest log files for `game_name` beyond `keep` most recent
/// ones. Previously every launch overwrote a single fixed log file, so
/// crashes from a previous run were unrecoverable by the time the user
/// noticed the game had stopped.
fn rotate_logs(logs_dir: &PathBuf, game_name: &str, keep: usize) -> Result<()> {
    if !logs_dir.exists() {
        return Ok(());
    }
    let prefix = format!("{}_", sanitize_name(game_name));
    let mut files: Vec<PathBuf> = fs::read_dir(logs_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(&prefix) && n.ends_with(".log"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    if files.len() >= keep {
        let remove_count = files.len() - keep + 1;
        for path in files.into_iter().take(remove_count) {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

fn find_proton_binary(base: &PathBuf) -> Option<PathBuf> {
    for entry in walkdir::WalkDir::new(base).max_depth(5) {
        if let Ok(e) = entry {
            if e.file_name() == "proton" && e.file_type().is_file() {
                return Some(e.path().to_path_buf());
            }
        }
    }
    None
}
