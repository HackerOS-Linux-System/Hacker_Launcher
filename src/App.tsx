import { createSignal, onMount, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import GamesTab from "./components/GamesTab";
import ProtonsTab from "./components/ProtonsTab";
import ControllersTab from "./components/ControllersTab";
import SettingsTab, { applyTheme } from "./components/SettingsTab";
import AboutTab from "./components/AboutTab";
import ToastContainer from "./components/ToastContainer";
import { Toast, Settings } from "./types";

type Tab = "Games" | "Protons" | "Controllers" | "Settings" | "About";
const TABS: Tab[] = ["Games", "Protons", "Controllers", "Settings", "About"];

export default function App() {
  const [activeTab, setActiveTab] = createSignal<Tab>("Games");
  const [toasts, setToasts] = createSignal<Toast[]>([]);
  // Read once at startup: changing "Library View" in Settings takes effect
  // after restarting the app, rather than reflowing the list mid-session.
  const [libraryView, setLibraryView] = createSignal<"List" | "Grid">("List");

  const addToast = (message: string, kind: Toast["kind"] = "info") => {
    const id = Date.now();
    setToasts((t) => [...t, { id, message, kind }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 4000);
  };

  onMount(() => {
    invoke<Settings>("get_settings")
      .then((s) => {
        applyTheme(s.theme);
        setLibraryView(s.library_view === "Grid" ? "Grid" : "List");
      })
      .catch(() => {});
  });

  return (
    <div class="app">
      <nav class="tab-bar">
        <For each={TABS}>
          {(tab) => (
            <button
              class={`tab-btn ${activeTab() === tab ? "active" : ""}`}
              onClick={() => setActiveTab(tab)}
            >
              {tab}
            </button>
          )}
        </For>
      </nav>

      <div class="tab-content">
        <Show when={activeTab() === "Games"}><GamesTab addToast={addToast} libraryView={libraryView()} /></Show>
        <Show when={activeTab() === "Protons"}><ProtonsTab addToast={addToast} /></Show>
        <Show when={activeTab() === "Controllers"}><ControllersTab addToast={addToast} /></Show>
        <Show when={activeTab() === "Settings"}><SettingsTab addToast={addToast} /></Show>
        <Show when={activeTab() === "About"}><AboutTab /></Show>
      </div>

      <ToastContainer toasts={toasts()} />
    </div>
  );
}
