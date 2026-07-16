import { createSignal, onMount, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Game, Toast, emptyGame } from "../types";

interface Props {
  onClose: () => void;
  onAdded: () => void;
  addToast: (msg: string, kind?: Toast["kind"]) => void;
  initialExe?: string;
  initialName?: string;
  initialRunner?: string;
  initialAppId?: string;
}

const RUNNERS = ["Native", "Wine", "Proton", "Flatpak", "Steam"];

export default function AddGameModal(props: Props) {
  const [name, setName] = createSignal(props.initialName ?? "");
  const [exe, setExe] = createSignal(props.initialExe ?? "");
  const [runner, setRunner] = createSignal(props.initialRunner ?? "Proton");
  const [protonVersion, setProtonVersion] = createSignal("");
  const [protonVersions, setProtonVersions] = createSignal<string[]>([]);
  const [prefix, setPrefix] = createSignal("");
  const [launchOptions, setLaunchOptions] = createSignal("");
  const [fpsLimit, setFpsLimit] = createSignal("");
  const [enableDxvk, setEnableDxvk] = createSignal(false);
  const [enableEsync, setEnableEsync] = createSignal(false);
  const [enableFsync, setEnableFsync] = createSignal(false);
  const [enableDxvkAsync, setEnableDxvkAsync] = createSignal(false);
  const [appId, setAppId] = createSignal(props.initialAppId ?? "");
  const [envVars, setEnvVars] = createSignal("");
  const [iconPath, setIconPath] = createSignal("");
  const [tags, setTags] = createSignal("");
  const [favorite, setFavorite] = createSignal(false);
  const [useSharedPrefix, setUseSharedPrefix] = createSignal(false);
  const [saving, setSaving] = createSignal(false);

  onMount(() => {
    invoke<Array<{ version: string }>>("get_installed_protons")
      .then((p) => {
        const versions = p.map((x) => x.version);
        setProtonVersions(versions);
        if (versions.length > 0) setProtonVersion(versions[0]);
      })
      .catch(() => {});
  });

  const isWineOrProton = () => runner() === "Wine" || runner() === "Proton";

  const browseExe = async () => {
    const result = await open({
      filters: [
        { name: "Executables", extensions: ["exe", "bat"] },
        { name: "All Files", extensions: ["*"] },
      ],
    });
    if (typeof result === "string") setExe(result);
  };

  const browsePrefix = async () => {
    const result = await open({ directory: true });
    if (typeof result === "string") setPrefix(result);
  };

  const browseIcon = async () => {
    const result = await open({
      filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "ico", "webp"] }],
    });
    if (typeof result === "string") setIconPath(result);
  };

  const handleAdd = async () => {
    if (!name().trim()) return props.addToast("Game name is required", "error");
    if (runner() !== "Steam" && !exe().trim())
      return props.addToast("Executable is required", "error");
    if (runner() === "Steam" && !appId().trim())
      return props.addToast("Steam App ID is required", "error");

    const finalRunner = runner() === "Proton" ? protonVersion() : runner();
    const game: Game = emptyGame({
      name: name().trim(),
      exe: exe().trim(),
      runner: finalRunner,
      prefix: prefix().trim(),
      launch_options: launchOptions().trim(),
      fps_limit: fpsLimit() ? parseInt(fpsLimit()) : null,
      enable_dxvk: enableDxvk(),
      enable_esync: enableEsync(),
      enable_fsync: enableFsync(),
      enable_dxvk_async: enableDxvkAsync(),
      app_id: appId().trim(),
      env_vars: envVars().trim(),
      icon_path: iconPath().trim(),
      tags: tags().trim(),
      favorite: favorite(),
      use_shared_prefix: useSharedPrefix(),
    });

    setSaving(true);
    try {
      await invoke("add_game", { game });
      props.onAdded();
    } catch (e) {
      props.addToast(`Failed to add game: ${e}`, "error");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div class="modal-overlay" onClick={props.onClose}>
      <div class="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Add New Game</h2>
        <div class="form-grid">
          <label class="form-label">Game Name</label>
          <input type="text" value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="Enter game name" />

          <label class="form-label">Runner</label>
          <select value={runner()} onChange={(e) => setRunner(e.currentTarget.value)}>
            <For each={RUNNERS}>{(r) => <option>{r}</option>}</For>
          </select>

          <Show when={runner() === "Proton"}>
            <label class="form-label">Proton Version</label>
            <select value={protonVersion()} onChange={(e) => setProtonVersion(e.currentTarget.value)}>
              <Show when={protonVersions().length > 0} fallback={<option>No Proton installed</option>}>
                <For each={protonVersions()}>{(v) => <option>{v}</option>}</For>
              </Show>
            </select>
          </Show>

          <Show when={runner() !== "Steam"}>
            <label class="form-label">Executable</label>
            <div class="input-group">
              <input type="text" value={exe()} onInput={(e) => setExe(e.currentTarget.value)} placeholder="Select or enter path" />
              <button onClick={browseExe}>📁 Browse</button>
            </div>
          </Show>

          <Show when={runner() === "Steam"}>
            <label class="form-label">Steam App ID</label>
            <input type="text" value={appId()} onInput={(e) => setAppId(e.currentTarget.value)} placeholder="e.g. 570" />
          </Show>

          <Show when={isWineOrProton()}>
            <label class="form-label">Wine Prefix</label>
            <div class="input-group">
              <input type="text" value={prefix()} onInput={(e) => setPrefix(e.currentTarget.value)} placeholder="Leave empty for auto" disabled={useSharedPrefix()} />
              <button onClick={browsePrefix} disabled={useSharedPrefix()}>📁 Browse</button>
            </div>
            <span></span>
            <label class="checkbox-row">
              <input type="checkbox" checked={useSharedPrefix()} onChange={(e) => setUseSharedPrefix(e.currentTarget.checked)} />
              Use shared prefix (configured in Settings) instead of a per-game one
            </label>
          </Show>

          <label class="form-label">Tags</label>
          <input type="text" value={tags()} onInput={(e) => setTags(e.currentTarget.value)} placeholder="e.g. RPG, Co-op, Favorites" />

          <span></span>
          <label class="checkbox-row">
            <input type="checkbox" checked={favorite()} onChange={(e) => setFavorite(e.currentTarget.checked)} />
            ⭐ Mark as favorite
          </label>

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
            placeholder='--fullscreen --config="C:\Path With Spaces\cfg.ini"'
          />

          <label class="form-label">Env Variables</label>
          <textarea
            rows="3"
            value={envVars()}
            onInput={(e) => setEnvVars(e.currentTarget.value)}
            placeholder={"One per line, e.g.\nMANGOHUD=1\nDXVK_HUD=fps"}
          />

          <label class="form-label">FPS Limit</label>
          <input type="number" value={fpsLimit()} onInput={(e) => setFpsLimit(e.currentTarget.value)} placeholder="e.g. 60 (Gamescope only)" />

          <Show when={isWineOrProton()}>
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
          </Show>
        </div>
        <div class="modal-actions">
          <button onClick={props.onClose} style={{ background: "rgba(255,255,255,0.05)" }}>Cancel</button>
          <button onClick={handleAdd} disabled={saving()}>
            {saving() ? (
              <>
                <span class="spinner" /> Adding…
              </>
            ) : (
              "＋ Add Game"
            )}
          </button>
        </div>
      </div>
    </div>
  );
}
