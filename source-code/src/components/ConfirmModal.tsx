import { Show } from "solid-js";

export interface ConfirmRequest {
  title?: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  onConfirm: () => void;
}

interface Props {
  request: ConfirmRequest | null;
  onDismiss: () => void;
}

/**
 * A single, theme-consistent confirmation dialog used everywhere the app
 * used to call the browser's native `window.confirm()` (removing a game,
 * removing/updating a Proton version, force-removing a Proton still in use
 * by a game, etc). Callers just set a signal to a `ConfirmRequest` object;
 * this component renders itself when that signal is non-null.
 */
export default function ConfirmModal(props: Props) {
  const confirm = () => {
    const req = props.request;
    if (!req) return;
    props.onDismiss();
    req.onConfirm();
  };

  return (
    <Show when={props.request}>
      {(req) => (
        <div class="modal-overlay" onClick={() => props.onDismiss()}>
          <div class="modal" style={{ "min-width": "380px" }} onClick={(e) => e.stopPropagation()}>
            <h2>{req().title ?? "Please confirm"}</h2>
            <p style={{ color: "var(--text-secondary)", "line-height": "1.6", "font-size": "13px" }}>
              {req().message}
            </p>
            <div class="modal-actions">
              <button
                onClick={() => props.onDismiss()}
                style={{ background: "rgba(255,255,255,0.05)" }}
              >
                {req().cancelLabel ?? "Cancel"}
              </button>
              <button class={req().danger ? "btn-danger" : ""} onClick={confirm}>
                {req().confirmLabel ?? "Confirm"}
              </button>
            </div>
          </div>
        </div>
      )}
    </Show>
  );
}
