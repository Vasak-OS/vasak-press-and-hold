mod accent_map;
mod char_input;
mod input;
mod uinput;

use std::sync::{Arc, Mutex, mpsc};

use gtk::prelude::Cast;
use gtk_layer_shell::LayerShell;
use tauri::{AppHandle, Manager};

struct AppState {
    gtk_tx: gtk::glib::Sender<GtkCommand>,
    current_variants: Arc<Mutex<Vec<char>>>,
    inject_tx: mpsc::Sender<input::InjectCommand>,
}

enum GtkCommand {
    SetKeyboardInteractivity(bool),
}

#[tauri::command]
fn select_accent(app: AppHandle, index: usize) -> Result<(), String> {
    let state = app.state::<AppState>();

    let ch = {
        let variants = state.current_variants.lock().map_err(|e| e.to_string())?;
        *variants.get(index).ok_or("Invalid index")?
    };

    state
        .inject_tx
        .send(input::InjectCommand::Char(ch))
        .map_err(|e| e.to_string())?;

    if let Some(win) = app.get_webview_window("main") {
        win.hide().map_err(|e| e.to_string())?;
    }

    state
        .gtk_tx
        .send(GtkCommand::SetKeyboardInteractivity(false))
        .ok();

    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_config_manager::init())
        .setup(|app| {
            let (gtk_tx, gtk_rx) = gtk::glib::MainContext::channel(gtk::glib::Priority::DEFAULT);

            gtk_rx.attach(None, move |cmd: GtkCommand| {
                match cmd {
                    GtkCommand::SetKeyboardInteractivity(enabled) => {
                        for w in gtk::Window::list_toplevels() {
                            if let Ok(win) = w.downcast::<gtk::ApplicationWindow>() {
                                win.set_keyboard_interactivity(enabled);
                            }
                        }
                    }
                }
                gtk::glib::ControlFlow::Continue
            });

            gtk::glib::idle_add_local_once(|| {
                for w in gtk::Window::list_toplevels() {
                    if let Ok(win) = w.downcast::<gtk::ApplicationWindow>() {
                        win.init_layer_shell();
                        win.set_layer(gtk_layer_shell::Layer::Overlay);
                        win.set_keyboard_interactivity(false);
                        win.set_anchor(gtk_layer_shell::Edge::Bottom, true);
                        win.set_layer_shell_margin(gtk_layer_shell::Edge::Bottom, 80);
                        win.set_namespace("vasak-accents");
                    }
                }
            });

            let (inject_tx, inject_rx) = mpsc::channel();

            // One list, shared: the input loop fills it when the picker opens and
            // `select_accent` reads it to know what the chosen index means. They
            // used to be two separate Arcs, so the one the command read was never
            // written to — every selection failed with «Invalid index» and the
            // accent could not be typed at all.
            let current_variants: Arc<Mutex<Vec<char>>> = Arc::new(Mutex::new(Vec::new()));

            app.manage(AppState {
                gtk_tx,
                current_variants: current_variants.clone(),
                inject_tx,
            });

            if let Some(win) = app.get_webview_window("main") {
                let _ = win.hide();
            }

            let handle = app.handle().clone();
            std::thread::spawn(move || {
                input::run_input_loop(handle, inject_rx, current_variants)
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![select_accent])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
