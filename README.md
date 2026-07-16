# Hacker Launcher

**v1.0** — A game launcher for running Windows games on Linux with Proton, Wine, Steam, Flatpak, and Native runners.

Built with **Rust + Tauri 2** on the backend and **SolidJS + TypeScript** on the frontend.

---

## Features

### Games
- Add games with Native / Wine / Proton / Flatpak / Steam runners
- Per-game DXVK, Esync, Fsync, DXVK-Async overrides (with global defaults in Settings)
- Custom per-game environment variables
- Custom icon/cover art per game, shown in both List and Grid library views
- Tags and favorites, with filtering in the library
- List view or cover-art Grid view (switchable in Settings, takes effect after restart)
- Drag & drop a `.exe` anywhere onto the window to add it as a game
- Live "Running" status with a Stop button, and automatic playtime tracking
- Rotating per-run logs (last 10 runs kept per game) with a log viewer
- Keyboard shortcuts: **Enter** launches/stops the selected game, **Delete** removes it
- Quote-aware launch options parsing (so `--config="C:\Path With Spaces\x.ini"` works correctly)
- Optional shared Wine prefix across multiple games, in addition to the default per-game prefix
- Steam library auto-import: scans default, Flatpak, and additional Steam Library Folder
  locations — including external/removable drives and unregistered `SteamLibrary` folders — and
  lets you review and correct the guessed executable before importing
- ProtonDB compatibility lookup by Steam App ID (cached for 15 minutes per App ID)
- Backup/restore: export games + settings to a JSON file, and import it back later either as a
  full restore or merged in as new games only

### Proton / Wine tooling
- Install Proton-GE, official Valve Proton (stable/experimental), or a custom `tar.gz`/folder
- Paginated GitHub release listing (not just the first ~30), with an in-memory cache to avoid
  hitting GitHub's unauthenticated rate limit
- Changelog preview (the GitHub release body) before installing a version
- Download integrity check against the release's published checksum, when one exists
- Cancellable installs, with live download/extraction progress
- System notification when a background install finishes (or fails)
- **Winetricks integration**: a curated shortlist of common components (VC++, .NET, DirectX,
  DXVK…) for one-click installs, or browse/search the full ~700-entry catalog (read live from
  your installed `winetricks list-all`)
- **Dependency scanner**: looks for bundled redistributable installers (`vcredist`, `dotnetfx`,
  `dxsetup`, …) next to a game's executable and offers to run them directly or via the matching
  Winetricks verb
- Live streaming output for both Winetricks and dependency-installer runs — long operations like
  `dotnet48` show real progress instead of an unresponsive-looking wait
- **Prefix locking**: launching a game, running Winetricks, and running a dependency installer all
  claim an exclusive lock on the Wine prefix they touch. A second attempt against the same prefix
  (e.g. starting a game that shares a prefix with one already running, or running Winetricks while
  that happens) is refused with a clear error instead of risking registry corruption
- Gamescope integration (adaptive-sync, resolution, FPS cap, Big Picture) via the `--gamescope`
  launch option

### Controllers
- Lists gamepads currently visible to the kernel (`/proc/bus/input/devices`)
- **SDL_GAMECONTROLLERCONFIG mapping wizard**: pick a connected controller, press each button/stick
  in turn, and the wizard builds the mapping string for you — reading raw Linux joystick events
  directly, so you don't need to know button/axis numbers by heart. The controller's SDL-style GUID
  is auto-derived from its USB/Bluetooth identity in sysfs. Usable standalone (copies to clipboard)
  or launched straight from a game's Configure dialog (fills the field directly)
- Per-game "Disable Steam Input" toggle and raw `SDL_GAMECONTROLLERCONFIG` override

### UI
- Dark theme (default) and a full Light theme, switchable live in Settings
- Toast notifications, themed confirmation dialogs (no native `window.confirm` popups)

---

## Requirements

### Runtime dependencies
- `wine` — for the Wine runner, dependency-installer runs, and as a fallback for GUID/controller info
- `winetricks` — optional, for the Winetricks integration (both the curated shortlist and the full
  catalog browser shell out to your installed copy)
- `gamescope` — optional, for Gamescope integration
- `steam` or `flatpak` — for the Steam runner and Steam library import
- Proton versions are downloaded and managed by the launcher itself

### Build dependencies

```bash
# Rust (stable)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Node.js >= 18
# (use your distro's package manager or nvm)

# Tauri system dependencies (Ubuntu/Debian)
sudo apt install libwebkit2gtk-4.0-dev build-essential curl wget \
  libssl-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev

# Tauri system dependencies (Fedora/RHEL)
sudo dnf install webkit2gtk3-devel openssl-devel curl wget \
  libappindicator-gtk3-devel librsvg2-devel

# Tauri system dependencies (Arch)
sudo pacman -S webkit2gtk base-devel curl wget openssl libappindicator-gtk3 librsvg
```

## Build & Run

```bash
# Install JS dependencies
npm install

# Development (hot reload)
npm run tauri dev

# Production build
npm run tauri build
# Binary will be in: src-tauri/target/release/hacker-launcher
```

> **Note:** `bundle.active` is currently `false` in `tauri.conf.json`, so `tauri build` produces a
> plain binary rather than a packaged AppImage/`.deb`/`.rpm`. Flip it on and configure the `bundle`
> section if you want installable packages.

## Architecture

```
source-code/
├── src/                        SolidJS + TypeScript frontend
│   ├── App.tsx                 Tab bar, applies saved theme/library view at startup
│   ├── types.ts                Shared TS types + small helpers (emptyGame, formatPlaytime, …)
│   └── components/
│       ├── GamesTab.tsx        Library (list/grid), search/tags/favorites, drag&drop, shortcuts
│       ├── AddGameModal.tsx / ConfigureGameModal.tsx
│       ├── ProtonsTab.tsx      Install/update/remove Proton versions, changelog preview
│       ├── ControllersTab.tsx  Detected gamepads + entry point to the mapping wizard
│       ├── ControllerMappingWizard.tsx
│       ├── SettingsTab.tsx     Theme, defaults, shared prefix, backup/restore
│       └── ConfirmModal.tsx / ToastContainer.tsx
└── src-tauri/src/               Rust backend
    ├── lib.rs                   Tauri commands + app wiring
    ├── config_manager.rs        Settings persistence
    ├── game_manager.rs          Game CRUD, process launch/tracking/playtime, log rotation
    ├── proton_manager.rs        GitHub release listing/cache, install/extract, checksums
    ├── tools.rs                 Steam import, Winetricks, dependency scan, ProtonDB, controllers,
    │                            backup/restore
    └── prefix_lock.rs           Cross-cutting Wine-prefix exclusivity lock
```

## Data locations

All data is stored in `~/.hackeros/Hacker-Launcher/`:

| Path | Purpose |
|------|---------|
| `Config/games.json` | Saved game list |
| `Config/settings.json` | Launcher settings |
| `Protons/` | Installed Proton versions |
| `Prefixes/` | Wine/Proton prefixes (including the optional shared one, under `Prefixes/shared`) |
| `Logs/` | Rotating per-game launch logs |

## Known limitations

Being upfront about what these features actually are, rather than overselling them:

- **Steam library import** and the **dependency scanner** are filename/heuristic-based (largest
  non-utility `.exe` in a folder; known installer filename patterns). They're right most of the
  time, wrong occasionally — the import dialog lets you fix the guessed executable before
  committing, and the dependency scanner can't tell whether something is *already* installed in the
  prefix, only that an installer for it exists on disk.
- **ProtonDB lookup** and **Steam library import** both need network/Steam-client data respectively;
  neither works offline or for games with no corresponding Steam App ID.
- The **controller mapping wizard** reads raw Linux joystick events directly rather than depending
  on SDL2 itself — it's a best-effort helper, and D-Pads reported as a "hat" (rather than four
  separate buttons) may need a manual touch-up to the generated string.
- **Prefix locking** prevents concurrent *launcher-initiated* operations on the same prefix; it
  can't stop something outside the launcher from touching the same prefix at the same time.
- No automated tests or CI yet, and the launcher itself has no auto-update mechanism (only Proton
  versions are update-checked).
