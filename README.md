# Hacker Launcher

**v1.0** — A game launcher for running Windows games on Linux with Proton, Wine, Steam, Flatpak, and Native runners — plus Android apps via Waydroid.

Built with **Rust + Tauri 2** on the backend and **SolidJS + TypeScript** on the frontend.

---

## Features

### Games
- Add games with Native / Wine / Proton / Flatpak / Steam / Android runners
- Per-game DXVK, Esync, Fsync, DXVK-Async overrides (with global defaults in Settings)
- Custom per-game environment variables
- Custom icon/cover art per game, shown in both List and Grid library views
- Tags and favorites, with filtering in the library
- List view or cover-art Grid view (switchable in Settings, takes effect after restart)
- Drag & drop a `.exe` or `.apk` anywhere onto the window to add it as a game
- Live "Running" status with a Stop button, and automatic playtime tracking
- Rotating per-run logs (last 10 runs kept per game) with a log viewer
- Keyboard shortcuts: **Enter** launches/stops the selected game, **Delete** removes it
- Quote-aware launch options parsing (so `--config="C:\Path With Spaces\x.ini"` works correctly)
- Optional shared Wine prefix across multiple games, in addition to the default per-game prefix
- Steam library auto-import: scans default, Flatpak, and additional Steam Library Folder
  locations — including external/removable drives and unregistered `SteamLibrary` folders — and
  lets you review and correct the guessed executable before importing. The exe guess uses a
  multi-signal scoring heuristic (name similarity to the game's title, Unreal/Unity "-Shipping"
  naming, install-folder depth, known non-game subfolders), not just "pick the largest file"
- ProtonDB compatibility lookup by Steam App ID (in-memory cache for 15 minutes, plus a
  disk-persisted cache so a lookup that already succeeded once still works offline afterwards).
  A built-in Steam store search lets you find an App ID by name for games with none on record
- Backup/restore: export games + settings to a JSON file, and import it back later either as a
  full restore or merged in as new games only

### Android
- Add an `.apk` as a game (runner "Android"), running under [Waydroid](https://docs.waydro.id)
- Package name auto-detected from the APK via `aapt`/`aapt2` (editable if detection fails)
- One-click install into the Waydroid container with live streaming progress
- Launch/stop by package ID, independent of the `.exe`-based runners' Wine/Proton pipeline

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
  Winetricks verb. Also flags dependencies that already look installed in the target prefix (via
  known marker DLLs/paths), so you're not nudged to reinstall something that's already there
- Live streaming output for both Winetricks and dependency-installer runs — long operations like
  `dotnet48` show real progress instead of an unresponsive-looking wait
- **Prefix locking**: launching a game, running Winetricks, and running a dependency installer all
  claim an exclusive lock on the Wine prefix they touch. A second attempt against the same prefix
  (e.g. starting a game that shares a prefix with one already running, or running Winetricks while
  that happens) is refused with a clear error instead of risking registry corruption — this now
  also detects prefixes already in use by processes *outside* the launcher (e.g. a `wine` command
  run manually in a terminal, or a second launcher instance), not just other launcher-initiated ops
- Gamescope integration (adaptive-sync, resolution, FPS cap, Big Picture) via the `--gamescope`
  launch option

### Controllers
- Lists gamepads currently visible to the kernel (`/proc/bus/input/devices`)
- **SDL_GAMECONTROLLERCONFIG mapping wizard**: pick a connected controller, press each button/stick
  in turn, and the wizard builds the mapping string for you — reading raw Linux joystick events
  directly, so you don't need to know button/axis numbers by heart. D-Pads reported by the kernel
  as a "hat" (rather than four separate buttons) are detected via the same `JSIOCGAXMAP` ioctl
  SDL2/`jstest` use, and encoded as SDL's `hH.MASK` syntax rather than a plain axis. The
  controller's SDL-style GUID is auto-derived from its USB/Bluetooth identity in sysfs. Usable
  standalone (copies to clipboard) or launched straight from a game's Configure dialog (fills the
  field directly)
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
- `waydroid` — for the Android runner (see [docs.waydro.id](https://docs.waydro.id) for setup);
  `aapt` or `aapt2` (from your distro's Android SDK build-tools package) — optional, used to
  auto-detect an APK's package name; without it you can still enter the package ID manually
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
# Packaged AppImage/.deb/.rpm will be in: src-tauri/target/release/bundle/
# (plain binary also at: src-tauri/target/release/hacker-launcher)
```

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
    ├── android_manager.rs       Waydroid: APK package-name extraction, install, launch, running-check, stop
    ├── prefix_lock.rs           Cross-cutting Wine-prefix exclusivity lock
    └── tools/                   Loosely-related features, one submodule per area:
        ├── mod.rs               Re-exports each submodule's public API as `tools::*`
        ├── process_util.rs      Shared "stream a child process's output live" helper
        ├── steam_import.rs      Library scan, appinfo.vdf launch-config read, exe-scoring fallback
        ├── winetricks.rs        Verb catalog + running winetricks against a prefix
        ├── dependency_scan.rs   Bundled-installer detection + already-installed heuristic
        ├── controllers.rs       Controller listing + SDL mapping wizard input capture
        ├── protondb.rs          ProtonDB lookup with in-memory + disk cache
        └── backup.rs            Export/import of games + settings
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

- **Steam library import**'s exe guess now tries Steam's own local launch-config cache first
  (`appcache/appinfo.vdf` — an undocumented, community-reverse-engineered binary format, parsed
  defensively with validation on the result), so it's reading Steam's real answer rather than
  guessing whenever that succeeds. When it doesn't (no local cache entry, or it doesn't parse — e.g.
  a future Steam client revision changes the format), it falls back to the filename/scoring
  heuristic as before. The import dialog tags each game with which one actually happened ("steam
  metadata" vs "heuristic scan") and always lets you correct the path either way before committing.
  **ProtonDB lookup** and **Steam library import** both need network/Steam-client data respectively
  to do a *fresh* lookup — ProtonDB results are now cached to disk so a prior successful lookup is
  still available offline, but a first-time lookup with no connection still fails, and neither works
  for games with no corresponding Steam App ID (the "Find by name" search only helps if the game
  *has* a Steam store entry, just not one on record locally).
- The **dependency scanner**'s "already installed" check combines two signals, both still
  heuristics rather than a real registry/component query (Wine doesn't expose one): winetricks' own
  install log (`<prefix>/winetricks.log`, checked first — a real record of what winetricks
  successfully ran against that exact prefix, not an inference) and, as a fallback, known marker
  DLLs/paths a given component would leave behind. The log-based check only knows about verbs
  installed *via* winetricks; the marker-file check only covers a fixed table of common verbs —
  anything installed some other way and not in that table still always shows as "not confirmed
  installed".
- The **controller mapping wizard** reads raw Linux joystick events directly rather than depending
  on SDL2 itself. D-Pads reported as a "hat" are detected and mapped correctly via the same
  `JSIOCGAXMAP` ioctl SDL2 uses, but this ioctl can fail silently on unusual devices/permissions, in
  which case the wizard falls back to treating every axis as plain analog — it's still a best-effort
  helper overall, so double-check the generated string.
- **Prefix locking** detects Wine prefixes already in use by processes outside the launcher via two
  combined `/proc` checks: a matching `WINEPREFIX` environment variable, and (to also catch
  processes that reach a prefix without ever setting that variable themselves) any open file
  descriptor pointing inside the prefix directory. Both are still limited to processes readable by
  the current user (same user, or root) — a process running as a different user touching the same
  prefix on a shared machine is invisible to either check — and both are inherently racy against a
  process that hasn't opened anything inside the prefix yet at the moment of the check.
- **Android/.apk support** runs entirely inside a separate [Waydroid](https://docs.waydro.id)
  container rather than as a process this launcher directly owns, so it doesn't get quite the same
  guarantees as the Wine/Proton runners. Playtime tracking now polls the container (`waydroid shell
  -- pidof <package>`, roughly every 4 seconds with a 2-strike debounce) to detect the app's actual
  exit rather than only reacting to the Stop button, so a game closed from inside Android itself
  still gets recorded correctly — but it's still polling-based (exit is noticed with a delay of up
  to ~8 seconds, not instantly) and, like `pidof` generally, assumes the app's main process is named
  after its package ID, which can miss/misreport for apps that spawn background services under a
  different name. Stop also calls `waydroid app stop`, which isn't supported on every Waydroid
  version. Package-name auto-detection needs `aapt`/`aapt2` installed separately (Waydroid itself
  doesn't provide it) — without it, the package ID can still be entered by hand.
- No automated tests or CI pipeline yet — some of the trickier logic above (the binary VDF parsing,
  the winetricks-log check) does have unit tests alongside it in the relevant module now, but
  there's no CI running them automatically and no coverage of the Tauri command layer or frontend.
  The launcher itself also has no auto-update mechanism (only Proton versions are update-checked).
