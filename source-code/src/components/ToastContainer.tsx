import { For, Show } from "solid-js";
import { Toast } from "../types";

interface Props {
  toasts: Toast[];
}

export default function ToastContainer(props: Props) {
  return (
    <Show when={props.toasts.length > 0}>
      <div class="toast-wrap">
        <For each={props.toasts}>
          {(t) => <div class={`toast ${t.kind}`}>{t.message}</div>}
        </For>
      </div>
    </Show>
  );
}
