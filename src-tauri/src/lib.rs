mod accent_map;
mod char_input;
mod energia;
mod input;
mod picker_window;
mod uinput;

use std::sync::{Arc, Mutex, mpsc};

use gtk::prelude::Cast;
use gtk_layer_shell::LayerShell;
use tauri::{AppHandle, Manager};

struct AppState {
    current_variants: Arc<Mutex<Vec<char>>>,
    inject_tx: mpsc::Sender<input::InjectCommand>,
}

/// Suelta el teclado que la ventana del selector hubiera tomado.
///
/// Antes esto viajaba por un canal de glib hacia el bucle principal, con el
/// `MainContext::channel` que gtk-rs marcó como obsoleto. El canal existía sólo
/// para llegar al hilo de GTK, y Tauri ya sabe hacer eso: `run_on_main_thread`
/// es el mismo mecanismo que usa el resto del archivo, sin un canal ni un enum
/// de comandos de un solo caso.
fn release_keyboard(app: &AppHandle) {
    // Sobre la ventana del selector, no sobre todos los toplevels.
    //
    // `list_toplevels()` devuelve cualquier `ApplicationWindow` del proceso, así
    // que esto podía llamar `set_keyboard_interactivity` sobre una ventana que
    // nunca pasó por `init_layer_shell` — donde la llamada no significa nada, o
    // significa otra cosa.
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(ventana) = handle.get_webview_window("main") else {
            return;
        };
        let Ok(gtk_win) = ventana.gtk_window() else {
            return;
        };
        if let Ok(win) = gtk_win.downcast::<gtk::ApplicationWindow>() {
            win.set_keyboard_interactivity(false);
        }
    });
}

/// Aplica layer-shell a la ventana del selector.
///
/// Recibe la ventana en vez de recorrer los toplevels de GTK: la ventana ahora
/// se crea cuando hace falta, así que en el arranque no había ninguna que
/// configurar — y recorrer los toplevels habría tocado cualquier otra que
/// existiera en el proceso.
pub(crate) fn attach_layer_shell(window: &tauri::WebviewWindow) {
    let Ok(gtk_win) = window.gtk_window() else {
        eprintln!("[press-and-hold] no se pudo obtener la ventana GTK");
        return;
    };

    if let Ok(win) = gtk_win.clone().downcast::<gtk::ApplicationWindow>() {
        win.init_layer_shell();
        win.set_layer(gtk_layer_shell::Layer::Overlay);
        win.set_keyboard_interactivity(false);
        win.set_anchor(gtk_layer_shell::Edge::Bottom, true);
        win.set_layer_shell_margin(gtk_layer_shell::Edge::Bottom, 80);
        win.set_namespace("vasak-accents");
    } else {
        eprintln!("[press-and-hold] la ventana no es una ApplicationWindow");
    }
}

/// El frontend montó: se lleva lo que haya que mostrar.
///
/// Devuelve `None` cuando no hay nada pendiente, que es lo normal si la ventana
/// se creó por el calentamiento del `keydown` y la tecla terminó siendo un tap.
#[tauri::command]
fn picker_ready() -> Option<input::AccentPayload> {
    picker_window::take_pending()
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

    release_keyboard(&app);

    Ok(())
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
            std::thread::spawn(move || {
                input::run_input_loop(handle, inject_rx, current_variants)
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![select_accent, picker_ready])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_app, event| {
            // Este proceso es un servicio de teclado, no una ventana. Tauri
            // cierra la aplicación al destruirse la última ventana, y el
            // selector ahora se desarma tras diez minutos sin acentos: sin esto
            // el demonio se apagaba solo y dejaba de atender el teclado.
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
            }
        });
}
