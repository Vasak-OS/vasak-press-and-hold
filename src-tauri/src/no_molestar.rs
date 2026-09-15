//! Cuándo el selector tiene que hacerse a un lado.
//!
//! # Por qué
//!
//! Las teclas que abren el selector son `a c e i o u n l`. En un videojuego eso
//! es **A** de moverse a la izquierda, **C** de agacharse y **E** de
//! interactuar — y este proceso toma el teclado en exclusiva, así que no las
//! deja pasar: mientras se mantiene una, el juego **no recibe nada**, y al
//! soltarla recibe un press y un release juntos, tarde.
//!
//! O sea que no alcanza con no dibujar la ventana. Mientras haya un juego
//! adelante, ninguna tecla puede ser «tecla objetivo»: se reenvían todas tal
//! cual, sin esperar, sin tragar y sin selector.
//!
//! # Cómo se sabe
//!
//! Se lo pregunta a Wayfire por su IPC: `window-rules/get-focused-view` contesta
//! `app-id` y `fullscreen`. Y `window-rules/events/watch` avisa de los cambios,
//! que es de donde ya vive `vasak-desktop`.
//!
//! **No se pregunta por cada tecla.** Hay cientos por segundo y cada consulta es
//! una ida y vuelta por el socket. Va al revés: un hilo escucha los eventos,
//! reconsulta cuando algo cambia y deja un booleano acá. Leerlo por tecla es un
//! `load` atómico.
//!
//! # Lo que no hace falta tocar
//!
//! Nada de la unidad de systemd: el socket de Wayfire es `AF_UNIX`, que
//! `RestrictAddressFamilies=AF_UNIX` ya permite, y el servicio ya recibe
//! `WAYFIRE_SOCKET` en su entorno.
//!
//! # Lo que no cubre
//!
//! Un juego en ventana común, ni a pantalla completa ni en la lista. Ahí el
//! selector sigue apareciendo y la salida es soltar la tecla. Taparlo del todo
//! pediría saber si hay un campo de texto con el foco, que es lo que resolvía el
//! diseño original —ser un método de entrada de Wayland— en lugar de leer el
//! teclado crudo.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Programas que corren en ventana sin bordes y aun así son un juego.
///
/// La pantalla completa cubre la mayoría; ésta es la excepción, que en Linux no
/// es rara.
const JUEGOS_POR_NOMBRE: &[&str] = &["gamescope", "steam", "lutris", "heroic", "bottles"];

/// Y los que se nombran por familia.
///
/// Separado de la lista de arriba a propósito: con `steam` como prefijo,
/// cualquier cosa que empiece igual —un `steamboat-editor`— quedaría tomada por
/// un juego y se apagaría el selector adentro. Un prefijo sólo vale cuando la
/// familia entera lo es, y `steam_app_` lo es: son los mil juegos de Steam.
const FAMILIAS_DE_JUEGOS: &[&str] = &["steam_app_"];

/// Si el programa que tiene el foco es uno de los que no hay que interrumpir.
///
/// En minúsculas porque los `app-id` no tienen una convención de mayúsculas y
/// comparar tal cual dejaría pasar un `Steam` con mayúscula.
pub fn es_un_juego(app_id: &str, pantalla_completa: bool) -> bool {
    if pantalla_completa {
        return true;
    }
    let id = app_id.to_ascii_lowercase();
    JUEGOS_POR_NOMBRE.contains(&id.as_str())
        || FAMILIAS_DE_JUEGOS
            .iter()
            .any(|familia| id.starts_with(familia))
}

/// El booleano que lee el bucle del teclado.
#[derive(Clone, Default)]
pub struct NoMolestar(Arc<AtomicBool>);

impl NoMolestar {
    pub fn nuevo() -> Self {
        Self::default()
    }

    /// Si hay que hacerse a un lado. Es lo que se consulta por cada tecla.
    pub fn activo(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn fijar(&self, valor: bool) {
        self.0.store(valor, Ordering::Relaxed);
    }
}

/// Un mensaje del IPC de Wayfire: cuatro bytes de largo y después el JSON.
fn enviar(socket: &mut UnixStream, metodo: &str) -> std::io::Result<serde_json::Value> {
    let cuerpo = serde_json::json!({ "method": metodo, "data": {} }).to_string();
    socket.write_all(&(cuerpo.len() as i32).to_ne_bytes())?;
    socket.write_all(cuerpo.as_bytes())?;
    leer(socket)
}

fn leer(socket: &mut UnixStream) -> std::io::Result<serde_json::Value> {
    let mut largo = [0u8; 4];
    socket.read_exact(&mut largo)?;
    let largo = i32::from_ne_bytes(largo);
    if !(0..=(1 << 22)).contains(&largo) {
        return Err(std::io::Error::other(format!(
            "largo fuera de rango: {largo}"
        )));
    }
    let mut cuerpo = vec![0u8; largo as usize];
    socket.read_exact(&mut cuerpo)?;
    serde_json::from_slice(&cuerpo).map_err(std::io::Error::other)
}

/// Lee de la respuesta de `get-focused-view` si hay que hacerse a un lado.
///
/// La respuesta viene envuelta en `info`, y con el escritorio vacío ese campo no
/// está: sin ventana enfocada no hay ningún juego, así que es `false`.
pub fn del_json(respuesta: &serde_json::Value) -> bool {
    let vista = respuesta.get("info").unwrap_or(respuesta);
    let app_id = vista.get("app-id").and_then(|v| v.as_str()).unwrap_or("");
    let completa = vista
        .get("fullscreen")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    es_un_juego(app_id, completa)
}

/// Arranca el hilo que mantiene el booleano al día.
///
/// Si Wayfire no contesta se devuelve un `NoMolestar` que siempre dice que no:
/// el selector se comporta como antes. Equivocarse para este lado sólo cuesta
/// que el selector aparezca donde no debía; equivocarse para el otro lo apagaría
/// del todo por un socket caído.
pub fn vigilar() -> NoMolestar {
    let estado = NoMolestar::nuevo();
    let ruta = match std::env::var("WAYFIRE_SOCKET") {
        Ok(r) if !r.is_empty() => r,
        _ => {
            eprintln!("[no-molestar] sin WAYFIRE_SOCKET: el selector no se va a apartar solo");
            return estado;
        }
    };

    let copia = estado.clone();
    std::thread::Builder::new()
        .name("no-molestar".into())
        .spawn(move || bucle(&ruta, &copia))
        .ok();
    estado
}

fn bucle(ruta: &str, estado: &NoMolestar) {
    loop {
        if let Err(e) = una_vuelta(ruta, estado) {
            eprintln!("[no-molestar] se cortó la conexión con Wayfire: {e}");
        }
        // Si Wayfire se cae o se reinicia, el selector queda como estaba y se
        // vuelve a intentar. Cinco segundos para no castigar un socket ausente.
        estado.fijar(false);
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}

fn una_vuelta(ruta: &str, estado: &NoMolestar) -> std::io::Result<()> {
    // Dos conexiones: una queda escuchando eventos y la otra pregunta. Sobre una
    // sola, la respuesta a la pregunta llega mezclada con los eventos y hay que
    // desenredarlas.
    let mut eventos = UnixStream::connect(ruta)?;
    enviar(&mut eventos, "window-rules/events/watch")?;

    consultar(ruta, estado)?;

    loop {
        // Cualquier evento significa «algo cambió»: se reconsulta en vez de
        // interpretar cada tipo. Es lo que hace `vasak-desktop`, y es lo que
        // sobrevive a que Wayfire agregue eventos nuevos.
        leer(&mut eventos)?;
        consultar(ruta, estado)?;
    }
}

fn consultar(ruta: &str, estado: &NoMolestar) -> std::io::Result<()> {
    let mut socket = UnixStream::connect(ruta)?;
    let respuesta = enviar(&mut socket, "window-rules/get-focused-view")?;
    estado.fijar(del_json(&respuesta));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// La pantalla completa es la regla principal: cubre la mayoría de los
    /// juegos sin nombrar ninguno.
    #[test]
    fn a_pantalla_completa_siempre_hay_que_apartarse() {
        assert!(es_un_juego("cualquier.cosa", true));
    }

    /// Y la lista es para los que corren en ventana sin bordes, que en Linux no
    /// es raro.
    #[test]
    fn los_de_la_lista_cuentan_aunque_esten_en_ventana() {
        for id in ["gamescope", "steam", "lutris", "heroic", "bottles"] {
            assert!(es_un_juego(id, false), "{id} tendría que contar");
        }
    }

    /// Un solo prefijo tiene que alcanzar para los mil juegos de Steam.
    #[test]
    fn los_juegos_de_steam_entran_por_prefijo() {
        assert!(es_un_juego("steam_app_570", false));
        assert!(es_un_juego("steam_app_1091500", false));
    }

    /// Los `app-id` no tienen convención de mayúsculas.
    #[test]
    fn la_comparacion_no_mira_mayusculas() {
        assert!(es_un_juego("Steam", false));
        assert!(es_un_juego("GameScope", false));
    }

    /// Y lo más importante: escribiendo en una aplicación normal el selector
    /// tiene que seguir apareciendo. Un falso positivo acá apaga la función.
    #[test]
    fn una_aplicacion_normal_no_es_un_juego() {
        for id in [
            "org.gnome.TextEditor",
            "com.anthropic.Claude",
            "vasak-terminal",
            "firefox",
        ] {
            assert!(!es_un_juego(id, false), "{id} no es un juego");
        }
    }

    /// Un nombre de la lista se compara entero, no como prefijo: si no,
    /// cualquier cosa que empiece igual quedaría tomada por un juego y el
    /// selector se apagaría adentro.
    #[test]
    fn un_nombre_parecido_no_alcanza() {
        assert!(!es_un_juego("steamboat-editor", false));
        assert!(!es_un_juego("heroic-typography", false));
        assert!(!es_un_juego("libreoffice-writer", false));
    }

    #[test]
    fn se_lee_la_respuesta_envuelta_en_info() {
        let r = json!({"info": {"app-id": "steam_app_440", "fullscreen": false}});
        assert!(del_json(&r));
    }

    #[test]
    fn se_lee_tambien_sin_el_envoltorio() {
        let r = json!({"app-id": "algo", "fullscreen": true});
        assert!(del_json(&r));
    }

    /// Sin ventana enfocada no hay juego. Con el escritorio vacío la respuesta
    /// no trae `info`, y tomar eso por un juego apagaría el selector.
    #[test]
    fn sin_ventana_enfocada_no_hay_juego() {
        assert!(!del_json(&json!({})));
        assert!(!del_json(&json!({"info": null})));
    }

    /// Una respuesta rara no puede apagar el selector.
    #[test]
    fn una_respuesta_sin_los_campos_no_apaga_nada() {
        assert!(!del_json(&json!({"info": {"title": "algo"}})));
    }
}
