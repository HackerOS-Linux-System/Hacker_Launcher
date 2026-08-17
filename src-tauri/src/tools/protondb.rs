use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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

/// Reads the disk-persisted mirror of successful ProtonDB lookups. This
/// exists so a lookup that already succeeded once is still available the
/// next time the launcher starts *without* a network connection — the
/// in-memory cache above is wiped on every restart, and ProtonDB itself
/// simply doesn't work offline on its own.
fn load_protondb_disk_cache(cache_file: &Path) -> HashMap<String, Option<ProtonDbInfo>> {
    fs::read_to_string(cache_file)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default()
}

fn save_protondb_disk_cache(cache_file: &Path, cache: &HashMap<String, Option<ProtonDbInfo>>) {
    if let Ok(content) = serde_json::to_string_pretty(cache) {
        let _ = fs::write(cache_file, content);
    }
}

/// Looks up a Steam App ID on ProtonDB's public summary API. Only
/// meaningful for games that correspond to a real Steam store entry — for
/// arbitrary non-Steam executables there's nothing to look up, so an empty
/// App ID simply returns `None` rather than an error. Results are cached
/// in memory for 15 minutes per App ID so re-checking the same game
/// repeatedly (e.g. re-opening its Configure dialog) doesn't hammer
/// ProtonDB and risk throttling, and are also persisted to `cache_file` so
/// a lookup that already succeeded once still works offline afterwards —
/// if the request itself fails (no network, ProtonDB unreachable), this
/// falls back to whatever was last seen on disk for that App ID instead of
/// erroring out every single time there's no connection.
pub async fn check_protondb(app_id: &str, cache_file: &Path) -> Result<Option<ProtonDbInfo>> {
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
    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            let disk = load_protondb_disk_cache(cache_file);
            if let Some(info) = disk.get(app_id) {
                return Ok(info.clone());
            }
            return Err(e).context("ProtonDB request failed (offline, or ProtonDB is unreachable)");
        }
    };

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
    let mut disk = load_protondb_disk_cache(cache_file);
    disk.insert(app_id.to_string(), result.clone());
    save_protondb_disk_cache(cache_file, &disk);

    Ok(result)
}
