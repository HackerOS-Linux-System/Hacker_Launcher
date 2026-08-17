use crate::config_manager::Settings;
use crate::game_manager::Game;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

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
