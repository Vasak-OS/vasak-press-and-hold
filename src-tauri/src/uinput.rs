use std::os::unix::io::RawFd;

#[repr(C)]
#[derive(Clone, Copy)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    type_: u16,
    code: u16,
    value: i32,
}

const UINPUT_MAX_NAME_SIZE: usize = 80;
/// ABS_CNT from the kernel's input-event-codes.h. The arrays below are part of
/// the struct even for a device with no absolute axes.
const ABS_CNT: usize = 64;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

/// `struct uinput_user_dev`, byte for byte as the kernel declares it.
///
/// This has to match exactly: the legacy setup path is a `write()` and
/// `uinput_setup_device` rejects anything whose length is not
/// `sizeof(struct uinput_user_dev)`. The previous definition declared the bus
/// type as 32 bits — it is 16 — and left out the four absolute-axis arrays
/// entirely, which made it 96 bytes against the 1116 the kernel expects. Every
/// write returned EINVAL, so the virtual keyboard was never created and the
/// accent picker could not type anything, on any machine.
#[repr(C)]
struct UinputUserDev {
    name: [u8; UINPUT_MAX_NAME_SIZE],
    id: InputId,
    ff_effects_max: u32,
    absmax: [i32; ABS_CNT],
    absmin: [i32; ABS_CNT],
    absfuzz: [i32; ABS_CNT],
    absflat: [i32; ABS_CNT],
}

mod ioctl_defs {
    pub const UI_SET_EVBIT: u64 = 0x40045564;
    pub const UI_SET_KEYBIT: u64 = 0x40045565;
    pub const UI_SET_LEDBIT: u64 = 0x40045569;
    pub const UI_DEV_CREATE: u64 = 0x5501;
    pub const UI_DEV_DESTROY: u64 = 0x5502;
}

const EV_KEY: u16 = 0x01;
const EV_SYN: u16 = 0x00;
const EV_LED: u16 = 0x11;
const SYN_REPORT: u16 = 0x00;

/// Las luces que tiene un teclado: Bloq Num, Bloq Mayús, Bloq Despl, Componer y
/// Kana. Se declaran todas y no sólo la de mayúsculas porque el compositor
/// maneja el conjunto: un teclado que dice tener una sola es un teclado raro.
const LEDS: [u16; 5] = [0, 1, 2, 3, 4];

/// The name the virtual keyboard reports to the kernel.
///
/// Public because `input.rs` has to recognise it: the device this creates
/// declares every keycode, so it looks exactly like a real keyboard to anything
/// scanning /dev/input — including us.
pub const DEVICE_NAME: &str = "vasak-press-and-hold";

pub struct VirtualKeyboard {
    fd: RawFd,
}

impl VirtualKeyboard {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let fd = libc::open(
                b"/dev/uinput\0".as_ptr() as *const _,
                libc::O_RDWR | libc::O_NONBLOCK,
            );
            if fd < 0 {
                let err = std::io::Error::last_os_error();
                return Err(format!(
                    "Cannot open /dev/uinput: {}. \
                     Ensure udev rules are installed: \
                     sudo udevadm control --reload-rules && sudo udevadm trigger",
                    err
                ));
            }

            // Enable EV_KEY
            if libc::ioctl(fd, ioctl_defs::UI_SET_EVBIT, EV_KEY as libc::c_uint) < 0 {
                libc::close(fd);
                return Err("Failed to set EV_KEY bit".into());
            }

            // Las luces.
            //
            // Sin esto el teclado virtual no tiene ninguna, y como el físico
            // está tomado en exclusiva el compositor se queda sin dónde
            // encender la de Bloq Mayús: la luz del teclado no se prendía nunca
            // y el indicador del panel no tenía qué leer. El compositor escribe
            // en este mismo descriptor cuando el estado cambia (ver
            // `read_led_events`).
            if libc::ioctl(fd, ioctl_defs::UI_SET_EVBIT, EV_LED as libc::c_uint) < 0 {
                libc::close(fd);
                return Err("Failed to set EV_LED bit".into());
            }

            for led in LEDS {
                if libc::ioctl(fd, ioctl_defs::UI_SET_LEDBIT, led as libc::c_uint) < 0 {
                    libc::close(fd);
                    return Err(format!("Failed to set ledbit for {}", led));
                }
            }

            // Enable all keycodes (0..256)
            for code in 0u16..256 {
                if libc::ioctl(fd, ioctl_defs::UI_SET_KEYBIT, code as libc::c_uint) < 0 {
                    libc::close(fd);
                    return Err(format!("Failed to set keybit for code {}", code));
                }
            }

            // Create device
            let mut dev: UinputUserDev = std::mem::zeroed();
            // The struct is zeroed, so the NUL terminator is already there as
            // long as the name is shorter than the field.
            let name = DEVICE_NAME.as_bytes();
            debug_assert!(name.len() < UINPUT_MAX_NAME_SIZE);
            dev.name[..name.len()].copy_from_slice(name);
            dev.id.bustype = 0x03; // BUS_USB
            dev.id.vendor = 0x1234;
            dev.id.product = 0x5678;
            dev.id.version = 1;

            let ptr = &dev as *const UinputUserDev as *const libc::c_void;
            if libc::write(fd, ptr, std::mem::size_of::<UinputUserDev>())
                != std::mem::size_of::<UinputUserDev>() as isize
            {
                let err = std::io::Error::last_os_error();
                libc::close(fd);
                return Err(format!(
                    "Failed to write device info ({} bytes): {}",
                    std::mem::size_of::<UinputUserDev>(),
                    err
                ));
            }

            if libc::ioctl(fd, ioctl_defs::UI_DEV_CREATE) < 0 {
                libc::close(fd);
                return Err("Failed to create uinput device".into());
            }

            // Let compositor detect the device
            std::thread::sleep(std::time::Duration::from_millis(100));

            Ok(Self { fd })
        }
    }

    pub fn send_event(&self, type_: u16, code: u16, value: i32) -> Result<(), String> {
        let event = InputEvent {
            tv_sec: 0,
            tv_usec: 0,
            type_,
            code,
            value,
        };
        let syn = InputEvent {
            tv_sec: 0,
            tv_usec: 0,
            type_: EV_SYN,
            code: SYN_REPORT,
            value: 0,
        };
        unsafe {
            let ev_bytes =
                std::slice::from_raw_parts(&event as *const InputEvent as *const u8, std::mem::size_of::<InputEvent>());
            let syn_bytes =
                std::slice::from_raw_parts(&syn as *const InputEvent as *const u8, std::mem::size_of::<InputEvent>());

            if libc::write(self.fd, ev_bytes.as_ptr() as *const _, ev_bytes.len()) < 0 {
                return Err("Failed to write input event".into());
            }
            if libc::write(self.fd, syn_bytes.as_ptr() as *const _, syn_bytes.len()) < 0 {
                return Err("Failed to write syn report".into());
            }
        }
        Ok(())
    }

    /// Las luces que el compositor quiere encender o apagar.
    ///
    /// Un dispositivo de uinput es de ida y vuelta: lo que el sistema decide
    /// sobre sus LEDs vuelve por el mismo descriptor. Como el teclado de verdad
    /// está tomado en exclusiva, este es el único lugar donde se puede saber que
    /// Bloq Mayús cambió; quien llama replica cada cambio en los teclados
    /// físicos para que la lucecita vuelva a funcionar.
    ///
    /// No bloquea: el descriptor es `O_NONBLOCK` y sin nada que leer devuelve
    /// la lista vacía.
    pub fn read_led_events(&self) -> Vec<(u16, bool)> {
        let mut cambios = Vec::new();
        let tamano = std::mem::size_of::<InputEvent>();
        let mut buffer = vec![0u8; tamano * 16];

        loop {
            let leidos = unsafe {
                libc::read(self.fd, buffer.as_mut_ptr() as *mut libc::c_void, buffer.len())
            };

            if leidos <= 0 {
                break;
            }

            let leidos = leidos as usize;
            for trozo in buffer[..leidos].chunks_exact(tamano) {
                // El buffer se llenó desde el kernel con esta misma estructura.
                let evento: InputEvent = unsafe { std::ptr::read(trozo.as_ptr() as *const _) };
                if evento.type_ == EV_LED {
                    cambios.push((evento.code, evento.value != 0));
                }
            }

            if leidos < buffer.len() {
                break;
            }
        }

        cambios
    }

    pub fn send_key(&self, code: u16, pressed: bool) -> Result<(), String> {
        self.send_event(EV_KEY, code, if pressed { 1 } else { 0 })
    }

    pub fn send_keysym(&self, keysym: u32) -> Result<(), String> {
        let keycode = find_keycode_for_keysym(keysym)?;
        self.send_key(keycode, true)?;
        self.send_key(keycode, false)?;
        Ok(())
    }
}

impl Drop for VirtualKeyboard {
    fn drop(&mut self) {
        unsafe {
            libc::ioctl(self.fd, ioctl_defs::UI_DEV_DESTROY);
            libc::close(self.fd);
        }
    }
}

fn find_keycode_for_keysym(keysym: u32) -> Result<u16, String> {
    use xkbcommon::xkb;

    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(
        &context,
        "",
        "",
        "",
        "",
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .ok_or("Failed to create keymap")?;
    let state = xkb::State::new(&keymap);

    for evdev_code in 0u16..256 {
        let xkb_code: xkb::Keycode = (evdev_code as u32 + 8).into();
        let sym = state.key_get_one_sym(xkb_code);
        if sym == keysym.into() {
            return Ok(evdev_code);
        }
    }

    Err(format!("No keycode found for keysym 0x{:x}", keysym))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kernel rejects the setup write unless its length is exactly
    /// `sizeof(struct uinput_user_dev)`, so this number is not a detail: at 96
    /// bytes — what the struct used to be — every write returned EINVAL and the
    /// virtual keyboard was never created.
    #[test]
    fn the_device_struct_is_the_size_the_kernel_expects() {
        assert_eq!(std::mem::size_of::<InputId>(), 8);
        assert_eq!(std::mem::size_of::<UinputUserDev>(), 1116);
        // name, then the id right after it, then ff_effects_max at 88.
        assert_eq!(std::mem::offset_of!(UinputUserDev, id), 80);
        assert_eq!(std::mem::offset_of!(UinputUserDev, ff_effects_max), 88);
    }

    /// The real thing, against the real kernel. Ignored by default because it
    /// needs access to /dev/uinput, which is exactly what it is checking:
    /// `cargo test -- --ignored --nocapture` on a machine where the udev rule
    /// this package ships is installed.
    #[test]
    #[ignore]
    fn the_kernel_accepts_the_device() {
        match VirtualKeyboard::new() {
            Ok(_) => println!("teclado virtual creado y destruido"),
            Err(e) => panic!("{e}"),
        }
    }
}
