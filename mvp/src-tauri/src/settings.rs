//! Réglages utilisateur, stockés en JSON lisible dans le dossier de config de l'app.
//!
//! Phase 1 : pas d'interface, le fichier est créé avec les valeurs par défaut au
//! premier lancement et peut être édité à la main (redémarrage requis).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("lecture/écriture de {path} impossible : {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{path} n'est pas un JSON de réglages valide : {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("réglage invalide : {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutMode {
    /// Maintenir la touche pendant la dictée.
    PushToTalk,
    /// Appuyer une fois pour démarrer, une fois pour arrêter.
    Toggle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// Syntaxe du plugin global-shortcut : "Alt+Space", "CmdOrCtrl+Shift+D", "F13"…
    /// Une pédale ou un bouton matériel qui émet F13–F20 fonctionne sans code en plus.
    pub shortcut: String,
    pub mode: ShortcutMode,
    /// Nom de fichier du modèle whisper.cpp, cherché dans `<données de l'app>/models/`.
    pub whisper_model: String,
    /// Code langue ISO 639-1 imposé à Whisper (évite la détection automatique).
    pub language: String,
    /// Nom du micro ; `None` = périphérique d'entrée par défaut du système.
    pub input_device: Option<String>,
    /// Durée maximale d'une dictée ; au-delà, l'enregistrement s'arrête seul.
    pub max_recording_secs: u32,
    /// Phase 1 uniquement : affiche la fenêtre de résultat après chaque dictée.
    /// Sera désactivé quand l'injection au curseur existera (phase 2).
    pub show_result_window: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shortcut: "Alt+Space".into(),
            mode: ShortcutMode::PushToTalk,
            whisper_model: "ggml-large-v3-turbo-q5_0.bin".into(),
            language: "fr".into(),
            input_device: None,
            max_recording_secs: 120,
            show_result_window: true,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.shortcut.trim().is_empty() {
            return Err(SettingsError::Invalid("le raccourci est vide".into()));
        }
        if !(5..=600).contains(&self.max_recording_secs) {
            return Err(SettingsError::Invalid(
                "max_recording_secs doit être compris entre 5 et 600".into(),
            ));
        }
        let lang_ok = self.language.len() == 2 && self.language.chars().all(|c| c.is_ascii_lowercase());
        if !lang_ok {
            return Err(SettingsError::Invalid(format!(
                "code langue attendu sur 2 lettres minuscules, reçu {:?}",
                self.language
            )));
        }
        let model_path = Path::new(&self.whisper_model);
        if model_path.components().count() != 1 || !self.whisper_model.ends_with(".bin") {
            return Err(SettingsError::Invalid(
                "whisper_model doit être un simple nom de fichier .bin".into(),
            ));
        }
        Ok(())
    }

    /// Charge les réglages ; crée le fichier avec les valeurs par défaut s'il n'existe pas.
    pub fn load_or_create(dir: &Path) -> Result<Self, SettingsError> {
        let path = dir.join(SETTINGS_FILE);
        let settings = match fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str::<Settings>(&raw).map_err(|source| SettingsError::Parse {
                path: path.clone(),
                source,
            })?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let defaults = Settings::default();
                defaults.save(dir)?;
                defaults
            }
            Err(source) => return Err(SettingsError::Io { path, source }),
        };
        settings.validate()?;
        Ok(settings)
    }

    pub fn save(&self, dir: &Path) -> Result<(), SettingsError> {
        let path = dir.join(SETTINGS_FILE);
        let io_err = |source| SettingsError::Io {
            path: path.clone(),
            source,
        };
        fs::create_dir_all(dir).map_err(io_err)?;
        let json = serde_json::to_string_pretty(self).map_err(|source| SettingsError::Parse {
            path: path.clone(),
            source,
        })?;
        // Écriture atomique : un crash en cours d'écriture ne corrompt pas le fichier.
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(io_err)?;
        fs::rename(&tmp, &path).map_err(io_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dictee-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn defaults_are_valid() {
        assert!(Settings::default().validate().is_ok());
    }

    #[test]
    fn creates_file_on_first_load_then_reads_it_back() {
        let dir = temp_dir("create");
        let first = Settings::load_or_create(&dir).expect("création");
        assert!(dir.join(SETTINGS_FILE).exists());
        let second = Settings::load_or_create(&dir).expect("relecture");
        assert_eq!(first, second);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let s: Settings = serde_json::from_str(r#"{ "mode": "toggle", "shortcut": "F13" }"#).expect("parse");
        assert_eq!(s.mode, ShortcutMode::Toggle);
        assert_eq!(s.shortcut, "F13");
        assert_eq!(s.language, "fr");
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let r = serde_json::from_str::<Settings>(r#"{ "shortcutt": "F13" }"#);
        assert!(r.is_err());
    }

    #[test]
    fn rejects_path_traversal_in_model_name() {
        let s = Settings {
            whisper_model: "../../etc/model.bin".into(),
            ..Settings::default()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn rejects_bad_language_and_duration() {
        let s = Settings {
            language: "français".into(),
            ..Settings::default()
        };
        assert!(s.validate().is_err());
        let s = Settings {
            max_recording_secs: 0,
            ..Settings::default()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn invalid_json_is_reported_not_overwritten() {
        let dir = temp_dir("invalid");
        fs::create_dir_all(&dir).expect("mkdir");
        fs::write(dir.join(SETTINGS_FILE), "{ pas du json").expect("write");
        assert!(matches!(Settings::load_or_create(&dir), Err(SettingsError::Parse { .. })));
        assert_eq!(fs::read_to_string(dir.join(SETTINGS_FILE)).expect("read"), "{ pas du json");
        let _ = fs::remove_dir_all(&dir);
    }
}
