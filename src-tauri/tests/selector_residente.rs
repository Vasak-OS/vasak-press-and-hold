//! Que el selector no vuelva a destruirse al cerrarse.
//!
//! La fuga era ésta: el selector es un webview en una ventana layer-shell, y
//! destruirlo tras un rato de inactividad dejaba vivo su `WebKitWebProcess`
//! —wry/GTK no lo reapea—. Una sesión real juntaba decenas de procesos colgados
//! (34 medidos en 25 h, ~86 MB). El arreglo: el selector se crea una vez, se
//! **esconde** entre usos y se **reusa**; nunca se destruye.
//!
//! Es una guarda de fuente, no de comportamiento: la fuga es de ciclo de vida de
//! GTK/WebKit y de reaping de procesos, que no se puede montar sin un display ni
//! un WebKit reales. Lo que sí se puede impedir es que el patrón que fugaba vuelva
//! al código. Se verificó además midiendo: con el arreglo, press-and-hold tiene un
//! solo WebKitWebProcess en reposo, no decenas.

const PICKER: &str = include_str!("../src/picker_window.rs");
const INPUT: &str = include_str!("../src/input.rs");
const COMMANDS: &str = include_str!("../src/commands.rs");

#[test]
fn no_vuelve_el_desarme_por_inactividad() {
    assert!(
        !PICKER.contains("schedule_teardown") && !INPUT.contains("schedule_teardown"),
        "volvió el desarme por inactividad (`schedule_teardown`): cada ciclo \
         crear→destruir del webview fuga un WebKitWebProcess. El selector se \
         esconde y se reusa; no se destruye al cerrarse."
    );
    assert!(
        !PICKER.contains("fn idle("),
        "volvió el plazo de inactividad (`idle`): sólo existía para el desarme \
         que fugaba."
    );
}

#[test]
fn el_glue_del_webview_no_destruye() {
    // Fuera del arranque fallido, nada construye y destruye el webview por uso.
    // `commands.rs` crea el selector una sola vez (`ensure_created`) y no debe
    // tener un camino que lo destruya en la vida normal.
    assert!(
        !COMMANDS.contains(".destroy()") && !COMMANDS.contains("fn teardown"),
        "commands.rs volvió a destruir el webview: esconderlo alcanza, y reusarlo \
         evita la fuga de WebKitWebProcess y la ventana de carga."
    );
}
