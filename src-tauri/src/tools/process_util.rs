use std::sync::{Arc, Mutex};
use tauri::Emitter;

/// Reads a piped child process stream line-by-line, appending each line to
/// a shared log buffer and emitting it live as a `process_output` event so
/// the UI can show real-time progress instead of a silent "is this frozen?"
/// wait for operations that can take several minutes (Winetricks verbs like
/// `dotnet48`, bundled dependency installers, APK installs).
///
/// Shared by `winetricks`, `dependency_scan`, and `crate::android_manager`
/// rather than each keeping its own copy, since it's the exact same
/// pattern in all three places.
pub(crate) fn stream_lines<R: std::io::Read>(
    reader: R,
    app: &tauri::AppHandle,
    log: &Arc<Mutex<String>>,
    source: &str,
) {
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
