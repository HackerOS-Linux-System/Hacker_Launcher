import { createSignal, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Settings, Paths, Toast } from "../types";

interface Props {
  addToast: (msg: string, kind?: Toast["kind"]) => void;
}

function defaultSettings(): Settings {
  return {
    fullscreen: false,
    default_runner: "Proton",
    auto_update: "Enabled",
    enable_esync: true,
    enable_fsync: true,
    enable_dxvk_async: false,
    theme: "Dark (Default)",
    use_shared_prefix_default: false,
    shared_prefix_path: "",
    library_view: "List",
  };
}

export function applyTheme(theme: string) {
  document.documentElement.dataset.theme = theme.startsWith("Light") ? "light" : "dark";
}

export default function SettingsTab(props: Props) {
  const [settings, setSettings] = createSignal<Settings>(defaultSettings());
  const [paths, setPaths] = createSignal<Paths>({ prefixes_dir: "", protons_dir: "", logs_dir: "" });
  const [saving, setSaving] = createSignal(false);
  const [exporting, setExporting] = createSignal(false);
  const [importing, setImporting] = createSignal(false);
  const [initialLibraryView, setInitialLibraryView] = createSignal<Settings["library_view"]>("List");

  onMount(() => {
    invoke<Settings>("get_settings").then((s) => {
      setSettings(s);
      setInitialLibraryView(s.library_view);
    }).catch(() => {});
    invoke<Paths>("get_paths").then(setPaths).catch(() => {});
  });

  const update = <K extends keyof Settings>(key: K, value: Settings[K]) => {
    setSettings((s) => ({ ...s, [key]: value }));
    if (key === "theme") applyTheme(value as string);
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      await invoke("save_settings", { settings: settings() });
      props.addToast(
        settings().library_view !== initialLibraryView()
          ? "Settings saved! Restart the app to switch the library view."
          : "Settings saved!",
        "success"
      );
    } catch (e) {
      props.addToast(`Failed to save: ${e}`, "error");
    } finally {
      setSaving(false);
    }
  };

  const browseSharedPrefix = async () => {
    const result = await open({ directory: true });
    if (typeof result === "string") update("shared_prefix_path", result);
  };

  const handleExport = async () => {
    const dest = await save({
      defaultPath: "hacker-launcher-backup.json",
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!dest) return;
    setExporting(true);
    try {
      await invoke("export_backup", { destPath: dest });
      props.addToast("Backup exported!", "success");
    } catch (e) {
      props.addToast(`Export failed: ${e}`, "error");
    } finally {
      setExporting(false);
    }
  };

  const handleImport = async (merge: boolean) => {
    const src = await open({ filters: [{ name: "JSON", extensions: ["json"] }] });
    if (typeof src !== "string") return;
    setImporting(true);
    try {
      await invoke("import_backup", { srcPath: src, merge });
      props.addToast(merge ? "New games imported!" : "Backup restored! Restart recommended.", "success");
      const s = await invoke<Settings>("get_settings");
      setSettings(s);
    } catch (e) {
      props.addToast(`Import failed: ${e}`, "error");
    } finally {
      setImporting(false);
    }
  };

  return (
    <div style={{ overflow: "auto", flex: 1 }}>
      <div class="section-label" style={{ "margin-bottom": "20px" }}>Settings</div>
      <div class="settings-grid">
        <div class="settings-section">Appearance</div>
        <span class="settings-label">Theme</span>
        <select value={settings().theme} onChange={(e) => update("theme", e.currentTarget.value)}>
          <option>Dark (Default)</option>
          <option>Light</option>
        </select>
        <span class="settings-label">Fullscreen Mode</span>
        <select
          value={settings().fullscreen ? "Enabled" : "Disabled"}
          onChange={(e) => update("fullscreen", e.currentTarget.value === "Enabled")}
        >
          <option>Enabled</option>
          <option>Disabled</option>
        </select>
        <span class="settings-label">Library View</span>
        <select
          value={settings().library_view}
          onChange={(e) => update("library_view", e.currentTarget.value as Settings["library_view"])}
        >
          <option value="List">List</option>
          <option value="Grid">Grid (cover art tiles)</option>
        </select>

        <div class="settings-section">Game Defaults</div>
        <span class="settings-label">Default Runner</span>
        <select
          value={settings().default_runner}
          onChange={(e) => update("default_runner", e.currentTarget.value)}
        >
          {["Native", "Wine", "Proton", "Flatpak", "Steam"].map((r) => (
            <option>{r}</option>
          ))}
        </select>
        <span class="settings-label">Auto-check Updates</span>
        <select
          value={settings().auto_update}
          onChange={(e) => update("auto_update", e.currentTarget.value)}
        >
          <option>Enabled</option>
          <option>Disabled</option>
        </select>

        <div class="settings-section">Wine / Proton (Global)</div>
        <span class="settings-label" style={{ "grid-column": "1 / -1" }}>
          <label class="checkbox-row">
            <input
              type="checkbox"
              checked={settings().enable_esync}
              onChange={(e) => update("enable_esync", e.currentTarget.checked)}
            />
            Enable Esync globally
          </label>
        </span>
        <span class="settings-label" style={{ "grid-column": "1 / -1" }}>
          <label class="checkbox-row">
            <input
              type="checkbox"
              checked={settings().enable_fsync}
              onChange={(e) => update("enable_fsync", e.currentTarget.checked)}
            />
            Enable Fsync globally
          </label>
        </span>
        <span class="settings-label" style={{ "grid-column": "1 / -1" }}>
          <label class="checkbox-row">
            <input
              type="checkbox"
              checked={settings().enable_dxvk_async}
              onChange={(e) => update("enable_dxvk_async", e.currentTarget.checked)}
            />
            Enable DXVK Async globally
          </label>
        </span>

        <div class="settings-section">Shared Prefix</div>
        <span class="settings-label" style={{ "grid-column": "1 / -1" }}>
          <label class="checkbox-row">
            <input
              type="checkbox"
              checked={settings().use_shared_prefix_default}
              onChange={(e) => update("use_shared_prefix_default", e.currentTarget.checked)}
            />
            Suggest shared prefix by default for new Wine/Proton games
          </label>
        </span>
        <span class="settings-label">Shared Prefix Path</span>
        <div class="input-group">
          <input
            type="text"
            value={settings().shared_prefix_path}
            onInput={(e) => update("shared_prefix_path", e.currentTarget.value)}
            placeholder="Leave empty for default (Prefixes/shared)"
          />
          <button onClick={browseSharedPrefix}>📁 Browse</button>
        </div>

        <div class="settings-section">Paths</div>
        <span class="settings-label">Prefixes</span>
        <span class="settings-path">{paths().prefixes_dir || "—"}</span>
        <span class="settings-label">Protons</span>
        <span class="settings-path">{paths().protons_dir || "—"}</span>
        <span class="settings-label">Logs</span>
        <span class="settings-path">{paths().logs_dir || "—"}</span>

        <div class="settings-section">Backup</div>
        <span class="settings-label">Export</span>
        <button onClick={handleExport} disabled={exporting()} style={{ "justify-self": "start" }}>
          {exporting() ? <span class="spinner" /> : "💾 Export Backup (games + settings)"}
        </button>
        <span class="settings-label">Import</span>
        <div style={{ display: "flex", gap: "8px" }}>
          <button onClick={() => handleImport(true)} disabled={importing()}>📥 Merge New Games</button>
          <button onClick={() => handleImport(false)} disabled={importing()} class="btn-danger">♻ Restore (Replace All)</button>
        </div>
      </div>
      <div style={{ "margin-top": "20px" }}>
        <button onClick={handleSave} disabled={saving()}>
          {saving() ? (
            <>
              <span class="spinner" /> Saving…
            </>
          ) : (
            "💾 Save Settings"
          )}
        </button>
      </div>
    </div>
  );
}
