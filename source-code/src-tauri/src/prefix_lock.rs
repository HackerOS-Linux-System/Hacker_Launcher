use anyhow::{bail, Result};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Registry of Wine prefixes currently "in use" — either a game running
/// against them, or a maintenance operation (Winetricks, a dependency
/// installer) running against them. Two operations sharing the same prefix
/// at the same time is a real corruption risk (concurrent writes to the
/// same `system.reg`/`user.reg`), so every entry point that touches a
/// prefix goes through `lock_prefix` first.
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
    map.insert(key.clone(), holder.to_string());
    Ok(PrefixLockGuard { key })
}
