import { listen } from "@tauri-apps/api/event";
import { EVT_STATE, type StateEvent } from "./types";

const pill = document.getElementById("pill") as HTMLDivElement;
const label = document.getElementById("label") as HTMLSpanElement;
let hideTimer: number | undefined;

function show(state: string, text: string, autoHideMs?: number) {
  window.clearTimeout(hideTimer);
  pill.dataset.state = state;
  label.textContent = text;
  if (autoHideMs) hideTimer = window.setTimeout(() => (pill.dataset.state = "hidden"), autoHideMs);
}

await listen<StateEvent>(EVT_STATE, ({ payload }) => {
  switch (payload.phase) {
    case "recording":
      show("recording", "Écoute…");
      break;
    case "processing":
      show("processing", payload.message ?? "Transcription…");
      break;
    case "idle":
      if (payload.message) show("notice", payload.message, 3500);
      else pill.dataset.state = "hidden";
      break;
  }
});
