//! Icône de barre des menus : seul point d'entrée visible de l'application.

use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::App;

use crate::settings::{Settings, ShortcutMode};
use crate::ui;

const TRAY_ICON: &[u8] = include_bytes!("../icons/tray.png");

pub fn create(app: &App, settings: &Settings) -> tauri::Result<()> {
    let mode = match settings.mode {
        ShortcutMode::PushToTalk => "maintenir",
        ShortcutMode::Toggle => "appuyer pour démarrer/arrêter",
    };
    let hint = MenuItemBuilder::with_id("hint", format!("Dicter : {} ({mode})", settings.shortcut))
        .enabled(false)
        .build(app)?;
    let show = MenuItemBuilder::with_id("show", "Afficher la dernière transcription…").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quitter").build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&hint)
        .separator()
        .item(&show)
        .separator()
        .item(&quit)
        .build()?;

    TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(TRAY_ICON)?)
        .icon_as_template(true)
        .tooltip("Dictée locale")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => ui::show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}
