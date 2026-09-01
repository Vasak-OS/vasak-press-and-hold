use std::collections::HashMap;
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use evdev::{Device, InputEventKind, Key};
use tauri::{AppHandle, Emitter};

use crate::accent_map::{char_to_xkb_keysym, get_variants, key_to_base_char};
use crate::char_input::CharKeyboard;
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

/// The accent picker, as far as the keyboard is concerned.
///
/// While it is on screen the number keys belong to it. They have to be caught
/// here rather than in the window, because the window deliberately does not
/// take the keyboard: if it did, the accent it types would land in the picker
/// instead of in whatever you were writing. So pressing 1 used to type a 1
/// into the document and leave the ñ unreachable from the keyboard — the only
/// way to pick a variant was the mouse, which defeats the point of an accent
/// picker you opened while typing.
#[derive(Default)]
struct Picker {
    open: bool,
    /// How many variants are on screen. A number past the last one does
    /// nothing, instead of typing a digit into the text underneath.
    count: usize,
}

/// Everything the input loop remembers between events.
#[derive(Default)]
struct State {
    pending: Option<Pending>,
    picker: Picker,
    /// Keys whose press the picker consumed. Their release has to be consumed
    /// too, or the compositor gets a key going up that never went down.
    swallowed: Vec<u16>,
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
    /// Type the variant at this position and close the picker.
    Choose(usize),
    /// Close the picker without typing anything.
    ClosePicker,
}

const KEY_ESC: u16 = 1;
const KEY_ENTER: u16 = 28;

/// The position a number key refers to, counting from zero.
///
/// The picker labels its variants 1, 2, 3…, so 1 means the first one. The
/// numeric keypad answers the same, for whoever has their hand there.
fn digit_index(code: u16) -> Option<usize> {
    const ROW: [u16; 10] = [2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
    const PAD: [u16; 10] = [79, 80, 81, 75, 76, 77, 71, 72, 73, 82];
    ROW.iter()
        .chain(PAD.iter())
        .position(|c| *c == code)
        .map(|i| i % 10)
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
fn decide(st: &mut State, code: u16, value: i32, now: Instant, is_target: bool) -> Vec<Action> {
    // A key whose press the picker ate must not deliver its release either.
    if value != 1 && st.swallowed.contains(&code) {
        if value == 0 {
            st.swallowed.retain(|c| *c != code);
        }
        return Vec::new();
    }

    let mut actions = Vec::new();

    // With the picker on screen, the numbers are its own.
    if st.picker.open && value == 1 {
        if let Some(index) = digit_index(code) {
            st.swallowed.push(code);
            if index < st.picker.count {
                st.picker.open = false;
                return vec![Action::Choose(index)];
            }
            // A number nobody offered: better to swallow it than to type it.
            return Vec::new();
        }

        if code == KEY_ESC {
            st.swallowed.push(code);
            st.picker.open = false;
            return vec![Action::ClosePicker];
        }

        if code == KEY_ENTER && st.picker.count > 0 {
            st.swallowed.push(code);
            st.picker.open = false;
            return vec![Action::Choose(0)];
        }

        // Anything else means the picker was not what they were after: it gets
        // out of the way and the key goes through as usual.
        st.picker.open = false;
        actions.push(Action::ClosePicker);
    }

    // Any event about a different key settles what was pending — a release
    // included: flushing a letter after Shift went back up would turn the
    // capital into a lowercase one.
    if st.pending.as_ref().is_some_and(|p| p.code != code) {
        if let Some(p) = st.pending.take() {
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
            st.pending = Some(Pending {
                code,
                pressed_at: now,
                picker_open: false,
            });
        }
        // Repeat: the key is being held and nothing else has happened.
        2 => {
            if let Some(p) = st.pending.as_mut() {
                if !p.picker_open && now.duration_since(p.pressed_at) >= HOLD_THRESHOLD {
                    p.picker_open = true;
                    st.picker.open = true;
                    actions.push(Action::OpenPicker(code));
                }
            }
        }
        // Release: if the picker never opened this was a letter, however long
        // it was held. Checking the elapsed time here as well used to swallow
        // letters that have no accented variants.
        0 => {
            if let Some(p) = st.pending.take() {
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
pub struct AccentPayload {
    base_char: String,
    /// Público porque `picker_window` necesita cuántas hay para darle el tamaño
    /// correcto a la ventana que crea.
    pub variants: Vec<String>,
}

pub fn find_keyboard_devices() -> Vec<Device> {
    let mut devices = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/dev/input") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("event"))
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

/// Cuántas teclas seguidas se pueden perder antes de soltar los teclados.
///
/// No es cero porque un fallo aislado escribiendo en uinput no justifica quedarse
/// sin acentos, y no es grande porque cada intento fallido es una tecla que la
/// persona apretó y no llegó a ninguna parte. Tres es lo que se nota como un
/// tropiezo y no como un teclado roto.
const FALLOS_ANTES_DE_SOLTAR: u32 = 3;

/// Cuántas teclas seguidas se perdieron al replicarlas.
///
/// Cuenta **seguidas** y no en total: un tropiezo aislado a lo largo de un día
/// entero no dice nada, y tres al hilo dicen que la reinyección dejó de andar.
#[derive(Default)]
struct Fallos(u32);

impl Fallos {
    /// Anota cómo salió una tecla. Una que sale bien borra las anteriores.
    fn anotar(&mut self, salio_bien: bool) {
        self.0 = if salio_bien { 0 } else { self.0 + 1 };
    }

    fn hay_que_soltar(&self) -> bool {
        self.0 >= FALLOS_ANTES_DE_SOLTAR
    }

    fn cuantos(&self) -> u32 {
        self.0
    }
}

/// Toma los teclados en exclusiva y los deja en modo no bloqueante.
///
/// Los que no se pueden tomar se descartan de la lista en vez de usarse: todo lo
/// que se teclea se replica por el teclado virtual, así que un dispositivo que
/// siguió entregando al compositor manda cada tecla dos veces. Sin acentos en
/// ese teclado es mucho mejor que letras duplicadas en él.
fn tomar_en_exclusiva(devices: &mut Vec<Device>) {
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

    for device in devices.iter_mut() {
        unsafe {
            let fd = device.as_raw_fd();
            let flags = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

/// Suelta todos los teclados. Desde acá los ve el compositor otra vez.
///
/// Es lo que separa «los acentos dejaron de andar» de «el teclado no responde»:
/// mientras este proceso los tenga tomados, nada de lo que se escriba llega a
/// ninguna parte si la reinyección falla.
fn soltar(devices: &mut [Device]) {
    for device in devices.iter_mut() {
        if let Err(error) = device.ungrab() {
            eprintln!(
                "No se pudo soltar {}: {error}",
                device.name().unwrap_or("teclado desconocido")
            );
        }
    }
}

/// Suelta lo que haya y vuelve a buscar los teclados desde cero. Se usa al
/// despertar.
///
/// Recibe la lista y la reemplaza, en vez de devolver una nueva, **por el
/// orden**: soltar tiene que pasar antes de volver a tomar, y con dos llamadas
/// separadas el orden queda librado a quien las escriba. Con
/// `devices = volver_a_tomar()` quedaba al revés, porque Rust evalúa el lado
/// derecho antes de soltar el valor viejo: los descriptores de antes seguían
/// agarrando el teclado mientras se intentaba tomarlo de nuevo, `EVIOCGRAB`
/// contestaba `EBUSY` —el kernel sólo admite un cliente por dispositivo— y el
/// teclado se descartaba de la lista. Quedaba sin acentos hasta reiniciar el
/// servicio.
///
/// No pasaba mientras la señal de «me voy a dormir» llegara, porque esa ya había
/// soltado todo. Dependía de recibir las dos mitades, y una de ellas se puede
/// perder: la suscripción a D-Bus tarda en armarse, y una suspensión en ese
/// hueco no avisa. Así el despertar se arregla solo, haya llegado o no.
///
/// Se enumera de cero en vez de reusar lo que había: si el dispositivo se fue y
/// volvió es otro nodo, y el descriptor viejo no apunta a nada. Los teclados
/// virtuales se filtran solos —`find_keyboard_devices` ya los descarta—, así que
/// el nuestro no se toma a sí mismo.
fn volver_a_tomar(devices: &mut Vec<Device>) {
    soltar(devices);
    devices.clear();

    *devices = find_keyboard_devices();
    tomar_en_exclusiva(devices);

    if devices.is_empty() {
        eprintln!(
            "Al despertar no se pudo tomar ningún teclado. El teclado funciona, \
             pero sin acentos hasta reiniciar vasak-press-and-hold."
        );
    }
}

pub fn run_input_loop(
    app_handle: AppHandle,
    inject_rx: mpsc::Receiver<InjectCommand>,
    current_variants: Arc<Mutex<Vec<char>>>,
) {
    // Suspender y despertar. Se arma antes de tomar nada: si la señal llega
    // mientras se está preparando, el canal la guarda.
    let (energia_tx, energia_rx) = mpsc::channel();
    crate::energia::escuchar(energia_tx);
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
    tomar_en_exclusiva(&mut devices);

    if devices.is_empty() {
        eprintln!(
            "Ningún teclado pudo tomarse en exclusiva; el selector de acentos queda \
             desactivado. El teclado sigue funcionando normalmente."
        );
        return;
    }

    let variants_map = get_variants();

    // El teclado que escribe los acentos. Es otro distinto del que replica lo
    // que tecleás: aquél manda códigos de tecla, que el compositor interpreta
    // con *tu* distribución, y por eso sólo podía escribir lo que tu teclado ya
    // tiene. Éste trae su propio mapa con una tecla por variante.
    let todas: Vec<char> = {
        let mut v: Vec<char> = variants_map.values().flatten().copied().collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let mut char_kb = match CharKeyboard::new(&todas) {
        Ok(kb) => Some(kb),
        Err(e) => {
            eprintln!(
                "No se pudo crear el teclado de caracteres: {e}.                  Los acentos se van a intentar con la distribución del sistema,                  que sólo puede escribir los que tu teclado ya tenga."
            );
            None
        }
    };
    let mut state = State::default();

    let mut fallos = Fallos::default();

    loop {
        // 0. Dormir y despertar.
        //
        // Va antes que nada: si la máquina está por suspender, lo único que
        // importa es soltar los teclados antes de que se apague.
        while let Ok(evento) = energia_rx.try_recv() {
            match evento {
                crate::energia::Energia::VaADormir => {
                    soltar(&mut devices);
                    devices.clear();
                    // El estado se limpia junto con los teclados: una tecla que
                    // quedó apretada al dormir nunca manda su release, y sin
                    // esto el selector podía despertar abierto y comiéndose lo
                    // que se escribiera.
                    state = State::default();
                    hide_picker(&app_handle);
                }
                crate::energia::Energia::Desperto => {
                    volver_a_tomar(&mut devices);
                    fallos.anotar(true);
                }
            }
        }

        // 1. Check inject channel
        while let Ok(cmd) = inject_rx.try_recv() {
            match cmd {
                InjectCommand::Char(ch) => escribir(ch, &mut char_kb, &vk),
            }
        }

        // 1b. Las luces del teclado.
        //
        // Los teclados de verdad están tomados en exclusiva, así que el
        // compositor no los ve: enciende y apaga las luces del teclado virtual,
        // que es el único que conoce. Sin replicarlo, la luz de Bloq Mayús no se
        // prende nunca —ni en el teclado ni para quien quiera leer ese estado—
        // aunque las mayúsculas se escriban bien.
        for (led, encendido) in vk.read_led_events() {
            let evento = evdev::InputEvent::new(
                evdev::EventType::LED,
                led,
                if encendido { 1 } else { 0 },
            );

            for device in &mut devices {
                if let Err(error) = device.send_events(&[evento]) {
                    eprintln!(
                        "No se pudo copiar la luz {led} a {}: {error}",
                        device.name().unwrap_or("teclado desconocido")
                    );
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
                            // La tecla bajó y tiene variantes: si el selector
                            // está frío, se empieza a construir ahora. Abre
                            // recién a los HOLD_THRESHOLD ms, así que la
                            // creación queda escondida detrás del gesto que la
                            // persona ya está haciendo. Con el selector tibio
                            // esto no hace nada, así que escribir no cuesta.
                            if is_target && event.value() == 1 {
                                crate::picker_window::warm_up(&app_handle);
                            }
                            let actions = decide(
                                &mut state,
                                code,
                                event.value(),
                                Instant::now(),
                                is_target,
                            );
                            for action in actions {
                                // How many variants are on screen decides which
                                // numbers the picker answers to.
                                if let Action::OpenPicker(code) = &action {
                                    state.picker.count = key_to_base_char(*code)
                                        .and_then(|base| variants_map.get(&base))
                                        .map_or(0, |vars| vars.len());
                                }
                                let salio = run_action(
                                    action,
                                    &vk,
                                    &mut char_kb,
                                    &app_handle,
                                    &variants_map,
                                    &current_variants,
                                );
                                fallos.anotar(salio);
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

        // La red de seguridad.
        //
        // Si la reinyección viene fallando, cada tecla que se aprieta se pierde:
        // los teclados de verdad están tomados y el compositor sólo ve el
        // virtual. Soltarlos devuelve el teclado —sin acentos, pero **vivo**— y
        // salir con error deja que systemd levante el daemon de nuevo con un
        // teclado virtual nuevo, que es lo que suele arreglarlo.
        //
        // Salir sin soltar sería casi lo mismo, porque los descriptores se
        // cierran igual, pero «casi» no alcanza para la única cosa que no puede
        // fallar acá.
        if fallos.hay_que_soltar() {
            eprintln!(
                "Se perdieron {} teclas seguidas al replicarlas por el teclado \
                 virtual. Se sueltan los teclados para que vuelvan a funcionar y se \
                 sale: systemd reinicia el daemon en unos segundos.",
                fallos.cuantos()
            );
            soltar(&mut devices);
            std::process::exit(1);
        }

        std::thread::sleep(Duration::from_millis(1));
    }
}


/// Carries out one decision.
/// Lleva a cabo una decisión. Devuelve `false` si no se pudo escribir en el
/// teclado virtual.
///
/// Ese valor no es decorativo: los teclados de verdad están tomados en
/// exclusiva, así que una tecla que no se logra replicar **no llega a ninguna
/// parte**. Antes se descartaba el error —`let _ = vk.send_event(...)`— y el
/// resultado era un teclado que no responde, sin una sola línea en el registro
/// que dijera por qué. Quien lo sufría no tenía cómo llegar ni a una terminal:
/// los atajos del compositor pasan por acá también.
fn run_action(
    action: Action,
    vk: &VirtualKeyboard,
    char_kb: &mut Option<CharKeyboard>,
    app: &AppHandle,
    variants_map: &HashMap<char, Vec<char>>,
    current_variants: &Arc<Mutex<Vec<char>>>,
) -> bool {
    match action {
        Action::Tap(code) => {
            let uno = vk.send_event(0x01, code, 1);
            let dos = vk.send_event(0x01, code, 0);
            if let Err(e) = uno.and(dos) {
                eprintln!("No se pudo escribir la tecla {code} en el teclado virtual: {e}");
                return false;
            }
        }
        Action::Forward(code, value) => {
            if let Err(e) = vk.send_event(0x01, code, value) {
                eprintln!("No se pudo replicar la tecla {code} ({value}): {e}");
                return false;
            }
        }
        Action::OpenPicker(code) => {
            traza(&format!("OpenPicker code={code}"));
            if let Some(base) = key_to_base_char(code) {
                if let Some(vars) = variants_map.get(&base) {
                    let payload = AccentPayload {
                        base_char: base.to_string(),
                        variants: vars.iter().map(|c| c.to_string()).collect(),
                    };
                    {
                        // Se recupera el guard en lugar de paniquear: un
                        // `unwrap` acá mata el hilo del teclado y deja de
                        // atender todas las teclas, no sólo los acentos.
                        let mut cv = current_variants
                            .lock()
                            .unwrap_or_else(|envenenado| envenenado.into_inner());
                        *cv = vars.clone();
                    }
                    crate::picker_window::deliver(app, payload);
                    show_picker(app, vars.len());
                }
            }
        }
        Action::Choose(index) => {
            traza(&format!("Choose {index}"));
            let chosen = current_variants
                .lock()
                .ok()
                .and_then(|vars| vars.get(index).copied());
            // The window goes away first: the character is typed into whatever
            // had the keyboard, and that is never the picker.
            hide_picker(app);
            if let Some(ch) = chosen {
                escribir(ch, char_kb, vk);
            }
        }
        Action::ClosePicker => {
            traza("ClosePicker");
            hide_picker(app);
        }
    }
    true
}

/// Escribe un carácter, con el teclado propio si lo hay.
///
/// La distribución del sistema queda de reserva: sirve para lo que el teclado
/// ya tiene —la ñ en latinoamericano, por ejemplo— y para nada más, así que es
/// mejor que nada pero no mucho más.
fn escribir(ch: char, char_kb: &mut Option<CharKeyboard>, vk: &VirtualKeyboard) {
    if let Some(kb) = char_kb.as_mut() {
        match kb.type_char(ch) {
            Ok(()) => return,
            Err(e) => traza(&format!("teclado propio: {e}")),
        }
    }

    // Que no se pueda escribir el acento elegido es una falla de verdad: la
    // persona eligió una variante y no apareció nada. Va al registro aunque no
    // esté puesta la traza.
    let fallo = match char_to_xkb_keysym(ch) {
        Some(keysym) => vk.send_keysym(keysym).err(),
        None => Some(format!("'{ch}' no tiene equivalente en la distribución")),
    };
    if let Some(e) = fallo {
        eprintln!("No se pudo escribir '{ch}': {e}");
    }
}

/// Medidas de la tarjeta, en la misma unidad que usa la página: cada variante
/// es un cuadrado de 44 px con 4 px de separación, más el relleno de la
/// tarjeta y lugar para la sombra.
// Las medidas viven en `picker_window`, que también las necesita para dar el
// tamaño correcto a la ventana que crea.
use crate::picker_window::tamano_para;

/// Pone el selector en pantalla.
///
/// La mostraba la página al recibir el evento, y no se mapeaba nunca: quedaba
/// en 0x0 e invisible, así que las variantes sólo se podían elegir a ciegas.
/// La ventana la abre ahora el mismo lado que decide abrirla, y la página se
/// ocupa únicamente de dibujar lo que hay adentro.
fn show_picker(app: &AppHandle, variantes: usize) {
    use tauri::Manager;

    let Some(win) = app.get_webview_window("main") else {
        traza("no hay ventana 'main'");
        return;
    };

    // La ventana se ajusta a cuántas variantes hay. Con un ancho fijo, la `a`
    // —que tiene siete— no entraba, y la `n` —que tiene dos— dejaba la tarjeta
    // perdida en una ventana enorme. La tarjeta se centra sola adentro.
    let (ancho, alto) = tamano_para(variantes);
    if let Err(e) = win.set_size(tauri::LogicalSize::new(ancho, alto)) {
        traza(&format!("no se pudo redimensionar: {e}"));
    }

    if let Err(e) = win.show() {
        traza(&format!("no se pudo mostrar: {e}"));
    }
    traza(&format!(
        "mostrada: visible={:?} tamaño={:?}",
        win.is_visible(),
        win.outer_size()
    ));
}

/// Deja rastro de lo que decide el demonio, para cuando lo que se ve en
/// pantalla y lo que el teclado creyó hacer no coinciden. Se activa con
/// VASAK_PAH_DEBUG=1.
fn traza(mensaje: &str) {
    if std::env::var_os("VASAK_PAH_DEBUG").is_some() {
        eprintln!("[press-and-hold] {mensaje}");
    }
}

/// Takes the picker off the screen.
///
/// Hiding the window is what the person sees; the event is so the page forgets
/// the variants it was showing, since it is the same window every time.
fn hide_picker(app: &AppHandle) {
    use tauri::Manager;

    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
    let _ = app.emit("hide-accent-menu", ());
    // Se cerró: si no se usa ningún acento por un rato largo, el webview entero
    // se desarma y el demonio vuelve a costar lo que cuesta un demonio.
    crate::picker_window::schedule_teardown(app);
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
        let mut st = State::default();
        let mut hechas = Vec::new();

        hechas.extend(decide(&mut st, A, 1, ahora, true));
        hechas.extend(decide(&mut st, S, 1, ahora, true));
        hechas.extend(decide(&mut st, A, 0, ahora, true)); // se suelta después
        hechas.extend(decide(&mut st, S, 0, ahora, true));

        assert_eq!(hechas, vec![Action::Tap(A), Action::Tap(S)]);
    }

    /// Una letra sola sigue escribiéndose una única vez.
    #[test]
    fn una_tecla_suelta_escribe_una_letra() {
        let ahora = Instant::now();
        let mut st = State::default();
        assert!(decide(&mut st, A, 1, ahora, true).is_empty());
        assert_eq!(decide(&mut st, A, 0, ahora, true), vec![Action::Tap(A)]);
    }

    /// Mantener apretado abre el selector y no escribe la letra base: eso lo
    /// hace después la variante elegida.
    #[test]
    fn mantener_apretado_abre_el_selector_y_no_escribe_la_letra() {
        let ahora = Instant::now();
        let mut st = State::default();
        decide(&mut st, A, 1, ahora, true);

        let tarde = ahora + HOLD_THRESHOLD;
        assert_eq!(decide(&mut st, A, 2, tarde, true), vec![Action::OpenPicker(A)]);
        assert!(decide(&mut st, A, 2, tarde, true).is_empty(), "el selector se abre una sola vez");
        assert!(decide(&mut st, A, 0, tarde, true).is_empty());
    }


    /// El motivo del cambio: la ventana no toma el teclado —si lo tomara, el
    /// acento se escribiría dentro del selector y no en tu documento—, así que
    /// apretar 1 escribía un 1 y la ñ no se podía elegir con el teclado.
    #[test]
    fn con_el_selector_abierto_los_numeros_eligen_la_variante() {
        let ahora = Instant::now();
        let mut st = State::default();
        decide(&mut st, A, 1, ahora, true);
        decide(&mut st, A, 2, ahora + HOLD_THRESHOLD, true);
        st.picker.count = 5;

        const UNO: u16 = 2;
        assert_eq!(decide(&mut st, UNO, 1, ahora, false), vec![Action::Choose(0)]);
        // Y no escribe el número: ni la bajada ni la subida llegan a nadie.
        assert!(decide(&mut st, UNO, 0, ahora, false).is_empty());
    }

    /// Un número que el selector no ofrece no escribe un dígito suelto en el
    /// medio de la palabra.
    #[test]
    fn un_numero_sin_variante_no_escribe_nada() {
        let ahora = Instant::now();
        let mut st = State::default();
        decide(&mut st, A, 1, ahora, true);
        decide(&mut st, A, 2, ahora + HOLD_THRESHOLD, true);
        st.picker.count = 2;

        const NUEVE: u16 = 10;
        assert!(decide(&mut st, NUEVE, 1, ahora, false).is_empty());
        assert!(st.picker.open, "el selector sigue abierto esperando una opción válida");
    }

    /// Escape cierra sin escribir, y cualquier otra tecla cierra y sigue de
    /// largo: el selector nunca se queda trabado tapando lo que escribís.
    #[test]
    fn escape_cierra_y_otra_tecla_cierra_y_pasa() {
        let ahora = Instant::now();
        let mut st = State::default();
        decide(&mut st, A, 1, ahora, true);
        decide(&mut st, A, 2, ahora + HOLD_THRESHOLD, true);
        st.picker.count = 3;
        assert_eq!(decide(&mut st, KEY_ESC, 1, ahora, false), vec![Action::ClosePicker]);

        // Otra vez, ahora con una tecla cualquiera.
        let mut st = State::default();
        decide(&mut st, A, 1, ahora, true);
        decide(&mut st, A, 2, ahora + HOLD_THRESHOLD, true);
        st.picker.count = 3;
        decide(&mut st, A, 0, ahora, true); // se suelta la tecla sostenida
        assert_eq!(
            decide(&mut st, S, 1, ahora, true),
            vec![Action::ClosePicker],
            "cierra el selector y la letra queda pendiente como cualquier otra"
        );
        assert_eq!(decide(&mut st, S, 0, ahora, true), vec![Action::Tap(S)]);
    }

    /// Soltar la tecla sostenida no cierra el selector: se suelta, se mira y
    /// recién ahí se elige.
    /// La red que evita que un fallo replicando deje el teclado muerto.
    ///
    /// Los teclados de verdad están tomados en exclusiva, así que una tecla que
    /// no se logra escribir en el virtual no llega a ninguna parte. Si eso pasa
    /// varias veces seguidas hay que soltarlos: sin acentos se vive, sin teclado
    /// hay que apagar la máquina del botón.
    #[test]
    fn tres_teclas_perdidas_seguidas_sueltan_los_teclados() {
        let mut f = Fallos::default();
        assert!(!f.hay_que_soltar(), "recién arrancado no suelta nada");

        f.anotar(false);
        f.anotar(false);
        assert!(!f.hay_que_soltar(), "dos no alcanzan: un tropiezo no es una falla");

        f.anotar(false);
        assert!(f.hay_que_soltar());
        assert_eq!(f.cuantos(), 3);
    }

    #[test]
    fn una_tecla_que_sale_bien_borra_las_perdidas() {
        // Cuenta seguidas, no en total: dos fallos por mes no pueden acumularse
        // hasta soltar los teclados un martes cualquiera.
        let mut f = Fallos::default();
        f.anotar(false);
        f.anotar(false);
        f.anotar(true);
        assert_eq!(f.cuantos(), 0);

        f.anotar(false);
        f.anotar(false);
        assert!(!f.hay_que_soltar(), "{}", f.cuantos());
    }

    /// Por qué al dormir se limpia el estado y no sólo se sueltan los teclados.
    ///
    /// Una tecla que quedó apretada cuando la máquina se durmió no manda nunca su
    /// release. Si su código quedó en `swallowed`, el `decide` de después se come
    /// todos los eventos de esa tecla que no sean presión: la tecla queda muerta
    /// para siempre, o peor, apretada desde el punto de vista del compositor.
    #[test]
    fn el_estado_que_quedo_de_antes_de_dormir_no_sobrevive() {
        let mut st = State::default();
        st.picker.open = true;
        st.picker.count = 5;
        st.swallowed.push(KEY_ESC);
        st.pending = Some(Pending {
            code: 30,
            pressed_at: Instant::now(),
            picker_open: true,
        });

        // Con ese estado, Esc no produce nada: se lo come el `swallowed`.
        let antes = decide(&mut st, KEY_ESC, 0, Instant::now(), false);
        assert!(antes.is_empty(), "{antes:?}");

        // Al despertar se arranca de cero, que es lo que hace el bucle.
        let mut st = State::default();
        let despues = decide(&mut st, KEY_ESC, 0, Instant::now(), false);
        assert_eq!(despues, vec![Action::Forward(KEY_ESC, 0)]);
    }

    #[test]
    fn soltar_la_tecla_sostenida_deja_el_selector_abierto() {
        let ahora = Instant::now();
        let mut st = State::default();
        decide(&mut st, A, 1, ahora, true);
        decide(&mut st, A, 2, ahora + HOLD_THRESHOLD, true);
        st.picker.count = 4;

        assert!(decide(&mut st, A, 0, ahora, true).is_empty());
        assert!(st.picker.open);
    }

    /// Una tecla sin variantes —o un modificador— también ordena lo pendiente.
    /// Soltar Shift antes de escribir la letra pendiente la habría convertido en
    /// minúscula.
    #[test]
    fn una_tecla_ajena_ordena_lo_pendiente_antes_de_pasar() {
        let ahora = Instant::now();
        let mut st = State::default();
        decide(&mut st, A, 1, ahora, true);

        assert_eq!(
            decide(&mut st, SHIFT, 0, ahora, false),
            vec![Action::Tap(A), Action::Forward(SHIFT, 0)]
        );
        // Al soltarla más tarde ya no escribe nada: la letra ya salió.
        assert!(decide(&mut st, A, 0, ahora, true).is_empty());
    }

    /// The bug that locked the machine: the virtual keyboard declares every
    /// keycode, so it matches the same search that looks for real keyboards. When
    /// it ended up in that list it got grabbed too, and every key replayed
    /// Al despertar se suelta antes de volver a tomar, y el orden es el arreglo.
    ///
    /// El kernel admite **un solo** cliente con `EVIOCGRAB` por dispositivo: el
    /// segundo recibe `EBUSY`. Con `devices = volver_a_tomar()` el lado derecho
    /// se evaluaba antes de soltar el valor viejo, así que el daemon competía
    /// contra sus propios descriptores y se quedaba sin ningún teclado —sin
    /// acentos hasta reiniciar el servicio—. No se notaba mientras la señal de
    /// «me voy a dormir» llegara, porque ésa ya había soltado todo; se notaba
    /// cuando esa mitad se perdía.
    ///
    /// Esta prueba hace justamente eso: toma los teclados, y **sin soltarlos**
    /// llama a la función del despertar. Si el orden estuviera mal, la lista
    /// volvería vacía.
    ///
    /// Ignorada por defecto porque toma el teclado de verdad: mientras corre,
    /// esta máquina no escribe en ninguna otra parte. Son milisegundos y suelta
    /// al terminar, pero no es algo para meter en una corrida automática.
    /// `cargo test -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn al_despertar_se_recupera_aunque_los_teclados_sigan_tomados() {
        let mut devices = find_keyboard_devices();
        if devices.is_empty() {
            eprintln!("sin teclados que tomar: no hay nada que probar");
            return;
        }
        tomar_en_exclusiva(&mut devices);
        let cuantos = devices.len();
        if cuantos == 0 {
            // `EBUSY` acá quiere decir que el daemon está corriendo y ya los
            // tiene: es el mismo límite de un cliente por dispositivo que hace
            // falta arreglar, visto desde afuera. Parar el servicio y repetir.
            eprintln!(
                "no se pudo tomar ninguno: o faltan permisos, o el daemon está \
                 corriendo y ya los tiene. No hay nada que probar."
            );
            return;
        }

        // Sin soltar nada: es el estado en que queda el daemon si se pierde la
        // señal de que la máquina se iba a dormir.
        volver_a_tomar(&mut devices);

        assert_eq!(
            devices.len(),
            cuantos,
            "se perdieron teclados al despertar: el orden de soltar y tomar está mal"
        );

        soltar(&mut devices);
    }

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
