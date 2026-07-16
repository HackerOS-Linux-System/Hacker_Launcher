use crate::config_manager::Settings;
use crate::game_manager::Game;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::Emitter;

// ─────────────────────────────────────────────
//  Steam library import
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SteamGameCandidate {
    pub name: String,
    pub app_id: String,
    pub exe_path: String,
    pub install_dir: String,
}

/// Very small, tolerant parser for Valve's VDF/ACF key-value format. It only
/// understands flat `"key"    "value"` lines (which is all `appmanifest_*`
/// files and `libraryfolders.vdf` actually need here) — nested blocks are
/// simply ignored rather than fully parsed. `entry().or_insert()` is used so
/// a key seen at the top level (which appears first in these files) always
/// wins over a same-named key nested deeper in the file.
fn parse_vdf_flat(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if !line.starts_with('"') {
            continue;
        }
        let parts: Vec<&str> = line.split('"').collect();
        if parts.len() >= 4 {
            let key = parts[1].to_string();
            let value = parts[3].to_string();
            map.entry(key).or_insert(value);
        }
    }
    map
}

/// Every `steamapps` directory that might contain installed games. Starts
/// from the well-known default locations (including their Flatpak
/// equivalent), resolves symlinks so a `~/.steam/steam` -> elsewhere setup
/// still works, honors a `STEAM_ROOT` env var override, then follows each
/// `libraryfolders.vdf` to any additional Library Folders the user has
/// added (which is how Steam itself represents "install games on this
/// other drive too", including external/removable drives). As a last
/// resort it also peeks at common mount points for a `SteamLibrary` folder,
/// since some users create one manually without it being registered in
/// `libraryfolders.vdf` (e.g. after moving a library folder around).
fn candidate_steamapps_dirs() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut roots = vec![
        home.join(".local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".steam/debian-installation"),
    ];
    if let Ok(custom) = std::env::var("STEAM_ROOT") {
        roots.push(PathBuf::from(custom));
    }

    // Resolve symlinks (`~/.steam/steam` is very commonly a symlink into
    // the real install, sometimes on a different filesystem entirely).
    let resolved_roots: Vec<PathBuf> = roots
        .iter()
        .map(|p| fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
        .collect();

    let mut steamapps_dirs: Vec<PathBuf> =
        resolved_roots.iter().map(|r| r.join("steamapps")).collect();

    // Follow libraryfolders.vdf for additional library locations (this is
    // how Steam tracks libraries on other drives).
    for steamapps in steamapps_dirs.clone() {
        let lib_file = steamapps.join("libraryfolders.vdf");
        if let Ok(content) = fs::read_to_string(&lib_file) {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with("\"path\"") {
                    let parts: Vec<&str> = line.split('"').collect();
                    if parts.len() >= 4 {
                        let raw = PathBuf::from(parts[3]);
                        let resolved = fs::canonicalize(&raw).unwrap_or(raw);
                        steamapps_dirs.push(resolved.join("steamapps"));
                    }
                }
            }
        }
    }

    // Best-effort scan of common external/removable-drive mount points for
    // an unregistered `SteamLibrary/steamapps` folder.
    for mount_base in ["/mnt", "/media", "/run/media"] {
        let base = PathBuf::from(mount_base);
        let Ok(entries) = fs::read_dir(&base) else { continue };
        for entry in entries.filter_map(|e| e.ok()) {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            // /run/media is namespaced one level deeper by username.
            let mut probe_dirs = vec![p.clone()];
            if mount_base == "/run/media" {
                if let Ok(sub_entries) = fs::read_dir(&p) {
                    probe_dirs.extend(sub_entries.filter_map(|e| e.ok()).map(|e| e.path()));
                }
            }
            for probe in probe_dirs {
                let steam_lib = probe.join("SteamLibrary").join("steamapps");
                if steam_lib.exists() {
                    steamapps_dirs.push(steam_lib);
                }
                let direct = probe.join("steamapps");
                if direct.exists() {
                    steamapps_dirs.push(direct);
                }
            }
        }
    }

    steamapps_dirs.sort();
    steamapps_dirs.dedup();
    steamapps_dirs.into_iter().filter(|d| d.exists()).collect()
}

const EXE_DENYLIST: &[&str] = &[
    "unins", "redist", "vcredist", "vc_redist", "dxsetup", "dotnetfx", "crashpad",
    "crashreport", "crashhandler", "easyanticheat", "battleye", "helper",
    "vulkan", "directx", "setup.exe", "installer",
];

fn find_main_exe(install_path: &Path) -> Option<PathBuf> {
    let mut best: Option<(PathBuf, u64)> = None;
    for entry in walkdir::WalkDir::new(install_path)
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if !name.ends_with(".exe") {
            continue;
        }
        if EXE_DENYLIST.iter().any(|d| name.contains(d)) {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if best.as_ref().map(|(_, s)| size > *s).unwrap_or(true) {
            best = Some((entry.path().to_path_buf(), size));
        }
    }
    best.map(|(p, _)| p)
}

/// Scans every known Steam library for installed games by reading each
/// `appmanifest_*.acf` and guessing the main executable inside
/// `steamapps/common/<installdir>`. This is a heuristic — Steam doesn't
/// record "the exe" anywhere accessible, so we pick the largest non-utility
/// `.exe` in the install folder, which is right the overwhelming majority
/// of the time but can occasionally pick a launcher/sub-tool instead of the
/// real game binary.
pub fn scan_steam_library() -> Result<Vec<SteamGameCandidate>> {
    let mut results = vec![];
    for steamapps in candidate_steamapps_dirs() {
        let common = steamapps.join("common");
        let entries = match fs::read_dir(&steamapps) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if !file_name.starts_with("appmanifest_") || !file_name.ends_with(".acf") {
                continue;
            }
            let content = match fs::read_to_string(entry.path()) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let map = parse_vdf_flat(&content);
            let app_id = map.get("appid").cloned().unwrap_or_default();
            let name = map.get("name").cloned().unwrap_or_default();
            let install_dir = map.get("installdir").cloned().unwrap_or_default();
            if app_id.is_empty() || install_dir.is_empty() {
                continue;
            }
            let install_path = common.join(&install_dir);
            if !install_path.exists() {
                continue;
            }
            let exe_path = find_main_exe(&install_path)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            results.push(SteamGameCandidate {
                name: if name.is_empty() { install_dir.clone() } else { name },
                app_id,
                exe_path,
                install_dir,
            });
        }
    }
    results.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    results.dedup_by(|a, b| a.app_id == b.app_id);
    Ok(results)
}

// ─────────────────────────────────────────────
//  Winetricks
// ─────────────────────────────────────────────

/// A short, curated menu of commonly-needed components shown as one-click
/// checkboxes before the user even has to search — winetricks itself
/// supports hundreds of verbs (see `all_winetricks_verbs`), this is just
/// what covers the vast majority of "game won't start, missing X" reports.
pub fn common_winetricks_verbs() -> Vec<(&'static str, &'static str)> {
    vec![
        ("vcrun2022", "Visual C++ 2015-2022 Redistributable"),
        ("vcrun2019", "Visual C++ 2019 Redistributable"),
        ("dotnet48", ".NET Framework 4.8"),
        ("dotnet6", ".NET 6 Runtime"),
        ("corefonts", "Core Windows Fonts"),
        ("d3dx9", "DirectX 9 (D3DX9)"),
        ("d3dx11_43", "DirectX 11 (D3DX11)"),
        ("dxvk", "DXVK (D3D9/10/11 → Vulkan)"),
        ("physx", "PhysX Runtime"),
        ("xact", "XACT (XAudio)"),
        ("openal", "OpenAL Runtime"),
        ("vkd3d", "VKD3D (D3D12 → Vulkan)"),
    ]
}

/// The full winetricks verb catalog (~700 entries), parsed from
/// `winetricks list-all`'s own output rather than hand-maintained here.
/// Format per line is roughly `verb short_name (description) [downloadable]`
/// with bare category header lines (ending in `:`) interspersed — we keep
/// the first whitespace-separated token as the verb id and whatever's
/// inside the first parenthesis as the human label, and just skip anything
/// that doesn't look like a verb line. If winetricks isn't installed, or
/// its output format doesn't parse into anything, callers get the curated
/// shortlist above instead so the UI never ends up empty.
pub fn all_winetricks_verbs() -> Result<Vec<(String, String)>> {
    if which::which("winetricks").is_err() {
        bail!("winetricks is not installed.");
    }
    let output = std::process::Command::new("winetricks")
        .arg("list-all")
        .output()
        .context("Failed to run `winetricks list-all`")?;
    let text = String::from_utf8_lossy(&output.stdout);

    let mut verbs = vec![];
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.ends_with(':') || line.starts_with('=') {
            continue; // category headers / banners, not verb entries
        }
        let Some(verb) = line.split_whitespace().next() else { continue };
        if verb.chars().any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
            continue; // not a plausible verb identifier
        }
        let label = if let (Some(start), Some(end)) = (line.find('('), line.rfind(')')) {
            if end > start {
                line[start + 1..end].to_string()
            } else {
                verb.to_string()
            }
        } else {
            verb.to_string()
        };
        verbs.push((verb.to_string(), label));
    }
    verbs.sort();
    verbs.dedup_by(|a, b| a.0 == b.0);

    if verbs.is_empty() {
        return Ok(common_winetricks_verbs()
            .into_iter()
            .map(|(v, l)| (v.to_string(), l.to_string()))
            .collect());
    }
    Ok(verbs)
}

/// Reads a piped child process stream line-by-line, appending each line to
/// a shared log buffer and emitting it live as a `process_output` event so
/// the UI can show real-time progress instead of a silent "is this frozen?"
/// wait for operations like `dotnet48` that can take several minutes.
fn stream_lines<R: std::io::Read>(reader: R, app: &tauri::AppHandle, log: &Arc<Mutex<String>>, source: &str) {
    use std::io::{BufRead, BufReader};
    let buffered = BufReader::new(reader);
    for line in buffered.lines().map_while(|l| l.ok()) {
        {
            let mut guard = log.lock().unwrap();
            guard.push_str(&line);
            guard.push('\n');
        }
        let _ = app.emit(
            "process_output",
            serde_json::json!({ "source": source, "line": line }),
        );
    }
}

/// Runs `winetricks -q <verbs...>` against a given prefix, streaming output
/// live (see `stream_lines`) and returning the full captured log once it
/// finishes. Blocking — the caller (a Tauri command) should run this inside
/// `spawn_blocking`, since a verb like `dotnet48` can legitimately take a
/// few minutes. Refuses to start if the prefix is already locked by another
/// game or maintenance operation.
pub fn run_winetricks(prefix: &str, verbs: &[String], app_handle: &tauri::AppHandle) -> Result<String> {
    if which::which("winetricks").is_err() {
        bail!(
            "winetricks is not installed. Install it via your package manager \
             (e.g. `sudo apt install winetricks` / `sudo dnf install winetricks`)."
        );
    }
    if verbs.is_empty() {
        bail!("No components selected");
    }
    fs::create_dir_all(prefix).ok();

    let _lock = crate::prefix_lock::lock_prefix(prefix, "Winetricks")?;

    let mut child = std::process::Command::new("winetricks")
        .env("WINEPREFIX", prefix)
        .arg("-q")
        .args(verbs)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to start winetricks")?;

    let log = Arc::new(Mutex::new(String::new()));
    let mut readers = vec![];
    if let Some(stdout) = child.stdout.take() {
        let app = app_handle.clone();
        let log = log.clone();
        readers.push(std::thread::spawn(move || stream_lines(stdout, &app, &log, "winetricks")));
    }
    if let Some(stderr) = child.stderr.take() {
        let app = app_handle.clone();
        let log = log.clone();
        readers.push(std::thread::spawn(move || stream_lines(stderr, &app, &log, "winetricks")));
    }
    for r in readers {
        let _ = r.join();
    }

    let status = child.wait().context("winetricks process error")?;
    let log_text = log.lock().unwrap().clone();

    if !status.success() {
        bail!("winetricks exited with an error:\n{}", log_text.trim());
    }
    Ok(log_text)
}

// ─────────────────────────────────────────────
//  Dependency scanning (VC++ Redist / .NET / DirectX installers)
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct DependencyHint {
    pub label: String,
    pub path: String,
    pub winetricks_verb: Option<String>,
}

/// Looks for common redistributable installers bundled next to a game's
/// executable (a real-world convention for many older/indie titles) and
/// suggests either running the bundled installer directly or the
/// equivalent winetricks verb. This is a filename-pattern heuristic, not
/// static analysis of the game binary's actual imports — it will miss
/// dependencies that aren't shipped as a visible installer file, and can't
/// tell you a dependency is *already* satisfied in the prefix.
pub fn scan_game_dependencies(exe_path: &str) -> Result<Vec<DependencyHint>> {
    let dir = Path::new(exe_path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let mut hints: Vec<DependencyHint> = vec![];
    if !dir.exists() {
        return Ok(hints);
    }
    for entry in walkdir::WalkDir::new(&dir)
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let hit: Option<(&str, Option<&str>)> = if name.contains("vcredist") || name.contains("vc_redist") {
            Some(("Visual C++ Redistributable", Some("vcrun2022")))
        } else if name.contains("dotnetfx") || name.contains("ndp48") || name.contains("dotnet") {
            Some((".NET Framework", Some("dotnet48")))
        } else if name.contains("dxsetup") {
            Some(("DirectX Runtime", Some("d3dx9")))
        } else if name.contains("oalinst") {
            Some(("OpenAL Runtime", Some("openal")))
        } else if name.contains("xnafx") {
            Some((".NET XNA Framework", None))
        } else if name.contains("physx") {
            Some(("PhysX Runtime", Some("physx")))
        } else {
            None
        };
        if let Some((label, verb)) = hit {
            hints.push(DependencyHint {
                label: label.to_string(),
                path: entry.path().to_string_lossy().to_string(),
                winetricks_verb: verb.map(|v| v.to_string()),
            });
        }
    }
    hints.sort_by(|a, b| a.label.cmp(&b.label));
    hints.dedup_by(|a, b| a.label == b.label);
    Ok(hints)
}

/// Runs a bundled installer executable directly inside a prefix using
/// `wine`. Silent/unattended install flags vary per-installer and aren't
/// guaranteed, so this just launches it and lets the user click through
/// whatever dialog appears; any console output it does produce is streamed
/// live the same way Winetricks' is. Refuses to start if the prefix is
/// already locked by another game or maintenance operation.
pub fn run_installer_in_prefix(prefix: &str, installer_path: &str, app_handle: &tauri::AppHandle) -> Result<()> {
    if which::which("wine").is_err() {
        bail!("Wine is not installed, cannot run the installer.");
    }
    fs::create_dir_all(prefix).ok();

    let _lock = crate::prefix_lock::lock_prefix(prefix, "a dependency installer")?;

    let mut child = std::process::Command::new("wine")
        .env("WINEPREFIX", prefix)
        .arg(installer_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to launch installer")?;

    let log = Arc::new(Mutex::new(String::new()));
    let mut readers = vec![];
    if let Some(stdout) = child.stdout.take() {
        let app = app_handle.clone();
        let log = log.clone();
        readers.push(std::thread::spawn(move || stream_lines(stdout, &app, &log, "installer")));
    }
    if let Some(stderr) = child.stderr.take() {
        let app = app_handle.clone();
        let log = log.clone();
        readers.push(std::thread::spawn(move || stream_lines(stderr, &app, &log, "installer")));
    }
    for r in readers {
        let _ = r.join();
    }

    let status = child.wait().context("Installer process error")?;
    if !status.success() {
        bail!("Installer exited with a non-zero status");
    }
    Ok(())
}

// ─────────────────────────────────────────────
//  Controllers (informational — listing only)
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ControllerInfo {
    pub name: String,
    pub handler: String,
}

/// Lists joystick/gamepad devices currently visible to the kernel by
/// reading `/proc/bus/input/devices`. This is informational only (confirms
/// the OS sees the pad at all) — actual per-game remapping is done through
/// the `SDL_GAMECONTROLLERCONFIG` env var and the "Disable Steam Input"
/// toggle stored on each Game, since a launcher-level input remapper akin
/// to Steam Input itself is out of scope here.
pub fn list_controllers() -> Result<Vec<ControllerInfo>> {
    let content = fs::read_to_string("/proc/bus/input/devices").unwrap_or_default();
    let mut controllers = vec![];
    let mut current_name = String::new();
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("N: Name=") {
            current_name = rest.trim_matches('"').to_string();
        } else if let Some(rest) = line.strip_prefix("H: Handlers=") {
            if let Some(handler) = rest.split_whitespace().find(|h| h.starts_with("js")) {
                controllers.push(ControllerInfo {
                    name: if current_name.is_empty() {
                        "Unknown controller".to_string()
                    } else {
                        current_name.clone()
                    },
                    handler: handler.to_string(),
                });
            }
        }
    }
    Ok(controllers)
}

/// One captured press/movement from a physical controller, used by the
/// `SDL_GAMECONTROLLERCONFIG` mapping wizard so the user doesn't have to
/// know raw button/axis numbers by heart.
#[derive(Debug, Clone, Serialize)]
pub struct ControllerInputEvent {
    pub kind: String, // "button" | "axis"
    pub number: u8,
    pub value: i16,
}

/// Blocks (in a background thread, joined with a timeout) until the given
/// joystick device reports a real button press or a significant axis
/// movement, then returns which one. Used to build an `SDL_GAMECONTROLLERCONFIG`
/// entry ("press the button you want to use for A") without requiring the
/// user to already know the Linux joystick numbering for their pad.
///
/// Implementation note: this reads the classic Linux joystick API
/// (`/dev/input/jsN`, 8-byte `js_event` records) directly rather than
/// depending on SDL2 itself. Startup "init" events (which replay the
/// current state of every axis/button when the device is opened) are
/// filtered out. If nothing arrives before the timeout, the reader thread
/// is left blocked on the device and is cleaned up whenever the next input
/// event on that device eventually wakes it — an accepted, bounded cost for
/// a manual, occasionally-used wizard tool rather than something that runs
/// continuously.
pub fn capture_controller_input(handler: &str, timeout_ms: u64) -> Result<ControllerInputEvent> {
    let path = format!("/dev/input/{}", handler);
    let mut file =
        std::fs::File::open(&path).with_context(|| format!("Cannot open {}", path))?;

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 8];
        loop {
            if file.read_exact(&mut buf).is_err() {
                break;
            }
            let value = i16::from_le_bytes([buf[4], buf[5]]);
            let kind_byte = buf[6];
            let number = buf[7];
            let is_init = kind_byte & 0x80 != 0;
            let kind = kind_byte & !0x80;
            if is_init {
                continue;
            }
            if kind == 0x01 && value == 1 {
                let _ = tx.send(ControllerInputEvent {
                    kind: "button".to_string(),
                    number,
                    value,
                });
                break;
            } else if kind == 0x02 && value.unsigned_abs() > 16000 {
                let _ = tx.send(ControllerInputEvent {
                    kind: "axis".to_string(),
                    number,
                    value,
                });
                break;
            }
        }
    });

    rx.recv_timeout(Duration::from_millis(timeout_ms)).map_err(|_| {
        anyhow::anyhow!(
            "No input detected within {}ms — press a button or move a stick/trigger on the \
             controller and try again.",
            timeout_ms
        )
    })
}

fn read_hex_u16(path: &str) -> Result<u16> {
    let s = fs::read_to_string(path).with_context(|| format!("Cannot read {}", path))?;
    u16::from_str_radix(s.trim().trim_start_matches("0x"), 16)
        .with_context(|| format!("Unexpected content in {}", path))
}

/// Reconstructs the SDL2-style joystick GUID for a Linux `jsN` device from
/// its USB/Bluetooth identity in sysfs (bustype/vendor/product/version),
/// using the same 16-byte little-endian layout SDL uses on Linux. This is
/// what `SDL_GAMECONTROLLERCONFIG` entries are keyed on — get it wrong and
/// the custom mapping simply won't match the pad at runtime, so if this
/// can't be read, the wizard should let the user fall back to editing the
/// GUID field manually (it's exposed as an editable field, not baked in).
pub fn get_controller_guid(handler: &str) -> Result<String> {
    let base = format!("/sys/class/input/{}/device/id", handler);
    let bustype = read_hex_u16(&format!("{}/bustype", base))?;
    let vendor = read_hex_u16(&format!("{}/vendor", base))?;
    let product = read_hex_u16(&format!("{}/product", base))?;
    let version = read_hex_u16(&format!("{}/version", base))?;

    let mut bytes = [0u8; 16];
    bytes[0..2].copy_from_slice(&bustype.to_le_bytes());
    bytes[4..6].copy_from_slice(&vendor.to_le_bytes());
    bytes[8..10].copy_from_slice(&product.to_le_bytes());
    bytes[12..14].copy_from_slice(&version.to_le_bytes());
    Ok(bytes.iter().map(|b| format!("{:02x}", b)).collect())
}

// ─────────────────────────────────────────────
//  ProtonDB compatibility heuristic
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProtonDbInfo {
    #[serde(default)]
    pub tier: String,
    #[serde(default, rename = "trendingTier")]
    pub trending_tier: String,
    #[serde(default)]
    pub confidence: String,
}

const PROTONDB_CACHE_TTL: Duration = Duration::from_secs(15 * 60);

struct ProtonDbCacheEntry {
    info: Option<ProtonDbInfo>,
    fetched_at: Instant,
}

fn protondb_cache() -> &'static Mutex<HashMap<String, ProtonDbCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, ProtonDbCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Looks up a Steam App ID on ProtonDB's public summary API. Only
/// meaningful for games that correspond to a real Steam store entry — for
/// arbitrary non-Steam executables there's nothing to look up, so an empty
/// App ID simply returns `None` rather than an error. Results are cached
/// for 15 minutes per App ID so re-checking the same game repeatedly (e.g.
/// re-opening its Configure dialog) doesn't hammer ProtonDB and risk
/// throttling.
pub async fn check_protondb(app_id: &str) -> Result<Option<ProtonDbInfo>> {
    let app_id = app_id.trim();
    if app_id.is_empty() || !app_id.chars().all(|c| c.is_ascii_digit()) {
        return Ok(None);
    }

    if let Some(entry) = protondb_cache().lock().unwrap().get(app_id) {
        if entry.fetched_at.elapsed() < PROTONDB_CACHE_TTL {
            return Ok(entry.info.clone());
        }
    }

    let url = format!(
        "https://www.protondb.com/api/v1/reports/summaries/{}.json",
        app_id
    );
    let client = reqwest::Client::builder()
        .user_agent("hacker-launcher/1.0")
        .build()?;
    let resp = client.get(&url).send().await?;

    let result: Option<ProtonDbInfo> = if resp.status() == reqwest::StatusCode::NOT_FOUND {
        None
    } else if !resp.status().is_success() {
        bail!("ProtonDB request failed with status {}", resp.status());
    } else {
        Some(
            resp.json()
                .await
                .context("Failed to parse ProtonDB response")?,
        )
    };

    protondb_cache().lock().unwrap().insert(
        app_id.to_string(),
        ProtonDbCacheEntry {
            info: result.clone(),
            fetched_at: Instant::now(),
        },
    );
    Ok(result)
}

// ─────────────────────────────────────────────
//  Backup / restore
// ─────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct BackupBundle {
    format_version: u32,
    games: Vec<Game>,
    settings: Settings,
}

pub fn export_backup(games_file: &Path, settings_file: &Path, dest: &Path) -> Result<()> {
    let games: Vec<Game> = if games_file.exists() {
        serde_json::from_str(&fs::read_to_string(games_file)?).unwrap_or_default()
    } else {
        vec![]
    };
    let settings: Settings = if settings_file.exists() {
        serde_json::from_str(&fs::read_to_string(settings_file)?).unwrap_or_default()
    } else {
        Settings::default()
    };
    let bundle = BackupBundle {
        format_version: 1,
        games,
        settings,
    };
    fs::write(dest, serde_json::to_string_pretty(&bundle)?)
        .with_context(|| format!("Failed to write backup to {}", dest.display()))?;
    Ok(())
}

/// Restores a backup. `merge = true` adds any games from the backup whose
/// name doesn't already exist locally (settings untouched); `merge = false`
/// replaces both the games list and settings outright.
pub fn import_backup(games_file: &Path, settings_file: &Path, src: &Path, merge: bool) -> Result<()> {
    let content =
        fs::read_to_string(src).with_context(|| format!("Failed to read {}", src.display()))?;
    let bundle: BackupBundle = serde_json::from_str(&content).context(
        "This doesn't look like a Hacker Launcher backup file (invalid or unrecognized JSON)",
    )?;

    if merge {
        let mut existing: Vec<Game> = if games_file.exists() {
            serde_json::from_str(&fs::read_to_string(games_file)?).unwrap_or_default()
        } else {
            vec![]
        };
        let mut added = 0;
        for g in bundle.games {
            if !existing.iter().any(|e| e.name.eq_ignore_ascii_case(&g.name)) {
                existing.push(g);
                added += 1;
            }
        }
        fs::write(games_file, serde_json::to_string_pretty(&existing)?)?;
        if added == 0 {
            bail!("Nothing new to import — every game in the backup already exists locally.");
        }
    } else {
        fs::write(games_file, serde_json::to_string_pretty(&bundle.games)?)?;
        fs::write(settings_file, serde_json::to_string_pretty(&bundle.settings)?)?;
    }
    Ok(())
}
