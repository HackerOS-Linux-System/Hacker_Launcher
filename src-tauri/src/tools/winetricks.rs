use super::process_util::stream_lines;
use anyhow::{bail, Context, Result};
use std::fs;
use std::sync::{Arc, Mutex};

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

/// Runs `winetricks -q <verbs...>` against a given prefix, streaming output
/// live (see `process_util::stream_lines`) and returning the full captured
/// log once it finishes. Blocking — the caller (a Tauri command) should run
/// this inside `spawn_blocking`, since a verb like `dotnet48` can
/// legitimately take a few minutes. Refuses to start if the prefix is
/// already locked by another game or maintenance operation.
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
