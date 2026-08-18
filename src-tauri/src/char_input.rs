//! Typing a character the keyboard cannot type.
//!
//! The accent picker offers é, ā, ø… and none of those are keys. Injection used
//! to work by searching the person's own layout for a key that produces the
//! character and pressing it, which is only possible for the handful of
//! characters that layout happens to have: on a Latin American keyboard ñ is a
//! key, so ñ worked, and everything else silently typed nothing at all. The
//! search cannot succeed for é there, because é is written with the dead acute
//! followed by e — there is no key that means é.
//!
//! So instead of looking for the character in somebody else's keymap, we bring
//! our own. Wayland's virtual keyboard protocol lets a client hand the
//! compositor a keymap and then press keys on it; ours has exactly one key per
//! character the picker can offer. It is the same thing wtype does, and it is
//! the only way that does not depend on which keyboard is plugged in.

use std::collections::HashMap;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::io::{Seek, Write};

use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

/// The keyboard that types the characters the picker offers.
pub struct CharKeyboard {
    connection: Connection,
    keyboard: ZwpVirtualKeyboardV1,
    /// Which key on our keymap types each character.
    keys: HashMap<char, u32>,
    /// Wayland wants a timestamp on every key. It only has to move forward.
    time: u32,
}

#[derive(Default)]
struct Globals {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<ZwpVirtualKeyboardManagerV1>,
}

impl CharKeyboard {
    /// Builds a keymap with one key per character and hands it to the
    /// compositor.
    pub fn new(chars: &[char]) -> Result<Self, String> {
        if chars.is_empty() {
            return Err("no hay caracteres que escribir".into());
        }

        let connection = Connection::connect_to_env()
            .map_err(|e| format!("no se pudo conectar al compositor: {e}"))?;
        let display = connection.display();
        let mut queue: EventQueue<Globals> = connection.new_event_queue();
        let handle = queue.handle();
        display.get_registry(&handle, ());

        let mut globals = Globals::default();
        queue
            .roundtrip(&mut globals)
            .map_err(|e| format!("no se pudo leer la lista de protocolos: {e}"))?;

        let seat = globals.seat.ok_or("el compositor no ofrece wl_seat")?;
        let manager = globals.manager.ok_or(
            "el compositor no implementa zwp_virtual_keyboard_manager_v1: \
             sin eso no hay forma de escribir un carácter que no esté en el teclado",
        )?;

        let keyboard = manager.create_virtual_keyboard(&seat, &handle, ());

        let keys: HashMap<char, u32> = chars
            .iter()
            .enumerate()
            .map(|(i, ch)| (*ch, i as u32 + 1))
            .collect();

        let keymap = keymap_for(chars);
        let file = memfd(&keymap)?;
        keyboard.keymap(1 /* xkb_v1 */, file.as_fd(), keymap.len() as u32);

        // Sin esto, un Shift que quedó apretado en el teclado real se aplicaría
        // también a lo que escribimos acá.
        keyboard.modifiers(0, 0, 0, 0);

        connection
            .flush()
            .map_err(|e| format!("no se pudo enviar el mapa de teclado: {e}"))?;

        Ok(Self {
            connection,
            keyboard,
            keys,
            time: 1,
        })
    }

    /// Types one character.
    pub fn type_char(&mut self, ch: char) -> Result<(), String> {
        let key = *self
            .keys
            .get(&ch)
            .ok_or_else(|| format!("'{ch}' no está en el mapa de teclado propio"))?;

        self.time = self.time.wrapping_add(1).max(1);
        self.keyboard.key(self.time, key, 1);
        self.time = self.time.wrapping_add(1).max(1);
        self.keyboard.key(self.time, key, 0);

        self.connection
            .flush()
            .map_err(|e| format!("no se pudo escribir '{ch}': {e}"))
    }
}

/// The keymap text: one key per character, each one a Unicode keysym.
fn keymap_for(chars: &[char]) -> String {
    let mut codes = String::new();
    let mut symbols = String::new();

    for (i, ch) in chars.iter().enumerate() {
        // Keycode 8 is where evdev's 0 lands, so ours start right after it.
        let code = i + 9;
        codes.push_str(&format!("    <K{i}> = {code};\n"));
        symbols.push_str(&format!("    key <K{i}> {{ [ U{:04X} ] }};\n", *ch as u32));
    }

    format!(
        "xkb_keymap {{\n\
         xkb_keycodes \"vasak\" {{\n    minimum = 8;\n    maximum = {max};\n{codes}}};\n\
         xkb_types \"vasak\" {{ include \"complete\" }};\n\
         xkb_compat \"vasak\" {{ include \"complete\" }};\n\
         xkb_symbols \"vasak\" {{\n{symbols}}};\n\
         }};\n",
        max = chars.len() + 9,
    )
}

/// The keymap travels to the compositor as a file descriptor.
fn memfd(contents: &str) -> Result<OwnedFd, String> {
    let fd = unsafe { libc::memfd_create(b"vasak-keymap\0".as_ptr() as *const _, 0) };
    if fd < 0 {
        return Err(format!(
            "no se pudo crear el archivo del mapa: {}",
            std::io::Error::last_os_error()
        ));
    }

    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(contents.as_bytes())
        .map_err(|e| format!("no se pudo escribir el mapa: {e}"))?;
    file.rewind()
        .map_err(|e| format!("no se pudo rebobinar el mapa: {e}"))?;

    Ok(OwnedFd::from(file))
}

impl Dispatch<wl_registry::WlRegistry, ()> for Globals {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_seat" => {
                    state.seat = Some(registry.bind(name, version.min(7), handle, ()));
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.manager = Some(registry.bind(name, version.min(1), handle, ()));
                }
                _ => {}
            }
        }
    }
}

macro_rules! sin_eventos {
    ($($tipo:ty),*) => {$(
        impl Dispatch<$tipo, ()> for Globals {
            fn event(
                _: &mut Self,
                _: &$tipo,
                _: <$tipo as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

// Ninguno de estos nos manda nada que nos importe: sólo escribimos.
sin_eventos!(wl_seat::WlSeat, ZwpVirtualKeyboardManagerV1, ZwpVirtualKeyboardV1);

#[cfg(test)]
mod tests {
    use super::*;

    /// El mapa tiene que compilar y cada tecla tiene que dar exactamente el
    /// carácter que le pedimos. Se prueba con la misma biblioteca que usa el
    /// compositor, así que no hace falta ni compositor ni teclado.
    #[test]
    fn el_mapa_escribe_cada_caracter_que_le_pedimos() {
        use xkbcommon::xkb;

        let chars = ['é', 'ñ', 'ā', 'ø', 'ç'];
        let texto = keymap_for(&chars);

        let contexto = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let mapa = xkb::Keymap::new_from_string(
            &contexto,
            texto.clone(),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .unwrap_or_else(|| panic!("el mapa no compila:\n{texto}"));

        let estado = xkb::State::new(&mapa);
        for (i, ch) in chars.iter().enumerate() {
            let keycode: xkb::Keycode = ((i + 9) as u32).into();
            assert_eq!(
                estado.key_get_utf8(keycode),
                ch.to_string(),
                "la tecla {i} no escribe {ch}"
            );
        }
    }
}
