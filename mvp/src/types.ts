// Miroir des structures sérialisées par src-tauri/src/pipeline.rs.

export type Phase = "idle" | "recording" | "processing";

export interface StateEvent {
  phase: Phase;
  message: string | null;
}

export interface Timings {
  capture_stop_ms: number;
  preprocess_ms: number;
  transcription_ms: number;
  total_ms: number;
}

export interface DictationResult {
  text: string;
  word_count: number;
  audio_secs: number;
  voiced_secs: number;
  timings: Timings;
}

export type ModelStatus =
  | { status: "loading" }
  | { status: "ready"; load_ms: number }
  | { status: "missing"; path: string }
  | { status: "failed"; message: string };

export interface Settings {
  shortcut: string;
  mode: "push_to_talk" | "toggle";
  whisper_model: string;
  language: string;
}

export interface StatusSnapshot {
  phase: Phase;
  model: ModelStatus;
  settings: Settings;
  last_result: DictationResult | null;
  last_error: string | null;
}

export const EVT_STATE = "dictation-state";
export const EVT_RESULT = "dictation-result";
