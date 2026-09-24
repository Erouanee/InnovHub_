//! Outil de développement : transcrit un WAV avec le même pipeline que l'app
//! (rééchantillonnage, détection de parole, Whisper) et affiche les latences.
//! Sert à mesurer la transcription sans micro ni interface graphique.
//!
//! cargo run --release --example transcribe_wav -- <modèle.bin> <fichier.wav> [répétitions]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use dictee_lib::audio::dsp::{self, WHISPER_SAMPLE_RATE};
use dictee_lib::stt::Transcriber;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage : transcribe_wav <modèle.bin> <fichier.wav> [répétitions]");
        return ExitCode::from(2);
    }
    match run(PathBuf::from(&args[1]), PathBuf::from(&args[2]), args.get(3)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erreur : {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(model: PathBuf, wav: PathBuf, repeats: Option<&String>) -> Result<(), Box<dyn std::error::Error>> {
    let repeats: usize = repeats.map(|r| r.parse()).transpose()?.unwrap_or(3);

    let mut reader = hound::WavReader::open(&wav)?;
    let spec = reader.spec();
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()?
        }
    };
    let mut mono = Vec::new();
    dsp::push_downmixed(&interleaved, spec.channels as usize, &mut mono);
    let audio_secs = mono.len() as f32 / spec.sample_rate as f32;

    let t = Instant::now();
    let mut transcriber = Transcriber::load(&model, "fr")?;
    if std::env::var_os("DICTEE_FULL_CTX").is_some() {
        println!("fenêtre d'encodeur complète (30 s)");
        transcriber.set_adaptive_context(false);
    }
    println!("chargement modèle + préchauffage : {} ms", t.elapsed().as_millis());
    println!("audio : {audio_secs:.1} s @ {} Hz", spec.sample_rate);

    for i in 1..=repeats {
        let t = Instant::now();
        let mut x = dsp::resample(&mono, spec.sample_rate, WHISPER_SAMPLE_RATE);
        let Some(span) = dsp::detect_speech(&x, WHISPER_SAMPLE_RATE) else {
            println!("aucune parole détectée");
            return Ok(());
        };
        let speech = &mut x[span.range];
        dsp::normalize_quiet(speech);
        let pre = t.elapsed();
        let t2 = Instant::now();
        let text = transcriber.transcribe(speech)?;
        let stt = t2.elapsed();
        println!(
            "#{i} prétraitement {} ms · transcription {} ms · total {} ms",
            pre.as_millis(),
            stt.as_millis(),
            (pre + stt).as_millis()
        );
        if i == 1 {
            println!("texte : {text}");
        }
    }
    Ok(())
}
