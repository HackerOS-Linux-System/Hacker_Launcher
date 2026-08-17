import { createSignal, createMemo, createEffect, onMount, onCleanup, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { ProtonEntry, Toast } from "../types";
import ConfirmModal, { ConfirmRequest } from "./ConfirmModal";

interface Props {
  addToast: (msg: string, kind?: Toast["kind"]) => void;
}

interface ProgressEvent {
  stage: string;
  value: number;
  total: number;
}

type ProtonType = "GE" | "Official" | "Experimental" | "Custom";

export default function ProtonsTab(props: Props) {
  const [protons, setProtons] = createSignal<ProtonEntry[]>([]);
  const [selected, setSelected] = createSignal(-1);
  const [loading, setLoading] = createSignal(false);
  const [showInstall, setShowInstall] = createSignal(false);
  const [protonType, setProtonType] = createSignal<ProtonType>("GE");
  const [availableVersions, setAvailableVersions] = createSignal<string[]>([]);
  const [versionFilter, setVersionFilter] = createSignal("");
  const [selectedVersion, setSelectedVersion] = createSignal("");
  const [customPath, setCustomPath] = createSignal("");
  const [customVersionName, setCustomVersionName] = createSignal("");
  const [customSourceType, setCustomSourceType] = createSignal<"Tar.gz File" | "Folder">("Tar.gz File");
  const [fetchingVersions, setFetchingVersions] = createSignal(false);
  const [installing, setInstalling] = createSignal(false);
  const [installingVersion, setInstallingVersion] = createSignal("");
  const [progressStage, setProgressStage] = createSignal("");
  const [progressPct, setProgressPct] = createSignal(0);
  const [confirmReq, setConfirmReq] = createSignal<ConfirmRequest | null>(null);
  const [releaseNotes, setReleaseNotes] = createSignal("");
  const [loadingNotes, setLoadingNotes] = createSignal(false);
  const [showNotes, setShowNotes] = createSignal(false);

  const loadProtons = async () => {
    setLoading(true);
    try {
      const p = await invoke<ProtonEntry[]>("get_installed_protons");
      setProtons(p);
    } catch (e) {
      props.addToast(`Failed to load protons: ${e}`, "error");
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    loadProtons();
    const unlisten = listen<ProgressEvent>("proton_progress", (event) => {
      const { stage, value, total } = event.payload;
      setProgressStage(stage);
      setProgressPct(total > 0 ? Math.round((value / total) * 100) : 0);
    });
    onCleanup(() => { unlisten.then((f) => f()); });
  });

  const fetchReleaseNotes = async (version: string, type: ProtonType) => {
    if (!version || type === "Custom") {
      setReleaseNotes("");
      return;
    }
    setLoadingNotes(true);
    try {
      const notes = await invoke<string>("get_release_notes", {
        version,
        protonType: type === "Experimental" ? "Official" : type,
      });
      setReleaseNotes(notes);
    } catch (e) {
      setReleaseNotes(`(Could not load release notes: ${e})`);
    } finally {
      setLoadingNotes(false);
    }
  };

  createEffect(() => {
    if (showInstall() && selectedVersion()) {
      fetchReleaseNotes(selectedVersion(), protonType());
    }
  });

  const filteredVersions = createMemo(() => {
    const q = versionFilter().trim().toLowerCase();
    if (!q) return availableVersions();
    return availableVersions().filter((v) => v.toLowerCase().includes(q));
  });

  const fetchVersions = async (type: ProtonType) => {
    if (type === "Custom") { setAvailableVersions([]); return; }
    setFetchingVersions(true);
    try {
      let versions: string[];
      if (type === "GE") versions = await invoke<string[]>("get_available_ge");
      else if (type === "Official") versions = await invoke<string[]>("get_available_official", { stable: true });
      else versions = await invoke<string[]>("get_available_official", { stable: false });
      setAvailableVersions(versions);
      if (versions.length > 0) setSelectedVersion(versions[0]);
      else setSelectedVersion("");
    } catch (e) {
      props.addToast(`Failed to fetch versions: ${e}`, "error");
      setAvailableVersions([]);
    } finally {
      setFetchingVersions(false);
    }
  };

  const openInstallDialog = () => {
    setProtonType("GE");
    setAvailableVersions([]);
    setVersionFilter("");
    setSelectedVersion("");
    setCustomPath("");
    setCustomVersionName("");
    setCustomSourceType("Tar.gz File");
    setShowInstall(true);
    fetchVersions("GE");
  };

  const handleTypeChange = (t: ProtonType) => {
    setProtonType(t);
    setVersionFilter("");
    fetchVersions(t);
  };

  const browseCustomPath = async () => {
    if (customSourceType() === "Tar.gz File") {
      const result = await open({ filters: [{ name: "Tar.gz", extensions: ["gz"] }] });
      if (typeof result === "string") {
        setCustomPath(result);
        if (!customVersionName()) setCustomVersionName(result.split("/").pop()?.replace(".tar.gz", "") ?? "");
      }
    } else {
      const result = await open({ directory: true });
      if (typeof result === "string") {
        setCustomPath(result);
        if (!customVersionName()) setCustomVersionName(result.split("/").pop() ?? "");
      }
    }
  };

  const handleInstall = async () => {
    if (protonType() === "Custom") {
      if (!customVersionName() || !customPath()) return props.addToast("Name and path are required", "error");
    } else if (!selectedVersion()) {
      return props.addToast("Select a version", "error");
    }
    setInstalling(true);
    setProgressPct(0);
    setProgressStage("Starting…");
    const versionForInstall = protonType() === "Custom" ? customVersionName() : selectedVersion();
    setInstallingVersion(versionForInstall);
    try {
      if (protonType() === "Custom") {
        if (customSourceType() === "Tar.gz File") {
          await invoke("install_custom_tar", { tarPath: customPath(), version: customVersionName() });
        } else {
          await invoke("install_custom_folder", { srcFolder: customPath(), version: customVersionName() });
        }
        props.addToast(`Custom Proton "${customVersionName()}" installed!`, "success");
      } else {
        await invoke("install_proton", { version: selectedVersion(), protonType: protonType() });
        props.addToast(`Proton ${selectedVersion()} installed!`, "success");
      }
      setShowInstall(false);
      await loadProtons();
    } catch (e) {
      props.addToast(`Installation failed: ${e}`, "error");
    } finally {
      setInstalling(false);
      setInstallingVersion("");
    }
  };

  const handleCancelInstall = async () => {
    if (!installingVersion()) return;
    try {
      await invoke("cancel_proton_install", { version: installingVersion() });
      props.addToast("Cancelling…", "info");
    } catch (e) {
      props.addToast(`Failed to cancel: ${e}`, "error");
    }
  };

  const doRemove = async (version: string, force: boolean) => {
    try {
      await invoke("remove_proton", { version, force });
      setSelected(-1);
      props.addToast(`Removed: ${version}`, "success");
      await loadProtons();
    } catch (e) {
      const msg = String(e);
      if (msg.startsWith("IN_USE:")) {
        const rest = msg.slice("IN_USE:".length);
        const sepIdx = rest.indexOf(":");
        const usedBy = sepIdx >= 0 ? rest.slice(sepIdx + 1) : rest;
        setConfirmReq({
          title: "Proton In Use",
          message: `"${version}" is still used by: ${usedBy}. Removing it will break those games until reconfigured. Remove anyway?`,
          confirmLabel: "Remove Anyway",
          danger: true,
          onConfirm: () => doRemove(version, true),
        });
      } else {
        props.addToast(`Failed to remove: ${msg}`, "error");
      }
    }
  };

  const handleRemove = () => {
    if (selected() < 0) return props.addToast("No Proton selected", "error");
    const p = protons()[selected()];
    setConfirmReq({
      title: "Remove Proton",
      message: `Remove Proton "${p.version}"? This will delete the installed files.`,
      confirmLabel: "Remove",
      danger: true,
      onConfirm: () => doRemove(p.version, false),
    });
  };

  const handleUpdate = async () => {
    if (selected() < 0) return props.addToast("No Proton selected", "error");
    const p = protons()[selected()];
    props.addToast("Checking for updates…", "info");
    try {
      const result = await invoke<[string, string] | null>("check_proton_update", { version: p.version, protonType: p.type });
      if (!result) return props.addToast("No update available", "info");
      const [newType, newVersion] = result;
      setConfirmReq({
        title: "Update Proton",
        message: `Update "${p.version}" to ${newVersion} (${newType})?`,
        confirmLabel: "Update",
        onConfirm: async () => {
          setInstalling(true);
          setProgressPct(0);
          setProgressStage("Starting update…");
          setInstallingVersion(newVersion);
          try {
            await invoke("install_proton", { version: newVersion, protonType: newType });
            await invoke("remove_proton", { version: p.version, force: true });
            props.addToast(`Updated to ${newVersion}`, "success");
            setSelected(-1);
            await loadProtons();
          } catch (e) {
            props.addToast(`Update failed: ${e}`, "error");
          } finally {
            setInstalling(false);
            setInstallingVersion("");
          }
        },
      });
    } catch (e) {
      props.addToast(`Update failed: ${e}`, "error");
    }
  };

  return (
    <>
      <div class="section-label">Installed Protons</div>
      <div class="table-wrap">
        <Show
          when={!loading()}
          fallback={<div class="empty-state"><span class="spinner" /><span>Loading protons…</span></div>}
        >
          <Show
            when={protons().length > 0}
            fallback={<div class="empty-state"><span>📦</span><span>No Proton versions installed</span></div>}
          >
            <table>
              <thead><tr><th>Version</th><th>Type</th><th>Installed Date</th><th>Status</th></tr></thead>
              <tbody>
                <For each={protons()}>
                  {(p, i) => (
                    <tr class={selected() === i() ? "selected" : ""} onClick={() => setSelected(i())}>
                      <td>{p.version}</td>
                      <td><span class="badge badge-purple">{p.type}</span></td>
                      <td style={{ color: "var(--text-muted)" }}>{p.date}</td>
                      <td><span class={`badge ${p.status === "Update Available" ? "badge-yellow" : "badge-green"}`}>{p.status}</span></td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Show>
        </Show>
      </div>
      <Show when={installing()}>
        <div style={{ "flex-shrink": 0 }}>
          <div class="progress-wrap"><div class="progress-bar" style={{ width: `${progressPct()}%` }} /></div>
          <div class="progress-label">{progressStage()} — {progressPct()}%</div>
        </div>
      </Show>
      <div class="btn-row">
        <button onClick={openInstallDialog} disabled={installing()}>＋ Install Proton</button>
        <button onClick={handleUpdate} disabled={selected() < 0 || installing()}>↑ Update Selected</button>
        <button class="btn-danger" onClick={handleRemove} disabled={selected() < 0 || installing()}>✕ Remove Selected</button>
        <button onClick={loadProtons} disabled={loading()}>↺ Refresh</button>
      </div>

      <Show when={showInstall()}>
        <div class="modal-overlay" onClick={() => !installing() && setShowInstall(false)}>
          <div class="modal" onClick={(e) => e.stopPropagation()}>
            <h2>Install Proton</h2>
            <div class="form-grid">
              <label class="form-label">Proton Type</label>
              <select value={protonType()} onChange={(e) => handleTypeChange(e.currentTarget.value as ProtonType)}>
                <option value="GE">GE (GloriousEggroll)</option>
                <option value="Official">Official (Stable)</option>
                <option value="Experimental">Official (Experimental)</option>
                <option value="Custom">Custom</option>
              </select>
              <Show
                when={protonType() !== "Custom"}
                fallback={
                  <>
                    <label class="form-label">Source Type</label>
                    <select value={customSourceType()} onChange={(e) => setCustomSourceType(e.currentTarget.value as "Tar.gz File" | "Folder")}>
                      <option>Tar.gz File</option>
                      <option>Folder</option>
                    </select>
                    <label class="form-label">Path</label>
                    <div class="input-group">
                      <input type="text" value={customPath()} onInput={(e) => setCustomPath(e.currentTarget.value)} placeholder="Select source…" />
                      <button onClick={browseCustomPath}>📁</button>
                    </div>
                    <label class="form-label">Version Name</label>
                    <input type="text" value={customVersionName()} onInput={(e) => setCustomVersionName(e.currentTarget.value)} placeholder="e.g. MyProton-8.0" />
                  </>
                }
              >
                <label class="form-label">Version</label>
                <Show
                  when={!fetchingVersions()}
                  fallback={<div style={{ display: "flex", gap: "8px", "align-items": "center" }}><span class="spinner" /><span style={{ color: "var(--text-muted)" }}>Fetching versions…</span></div>}
                >
                  <select value={selectedVersion()} onChange={(e) => setSelectedVersion(e.currentTarget.value)}>
                    <Show when={filteredVersions().length > 0} fallback={<option>No versions available</option>}>
                      <For each={filteredVersions()}>{(v) => <option>{v}</option>}</For>
                    </Show>
                  </select>
                </Show>
                <label class="form-label">Filter</label>
                <input
                  type="text"
                  value={versionFilter()}
                  onInput={(e) => setVersionFilter(e.currentTarget.value)}
                  placeholder={`Filter ${availableVersions().length} versions…`}
                />
                <span></span>
                <button style={{ "font-size": "11px", "justify-self": "start" }} onClick={() => setShowNotes(!showNotes())}>
                  {showNotes() ? "▾ Hide changelog" : "▸ Show changelog"}
                </button>
              </Show>
            </div>
            <Show when={protonType() !== "Custom" && showNotes()}>
              <Show
                when={!loadingNotes()}
                fallback={<div style={{ display: "flex", gap: "8px", "align-items": "center" }}><span class="spinner" /><span style={{ color: "var(--text-muted)" }}>Loading release notes…</span></div>}
              >
                <pre style={{
                  background: "rgba(12,12,24,0.8)", border: "1px solid var(--border-subtle)",
                  "border-radius": "8px", padding: "10px", "max-height": "200px", overflow: "auto",
                  "font-size": "11px", "white-space": "pre-wrap", color: "#c4c4d4",
                }}>{releaseNotes()}</pre>
              </Show>
            </Show>
            <Show when={installing()}>
              <div>
                <div class="progress-wrap"><div class="progress-bar" style={{ width: `${progressPct()}%` }} /></div>
                <div class="progress-label">{progressStage()} — {progressPct()}%</div>
              </div>
            </Show>
            <div class="modal-actions">
              <Show
                when={!installing()}
                fallback={<button class="btn-danger" onClick={handleCancelInstall}>✕ Cancel Install</button>}
              >
                <button onClick={() => setShowInstall(false)} style={{ background: "rgba(255,255,255,0.05)" }}>Cancel</button>
                <button onClick={handleInstall} disabled={fetchingVersions()}>⬇ Install</button>
              </Show>
            </div>
          </div>
        </div>
      </Show>

      <ConfirmModal request={confirmReq()} onDismiss={() => setConfirmReq(null)} />
    </>
  );
}
