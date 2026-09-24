import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  EVT_RESULT,
  EVT_STATE,
  type DictationResult,
  type ModelStatus,
  type StateEvent,
  type StatusSnapshot,
} from "./types";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const badge = $<HTMLSpanElement>("model-badge");
const modelHelp = $<HTMLElement>("model-help");
const errorBox = $<HTMLElement>("error");
const text = $<HTMLTextAreaElement>("text");
const copy = $<HTMLButtonElement>("copy");
const meta = $<HTMLParagraphElement>("meta");

function renderModel(model: ModelStatus) {
  const labels: Record<ModelStatus["status"], string> = {
    loading: "Chargement du modèle…",
    ready: "Prêt",
    missing: "Modèle absent",
    failed: "Erreur modèle",
  };
  badge.textContent = labels[model.status];
  badge.dataset.status = model.status;
  modelHelp.hidden = model.status !== "missing";
  if (model.status === "missing") $("model-path").textContent = model.path;
  if (model.status === "failed") renderError(model.message);
}

function renderError(message: string | null) {
  errorBox.hidden = !message;
  errorBox.textContent = message ?? "";
}

function renderResult(result: DictationResult) {
  text.value = result.text;
  copy.disabled = false;
  for (const dd of document.querySelectorAll<HTMLElement>("#timings dd")) {
    const key = dd.dataset.k as keyof DictationResult["timings"];
    dd.textContent = `${result.timings[key]} ms`;
  }
  meta.textContent =
    `${result.word_count} mots · ${result.audio_secs.toFixed(1)} s enregistrées ` +
    `dont ${result.voiced_secs.toFixed(1)} s de parole`;
}

async function refresh() {
  const snap = await invoke<StatusSnapshot>("get_status");
  renderModel(snap.model);
  if (snap.model.status !== "failed") renderError(snap.last_error);
  if (snap.last_result) renderResult(snap.last_result);
  const how = snap.settings.mode === "push_to_talk" ? "maintenez" : "appuyez une fois pour démarrer, une fois pour arrêter";
  $("hint").textContent = `Raccourci : ${snap.settings.shortcut} (${how}).`;
}

copy.addEventListener("click", async () => {
  await navigator.clipboard.writeText(text.value);
  copy.textContent = "Copié";
  setTimeout(() => (copy.textContent = "Copier"), 1200);
});

await listen<DictationResult>(EVT_RESULT, (e) => {
  renderError(null);
  renderResult(e.payload);
});
await listen<StateEvent>(EVT_STATE, () => {
  // L'état du modèle ou une erreur a pu changer : on relit l'instantané complet.
  void refresh();
});
// La fenêtre est cachée, pas détruite : on se resynchronise quand elle réapparaît.
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) void refresh();
});
await refresh();
