use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct SteamGameCandidate {
    pub name: String,
    pub app_id: String,
    pub exe_path: String,
    pub install_dir: String,
    /// Where `exe_path` came from, shown in the import review dialog so the
    /// user knows how much to trust it before committing:
    ///  - `"steam metadata"` — read directly out of Steam's own launch
    ///    config for this App ID (see `find_launch_executable`). This is
    ///    what the real Steam client itself would run, not a guess.
    ///  - `"heuristic scan"` — Steam's launch config wasn't available (no
    ///    local `appinfo.vdf` entry, unreadable, or didn't parse), so this
    ///    falls back to `find_main_exe`'s scoring heuristic instead.
    ///  - `""` — neither approach found anything; the field is left for
    ///    the user to fill in by hand.
    pub source: String,
}

/// Very small, tolerant parser for Valve's *text* VDF/ACF key-value format
/// (`appmanifest_*.acf`, `libraryfolders.vdf`). It only understands flat
/// `"key"    "value"` lines — nested blocks are simply ignored rather than
/// fully parsed, which is all these particular files need. `entry().or_insert()`
/// is used so a key seen at the top level (which appears first in these
/// files) always wins over a same-named key nested deeper in the file.
///
/// This is unrelated to the *binary* VDF format `appinfo.vdf` uses — see
/// `parse_binary_object` below for that one.
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

// ─────────────────────────────────────────────
//  appinfo.vdf — Steam's own launch config (primary signal)
// ─────────────────────────────────────────────
//
// Steam caches per-app metadata — including the *actual* launch config it
// uses to start each game — in a binary file at
// `<Steam root>/appcache/appinfo.vdf`. This isn't official/documented
// format (Valve has never published a spec for it), it's understood from
// community reverse-engineering (SteamKit and various Python/Go
// reimplementations all agree on this layout), so treat it as: try it,
// validate the result looks like a real relative path, and silently fall
// back to the `find_main_exe` heuristic below if *anything* about it
// doesn't check out. This is a strictly-better starting guess than the
// heuristic alone when it works, not a replacement for the "let the user
// correct it" safety net the import dialog already has — a changed Steam
// client version could shift this format at any point.

/// One key's parsed value from a binary VDF object. Anything we don't
/// specifically need (ints, floats, colors, etc.) is parsed just enough to
/// know its byte length (so the reader position stays correct) and
/// otherwise discarded — `Other`.
enum VdfValue {
    Str(String),
    Obj(BTreeMap<String, VdfValue>),
    Other,
}

struct BinReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BinReader<'a> {
    fn u8(&mut self) -> Option<u8> {
        let b = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        if end > self.data.len() {
            return None;
        }
        let s = &self.data[self.pos..end];
        self.pos = end;
        Some(s)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    /// Reads a null-terminated string. Bounded by the buffer length so a
    /// corrupt/truncated blob can't spin this into an infinite loop.
    fn cstr(&mut self) -> Option<String> {
        let start = self.pos;
        loop {
            if self.pos >= self.data.len() {
                return None; // ran off the end without a terminator
            }
            if self.data[self.pos] == 0 {
                break;
            }
            self.pos += 1;
        }
        let s = String::from_utf8_lossy(&self.data[start..self.pos]).to_string();
        self.pos += 1; // consume the terminator
        Some(s)
    }
}

/// Parses one binary-VDF object (a run of typed key/value pairs terminated
/// by a `0x08` end marker). Recognized type bytes, per the community
/// reverse-engineering of this format: `0x00` nested object, `0x01`
/// string, `0x02`/`0x03`/`0x04` 4-byte scalars (int32/float32/pointer),
/// `0x05` wide string, `0x06` 4-byte "color", `0x07` 8-byte uint64. An
/// unrecognized type byte means our understanding of the format doesn't
/// match this file (e.g. a future Steam client revision) — bail out
/// rather than guess, so the caller falls back to the exe-scoring
/// heuristic instead of returning garbage.
fn parse_binary_object(r: &mut BinReader) -> Option<BTreeMap<String, VdfValue>> {
    let mut map = BTreeMap::new();
    loop {
        let t = r.u8()?;
        if t == 0x08 {
            return Some(map);
        }
        let key = r.cstr()?;
        let val = match t {
            0x00 => VdfValue::Obj(parse_binary_object(r)?),
            0x01 => VdfValue::Str(r.cstr()?),
            0x02 | 0x03 | 0x04 => {
                r.take(4)?;
                VdfValue::Other
            }
            0x05 => {
                r.cstr()?; // approximated as a null-terminated read
                VdfValue::Other
            }
            0x06 => {
                r.take(4)?;
                VdfValue::Other
            }
            0x07 => {
                r.take(8)?;
                VdfValue::Other
            }
            _ => return None, // unknown type: format mismatch, bail
        };
        map.insert(key, val);
    }
}

/// Walks a parsed object looking for `"executable"` string values that sit
/// somewhere underneath a `"launch"` key (Steam's launch config can nest
/// multiple numbered launch entries, one per OS/config combination — we
/// don't attempt to pick the "right" one for the current OS/arch, since in
/// practice the first one found is virtually always correct for a
/// Windows-only game running under Proton). Returns the first match found
/// via a straightforward depth-first walk.
fn find_launch_executable(obj: &BTreeMap<String, VdfValue>) -> Option<String> {
    fn walk(map: &BTreeMap<String, VdfValue>, in_launch: bool) -> Option<String> {
        for (key, val) in map {
            let now_in_launch = in_launch || key.eq_ignore_ascii_case("launch");
            if in_launch && key.eq_ignore_ascii_case("executable") {
                if let VdfValue::Str(s) = val {
                    return Some(s.clone());
                }
            }
            if let VdfValue::Obj(nested) = val {
                if let Some(found) = walk(nested, now_in_launch) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(obj, false)
}

/// Scans `<steam_root>/appcache/appinfo.vdf` for `app_id` and returns the
/// executable path from its launch config, if the entry exists and parses
/// cleanly. `steam_root` is a Steam *root* directory (the parent of a
/// `steamapps` dir from `candidate_steamapps_dirs`), not the steamapps dir
/// itself. Every failure mode here (missing file, unrecognized header,
/// corrupt/truncated entry, app ID not found, parse error) is treated the
/// same way: return `None` and let the caller fall back to the heuristic —
/// this function deliberately never surfaces an error, since "appinfo.vdf
/// didn't give us an answer" is an expected, common outcome, not a bug.
fn read_appinfo_launch_exe(steam_root: &Path, app_id: &str) -> Option<String> {
    let target_appid: u32 = app_id.parse().ok()?;
    let path = steam_root.join("appcache").join("appinfo.vdf");
    let bytes = fs::read(&path).ok()?;

    let mut r = BinReader { data: &bytes, pos: 0 };
    let magic = r.u32()?;
    let _universe = r.u32()?;
    // 0x07564427 = pre-2023 format; 0x07564428 = current format (adds a
    // second, binary-VDF checksum after `change_number`). Both are known,
    // stable Steam appinfo.vdf magics; anything else means this parser's
    // understanding of the format is out of date.
    let fixed_header_len = match magic {
        0x07564427 => 40, // state+last_updated+access_token+checksum_txt(20)+change_number
        0x07564428 => 60, // ...+checksum_binary(20)
        _ => return None,
    };

    loop {
        let this_appid = r.u32()?;
        if this_appid == 0 {
            return None; // reached the terminator without finding our app
        }
        let size = r.u32()? as usize;
        let data_len = size.checked_sub(fixed_header_len)?;
        r.take(fixed_header_len)?; // skip state/timestamps/checksums we don't need
        let obj_bytes = r.take(data_len)?;

        if this_appid != target_appid {
            continue; // `size` let us skip straight past this app's data
        }

        let mut obj_reader = BinReader { data: obj_bytes, pos: 0 };
        let obj = parse_binary_object(&mut obj_reader)?;
        let raw_exe = find_launch_executable(&obj)?;

        // Validate before trusting it: a plausible relative Windows path
        // ending in .exe, no path traversal. Anything else and we treat
        // this the same as "didn't find one" rather than risk handing back
        // something nonsensical from a misparsed blob.
        let normalized = raw_exe.replace('\\', "/");
        if normalized.contains("..") || normalized.starts_with('/') {
            return None;
        }
        if !normalized.to_lowercase().ends_with(".exe") {
            return None;
        }
        return Some(normalized);
    }
}

const EXE_DENYLIST: &[&str] = &[
    "unins", "redist", "vcredist", "vc_redist", "dxsetup", "dotnetfx", "crashpad",
    "crashreport", "crashhandler", "easyanticheat", "battleye", "helper",
    "vulkan", "directx", "setup.exe", "installer",
];

/// Scores a candidate `.exe` for "how likely is this the game's real launch
/// binary" — the fallback used when `read_appinfo_launch_exe` can't answer
/// the question directly (no local appinfo.vdf entry, or it didn't parse).
/// Combines several weak signals instead of the old "just pick the biggest
/// file" rule, which was frequently fooled by large non-game executables
/// (crash reporters, CEF subprocess helpers, bundled tool binaries) sitting
/// right next to a much smaller real one:
///  - name similarity to the game's title / install folder — by far the
///    strongest signal, since the large majority of games ship a root exe
///    named after themselves (or a close variant, e.g. `ELDEN RING.exe`)
///  - Unreal/Unity "-Win64-Shipping" style naming, a strong positive signal
///    when there's no better name match
///  - being at (or near) the root of the install folder rather than nested
///    several directories deep — dependency/tool binaries are usually
///    tucked away in subfolders, and even when the real work happens in a
///    nested "Shipping" binary, the root exe is very often the one meant
///    to be launched (it may do first-run setup before handing off)
///  - file size, kept only as a weak logarithmic tie-breaker, since it was
///    the sole and unreliable signal before
/// This still can't be perfect without Steam's own launch config, which is
/// exactly why `read_appinfo_launch_exe` is tried first — this is only
/// reached when that comes up empty, and even then the import dialog lets
/// the user correct the guess before committing.
fn score_exe_candidate(path: &Path, install_root: &Path, name_hint: &str) -> f64 {
    let file_stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let hint_compact: String = name_hint
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    let stem_compact: String = file_stem.chars().filter(|c| c.is_alphanumeric()).collect();

    let mut score = 0.0;

    if !hint_compact.is_empty() {
        if stem_compact == hint_compact {
            score += 100.0;
        } else if !stem_compact.is_empty()
            && (stem_compact.contains(&hint_compact) || hint_compact.contains(&stem_compact))
        {
            score += 60.0;
        }
    }

    if file_stem.ends_with("-win64-shipping")
        || file_stem.ends_with("-win32-shipping")
        || file_stem.ends_with("_win64_shipping")
        || file_stem.ends_with("_win32_shipping")
    {
        score += 50.0;
    }

    let depth = path
        .strip_prefix(install_root)
        .map(|p| p.components().count())
        .unwrap_or(4);
    score -= (depth.saturating_sub(1)) as f64 * 8.0;

    let lower_path = path.to_string_lossy().to_lowercase();
    for bad_dir in [
        "redist",
        "_commonredist",
        "support",
        "thirdparty",
        "third_party",
        "tools",
        "installer",
    ] {
        if lower_path.contains(bad_dir) {
            score -= 40.0;
        }
    }
    // Mild only, since plenty of legitimate games really are launched via a
    // "Launcher.exe" — the name-similarity and depth signals above already
    // do most of the real work of telling that apart from a bootstrapper.
    if file_stem.contains("launcher") {
        score -= 5.0;
    }

    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0) as f64;
    score += size.max(1.0).log10();

    score
}

fn find_main_exe(install_path: &Path, name_hint: &str) -> Option<PathBuf> {
    let mut best: Option<(PathBuf, f64)> = None;
    for entry in walkdir::WalkDir::new(install_path)
        .max_depth(4)
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
        let score = score_exe_candidate(entry.path(), install_path, name_hint);
        if best.as_ref().map(|(_, s)| score > *s).unwrap_or(true) {
            best = Some((entry.path().to_path_buf(), score));
        }
    }
    best.map(|(p, _)| p)
}

/// Scans every known Steam library for installed games by reading each
/// `appmanifest_*.acf`. For the main executable, this first tries reading
/// it straight out of Steam's own local launch-config cache
/// (`read_appinfo_launch_exe` — see that function's doc comment for what
/// this actually is and its caveats), and only falls back to the
/// `find_main_exe` scoring heuristic when that doesn't produce an answer.
/// Either way, `SteamGameCandidate::source` tells the import dialog which
/// one actually happened, so the user knows how much to double-check it.
pub fn scan_steam_library() -> Result<Vec<SteamGameCandidate>> {
    let mut results = vec![];
    for steamapps in candidate_steamapps_dirs() {
        // appinfo.vdf lives at the Steam *root*, one level up from steamapps.
        let steam_root = steamapps.parent().map(|p| p.to_path_buf());
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
            let display_name = if name.is_empty() { install_dir.clone() } else { name };

            let from_appinfo = steam_root
                .as_deref()
                .and_then(|root| read_appinfo_launch_exe(root, &app_id))
                .map(|rel| install_path.join(rel))
                .filter(|p| p.exists());

            let (exe_path, source) = if let Some(p) = from_appinfo {
                (p.to_string_lossy().to_string(), "steam metadata")
            } else if let Some(p) = find_main_exe(&install_path, &display_name) {
                (p.to_string_lossy().to_string(), "heuristic scan")
            } else {
                (String::new(), "")
            };

            results.push(SteamGameCandidate {
                name: display_name,
                app_id,
                exe_path,
                install_dir,
                source: source.to_string(),
            });
        }
    }
    results.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    results.dedup_by(|a, b| a.app_id == b.app_id);
    Ok(results)
}

// ─────────────────────────────────────────────
//  Steam App ID search (for games with no App ID on record)
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteamSearchResult {
    pub app_id: String,
    pub name: String,
}

/// Minimal percent-encoding for a search query, just enough to safely drop
/// arbitrary game-title text into a URL (spaces, punctuation, non-ASCII)
/// without pulling in a whole crate for it.
fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Looks up possible Steam App IDs for a game by name, via Steam's public
/// store-search endpoint. This is what makes ProtonDB lookup and (in
/// principle) future Steam-side metadata usable for games that were added
/// manually rather than via the Steam library importer — without an App
/// ID there's nothing for `protondb::check_protondb` to key on. Needs
/// network; on failure this surfaces the actual error rather than
/// returning an empty list, since "no results" and "couldn't reach Steam"
/// are different situations the user should be able to tell apart.
pub async fn search_steam_appid(query: &str) -> Result<Vec<SteamSearchResult>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(vec![]);
    }
    let url = format!(
        "https://store.steampowered.com/api/storesearch/?term={}&cc=us&l=english",
        percent_encode(query)
    );
    let client = reqwest::Client::builder()
        .user_agent("hacker-launcher/1.0")
        .build()?;
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Steam search failed with status {}", resp.status());
    }

    #[derive(Deserialize)]
    struct RawItem {
        id: u64,
        name: String,
    }
    #[derive(Deserialize, Default)]
    struct RawResponse {
        #[serde(default)]
        items: Vec<RawItem>,
    }

    let parsed: RawResponse = resp
        .json()
        .await
        .context("Failed to parse Steam search response")?;
    Ok(parsed
        .items
        .into_iter()
        .map(|i| SteamSearchResult {
            app_id: i.id.to_string(),
            name: i.name,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal, valid appinfo.vdf-shaped blob for one app and
    /// confirms `read_appinfo_launch_exe`-equivalent parsing extracts the
    /// launch executable — exercises the exact binary-object parsing logic
    /// used against real Steam caches, without needing one on disk.
    #[test]
    fn parses_synthetic_appinfo_blob() {
        fn obj_start(out: &mut Vec<u8>, key: &str) {
            out.push(0x00);
            out.extend(key.as_bytes());
            out.push(0);
        }
        fn str_kv(out: &mut Vec<u8>, key: &str, val: &str) {
            out.push(0x01);
            out.extend(key.as_bytes());
            out.push(0);
            out.extend(val.as_bytes());
            out.push(0);
        }
        fn end(out: &mut Vec<u8>) {
            out.push(0x08);
        }

        let mut obj = vec![];
        obj_start(&mut obj, "appinfo");
        obj_start(&mut obj, "config");
        obj_start(&mut obj, "launch");
        obj_start(&mut obj, "0");
        str_kv(&mut obj, "executable", "Game.exe");
        str_kv(&mut obj, "type", "default");
        end(&mut obj); // "0"
        end(&mut obj); // launch
        end(&mut obj); // config
        end(&mut obj); // appinfo
        end(&mut obj); // implicit root

        let mut reader = BinReader { data: &obj, pos: 0 };
        let parsed = parse_binary_object(&mut reader).expect("should parse");
        assert_eq!(find_launch_executable(&parsed).as_deref(), Some("Game.exe"));
    }
}
