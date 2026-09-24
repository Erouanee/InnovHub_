//! Fenêtres : indicateur flottant et fenêtre principale (résultat/état).

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const MAIN_WINDOW: &str = "main";
pub const INDICATOR_WINDOW: &str = "indicator";

const INDICATOR_WIDTH: f64 = 280.0;
const INDICATOR_HEIGHT: f64 = 64.0;
const INDICATOR_BOTTOM_MARGIN: f64 = 96.0;

/// Crée l'indicateur une seule fois, au démarrage.
///
/// Il reste ouvert en permanence, transparent et traversé par la souris ; seul
/// son contenu HTML apparaît ou disparaît. Le montrer/cacher à chaque dictée
/// risquerait d'activer l'app et de voler le focus à l'application où
/// l'utilisateur écrit, ce qui casserait l'injection au curseur (phase 2).
pub fn create_indicator(app: &AppHandle) -> tauri::Result<()> {
    let mut builder = WebviewWindowBuilder::new(app, INDICATOR_WINDOW, WebviewUrl::App("indicator.html".into()))
        .title("")
        .inner_size(INDICATOR_WIDTH, INDICATOR_HEIGHT)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false);

    if let Some(monitor) = app.primary_monitor()? {
        let scale = monitor.scale_factor();
        let size = monitor.size().to_logical::<f64>(scale);
        let origin = monitor.position().to_logical::<f64>(scale);
        builder = builder.position(
            origin.x + (size.width - INDICATOR_WIDTH) / 2.0,
            origin.y + size.height - INDICATOR_HEIGHT - INDICATOR_BOTTOM_MARGIN,
        );
    } else {
        builder = builder.center();
    }

    let window = builder.build()?;
    window.set_ignore_cursor_events(true)?;
    Ok(())
}

pub fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        tracing::warn!("fenêtre principale introuvable");
        return;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}
