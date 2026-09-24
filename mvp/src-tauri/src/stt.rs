//! Transcription locale via whisper.cpp (Metal sur Apple Silicon).

use std::path::Path;

use thiserror::Error;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

use crate::audio::dsp::WHISPER_SAMPLE_RATE;

#[derive(Debug, Error)]
pub enum SttError {
    #[error("modèle introuvable : {0}")]
    ModelMissing(String),
    #[error("chargement du modèle impossible : {0}")]
    Load(String),
    #[error("échec de la transcription : {0}")]
    Inference(String),
}

pub struct Transcriber {
    // L'état garde une référence au contexte : pas besoin de stocker les deux.
    state: WhisperState,
    language: String,
    threads: i32,
    adaptive_context: bool,
}

impl Transcriber {
    pub fn load(model_path: &Path, language: &str) -> Result<Self, SttError> {
        if !model_path.is_file() {
            return Err(SttError::ModelMissing(model_path.display().to_string()));
        }
        let mut params = WhisperContextParameters::default();
        params.use_gpu(true).flash_attn(true);
        let ctx = WhisperContext::new_with_params(model_path, params).map_err(|e| SttError::Load(e.to_string()))?;
        let state = ctx.create_state().map_err(|e| SttError::Load(e.to_string()))?;

        let threads = std::thread::available_parallelism()
            .map(|n| n.get().min(8) as i32)
            .unwrap_or(4);
        let mut transcriber = Self {
            state,
            language: language.to_owned(),
            threads,
            adaptive_context: true,
        };
        transcriber.warm_up();
        Ok(transcriber)
    }

    /// Désactive la fenêtre d'encodeur adaptée à la durée (comparaison qualité/latence).
    pub fn set_adaptive_context(&mut self, enabled: bool) {
        self.adaptive_context = enabled;
    }

    /// La première inférence compile les shaders Metal et alloue les buffers :
    /// on la paie au démarrage plutôt qu'à la première dictée.
    fn warm_up(&mut self) {
        let silence = vec![0.0f32; WHISPER_SAMPLE_RATE as usize];
        if let Err(e) = self.run(&silence) {
            tracing::warn!("préchauffage whisper échoué : {e}");
        }
    }

    /// `samples` : mono, 16 kHz, valeurs dans [-1, 1].
    pub fn transcribe(&mut self, samples: &[f32]) -> Result<String, SttError> {
        let raw = self.run(samples)?;
        Ok(clean_transcript(&raw))
    }

    fn run(&mut self, samples: &[f32]) -> Result<String, SttError> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(&self.language));
        params.set_n_threads(self.threads);
        params.set_no_context(true);
        params.set_no_timestamps(true);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_temperature(0.0);
        if self.adaptive_context {
            params.set_audio_ctx(audio_ctx_for(samples.len()));
        }
        // Ces options imprimeraient le texte sur la sortie standard : interdit.
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);

        self.state
            .full(params, samples)
            .map_err(|e| SttError::Inference(e.to_string()))?;

        let mut text = String::new();
        for segment in self.state.as_iter() {
            let part = segment.to_str_lossy().map_err(|e| SttError::Inference(e.to_string()))?;
            text.push_str(&part);
            text.push(' ');
        }
        Ok(text)
    }
}

/// Nombre de trames d'encodeur (1500 = 30 s) couvrant `n_samples` à 16 kHz,
/// avec une marge. Whisper encode sinon toujours 30 s, silence compris : c'est
/// le coût dominant pour une dictée courte.
fn audio_ctx_for(n_samples: usize) -> i32 {
    const FULL_CTX: usize = 1500;
    const FRAMES_PER_SEC: usize = FULL_CTX / 30;
    const MARGIN_FRAMES: usize = 64;
    let needed = n_samples.div_ceil(WHISPER_SAMPLE_RATE as usize / FRAMES_PER_SEC) + MARGIN_FRAMES;
    needed.min(FULL_CTX) as i32
}

/// Phrases que Whisper produit sur du silence ou du bruit en français
/// (héritées des sous-titres de son corpus d'entraînement).
const HALLUCINATIONS: &[&str] = &[
    "sous-titres réalisés par",
    "sous-titrage st'",
    "sous-titrage société radio-canada",
    "amara.org",
    "merci d'avoir regardé",
    "abonnez-vous",
];

/// Normalise les espaces et écarte une transcription entièrement hallucinée.
pub fn clean_transcript(raw: &str) -> String {
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = text.to_lowercase();
    let short = text.split_whitespace().count() <= 12;
    if short && HALLUCINATIONS.iter().any(|h| lower.contains(h)) {
        return String::new();
    }
    text
}

pub fn word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_whitespace_between_segments() {
        assert_eq!(clean_transcript("  Bonjour,  \n comment ça va ?  "), "Bonjour, comment ça va ?");
    }

    #[test]
    fn drops_known_hallucinations() {
        assert_eq!(clean_transcript(" Sous-titres réalisés par la communauté d'Amara.org "), "");
        assert_eq!(clean_transcript("Merci d'avoir regardé cette vidéo !"), "");
    }

    #[test]
    fn keeps_real_dictation_mentioning_similar_words() {
        let long = "Je voulais vous dire merci d'avoir regardé le dossier hier soir, nous en reparlerons à la réunion de jeudi prochain";
        assert_eq!(clean_transcript(long), long);
    }

    #[test]
    fn audio_ctx_scales_with_duration_and_caps() {
        let sr = WHISPER_SAMPLE_RATE as usize;
        assert_eq!(audio_ctx_for(10 * sr), 500 + 64);
        assert_eq!(audio_ctx_for(sr / 2), 25 + 64);
        assert_eq!(audio_ctx_for(40 * sr), 1500);
    }

    #[test]
    fn counts_words_ignoring_punctuation() {
        assert_eq!(word_count("Bonjour , je suis là !"), 4);
        assert_eq!(word_count(""), 0);
    }
}
