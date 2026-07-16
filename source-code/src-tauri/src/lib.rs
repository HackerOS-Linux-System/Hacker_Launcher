mod config_manager;
mod game_manager;
mod prefix_lock;
mod proton_manager;
mod tools;

use config_manager::ConfigManager;
use game_manager::GameManager;
use proton_manager::ProtonManager;
use std::sync::Mutex;
use tauri::State;

pub struct AppState {
    pub config: Mutex<ConfigManager>,
    pub proton: Mutex<ProtonManager>,
    pub game: Mutex<GameManager>,
}

// ─────────────────────────────────────────────
//  Config / Settings commands
// ─────────────────────────────────────────────

#[tauri::command]
fn get_settings(state: State<AppState>) -> Result<config_manager::Settings, String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    Ok(cfg.settings.clone())
}

#[tauri::command]
fn save_settings(
    state: State<AppState>,
    settings: config_manager::Settings,
) -> Result<(), String> {
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    cfg.save_settings(&settings).map_err(|e| e.to_string())?;
    cfg.settings = settings.clone();
    drop(cfg);
    // Keep GameManager's cached copy (used for global Esync/Fsync/DXVK
    // Async defaults and the shared-prefix path) in sync immediately,
    // instead of only picking up changes on the next app restart.
    let mut gm = state.game.lock().map_err(|e| e.to_string())?;
    gm.update_settings(settings);
    Ok(())
}

#[tauri::command]
fn get_paths(state: State<AppState>) -> Result<config_manager::Paths, String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    Ok(config_manager::Paths {
        prefixes_dir: cfg.prefixes_dir.to_string_lossy().to_string(),
        protons_dir: cfg.protons_dir.to_string_lossy().to_string(),
        logs_dir: cfg.logs_dir.to_string_lossy().to_string(),
    })
}

// ─────────────────────────────────────────────
//  Game commands
// ─────────────────────────────────────────────

#[tauri::command]
fn get_games(state: State<AppState>) -> Result<Vec<game_manager::Game>, String> {
    let gm = state.game.lock().map_err(|e| e.to_string())?;
    gm.load_games().map_err(|e| e.to_string())
}

#[tauri::command]
fn add_game(state: State<AppState>, game: game_manager::Game) -> Result<(), String> {
    let gm = state.game.lock().map_err(|e| e.to_string())?;
    gm.add_game(game).map_err(|e| e.to_string())
}

#[tauri::command]
fn remove_game(state: State<AppState>, name: String) -> Result<(), String> {
    let gm = state.game.lock().map_err(|e| e.to_string())?;
    gm.remove_game(&name).map_err(|e| e.to_string())
}

/// `original_name` is required because a rename means `game.name` no longer
/// matches the on-disk record we need to find and replace.
#[tauri::command]
fn update_game(
    state: State<AppState>,
    game: game_manager::Game,
    original_name: String,
) -> Result<(), String> {
    let gm = state.game.lock().map_err(|e| e.to_string())?;
    gm.update_game(game, &original_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn launch_game(
    state: State<AppState>,
    game: game_manager::Game,
    gamescope: bool,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let gm = state.game.lock().map_err(|e| e.to_string())?;
    gm.launch_game(game, gamescope, app_handle)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn stop_game(state: State<AppState>, name: String) -> Result<(), String> {
    let gm = state.game.lock().map_err(|e| e.to_string())?;
    gm.stop_game(&name).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_running_games(
    state: State<AppState>,
) -> Result<Vec<game_manager::RunningGameInfo>, String> {
    let gm = state.game.lock().map_err(|e| e.to_string())?;
    Ok(gm.get_running_games())
}

#[tauri::command]
fn list_game_logs(
    state: State<AppState>,
    name: String,
) -> Result<Vec<game_manager::GameLogEntry>, String> {
    let gm = state.game.lock().map_err(|e| e.to_string())?;
    gm.list_game_logs(&name).map_err(|e| e.to_string())
}

#[tauri::command]
fn read_game_log(state: State<AppState>, path: String) -> Result<String, String> {
    let gm = state.game.lock().map_err(|e| e.to_string())?;
    gm.read_log_file(&path).map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────
//  Proton commands
// ─────────────────────────────────────────────

#[tauri::command]
fn get_installed_protons(
    state: State<AppState>,
) -> Result<Vec<proton_manager::ProtonEntry>, String> {
    let pm = state.proton.lock().map_err(|e| e.to_string())?;
    pm.get_installed_protons().map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_available_ge() -> Result<Vec<String>, String> {
    proton_manager::ProtonManager::fetch_available_ge()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_available_official(stable: bool) -> Result<Vec<String>, String> {
    proton_manager::ProtonManager::fetch_available_official(stable)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn install_proton(
    state: State<'_, AppState>,
    version: String,
    proton_type: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let protons_dir = {
        let pm = state.proton.lock().map_err(|e| e.to_string())?;
        pm.protons_dir.clone()
    };
    let result = proton_manager::ProtonManager::install_proton_async(
        protons_dir,
        version.clone(),
        proton_type,
        app_handle.clone(),
    )
    .await;
    match &result {
        Ok(()) => notify(&app_handle, "Proton installed", &format!("{} is ready to use.", version)),
        Err(e) => notify(&app_handle, "Proton installation failed", &format!("{}: {}", version, e)),
    }
    result.map_err(|e| e.to_string())
}

/// Signals a currently in-progress `install_proton` call for `version` to
/// stop as soon as possible (checked between download chunks and between
/// extracted tar entries), cleaning up any partial files.
#[tauri::command]
fn cancel_proton_install(version: String) -> Result<bool, String> {
    Ok(proton_manager::cancel_install(&version))
}

#[tauri::command]
async fn install_custom_tar(
    state: State<'_, AppState>,
    tar_path: String,
    version: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let protons_dir = {
        let pm = state.proton.lock().map_err(|e| e.to_string())?;
        pm.protons_dir.clone()
    };
    let result = proton_manager::ProtonManager::install_custom_tar_async(
        protons_dir,
        tar_path,
        version.clone(),
        app_handle.clone(),
    )
    .await;
    match &result {
        Ok(()) => notify(&app_handle, "Proton installed", &format!("{} is ready to use.", version)),
        Err(e) => notify(&app_handle, "Proton installation failed", &format!("{}: {}", version, e)),
    }
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn install_custom_folder(
    state: State<AppState>,
    src_folder: String,
    version: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let pm = state.proton.lock().map_err(|e| e.to_string())?;
    let result = pm.install_custom_folder(&src_folder, &version);
    match &result {
        Ok(()) => notify(&app_handle, "Proton installed", &format!("{} is ready to use.", version)),
        Err(e) => notify(&app_handle, "Proton installation failed", &format!("{}: {}", version, e)),
    }
    result.map_err(|e| e.to_string())
}

/// Removes an installed Proton version. If any saved game currently uses it
/// as its runner, the removal is refused unless `force` is true — previously
/// this check didn't exist at all, so removing a Proton version out from
/// under a configured game left a dangling reference that only surfaced the
/// next time the user tried to launch it.
#[tauri::command]
fn remove_proton(state: State<AppState>, version: String, force: bool) -> Result<(), String> {
    {
        let gm = state.game.lock().map_err(|e| e.to_string())?;
        let games = gm.load_games().map_err(|e| e.to_string())?;
        let affected: Vec<String> = games
            .iter()
            .filter(|g| g.runner == version)
            .map(|g| g.name.clone())
            .collect();
        if !affected.is_empty() && !force {
            return Err(format!(
                "IN_USE:{}:Used by: {}",
                version,
                affected.join(", ")
            ));
        }
    }
    let pm = state.proton.lock().map_err(|e| e.to_string())?;
    pm.remove_proton(&version).map_err(|e| e.to_string())
}

#[tauri::command]
async fn check_proton_update(
    _state: State<'_, AppState>,
    version: String,
    proton_type: String,
) -> Result<Option<(String, String)>, String> {
    proton_manager::ProtonManager::check_update_async(version, proton_type)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_release_notes(version: String, proton_type: String) -> Result<String, String> {
    proton_manager::ProtonManager::fetch_release_notes(version, proton_type)
        .await
        .map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────
//  Winetricks
// ─────────────────────────────────────────────

#[tauri::command]
fn get_common_winetricks_verbs() -> Vec<(String, String)> {
    tools::common_winetricks_verbs()
        .into_iter()
        .map(|(v, l)| (v.to_string(), l.to_string()))
        .collect()
}

/// The full ~700-entry winetricks catalog, fetched lazily (only when the
/// user actually opens the search) rather than on every Winetricks dialog
/// open, since it shells out to `winetricks list-all`.
#[tauri::command]
async fn get_all_winetricks_verbs() -> Result<Vec<(String, String)>, String> {
    tokio::task::spawn_blocking(tools::all_winetricks_verbs)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn run_winetricks(
    prefix: String,
    verbs: Vec<String>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    let app_for_run = app_handle.clone();
    let result =
        tokio::task::spawn_blocking(move || tools::run_winetricks(&prefix, &verbs, &app_for_run))
            .await
            .map_err(|e| e.to_string())?;
    match &result {
        Ok(_) => notify(&app_handle, "Winetricks finished", "Selected components were installed."),
        Err(e) => notify(&app_handle, "Winetricks failed", &e.to_string()),
    }
    result.map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────
//  Dependency scanning
// ─────────────────────────────────────────────

#[tauri::command]
fn scan_game_dependencies(exe_path: String) -> Result<Vec<tools::DependencyHint>, String> {
    tools::scan_game_dependencies(&exe_path).map_err(|e| e.to_string())
}

#[tauri::command]
async fn run_dependency_installer(
    prefix: String,
    installer_path: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let app_for_run = app_handle.clone();
    let result = tokio::task::spawn_blocking(move || {
        tools::run_installer_in_prefix(&prefix, &installer_path, &app_for_run)
    })
    .await
    .map_err(|e| e.to_string())?;
    match &result {
        Ok(()) => notify(&app_handle, "Installer finished", "Dependency installer has closed."),
        Err(e) => notify(&app_handle, "Installer failed", &e.to_string()),
    }
    result.map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────
//  Controllers
// ─────────────────────────────────────────────

#[tauri::command]
fn list_controllers() -> Result<Vec<tools::ControllerInfo>, String> {
    tools::list_controllers().map_err(|e| e.to_string())
}

/// Waits for one button press / significant axis movement on the given
/// controller and reports it, for the `SDL_GAMECONTROLLERCONFIG` mapping
/// wizard's "press a button now" step.
#[tauri::command]
async fn capture_controller_input(
    handler: String,
    timeout_ms: u64,
) -> Result<tools::ControllerInputEvent, String> {
    tokio::task::spawn_blocking(move || tools::capture_controller_input(&handler, timeout_ms))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_controller_guid(handler: String) -> Result<String, String> {
    tools::get_controller_guid(&handler).map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────
//  ProtonDB
// ─────────────────────────────────────────────

#[tauri::command]
async fn check_protondb(app_id: String) -> Result<Option<tools::ProtonDbInfo>, String> {
    tools::check_protondb(&app_id).await.map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────
//  Steam library import
// ─────────────────────────────────────────────

#[tauri::command]
fn scan_steam_library() -> Result<Vec<tools::SteamGameCandidate>, String> {
    tools::scan_steam_library().map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────
//  Backup / restore
// ─────────────────────────────────────────────

#[tauri::command]
fn export_backup(state: State<AppState>, dest_path: String) -> Result<(), String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    tools::export_backup(&cfg.games_file, &cfg.settings_file, std::path::Path::new(&dest_path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn import_backup(state: State<AppState>, src_path: String, merge: bool) -> Result<(), String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    tools::import_backup(
        &cfg.games_file,
        &cfg.settings_file,
        std::path::Path::new(&src_path),
        merge,
    )
    .map_err(|e| e.to_string())?;
    drop(cfg);
    // Reload the in-memory settings copy so a full (non-merge) restore
    // takes effect immediately rather than requiring a restart.
    if !merge {
        let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
        if cfg.settings_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&cfg.settings_file) {
                if let Ok(s) = serde_json::from_str::<config_manager::Settings>(&content) {
                    cfg.settings = s.clone();
                    let mut gm = state.game.lock().map_err(|e| e.to_string())?;
                    gm.update_settings(s);
                }
            }
        }
    }
    Ok(())
}

fn notify(app: &tauri::AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app.notification().builder().title(title).body(body).show();
}

// ─────────────────────────────────────────────
//  Entry point
// ─────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config = ConfigManager::new().expect("Failed to initialize ConfigManager");
    let proton = ProtonManager::new(config.protons_dir.clone());
    let game = GameManager::new(
        config.games_file.clone(),
        config.prefixes_dir.clone(),
        config.logs_dir.clone(),
        config.protons_dir.clone(),
        config.settings.clone(),
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState {
            config: Mutex::new(config),
            proton: Mutex::new(proton),
            game: Mutex::new(game),
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            get_paths,
            get_games,
            add_game,
            remove_game,
            update_game,
            launch_game,
            stop_game,
            get_running_games,
            list_game_logs,
            read_game_log,
            get_installed_protons,
            get_available_ge,
            get_available_official,
            install_proton,
            cancel_proton_install,
            install_custom_tar,
            install_custom_folder,
            remove_proton,
            check_proton_update,
            get_release_notes,
            get_common_winetricks_verbs,
            get_all_winetricks_verbs,
            run_winetricks,
            scan_game_dependencies,
            run_dependency_installer,
            list_controllers,
            capture_controller_input,
            get_controller_guid,
            check_protondb,
            scan_steam_library,
            export_backup,
            import_backup,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
