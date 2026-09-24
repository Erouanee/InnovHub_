// Empêche l'ouverture d'une console supplémentaire sous Windows en release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    dictee_lib::run();
}
