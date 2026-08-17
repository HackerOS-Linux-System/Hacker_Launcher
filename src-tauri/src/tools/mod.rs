mod backup;
mod controllers;
mod dependency_scan;
mod protondb;
pub(crate) mod process_util;
mod steam_import;
mod winetricks;

pub use backup::{export_backup, import_backup};
pub use controllers::{
    capture_controller_input, get_controller_guid, list_controllers, ControllerInfo,
    ControllerInputEvent,
};
pub use dependency_scan::{run_installer_in_prefix, scan_game_dependencies, DependencyHint};
pub use protondb::{check_protondb, ProtonDbInfo};
pub use steam_import::{scan_steam_library, search_steam_appid, SteamGameCandidate, SteamSearchResult};
pub use winetricks::{all_winetricks_verbs, common_winetricks_verbs, run_winetricks};
