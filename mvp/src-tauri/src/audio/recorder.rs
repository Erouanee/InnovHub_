//! Enregistrement micro via cpal.
//!
//! Le flux cpal n'est pas `Send` sur toutes les plateformes : il vit dans un
//! thread dédié, piloté par un canal. Le reste de l'app ne manipule que
//! `ActiveRecording`, qui est `Send`.

use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use thiserror::Error;
use zeroize::Zeroizing;

use super::dsp::push_downmixed;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("aucun micro disponible")]
    NoInputDevice,
    #[error("micro inaccessible : {0}")]
    Device(String),
    #[error("format audio non pris en charge : {0}")]
    UnsupportedFormat(String),
    #[error("le thread de capture s'est arrêté de façon inattendue")]
    CaptureThreadDied,
}

/// Audio mono capturé. Le buffer est mis à zéro à la libération (`Zeroizing`).
pub struct CapturedAudio {
    pub samples: Zeroizing<Vec<f32>>,
    pub sample_rate: u32,
}

impl CapturedAudio {
    pub fn duration_secs(&self) -> f32 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.samples.len() as f32 / self.sample_rate as f32
    }
}

pub struct ActiveRecording {
    stop_tx: mpsc::Sender<()>,
    handle: JoinHandle<Result<CapturedAudio, AudioError>>,
    started_at: Instant,
    pub device_name: String,
}

impl ActiveRecording {
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Arrête le flux et rend l'audio. Bloque le temps de fermer le flux (quelques ms).
    pub fn stop(self) -> Result<CapturedAudio, AudioError> {
        // Si le thread est déjà mort, l'envoi échoue : join() remontera l'erreur.
        let _ = self.stop_tx.send(());
        self.handle.join().map_err(|_| AudioError::CaptureThreadDied)?
    }
}

/// Démarre la capture ; retourne une fois le flux réellement ouvert.
pub fn start_recording(device_name: Option<&str>, max_secs: u32) -> Result<ActiveRecording, AudioError> {
    let (ready_tx, ready_rx) = mpsc::channel::<Result<String, AudioError>>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let wanted = device_name.map(str::to_owned);

    let handle = std::thread::Builder::new()
        .name("audio-capture".into())
        .spawn(move || capture_thread(wanted, max_secs, ready_tx, stop_rx))
        .map_err(|e| AudioError::Device(e.to_string()))?;

    match ready_rx.recv() {
        Ok(Ok(device_name)) => Ok(ActiveRecording {
            stop_tx,
            handle,
            started_at: Instant::now(),
            device_name,
        }),
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(e)
        }
        Err(_) => Err(AudioError::CaptureThreadDied),
    }
}

type SharedBuffer = Arc<Mutex<Zeroizing<Vec<f32>>>>;

fn capture_thread(
    wanted: Option<String>,
    max_secs: u32,
    ready_tx: mpsc::Sender<Result<String, AudioError>>,
    stop_rx: mpsc::Receiver<()>,
) -> Result<CapturedAudio, AudioError> {
    let opened = open_stream(wanted.as_deref(), max_secs);
    let (stream, buffer, sample_rate, name) = match opened {
        Ok(v) => v,
        Err(e) => {
            // Le message part vers start_recording ; on renvoie une copie ici.
            let copy = AudioError::Device(e.to_string());
            let _ = ready_tx.send(Err(e));
            return Err(copy);
        }
    };
    let _ = ready_tx.send(Ok(name));

    // Attend l'ordre d'arrêt (ou la fermeture du canal si l'app se termine).
    let _ = stop_rx.recv();
    drop(stream);

    let mut guard = buffer.lock().unwrap_or_else(PoisonError::into_inner);
    let samples = std::mem::take(&mut *guard);
    Ok(CapturedAudio {
        samples,
        sample_rate,
    })
}

fn open_stream(
    wanted: Option<&str>,
    max_secs: u32,
) -> Result<(cpal::Stream, SharedBuffer, u32, String), AudioError> {
    let host = cpal::default_host();
    let device = pick_device(&host, wanted)?;
    let name = device_label(&device);

    let supported = device
        .default_input_config()
        .map_err(|e| AudioError::Device(e.to_string()))?;
    let sample_rate = supported.sample_rate();
    let channels = supported.channels() as usize;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    let max_samples = sample_rate as usize * max_secs as usize;
    // Pré-allouer 30 s limite les réallocations (donc les copies non effacées).
    let buffer: SharedBuffer = Arc::new(Mutex::new(Zeroizing::new(Vec::with_capacity(
        (sample_rate as usize * 30).min(max_samples),
    ))));

    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, &config, channels, max_samples, &buffer),
        SampleFormat::I16 => build::<i16>(&device, &config, channels, max_samples, &buffer),
        SampleFormat::I32 => build::<i32>(&device, &config, channels, max_samples, &buffer),
        SampleFormat::U16 => build::<u16>(&device, &config, channels, max_samples, &buffer),
        SampleFormat::U8 => build::<u8>(&device, &config, channels, max_samples, &buffer),
        other => return Err(AudioError::UnsupportedFormat(format!("{other:?}"))),
    }?;
    stream.play().map_err(|e| AudioError::Device(e.to_string()))?;
    tracing::debug!(sample_rate, channels, ?format, "flux micro ouvert");
    Ok((stream, buffer, sample_rate, name))
}

fn pick_device(host: &cpal::Host, wanted: Option<&str>) -> Result<cpal::Device, AudioError> {
    if let Some(wanted) = wanted {
        let found = host
            .input_devices()
            .map_err(|e| AudioError::Device(e.to_string()))?
            .find(|d| device_label(d) == wanted);
        match found {
            Some(d) => return Ok(d),
            None => tracing::warn!("micro configuré introuvable, repli sur le micro par défaut"),
        }
    }
    host.default_input_device().ok_or(AudioError::NoInputDevice)
}

fn device_label(device: &cpal::Device) -> String {
    device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "micro inconnu".into())
}

/// Noms des micros disponibles (pour les réglages).
pub fn list_input_devices() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|devs| devs.map(|d| device_label(&d)).collect())
        .unwrap_or_default()
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    max_samples: usize,
    buffer: &SharedBuffer,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let buffer = Arc::clone(buffer);
    let mut scratch: Vec<f32> = Vec::with_capacity(8192);
    device
        .build_input_stream::<T, _, _>(
            *config,
            move |data: &[T], _| {
                scratch.clear();
                scratch.extend(data.iter().map(|&s| <f32 as FromSample<T>>::from_sample_(s)));
                let mut out = buffer.lock().unwrap_or_else(PoisonError::into_inner);
                if out.len() < max_samples {
                    push_downmixed(&scratch, channels, &mut out);
                    out.truncate(max_samples);
                }
                // `scratch` contient de la voix : on l'efface aussitôt.
                scratch.iter_mut().for_each(|s| *s = 0.0);
            },
            |err| tracing::warn!("erreur du flux micro : {err}"),
            None,
        )
        .map_err(|e| AudioError::Device(e.to_string()))
}
