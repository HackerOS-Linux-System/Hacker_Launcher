use super::process_util::stream_lines;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize)]
pub struct DependencyHint {
    pub label: String,
    pub path: String,
    pub winetricks_verb: Option<String>,
    /// Best-effort guess at whether this dependency is already satisfied
    /// in the target prefix (see `is_component_installed`). `false` just
    /// means "we can't confirm it's installed" — it's still shown either
    /// way, since we'd rather over-suggest than silently hide a real gap.
    pub already_installed: bool,
}

/// Paths (relative to a prefix's `drive_c`) whose presence is a reasonable
/// signal that a given winetricks verb's payload is already installed.
/// This is the fallback signal — see `is_component_installed` for the
/// stronger one (winetricks' own install log) this is paired with. Can't be
/// a real "is it installed" query either way — Wine doesn't expose one —
/// so it's the same kind of filename/existence heuristic as the installer
/// scan below, just checking the *result* of an install instead of the
/// presence of an installer.
fn verb_marker_paths(verb: &str) -> &'static [&'static str] {
    match verb {
        "vcrun2022" | "vcrun2019" | "vcrun2017" | "vcrun2015" | "vcrun2013" => &[
            "windows/system32/vcruntime140.dll",
            "windows/syswow64/vcruntime140.dll",
        ],
        "dotnet48" | "dotnet472" | "dotnet471" | "dotnet47" | "dotnet462" => &[
            "windows/Microsoft.NET/Framework/v4.0.30319/mscorlib.dll",
            "windows/Microsoft.NET/Framework64/v4.0.30319/mscorlib.dll",
        ],
        "dotnet6" | "dotnet7" | "dotnet8" => &["windows/system32/hostfxr.dll"],
        "d3dx9" => &["windows/system32/d3dx9_43.dll"],
        "d3dx11_43" => &["windows/system32/d3dx11_43.dll"],
        // DXVK/VKD3D install by overriding these DLLs with native (non-Wine)
        // builds — their mere existence isn't proof of a DXVK build
        // specifically, but is still a reasonable signal something d3d-ish
        // is already in place, worth surfacing as "probably already set up".
        "dxvk" => &["windows/system32/d3d9.dll", "windows/syswow64/d3d9.dll"],
        "vkd3d" => &["windows/system32/d3d12.dll"],
        "physx" => &[
            "Program Files (x86)/NVIDIA Corporation/PhysX",
            "Program Files/NVIDIA Corporation/PhysX",
        ],
        "openal" => &["windows/system32/OpenAL32.dll"],
        "corefonts" => &["windows/Fonts/times.ttf"],
        "xact" => &["windows/system32/xactengine3_7.dll"],
        _ => &[],
    }
}

/// Reads `<prefix>/winetricks.log` — the file winetricks itself appends one
/// verb name to after each successful install, and checks itself before
/// deciding whether to re-run something. Piggybacking on that file is a
/// meaningfully stronger signal than the marker-file check below: it's not
/// inferring "installed" from a side effect, it's reading winetricks' own
/// record of what it actually ran successfully against this exact prefix.
/// Its blind spot is the mirror image of the marker-file check's: it only
/// knows about verbs that were installed *via winetricks* (this launcher's
/// "Run via Winetricks" button, or a manual `winetricks` invocation against
/// the same prefix) — a dependency the game's own bundled installer put in
/// place directly wouldn't be logged here at all.
fn winetricks_log_has_verb(prefix: &str, verb: &str) -> bool {
    let log_path = Path::new(prefix).join("winetricks.log");
    let Ok(content) = fs::read_to_string(&log_path) else {
        return false;
    };
    content.lines().any(|line| line.trim() == verb)
}

/// Best-effort check for whether `verb`'s payload already looks present in
/// `prefix`, combining two independent signals: winetricks' own install
/// log (`winetricks_log_has_verb`, checked first since it's the stronger
/// signal when it applies) and known marker DLLs/paths a successful
/// install would leave behind (`verb_marker_paths`, checked as a fallback
/// for dependencies installed some other way — e.g. bundled directly with
/// the game). Returns `false` (i.e. "still offer to install it") whenever
/// neither signal is available or matches — the safe default is to keep
/// suggesting the install rather than silently hide a hint. Neither signal
/// is a real registry/component query; Wine doesn't expose one.
fn is_component_installed(prefix: &str, verb: &str) -> bool {
    if prefix.trim().is_empty() {
        return false;
    }
    if winetricks_log_has_verb(prefix, verb) {
        return true;
    }
    let drive_c = Path::new(prefix).join("drive_c");
    verb_marker_paths(verb).iter().any(|m| drive_c.join(m).exists())
}

/// Looks for common redistributable installers bundled next to a game's
/// executable (a real-world convention for many older/indie titles) and
/// suggests either running the bundled installer directly or the
/// equivalent winetricks verb, flagging any whose payload already looks
/// present in `prefix` so the user isn't nudged to reinstall something
/// that's already there. This is a filename-pattern heuristic, not static
/// analysis of the game binary's actual imports — it will miss
/// dependencies that aren't shipped as a visible installer file, and the
/// "already installed" flag is itself a best-effort check (see
/// `is_component_installed`), not a real registry/component query.
pub fn scan_game_dependencies(exe_path: &str, prefix: &str) -> Result<Vec<DependencyHint>> {
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
            let already_installed = verb.map(|v| is_component_installed(prefix, v)).unwrap_or(false);
            hints.push(DependencyHint {
                label: label.to_string(),
                path: entry.path().to_string_lossy().to_string(),
                winetricks_verb: verb.map(|v| v.to_string()),
                already_installed,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winetricks_log_match_is_exact_line() {
        let dir = std::env::temp_dir().join(format!("hl-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("winetricks.log"), "corefonts\nvcrun2022\ndxvk\n").unwrap();

        assert!(winetricks_log_has_verb(dir.to_str().unwrap(), "vcrun2022"));
        assert!(!winetricks_log_has_verb(dir.to_str().unwrap(), "vcrun2019"));
        assert!(!is_component_installed("", "vcrun2022")); // empty prefix -> always false

        fs::remove_dir_all(&dir).ok();
    }
}
