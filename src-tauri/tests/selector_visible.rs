//! Que el selector de acentos no vuelva a quedar mapeado pero invisible.
//!
//! El síntoma: el selector se mostraba y la traza decía
//! `visible=Ok(true) tamaño=Ok(PhysicalSize { width: 0, height: 0 })`; Wayfire
//! veía una geometría 0×0. El acento igual se escribía (la selección por número
//! va por el bucle del teclado), por eso nadie notaba la ventana ausente. Dos
//! causas:
//!
//! 1. `set_size` de Tauri no dimensiona una superficie de layer-shell anclada a
//!    un borde: la deja commiteada en 0×0. El tamaño lo fija la ventana GTK
//!    (`set_size_request` + `resize`), como el overlay de vasak-flare-daemon.
//! 2. `HardwareAccelerationPolicy::Never` en una superficie de capa transparente
//!    deja el webview renderizando por software sin presentar ningún pixel.
//!
//! Son guardas de fuente, no de comportamiento: presentar un buffer en una
//! superficie de capa necesita un compositor y un WebKitGTK reales, que no se
//! pueden montar sin display. Lo que sí se puede impedir es que vuelva el patrón
//! que lo rompía.

const COMMANDS: &str = include_str!("../src/commands.rs");
const INPUT: &str = include_str!("../src/input.rs");

/// Cuerpo de `fn <nombre>(` en `fuente`, sin líneas de comentario: los
/// comentarios explican por qué algo NO está y nombran justo lo prohibido.
fn cuerpo_sin_comentarios(fuente: &str, nombre: &str) -> String {
    let desde = fuente
        .find(&format!("fn {nombre}("))
        .unwrap_or_else(|| panic!("falta fn {nombre}"));
    let cuerpo = &fuente[desde..];
    let hasta = cuerpo
        .find("\n}\n")
        .unwrap_or_else(|| panic!("no se encontró el fin de {nombre}"));
    cuerpo[..hasta]
        .lines()
        .filter(|linea| !linea.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn el_selector_se_dimensiona_con_la_ventana_gtk() {
    let cuerpo = cuerpo_sin_comentarios(COMMANDS, "dimensionar");
    assert!(
        cuerpo.contains("set_size_request(") && cuerpo.contains(".resize("),
        "dimensionar tiene que fijar el tamaño con `set_size_request` + `resize` \
         sobre la ventana GTK: es lo único que dimensiona una superficie de capa."
    );
    assert!(
        !cuerpo.contains(".set_size("),
        "dimensionar no debe usar el `set_size` de Tauri: no toca la superficie \
         de layer-shell y la deja en 0x0."
    );
}

#[test]
fn mostrar_el_selector_no_usa_set_size_de_tauri() {
    let cuerpo = cuerpo_sin_comentarios(INPUT, "show_picker");
    assert!(
        !cuerpo.contains(".set_size("),
        "show_picker volvió a usar `set_size` de Tauri: el selector queda 0x0, \
         mapeado pero invisible."
    );
    assert!(
        cuerpo.contains("resize_and_show("),
        "show_picker tiene que mostrar el selector por `resize_and_show`, que \
         dimensiona la ventana GTK en el hilo principal."
    );
}

#[test]
fn el_selector_recien_creado_tampoco_usa_set_size_de_tauri() {
    // El acento pendiente al terminar de crear la ventana es el otro camino que
    // dimensiona el selector.
    let cuerpo = cuerpo_sin_comentarios(COMMANDS, "ensure_created");
    assert!(
        !cuerpo.contains(".set_size("),
        "ensure_created volvió a usar `set_size` de Tauri para el acento \
         pendiente: usá `dimensionar`."
    );
    assert!(
        cuerpo.contains("dimensionar("),
        "ensure_created tiene que usar `dimensionar`."
    );
}

#[test]
fn el_selector_no_apaga_la_aceleracion_por_hardware() {
    let cuerpo = cuerpo_sin_comentarios(COMMANDS, "tune_webkit");
    assert!(
        !cuerpo.contains("set_hardware_acceleration_policy")
            && !cuerpo.contains("HardwareAccelerationPolicy"),
        "tune_webkit volvió a tocar la política de aceleración por hardware: con \
         `Never` el selector (superficie de capa transparente) se mapea pero no \
         presenta ningún pixel. Dejala en su valor por omisión."
    );
}
