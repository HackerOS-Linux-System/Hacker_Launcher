import { createSignal, onMount, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { ControllerInfo, Toast } from "../types";
import ControllerMappingWizard from "./ControllerMappingWizard";

interface Props {
  addToast: (msg: string, kind?: Toast["kind"]) => void;
}

export default function ControllersTab(props: Props) {
  const [controllers, setControllers] = createSignal<ControllerInfo[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [showWizard, setShowWizard] = createSignal(false);

  const load = async () => {
    setLoading(true);
    try {
      const c = await invoke<ControllerInfo[]>("list_controllers");
      setControllers(c);
    } catch (e) {
      props.addToast(`Failed to list controllers: ${e}`, "error");
    } finally {
      setLoading(false);
    }
  };

  onMount(load);

  return (
    <>
      <div class="section-label">Controllers</div>
      <p style={{ color: "var(--text-muted)", "font-size": "12px" }}>
        Devices currently visible to the kernel. This is informational only — it confirms Linux sees
        the pad at all, it isn't a full remapper. Use the mapping wizard below to build an
        <code>SDL_GAMECONTROLLERCONFIG</code> value by pressing buttons, then paste it into a game's
        Configure dialog (or build it directly from there, which fills the field for you).
      </p>
      <div class="table-wrap">
        <Show
          when={!loading()}
          fallback={<div class="empty-state"><span class="spinner" /><span>Detecting controllers…</span></div>}
        >
          <Show
            when={controllers().length > 0}
            fallback={<div class="empty-state"><span>🎮</span><span>No game controllers detected</span></div>}
          >
            <table>
              <thead><tr><th>Name</th><th>Device</th></tr></thead>
              <tbody>
                <For each={controllers()}>
                  {(c) => (
                    <tr>
                      <td>{c.name}</td>
                      <td style={{ color: "var(--text-muted)", "font-family": "monospace" }}>/dev/input/{c.handler}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Show>
        </Show>
      </div>
      <div class="btn-row">
        <button onClick={load} disabled={loading()}>↺ Refresh</button>
        <button onClick={() => setShowWizard(true)}>🛠 Open Mapping Wizard</button>
      </div>

      <Show when={showWizard()}>
        <ControllerMappingWizard addToast={props.addToast} onClose={() => setShowWizard(false)} />
      </Show>
    </>
  );
}
