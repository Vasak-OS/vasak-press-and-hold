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
    // Y se le arma el reloj del desarme.
    //
    // La mayoría de las teclas que calientan la ventana terminan siendo un tap,
    // así que el selector nunca se abre y `hide_picker` —que es quien programa
    // el desarme— no llega a correr nunca. Sin esto, escribir una sola vocal
    // dejaba el webview levantado hasta cerrar la sesión, que es justo lo que
    // este módulo vino a evitar.
    arm_teardown(app);
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
    arm_teardown(app);
}

/// Programa el desarme sin tocar lo que haya pendiente.
///
/// Separado de [`schedule_teardown`] porque el calentamiento también necesita
/// armarlo, y ahí puede haber un acento en camino: borrar la cola desde el
/// `keydown` se llevaría el selector que está a punto de abrirse.
fn arm_teardown(app: &AppHandle) {
    let generation = touch();
    let app = app.clone();
    // Un hilo y no una tarea async: este proceso no tiene runtime de tokio, y
    // traerlo entero para dormir diez minutos sería peor que el hilo.
    std::thread::spawn(move || {
        std::thread::sleep(idle());
        if GENERATION.load(Ordering::SeqCst) == generation {
            traza("silencio: se desarma el selector");
            teardown(&app, Some(generation));
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
                crate::attach_layer_shell(&ventana);
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

fn teardown(app: &AppHandle, esperada: Option<u64>) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        // La generación se revalida **acá dentro**. `READY` se limpia en este
        // cierre, no cuando salta el temporizador, así que entre la comprobación
        // del hilo y esta línea `deliver` puede haber emitido el acento —viendo
        // READY todavía en true, así que no lo encoló— y destruir la ventana en
        // ese hueco lo perdía sin error: sólo se recuperaba con la tecla
        // siguiente.
        if let Some(esperada) = esperada {
            if GENERATION.load(Ordering::SeqCst) != esperada {
                traza("llegó un acento mientras se desarmaba: se cancela");
                return;
            }
        }
        READY.store(false, Ordering::SeqCst);
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.destroy();
        }
    });
}
