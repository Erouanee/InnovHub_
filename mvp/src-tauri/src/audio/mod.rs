//! Capture micro et prétraitement. L'audio ne quitte jamais la mémoire :
//! aucun chemin de ce module n'écrit sur disque.

pub mod dsp;
mod recorder;

pub use recorder::{list_input_devices, start_recording, ActiveRecording, AudioError, CapturedAudio};
