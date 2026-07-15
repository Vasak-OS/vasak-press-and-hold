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

struct KeyState {
    pressed_at: Instant,
    held_emitted: bool,
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

pub fn run_input_loop(app_handle: AppHandle, inject_rx: mpsc::Receiver<InjectCommand>) {
    let vk = match VirtualKeyboard::new() {
        Ok(vk) => vk,
        Err(e) => {
            eprintln!("Failed to create virtual keyboard: {}", e);
            return;
        }
    };

    let mut devices = find_keyboard_devices();
    if devices.is_empty() {
        eprintln!("No keyboard devices found. Exiting input loop.");
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
    let mut states: HashMap<u16, KeyState> = HashMap::new();
    let current_variants: Arc<Mutex<Vec<char>>> = Arc::new(Mutex::new(Vec::new()));

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
                            if key_to_base_char(code).is_some() {
                                handle_target_key(
                                    code,
                                    event.value(),
                                    &mut states,
                                    &vk,
                                    &app_handle,
                                    &variants_map,
                                    &current_variants,
                                );
                            } else {
                                forward_key_event(&vk, code, event.value());
                            }
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

fn forward_key_event(vk: &VirtualKeyboard, code: u16, value: i32) {
    let _ = vk.send_event(0x01, code, value);
}

fn handle_target_key(
    code: u16,
    value: i32,
    states: &mut HashMap<u16, KeyState>,
    vk: &VirtualKeyboard,
    app: &AppHandle,
    variants_map: &std::collections::HashMap<char, Vec<char>>,
    current_variants: &Arc<Mutex<Vec<char>>>,
) {
    match value {
        1 => {
            // Press
            states.insert(
                code,
                KeyState {
                    pressed_at: Instant::now(),
                    held_emitted: false,
                },
            );
        }
        0 => {
            // Release
            if let Some(state) = states.remove(&code) {
                if !state.held_emitted && state.pressed_at.elapsed() < HOLD_THRESHOLD {
                    // Quick tap - forward as normal keystroke
                    let _ = vk.send_event(0x01, code, 1);
                    let _ = vk.send_event(0x01, code, 0);
                }
            }
        }
        2 => {
            // Repeat
            if let Some(state) = states.get_mut(&code) {
                if !state.held_emitted && state.pressed_at.elapsed() >= HOLD_THRESHOLD {
                    state.held_emitted = true;
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
        _ => {}
    }
}
