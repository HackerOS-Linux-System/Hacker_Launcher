export interface Game {
  name: string;
  exe: string;
  runner: string;
  prefix: string;
  launch_options: string;
  fps_limit: number | null;
  enable_dxvk: boolean;
  enable_esync: boolean;
  enable_fsync: boolean;
  enable_dxvk_async: boolean;
  app_id: string;
  env_vars: string;
  icon_path: string;
  total_playtime_secs: number;
  last_played: string | null;
  tags: string;
  favorite: boolean;
  use_shared_prefix: boolean;
  disable_steam_input: boolean;
  sdl_controller_config: string;
}

export interface Settings {
  fullscreen: boolean;
  default_runner: string;
  auto_update: string;
  enable_esync: boolean;
  enable_fsync: boolean;
  enable_dxvk_async: boolean;
  theme: string;
  use_shared_prefix_default: boolean;
  shared_prefix_path: string;
  library_view: "List" | "Grid";
}

export interface ProtonEntry {
  version: string;
  type: string;
  date: string;
  status: string;
}

export interface Paths {
  prefixes_dir: string;
  protons_dir: string;
  logs_dir: string;
}

export interface Toast {
  id: number;
  message: string;
  kind: "success" | "error" | "info";
}

export interface RunningGameInfo {
  name: string;
  pid: number;
  started_at: string;
}

export interface GameLogEntry {
  file_name: string;
  path: string;
  modified: string;
}

/** Sensible defaults for a brand-new Game record, so components never have
 * to sprinkle `?? ""` / `?? false` everywhere. */
export function emptyGame(overrides: Partial<Game> = {}): Game {
  return {
    name: "",
    exe: "",
    runner: "Proton",
    prefix: "",
    launch_options: "",
    fps_limit: null,
    enable_dxvk: false,
    enable_esync: false,
    enable_fsync: false,
    enable_dxvk_async: false,
    app_id: "",
    env_vars: "",
    icon_path: "",
    total_playtime_secs: 0,
    last_played: null,
    tags: "",
    favorite: false,
    use_shared_prefix: false,
    disable_steam_input: false,
    sdl_controller_config: "",
    ...overrides,
  };
}

/** Formats a playtime duration in seconds as e.g. "3h 24m" / "42m" / "—". */
export function formatPlaytime(totalSecs: number): string {
  if (!totalSecs || totalSecs <= 0) return "—";
  const hours = Math.floor(totalSecs / 3600);
  const minutes = Math.floor((totalSecs % 3600) / 60);
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m`;
  return "<1m";
}

export interface ProtonDbInfo {
  tier: string;
  trending_tier: string;
  confidence: string;
}

export interface DependencyHint {
  label: string;
  path: string;
  winetricks_verb: string | null;
}

export interface ControllerInfo {
  name: string;
  handler: string;
}

export interface ControllerInputEvent {
  kind: "button" | "axis";
  number: number;
  value: number;
}

export interface SteamGameCandidate {
  name: string;
  app_id: string;
  exe_path: string;
  install_dir: string;
}

/** Tags stored as a single comma-separated string; helpers keep the split
 * logic in one place. */
export function parseTags(tags: string): string[] {
  return tags
    .split(",")
    .map((t) => t.trim())
    .filter((t) => t.length > 0);
}
