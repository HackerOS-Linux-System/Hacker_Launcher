import { createSignal, createMemo, onMount, onCleanup, For, Show } from "solid-js";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Game,
  Toast,
  RunningGameInfo,
  GameLogEntry,
  SteamGameCandidate,
  formatPlaytime,
  parseTags,
} from "../types";
import AddGameModal from "./AddGameModal";
import ConfigureGameModal from "./ConfigureGameModal";
import ConfirmModal, { ConfirmRequest } from "./ConfirmModal";

interface Props {
  addToast: (msg: string, kind?: Toast["kind"]) => void;
  libraryView: "List" | "Grid";
}

type SortKey = "name" | "runner" | "playtime";

export default function GamesTab(props: Props) {
  const [games, setGames] = createSignal<Game[]>([]);
  const [selected, setSelected] = createSignal<number>(-1);
  const [showAdd, setShowAdd] = createSignal(false);
  const [addPrefill, setAddPrefill] = createSignal<{ exe?: string; name?: string } | null>(null);
  const [showConfigure, setShowConfigure] = createSignal(false);
  const [launching, setLaunching] = createSignal(false);
  const [confirmReq, setConfirmReq] = createSignal<ConfirmRequest | null>(null);

  const [search, setSearch] = createSignal("");
  const [tagFilter, setTagFilter] = createSignal("");
  const [favoritesOnly, setFavoritesOnly] = createSignal(false);
  const [sortKey, setSortKey] = createSignal<SortKey>("name");
  const [sortAsc, setSortAsc] = createSignal(true);

  const [running, setRunning] = createSignal<Record<string, RunningGameInfo>>({});
  const [, setTick] = createSignal(0); // forces re-render every second for live elapsed time

  const [showLogs, setShowLogs] = createSignal(false);
  const [logEntries, setLogEntries] = createSignal<GameLogEntry[]>([]);
  const [logContent, setLogContent] = createSignal("");
  const [logGameName, setLogGameName] = createSignal("");

  const [dragActive, setDragActive] = createSignal(false);

  const [showSteamImport, setShowSteamImport] = createSignal(false);
  const [steamCandidates, setSteamCandidates] = createSignal<SteamGameCandidate[] | null>(null);
  const [scanningSteam, setScanningSteam] = createSignal(false);
  const [selectedSteamIds, setSelectedSteamIds] = createSignal<Set<string>>(new Set());
  const [editedExePaths, setEditedExePaths] = createSignal<Record<string, string>>({});
  const [importingSteam, setImportingSteam] = createSignal(false);

  const anyModalOpen = () =>
    showAdd() || showConfigure() || showLogs() || showSteamImport() || confirmReq() !== null;

  const loadGames = async () => {
    try {
      const g = await invoke<Game[]>("get_games");
      setGames(g);
    } catch (e) {
      props.addToast(`Failed to load games: ${e}`, "error");
    }
  };

  const loadRunning = async () => {
    try {
      const r = await invoke<RunningGameInfo[]>("get_running_games");
      const map: Record<string, RunningGameInfo> = {};
      for (const entry of r) map[entry.name] = entry;
      setRunning(map);
    } catch {
      /* non-fatal */
    }
  };

  onMount(() => {
    loadGames();
    loadRunning();

    const interval = setInterval(() => setTick((t) => t + 1), 1000);
    onCleanup(() => clearInterval(interval));

    const unlistenStart = listen<{ name: string }>("game_started", async () => {
      await loadRunning();
    });
    const unlistenStop = listen<{ name: string }>("game_stopped", async () => {
      await loadRunning();
      await loadGames();
    });
    onCleanup(() => {
      unlistenStart.then((f) => f());
      unlistenStop.then((f) => f());
    });

    // Keyboard shortcuts: Enter = launch/stop selected game, Delete = remove
    // selected game. Ignored while typing in a field or while any modal is
    // open, so it never fights with normal form editing.
    const onKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      const tag = target?.tagName;
      const isTyping = tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target?.isContentEditable;
      if (isTyping || anyModalOpen()) return;
      if (!selectedGame()) return;

      if (e.key === "Enter") {
        e.preventDefault();
        if (isRunning(selectedGame()!.name)) handleStop();
        else handleLaunch();
      } else if (e.key === "Delete") {
        e.preventDefault();
        handleRemove();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    onCleanup(() => window.removeEventListener("keydown", onKeyDown));

    // Drag & drop: dropping a .exe anywhere on the window opens Add Game
    // prefilled with that path, instead of requiring the Browse button.
    const unlistenDrop = getCurrentWebview().onDragDropEvent((event) => {
      const payload = event.payload;
      if (payload.type === "enter" || payload.type === "over") {
        setDragActive(true);
      } else if (payload.type === "leave") {
        setDragActive(false);
      } else if (payload.type === "drop") {
        setDragActive(false);
        if (anyModalOpen()) return;
        const exePath = payload.paths.find((p) => p.toLowerCase().endsWith(".exe"));
        if (exePath) {
          const fileName = exePath.split(/[\\/]/).pop() ?? "";
          const guessedName = fileName.replace(/\.exe$/i, "").replace(/[_-]+/g, " ").trim();
          setAddPrefill({ exe: exePath, name: guessedName });
          setShowAdd(true);
        } else {
          props.addToast("Drop a Windows .exe file to add it as a game", "info");
        }
      }
    });
    onCleanup(() => { unlistenDrop.then((f) => f()); });
  });

  const allTags = createMemo(() => {
    const set = new Set<string>();
    for (const g of games()) for (const t of parseTags(g.tags)) set.add(t);
    return Array.from(set).sort();
  });

  const filteredSorted = createMemo(() => {
    const q = search().trim().toLowerCase();
    let list = games();
    if (q) {
      list = list.filter(
        (g) => g.name.toLowerCase().includes(q) || g.runner.toLowerCase().includes(q)
      );
    }
    if (tagFilter()) {
      list = list.filter((g) => parseTags(g.tags).includes(tagFilter()));
    }
    if (favoritesOnly()) {
      list = list.filter((g) => g.favorite);
    }
    const key = sortKey();
    const dir = sortAsc() ? 1 : -1;
    return [...list].sort((a, b) => {
      if (a.favorite !== b.favorite) return a.favorite ? -1 : 1;
      if (key === "playtime") return (a.total_playtime_secs - b.total_playtime_secs) * dir;
      const av = (key === "name" ? a.name : a.runner).toLowerCase();
      const bv = (key === "name" ? b.name : b.runner).toLowerCase();
      return av.localeCompare(bv) * dir;
    });
  });

  const toggleSort = (key: SortKey) => {
    if (sortKey() === key) setSortAsc(!sortAsc());
    else {
      setSortKey(key);
      setSortAsc(true);
    }
  };

  const sortIndicator = (key: SortKey) => (sortKey() === key ? (sortAsc() ? " ▲" : " ▼") : "");

  const selectedGame = () => {
    const list = filteredSorted();
    const i = selected();
    return i >= 0 && i < list.length ? list[i] : null;
  };

  const isRunning = (name: string) => name in running();

  const handleLaunch = async () => {
    const game = selectedGame();
    if (!game) return props.addToast("No game selected", "error");
    const gamescope = game.launch_options.includes("--gamescope");
    setLaunching(true);
    try {
      await invoke("launch_game", { game, gamescope });
      props.addToast(`Launched: ${game.name}`, "success");
      await loadRunning();
    } catch (e) {
      props.addToast(`Failed to launch ${game.name}: ${e}`, "error");
    } finally {
      setLaunching(false);
      await loadGames();
    }
  };

  const handleStop = async () => {
    const game = selectedGame();
    if (!game) return;
    try {
      await invoke("stop_game", { name: game.name });
      props.addToast(`Stopped: ${game.name}`, "info");
      await loadRunning();
    } catch (e) {
      props.addToast(`Failed to stop ${game.name}: ${e}`, "error");
    }
  };

  const handleRemove = () => {
    const game = selectedGame();
    if (!game) return props.addToast("No game selected", "error");
    setConfirmReq({
      title: "Remove Game",
      message: `Remove "${game.name}"? This won't delete its Wine prefix or logs.`,
      confirmLabel: "Remove",
      danger: true,
      onConfirm: async () => {
        try {
          await invoke("remove_game", { name: game.name });
          setSelected(-1);
          props.addToast(`Removed: ${game.name}`, "success");
          await loadGames();
        } catch (e) {
          props.addToast(`Failed to remove: ${e}`, "error");
        }
      },
    });
  };

  const toggleFavorite = async (g: Game, e?: MouseEvent) => {
    e?.stopPropagation();
    try {
      await invoke("update_game", { game: { ...g, favorite: !g.favorite }, originalName: g.name });
      await loadGames();
    } catch (err) {
      props.addToast(`Failed to update: ${err}`, "error");
    }
  };

  const openLogs = async () => {
    const game = selectedGame();
    if (!game) return props.addToast("No game selected", "error");
    setLogGameName(game.name);
    setLogContent("");
    try {
      const entries = await invoke<GameLogEntry[]>("list_game_logs", { name: game.name });
      setLogEntries(entries);
      if (entries.length > 0) await viewLog(entries[0]);
      setShowLogs(true);
    } catch (e) {
      props.addToast(`Failed to list logs: ${e}`, "error");
    }
  };

  const viewLog = async (entry: GameLogEntry) => {
    try {
      const content = await invoke<string>("read_game_log", { path: entry.path });
      setLogContent(content || "(empty log)");
    } catch (e) {
      setLogContent(`Failed to read log: ${e}`);
    }
  };

  // ── Steam library import ────────────────────
  const openSteamImport = async () => {
    setShowSteamImport(true);
    setScanningSteam(true);
    setSteamCandidates(null);
    setSelectedSteamIds(new Set<string>());
    setEditedExePaths({});
    try {
      const candidates = await invoke<SteamGameCandidate[]>("scan_steam_library");
      const existingIds = new Set(games().map((g) => g.app_id).filter(Boolean));
      const fresh = candidates.filter((c) => !existingIds.has(c.app_id));
      setSteamCandidates(fresh);
      // Pre-fill the editable exe field with the guessed path so the user
      // only has to touch it when the heuristic picked the wrong file.
      const initial: Record<string, string> = {};
      for (const c of fresh) initial[c.app_id] = c.exe_path;
      setEditedExePaths(initial);
    } catch (e) {
      props.addToast(`Steam scan failed: ${e}`, "error");
      setSteamCandidates([]);
    } finally {
      setScanningSteam(false);
    }
  };

  const browseSteamExe = async (appId: string) => {
    const result = await open({
      filters: [
        { name: "Executables", extensions: ["exe", "bat"] },
        { name: "All Files", extensions: ["*"] },
      ],
    });
    if (typeof result === "string") {
      setEditedExePaths((m) => ({ ...m, [appId]: result }));
    }
  };

  const toggleSteamCandidate = (id: string) => {
    setSelectedSteamIds((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const importSelectedSteamGames = async () => {
    const candidates = (steamCandidates() ?? []).filter((c) => selectedSteamIds().has(c.app_id));
    if (candidates.length === 0) return props.addToast("Select at least one game", "error");
    setImportingSteam(true);
    let imported = 0;
    for (const c of candidates) {
      try {
        await invoke("add_game", {
          game: {
            name: c.name,
            exe: editedExePaths()[c.app_id]?.trim() || c.exe_path,
            runner: "Steam",
            prefix: "",
            launch_options: "",
            fps_limit: null,
            enable_dxvk: false,
            enable_esync: false,
            enable_fsync: false,
            enable_dxvk_async: false,
            app_id: c.app_id,
            env_vars: "",
            icon_path: "",
            total_playtime_secs: 0,
            last_played: null,
            tags: "",
            favorite: false,
            use_shared_prefix: false,
            disable_steam_input: false,
            sdl_controller_config: "",
          } as Game,
        });
        imported++;
      } catch (e) {
        props.addToast(`Failed to import ${c.name}: ${e}`, "error");
      }
    }
    setImportingSteam(false);
    setShowSteamImport(false);
    await loadGames();
    if (imported > 0) props.addToast(`Imported ${imported} game(s) from Steam`, "success");
  };

  return (
    <>
      <Show when={dragActive()}>
        <div class="dropzone-overlay"><span>📥 Drop the .exe to add it as a game</span></div>
      </Show>

      <div class="section-label" style={{ display: "flex", "align-items": "center", "justify-content": "space-between", gap: "12px", "flex-wrap": "wrap" }}>
        <span>Installed Games</span>
        <div style={{ display: "flex", gap: "8px", "align-items": "center" }}>
          <Show when={allTags().length > 0}>
            <select value={tagFilter()} onChange={(e) => { setTagFilter(e.currentTarget.value); setSelected(-1); }} style={{ "font-size": "12px", padding: "6px 10px", width: "auto" }}>
              <option value="">All tags</option>
              <For each={allTags()}>{(t) => <option value={t}>{t}</option>}</For>
            </select>
          </Show>
          <button
            class={favoritesOnly() ? "" : ""}
            style={{ "font-size": "12px", padding: "6px 10px", background: favoritesOnly() ? "linear-gradient(180deg, rgba(250,204,21,0.4) 0%, rgba(202,138,4,0.3) 100%)" : undefined }}
            onClick={() => { setFavoritesOnly(!favoritesOnly()); setSelected(-1); }}
          >
            ⭐ Favorites
          </button>
          <input
            type="text"
            value={search()}
            onInput={(e) => { setSearch(e.currentTarget.value); setSelected(-1); }}
            placeholder="🔍 Search by name or runner…"
            style={{ "max-width": "220px", "font-size": "12px", padding: "6px 10px" }}
          />
        </div>
      </div>

      <div class="table-wrap">
        <Show
          when={games().length > 0}
          fallback={
            <div class="empty-state">
              <span>🎮</span>
              <span>No games added yet</span>
              <span style={{ "font-size": "11px" }}>Tip: drag & drop a .exe file anywhere onto the window</span>
            </div>
          }
        >
          <Show
            when={filteredSorted().length > 0}
            fallback={
              <div class="empty-state">
                <span>🔍</span>
                <span>No games match your filters</span>
              </div>
            }
          >
            <Show
              when={props.libraryView === "Grid"}
              fallback={
                <table>
                  <thead>
                    <tr>
                      <th style={{ width: "30px" }}></th>
                      <th style={{ width: "44px" }}></th>
                      <th class="sortable" onClick={() => toggleSort("name")} style={{ cursor: "pointer" }}>
                        Game Name{sortIndicator("name")}
                      </th>
                      <th class="sortable" onClick={() => toggleSort("runner")} style={{ cursor: "pointer" }}>
                        Runner{sortIndicator("runner")}
                      </th>
                      <th>Status</th>
                      <th class="sortable" onClick={() => toggleSort("playtime")} style={{ cursor: "pointer" }}>
                        Playtime{sortIndicator("playtime")}
                      </th>
                      <th>Tags</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={filteredSorted()}>
                      {(g, i) => (
                        <tr
                          class={selected() === i() ? "selected" : ""}
                          onClick={() => setSelected(i())}
                          onDblClick={handleLaunch}
                        >
                          <td>
                            <button class={`star-btn ${g.favorite ? "active" : ""}`} onClick={(e) => toggleFavorite(g, e)}>
                              {g.favorite ? "★" : "☆"}
                            </button>
                          </td>
                          <td>
                            <Show when={g.icon_path} fallback={<span style={{ opacity: 0.5 }}>🎮</span>}>
                              <img src={convertFileSrc(g.icon_path)} alt="" style={{ width: "28px", height: "28px", "object-fit": "cover", "border-radius": "6px" }} />
                            </Show>
                          </td>
                          <td>{g.name}</td>
                          <td><span class="badge badge-purple">{g.runner}</span></td>
                          <td>
                            <Show when={isRunning(g.name)} fallback={<span class="badge" style={{ background: "rgba(255,255,255,0.05)", color: "var(--text-dim)" }}>Stopped</span>}>
                              <span class="badge badge-green">● Running</span>
                            </Show>
                          </td>
                          <td style={{ color: "var(--text-muted)" }}>{formatPlaytime(g.total_playtime_secs)}</td>
                          <td style={{ color: "var(--text-muted)", "font-size": "11px" }}>
                            <For each={parseTags(g.tags)}>{(t) => <span class="badge badge-gray" style={{ "margin-right": "4px" }}>{t}</span>}</For>
                          </td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              }
            >
              <div class="game-grid">
                <For each={filteredSorted()}>
                  {(g, i) => (
                    <div
                      class={`game-card ${selected() === i() ? "selected" : ""}`}
                      onClick={() => setSelected(i())}
                      onDblClick={handleLaunch}
                    >
                      <Show when={isRunning(g.name)}><div class="running-dot" /></Show>
                      <Show
                        when={g.icon_path}
                        fallback={<div class="cover">🎮</div>}
                      >
                        <img class="cover" src={convertFileSrc(g.icon_path)} alt="" />
                      </Show>
                      <div class="card-title">{g.name}</div>
                      <div class="card-meta">
                        <span class="badge badge-purple" style={{ "font-size": "9px" }}>{g.runner}</span>
                        <button class={`star-btn ${g.favorite ? "active" : ""}`} onClick={(e) => toggleFavorite(g, e)}>
                          {g.favorite ? "★" : "☆"}
                        </button>
                      </div>
                      <div style={{ "font-size": "10px", color: "var(--text-muted)" }}>{formatPlaytime(g.total_playtime_secs)}</div>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </Show>
        </Show>
      </div>

      <div class="btn-row">
        <button onClick={() => { setAddPrefill(null); setShowAdd(true); }}>＋ Add Game</button>
        <button onClick={openSteamImport}>🎮 Import from Steam</button>
        <Show
          when={selectedGame() && isRunning(selectedGame()!.name)}
          fallback={
            <button onClick={handleLaunch} disabled={!selectedGame() || launching()}>
              {launching() ? (<><span class="spinner" /> Launching…</>) : "▶ Launch Game"}
            </button>
          }
        >
          <button class="btn-danger" onClick={handleStop}>■ Stop Game</button>
        </Show>
        <button
          onClick={() => {
            if (!selectedGame()) return props.addToast("No game selected", "error");
            setShowConfigure(true);
          }}
          disabled={!selectedGame()}
        >
          ⚙ Configure Game
        </button>
        <button onClick={openLogs} disabled={!selectedGame()}>🗒 View Logs</button>
        <button class="btn-danger" onClick={handleRemove} disabled={!selectedGame()}>✕ Remove Game</button>
      </div>
      <div style={{ "font-size": "10px", color: "var(--text-dim)" }}>
        Tip: Enter launches/stops the selected game, Delete removes it.
      </div>

      <Show when={showAdd()}>
        <AddGameModal
          initialExe={addPrefill()?.exe}
          initialName={addPrefill()?.name}
          onClose={() => setShowAdd(false)}
          onAdded={async () => {
            setShowAdd(false);
            await loadGames();
            props.addToast("Game added successfully!", "success");
          }}
          addToast={props.addToast}
        />
      </Show>

      <Show when={showConfigure() && selectedGame()}>
        <ConfigureGameModal
          game={selectedGame()!}
          onClose={() => setShowConfigure(false)}
          onSaved={async () => {
            setShowConfigure(false);
            await loadGames();
            props.addToast("Game configuration updated!", "success");
          }}
          addToast={props.addToast}
        />
      </Show>

      <Show when={showLogs()}>
        <div class="modal-overlay" onClick={() => setShowLogs(false)}>
          <div class="modal" style={{ "min-width": "640px" }} onClick={(e) => e.stopPropagation()}>
            <h2>Logs: {logGameName()}</h2>
            <Show
              when={logEntries().length > 0}
              fallback={<div style={{ color: "var(--text-muted)" }}>No log files recorded yet for this game.</div>}
            >
              <div style={{ display: "flex", gap: "6px", "flex-wrap": "wrap" }}>
                <For each={logEntries()}>
                  {(entry) => (
                    <button
                      style={{ "font-size": "11px", padding: "4px 8px" }}
                      onClick={() => viewLog(entry)}
                    >
                      {entry.file_name}
                    </button>
                  )}
                </For>
              </div>
              <pre
                style={{
                  background: "rgba(12,12,24,0.8)",
                  border: "1px solid var(--border-subtle)",
                  "border-radius": "8px",
                  padding: "12px",
                  "max-height": "320px",
                  overflow: "auto",
                  "font-size": "11px",
                  "white-space": "pre-wrap",
                  color: "#c4c4d4",
                }}
              >
                {logContent()}
              </pre>
            </Show>
            <div class="modal-actions">
              <button onClick={() => setShowLogs(false)}>Close</button>
            </div>
          </div>
        </div>
      </Show>

      <Show when={showSteamImport()}>
        <div class="modal-overlay" onClick={() => !importingSteam() && setShowSteamImport(false)}>
          <div class="modal" style={{ "min-width": "560px" }} onClick={(e) => e.stopPropagation()}>
            <h2>Import from Steam Library</h2>
            <p style={{ color: "var(--text-muted)", "font-size": "12px" }}>
              Scans your Steam library folders (including external/removable drives) for installed games.
              The main executable is guessed heuristically — check and fix the path below before importing
              if it picked the wrong file. Imported games use the Steam runner, so Steam itself manages
              Proton/prefix for them.
            </p>
            <Show when={!scanningSteam()} fallback={<div class="empty-state"><span class="spinner" /><span>Scanning Steam library…</span></div>}>
              <Show
                when={steamCandidates() && steamCandidates()!.length > 0}
                fallback={<div style={{ color: "var(--text-muted)" }}>No new, uninstalled-here Steam games found.</div>}
              >
                <div style={{ display: "flex", "flex-direction": "column", gap: "6px", "max-height": "360px", overflow: "auto" }}>
                  <For each={steamCandidates()!}>
                    {(c) => (
                      <div style={{ padding: "8px", "border-radius": "6px", background: "rgba(255,255,255,0.02)", display: "flex", "flex-direction": "column", gap: "6px" }}>
                        <label class="checkbox-row">
                          <input type="checkbox" checked={selectedSteamIds().has(c.app_id)} onChange={() => toggleSteamCandidate(c.app_id)} />
                          <span style={{ flex: 1, "font-weight": 600 }}>{c.name}</span>
                          <span style={{ "font-size": "11px", color: "var(--text-dim)" }}>AppID {c.app_id}</span>
                        </label>
                        <div class="input-group" style={{ "padding-left": "24px" }}>
                          <input
                            type="text"
                            value={editedExePaths()[c.app_id] ?? c.exe_path}
                            onInput={(e) => setEditedExePaths((m) => ({ ...m, [c.app_id]: e.currentTarget.value }))}
                            placeholder="Guessed executable — fix if wrong"
                            style={{ "font-size": "11px" }}
                          />
                          <button style={{ "font-size": "11px" }} onClick={() => browseSteamExe(c.app_id)}>📁</button>
                        </div>
                      </div>
                    )}
                  </For>
                </div>
              </Show>
            </Show>
            <div class="modal-actions">
              <button onClick={() => setShowSteamImport(false)} disabled={importingSteam()} style={{ background: "rgba(255,255,255,0.05)" }}>Cancel</button>
              <button onClick={importSelectedSteamGames} disabled={importingSteam() || selectedSteamIds().size === 0}>
                {importingSteam() ? (<><span class="spinner" /> Importing…</>) : `Import Selected (${selectedSteamIds().size})`}
              </button>
            </div>
          </div>
        </div>
      </Show>

      <ConfirmModal request={confirmReq()} onDismiss={() => setConfirmReq(null)} />
    </>
  );
}
