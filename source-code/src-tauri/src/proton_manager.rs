use anyhow::{bail, Context, Result};
use chrono::{DateTime, Local};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
// Tauri v2: emit via AppHandle, not Window
use tauri::Emitter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtonEntry {
    pub version: String,
    pub r#type: String,
    pub date: String,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
    #[serde(default)]
    body: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

const RELEASE_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const GE_REPO: &str = "GloriousEggroll/proton-ge-custom";
const OFFICIAL_REPO: &str = "ValveSoftware/Proton";

/// Cache of full release lists, keyed by "owner/repo". This is what actually
/// saves API calls: listing GE, listing Official (stable + experimental),
/// checking for updates, and starting an install all used to hit
/// `GET /releases` separately, and GitHub's unauthenticated limit is only
/// 60 requests/hour — a handful of clicks around the Protons tab could burn
/// through that in minutes.
struct ReleaseCache {
    releases: Vec<GithubRelease>,
    fetched_at: Instant,
}

fn release_cache() -> &'static Mutex<HashMap<String, ReleaseCache>> {
    static CACHE: OnceLock<Mutex<HashMap<String, ReleaseCache>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// In-flight cancellation flags for Proton installs, keyed by version. The
/// UI can only have one install running at a time, so the version string is
/// a sufficiently unique key.
fn cancel_flags() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static FLAGS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    FLAGS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_cancel_token(key: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    cancel_flags()
        .lock()
        .unwrap()
        .insert(key.to_string(), flag.clone());
    flag
}

fn clear_cancel_token(key: &str) {
    cancel_flags().lock().unwrap().remove(key);
}

/// Called from the `cancel_proton_install` Tauri command.
pub fn cancel_install(version: &str) -> bool {
    if let Some(flag) = cancel_flags().lock().unwrap().get(version) {
        flag.store(true, Ordering::SeqCst);
        true
    } else {
        false
    }
}

pub struct ProtonManager {
    pub protons_dir: PathBuf,
}

impl ProtonManager {
    pub fn new(protons_dir: PathBuf) -> Self {
        fs::create_dir_all(&protons_dir).ok();
        Self { protons_dir }
    }

    pub fn get_installed_protons(&self) -> Result<Vec<ProtonEntry>> {
        let mut protons = vec![];
        if !self.protons_dir.exists() {
            return Ok(protons);
        }
        for entry in fs::read_dir(&self.protons_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let version = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let proton_type = if version.starts_with("GE-Proton") {
                    "GE"
                } else {
                    "Official"
                }
                .to_string();
                let metadata = fs::metadata(&path)?;
                let created: DateTime<Local> = metadata
                    .created()
                    .unwrap_or(std::time::SystemTime::now())
                    .into();
                let date = created.format("%Y-%m-%d").to_string();
                protons.push(ProtonEntry {
                    version,
                    r#type: proton_type,
                    date,
                    status: "Installed".to_string(),
                });
            }
        }
        protons.sort_by(|a, b| b.version.cmp(&a.version));
        Ok(protons)
    }

    fn get_proton_binary(base: &PathBuf) -> Option<PathBuf> {
        for entry in walkdir::WalkDir::new(base).max_depth(5) {
            if let Ok(e) = entry {
                if e.file_name() == "proton" && e.file_type().is_file() {
                    return Some(e.path().to_path_buf());
                }
            }
        }
        None
    }

    /// Fetches every release page for `repo`, following GitHub's pagination
    /// (`per_page=100`) instead of only reading the implicit first page of
    /// ~30 results. Without this, older Proton/GE-Proton versions further
    /// back in release history were simply invisible in the install dialog.
    /// Results are cached for `RELEASE_CACHE_TTL` to keep the rate-limit
    /// footprint down.
    async fn fetch_all_releases(repo: &str) -> Result<Vec<GithubRelease>> {
        if let Some(cached) = release_cache().lock().unwrap().get(repo) {
            if cached.fetched_at.elapsed() < RELEASE_CACHE_TTL {
                return Ok(cached.releases.clone());
            }
        }

        let client = reqwest::Client::builder()
            .user_agent("hacker-launcher/0.10")
            .build()?;

        let mut all: Vec<GithubRelease> = vec![];
        // Cap at 10 pages (up to 1000 releases) as a sane upper bound; no
        // Proton fork has anywhere near that many releases today, but this
        // avoids an unbounded loop if GitHub ever behaves unexpectedly.
        for page in 1..=10u32 {
            let url = format!(
                "https://api.github.com/repos/{}/releases?per_page=100&page={}",
                repo, page
            );
            let resp = client.get(&url).send().await?;
            if !resp.status().is_success() {
                if all.is_empty() {
                    bail!("GitHub API request failed with status {}", resp.status());
                }
                break;
            }
            let page_releases: Vec<GithubRelease> = resp.json().await.unwrap_or_default();
            let got = page_releases.len();
            all.extend(page_releases);
            if got < 100 {
                break;
            }
        }

        release_cache().lock().unwrap().insert(
            repo.to_string(),
            ReleaseCache {
                releases: all.clone(),
                fetched_at: Instant::now(),
            },
        );

        Ok(all)
    }

    pub async fn fetch_available_ge() -> Result<Vec<String>> {
        let releases = Self::fetch_all_releases(GE_REPO).await?;
        let mut tags: Vec<String> = releases
            .into_iter()
            .filter(|r| r.tag_name.starts_with("GE-Proton"))
            .map(|r| r.tag_name)
            .collect();
        tags.sort_by(|a, b| version_sort_key(b).cmp(&version_sort_key(a)));
        Ok(tags)
    }

    pub async fn fetch_available_official(stable: bool) -> Result<Vec<String>> {
        let releases = Self::fetch_all_releases(OFFICIAL_REPO).await?;
        let mut tags: Vec<String> = releases
            .into_iter()
            .filter(|r| {
                let lower = r.tag_name.to_lowercase();
                if stable {
                    !lower.contains("experimental") && !lower.contains("hotfix")
                } else {
                    lower.contains("experimental") || lower.contains("hotfix")
                }
            })
            .map(|r| r.tag_name)
            .collect();
        tags.sort_by(|a, b| version_sort_key(b).cmp(&version_sort_key(a)));
        Ok(tags)
    }

    /// Returns the raw (markdown) release-notes body GitHub stores for a
    /// given tag, so the install dialog can show what changed before the
    /// user commits to downloading it.
    pub async fn fetch_release_notes(version: String, proton_type: String) -> Result<String> {
        let repo = if proton_type == "GE" { GE_REPO } else { OFFICIAL_REPO };
        let releases = Self::fetch_all_releases(repo).await?;
        let release = releases
            .into_iter()
            .find(|r| r.tag_name == version)
            .with_context(|| format!("No release found for {}", version))?;
        Ok(release
            .body
            .unwrap_or_else(|| "(No release notes provided)".to_string()))
    }

    pub async fn check_update_async(
        version: String,
        proton_type: String,
    ) -> Result<Option<(String, String)>> {
        match proton_type.as_str() {
            "GE" => {
                let available = Self::fetch_available_ge().await?;
                if let Some(latest) = available.first() {
                    if latest != &version {
                        return Ok(Some(("GE".to_string(), latest.clone())));
                    }
                }
            }
            "Official" => {
                let stable = Self::fetch_available_official(true).await?;
                let exp = Self::fetch_available_official(false).await?;
                let mut all = stable.clone();
                all.extend(exp.clone());
                all.sort_by(|a, b| version_sort_key(b).cmp(&version_sort_key(a)));
                if let Some(latest) = all.first() {
                    if latest != &version {
                        let t = if stable.contains(latest) {
                            "Official"
                        } else {
                            "Experimental"
                        };
                        return Ok(Some((t.to_string(), latest.clone())));
                    }
                }
            }
            _ => {}
        }
        Ok(None)
    }

    /// Looks for a checksum asset (`<name>.sha512sum` / `.sha256sum`, or a
    /// generic `sha512sums.txt`-style manifest containing the asset name)
    /// alongside the given release asset.
    fn find_checksum_asset<'a>(
        assets: &'a [GithubAsset],
        asset_name: &str,
    ) -> Option<(&'a GithubAsset, ChecksumAlgo)> {
        assets.iter().find_map(|a| {
            if a.name == format!("{}.sha512sum", asset_name) {
                Some((a, ChecksumAlgo::Sha512))
            } else if a.name == format!("{}.sha256sum", asset_name) {
                Some((a, ChecksumAlgo::Sha256))
            } else {
                None
            }
        })
    }

    async fn verify_download_checksum(
        client: &reqwest::Client,
        checksum_url: &str,
        algo: ChecksumAlgo,
        asset_name: &str,
        downloaded_path: &PathBuf,
        app_handle: &tauri::AppHandle,
    ) -> Result<()> {
        emit_progress(app_handle, "Verifying checksum", 0, 100);
        let text = client.get(checksum_url).send().await?.text().await?;
        // Typical format: "<hex>  <filename>" per line (sha256sum/sha512sum
        // style), but some releases just publish the bare hex digest.
        let single_line = text.lines().filter(|l| !l.trim().is_empty()).count() == 1;
        let expected = text
            .lines()
            .find_map(|line| {
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }
                if single_line || line.contains(asset_name) {
                    line.split_whitespace().next().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .with_context(|| "Checksum file was empty or unparseable")?;

        let actual = hash_file(downloaded_path, algo)?;
        if !actual.eq_ignore_ascii_case(expected.trim()) {
            bail!(
                "Checksum verification failed for {} (expected {}, got {})",
                asset_name,
                expected,
                actual
            );
        }
        emit_progress(app_handle, "Verifying checksum", 100, 100);
        Ok(())
    }

    // Tauri v2: accept AppHandle instead of Window
    pub async fn install_proton_async(
        protons_dir: PathBuf,
        version: String,
        proton_type: String,
        app_handle: tauri::AppHandle,
    ) -> Result<()> {
        let repo = if proton_type == "GE" { GE_REPO } else { OFFICIAL_REPO };

        let cancel_flag = register_cancel_token(&version);
        let cleanup_token = |version: &str| clear_cancel_token(version);

        let result = Self::install_proton_inner(
            protons_dir,
            version.clone(),
            repo,
            app_handle,
            cancel_flag,
        )
        .await;

        cleanup_token(&version);
        result
    }

    async fn install_proton_inner(
        protons_dir: PathBuf,
        version: String,
        repo: &str,
        app_handle: tauri::AppHandle,
        cancel_flag: Arc<AtomicBool>,
    ) -> Result<()> {
        let client = reqwest::Client::builder()
            .user_agent("hacker-launcher/0.10")
            .build()?;

        let releases = Self::fetch_all_releases(repo).await?;
        let release = releases
            .into_iter()
            .find(|r| r.tag_name == version)
            .with_context(|| format!("No release found for {}", version))?;

        let asset = release
            .assets
            .iter()
            .find(|a| a.name.ends_with(".tar.gz"))
            .with_context(|| format!("No tar.gz asset found for {}", version))?
            .clone();

        let checksum_info = Self::find_checksum_asset(&release.assets, &asset.name)
            .map(|(a, algo)| (a.browser_download_url.clone(), algo));

        emit_progress(&app_handle, "Downloading", 0, 100);

        let response = client.get(&asset.browser_download_url).send().await?;
        let total_size = response.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;

        let tmp_path = protons_dir.join(format!("{}.tmp.tar.gz", version));
        {
            let mut file = fs::File::create(&tmp_path)?;
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                if cancel_flag.load(Ordering::SeqCst) {
                    drop(file);
                    fs::remove_file(&tmp_path).ok();
                    emit_progress(&app_handle, "Cancelled", 0, 100);
                    bail!("Installation cancelled by user");
                }
                let chunk = chunk?;
                file.write_all(&chunk)?;
                downloaded += chunk.len() as u64;
                if total_size > 0 {
                    emit_progress(&app_handle, "Downloading", downloaded, total_size);
                }
            }
        }

        if cancel_flag.load(Ordering::SeqCst) {
            fs::remove_file(&tmp_path).ok();
            emit_progress(&app_handle, "Cancelled", 0, 100);
            bail!("Installation cancelled by user");
        }

        // Verify integrity if the release publishes a checksum for this
        // asset. Official Proton releases generally do not, in which case
        // we skip verification rather than fail the install outright — but
        // GE-Proton releases do, and previously nothing checked them, so a
        // corrupted or tampered download would silently extract and run.
        if let Some((checksum_url, algo)) = checksum_info {
            if let Err(e) = Self::verify_download_checksum(
                &client,
                &checksum_url,
                algo,
                &asset.name,
                &tmp_path,
                &app_handle,
            )
            .await
            {
                fs::remove_file(&tmp_path).ok();
                return Err(e);
            }
        } else {
            emit_progress(&app_handle, "No checksum published, skipping verification", 100, 100);
        }

        emit_progress(&app_handle, "Extracting", 0, 100);
        let extract_dir = protons_dir.join(&version);
        fs::create_dir_all(&extract_dir)?;

        let tmp_path_clone = tmp_path.clone();
        let extract_dir_clone = extract_dir.clone();
        let app_handle_clone = app_handle.clone();
        let cancel_flag_extract = cancel_flag.clone();

        let extract_result = tokio::task::spawn_blocking(move || -> Result<()> {
            let file = fs::File::open(&tmp_path_clone)?;
            let gz = flate2::read::GzDecoder::new(file);
            let mut archive = tar::Archive::new(gz);
            let total_size = fs::metadata(&tmp_path_clone)?.len();
            let mut extracted: u64 = 0;

            for entry in archive.entries()? {
                if cancel_flag_extract.load(Ordering::SeqCst) {
                    bail!("Installation cancelled by user");
                }
                let mut entry = entry?;
                let path = entry.path()?.to_path_buf();
                // Security: strip absolute paths
                let stripped = path.components().skip(1).collect::<std::path::PathBuf>();
                let dest = extract_dir_clone.join(&stripped);
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).ok();
                }
                entry.unpack(&dest).ok();
                extracted += entry.size();
                if total_size > 0 {
                    emit_progress(&app_handle_clone, "Extracting", extracted, total_size);
                }
            }
            Ok(())
        })
        .await?;

        fs::remove_file(&tmp_path).ok();

        if let Err(e) = extract_result {
            fs::remove_dir_all(&extract_dir).ok();
            return Err(e);
        }

        if cancel_flag.load(Ordering::SeqCst) {
            fs::remove_dir_all(&extract_dir).ok();
            emit_progress(&app_handle, "Cancelled", 0, 100);
            bail!("Installation cancelled by user");
        }

        if Self::get_proton_binary(&extract_dir).is_none() {
            fs::remove_dir_all(&extract_dir).ok();
            bail!("Proton binary not found after extraction for {}", version);
        }

        emit_progress(&app_handle, "Done", 100, 100);
        Ok(())
    }

    pub async fn install_custom_tar_async(
        protons_dir: PathBuf,
        tar_path: String,
        version: String,
        app_handle: tauri::AppHandle,
    ) -> Result<()> {
        let extract_dir = protons_dir.join(&version);
        fs::create_dir_all(&extract_dir)?;

        emit_progress(&app_handle, "Extracting", 0, 100);

        let cancel_flag = register_cancel_token(&version);

        let tar_path = PathBuf::from(tar_path);
        let extract_dir_clone = extract_dir.clone();
        let app_handle_clone = app_handle.clone();
        let cancel_flag_extract = cancel_flag.clone();

        let extract_result = tokio::task::spawn_blocking(move || -> Result<()> {
            let file = fs::File::open(&tar_path)?;
            let gz = flate2::read::GzDecoder::new(file);
            let mut archive = tar::Archive::new(gz);
            let total_size = fs::metadata(&tar_path)?.len();
            let mut extracted: u64 = 0;

            for entry in archive.entries()? {
                if cancel_flag_extract.load(Ordering::SeqCst) {
                    bail!("Installation cancelled by user");
                }
                let mut entry = entry?;
                let path = entry.path()?.to_path_buf();
                let stripped = path.components().skip(1).collect::<std::path::PathBuf>();
                let dest = extract_dir_clone.join(&stripped);
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).ok();
                }
                entry.unpack(&dest).ok();
                extracted += entry.size();
                if total_size > 0 {
                    emit_progress(&app_handle_clone, "Extracting", extracted, total_size);
                }
            }
            Ok(())
        })
        .await?;

        clear_cancel_token(&version);

        if let Err(e) = extract_result {
            fs::remove_dir_all(&extract_dir).ok();
            return Err(e);
        }

        if Self::get_proton_binary(&extract_dir).is_none() {
            fs::remove_dir_all(&extract_dir).ok();
            bail!("Proton binary not found after extraction for {}", version);
        }

        emit_progress(&app_handle, "Done", 100, 100);
        Ok(())
    }

    pub fn install_custom_folder(&self, src: &str, version: &str) -> Result<()> {
        let dest = self.protons_dir.join(version);
        copy_dir_all(std::path::Path::new(src), &dest)?;
        if Self::get_proton_binary(&dest).is_none() {
            fs::remove_dir_all(&dest).ok();
            bail!("Proton binary not found in folder for {}", version);
        }
        Ok(())
    }

    pub fn remove_proton(&self, version: &str) -> Result<()> {
        let path = self.protons_dir.join(version);
        if !path.exists() {
            bail!("Proton version not found: {}", version);
        }
        fs::remove_dir_all(path)?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum ChecksumAlgo {
    Sha256,
    Sha512,
}

fn hash_file(path: &PathBuf, algo: ChecksumAlgo) -> Result<String> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut buffer = [0u8; 65536];

    let hex = match algo {
        ChecksumAlgo::Sha256 => {
            let mut hasher = Sha256::new();
            loop {
                let n = reader.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
            }
            to_hex(&hasher.finalize())
        }
        ChecksumAlgo::Sha512 => {
            let mut hasher = Sha512::new();
            loop {
                let n = reader.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
            }
            to_hex(&hasher.finalize())
        }
    };
    Ok(hex)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// Tauri v2: AppHandle::emit (global broadcast), requires tauri::Emitter trait in scope
fn emit_progress(app: &tauri::AppHandle, stage: &str, value: u64, total: u64) {
    let _ = app.emit(
        "proton_progress",
        serde_json::json!({
            "stage": stage,
            "value": value,
            "total": total
        }),
    );
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn version_sort_key(v: &str) -> Vec<i64> {
    v.chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>()
        .split('.')
        .filter_map(|p| p.parse::<i64>().ok())
        .collect()
}
