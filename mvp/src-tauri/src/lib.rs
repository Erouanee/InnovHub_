pub mod audio;
pub mod dictation;
mod pipeline;
pub mod settings;
pub mod stt;
mod tray;
mod ui;

use std::sync::Arc;

use tauri::{Manager, RunEvent, State, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tracing_subscriber::EnvFilter;

use crate::dictation::KeyEvent;
use crate::pipeline::{Controller, StatusSnapshot};
use crate::settings::Settings;

#[tauri::command]
fn get_status(controller: State<'_, Arc<Controller>>) -> StatusSnapshot {
    controller.snapshot()
}

fn init_tracing() {
    // Journaux techniques sur stderr uniquement, jamais dans un fichier, et
    // jamais de contenu dicté (voir README, « Garanties de confidentialité »).
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,whisper_rs=warn"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_target(false).try_init();
    // Redirige les logs C de whisper.cpp/ggml vers `tracing` (filtrés ci-dessus).
    whisper_rs::install_logging_hooks();
}

pub fn run() {
    init_tracing();

    let shortcut_plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            let key = match event.state() {
                ShortcutState::Pressed => KeyEvent::Pressed,
                ShortcutState::Released => KeyEvent::Released,
            };
            if let Some(controller) = app.try_state::<Arc<Controller>>() {
                controller.on_key(key);
            }
        })
        .build();

    let app = tauri::Builder::default()
        .plugin(shortcut_plugin)
        .invoke_handler(tauri::generate_handler![get_status])
        .setup(|app| {
            // Pas d'icône dans le Dock : l'app vit dans la barre des menus.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let config_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_data_dir()?;
            let (settings, settings_error) = match Settings::load_or_create(&config_dir) {
                Ok(s) => (s, None),
                Err(e) => (Settings::default(), Some(format!("Réglages ignorés : {e}"))),
            };
            let model_path = data_dir.join("models").join(&settings.whisper_model);

            let controller = Controller::start(app.handle().clone(), settings.clone(), model_path);
            app.manage(Arc::clone(&controller));

            tray::create(app, &settings)?;
            if let Err(e) = ui::create_indicator(app.handle()) {
                tracing::warn!("indicateur flottant indisponible : {e}");
            }

            let registered = settings
                .shortcut
                .parse::<Shortcut>()
                .map_err(|e| e.to_string())
                .and_then(|s| app.global_shortcut().register(s).map_err(|e| e.to_string()));
            let shortcut_error = registered
                .err()
                .map(|e| format!("Raccourci « {} » inutilisable : {e}", settings.shortcut));

            if let Some(message) = settings_error.or(shortcut_error) {
                controller.report_error(message);
                ui::show_main_window(app.handle());
            }
            tracing::info!(config = %config_dir.display(), "application prête");
            Ok(())
        })
        .on_window_event(|window, event| {
            // Fermer la fenêtre principale la cache ; l'app reste dans la barre des menus.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(e) => {
            tracing::error!("démarrage impossible : {e}");
            std::process::exit(1);
        }
    };
    app.run(|_app, event| {
        // Seul « Quitter » (code explicite) termine l'app.
        if let RunEvent::ExitRequested { code: None, api, .. } = event {
            api.prevent_exit();
        }
    });
}
