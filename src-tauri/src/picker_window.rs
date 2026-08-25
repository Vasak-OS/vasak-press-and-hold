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

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::input::AccentPayload;

/// Cuánto silencio desarma el webview. Diez minutos: quien acentúa una palabra
/// suele acentuar otra, y la creación no se paga dos veces en la misma sesión
/// de escritura.
const IDLE_DEFAULT: Duration = Duration::from_secs(600);

/// El plazo real, con un escape para poder probarlo sin esperar diez minutos.
fn idle() -> Duration {
    std::env::var("VASAK_PICKER_IDLE_SECS")
        .ok()
        .and_then(|valor| valor.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(IDLE_DEFAULT)
}

/// Traza sólo cuando se pide.
fn traza(mensaje: &str) {
    if std::env::var_os("VASAK_PICKER_TRACE").is_some() {
        eprintln!("[press-and-hold/picker] {mensaje}");
    }
}

/// Alto de la ventana y medidas de los ítems. Los mismos valores que declaraba
/// `tauri.conf.json`; `show_picker` la redimensiona según cuántas variantes haya.
const ANCHO_INICIAL: f64 = 320.0;
const ALTO_INICIAL: f64 = 48.0;

static READY: AtomicBool = AtomicBool::new(false);
static CREATING: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Lo que el selector tiene que mostrar en cuanto pueda.
static PENDIENTE: Mutex<Option<AccentPayload>> = Mutex::new(None);

fn touch() -> u64 {
    GENERATION.fetch_add(1, Ordering::SeqCst) + 1
}

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
    touch();
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

/// Se cerró el selector: nada que mostrar, y el reloj del desarme arranca.
pub fn schedule_teardown(app: &AppHandle) {
    {
        let mut pendiente = PENDIENTE.lock().unwrap_or_else(|e| e.into_inner());
        *pendiente = None;
    }

    let generation = touch();
    let app = app.clone();
    // Un hilo y no una tarea async: este proceso no tiene runtime de tokio, y
    // traerlo entero para dormir diez minutos sería peor que el hilo.
    std::thread::spawn(move || {
        std::thread::sleep(idle());
        if GENERATION.load(Ordering::SeqCst) == generation {
            traza("silencio: se desarma el selector");
            teardown(&app);
        }
    });
}

fn ensure_created(app: &AppHandle) {
    if app.get_webview_window("main").is_some() {
        return;
    }
    if CREATING.swap(true, Ordering::SeqCst) {
        return;
    }

    let app = app.clone();
    let result = app.clone().run_on_main_thread(move || {
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
            Ok(window) => {
                crate::attach_layer_shell(&window);
                // Si ya hay algo que mostrar, mapear la ventana ahora.
                //
                // `show_picker` corre en el hilo del teclado inmediatamente
                // después de `deliver`, y la construcción de la ventana es
                // asíncrona: cuando llega a buscarla puede no existir todavía y
                // se va sin mostrar nada. Como además un webview cuya ventana no
                // se mapeó no carga la página, esa carrera dejaba el selector
                // esperando para siempre. Acá ya sabemos que la ventana existe.
                let hay_pendiente = PENDIENTE.lock().map(|p| p.is_some()).unwrap_or(false);
                if hay_pendiente {
                    traza("hay un acento esperando: se mapea la ventana");
                    let _ = window.show();
                }
            }
            Err(error) => eprintln!("[press-and-hold] no se pudo crear el selector: {error}"),
        }
        CREATING.store(false, Ordering::SeqCst);
    });

    if result.is_err() {
        CREATING.store(false, Ordering::SeqCst);
    }
}

fn teardown(app: &AppHandle) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        READY.store(false, Ordering::SeqCst);
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.destroy();
        }
    });
}
