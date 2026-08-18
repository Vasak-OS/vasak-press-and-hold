use std::collections::HashMap;
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use evdev::{Device, InputEventKind, Key};
use tauri::{AppHandle, Emitter};

use crate::accent_map::{char_to_xkb_keysym, get_variants, key_to_base_char};
use crate::uinput::VirtualKeyboard;

const HOLD_THRESHOLD: Duration = Duration::from_millis(400);

pub enum InjectCommand {
    Char(char),
}

/// The key we are still deciding about: letter, or the start of a hold?
///
/// There is only ever one. Anything else that happens settles it first, and
/// that is what keeps letters in the order they were typed.
struct Pending {
    code: u16,
    pressed_at: Instant,
    /// The picker is open for this key, so releasing it types nothing: the
    /// character is going to come from whichever variant gets chosen.
    picker_open: bool,
}

/// What one key event should produce.
///
/// Kept apart from the writing so the ordering — the thing this module gets
/// wrong when it gets anything wrong — can be tested without a virtual
/// keyboard and without a compositor.
#[derive(Debug, PartialEq)]
enum Action {
    /// Type the key: press and release, in one go.
    Tap(u16),
    /// Pass the event through untouched.
    Forward(u16, i32),
    /// Show the accent picker for this key.
    OpenPicker(u16),
}

/// Decides what to do with one key event.
///
/// A key that has accented variants cannot be typed the moment it goes down:
/// we do not know yet whether it is a letter or the beginning of a hold. It
/// used to be typed on release, and that is what scrambled fast typing —
/// "as" comes out as "sa" the moment you press the second key before letting
/// go of the first, which is how anybody types above a certain speed. It was
/// invisible in a text editor, where you notice and fix it, and brutal in a
/// password field, where you cannot see what you typed: the lock screen kept
/// rejecting the right password.
///
/// So the wait ends at the *next* key event rather than at the release.
/// Whatever happens next settles the pending key first, in the order it was
/// pressed. A hold is by definition a key with nothing happening after it, so
/// the picker still works exactly as before.
fn decide(
    pending: &mut Option<Pending>,
    code: u16,
    value: i32,
    now: Instant,
    is_target: bool,
) -> Vec<Action> {
    let mut actions = Vec::new();

    // Any event about a different key settles what was pending — a release
    // included: flushing a letter after Shift went back up would turn the
    // capital into a lowercase one.
    if pending.as_ref().is_some_and(|p| p.code != code) {
        if let Some(p) = pending.take() {
            if !p.picker_open {
                actions.push(Action::Tap(p.code));
            }
        }
    }

    if !is_target {
        actions.push(Action::Forward(code, value));
        return actions;
    }

    match value {
        // Press: nothing to type yet.
        1 => {
            *pending = Some(Pending {
                code,
                pressed_at: now,
                picker_open: false,
            });
        }
        // Repeat: the key is being held and nothing else has happened.
        2 => {
            if let Some(p) = pending.as_mut() {
                if !p.picker_open && now.duration_since(p.pressed_at) >= HOLD_THRESHOLD {
                    p.picker_open = true;
                    actions.push(Action::OpenPicker(code));
                }
            }
        }
        // Release: if the picker never opened this was a letter, however long
        // it was held. Checking the elapsed time here as well used to swallow
        // letters that have no accented variants.
        0 => {
            if let Some(p) = pending.take() {
                if !p.picker_open {
                    actions.push(Action::Tap(p.code));
                }
            }
        }
        _ => {}
    }

    actions
}

#[derive(Clone, serde::Serialize)]
struct AccentPayload {
    base_char: String,
    variants: Vec<String>,
}

pub fn find_keyboard_devices() -> Vec<Device> {
    let mut devices = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/dev/input") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map_or(false, |n| n.starts_with("event"))
            {
                if let Ok(device) = Device::open(&path) {
                    // Never our own virtual keyboard.
                    //
                    // It declares every keycode, so it satisfies the KEY_A test
                    // like any real keyboard. Grabbing it takes every key we
                    // replay through it away from the compositor and feeds it
                    // straight back into this loop: the keyboard goes dead and
                    // the process spins. This is normally impossible because the
                    // enumeration below runs before the device is created, but a
                    // second instance — or one left over from a crash — would
                    // put it there, and the cost of being wrong is a machine
                    // nobody can type on.
                    if device.name() == Some(crate::uinput::DEVICE_NAME) {
                        continue;
                    }
                    if let Some(keys) = device.supported_keys() {
                        if keys.contains(Key::KEY_A) {
                            devices.push(device);
                        }
                    }
                }
            }
        }
    }
    if devices.is_empty() {
        eprintln!(
            "No keyboard devices found in /dev/input/. \
             Check permissions: ls -la /dev/input/event*"
        );
    }
    devices
}

/// How long to keep trying to open /dev/uinput before giving up.
///
/// At login the daemon can easily win the race against udev and logind, which
/// are what put the session's ACL on the device. Retrying costs nothing and
/// turns a permanent failure into a slightly slower start.
const UINPUT_ATTEMPTS: u32 = 10;
const UINPUT_RETRY: std::time::Duration = std::time::Duration::from_millis(500);

pub fn run_input_loop(
    app_handle: AppHandle,
    inject_rx: mpsc::Receiver<InjectCommand>,
    current_variants: Arc<Mutex<Vec<char>>>,
) {
    // The real keyboards are enumerated first, while the virtual one does not
    // exist yet, so it cannot end up in this list. The order used to be the
    // other way round and that was enough to lock the machine: the virtual
    // keyboard declares every keycode, so it matched the search, got grabbed
    // along with the rest, and every key replayed through it was captured by
    // this process instead of reaching the compositor.
    let mut devices = find_keyboard_devices();
    if devices.is_empty() {
        // No keyboard to watch means the feature does not exist, and a process
        // that stays up hides that from `systemctl --user status`.
        eprintln!("No keyboard devices found. Exiting.");
        std::process::exit(1);
    }

    let mut last_error = String::new();
    let mut keyboard = None;

    for attempt in 0..UINPUT_ATTEMPTS {
        match VirtualKeyboard::new() {
            Ok(vk) => {
                keyboard = Some(vk);
                break;
            }
            Err(e) => {
                last_error = e;
                if attempt + 1 < UINPUT_ATTEMPTS {
                    std::thread::sleep(UINPUT_RETRY);
                }
            }
        }
    }

    // Without the virtual keyboard there is no feature: every key it grabs has
    // to be replayed through it. Returning quietly left the process running and
    // systemd reporting the unit as healthy, which is how this went unnoticed —
    // `systemctl --user status` said "active (running)" while pressing and
    // holding did nothing at all. Exiting is what makes the failure visible.
    // Nothing is grabbed yet at this point, on purpose: the retry loop above can
    // take five seconds, and holding an exclusive grab while waiting for
    // /dev/uinput would leave the keyboard dead for exactly as long.
    let Some(vk) = keyboard else {
        eprintln!("Failed to create virtual keyboard: {}", last_error);
        std::process::exit(1);
    };

    // Exclusive access, and the feature cannot work without it.
    //
    // Everything typed is replayed through the virtual keyboard, so a device we
    // failed to grab keeps delivering to the compositor as well — every
    // keystroke arrives twice. A device that cannot be grabbed is therefore
    // dropped rather than used: no accents on that keyboard is a far better
    // outcome than doubled letters on it.
    devices.retain_mut(|device| match device.grab() {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "No se pudo tomar el control exclusivo de {}: {error}. \
                 Se ignora ese teclado para no duplicar lo que escribas. \
                 Suele ser permisos: revisá /dev/input/event* y las reglas de udev.",
                device.name().unwrap_or("teclado desconocido")
            );
            false
        }
    });

    if devices.is_empty() {
        eprintln!(
            "Ningún teclado pudo tomarse en exclusiva; el selector de acentos queda \
             desactivado. El teclado sigue funcionando normalmente."
        );
        return;
    }

    // Set devices to non-blocking mode
    for device in &mut devices {
        unsafe {
            let fd = device.as_raw_fd();
            let flags = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }

    let variants_map = get_variants();
    let mut pending: Option<Pending> = None;

    loop {
        // 1. Check inject channel
        while let Ok(cmd) = inject_rx.try_recv() {
            match cmd {
                InjectCommand::Char(ch) => {
                    if let Some(keysym) = char_to_xkb_keysym(ch) {
                        let _ = vk.send_keysym(keysym);
                    }
                }
            }
        }

        // 2. Read events from all devices
        for device in &mut devices {
            if let Ok(events) = device.fetch_events() {
                for event in events {
                    match event.kind() {
                        InputEventKind::Key(key) => {
                            let code = key.code();
                            let is_target = key_to_base_char(code).is_some();
                            for action in decide(
                                &mut pending,
                                code,
                                event.value(),
                                Instant::now(),
                                is_target,
                            ) {
                                run_action(
                                    action,
                                    &vk,
                                    &app_handle,
                                    &variants_map,
                                    &current_variants,
                                );
                            }
                        }
                        InputEventKind::Synchronization(_) => {
                            let _ = vk.send_event(0x00, 0, 0);
                        }
                        InputEventKind::RelAxis(axis) => {
                            let _ = vk.send_event(0x02, axis.0, event.value());
                        }
                        _ => {}
                    }
                }
            }
        }

        std::thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Códigos evdev de teclas con variantes acentuadas. Son opacos a propósito:
    // lo que se prueba es el orden, no el mapa de teclado.
    const A: u16 = 30;
    const S: u16 = 31;
    const SHIFT: u16 = 42;

    /// El bug que hacía que la pantalla de bloqueo rechazara la contraseña
    /// correcta: escribiendo rápido se aprieta la segunda tecla antes de soltar
    /// la primera, y como la letra se escribía al soltar, "as" salía "sa".
    #[test]
    fn escribir_rapido_respeta_el_orden_en_que_se_tecleo() {
        let ahora = Instant::now();
        let mut pending = None;
        let mut hechas = Vec::new();

        hechas.extend(decide(&mut pending, A, 1, ahora, true));
        hechas.extend(decide(&mut pending, S, 1, ahora, true));
        hechas.extend(decide(&mut pending, A, 0, ahora, true)); // se suelta después
        hechas.extend(decide(&mut pending, S, 0, ahora, true));

        assert_eq!(hechas, vec![Action::Tap(A), Action::Tap(S)]);
    }

    /// Una letra sola sigue escribiéndose una única vez.
    #[test]
    fn una_tecla_suelta_escribe_una_letra() {
        let ahora = Instant::now();
        let mut pending = None;
        assert!(decide(&mut pending, A, 1, ahora, true).is_empty());
        assert_eq!(decide(&mut pending, A, 0, ahora, true), vec![Action::Tap(A)]);
    }

    /// Mantener apretado abre el selector y no escribe la letra base: eso lo
    /// hace después la variante elegida.
    #[test]
    fn mantener_apretado_abre_el_selector_y_no_escribe_la_letra() {
        let ahora = Instant::now();
        let mut pending = None;
        decide(&mut pending, A, 1, ahora, true);

        let tarde = ahora + HOLD_THRESHOLD;
        assert_eq!(decide(&mut pending, A, 2, tarde, true), vec![Action::OpenPicker(A)]);
        assert!(decide(&mut pending, A, 2, tarde, true).is_empty(), "el selector se abre una sola vez");
        assert!(decide(&mut pending, A, 0, tarde, true).is_empty());
    }

    /// Una tecla sin variantes —o un modificador— también ordena lo pendiente.
    /// Soltar Shift antes de escribir la letra pendiente la habría convertido en
    /// minúscula.
    #[test]
    fn una_tecla_ajena_ordena_lo_pendiente_antes_de_pasar() {
        let ahora = Instant::now();
        let mut pending = None;
        decide(&mut pending, A, 1, ahora, true);

        assert_eq!(
            decide(&mut pending, SHIFT, 0, ahora, false),
            vec![Action::Tap(A), Action::Forward(SHIFT, 0)]
        );
        // Al soltarla más tarde ya no escribe nada: la letra ya salió.
        assert!(decide(&mut pending, A, 0, ahora, true).is_empty());
    }

    /// The bug that locked the machine: the virtual keyboard declares every
    /// keycode, so it matches the same search that looks for real keyboards. When
    /// it ended up in that list it got grabbed too, and every key replayed
    /// through it was captured by this process instead of reaching the
    /// compositor — the keyboard went dead and only killing the daemon from
    /// another machine or a TTY brought it back.
    ///
    /// Ignored by default because it needs /dev/uinput and read access to
    /// /dev/input/event*, which is what the shipped udev rule grants:
    /// `cargo test -- --ignored --nocapture`. It never grabs anything, so it
    /// cannot lock the keyboard it runs on.
    #[test]
    #[ignore]
    fn the_search_never_returns_our_own_virtual_keyboard() {
        let _vk = crate::uinput::VirtualKeyboard::new().expect("no se pudo crear el teclado virtual");
        // The device shows up asynchronously; the constructor already waits, but
        // give udev room so a miss here means the filter worked, not that the
        // device was not there yet.
        std::thread::sleep(std::time::Duration::from_millis(300));

        let found = find_keyboard_devices();
        let names: Vec<String> = found
            .iter()
            .map(|d| d.name().unwrap_or("(sin nombre)").to_string())
            .collect();
        println!("teclados encontrados: {names:?}");

        assert!(
            !names.iter().any(|n| n == crate::uinput::DEVICE_NAME),
            "la búsqueda devolvió nuestro propio teclado virtual: {names:?}"
        );
    }
}

/// Carries out one decision.
fn run_action(
    action: Action,
    vk: &VirtualKeyboard,
    app: &AppHandle,
    variants_map: &HashMap<char, Vec<char>>,
    current_variants: &Arc<Mutex<Vec<char>>>,
) {
    match action {
        Action::Tap(code) => {
            let _ = vk.send_event(0x01, code, 1);
            let _ = vk.send_event(0x01, code, 0);
        }
        Action::Forward(code, value) => {
            let _ = vk.send_event(0x01, code, value);
        }
        Action::OpenPicker(code) => {
            if let Some(base) = key_to_base_char(code) {
                if let Some(vars) = variants_map.get(&base) {
                    let payload = AccentPayload {
                        base_char: base.to_string(),
                        variants: vars.iter().map(|c| c.to_string()).collect(),
                    };
                    {
                        let mut cv = current_variants.lock().unwrap();
                        *cv = vars.clone();
                    }
                    let _ = app.emit("show-accent-menu", payload);
                }
            }
        }
    }
}
