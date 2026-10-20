//! La ventana del selector de acentos, creada cuando hace falta.
//!
//! Declarada en `tauri.conf.json`, esta ventana existía desde el arranque y el
//! WebKit completo quedaba residente toda la sesión. Medido en una sesión real:
//! **~329 MB** para dibujar una fila de botones que en el frontend son noventa
//! líneas de Vue, y que aparece un par de segundos por día.
//!
//! Acá la latencia sí importa —esto es un método de entrada, no un cartel— así
//! que la creación diferida se apoya en dos cosas:
//!
//! 1. **Se calienta en el `keydown`, no en el `OpenPicker`.** El selector abre
//!    recién a los [`HOLD_THRESHOLD`](crate::input::HOLD_THRESHOLD) ms de tener
//!    la tecla apretada, así que empezar a construir cuando la tecla *baja*
//!    esconde el costo detrás de un gesto que la persona ya está haciendo. Y
//!    sólo se calienta si la ventana no existe: con el selector tibio, escribir
//!    no dispara nada.
//! 2. **El estado se pide, no se emite.** Entre crear el webview y que Vue
//!    monte pasan cientos de milisegundos, y un `emit` en ese hueco se pierde.
//!    El backend guarda qué variantes hay que mostrar y el frontend las reclama
//!    al montar con `picker_ready`. Si la ventana llega tarde, el selector
//!    aparece tarde — nunca vacío, y nunca se pierde el acento.
//!
//! El desarme es deliberadamente perezoso: [`IDLE`] es largo porque quien usó
//! un acento va a usar otro, y pagar la creación en cada palabra sería peor que
//! el problema que esto resuelve.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};

use crate::commands::ensure_created;
use crate::input::AccentPayload;

/// Traza sólo cuando se pide.
pub(crate) fn traza(mensaje: &str) {
    if std::env::var_os("VASAK_PICKER_TRACE").is_some() {
        eprintln!("[press-and-hold/picker] {mensaje}");
    }
}

/// Medidas del selector, compartidas con `input.rs`.
///
/// Estaban sólo allá, y acá se mostraba la ventana con el tamaño del
/// constructor: la primera vez después de cada desarme el selector abría en
/// 320x48 en lugar de lo que necesitaban las variantes. Para la `a`, con siete,
/// hacen falta 376x76 — la tarjeta salía cortada, y sólo se acomodaba en el
/// siguiente uso.
pub const ANCHO_ITEM: f64 = 48.0;
pub const ANCHO_MARGEN: f64 = 40.0;
pub const ALTO_VENTANA: f64 = 76.0;

/// Tamaño que necesita el selector para `variantes` letras.
pub fn tamano_para(variantes: usize) -> (f64, f64) {
    (
        ANCHO_ITEM * variantes.max(1) as f64 + ANCHO_MARGEN,
        ALTO_VENTANA,
    )
}

static READY: AtomicBool = AtomicBool::new(false);
pub(crate) static CREATING: AtomicBool = AtomicBool::new(false);

/// Lo que el selector tiene que mostrar en cuanto pueda.
pub(crate) static PENDIENTE: Mutex<Option<AccentPayload>> = Mutex::new(None);

/// La tecla bajó: si el selector está frío, empezar a construirlo ya.
///
/// Se llama en cada pulsación de una tecla con variantes, así que tiene que ser
/// baratísimo cuando no hay nada que hacer — y lo es: una lectura del mapa de
/// ventanas y nada más.
pub fn warm_up(app: &AppHandle) {
    if app.get_webview_window("main").is_some() {
        return;
    }
    ensure_created(app);
}

/// Deja anotado qué mostrar y lo emite si el frontend ya está escuchando.
pub fn deliver(app: &AppHandle, payload: AccentPayload) {
    let mut pendiente = PENDIENTE.lock().unwrap_or_else(|e| e.into_inner());
    if READY.load(Ordering::SeqCst) {
        let _ = app.emit("show-accent-menu", payload);
        return;
    }
    *pendiente = Some(payload);
    drop(pendiente);
    ensure_created(app);
}

/// El frontend montó: se lleva lo que haya quedado esperando.
pub fn take_pending() -> Option<AccentPayload> {
    let mut pendiente = PENDIENTE.lock().unwrap_or_else(|e| e.into_inner());
    READY.store(true, Ordering::SeqCst);
    pendiente.take()
}
