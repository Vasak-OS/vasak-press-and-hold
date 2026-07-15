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

#[repr(C)]
struct UinputUserDev {
    name: [u8; 80],
    id_bustype: u32,
    id_vendor: u16,
    id_product: u16,
    id_version: u16,
    ff_effects_max: u32,
}

mod ioctl_defs {
    pub const UI_SET_EVBIT: u64 = 0x40045564;
    pub const UI_SET_KEYBIT: u64 = 0x40045565;
    pub const UI_DEV_CREATE: u64 = 0x5501;
    pub const UI_DEV_DESTROY: u64 = 0x5502;
}

const EV_KEY: u16 = 0x01;
const EV_SYN: u16 = 0x00;
const SYN_REPORT: u16 = 0x00;

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
                     Ensure your user is in the 'input' group: \
                     sudo usermod -aG input $USER",
                    err
                ));
            }

            // Enable EV_KEY
            if libc::ioctl(fd, ioctl_defs::UI_SET_EVBIT, EV_KEY as libc::c_uint) < 0 {
                libc::close(fd);
                return Err("Failed to set EV_KEY bit".into());
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
            let name = b"vasak-press-and-hold\0";
            dev.name[..name.len()].copy_from_slice(name);
            dev.id_bustype = 0x03; // BUS_USB
            dev.id_vendor = 0x1234;
            dev.id_product = 0x5678;
            dev.id_version = 1;

            let ptr = &dev as *const UinputUserDev as *const libc::c_void;
            if libc::write(fd, ptr, std::mem::size_of::<UinputUserDev>())
                != std::mem::size_of::<UinputUserDev>() as isize
            {
                libc::close(fd);
                return Err("Failed to write device info".into());
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
