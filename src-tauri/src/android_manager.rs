use crate::tools::process_util::stream_lines;
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

/// Android apps run under **Waydroid**, a container-based Android runtime
/// for Linux — there's no Proton/Wine equivalent here since `.apk`s are
/// Android (Dalvik/ART) binaries, not Windows ones. Waydroid is the de
/// facto standard for this on modern (Wayland) Linux desktops; see
/// https://docs.waydro.id for what it is and how to install it.
///
/// True if the `waydroid` CLI is available at all.
pub fn waydroid_available() -> bool {
    which::which("waydroid").is_ok()
}

/// Extracts the Android package ID (e.g. `com.mojang.minecraftpe`) from an
/// `.apk` file using `aapt` or `aapt2` (Android SDK build-tools). Waydroid
/// launches/stops apps by package ID, not by file path, so this is needed
/// right after picking an APK — same role `find_main_exe` plays for Steam
/// imports, except here there's a tool that can just tell us directly
/// instead of having to guess. If neither `aapt` nor `aapt2` is installed,
/// this fails with a clear message so the caller can fall back to letting
/// the user type the package ID in by hand (the ADD/CONFIGURE dialogs both
/// keep that field editable for exactly this reason).
pub fn extract_apk_package_name(apk_path: &str) -> Result<String> {
    if !Path::new(apk_path).exists() {
        bail!("APK not found: {}", apk_path);
    }
    for tool in ["aapt", "aapt2"] {
        if which::which(tool).is_err() {
            continue;
        }
        let Ok(output) = std::process::Command::new(tool)
            .arg("dump")
            .arg("badging")
            .arg(apk_path)
            .output()
        else {
            continue;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some(line) = text.lines().find(|l| l.starts_with("package:")) {
            if let Some(start) = line.find("name='") {
                let after = &line[start + "name='".len()..];
                if let Some(quote) = after.find('\'') {
                    return Ok(after[..quote].to_string());
                }
            }
        }
    }
    bail!(
        "Couldn't read the package name from this APK — `aapt`/`aapt2` isn't installed, or the \
         file isn't a valid APK. Install your distro's `android-sdk-build-tools` (or similar) \
         package, or just enter the package ID manually (e.g. com.example.app)."
    )
}

/// Makes sure a Waydroid session is up before installing/launching
/// anything. `waydroid session start` blocks until the container is ready
/// the first time (which can take a little while on a cold start) and is
/// harmless to call again if a session is already running.
fn ensure_session_started() {
    let running = std::process::Command::new("waydroid")
        .arg("status")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("RUNNING"))
        .unwrap_or(false);
    if !running {
        let _ = std::process::Command::new("waydroid")
            .arg("session")
            .arg("start")
            .status();
    }
}

/// Installs an `.apk` into the Waydroid container, streaming output live
/// the same way Winetricks does, then returns the package name (extracted
/// from the APK itself) so the caller can save it on the Game record —
/// launching/stopping later needs the package ID, not the file path.
pub fn install_apk(apk_path: &str, app_handle: &tauri::AppHandle) -> Result<String> {
    if !waydroid_available() {
        bail!(
            "Waydroid is not installed. Android/.apk support needs it — see \
             https://docs.waydro.id for installation instructions."
        );
    }
    if !Path::new(apk_path).exists() {
        bail!("APK not found: {}", apk_path);
    }
    ensure_session_started();

    let mut child = std::process::Command::new("waydroid")
        .arg("app")
        .arg("install")
        .arg(apk_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to start `waydroid app install`")?;

    let log = Arc::new(Mutex::new(String::new()));
    let mut readers = vec![];
    if let Some(stdout) = child.stdout.take() {
        let app = app_handle.clone();
        let log = log.clone();
        readers.push(std::thread::spawn(move || {
            stream_lines(stdout, &app, &log, "apk_install")
        }));
    }
    if let Some(stderr) = child.stderr.take() {
        let app = app_handle.clone();
        let log = log.clone();
        readers.push(std::thread::spawn(move || {
            stream_lines(stderr, &app, &log, "apk_install")
        }));
    }
    for r in readers {
        let _ = r.join();
    }

    let status = child.wait().context("`waydroid app install` process error")?;
    if !status.success() {
        bail!(
            "`waydroid app install` exited with an error:\n{}",
            log.lock().unwrap().trim()
        );
    }

    extract_apk_package_name(apk_path).context(
        "APK installed, but its package name couldn't be read automatically — enter it \
         manually in the game's settings so it can be launched.",
    )
}

/// Launches an already-installed Android app by package ID. Unlike
/// Native/Wine/Proton/Flatpak games, the returned `Child` is just the
/// short-lived `waydroid app launch` helper — Waydroid hands the request
/// off to its container and the app runs there, not as a child of this
/// process — so this alone can't be used to track "is it still running"
/// or measure playtime the way the other runners' processes can. See
/// `GameManager::launch_android_game` / `stop_apk` for how that's handled.
pub fn launch_apk(package_name: &str) -> Result<std::process::Child> {
    if !waydroid_available() {
        bail!("Waydroid is not installed.");
    }
    let package_name = package_name.trim();
    if package_name.is_empty() {
        bail!("No Android package name set for this game — install/configure it before launching.");
    }
    ensure_session_started();
    std::process::Command::new("waydroid")
        .arg("app")
        .arg("launch")
        .arg(package_name)
        .spawn()
        .context("Failed to launch `waydroid app launch`")
}

/// Best-effort check for whether `package_name` currently has a running
/// process inside the Waydroid container, by asking Android's own process
/// table via `waydroid shell -- pidof <package>`. This is what lets
/// `GameManager::launch_android_game` detect the app's *actual* exit
/// (closed from inside Android, crashed, killed by the system) instead of
/// only reacting to the user clicking Stop in the launcher — the same role
/// `Child::wait()` plays for the other runners, just polled instead of
/// event-driven, since Waydroid gives no way to be notified of a
/// container-internal process exit directly.
///
/// This assumes the app's main process is named after its package ID,
/// which `pidof` needs to match on and which holds for the large majority
/// of single-process Android apps — but can miss or misreport for apps
/// that spawn background services under a different process name, or that
/// keep a service alive briefly after their visible UI has closed. It's a
/// reasonable approximation, not a precise "is this app still doing
/// anything" signal.
pub fn is_app_running(package_name: &str) -> bool {
    let package_name = package_name.trim();
    if package_name.is_empty() {
        return false;
    }
    std::process::Command::new("waydroid")
        .arg("shell")
        .arg("--")
        .arg("pidof")
        .arg(package_name)
        .output()
        .map(|o| o.status.success() && !o.stdout.iter().all(|b| b.is_ascii_whitespace()))
        .unwrap_or(false)
}

/// Best-effort stop for a running Android app. Recent Waydroid versions
/// support `waydroid app stop <package>`; older ones don't have a
/// per-app kill at all (only stopping the whole container/session), so
/// this reports failure honestly rather than pretending the app stopped
/// when it may not have.
pub fn stop_apk(package_name: &str) -> Result<()> {
    let package_name = package_name.trim();
    if package_name.is_empty() {
        bail!("No Android package name set for this game.");
    }
    let status = std::process::Command::new("waydroid")
        .arg("app")
        .arg("stop")
        .arg(package_name)
        .status()
        .context("Failed to invoke `waydroid app stop`")?;
    if !status.success() {
        bail!(
            "`waydroid app stop` didn't report success — your Waydroid version may not support \
             stopping individual apps. You can stop the whole session from a terminal instead \
             with `waydroid session stop` (this will close every running Android app)."
        );
    }
    Ok(())
}
