import { createSignal, onMount, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { ControllerInfo, ControllerInputEvent, Toast } from "../types";

interface Props {
  addToast: (msg: string, kind?: Toast["kind"]) => void;
  onClose: () => void;
  /** If provided, "Use This Mapping" calls this instead of just copying to
   * the clipboard (used when opened from a game's Configure dialog). */
  onGenerated?: (config: string) => void;
}

const CONTROLS: Array<{ key: string; label: string }> = [
  { key: "a", label: "A / Cross" },
  { key: "b", label: "B / Circle" },
  { key: "x", label: "X / Square" },
  { key: "y", label: "Y / Triangle" },
  { key: "back", label: "Back / Select" },
  { key: "guide", label: "Guide / Home" },
  { key: "start", label: "Start" },
  { key: "leftshoulder", label: "Left Shoulder (LB)" },
  { key: "rightshoulder", label: "Right Shoulder (RB)" },
  { key: "leftstick", label: "Left Stick Click (L3)" },
  { key: "rightstick", label: "Right Stick Click (R3)" },
  { key: "dpup", label: "D-Pad Up" },
  { key: "dpdown", label: "D-Pad Down" },
  { key: "dpleft", label: "D-Pad Left" },
  { key: "dpright", label: "D-Pad Right" },
  { key: "leftx", label: "Left Stick X-axis" },
  { key: "lefty", label: "Left Stick Y-axis" },
  { key: "rightx", label: "Right Stick X-axis" },
  { key: "righty", label: "Right Stick Y-axis" },
  { key: "lefttrigger", label: "Left Trigger (LT/L2)" },
  { key: "righttrigger", label: "Right Trigger (RT/R2)" },
];

function eventToCode(e: ControllerInputEvent): string {
  // Linux joystick buttons map straight to SDL's "bN". Axes map to "aN" —
  // note some pads report the D-Pad as a hat rather than buttons, which SDL
  // actually represents as "hH.V"; this wizard always emits "aN" for axis
  // captures for simplicity, so double-check D-Pad entries manually if your
  // pad reports it as a hat instead of four buttons.
  return e.kind === "button" ? `b${e.number}` : `a${e.number}`;
}

export default function ControllerMappingWizard(props: Props) {
  const [controllers, setControllers] = createSignal<ControllerInfo[]>([]);
  const [selectedHandler, setSelectedHandler] = createSignal("");
  const [deviceName, setDeviceName] = createSignal("Custom Controller");
  const [guid, setGuid] = createSignal("");
  const [loadingGuid, setLoadingGuid] = createSignal(false);
  const [mapping, setMapping] = createSignal<Record<string, string>>({});
  const [capturingKey, setCapturingKey] = createSignal<string | null>(null);

  onMount(async () => {
    try {
      const list = await invoke<ControllerInfo[]>("list_controllers");
      setControllers(list);
      if (list.length > 0) {
        setSelectedHandler(list[0].handler);
        setDeviceName(list[0].name);
        await loadGuid(list[0].handler);
      }
    } catch (e) {
      props.addToast(`Failed to list controllers: ${e}`, "error");
    }
  });

  const loadGuid = async (handler: string) => {
    setLoadingGuid(true);
    setGuid("");
    try {
      const g = await invoke<string>("get_controller_guid", { handler });
      setGuid(g);
    } catch (e) {
      props.addToast(`Couldn't auto-detect GUID: ${e}. You can still enter it manually.`, "info");
    } finally {
      setLoadingGuid(false);
    }
  };

  const onSelectController = async (handler: string) => {
    setSelectedHandler(handler);
    const c = controllers().find((c) => c.handler === handler);
    if (c) setDeviceName(c.name);
    await loadGuid(handler);
  };

  const capture = async (key: string) => {
    if (!selectedHandler()) return props.addToast("Select a controller first", "error");
    setCapturingKey(key);
    try {
      const event = await invoke<ControllerInputEvent>("capture_controller_input", {
        handler: selectedHandler(),
        timeoutMs: 6000,
      });
      setMapping((m) => ({ ...m, [key]: eventToCode(event) }));
    } catch (e) {
      props.addToast(`${e}`, "error");
    } finally {
      setCapturingKey(null);
    }
  };

  const configString = () => {
    const entries = CONTROLS.filter((c) => mapping()[c.key]?.trim())
      .map((c) => `${c.key}:${mapping()[c.key].trim()}`)
      .join(",");
    if (!entries) return "";
    return `${guid() || "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"},${deviceName() || "Custom Controller"},${entries},platform:Linux`;
  };

  const useMapping = () => {
    const cfg = configString();
    if (!cfg) return props.addToast("Map at least one control first", "error");
    if (props.onGenerated) {
      props.onGenerated(cfg);
      props.onClose();
    } else {
      navigator.clipboard.writeText(cfg).then(
        () => props.addToast("Mapping copied to clipboard!", "success"),
        () => props.addToast("Couldn't copy to clipboard", "error")
      );
    }
  };

  return (
    <div class="modal-overlay" onClick={props.onClose}>
      <div class="modal" style={{ "min-width": "620px" }} onClick={(e) => e.stopPropagation()}>
        <h2>SDL Controller Mapping Wizard</h2>
        <p style={{ color: "var(--text-muted)", "font-size": "12px" }}>
          Reads raw Linux joystick input directly — no need to know button/axis numbers by heart.
          For each control, click Capture and press the corresponding button (or move the stick/trigger)
          on your pad within 6 seconds. This is a best-effort helper: D-Pads reported as a "hat" rather
          than four buttons, and the auto-detected GUID, may need manual touch-ups.
        </p>

        <div class="form-grid">
          <label class="form-label">Controller</label>
          <Show when={controllers().length > 0} fallback={<span style={{ color: "var(--text-muted)" }}>No controllers detected</span>}>
            <select value={selectedHandler()} onChange={(e) => onSelectController(e.currentTarget.value)}>
              <For each={controllers()}>{(c) => <option value={c.handler}>{c.name} (/dev/input/{c.handler})</option>}</For>
            </select>
          </Show>

          <label class="form-label">Mapping Name</label>
          <input type="text" value={deviceName()} onInput={(e) => setDeviceName(e.currentTarget.value)} />

          <label class="form-label">GUID</label>
          <div class="input-group">
            <input type="text" value={guid()} onInput={(e) => setGuid(e.currentTarget.value)} placeholder={loadingGuid() ? "Detecting…" : "32 hex chars"} />
            <button onClick={() => selectedHandler() && loadGuid(selectedHandler())} disabled={!selectedHandler() || loadingGuid()}>↺</button>
          </div>
        </div>

        <div style={{ display: "grid", "grid-template-columns": "1fr auto auto", gap: "6px 10px", "align-items": "center", "margin-top": "10px", "max-height": "280px", overflow: "auto" }}>
          <For each={CONTROLS}>
            {(c) => (
              <>
                <span style={{ "font-size": "12px" }}>{c.label}</span>
                <span style={{ "font-family": "monospace", "font-size": "12px", color: "var(--text-muted)", "min-width": "40px", "text-align": "center" }}>
                  {mapping()[c.key] ?? "—"}
                </span>
                <button
                  style={{ "font-size": "11px", padding: "4px 10px" }}
                  onClick={() => capture(c.key)}
                  disabled={capturingKey() !== null || !selectedHandler()}
                >
                  {capturingKey() === c.key ? (<><span class="spinner" /> Press now…</>) : "🎯 Capture"}
                </button>
              </>
            )}
          </For>
        </div>

        <Show when={configString()}>
          <div style={{ "margin-top": "10px" }}>
            <div class="form-label" style={{ "margin-bottom": "4px" }}>Generated config</div>
            <pre style={{
              background: "rgba(12,12,24,0.8)", border: "1px solid var(--border-subtle)",
              "border-radius": "8px", padding: "10px", "max-height": "80px", overflow: "auto",
              "font-size": "11px", "white-space": "pre-wrap", "word-break": "break-all", color: "#c4c4d4",
            }}>{configString()}</pre>
          </div>
        </Show>

        <div class="modal-actions">
          <button onClick={props.onClose} style={{ background: "rgba(255,255,255,0.05)" }}>Close</button>
          <button onClick={useMapping} disabled={!configString()}>
            {props.onGenerated ? "✅ Use This Mapping" : "📋 Copy to Clipboard"}
          </button>
        </div>
      </div>
    </div>
  );
}
