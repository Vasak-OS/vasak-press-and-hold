mod accent_map;
mod char_input;
mod commands;
mod energia;
mod input;
mod no_molestar;
mod picker_window;
mod uinput;

use std::sync::{mpsc, Arc, Mutex};

use tauri::Manager;

use commands::{picker_ready, select_accent};

struct AppState {
    current_variants: Arc<Mutex<Vec<char>>>,
    inject_tx: mpsc::Sender<input::InjectCommand>,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_config_manager::init())
        .setup(|app| {
            let (inject_tx, inject_rx) = mpsc::channel();

            // One list, shared: the input loop fills it when the picker opens and
            // `select_accent` reads it to know what the chosen index means. They
            // used to be two separate Arcs, so the one the command read was never
            // written to — every selection failed with «Invalid index» and the
            // accent could not be typed at all.
            let current_variants: Arc<Mutex<Vec<char>>> = Arc::new(Mutex::new(Vec::new()));

            app.manage(AppState {
                current_variants: current_variants.clone(),
                inject_tx,
            });

            let handle = app.handle().clone();
            std::thread::spawn(move || input::run_input_loop(handle, inject_rx, current_variants));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![select_accent, picker_ready])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_app, event| {
            // Este proceso es un servicio de teclado, no una ventana. Tauri
            // cierra la aplicación al destruirse la última ventana; al arrancar
            // no hay ninguna (el selector se crea con la primera tecla con
            // variantes), así que sin esto el demonio se apagaría solo antes de
            // atender el teclado.
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
            }
        });
}
