import { createSignal, createMemo, onMount, onCleanup, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { Game, Toast, ProtonDbInfo, DependencyHint } from "../types";
import ControllerMappingWizard from "./ControllerMappingWizard";

interface Props {
  game: Game;
  onClose: () => void;
  onSaved: () => void;
  addToast: (msg: string, kind?: Toast["kind"]) => void;
}

const PROTONDB_TIER_BADGE: Record<string, string> = {
  platinum: "badge-purple",
  gold: "badge-yellow",
  silver: "badge-gray",
  bronze: "badge-red",
  borked: "badge-red",
  native: "badge-green",
  pending: "badge-gray",
};

export default function ConfigureGameModal(props: Props) {
  const originalName = props.game.name;

  const [name, setName] = createSignal(props.game.name);
  const [runner, setRunner] = createSignal(props.game.runner.includes("Proton") ? "Proton" : props.game.runner);
  const [protonVersion, setProtonVersion] = createSignal(props.game.runner.includes("Proton") ? props.game.runner : "");
  const [protonVersions, setProtonVersions] = createSignal<string[]>([]);
  const [fpsLimit, setFpsLimit] = createSignal(props.game.fps_limit != null ? String(props.game.fps_limit) : "");
  const [launchOptions, setLaunchOptions] = createSignal(props.game.launch_options);
  const [prefix, setPrefix] = createSignal(props.game.prefix);
  const [envVars, setEnvVars] = createSignal(props.game.env_vars ?? "");
  const [iconPath, setIconPath] = createSignal(props.game.icon_path ?? "");
  const [enableDxvk, setEnableDxvk] = createSignal(props.game.enable_dxvk);
  const [enableEsync, setEnableEsync] = createSignal(props.game.enable_esync);
  const [enableFsync, setEnableFsync] = createSignal(props.game.enable_fsync);
  const [enableDxvkAsync, setEnableDxvkAsync] = createSignal(props.game.enable_dxvk_async);
  const [tags, setTags] = createSignal(props.game.tags ?? "");
  const [favorite, setFavorite] = createSignal(props.game.favorite ?? false);
  const [useSharedPrefix, setUseSharedPrefix] = createSignal(props.game.use_shared_prefix ?? false);
  const [appId, setAppId] = createSignal(props.game.app_id ?? "");
  const [disableSteamInput, setDisableSteamInput] = createSignal(props.game.disable_steam_input ?? false);
  const [sdlControllerConfig, setSdlControllerConfig] = createSignal(props.game.sdl_controller_config ?? "");
  const [saving, setSaving] = createSignal(false);

  // ProtonDB
  const [protonDb, setProtonDb] = createSignal<ProtonDbInfo | null>(null);
  const [checkingProtonDb, setCheckingProtonDb] = createSignal(false);

  // Winetricks
  const [showWinetricks, setShowWinetricks] = createSignal(false);
  const [verbs, setVerbs] = createSignal<Array<[string, string]>>([]);
  const [fullCatalog, setFullCatalog] = createSignal<Array<[string, string]> | null>(null);
  const [loadingCatalog, setLoadingCatalog] = createSignal(false);
  const [catalogSearch, setCatalogSearch] = createSignal("");
  const [selectedVerbs, setSelectedVerbs] = createSignal<Set<string>>(new Set());
  const [runningWinetricks, setRunningWinetricks] = createSignal(false);
  const [winetricksLog, setWinetricksLog] = createSignal("");

  // Dependency scan
  const [showDeps, setShowDeps] = createSignal(false);
  const [depHints, setDepHints] = createSignal<DependencyHint[] | null>(null);
  const [scanningDeps, setScanningDeps] = createSignal(false);
  const [runningInstaller, setRunningInstaller] = createSignal("");
  const [installerLog, setInstallerLog] = createSignal("");

  // Controller mapping wizard
  const [showControllerWizard, setShowControllerWizard] = createSignal(false);

  onMount(() => {
    invoke<Array<{ version: string }>>("get_installed_protons")
      .then((p) => {
        const versions = p.map((x) => x.version);
        setProtonVersions(versions);
        if (!protonVersion() && versions.length > 0) setProtonVersion(versions[0]);
      })
      .catch(() => {});
    if (appId().trim()) checkProtonDb();

    // Live progress: winetricks / installer both stream stdout+stderr line
    // by line via this event instead of only showing a log once finished,
    // so long operations (dotnet48 can take minutes) don't look frozen.
    const unlistenOutput = listen<{ source: string; line: string }>("process_output", (event) => {
      const { source, line } = event.payload;
      if (source === "winetricks") {
        setWinetricksLog((l) => l + line + "\n");
      } else if (source === "installer") {
        setInstallerLog((l) => l + line + "\n");
      }
    });
    onCleanup(() => { unlistenOutput.then((f) => f()); });
  });

  const isWineOrProton = () => runner() === "Wine" || runner() === "Proton";
  const effectivePrefix = () => prefix().trim() || `(auto: ~/.hackeros/.../Prefixes/${props.game.name.replace(/ /g, "_")})`;

  const browseIcon = async () => {
    const result = await open({
      filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "ico", "webp"] }],
    });
    if (typeof result === "string") setIconPath(result);
  };

  const checkProtonDb = async () => {
    if (!appId().trim()) return props.addToast("Enter a Steam App ID first", "error");
    setCheckingProtonDb(true);
    setProtonDb(null);
    try {
      const info = await invoke<ProtonDbInfo | null>("check_protondb", { appId: appId().trim() });
      if (!info) props.addToast("No ProtonDB reports found for this App ID", "info");
      setProtonDb(info);
    } catch (e) {
      props.addToast(`ProtonDB lookup failed: ${e}`, "error");
    } finally {
      setCheckingProtonDb(false);
    }
  };

  const handleSave = async () => {
    if (!name().trim()) return props.addToast("Name is required", "error");
    const finalRunner = runner() === "Proton" ? protonVersion() : runner();
    const updated: Game = {
      ...props.game,
      name: name().trim(),
      runner: finalRunner,
      fps_limit: fpsLimit() ? parseInt(fpsLimit()) : null,
      launch_options: launchOptions(),
      prefix: prefix(),
      env_vars: envVars(),
      icon_path: iconPath(),
      enable_dxvk: enableDxvk(),
      enable_esync: enableEsync(),
      enable_fsync: enableFsync(),
      enable_dxvk_async: enableDxvkAsync(),
      tags: tags().trim(),
      favorite: favorite(),
      use_shared_prefix: useSharedPrefix(),
      app_id: appId().trim(),
      disable_steam_input: disableSteamInput(),
      sdl_controller_config: sdlControllerConfig().trim(),
    };
    setSaving(true);
    try {
      await invoke("update_game", { game: updated, originalName });
      props.onSaved();
    } catch (e) {
      props.addToast(`Failed to save: ${e}`, "error");
    } finally {
      setSaving(false);
    }
  };

  // ── Winetricks ──────────────────────────────
  const openWinetricks = async () => {
    setShowWinetricks(true);
    setWinetricksLog("");
    setSelectedVerbs(new Set<string>());
    if (verbs().length === 0) {
      try {
        const v = await invoke<Array<[string, string]>>("get_common_winetricks_verbs");
        setVerbs(v);
      } catch (e) {
        props.addToast(`Failed to load winetricks components: ${e}`, "error");
      }
    }
  };

  const loadFullCatalog = async () => {
    if (fullCatalog() !== null) return;
    setLoadingCatalog(true);
    try {
      const all = await invoke<Array<[string, string]>>("get_all_winetricks_verbs");
      setFullCatalog(all);
    } catch (e) {
      props.addToast(`Failed to load full catalog: ${e}`, "error");
      setFullCatalog([]);
    } finally {
      setLoadingCatalog(false);
    }
  };

  const filteredCatalog = createMemo(() => {
    const q = catalogSearch().trim().toLowerCase();
    const list = fullCatalog() ?? [];
    if (!q) return list.slice(0, 200); // avoid rendering ~700 rows unfiltered
    return list.filter(([verb, label]) => verb.toLowerCase().includes(q) || label.toLowerCase().includes(q));
  });

  const toggleVerb = (verb: string) => {
    setSelectedVerbs((s) => {
      const next = new Set(s);
      if (next.has(verb)) next.delete(verb);
      else next.add(verb);
      return next;
    });
  };

  const runWinetricks = async () => {
    if (selectedVerbs().size === 0) return props.addToast("Select at least one component", "error");
    setRunningWinetricks(true);
    setWinetricksLog("");
    try {
      const log = await invoke<string>("run_winetricks", {
        prefix: prefix().trim() || effectivePrefix(),
        verbs: Array.from(selectedVerbs()),
      });
      setWinetricksLog(log || "(finished, no output)");
      props.addToast("Winetricks finished!", "success");
    } catch (e) {
      setWinetricksLog(String(e));
      props.addToast(`Winetricks failed: ${e}`, "error");
    } finally {
      setRunningWinetricks(false);
    }
  };

  // ── Dependency scan ─────────────────────────
  const openDeps = async () => {
    setShowDeps(true);
    setScanningDeps(true);
    setDepHints(null);
    setInstallerLog("");
    setWinetricksLog("");
    try {
      const hints = await invoke<DependencyHint[]>("scan_game_dependencies", { exePath: props.game.exe });
      setDepHints(hints);
    } catch (e) {
      props.addToast(`Scan failed: ${e}`, "error");
      setDepHints([]);
    } finally {
      setScanningDeps(false);
    }
  };

  const runInstaller = async (hint: DependencyHint) => {
    setRunningInstaller(hint.path);
    setInstallerLog("");
    try {
      await invoke("run_dependency_installer", {
        prefix: prefix().trim() || effectivePrefix(),
        installerPath: hint.path,
      });
      props.addToast(`${hint.label} installer closed`, "success");
    } catch (e) {
      props.addToast(`Installer failed: ${e}`, "error");
    } finally {
      setRunningInstaller("");
    }
  };

  const runVerbFor = async (hint: DependencyHint) => {
    if (!hint.winetricks_verb) return;
    setRunningInstaller(hint.path);
    setWinetricksLog("");
    try {
      await invoke<string>("run_winetricks", {
        prefix: prefix().trim() || effectivePrefix(),
        verbs: [hint.winetricks_verb],
      });
      props.addToast(`${hint.label} (via winetricks) installed`, "success");
    } catch (e) {
      props.addToast(`Winetricks failed: ${e}`, "error");
    } finally {
      setRunningInstaller("");
    }
  };

  return (
    <div class="modal-overlay" onClick={props.onClose}>
      <div class="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Configure: {props.game.name}</h2>
        <div class="form-grid">
          <label class="form-label">Game Name</label>
          <input type="text" value={name()} onInput={(e) => setName(e.currentTarget.value)} />

          <label class="form-label">Runner</label>
          <select value={runner()} onChange={(e) => setRunner(e.currentTarget.value)}>
            <For each={["Native", "Wine", "Proton", "Flatpak", "Steam"]}>{(r) => <option>{r}</option>}</For>
          </select>

          <Show when={runner() === "Proton"}>
            <label class="form-label">Proton Version</label>
            <select value={protonVersion()} onChange={(e) => setProtonVersion(e.currentTarget.value)}>
              <Show when={protonVersions().length > 0} fallback={<option>No Proton installed</option>}>
                <For each={protonVersions()}>{(v) => <option>{v}</option>}</For>
              </Show>
            </select>
          </Show>

          <label class="form-label">Icon / Cover</label>
          <div class="input-group">
            <input type="text" value={iconPath()} onInput={(e) => setIconPath(e.currentTarget.value)} placeholder="Optional image path" />
            <button onClick={browseIcon}>🖼️ Browse</button>
          </div>

          <label class="form-label">Launch Options</label>
          <input
            type="text"
            value={launchOptions()}
            onInput={(e) => setLaunchOptions(e.currentTarget.value)}
            placeholder='--fullscreen --gamescope ...'
          />

          <label class="form-label">Env Variables</label>
          <textarea
            rows="3"
            value={envVars()}
            onInput={(e) => setEnvVars(e.currentTarget.value)}
            placeholder={"One per line, e.g.\nMANGOHUD=1\nDXVK_HUD=fps"}
          />

          <label class="form-label">FPS Limit</label>
          <input type="number" value={fpsLimit()} onInput={(e) => setFpsLimit(e.currentTarget.value)} placeholder="e.g. 60" />

          <label class="form-label">Tags</label>
          <input type="text" value={tags()} onInput={(e) => setTags(e.currentTarget.value)} placeholder="e.g. RPG, Co-op" />

          <span></span>
          <label class="checkbox-row">
            <input type="checkbox" checked={favorite()} onChange={(e) => setFavorite(e.currentTarget.checked)} />
            ⭐ Mark as favorite
          </label>

          <label class="form-label">Steam App ID</label>
          <div class="input-group">
            <input type="text" value={appId()} onInput={(e) => setAppId(e.currentTarget.value)} placeholder="For Steam runner, or just for ProtonDB lookup" />
            <button onClick={checkProtonDb} disabled={checkingProtonDb()}>
              {checkingProtonDb() ? <span class="spinner" /> : "🔍 ProtonDB"}
            </button>
          </div>
          <Show when={protonDb()}>
            {(info) => (
              <>
                <span></span>
                <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
                  <span class={`badge ${PROTONDB_TIER_BADGE[info().tier.toLowerCase()] ?? "badge-gray"}`}>
                    {info().tier || "unknown"}
                  </span>
                  <Show when={info().trending_tier && info().trending_tier !== info().tier}>
                    <span style={{ color: "var(--text-muted)", "font-size": "11px" }}>
                      trending: {info().trending_tier}
                    </span>
                  </Show>
                  <Show when={["borked", "bronze"].includes(info().tier.toLowerCase())}>
                    <span style={{ color: "#fca5a5", "font-size": "11px" }}>
                      ⚠ Heuristic: reports suggest this may run poorly. Check ProtonDB for details.
                    </span>
                  </Show>
                </div>
              </>
            )}
          </Show>

          <Show when={isWineOrProton()}>
            <label class="form-label">Prefix Path</label>
            <input
              type="text"
              value={prefix()}
              onInput={(e) => setPrefix(e.currentTarget.value)}
              placeholder={useSharedPrefix() ? "Using shared prefix (see Settings)" : "Leave empty for default"}
              disabled={useSharedPrefix()}
            />
            <span></span>
            <label class="checkbox-row">
              <input type="checkbox" checked={useSharedPrefix()} onChange={(e) => setUseSharedPrefix(e.currentTarget.checked)} />
              Use shared prefix instead of a per-game one
            </label>

            <div class="full-row" style={{ display: "flex", "flex-direction": "column", gap: "8px", "padding-top": "4px" }}>
              <label class="checkbox-row">
                <input type="checkbox" checked={enableDxvk()} onChange={(e) => setEnableDxvk(e.currentTarget.checked)} />
                Enable DXVK / VKD3D
              </label>
              <label class="checkbox-row">
                <input type="checkbox" checked={enableEsync()} onChange={(e) => setEnableEsync(e.currentTarget.checked)} />
                Enable Esync (Override)
              </label>
              <label class="checkbox-row">
                <input type="checkbox" checked={enableFsync()} onChange={(e) => setEnableFsync(e.currentTarget.checked)} />
                Enable Fsync (Override)
              </label>
              <label class="checkbox-row">
                <input type="checkbox" checked={enableDxvkAsync()} onChange={(e) => setEnableDxvkAsync(e.currentTarget.checked)} />
                Enable DXVK Async (Override)
              </label>
            </div>

            <div class="full-row" style={{ display: "flex", gap: "8px" }}>
              <button onClick={openWinetricks}>🍷 Winetricks</button>
              <button onClick={openDeps}>🔎 Scan Dependencies</button>
            </div>

            <details class="collapsible full-row">
              <summary>Controller / Gamepad</summary>
              <div class="collapsible-body">
                <span></span>
                <label class="checkbox-row">
                  <input type="checkbox" checked={disableSteamInput()} onChange={(e) => setDisableSteamInput(e.currentTarget.checked)} />
                  Disable Steam Input (use raw XInput/DirectInput instead)
                </label>
                <label class="form-label">SDL Controller Config</label>
                <div style={{ display: "flex", "flex-direction": "column", gap: "6px" }}>
                  <textarea
                    rows="2"
                    value={sdlControllerConfig()}
                    onInput={(e) => setSdlControllerConfig(e.currentTarget.value)}
                    placeholder="Advanced: raw SDL_GAMECONTROLLERCONFIG value, leave empty unless you need a custom mapping"
                  />
                  <button style={{ "font-size": "11px", "justify-self": "start" }} onClick={() => setShowControllerWizard(true)}>
                    🎮 Build Config (press buttons instead of typing the string)
                  </button>
                </div>
              </div>
            </details>
          </Show>
        </div>
        <div class="modal-actions">
          <button onClick={props.onClose} style={{ background: "rgba(255,255,255,0.05)" }}>Cancel</button>
          <button onClick={handleSave} disabled={saving()}>
            {saving() ? (
              <>
                <span class="spinner" /> Saving…
              </>
            ) : (
              "💾 Save Changes"
            )}
          </button>
        </div>
      </div>

      <Show when={showWinetricks()}>
        <div class="modal-overlay" onClick={(e) => { e.stopPropagation(); if (!runningWinetricks()) setShowWinetricks(false); }}>
          <div class="modal" onClick={(e) => e.stopPropagation()}>
            <h2>Winetricks — {props.game.name}</h2>
            <p style={{ color: "var(--text-muted)", "font-size": "12px" }}>
              Installs common Windows components into this game's prefix. Requires <code>winetricks</code> to be installed on your system.
            </p>
            <div style={{ display: "flex", "align-items": "center", gap: "8px", "flex-wrap": "wrap" }}>
              <button style={{ "font-size": "11px" }} onClick={loadFullCatalog} disabled={loadingCatalog()}>
                {loadingCatalog() ? (<><span class="spinner" /> Loading catalog…</>) : fullCatalog() ? "🔎 Full catalog loaded (~700)" : "🔎 Browse full catalog (~700)"}
              </button>
              <Show when={fullCatalog() !== null}>
                <input
                  type="text"
                  value={catalogSearch()}
                  onInput={(e) => setCatalogSearch(e.currentTarget.value)}
                  placeholder={`Search ${fullCatalog()?.length ?? 0} components…`}
                  style={{ "font-size": "11px", padding: "5px 8px", flex: 1, "min-width": "160px" }}
                />
              </Show>
            </div>
            <Show
              when={fullCatalog() === null}
              fallback={
                <div style={{ display: "grid", "grid-template-columns": "1fr 1fr", gap: "6px", "max-height": "220px", overflow: "auto" }}>
                  <Show when={filteredCatalog().length > 0} fallback={<div style={{ color: "var(--text-muted)", "font-size": "12px" }}>No matching components.</div>}>
                    <For each={filteredCatalog()}>
                      {([verb, label]) => (
                        <label class="checkbox-row" title={verb}>
                          <input type="checkbox" checked={selectedVerbs().has(verb)} onChange={() => toggleVerb(verb)} />
                          <span style={{ overflow: "hidden", "text-overflow": "ellipsis", "white-space": "nowrap" }}>{verb} — {label}</span>
                        </label>
                      )}
                    </For>
                  </Show>
                </div>
              }
            >
              <div style={{ display: "grid", "grid-template-columns": "1fr 1fr", gap: "6px" }}>
                <For each={verbs()}>
                  {([verb, label]) => (
                    <label class="checkbox-row">
                      <input type="checkbox" checked={selectedVerbs().has(verb)} onChange={() => toggleVerb(verb)} />
                      {label}
                    </label>
                  )}
                </For>
              </div>
            </Show>
            <Show when={winetricksLog()}>
              <pre style={{
                background: "rgba(12,12,24,0.8)", border: "1px solid var(--border-subtle)",
                "border-radius": "8px", padding: "10px", "max-height": "180px", overflow: "auto",
                "font-size": "11px", "white-space": "pre-wrap", color: "#c4c4d4",
              }}>{winetricksLog()}</pre>
            </Show>
            <div class="modal-actions">
              <button onClick={() => setShowWinetricks(false)} disabled={runningWinetricks()} style={{ background: "rgba(255,255,255,0.05)" }}>Close</button>
              <button onClick={runWinetricks} disabled={runningWinetricks() || selectedVerbs().size === 0}>
                {runningWinetricks() ? (<><span class="spinner" /> Running (can take a while)…</>) : "▶ Run Selected"}
              </button>
            </div>
          </div>
        </div>
      </Show>

      <Show when={showDeps()}>
        <div class="modal-overlay" onClick={(e) => { e.stopPropagation(); setShowDeps(false); }}>
          <div class="modal" onClick={(e) => e.stopPropagation()}>
            <h2>Dependency Scan — {props.game.name}</h2>
            <p style={{ color: "var(--text-muted)", "font-size": "12px" }}>
              Looks for bundled redistributable installers (VC++, .NET, DirectX…) next to the game's executable.
              This is a filename heuristic, not a guarantee — it can't detect dependencies that aren't shipped as a visible installer.
            </p>
            <Show when={!scanningDeps()} fallback={<div class="empty-state"><span class="spinner" /><span>Scanning…</span></div>}>
              <Show
                when={depHints() && depHints()!.length > 0}
                fallback={<div style={{ color: "var(--text-muted)" }}>No known installer files found in the game's folder.</div>}
              >
                <div style={{ display: "flex", "flex-direction": "column", gap: "8px" }}>
                  <For each={depHints()!}>
                    {(hint) => (
                      <div style={{ display: "flex", "align-items": "center", "justify-content": "space-between", gap: "8px", padding: "8px", background: "rgba(255,255,255,0.03)", "border-radius": "8px" }}>
                        <div>
                          <div style={{ "font-weight": 600 }}>{hint.label}</div>
                          <div style={{ "font-size": "11px", color: "var(--text-muted)" }}>{hint.path}</div>
                        </div>
                        <div style={{ display: "flex", gap: "6px" }}>
                          <button
                            onClick={() => runInstaller(hint)}
                            disabled={runningInstaller() === hint.path}
                            style={{ "font-size": "11px" }}
                          >
                            Run Installer
                          </button>
                          <Show when={hint.winetricks_verb}>
                            <button
                              onClick={() => runVerbFor(hint)}
                              disabled={runningInstaller() === hint.path}
                              style={{ "font-size": "11px" }}
                            >
                              Via Winetricks
                            </button>
                          </Show>
                        </div>
                      </div>
                    )}
                  </For>
                </div>
              </Show>
            </Show>
            <Show when={runningInstaller() || installerLog() || winetricksLog()}>
              <pre style={{
                background: "rgba(12,12,24,0.8)", border: "1px solid var(--border-subtle)",
                "border-radius": "8px", padding: "10px", "max-height": "160px", overflow: "auto",
                "font-size": "11px", "white-space": "pre-wrap", color: "#c4c4d4",
              }}>{installerLog() || winetricksLog() || "Starting…"}</pre>
            </Show>
            <div class="modal-actions">
              <button onClick={() => setShowDeps(false)}>Close</button>
            </div>
          </div>
        </div>
      </Show>

      <Show when={showControllerWizard()}>
        <ControllerMappingWizard
          addToast={props.addToast}
          onClose={() => setShowControllerWizard(false)}
          onGenerated={(cfg) => setSdlControllerConfig(cfg)}
        />
      </Show>
    </div>
  );
}
