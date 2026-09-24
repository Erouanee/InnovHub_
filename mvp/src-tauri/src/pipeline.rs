//! Orchestration : raccourci → capture → prétraitement → transcription → UI.
//!
//! - `Controller` (état partagé Tauri) réagit aux touches ; il ne fait que des
//!   opérations courtes (ouvrir/fermer le micro).
//! - Un thread « worker » possède le modèle Whisper et traite les dictées une
//!   par une. Le modèle est chargé une seule fois, au démarrage.

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use zeroize::Zeroizing;

use crate::audio::dsp::{self, WHISPER_SAMPLE_RATE};
use crate::audio::{self, ActiveRecording, CapturedAudio};
use crate::dictation::{self, Action, KeyEvent, Phase};
use crate::settings::Settings;
use crate::stt::{self, Transcriber};
use crate::ui;

pub const EVT_STATE: &str = "dictation-state";
pub const EVT_RESULT: &str = "dictation-result";

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ModelStatus {
    Loading,
    Ready { load_ms: u64 },
    Missing { path: String },
    Failed { message: String },
}

/// Latences mesurées pour chaque étape, en millisecondes.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Timings {
    /// Fermeture du micro après le relâchement de la touche.
    pub capture_stop_ms: u64,
    /// Rééchantillonnage + détection de parole.
    pub preprocess_ms: u64,
    pub transcription_ms: u64,
    /// Du relâchement de la touche au texte prêt.
    pub total_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DictationResult {
    pub text: String,
    pub word_count: usize,
    pub audio_secs: f32,
    pub voiced_secs: f32,
    pub timings: Timings,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateEvent {
    pub phase: Phase,
    /// Message court pour l'indicateur (jamais de contenu dicté).
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusSnapshot {
    pub phase: Phase,
    pub model: ModelStatus,
    pub settings: Settings,
    pub last_result: Option<DictationResult>,
    pub last_error: Option<String>,
}

struct Job {
    audio: CapturedAudio,
    released_at: Instant,
    capture_stop: Duration,
}

struct Inner {
    phase: Phase,
    recording: Option<ActiveRecording>,
    /// Incrémenté à chaque enregistrement, pour que le minuteur de durée
    /// maximale n'arrête jamais une session plus récente.
    session: u64,
    model: ModelStatus,
    last_result: Option<DictationResult>,
    last_error: Option<String>,
}

pub struct Controller {
    app: AppHandle,
    settings: Settings,
    inner: Mutex<Inner>,
    jobs: mpsc::Sender<Job>,
}

impl Controller {
    /// Crée le contrôleur et lance le chargement du modèle en arrière-plan.
    pub fn start(app: AppHandle, settings: Settings, model_path: PathBuf) -> Arc<Self> {
        let (jobs, rx) = mpsc::channel();
        let controller = Arc::new(Self {
            app,
            settings,
            inner: Mutex::new(Inner {
                phase: Phase::Idle,
                recording: None,
                session: 0,
                model: ModelStatus::Loading,
                last_result: None,
                last_error: None,
            }),
            jobs,
        });
        let worker = Arc::clone(&controller);
        let spawned = std::thread::Builder::new()
            .name("stt-worker".into())
            .spawn(move || worker.run_worker(model_path, rx));
        if let Err(e) = spawned {
            controller.lock().model = ModelStatus::Failed {
                message: format!("thread de transcription non démarré : {e}"),
            };
        }
        controller
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn snapshot(&self) -> StatusSnapshot {
        let inner = self.lock();
        StatusSnapshot {
            phase: inner.phase,
            model: inner.model.clone(),
            settings: self.settings.clone(),
            last_result: inner.last_result.clone(),
            last_error: inner.last_error.clone(),
        }
    }

    /// Erreur de démarrage (réglages, raccourci) à afficher dans la fenêtre.
    pub fn report_error(&self, message: String) {
        let mut inner = self.lock();
        self.fail(&mut inner, message);
    }

    pub fn on_key(self: &Arc<Self>, key: KeyEvent) {
        let mut inner = self.lock();
        let held = inner.recording.as_ref().map(ActiveRecording::elapsed);
        match dictation::on_key(self.settings.mode, inner.phase, key, held) {
            Action::StartRecording => self.start_recording(&mut inner),
            Action::StopAndTranscribe => self.stop_and_submit(&mut inner),
            Action::Cancel => {
                if let Some(rec) = inner.recording.take() {
                    // L'audio est effacé à la libération (Zeroizing).
                    drop(rec.stop());
                }
                self.set_phase(&mut inner, Phase::Idle, None);
            }
            Action::Nothing => {
                if inner.phase == Phase::Processing && key == KeyEvent::Pressed {
                    self.emit_state(Phase::Processing, Some("Traitement en cours…".into()));
                }
            }
        }
    }

    fn start_recording(self: &Arc<Self>, inner: &mut Inner) {
        match &inner.model {
            ModelStatus::Missing { .. } | ModelStatus::Failed { .. } => {
                self.fail(inner, "Modèle de transcription indisponible (voir la fenêtre).".into());
                return;
            }
            ModelStatus::Loading | ModelStatus::Ready { .. } => {}
        }
        let max_secs = self.settings.max_recording_secs;
        match audio::start_recording(self.settings.input_device.as_deref(), max_secs) {
            Ok(rec) => {
                tracing::info!(device = %rec.device_name, "enregistrement démarré");
                inner.recording = Some(rec);
                inner.session += 1;
                self.set_phase(inner, Phase::Recording, None);
                self.spawn_max_duration_guard(inner.session, max_secs);
            }
            Err(e) => self.fail(inner, format!("Micro : {e}")),
        }
    }

    /// Arrête automatiquement une dictée trop longue (ex. mode toggle oublié).
    fn spawn_max_duration_guard(self: &Arc<Self>, session: u64, max_secs: u32) {
        let this = Arc::clone(self);
        let _ = std::thread::Builder::new()
            .name("max-duration".into())
            .spawn(move || {
                std::thread::sleep(Duration::from_secs(max_secs as u64));
                let mut inner = this.lock();
                if inner.session == session && inner.phase == Phase::Recording {
                    tracing::info!("durée maximale atteinte, arrêt automatique");
                    this.stop_and_submit(&mut inner);
                }
            });
    }

    fn stop_and_submit(&self, inner: &mut Inner) {
        let released_at = Instant::now();
        let Some(rec) = inner.recording.take() else {
            self.set_phase(inner, Phase::Idle, None);
            return;
        };
        match rec.stop() {
            Ok(audio) => {
                let job = Job {
                    audio,
                    released_at,
                    capture_stop: released_at.elapsed(),
                };
                if self.jobs.send(job).is_err() {
                    self.fail(inner, "Le moteur de transcription ne répond plus.".into());
                    return;
                }
                self.set_phase(inner, Phase::Processing, None);
            }
            Err(e) => self.fail(inner, format!("Micro : {e}")),
        }
    }

    fn run_worker(self: Arc<Self>, model_path: PathBuf, rx: mpsc::Receiver<Job>) {
        let loaded_at = Instant::now();
        let mut transcriber = match Transcriber::load(&model_path, &self.settings.language) {
            Ok(t) => {
                let load_ms = loaded_at.elapsed().as_millis() as u64;
                tracing::info!(load_ms, "modèle whisper chargé");
                self.lock().model = ModelStatus::Ready { load_ms };
                Some(t)
            }
            Err(stt::SttError::ModelMissing(path)) => {
                tracing::warn!("modèle whisper absent");
                self.lock().model = ModelStatus::Missing { path };
                None
            }
            Err(e) => {
                tracing::error!("chargement whisper : {e}");
                self.lock().model = ModelStatus::Failed { message: e.to_string() };
                None
            }
        };
        self.emit_state(self.lock().phase, None);

        // Tant que l'app vit, on traite les dictées une par une.
        while let Ok(job) = rx.recv() {
            let outcome = match transcriber.as_mut() {
                Some(t) => process(t, job),
                None => Err("Modèle de transcription indisponible.".into()),
            };
            let mut inner = self.lock();
            match outcome {
                Ok(Some(result)) => {
                    let t = &result.timings;
                    tracing::info!(
                        audio_s = result.audio_secs,
                        words = result.word_count,
                        capture_stop_ms = t.capture_stop_ms,
                        preprocess_ms = t.preprocess_ms,
                        transcription_ms = t.transcription_ms,
                        total_ms = t.total_ms,
                        "dictée traitée"
                    );
                    inner.last_result = Some(result.clone());
                    inner.last_error = None;
                    self.set_phase(&mut inner, Phase::Idle, None);
                    let _ = self.app.emit(EVT_RESULT, &result);
                    if self.settings.show_result_window {
                        ui::show_main_window(&self.app);
                    }
                }
                Ok(None) => self.set_phase(&mut inner, Phase::Idle, Some("Aucune parole détectée".into())),
                Err(message) => self.fail(&mut inner, message),
            }
        }
    }

    fn set_phase(&self, inner: &mut Inner, phase: Phase, message: Option<String>) {
        inner.phase = phase;
        self.emit_state(phase, message);
    }

    fn fail(&self, inner: &mut Inner, message: String) {
        tracing::warn!("{message}");
        inner.last_error = Some(message.clone());
        inner.phase = Phase::Idle;
        let _ = self.app.emit(
            EVT_STATE,
            StateEvent {
                phase: Phase::Idle,
                message: Some(message),
            },
        );
    }

    fn emit_state(&self, phase: Phase, message: Option<String>) {
        let _ = self.app.emit(EVT_STATE, StateEvent { phase, message });
    }
}

/// Traite une dictée. `Ok(None)` = rien à transcrire (silence).
fn process(transcriber: &mut Transcriber, job: Job) -> Result<Option<DictationResult>, String> {
    let Job {
        audio,
        released_at,
        capture_stop,
    } = job;
    let audio_secs = audio.duration_secs();

    if dsp::is_digital_silence(&audio.samples) {
        return Err("Aucun signal micro. Vérifiez l'autorisation Micro dans Réglages Système › \
                    Confidentialité et sécurité."
            .into());
    }

    let pre_start = Instant::now();
    let mut mono16k = Zeroizing::new(dsp::resample(&audio.samples, audio.sample_rate, WHISPER_SAMPLE_RATE));
    drop(audio);
    let Some(span) = dsp::detect_speech(&mono16k, WHISPER_SAMPLE_RATE) else {
        return Ok(None);
    };
    let speech = &mut mono16k[span.range.clone()];
    dsp::normalize_quiet(speech);
    let preprocess = pre_start.elapsed();

    let stt_start = Instant::now();
    let text = transcriber.transcribe(speech).map_err(|e| e.to_string())?;
    let transcription = stt_start.elapsed();
    drop(mono16k);

    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(DictationResult {
        word_count: stt::word_count(&text),
        text,
        audio_secs,
        voiced_secs: span.voiced_secs,
        timings: Timings {
            capture_stop_ms: capture_stop.as_millis() as u64,
            preprocess_ms: preprocess.as_millis() as u64,
            transcription_ms: transcription.as_millis() as u64,
            total_ms: released_at.elapsed().as_millis() as u64,
        },
    }))
}
