//! Machine à états du raccourci : logique pure, sans I/O, testée unitairement.
//!
//! Elle décide *quoi faire* à chaque évènement clavier ; `pipeline.rs` exécute.

use std::time::Duration;

use serde::Serialize;

use crate::settings::ShortcutMode;

/// En push-to-talk, un appui plus court est considéré comme accidentel.
pub const MIN_PUSH_TO_TALK: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Recording,
    Processing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    StartRecording,
    StopAndTranscribe,
    /// Arrête et jette l'audio sans le transcrire.
    Cancel,
    Nothing,
}

/// `held_for` : durée écoulée depuis le début de l'enregistrement (si en cours).
pub fn on_key(mode: ShortcutMode, phase: Phase, key: KeyEvent, held_for: Option<Duration>) -> Action {
    match (phase, mode, key) {
        // Un traitement est en cours : on ignore, l'indicateur signale « occupé ».
        (Phase::Processing, _, _) => Action::Nothing,

        (Phase::Idle, _, KeyEvent::Pressed) => Action::StartRecording,
        (Phase::Idle, _, KeyEvent::Released) => Action::Nothing,

        // Push-to-talk : l'auto-répétition du clavier peut renvoyer Pressed.
        (Phase::Recording, ShortcutMode::PushToTalk, KeyEvent::Pressed) => Action::Nothing,
        (Phase::Recording, ShortcutMode::PushToTalk, KeyEvent::Released) => match held_for {
            Some(d) if d < MIN_PUSH_TO_TALK => Action::Cancel,
            _ => Action::StopAndTranscribe,
        },

        (Phase::Recording, ShortcutMode::Toggle, KeyEvent::Pressed) => Action::StopAndTranscribe,
        (Phase::Recording, ShortcutMode::Toggle, KeyEvent::Released) => Action::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Action::*;
    use KeyEvent::*;
    use ShortcutMode::*;

    const LONG: Option<Duration> = Some(Duration::from_secs(5));

    #[test]
    fn push_to_talk_cycle() {
        assert_eq!(on_key(PushToTalk, Phase::Idle, Pressed, None), StartRecording);
        assert_eq!(on_key(PushToTalk, Phase::Recording, Pressed, LONG), Nothing);
        assert_eq!(on_key(PushToTalk, Phase::Recording, Released, LONG), StopAndTranscribe);
    }

    #[test]
    fn push_to_talk_short_tap_is_cancelled() {
        let tap = Some(Duration::from_millis(80));
        assert_eq!(on_key(PushToTalk, Phase::Recording, Released, tap), Cancel);
    }

    #[test]
    fn toggle_cycle_ignores_releases() {
        assert_eq!(on_key(Toggle, Phase::Idle, Pressed, None), StartRecording);
        assert_eq!(on_key(Toggle, Phase::Recording, Released, LONG), Nothing);
        assert_eq!(on_key(Toggle, Phase::Recording, Pressed, LONG), StopAndTranscribe);
        assert_eq!(on_key(Toggle, Phase::Idle, Released, None), Nothing);
    }

    #[test]
    fn toggle_short_session_is_not_cancelled() {
        let short = Some(Duration::from_millis(80));
        assert_eq!(on_key(Toggle, Phase::Recording, Pressed, short), StopAndTranscribe);
    }

    #[test]
    fn busy_while_processing() {
        for mode in [PushToTalk, Toggle] {
            for key in [Pressed, Released] {
                assert_eq!(on_key(mode, Phase::Processing, key, None), Nothing);
            }
        }
    }
}
