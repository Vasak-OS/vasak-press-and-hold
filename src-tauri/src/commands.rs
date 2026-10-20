//! Los comandos de Tauri y el glue de GTK/WebKit del selector.
//!
//! Aparte de `lib.rs` a propósito: es la capa que habla con la ventana, el
//! webview y el teclado, y no se puede ejercitar sin un display ni un WebKit
//! reales. Por eso vive en `commands.rs`, que el análisis de cobertura del taller
//! excluye —igual que los `.vue`—: medir cobertura de lo que ninguna prueba puede
//! montar sólo bloquea sin informar. Lo testeable (la cola y el acotado del
//! tamaño) se queda en `picker_window.rs`.

use std::sync::atomic::Ordering;

use gtk::prelude::*;
use gtk_layer_shell::LayerShell;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::input::{self, AccentPayload};
use crate::picker_window::{tamano_para, CREATING, PENDIENTE};

/// Traza sólo cuando se pide.
fn traza(mensaje: &str) {
    if std::env::var_os("VASAK_PICKER_TRACE").is_some() {
        eprintln!("[press-and-hold/picker] {mensaje}");
    }
}
use crate::AppState;

/// Alto y ancho de arranque de la ventana. Los mismos valores que declaraba
/// `tauri.conf.json`; `show_picker` la redimensiona según cuántas variantes haya.
const ANCHO_INICIAL: f64 = 320.0;
const ALTO_INICIAL: f64 = 48.0;

/// El frontend montó: se lleva lo que haya que mostrar.
///
/// Devuelve `None` cuando no hay nada pendiente, que es lo normal si la ventana
/// se creó por el calentamiento del `keydown` y la tecla terminó siendo un tap.
#[tauri::command]
pub(crate) fn picker_ready() -> Option<AccentPayload> {
    crate::picker_window::take_pending()
}

#[tauri::command]
pub(crate) fn select_accent(app: AppHandle, index: usize) -> Result<(), String> {
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

/// Busca el `WebKitWebView` dentro del árbol de widgets de la ventana.
///
/// wry lo anida un par de contenedores adentro, así que hay que bajar
/// recursivamente hasta dar con el tipo. Mismo criterio que vasak-file-manager.
fn find_webkit_webview(container: &gtk::Container) -> Option<webkit2gtk::WebView> {
    for child in container.children() {
        if child.type_().name() == "WebKitWebView" {
            return child.downcast::<webkit2gtk::WebView>().ok();
        }
        if let Some(child_container) = child.downcast_ref::<gtk::Container>() {
            if let Some(found) = find_webkit_webview(child_container) {
                return Some(found);
            }
        }
    }
    None
}

/// Afina WebKit para el selector: es estático, chico y efímero, no necesita los
/// cachés ni la GPU de un navegador. Baja el piso de memoria del webview.
fn tune_webkit(window: &tauri::WebviewWindow) {
    use webkit2gtk::{
        CacheModel, HardwareAccelerationPolicy, SettingsExt, WebContextExt, WebViewExt,
    };

    let Ok(gtk_win) = window.gtk_window() else {
        return;
    };
    let container = gtk_win.upcast::<gtk::Container>();
    let Some(webview) = find_webkit_webview(&container) else {
        eprintln!("[press-and-hold] no se encontró el WebKitWebView para afinar");
        return;
    };

    if let Some(settings) = WebViewExt::settings(&webview) {
        settings.set_enable_developer_extras(false);
        settings.set_enable_back_forward_navigation_gestures(false);
        settings.set_enable_webgl(false);
        // Una fila de botones estática no gana nada con la GPU, y sin aceleración
        // se ahorra un proceso de GPU y los buffers DMABUF.
        settings.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Never);
    }
    if let Some(context) = WebViewExt::web_context(&webview) {
        // No navega páginas: con el caché mínimo alcanza.
        context.set_cache_model(CacheModel::DocumentViewer);
    }

    // El webview es residente toda la sesión y ya no se recrea: si su
    // `WebKitWebProcess` muere (un crash del renderer, el OOM killer), sin esto
    // quedaría con una página muerta y `READY` en `true` para siempre —`deliver`
    // emitiría a la nada y `show_picker` mapearía un overlay vacío—, roto hasta
    // reiniciar el servicio. Antes el desarme por inactividad lo recreaba; ahora
    // lo recupera recargándolo, que respawnea el proceso y remonta el frontend
    // (que vuelve a pedir con `picker_ready`). `mark_not_ready` deja lo que
    // llegue en `PENDIENTE` mientras tanto.
    webview.connect_web_process_terminated(|webview, reason| {
        eprintln!("[press-and-hold/picker] el WebKitWebProcess terminó ({reason:?}); recargando");
        crate::picker_window::mark_not_ready();
        webview.reload();
    });
}

/// Crea el selector la primera vez que hace falta (y una sola): construye el
/// webview, le aplica layer-shell, lo afina y —si ya hay algo que mostrar— lo
/// mapea. Después se esconde y se reusa; no se destruye al cerrarse.
///
/// Destruir la ventana al cerrarla dejaba vivo su `WebKitWebProcess` —wry/GTK no
/// lo reapea—: una sesión real juntaba decenas de procesos colgados. Por eso una
/// vez creado el webview queda residente y oculto.
pub(crate) fn ensure_created(app: &AppHandle) {
    if app.get_webview_window("main").is_some() {
        return;
    }
    if CREATING.swap(true, Ordering::SeqCst) {
        return;
    }

    let app = app.clone();
    // Se construye desde este hilo, **no** dentro de `run_on_main_thread`.
    //
    // Tauri despacha la creación al bucle de eventos por su cuenta; hacerlo a
    // mano desde dentro de una vuelta del bucle de GTK reentra en él y el
    // webview queda a medio inicializar. Pero el trabajo de GTK —el
    // layer-shell— sí tiene que ir en el hilo principal o aborta con «GTK may
    // only be used from the main thread». Son dos requisitos opuestos y hay que
    // respetar los dos.
    let built = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .title("Accents")
        .inner_size(ANCHO_INICIAL, ALTO_INICIAL)
        .decorations(false)
        .transparent(true)
        .skip_taskbar(true)
        .resizable(false)
        .always_on_top(true)
        .visible(false)
        .build();

    match built {
        Ok(_) => {
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                let Some(ventana) = handle.get_webview_window("main") else {
                    return;
                };
                attach_layer_shell(&ventana);
                tune_webkit(&ventana);
                // Si ya hay algo que mostrar, mapear la ventana ahora.
                //
                // `show_picker` corre en el hilo del teclado justo después de
                // `deliver` y puede no encontrarla todavía; y una ventana sin
                // mapear no carga la página. Acá ya sabemos que existe.
                // Se lee cuántas variantes hay para darle el tamaño correcto:
                // `show_picker` corre en el hilo del teclado y suele llegar
                // antes de que esta ventana exista, así que sin esto la primera
                // apertura de cada tanda quedaba con el tamaño del constructor.
                let pendiente = PENDIENTE
                    .lock()
                    .unwrap_or_else(|envenenado| envenenado.into_inner())
                    .as_ref()
                    .map(|payload| payload.variants.len());
                if let Some(variantes) = pendiente {
                    let (ancho, alto) = tamano_para(variantes);
                    traza(&format!("hay un acento esperando: {ancho}x{alto}"));
                    let _ = ventana.set_size(tauri::LogicalSize::new(ancho, alto));
                    let _ = ventana.show();
                }
            });
        }
        Err(error) => eprintln!("[press-and-hold] no se pudo crear el selector: {error}"),
    }

    CREATING.store(false, Ordering::SeqCst);
}
