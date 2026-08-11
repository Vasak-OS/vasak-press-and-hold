# Press & Hold Accents Daemon - Development Steps

## Overview

Tauri v2 application that intercepts global keyboard inputs on Wayland (Wayfire).
When a key with variants (a, c, e, i, o, u, n, l) is held >400ms:
- Suppress OS native character repetition
- Show accent menu via Wayland Layer Shell overlay
- Inject selected Unicode variant via uinput virtual keyboard

---

## Phase 1: Project Configuration

### Step 1.1 - Update Cargo.toml

File: `src-tauri/Cargo.toml`

Add dependencies:
```toml
[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-shell = "2"
tauri-plugin-config-manager = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
gtk = "0.18"
gtk-layer-shell = "0.8"
evdev = "0.12"
xkbcommon = "0.7"
base64 = "0.22"
```

Remove:
- `tauri-plugin-vicons`

Keep:
- `tauri-plugin-config-manager` (required for system theme integration)

### Step 1.2 - Update tauri.conf.json

File: `src-tauri/tauri.conf.json`

Changes:
```json
{
  "productName": "vasak-press-and-hold",
  "identifier": "ar.net.vasak.press-and-hold",
  "app": {
    "windows": [
      {
        "title": "Accents",
        "width": 320,
        "height": 48,
        "decorations": false,
        "transparent": true,
        "show": false,
        "skipTaskbar": true,
        "resizable": false,
        "alwaysOnTop": true,
        "visible": false
      }
    ]
  },
  "bundle": {
    "linux": {
      "deb": {
        "depends": [
          "libcairo2",
          "libgdk-pixbuf-2.0-0",
          "libglib2.0-0t64",
          "libgtk-3-0t64",
          "libpango-1.0-0",
          "libwebkit2gtk-4.1-0",
          "libxkbcommon0"
        ]
      }
    }
  }
}
```

### Step 1.3 - Update capabilities

File: `src-tauri/capabilities/default.json`

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Capability for the accent overlay",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "core:window:default",
    "core:window:allow-show",
    "core:window:allow-hide",
    "core:window:allow-set-focus",
    "core:window:allow-close",
    "config-manager:default"
  ]
}
```

### Step 1.4 - System dependencies (handled by PKGBUILD)

System-level setup (udev rules, user group, dependencies) is managed entirely by
the PKGBUILD and the `.install` file. **Do NOT instruct users to run manual
`usermod`, `chmod`, or `udev` commands.** Everything is handled at package
install time.

See **Phase 10: Packaging (PKGBUILD)** for full details.

---

## Phase 2: Rust Backend - Core Types & Constants

### Step 2.1 - Create module structure

File: `src-tauri/src/lib.rs`

```
src-tauri/src/
  lib.rs          # Entry point + Tauri builder
  main.rs         # Binary entry (unchanged)
  accent_map.rs   # Character variant definitions
  uinput.rs       # Virtual keyboard via /dev/uinput
  input.rs        # evdev interception + state machine
```

### Step 2.2 - accent_map.rs

Define all character variants and keysym mappings:

```rust
use std::collections::HashMap;

pub fn get_variants() -> HashMap<char, Vec<char>> {
    let mut m = HashMap::new();
    m.insert('a', vec!['á', 'à', 'â', 'ä', 'ã', 'å', 'ā']);
    m.insert('c', vec!['ç', 'ć', 'č']);
    m.insert('e', vec!['é', 'è', 'ê', 'ë', 'ē', 'ė', 'ę']);
    m.insert('i', vec!['í', 'ì', 'î', 'ï', 'ī', 'į']);
    m.insert('o', vec!['ó', 'ò', 'ô', 'ö', 'õ', 'ø', 'ō']);
    m.insert('u', vec!['ú', 'ù', 'û', 'ü', 'ū', 'ų']);
    m.insert('n', vec!['ñ', 'ń']);
    m.insert('l', vec!['ł', 'ĺ', 'ļ']);
    m
}

pub fn key_to_base_char(code: u16) -> Option<char> {
    match code {
        30 => Some('a'),   // KEY_A
        46 => Some('c'),   // KEY_C
        18 => Some('e'),   // KEY_E
        23 => Some('i'),   // KEY_I
        24 => Some('o'),   // KEY_O
        22 => Some('u'),   // KEY_U
        49 => Some('n'),   // KEY_N
        38 => Some('l'),   // KEY_L
        _ => None,
    }
}

pub fn char_to_xkb_keysym(ch: char) -> Option<u32> {
    match ch {
        'a'..='z' | 'A'..='Z' => Some(ch as u32),
        'á' => Some(0x00e1), 'à' => Some(0x00e0), 'â' => Some(0x00e2),
        'ä' => Some(0x00e4), 'ã' => Some(0x00e3), 'å' => Some(0x00e5),
        'ç' => Some(0x00e7), 'ć' => Some(0x0106), 'č' => Some(0x010d),
        'é' => Some(0x00e9), 'è' => Some(0x00e8), 'ê' => Some(0x00ea),
        'ë' => Some(0x00eb), 'ē' => Some(0x0113), 'ė' => Some(0x0116),
        'ę' => Some(0x0118),
        'í' => Some(0x00ed), 'ì' => Some(0x00ec), 'î' => Some(0x00ee),
        'ï' => Some(0x00ef), 'ī' => Some(0x012a), 'į' => Some(0x012e),
        'ó' => Some(0x00f3), 'ò' => Some(0x00f2), 'ô' => Some(0x00f4),
        'ö' => Some(0x00f6), 'õ' => Some(0x00f5), 'ø' => Some(0x00f8),
        'ō' => Some(0x014c),
        'ú' => Some(0x00fa), 'ù' => Some(0x00f9), 'û' => Some(0x00fb),
        'ü' => Some(0x00fc), 'ū' => Some(0x016a), 'ų' => Some(0x0173),
        'ñ' => Some(0x00f1), 'ń' => Some(0x0143),
        'ł' => Some(0x0141), 'ĺ' => Some(0x0139), 'ļ' => Some(0x013b),
        _ => Some(0x01000000 | ch as u32),
    }
}
```

### Step 2.3 - uinput.rs

Virtual keyboard device via raw /dev/uinput:

Key structures:
```rust
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
```

Key constants (x86_64 Linux ioctl numbers):
```rust
const UI_SET_EVBIT: u64 = 0x40045564;
const UI_SET_KEYBIT: u64 = 0x40045565;
const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;
const EV_KEY: u16 = 0x01;
const EV_SYN: u16 = 0x00;
const SYN_REPORT: u16 = 0x00;
```

VirtualKeyboard struct with methods:
- `new() -> Result<Self, String>`: Opens /dev/uinput, sets capabilities, creates device
- `send_event(type_, code, value) -> Result<()>`: Writes input_event + SYN_REPORT
- `send_key(code, pressed) -> Result<()>`: Wrapper for EV_KEY events
- `send_keysym(keysym: u32) -> Result<()>`: Find keycode via xkbcommon then send
- `Drop`: UI_DEV_DESTROY + close fd

Important: `std::thread::sleep(100ms)` after UI_DEV_CREATE to let compositor detect device.

### Step 2.4 - input.rs

Evdev device reading + state machine:

```rust
use std::collections::HashMap;
use std::time::{Duration, Instant};
use evdev::{Device, InputEventKind, Key};

const HOLD_THRESHOLD: Duration = Duration::from_millis(400);

struct KeyState {
    pressed_at: Instant,
    held_emitted: bool,
}

pub fn find_keyboard_devices() -> Vec<Device> {
    // Scan /dev/input/event*
    // Check each device supports KEY_A (is a keyboard)
    // Return list of keyboard devices
}

pub fn run_input_loop(
    app_handle: tauri::AppHandle,
    inject_rx: std::sync::mpsc::Receiver<InjectCommand>,
) {
    // 1. Find and grab keyboard devices
    // 2. Create VirtualKeyboard
    // 3. Setup xkbcommon state for keycode lookup
    // 4. Main loop:
    //    - Check inject_rx for pending character injections
    //    - Read events from all devices (non-blocking)
    //    - Process each event
}
```

State machine per target key:
```
IDLE -> (Press) -> TRACKING { pressed_at: now }
TRACKING -> (Release < 400ms) -> forward press+release via uinput -> IDLE
TRACKING -> (Repeat, elapsed >= 400ms) -> emit "show-accent-menu" -> HELD
HELD -> (Release) -> IDLE (character handled by frontend)
HELD -> (Any event) -> suppress
```

Non-target keys: forward all events immediately via uinput.

### Step 2.5 - lib.rs (Main Tauri setup)

```rust
mod accent_map;
mod uinput;
mod input;

use std::sync::{Arc, Mutex, mpsc};
use tauri::{AppHandle, Emitter, Manager, State};
use gtk::prelude::*;

struct AppState {
    gtk_tx: glib::Sender<GtkCommand>,
    current_variants: Arc<Mutex<Vec<char>>>,
    inject_tx: mpsc::Sender<InjectCommand>,
}

enum GtkCommand {
    SetKeyboardInteractivity(bool),
}
```

Tauri builder setup:
```rust
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_config_manager::init())
        .setup(|app| {
            // 1. Create glib channel for GTK thread communication
            let (gtk_tx, gtk_rx) = glib::MainContext::channel(glib::Priority::DEFAULT);

            // 2. Attach channel receiver to process GTK commands
            gtk_rx.attach(None, move |cmd: GtkCommand| {
                match cmd {
                    GtkCommand::SetKeyboardInteractivity(enabled) => {
                        for w in gtk::Window::list_toplevels() {
                            if let Ok(win) = w.downcast::<gtk::ApplicationWindow>() {
                                gtk_layer_shell::set_keyboard_interactivity(&win, enabled);
                            }
                        }
                    }
                }
                glib::ControlFlow::Continue
            });

            // 3. Initialize GTK Layer Shell (deferred to idle)
            glib::idle_add_local_once(|| {
                for w in gtk::Window::list_toplevels() {
                    if let Ok(win) = w.downcast::<gtk::ApplicationWindow>() {
                        gtk_layer_shell::init_for_window(&win);
                        gtk_layer_shell::set_layer(&win, gtk_layer_shell::Layer::Overlay);
                        gtk_layer_shell::set_keyboard_interactivity(&win, false);
                        gtk_layer_shell::set_anchor(&win, gtk_layer_shell::Edge::Bottom, true);
                        gtk_layer_shell::set_margin(&win, gtk_layer_shell::Edge::Bottom, 80);
                        gtk_layer_shell::set_namespace(&win, "vasak-accents");
                    }
                }
            });

            // 4. Create inject channel
            let (inject_tx, inject_rx) = mpsc::channel();

            // 5. Store app state
            app.manage(AppState {
                gtk_tx,
                current_variants: Arc::new(Mutex::new(Vec::new())),
                inject_tx,
            });

            // 6. Hide window initially
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.hide();
            }

            // 7. Spawn input reader thread
            let handle = app.handle().clone();
            std::thread::spawn(move || input::run_input_loop(handle, inject_rx));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            select_accent,
            dismiss_accent,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### Step 2.6 - Tauri commands

```rust
#[tauri::command]
fn select_accent(app: AppHandle, index: usize) -> Result<(), String> {
    let state = app.state::<AppState>();

    // 1. Get selected character
    let ch = {
        let variants = state.current_variants.lock().map_err(|e| e.to_string())?;
        *variants.get(index).ok_or("Invalid index")?
    };

    // 2. Inject character via uinput
    state.inject_tx.send(InjectCommand::Char(ch)).map_err(|e| e.to_string())?;

    // 3. Hide window
    if let Some(win) = app.get_webview_window("main") {
        win.hide().map_err(|e| e.to_string())?;
    }

    // 4. Disable keyboard interactivity
    state.gtk_tx.send(GtkCommand::SetKeyboardInteractivity(false)).ok();

    Ok(())
}

#[tauri::command]
fn dismiss_accent(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();

    if let Some(win) = app.get_webview_window("main") {
        win.hide().map_err(|e| e.to_string())?;
    }

    state.gtk_tx.send(GtkCommand::SetKeyboardInteractivity(false)).ok();

    Ok(())
}
```

---

## Phase 3: Frontend - Accent Menu UI

### Step 3.1 - Simplify main.ts

File: `src/main.ts`

```typescript
import { createApp } from 'vue';
import App from '@/App.vue';
import '@/assets/main.css';

createApp(App).mount('#app');
```

Remove pinia, unused imports.

### Step 3.2 - Simplify App.vue

File: `src/App.vue`

Single component that listens for events and shows accent menu:

```vue
<script setup lang="ts">
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';
import { ref, onMounted, onUnmounted } from 'vue';

interface AccentPayload {
  base_char: string;
  variants: string[];
}

const visible = ref(false);
const baseChar = ref('');
const variants = ref<string[]>([]);
let unlistenFn: (() => void) | null = null;

onMounted(async () => {
  unlistenFn = await listen<AccentPayload>('show-accent-menu', async (event) => {
    baseChar.value = event.payload.base_char;
    variants.value = event.payload.variants;
    visible.value = true;
    await getCurrentWindow().show();
    await getCurrentWindow().setFocus();
  });

  window.addEventListener('keydown', handleKeyDown);
});

onUnmounted(() => {
  unlistenFn?.();
  window.removeEventListener('keydown', handleKeyDown);
});

async function handleKeyDown(e: KeyboardEvent) {
  if (!visible.value) return;

  if (e.key === 'Escape') {
    await dismiss();
    return;
  }

  if (e.key === 'Enter' || e.key === ' ') {
    await select(0);
    return;
  }

  const num = parseInt(e.key);
  if (num >= 1 && num <= variants.value.length) {
    await select(num - 1);
  }
}

async function select(index: number) {
  visible.value = false;
  await invoke('select_accent', { index });
}

async function dismiss() {
  visible.value = false;
  await invoke('dismiss_accent');
}
</script>

<template>
  <div v-if="visible" class="accent-menu">
    <button
      v-for="(ch, i) in variants"
      :key="ch"
      class="accent-item"
      @click="select(i)"
    >
      <span class="accent-key">{{ i + 1 }}</span>
      <span class="accent-char">{{ ch }}</span>
    </button>
  </div>
</template>
```

### Step 3.3 - Accent menu styles

File: `src/assets/main.css`

Add at the end (keep existing variables):

```css
.accent-menu {
  display: flex;
  gap: 4px;
  padding: 8px 12px;
  background: var(--ui-background-dark);
  border: 1px solid var(--ui-border-dark);
  border-radius: var(--corner-radius);
  box-shadow: 0 4px 24px rgba(0, 0, 0, 0.3);
  backdrop-filter: blur(12px);
}

.accent-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  padding: 6px 10px;
  border: none;
  border-radius: 6px;
  background: transparent;
  cursor: pointer;
  transition: background 0.15s;
}

.accent-item:hover {
  background: var(--ui-surface-dark);
}

.accent-key {
  font-size: 10px;
  color: var(--text-muted-dark);
  font-family: monospace;
}

.accent-char {
  font-size: 20px;
  color: var(--text-main-dark);
}
```

---

## Phase 4: GTK Layer Shell Integration

### Step 4.1 - Window setup flow

The window must be configured as a Wayland Layer Shell overlay. This happens in three stages:

1. **tauri.conf.json**: Window starts hidden (`show: false`, `visible: false`)
2. **lib.rs setup**: `glib::idle_add_local_once` initializes layer shell after GTK loop starts
3. **Event handler**: `app.emit("show-accent-menu", payload)` triggers frontend to call `window.show()`

### Step 4.2 - GTK thread safety

All `gtk_layer_shell::*` calls MUST run on the GTK main thread. The approach:

- `glib::idle_add_local_once` for one-time setup (runs on main thread after setup)
- `glib::MainContext::channel` for ongoing commands from Tauri command handlers (which run on thread pool)

```rust
// Command handler (runs on thread pool)
#[tauri::command]
fn select_accent(app: AppHandle, index: usize) -> Result<(), String> {
    let state = app.state::<AppState>();
    // ... injection logic ...

    // This sends to the channel, processed on GTK main thread
    state.gtk_tx.send(GtkCommand::SetKeyboardInteractivity(false)).ok();
    Ok(())
}

// Channel receiver (attached to GTK main loop)
gtk_rx.attach(None, move |cmd: GtkCommand| {
    match cmd {
        GtkCommand::SetKeyboardInteractivity(enabled) => {
            for w in gtk::Window::list_toplevels() {
                if let Ok(win) = w.downcast::<gtk::ApplicationWindow>() {
                    gtk_layer_shell::set_keyboard_interactivity(&win, enabled);
                }
            }
        }
    }
    glib::ControlFlow::Continue
});
```

### Step 4.3 - Layer shell configuration

```
Layer: Overlay (above all windows)
Keyboard interactivity: false by default, true when menu shown
Anchor: Bottom center
Margin bottom: 80px (above panel/taskbar)
Namespace: "vasak-accents" (for Wayfire rules)
```

---

## Phase 5: Character Injection Pipeline

### Step 5.1 - Injection flow

```
User selects char -> invoke("select_accent", {index})
  -> Rust: get char from current_variants
  -> Rust: char_to_xkb_keysym(char) -> keysym
  -> Rust: find_keycode(xkb_state, keysym) -> keycode
  -> Rust: virtual_keyboard.send_key(keycode, true)
  -> Rust: virtual_keyboard.send_key(keycode, false)
  -> Rust: window.hide()
  -> Rust: set_keyboard_interactivity(false)
```

### Step 5.2 - Keycode lookup via xkbcommon

```rust
fn find_keycode(state: &xkb::State, keysym: u32) -> Option<u16> {
    for code in 8..=255u32 {
        if state.key_get_one_sym(code) == keysym {
            return Some(code as u16);
        }
    }
    None
}
```

xkbcommon state created from system default keymap:
```rust
let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
let keymap = xkb::Keymap::new_from_names(&ctx, &xkb::Names::default(), xkb::KEYMAP_COMPILE_NO_FLAGS)
    .expect("Failed to create keymap");
let state = keymap.state();
```

### Step 5.3 - uinput event writing

```rust
fn send_event(&self, type_: u16, code: u16, value: i32) -> Result<(), String> {
    unsafe {
        let ev = InputEvent {
            tv_sec: 0, tv_usec: 0,
            type_, code, value,
        };
        libc::write(self.fd, &ev as *const _ as *const _, std::mem::size_of::<InputEvent>());

        let syn = InputEvent {
            tv_sec: 0, tv_usec: 0,
            type_: EV_SYN, code: SYN_REPORT, value: 0,
        };
        libc::write(self.fd, &syn as *const _ as *const _, std::mem::size_of::<InputEvent>());
    }
    Ok(())
}
```

---

## Phase 6: Input Interception Details

### Step 6.1 - Device discovery

```rust
fn find_keyboard_devices() -> Vec<Device> {
    let mut devices = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/dev/input") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name()
                .and_then(|n| n.to_str())
                .map_or(false, |n| n.starts_with("event"))
            {
                if let Ok(mut device) = Device::open(&path) {
                    if let Ok(keys) = device.supported_keys() {
                        if keys.contains(Key::KEY_A) {
                            devices.push(device);
                        }
                    }
                }
            }
        }
    }
    devices
}
```

### Step 6.2 - Event forwarding

Non-target keys must be forwarded through uinput to maintain normal typing:

```rust
fn forward_event(vk: &VirtualKeyboard, ev: &evdev::InputEvent) {
    match ev.kind() {
        InputEventKind::Key(key) => {
            let _ = vk.send_event(0x01, key.code(), ev.value());
        }
        InputEventKind::RelativeAxis(axis) => {
            let _ = vk.send_event(0x02, axis.code(), ev.value());
        }
        _ => {}
    }
}
```

### Step 6.3 - Target key state machine

```rust
fn handle_target_key(
    code: u16,
    value: i32,
    states: &mut HashMap<u16, KeyState>,
    vk: &VirtualKeyboard,
    app: &AppHandle,
    variants_map: &HashMap<char, Vec<char>>,
    current_variants: &Arc<Mutex<Vec<char>>>,
) {
    match value {
        1 => { // Press
            states.insert(code, KeyState {
                pressed_at: Instant::now(),
                held_emitted: false,
            });
            // Do NOT forward - wait for release or threshold
        }
        0 => { // Release
            if let Some(state) = states.remove(&code) {
                if !state.held_emitted && state.pressed_at.elapsed() < HOLD_THRESHOLD {
                    // Quick tap - forward as normal keystroke
                    let _ = vk.send_event(0x01, code, 1); // press
                    let _ = vk.send_event(0x01, code, 0); // release
                }
                // If held_emitted: already handled by menu, don't forward
            }
        }
        2 => { // Repeat
            if let Some(state) = states.get_mut(&code) {
                if !state.held_emitted && state.pressed_at.elapsed() >= HOLD_THRESHOLD {
                    state.held_emitted = true;
                    // Show accent menu
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
```

### Step 6.4 - Non-blocking device reading

Set devices to non-blocking mode to prevent the loop from stalling:

```rust
use std::os::unix::io::AsRawFd;

for device in &mut devices {
    unsafe {
        let fd = device.as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
}
```

Main loop with 1ms sleep to prevent busy-waiting:
```rust
loop {
    // 1. Check inject channel
    while let Ok(cmd) = inject_rx.try_recv() {
        match cmd {
            InjectCommand::Char(ch) => {
                if let Some(keysym) = char_to_xkb_keysym(ch) {
                    if let Some(keycode) = find_keycode(&xkb_state, keysym) {
                        let _ = vk.send_key(keycode, true);
                        let _ = vk.send_key(keycode, false);
                    }
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
                            handle_target_key(code, event.value(), ...);
                        } else {
                            forward_event(&vk, &event);
                        }
                    }
                    _ => forward_event(&vk, &event),
                }
            }
        }
    }

    std::thread::sleep(Duration::from_millis(1));
}
```

---

## Phase 7: Permission Handling

### Step 7.1 - Graceful error messages

In `uinput.rs::new()`:
```rust
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
        // ...
    }
}
```

In `input.rs::find_keyboard_devices()`:
```rust
pub fn find_keyboard_devices() -> Vec<Device> {
    // ...
    if devices.is_empty() {
        eprintln!(
            "No keyboard devices found in /dev/input/. \
             Check permissions: ls -la /dev/input/event*"
        );
    }
    devices
}
```

### Step 7.2 - Startup validation

In `lib.rs setup`:
```rust
// Validate permissions before spawning input thread
if std::fs::File::open("/dev/uinput").is_err() {
    eprintln!("ERROR: Cannot access /dev/uinput");
    eprintln!("Fix: sudo chmod 666 /dev/uinput");
    eprintln!("Or add udev rule for persistent access");
}
```

---

## Phase 8: Cleanup & Polish

### Step 8.1 - Remove unused files

Delete:
- `src/layouts/WindowAppLayout.vue`
- `src/components/topbar/` (entire directory)
- `src/composables/useReactiveIcon.ts`
- `src/assets/vue.svg`
- `package.json`: Remove `@vasakgroup/plugin-config-manager`, `@vasakgroup/plugin-vicons`, `pinia`
- `src/main.ts`: Remove pinia import and usage

### Step 8.2 - Simplify package.json

```json
{
  "name": "vasak-press-and-hold",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "bunx --bun vite",
    "build": "bunx --bun vue-tsc --noEmit && bunx --bun vite build",
    "preview": "bunx --bun vite preview",
    "tauri": "tauri",
    "lint": "bunx --bun biome check .",
    "lint:fix": "bunx --bun biome check --write . && bunx --bun biome format --write .",
    "format": "bunx --bun biome format --write ."
  },
  "dependencies": {
    "@tailwindcss/vite": "^4.3.2",
    "@tauri-apps/api": "^2.11.1",
    "@tauri-apps/plugin-shell": "^2.3.5",
    "@vasakgroup/plugin-config-manager": "^2.2.6",
    "path": "^0.12.7",
    "url": "^0.11.4",
    "vue": "^3.5.39"
  },
  "devDependencies": {
    "@biomejs/biome": "^2.5.3",
    "@tailwindcss/postcss": "^4.3.2",
    "@tauri-apps/cli": "^2.11.4",
    "@types/node": "^25.9.5",
    "@vitejs/plugin-vue": "^6.0.8",
    "postcss": "^8.5.19",
    "tailwindcss": "^4.3.2",
    "typescript": "^5.9.3",
    "vite": "^7.3.6",
    "vue-tsc": "^3.3.7"
  }
}
```

### Step 8.3 - Update main.rs

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    vapp_lib::run()
}
```

### Step 8.4 - Verify ioctl portability

The ioctl numbers in `uinput.rs` are architecture-specific:
- x86_64: Use the constants listed in Step 2.3
- aarch64: Different values

For multi-arch support:
```rust
#[cfg(target_arch = "x86_64")]
mod ioctl_defs {
    pub const UI_SET_EVBIT: u64 = 0x40045564;
    pub const UI_SET_KEYBIT: u64 = 0x40045565;
    pub const UI_DEV_CREATE: u64 = 0x5501;
    pub const UI_DEV_DESTROY: u64 = 0x5502;
}

#[cfg(target_arch = "aarch64")]
mod ioctl_defs {
    pub const UI_SET_EVBIT: u64 = 0x40045564;  // Same on aarch64 for this ioctl
    pub const UI_SET_KEYBIT: u64 = 0x40045565;
    pub const UI_DEV_CREATE: u64 = 0x5501;
    pub const UI_DEV_DESTROY: u64 = 0x5502;
}
```

Note: These particular UI_* ioctls use `_IOW` which has the same encoding on both architectures.

---

## Phase 9: Testing

### Step 9.1 - Permissions test

```bash
# Check /dev/input access
ls -la /dev/input/event*
groups  # should include 'input'

# Check /dev/uinput access
ls -la /dev/uinput
echo test > /dev/uinput  # should fail with permission error, not "no such file"
```

### Step 9.2 - Run in dev mode

```bash
bun install
cargo tauri dev
```

### Step 9.3 - Test matrix

| Test Case | Expected Result |
|-----------|----------------|
| Press+release 'a' quickly | 'a' appears in focused app |
| Hold 'a' >400ms | Accent menu appears at bottom center |
| Press '2' while menu open | 'à' injected, menu closes |
| Press Escape while menu open | Menu closes, no injection |
| Hold 'e' >400ms, press Space | 'é' injected (index 0 = Space maps to first) |
| Hold non-target key (e.g., 'b') | Character repeats normally, no menu |
| Hold 'a' >400ms, release without selection | Menu stays, no character |
| Multiple rapid holds | Each triggers its own menu |
| Hold 'n' >400ms | Shows ñ, ń options |

### Step 9.4 - Debug logging

Add to input loop for debugging:
```rust
eprintln!("[key] code={} value={} elapsed={:?}", code, value, elapsed);
```

Remove or gate behind `#[cfg(debug_assertions)]` for release.

---

## Phase 10: Packaging (PKGBUILD)

All system-level configuration is handled by the package. The user never runs
manual `usermod`, `chmod`, or `udevadm` commands.

### Step 10.1 - Create udev rules file

File: `99-vasak-press-and-hold.rules` (project root)

```
# udev rules for vasak-press-and-hold
# Grants input group access to /dev/input and /dev/uinput
# Installed by the package to /usr/lib/udev/rules.d/

KERNEL=="uinput", MODE="0660", GROUP="input"
KERNEL=="event*", MODE="0660", GROUP="input"
```

### Step 10.2 - Create .install file

File: `vasak-press-and-hold.install`

```bash
post_install() {
    echo ""
    echo "==> vasak-press-and-hold installed."
    echo ""
    echo "  To use this application, your user must be in the 'input' group."
    echo "  Run the following command, then log out and back in:"
    echo ""
    echo "    sudo usermod -aG input \$USER"
    echo ""
    echo "  udev rules have been installed to /usr/lib/udev/rules.d/"
    echo "  They will take effect after the next udev rule reload:"
    echo ""
    echo "    sudo udevadm control --reload-rules && sudo udevadm trigger"
    echo ""
}

post_upgrade() {
    post_install
}
```

### Step 10.3 - Create PKGBUILD

File: `PKGBUILD` (project root)

Key points:
- `depends`: runtime libs (gtk3, webkit2gtk-4.1, libxkbcommon)
- `makedepends`: build tools (cargo, bun, dev headers)
- `install`: points to the `.install` file
- `source`: includes the udev rules file
- `package()`: installs binary, udev rules, icons
- Binary name in tauri.conf.json `productName` must match the installed binary

```bash
# Maintainer: Vasak Group
pkgname=vasak-press-and-hold
pkgver=0.1.0
pkgrel=1
pkgdesc="Press & Hold Accents daemon - hold a key to get accented character variants on Wayland"
arch=('x86_64' 'aarch64')
url="https://github.com/VasakOS/vasak-press-and-hold"
license=('MIT')
depends=(
  'gtk3'
  'webkit2gtk-4.1'
  'libxkbcommon'
  'libappindicator-gtk3'
)
makedepends=(
  'cargo'
  'bun'
  'webkit2gtk-4.1'
  'libappindicator-gtk3'
  'libxkbcommon'
  'gtk3'
)
install=vasak-press-and-hold.install
source=("$pkgname-$pkgver.tar.gz::https://github.com/VasakOS/$pkgname/archive/refs/tags/v$pkgver.tar.gz"
        "99-vasak-press-and-hold.rules")
sha256sums=('SKIP' 'SKIP')

prepare() {
  cd "$pkgname-$pkgver"
  export RUSTUP_TOOLCHAIN=stable
  cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
  cd "$pkgname-$pkgver"
  export RUSTUP_TOOLCHAIN=stable
  export RUSTFLAGS="-C link-arg=-fuse-ld=lld"
  bun install --frozen-lockfile
  bun run tauri build
}

package() {
  cd "$pkgname-$pkgver"

  install -Dm755 "src-tauri/target/release/vapp" \
    "$pkgdir/usr/bin/vasak-press-and-hold"

  install -Dm644 "$srcdir/99-vasak-press-and-hold.rules" \
    "$pkgdir/usr/lib/udev/rules.d/99-vasak-press-and-hold.rules"

  if [ -d src-tauri/icons ]; then
    for size in 32x32 128x128 128x128@2x; do
      if [ -f "src-tauri/icons/${size}.png" ]; then
        install -Dm644 "src-tauri/icons/${size}.png" \
          "$pkgdir/usr/share/icons/hicolor/${size}/apps/vasak-press-and-hold.png"
      fi
    done
  fi
}
```

### Step 10.4 - Post-install user setup flow

The `.install` file prints instructions. The actual steps are:

```
1. Package installs udev rules -> /usr/lib/udev/rules.d/99-vasak-press-and-hold.rules
2. Package runs udevadm reload -> rules active immediately
3. User runs: sudo usermod -aG input $USER
4. User logs out and back in (or reboots)
5. Application can now access /dev/input/* and /dev/uinput
```

### Step 10.5 - Verify package builds

```bash
# From project root
makepkg -sf
# Check package contents
pacman -Ql vasak-press-and-hold
# Verify udev rules included
ls /usr/lib/udev/rules.d/99-vasak-press-and-hold.rules
```

---

## File Inventory

| File | Action | Description |
|------|--------|-------------|
| `PKGBUILD` | CREATE | Arch Linux package build script |
| `vasak-press-and-hold.install` | CREATE | Post-install user setup instructions |
| `99-vasak-press-and-hold.rules` | CREATE | udev rules for /dev/input and /dev/uinput |
| `src-tauri/Cargo.toml` | MODIFY | Add evdev, xkbcommon, gtk-layer-shell; remove vicons; keep config-manager |
| `src-tauri/tauri.conf.json` | MODIFY | Window config for overlay |
| `src-tauri/capabilities/default.json` | MODIFY | Minimal permissions |
| `src-tauri/src/lib.rs` | REWRITE | Tauri builder + commands + state |
| `src-tauri/src/main.rs` | KEEP | Binary entry point |
| `src-tauri/src/accent_map.rs` | CREATE | Character variants + keysyms |
| `src-tauri/src/uinput.rs` | CREATE | Virtual keyboard via /dev/uinput |
| `src-tauri/src/input.rs` | CREATE | evdev interception + state machine |
| `src/App.vue` | REWRITE | Accent menu component |
| `src/main.ts` | SIMPLIFY | Remove pinia |
| `src/assets/main.css` | MODIFY | Add accent menu styles |
| `package.json` | MODIFY | Remove vicons, pinia; keep config-manager |
| `src/layouts/` | DELETE | Unused |
| `src/components/topbar/` | DELETE | Unused |
| `src/composables/` | DELETE | Unused |

---

## Architecture Diagram

```
Startup:
  main() -> lib::run() -> tauri::Builder::setup()
    |
    +-> glib::idle_add_local_once()     [GTK main thread]
    |     +-> gtk_layer_shell::init_for_window()
    |     +-> set_layer(Overlay)
    |     +-> set_keyboard_interactivity(false)
    |
    +-> glib::MainContext::channel()    [GTK <-> thread pool bridge]
    |     +-> gtk_rx.attach() -> process GtkCommand enum
    |
    +-> std::thread::spawn()            [Dedicated input thread]
          +-> find_keyboard_devices()
          +-> device.grab() (exclusive)
          +-> VirtualKeyboard::new() (/dev/uinput)
          +-> xkb::Keymap::new_from_names()
          +-> loop: poll devices + inject_rx

Runtime:
  Physical Keyboard -> /dev/input/event* -> [evdev read]
    |
    +-> Non-target key -> forward via uinput -> compositor -> focused app
    |
    +-> Target key press -> start timer
    +-> Target key release <400ms -> forward press+release -> normal char
    +-> Target key repeat >=400ms -> emit("show-accent-menu") -> frontend
          |
          +-> Frontend: window.show() + setFocus()
          +-> User presses number -> invoke("select_accent", {index})
          +-> Backend: char_to_xkb_keysym() -> find_keycode() -> uinput send
          +-> Backend: window.hide() + set_keyboard_interactivity(false)
```
