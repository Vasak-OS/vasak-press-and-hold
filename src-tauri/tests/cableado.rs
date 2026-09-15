//! Que la compuerta del «no molestar» siga enchufada.
//!
//! La lógica tiene sus tests: con `is_target: false` la tecla se reenvía al
//! bajar, y `es_un_juego` decide bien. Lo que ninguno de los dos ve es el cable
//! entre las dos cosas — el sitio donde se calcula `is_target`.
//!
//! Si alguien saca el `&& !no_molestar.activo()` de ahí, todo compila, los
//! treinta tests siguen en verde, y el selector vuelve a comerse las teclas
//! dentro de los juegos. Se vería sólo jugando.

use std::path::PathBuf;

fn fuente(nombre: &str) -> String {
    let ruta = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join(nombre);
    std::fs::read_to_string(&ruta).unwrap_or_else(|e| panic!("no se pudo leer {nombre}: {e}"))
}

/// Lo que decide si una tecla es objetivo tiene que mirar el «no molestar».
#[test]
fn el_calculo_de_is_target_consulta_el_no_molestar() {
    let input = fuente("input.rs");

    let linea = input
        .lines()
        .skip_while(|l| !l.contains("let is_target ="))
        .take(3)
        .collect::<Vec<_>>()
        .join(" ");

    assert!(
        !linea.is_empty(),
        "no se encontró dónde se calcula is_target"
    );
    assert!(
        linea.contains("no_molestar"),
        "is_target dejó de mirar el no_molestar: dentro de un juego el selector \
         vuelve a comerse las teclas de movimiento. Línea: {linea}"
    );
}

/// Y el vigilante tiene que arrancar, o el booleano se queda en `false` para
/// siempre y la compuerta no cierra nunca.
#[test]
fn el_vigilante_arranca_con_el_bucle_del_teclado() {
    let input = fuente("input.rs");
    assert!(
        input.contains("no_molestar::vigilar()"),
        "nadie arranca el vigilante: el booleano queda en false y la compuerta no cierra nunca"
    );
}

/// La consulta no puede hacerse por tecla.
///
/// Son cientos por segundo y cada una es una ida y vuelta por un socket. Si
/// aparece una conexión o un `get-focused-view` dentro de `input.rs`, se volvió
/// a preguntar en el lugar equivocado.
#[test]
fn no_se_le_pregunta_a_wayfire_por_cada_tecla() {
    let input = fuente("input.rs");
    for señal in ["UnixStream", "get-focused-view", "WAYFIRE_SOCKET"] {
        assert!(
            !input.contains(señal),
            "«{señal}» apareció en input.rs: la consulta a Wayfire va en el hilo \
             del vigilante, no en el camino de cada tecla"
        );
    }
}
