use anyhow::{bail, Result};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Registry of Wine prefixes currently "in use" — either a game running
/// against them, or a maintenance operation (Winetricks, a dependency
/// installer) running against them. Two operations sharing the same prefix
/// at the same time is a real corruption risk (concurrent writes to the
/// same `system.reg`/`user.reg`), so every entry point that touches a
/// prefix goes through `lock_prefix` first.
///
/// This in-memory map only ever knows about operations this launcher
/// process itself started. `lock_prefix` additionally cross-checks
/// `/proc` (see `find_external_prefix_users`) for anything else on the
/// system already using the same prefix, so the guard isn't limited to
/// purely launcher-initiated conflicts.
fn registry() -> &'static Mutex<HashMap<String, String>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Normalizes a prefix path so trivially different strings that point at
/// the same directory (trailing slash, `.` components) don't bypass the
/// lock.
fn normalize(prefix: &str) -> String {
    let trimmed = prefix.trim().trim_end_matches('/');
    std::fs::canonicalize(trimmed)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| trimmed.to_string())
}

/// RAII guard: the prefix is released automatically when this value is
/// dropped, whether that's at the end of a function (Winetricks, dependency
/// installer) or — for a launched game — whenever the background thread
/// watching that game's process finishes, since the guard is moved into
/// that thread's closure.
pub struct PrefixLockGuard {
    key: String,
}

impl Drop for PrefixLockGuard {
    fn drop(&mut self) {
        registry().lock().unwrap().remove(&self.key);
    }
}

/// Scans `/proc` for other processes already touching `normalized_prefix`,
/// combining two independent signals so something started completely
/// outside this launcher — a bare `wine game.exe` in a terminal, a second
/// independent copy of Hacker Launcher, another tool entirely — can still
/// be caught, rather than only guarding against the launcher's own
/// in-memory bookkeeping (which is all the in-memory `registry()` above
/// can ever see):
///
///  1. `WINEPREFIX` in the process's environment matching this prefix —
///     catches the common case of someone running `wine`/`winetricks`
///     directly with that variable set.
///  2. An open file descriptor pointing to something *inside* the prefix
///     directory (checked via `/proc/<pid>/fd/*`, which are symlinks to
///     the real file each fd refers to) — catches processes that reach a
///     prefix some other way (a wrapper script, a tool that computes the
///     prefix path internally) without ever setting `WINEPREFIX`
///     themselves, as long as they have at least one prefix file open
///     (e.g. `system.reg`/`user.reg`, which Wine keeps open for the life
///     of the session).
///
/// Both signals are still best-effort: they only see processes that are
/// (a) still alive at the moment of the check, and (b) readable by the
/// current user (or root) — `/proc/<pid>/environ` and `/proc/<pid>/fd/*`
/// are both restricted to the process owner. A process running as a
/// different user touching the same prefix (unusual, but possible on a
/// shared machine) is invisible to either check.
fn find_external_prefix_users(normalized_prefix: &str) -> Vec<(u32, String)> {
    let mut found: HashMap<u32, String> = HashMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return vec![];
    };
    let our_pid = std::process::id();

    for entry in entries.filter_map(|e| e.ok()) {
        let file_name = entry.file_name();
        let Some(pid_str) = file_name.to_str() else { continue };
        let Ok(pid) = pid_str.parse::<u32>() else { continue }; // skip non-PID /proc entries
        if pid == our_pid {
            continue;
        }

        let comm = || {
            std::fs::read_to_string(entry.path().join("comm"))
                .unwrap_or_default()
                .trim()
                .to_string()
        };

        // Signal 1: WINEPREFIX in the environment.
        if let Ok(raw) = std::fs::read(entry.path().join("environ")) {
            let matched = raw.split(|b| *b == 0).any(|var| {
                let Ok(s) = std::str::from_utf8(var) else { return false };
                let Some(value) = s.strip_prefix("WINEPREFIX=") else { return false };
                let candidate = std::fs::canonicalize(value)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| value.trim_end_matches('/').to_string());
                candidate == normalized_prefix
            });
            if matched {
                let name = comm();
                found.entry(pid).or_insert(if name.is_empty() { "unknown process".to_string() } else { name });
                continue; // already confirmed; no need to also scan its fds
            }
        }

        // Signal 2: an open file descriptor into the prefix directory.
        let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else {
            continue; // process exited mid-scan, or not readable by us
        };
        for fd in fds.filter_map(|f| f.ok()) {
            let Ok(target) = std::fs::read_link(fd.path()) else { continue };
            if target.starts_with(normalized_prefix) {
                let name = comm();
                found.entry(pid).or_insert(if name.is_empty() { "unknown process".to_string() } else { name });
                break;
            }
        }
    }

    found.into_iter().collect()
}

/// Attempts to claim `prefix` for `holder` (a human-readable description
/// like `game "Foo"` or `Winetricks`, used in the error message shown to
/// the user if someone else already holds it). Fails immediately rather
/// than waiting, since blocking here would freeze the launcher's UI thread.
pub fn lock_prefix(prefix: &str, holder: &str) -> Result<PrefixLockGuard> {
    if prefix.trim().is_empty() {
        // Nothing to lock (e.g. a runner that doesn't use a prefix at all);
        // return a guard whose key simply doesn't exist in the registry.
        return Ok(PrefixLockGuard { key: String::new() });
    }
    let key = normalize(prefix);
    let mut map = registry().lock().unwrap();
    if let Some(existing) = map.get(&key) {
        bail!(
            "This Wine prefix is currently in use by {} — wait for it to finish before starting \
             another operation on the same prefix (running two things against one prefix at once \
             can corrupt it).",
            existing
        );
    }

    // The in-memory check above only knows about operations *this*
    // launcher process started. Also check for anything else on the system
    // already sitting on this prefix before handing out the lock.
    let external = find_external_prefix_users(&key);
    if !external.is_empty() {
        let details = external
            .iter()
            .map(|(pid, comm)| format!("{} (pid {})", comm, pid))
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "This Wine prefix already appears to be in use outside the launcher by: {} — wait \
             for it to finish first (running two things against one prefix at once can corrupt \
             it). If this looks wrong (e.g. a stale process), check with `ps` and close it \
             manually before retrying.",
            details
        );
    }

    map.insert(key.clone(), holder.to_string());
    Ok(PrefixLockGuard { key })
}
