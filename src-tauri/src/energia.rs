//! Suspender y reanudar, que es cuando el teclado se queda muerto.
//!
//! # El problema
//!
//! Este daemon toma los teclados en **exclusiva** (`EVIOCGRAB`) y replica todo
//! por un teclado virtual: el compositor no ve el teclado físico, ve el de
//! uinput. Eso lo convierte en punto único de falla de todo el teclado — los
//! atajos del compositor incluidos—, y no había nada que reaccionara a una
//! suspensión: los dispositivos se enumeran una vez al arrancar y nunca más.
//!
//! Si al reanudar el descriptor viejo dejó de servir, el daemon se queda con un
//! grab que no lee nada y no hay forma de recuperar el teclado sin apagar la
//! máquina a la fuerza. No es hipotético: pasó.
//!
//! # Lo que hace
//!
//! Escucha `PrepareForSleep` de logind, que es la señal que existe justamente
//! para esto. Llega **dos veces** por ciclo: con `true` antes de dormir y con
//! `false` después de despertar.
//!
//! - Antes de dormir se sueltan los grabs. Mientras la máquina duerme nadie
//!   escribe, así que no se pierde nada, y si el daemon no sobreviviera al ciclo
//!   el teclado igual queda usable.
//! - Al despertar se vuelven a enumerar los teclados desde cero y se toman de
//!   nuevo. Enumerar y no reusar es a propósito: si el dispositivo se fue y
//!   volvió, es otro nodo, y el descriptor viejo no apunta a nada.
//!
//! Va por el bus **de sistema**, que es donde vive logind, y en su propio hilo:
//! el bucle de entrada no puede bloquearse esperando señales de D-Bus.

use std::sync::mpsc::Sender;

/// Lo que el bucle de entrada tiene que hacer con los teclados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Energia {
    /// La máquina se va a dormir: soltar los teclados.
    VaADormir,
    /// La máquina despertó: volver a buscarlos y tomarlos.
    Desperto,
}

/// Traduce la señal de logind a lo que significa acá.
///
/// `PrepareForSleep(true)` es «me voy a dormir» y `false` es «desperté». Se
/// separa de la escucha para poder probarlo sin un bus.
pub fn que_hacer(va_a_dormir: bool) -> Energia {
    if va_a_dormir {
        Energia::VaADormir
    } else {
        Energia::Desperto
    }
}

/// Escucha a logind en un hilo aparte y avisa por el canal.
///
/// No devuelve error: si no hay bus de sistema —o logind no está—, se pierde la
/// recuperación automática pero el daemon tiene que seguir andando igual. Se
/// avisa por el registro, que es donde alguien lo va a buscar cuando el teclado
/// no vuelva.
pub fn escuchar(tx: Sender<Energia>) {
    std::thread::spawn(move || {
        if let Err(e) = escuchar_de_verdad(&tx) {
            eprintln!(
                "No se pudo escuchar a logind ({e}). El teclado va a seguir \
                 funcionando, pero los acentos pueden dejar de andar después de \
                 suspender: reiniciá vasak-press-and-hold si pasa."
            );
        }
    });
}

fn escuchar_de_verdad(tx: &Sender<Energia>) -> Result<(), zbus::Error> {
    let conexion = zbus::blocking::Connection::system()?;
    let proxy = zbus::blocking::Proxy::new(
        &conexion,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )?;

    for senal in proxy.receive_signal("PrepareForSleep")? {
        let va_a_dormir: bool = match senal.body().deserialize() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("PrepareForSleep con un cuerpo que no se entiende: {e}");
                continue;
            }
        };
        // Si el bucle de entrada se fue, no hay a quién avisarle.
        if tx.send(que_hacer(va_a_dormir)).is_err() {
            return Ok(());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_senal_dice_las_dos_cosas() {
        // Llega dos veces por ciclo, y confundirlas es soltar los teclados al
        // despertar y tomarlos justo antes de dormir: exactamente al revés.
        assert_eq!(que_hacer(true), Energia::VaADormir);
        assert_eq!(que_hacer(false), Energia::Desperto);
    }
}
