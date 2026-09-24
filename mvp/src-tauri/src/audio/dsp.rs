//! Traitements audio purs (sans I/O) : rééchantillonnage, détection de parole,
//! gain. Tout est testable sans micro.

use std::f64::consts::PI;
use std::ops::Range;

/// Fréquence attendue par Whisper.
pub const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// Moyenne des canaux d'un buffer entrelacé, ajoutée à `out` (mono).
pub fn push_downmixed(interleaved: &[f32], channels: usize, out: &mut Vec<f32>) {
    if channels <= 1 {
        out.extend_from_slice(interleaved);
        return;
    }
    let scale = 1.0 / channels as f32;
    out.extend(
        interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() * scale),
    );
}

/// Rééchantillonnage polyphase par sinc fenêtrée (Blackman).
///
/// Le filtre passe-bas est placé sous la plus petite des deux fréquences de Nyquist,
/// ce qui évite le repliement lors du passage 48 kHz → 16 kHz. Pour un rapport
/// rationnel `to/from = L/M`, il n'existe que L décalages fractionnaires distincts :
/// les coefficients sont précalculés une fois par phase, la boucle interne ne fait
/// plus que des multiplications-additions.
pub fn resample(input: &[f32], from_hz: u32, to_hz: u32) -> Vec<f32> {
    if from_hz == to_hz || input.is_empty() || from_hz == 0 || to_hz == 0 {
        return input.to_vec();
    }
    const ZERO_CROSSINGS: f64 = 16.0;
    const ROLLOFF: f64 = 0.95;
    // Au-delà, les phases sont arrondies : erreur de position < 1/1024 d'échantillon.
    const MAX_PHASES: u64 = 1024;

    let g = gcd(from_hz as u64, to_hz as u64);
    let (up, down) = (to_hz as u64 / g, from_hz as u64 / g); // L et M
    let phases = up.min(MAX_PHASES);

    let cutoff = (to_hz as f64 / from_hz as f64).min(1.0) * ROLLOFF; // relatif au Nyquist d'entrée
    let half_width = ZERO_CROSSINGS / cutoff; // en échantillons d'entrée
    let reach = half_width.ceil() as i64;
    let taps = (2 * reach + 2) as usize; // indices relatifs -reach ..= reach+1

    let table: Vec<f32> = (0..phases)
        .flat_map(|p| {
            let frac = p as f64 / phases as f64;
            (0..taps).map(move |t| {
                let x = (t as i64 - reach) as f64 - frac;
                (cutoff * sinc(cutoff * x) * blackman(x / half_width)) as f32
            })
        })
        .collect();

    let out_len = (input.len() as u64 * up / down) as usize;
    let len = input.len() as i64;
    let mut out = Vec::with_capacity(out_len);
    for n in 0..out_len as u64 {
        let pos = n * down;
        let base = (pos / up) as i64;
        let phase = (pos % up) * phases / up;
        let weights = &table[phase as usize * taps..(phase as usize + 1) * taps];

        let first = base - reach;
        let lo = (-first).max(0) as usize;
        let hi = taps.min((len - first).max(0) as usize);
        let mut acc = 0.0f32;
        let mut norm = 0.0f32;
        if lo < hi {
            let start = (first + lo as i64) as usize;
            let src = &input[start..start + (hi - lo)];
            for (x, w) in src.iter().zip(&weights[lo..hi]) {
                acc += x * w;
                norm += w;
            }
        }
        // Normaliser par la somme des poids garde un gain unitaire, y compris aux bords.
        out.push(if norm.abs() > 1e-9 { acc / norm } else { 0.0 });
    }
    out
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

/// Fenêtre de Blackman sur u ∈ [-1, 1].
fn blackman(u: f64) -> f64 {
    if u.abs() >= 1.0 {
        0.0
    } else {
        0.42 + 0.5 * (PI * u).cos() + 0.08 * (2.0 * PI * u).cos()
    }
}

/// Vrai si le signal est nul : typique d'un accès micro refusé sur macOS
/// (CoreAudio livre alors des zéros au lieu d'une erreur).
pub fn is_digital_silence(samples: &[f32]) -> bool {
    samples.iter().all(|s| s.abs() < 1e-6)
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpeechSpan {
    /// Portion à transcrire (bords silencieux retirés, marge incluse).
    pub range: Range<usize>,
    /// Durée cumulée des trames jugées parlées.
    pub voiced_secs: f32,
}

/// Détection d'activité vocale par énergie, avec seuil adaptatif au bruit de fond.
///
/// Rôle en phase 1 : couper les silences de début/fin et ne pas envoyer à Whisper
/// un enregistrement vide (Whisper « hallucine » alors des phrases entières).
/// Un VAD neuronal (Silero, déjà intégré à whisper.cpp) pourra le remplacer si
/// ce seuil se montre insuffisant en environnement bruyant.
pub fn detect_speech(samples: &[f32], sample_rate: u32) -> Option<SpeechSpan> {
    const FRAME_SECS: f32 = 0.03;
    const PADDING_SECS: f32 = 0.3;
    const MIN_VOICED_SECS: f32 = 0.25;
    const MIN_THRESHOLD: f32 = 0.004;
    const MAX_THRESHOLD: f32 = 0.02;

    let frame = ((sample_rate as f32 * FRAME_SECS) as usize).max(1);
    let rms: Vec<f32> = samples
        .chunks(frame)
        .map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt())
        .collect();
    if rms.is_empty() {
        return None;
    }

    let mut sorted = rms.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let noise_floor = sorted[sorted.len() / 10];
    let threshold = (noise_floor * 3.0).clamp(MIN_THRESHOLD, MAX_THRESHOLD);

    let voiced: Vec<usize> = rms
        .iter()
        .enumerate()
        .filter(|(_, &r)| r > threshold)
        .map(|(i, _)| i)
        .collect();
    let voiced_secs = voiced.len() as f32 * FRAME_SECS;
    if voiced_secs < MIN_VOICED_SECS {
        return None;
    }

    let (first, last) = (*voiced.first()?, *voiced.last()?);
    let pad = (sample_rate as f32 * PADDING_SECS) as usize;
    let start = (first * frame).saturating_sub(pad);
    let end = ((last + 1) * frame + pad).min(samples.len());
    Some(SpeechSpan {
        range: start..end,
        voiced_secs,
    })
}

/// Remonte le niveau d'une prise trop faible (micro lointain), sans toucher
/// à une prise déjà correcte. Gain plafonné pour ne pas amplifier le bruit.
pub fn normalize_quiet(samples: &mut [f32]) {
    const TARGET_PEAK: f32 = 0.5;
    const MAX_GAIN: f32 = 8.0;
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak <= 1e-6 || peak >= TARGET_PEAK {
        return;
    }
    let gain = (TARGET_PEAK / peak).min(MAX_GAIN);
    samples.iter_mut().for_each(|s| *s *= gain);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sr: u32, secs: f32, amp: f32) -> Vec<f32> {
        let n = (sr as f32 * secs) as usize;
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / sr as f32).sin())
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn downmix_stereo_averages_channels() {
        let mut out = Vec::new();
        push_downmixed(&[1.0, 0.0, 0.5, 0.5, -1.0, 1.0], 2, &mut out);
        assert_eq!(out, vec![0.5, 0.5, 0.0]);
    }

    #[test]
    fn downmix_mono_is_identity() {
        let mut out = vec![0.1];
        push_downmixed(&[0.2, 0.3], 1, &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn resample_length_matches_ratio() {
        let input = vec![0.0; 48_000];
        assert_eq!(resample(&input, 48_000, 16_000).len(), 16_000);
        let input = vec![0.0; 44_100];
        assert_eq!(resample(&input, 44_100, 16_000).len(), 16_000);
    }

    #[test]
    fn resample_keeps_in_band_tone() {
        let input = sine(440.0, 48_000, 1.0, 0.5);
        let out = resample(&input, 48_000, 16_000);
        // Ignorer les bords, où le filtre n'a qu'une moitié de support.
        let mid = &out[1_000..15_000];
        let expected = 0.5 / 2f32.sqrt();
        assert!((rms(mid) - expected).abs() < 0.01, "rms = {}", rms(mid));
    }

    #[test]
    fn resample_rejects_out_of_band_tone() {
        // 12 kHz est au-dessus du Nyquist de sortie (8 kHz) : doit disparaître
        // au lieu de se replier en 4 kHz audible.
        let input = sine(12_000.0, 48_000, 1.0, 0.5);
        let out = resample(&input, 48_000, 16_000);
        assert!(rms(&out[1_000..15_000]) < 0.01, "rms = {}", rms(&out[1_000..15_000]));
    }

    #[test]
    fn resample_same_rate_is_copy() {
        let input = vec![0.1, 0.2, 0.3];
        assert_eq!(resample(&input, 16_000, 16_000), input);
    }

    #[test]
    fn digital_silence() {
        assert!(is_digital_silence(&[0.0; 100]));
        assert!(!is_digital_silence(&[0.0, 0.01, 0.0]));
    }

    #[test]
    fn detects_speech_surrounded_by_silence() {
        let sr = 16_000;
        let mut x = vec![0.0005; sr as usize]; // 1 s de bruit faible
        x.extend(sine(300.0, sr, 1.0, 0.2)); // 1 s de « parole »
        x.extend(vec![0.0005; sr as usize]);
        let span = detect_speech(&x, sr).expect("parole détectée");
        let pad = (sr as f32 * 0.3) as usize;
        assert!(span.range.start >= sr as usize - pad - 480);
        assert!(span.range.end <= 2 * sr as usize + pad + 480);
        assert!((span.voiced_secs - 1.0).abs() < 0.1, "voiced = {}", span.voiced_secs);
    }

    #[test]
    fn rejects_noise_only() {
        let x = vec![0.001; 32_000];
        assert!(detect_speech(&x, 16_000).is_none());
        assert!(detect_speech(&[], 16_000).is_none());
    }

    #[test]
    fn rejects_too_short_blip() {
        let sr = 16_000;
        let mut x = vec![0.0; sr as usize];
        x.extend(sine(300.0, sr, 0.1, 0.3));
        x.extend(vec![0.0; sr as usize]);
        assert!(detect_speech(&x, sr).is_none());
    }

    #[test]
    fn normalize_boosts_quiet_but_caps_gain() {
        let mut x = vec![0.1, -0.05];
        normalize_quiet(&mut x);
        assert!((x[0] - 0.5).abs() < 1e-6);

        let mut tiny = vec![0.01];
        normalize_quiet(&mut tiny);
        assert!((tiny[0] - 0.08).abs() < 1e-6);

        let mut loud = vec![0.9];
        normalize_quiet(&mut loud);
        assert_eq!(loud, vec![0.9]);
    }
}
